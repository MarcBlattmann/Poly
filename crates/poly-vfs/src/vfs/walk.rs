//! Devices, path walking and the checks shared by operations that change a directory.

use super::*;

impl Vfs {
    /// Makes `dev` the character device with number `id` (`EBUSY` if one
    /// is registered already).
    pub fn register_char_device(&self, id: DeviceId, dev: Arc<dyn CharDevice>) -> Result<()> {
        let mut devices = self.devices.lock();
        if devices.contains_key(&id) {
            return Err(Errno::EBUSY);
        }
        devices.insert(id, dev);
        Ok(())
    }

    /// Removes a device; files already open on it keep it.
    pub fn unregister_char_device(&self, id: DeviceId) -> Option<Arc<dyn CharDevice>> {
        self.devices.lock().remove(&id)
    }

    pub fn char_device(&self, id: DeviceId) -> Option<Arc<dyn CharDevice>> {
        self.devices.lock().get(&id).cloned()
    }

    // ----- walking -----

    /// The location `loc` shows: if file systems are mounted on it, the
    /// root of the topmost one (repeatedly).
    pub(crate) fn follow_mounts(&self, mut loc: Location) -> Location {
        let table = self.mounts.lock();
        for _ in 0..=table.mounts.len() + 64 {
            match Self::covering_of(&table, &loc) {
                Some(m) => loc = Location { ino: m.root, mount: m },
                None => break,
            }
        }
        loc
    }

    /// The topmost mount on `loc`: in the table, or among the mounts a
    /// detached tree keeps ([`Mount::detached_covering`]).
    pub(crate) fn covering_of(table: &crate::mount::MountTable, loc: &Location) -> Option<Arc<Mount>> {
        match table.covering(loc.mount.id, loc.ino) {
            Some(m) => Some(m.clone()),
            None => loc.mount.detached_covering(loc.ino),
        }
    }

    /// Looks `name` up in `dir`, without following mounts.
    pub(super) fn raw_child(&self, dir: &Location, name: &[u8]) -> Result<Location> {
        let ino = dir.mount.sb.lookup(dir.ino, name)?;
        Ok(Location { mount: dir.mount.clone(), ino })
    }

    /// Looks `name` up in `dir` and follows mounts on the result.
    pub(crate) fn child(&self, dir: &Location, name: &[u8]) -> Result<Location> {
        Ok(self.follow_mounts(self.raw_child(dir, name)?))
    }

    pub(super) fn is_mount_point(&self, loc: &Location) -> bool {
        Self::covering_of(&self.mounts.lock(), loc).is_some()
    }

    pub(super) fn start(ctx: &FsContext, at: At<'_>) -> Location {
        match at {
            At::Cwd => ctx.cwd.loc().clone(),
            At::Node(n) => n.loc().clone(),
        }
    }

    pub(super) fn walk<'a>(&'a self, ctx: &FsContext, cred: &'a Cred) -> Walk<'a> {
        Walk::new(self, cred, ctx.root.loc().clone())
    }

    pub(super) fn parent_of(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Parent> {
        self.parent_arg(ctx, cred, PathArg::new(at, path))
    }

    /// Steps 1 to 3 of the order of path errors (`path.rs`) for everything
    /// but the last name's own length: the string, the start, every
    /// directory on the way.
    pub(super) fn dirs_of(&self, ctx: &FsContext, cred: &Cred, arg: PathArg<'_>) -> Result<Parent> {
        let (path, at) = arg.begin()?;
        self.walk(ctx, cred).dirs(Self::start(ctx, at), path)
    }

    /// [`dirs_of`](Self::dirs_of) and the last name's length.
    pub(super) fn parent_arg(&self, ctx: &FsContext, cred: &Cred, arg: PathArg<'_>) -> Result<Parent> {
        let p = self.dirs_of(ctx, cred, arg)?;
        p.check_name()?;
        Ok(p)
    }

    pub(super) fn resolve(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        follow: bool,
    ) -> Result<Location> {
        check_path(path)?;
        self.walk(ctx, cred).resolve(Self::start(ctx, at), path, follow)
    }

    /// Resolves `path` to a pinned node; the last component is followed if
    /// it is a symbolic link and `follow` is set (or the path ends in `/`).
    pub fn lookup(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<NodeRef> {
        self.resolve(ctx, cred, at, path, follow)?.pin()
    }

    // ----- checks shared by the operations that change a directory -----

    /// Write and search permission on `dir`, on a writable mount.
    pub(super) fn may_change(&self, cred: &Cred, dir: &Location) -> Result<Attr> {
        if dir.mount.read_only() {
            return Err(Errno::EROFS);
        }
        let attr = dir.attr()?;
        cred.check(&attr, Access::WRITE | Access::EXEC)?;
        Ok(attr)
    }

    /// The sticky-bit rule for removing or replacing `victim` in `dir`
    /// (`unlink(2)`, `rename(2)`: `EPERM` unless the caller owns the file
    /// or the directory or has `CAP_FOWNER`).
    pub(super) fn sticky_allows(cred: &Cred, dir: &Attr, victim: &Attr) -> Result<()> {
        if dir.perm & S_ISVTX != 0 && !cred.has(cap::FOWNER) && cred.uid != dir.uid && cred.uid != victim.uid {
            Err(Errno::EPERM)
        } else {
            Ok(())
        }
    }

    /// Owner, group and permission bits of a node created in a directory
    /// with attributes `dir`: the caller's ids, or the directory's group if
    /// it is set-group-ID (a new directory then inherits the bit), as
    /// `open(2)` and `mkdir(2)` describe for Linux. A new file asking for
    /// set-group-ID in a group the caller is not in loses the bit unless
    /// the caller has `CAP_FSETID` (`inode(7)`).
    pub(super) fn new_owner(cred: &Cred, dir: &Attr, perm: u16, is_dir: bool) -> (u32, u32, u16) {
        let mut perm = perm & PERM_MASK;
        let gid = if dir.perm & S_ISGID != 0 {
            if is_dir {
                perm |= S_ISGID;
            }
            dir.gid
        } else {
            cred.gid
        };
        if !is_dir && perm & S_ISGID != 0 && !cred.has(cap::FSETID) && !cred.in_group(gid) {
            perm &= !S_ISGID;
        }
        (cred.uid, gid, perm)
    }

    /// The name a creating operation makes: `EEXIST` for `/`, `.` and `..`,
    /// which always exist, and for names that do.
    pub(super) fn new_name<'p>(&self, p: &'p Parent) -> Result<&'p [u8]> {
        let Last::Name(name) = &p.last else { return Err(Errno::EEXIST) };
        match self.raw_child(&p.dir, name) {
            Ok(_) => Err(Errno::EEXIST),
            Err(Errno::ENOENT) => Ok(name),
            Err(e) => Err(e),
        }
    }
}
