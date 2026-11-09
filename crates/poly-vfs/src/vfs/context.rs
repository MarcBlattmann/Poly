//! The process context: chdir, chroot, getcwd and path reconstruction.

use super::*;

impl Vfs {
    /// `chdir(2)`.
    pub fn chdir(&self, ctx: &mut FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        let node = self.lookup(ctx, cred, at, path, true)?;
        ctx.set_cwd(cred, node)
    }

    /// `chroot(2)`.
    pub fn chroot(&self, ctx: &mut FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        let node = self.lookup(ctx, cred, at, path, true)?;
        ctx.set_root(cred, node)
    }

    /// `getcwd(2)`: the working directory's path from the process root.
    /// A working directory outside the root (after a chroot(2) that left
    /// it there) is `(unreachable)` followed by its path from the topmost
    /// root (chroot's spec sheet; getcwd(3) describes the prefix).
    pub fn getcwd(&self, ctx: &FsContext) -> Result<Vec<u8>> {
        match self.path_from_root(ctx, &ctx.cwd)? {
            (path, true) => Ok(path),
            (path, false) => {
                let mut out = b"(unreachable)".to_vec();
                out.extend_from_slice(&path);
                Ok(out)
            }
        }
    }

    /// The absolute path of `node` as seen from `ctx`'s root, found by
    /// walking `..` and looking for each directory's name in its parent.
    /// `ENOENT` if the node has been removed or is not below the root, or
    /// if it is not a directory and has no name in the scanned parents.
    /// Meant for directories (getcwd, `/proc/self/cwd`).
    pub fn path_of(&self, ctx: &FsContext, node: &NodeRef) -> Result<Vec<u8>> {
        match self.path_from_root(ctx, node)? {
            (path, true) => Ok(path),
            (_, false) => Err(Errno::ENOENT),
        }
    }

    /// [`path_of`](Self::path_of), and for a node outside the process root
    /// its path from the topmost root instead; the flag says whether the
    /// process root was met.
    pub(super) fn path_from_root(&self, ctx: &FsContext, node: &NodeRef) -> Result<(Vec<u8>, bool)> {
        let cred = Cred::root();
        let walk = self.walk(ctx, &cred);
        let mut cur = self.follow_mounts(node.loc().clone());
        let mut names: Vec<Vec<u8>> = Vec::new();
        let mut total = 0;
        let mut reachable = true;
        loop {
            if cur.same(walk.top_root()) || cur.same(ctx.root.loc()) {
                break;
            }
            if cur.is_mount_root() {
                match cur.mount.parent() {
                    Some((m, ino)) => {
                        // Name the mount point, not the mounted root.
                        cur = Location { mount: m, ino };
                        if cur.same(walk.top_root()) || cur.same(ctx.root.loc()) {
                            break;
                        }
                        if cur.is_mount_root() {
                            continue;
                        }
                    }
                    // The topmost root without meeting the process root:
                    // the node is outside it.
                    None => {
                        reachable = false;
                        break;
                    }
                }
            }
            if cur.attr()?.nlink == 0 {
                return Err(Errno::ENOENT);
            }
            let parent = cur.mount.sb.fs.lookup(cur.ino, b"..")?;
            let name = self.name_in(&cur.mount, parent, cur.ino)?;
            total += name.len() + 1;
            if total >= PATH_MAX {
                return Err(Errno::ENAMETOOLONG);
            }
            names.push(name);
            cur = Location { mount: cur.mount.clone(), ino: parent };
        }
        let mut path = Vec::with_capacity(total.max(1));
        for name in names.iter().rev() {
            path.push(b'/');
            path.extend_from_slice(name);
        }
        if path.is_empty() {
            path.push(b'/');
        }
        Ok((path, reachable))
    }

    /// The absolute path of the file `file` is open on, as seen from
    /// `ctx`'s root: what `/proc/<pid>/fd/<n>` links to and what
    /// `/proc/<pid>/maps` shows. A directory's path is found by walking
    /// `..` ([`path_of`](Self::path_of)); any other file's from the
    /// directory and name `open` found it by. If that name no longer leads
    /// to the file, the directory is searched for its new name (a rename
    /// within the directory); a file with no links left gets `" (deleted)"`
    /// appended (proc(5)), and one renamed into another directory keeps
    /// its old path. `ENOENT` when the file was not opened by name (pipes,
    /// sockets) or its directory is gone or outside the root.
    pub fn file_path(&self, ctx: &FsContext, file: &OpenFile) -> Result<Vec<u8>> {
        if let Some(m) = file.memfd() {
            return Ok(m.path());
        }
        let node = file.node();
        if file.kind() == FileType::Directory {
            return self.path_of(ctx, node);
        }
        let origin = file.origin().ok_or(Errno::ENOENT)?;
        let mount = origin.mount.upgrade().ok_or(Errno::ENOENT)?;
        let dir = Location { mount, ino: origin.dir }.pin()?;
        let mut path = self.path_of(ctx, &dir)?;
        if path != b"/" {
            path.push(b'/');
        }
        let linked = node.attr()?.nlink > 0;
        let same = self.child(dir.loc(), &origin.name).is_ok_and(|l| l.same(node.loc()));
        let name = if linked && !same && Arc::ptr_eq(dir.mount(), node.mount()) {
            self.name_in(dir.mount(), dir.ino(), node.ino()).ok()
        } else {
            None
        };
        path.extend_from_slice(name.as_deref().unwrap_or(&origin.name));
        if !linked {
            path.extend_from_slice(b" (deleted)");
        }
        Ok(path)
    }

    /// The name `child` has in directory `dir`: a scan of the directory,
    /// remembered until a name changes anywhere (file systems whose
    /// entries come and go on their own, `cache_lookups` false, are always
    /// scanned).
    pub(super) fn name_in(&self, mount: &Mount, dir: Ino, child: Ino) -> Result<Vec<u8>> {
        let cacheable = mount.sb.info.cache_lookups;
        let key = (mount.sb.dev.major, mount.sb.dev.minor, dir, child);
        let generation = self.namespace_generation();
        if cacheable {
            let names = self.names.lock();
            if names.generation == generation
                && let Some(n) = names.map.get(&key)
            {
                return Ok(n.clone());
            }
        }
        let mut found = None;
        mount.sb.fs.readdir(dir, 0, &mut |e| {
            if e.ino == child && e.name != b"." && e.name != b".." {
                found = Some(e.name.to_vec());
                return false;
            }
            true
        })?;
        let name = found.ok_or(Errno::ENOENT)?;
        if cacheable {
            let mut names = self.names.lock();
            if names.generation != generation || names.map.len() >= NAME_CACHE_MAX {
                names.map.clear();
                names.generation = generation;
            }
            names.map.insert(key, name.clone());
        }
        Ok(name)
    }
}
