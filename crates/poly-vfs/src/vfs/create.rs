//! Creating: open (with O_CREAT, O_TMPFILE, exec), mkdir, mknod, symlinks and hard links.

use super::*;

impl Vfs {
    /// `open(2)` / `openat(2)`. `mode` is used when a file is created
    /// (masked by the umask). The `O_CLOEXEC` flag is the descriptor's
    /// business: pass it to [`FdTable::install`](crate::FdTable::install).
    pub fn open(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        flags: OpenFlags,
        mode: u16,
    ) -> Result<Arc<OpenFile>> {
        self.open_resolve(ctx, cred, at, path, flags, mode, Resolve::empty())
    }

    /// openat2(2): [`open`](Self::open) with the walk restricted by
    /// `resolve` ([`Resolve`]). With [`Resolve::IN_ROOT`] or
    /// [`Resolve::BENEATH`], `at` is the directory the walk is rooted at
    /// or must stay below, also for an absolute `path`.
    #[allow(clippy::too_many_arguments)]
    pub fn open_resolve(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        flags: OpenFlags,
        mode: u16,
        resolve: Resolve,
    ) -> Result<Arc<OpenFile>> {
        let mut flags = flags;
        if flags.contains(OpenFlags::PATH) {
            flags = flags.intersection(OpenFlags::PATH | OpenFlags::DIRECTORY | OpenFlags::NOFOLLOW);
        } else {
            if flags.contains(OpenFlags::TMPFILE) {
                return self.open_tmpfile(ctx, cred, at, path, flags, mode);
            }
            if flags.contains(OpenFlags::CREAT) && flags.contains(OpenFlags::DIRECTORY) {
                return Err(Errno::EINVAL);
            }
        }
        let follow = !flags.contains(OpenFlags::NOFOLLOW);
        check_path(path)?;
        let start = Self::start(ctx, at);
        let mut walk = Walk::restricted(self, cred, ctx.root.loc().clone(), &start, resolve);
        let ((loc, named), created) = if !flags.contains(OpenFlags::CREAT) {
            (walk.resolve_named(start, path, follow)?, false)
        } else {
            self.open_creat(&mut walk, ctx, start, path, flags, mode)?
        };
        let origin = named.map(|(dir, name)| Origin {
            mount: Arc::downgrade(&dir.mount),
            dir: dir.ino,
            name: name.into_boxed_slice(),
        });
        let attr = loc.attr()?;
        let path_only = flags.contains(OpenFlags::PATH);
        if attr.kind == FileType::Symlink && !path_only {
            // Only reachable with O_NOFOLLOW.
            return Err(Errno::ELOOP);
        }
        if flags.contains(OpenFlags::DIRECTORY) && attr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        if path_only {
            return Ok(Arc::new(OpenFile::new(loc.pin()?, attr.kind, Backend::Path, flags).with_origin(origin)));
        }
        // Access mode 3 opens for neither reading nor writing, but needs
        // the permission to do both (open(2)).
        let neither = flags.intersection(OpenFlags::ACCMODE) == OpenFlags::ACCMODE;
        let write = flags.writable() || neither;
        // O_TMPFILE's own bit without O_DIRECTORY is not O_TMPFILE, and
        // on a directory it is EINVAL (the spec of open).
        if attr.kind == FileType::Directory && flags.intersects(OpenFlags::TMPFILE.without(OpenFlags::DIRECTORY)) {
            return Err(Errno::EINVAL);
        }
        if attr.kind == FileType::Directory && (write || flags.intersects(OpenFlags::CREAT | OpenFlags::TRUNC)) {
            return Err(Errno::EISDIR);
        }
        if !created {
            if (write || flags.contains(OpenFlags::TRUNC))
                && is_fs_object(attr.kind)
                && loc.mount.read_only_for(loc.ino)
            {
                return Err(Errno::EROFS);
            }
            let mut want = Access::NONE;
            if flags.readable() || neither {
                want = want | Access::READ;
            }
            if write || flags.contains(OpenFlags::TRUNC) {
                want = want | Access::WRITE;
            }
            cred.check(&attr, want)?;
            // open(2): "ETXTBSY pathname refers to an executable image
            // which is currently being executed and write access was
            // requested."
            if want.contains(Access::WRITE) && attr.kind == FileType::Regular && loc.mount.sb.exec_busy(loc.ino) {
                return Err(Errno::ETXTBSY);
            }
            // open(2): "EPERM The O_NOATIME flag was specified, but the
            // effective user ID of the caller did not match the owner of
            // the file and the caller was not privileged" (CAP_FOWNER).
            if flags.contains(OpenFlags::NOATIME) && !cred.owns(&attr) {
                return Err(Errno::EPERM);
            }
            // An append-only or immutable inode (`crate::fileattr`).
            crate::fileattr::check_open(&loc, flags, write)?;
        }
        let backend = match attr.kind {
            FileType::Regular | FileType::Directory => Backend::Inode,
            FileType::CharDevice => {
                if loc.mount.flags().no_dev {
                    return Err(Errno::EACCES);
                }
                let dev = self.char_device(attr.rdev).ok_or(Errno::ENXIO)?;
                // A device may open as another one (`CharDevice::open`).
                Backend::Device(dev.open(flags)?.unwrap_or(dev))
            }
            // A FIFO opens as an end of the inode's pipe (`fifo.rs`; a
            // blocking open waits in the caller, `PipeEnd::wait_for_peer`).
            FileType::Fifo => {
                let fs = Arc::as_ptr(loc.mount.fs()) as *const () as usize;
                let (read, write) = (flags.readable(), flags.writable());
                let end = self.fifos.open(fs, loc.ino, read, write, flags.contains(OpenFlags::NONBLOCK))?;
                Backend::Object(Arc::new(end))
            }
            // Block devices are reached through the block layer, and a
            // socket's name cannot be opened (`open(2)`: ENXIO).
            FileType::BlockDevice | FileType::Socket | FileType::Symlink => {
                return Err(Errno::ENXIO);
            }
        };
        let node = loc.pin()?;
        if flags.contains(OpenFlags::TRUNC) && attr.kind == FileType::Regular && !created {
            node.clear_setid(cred)?;
            if attr.size != 0 {
                node.truncate_unchecked(0)?;
            }
        }
        let mut file = OpenFile::new(node, attr.kind, backend, flags).with_origin(origin);
        if attr.kind == FileType::Regular && flags.writable() && !flags.contains(OpenFlags::PATH) {
            file = file.counting_text_writer();
        }
        let file = Arc::new(file);
        inotify::file_event(&file, inotify::mask::IN_OPEN);
        Ok(file)
    }

    /// `O_TMPFILE` (open(2)): an unnamed regular file in the directory
    /// `path` names, open for writing (`EINVAL` without write access or
    /// with `O_CREAT`; `ENOTDIR` if `path` is not a directory), with
    /// `mode & ~umask`; it has no links and goes when closed, unless
    /// `linkat` with `AT_EMPTY_PATH` names it, which `O_EXCL` rules out.
    /// `F_GETFL` reports `O_TMPFILE`.
    pub(super) fn open_tmpfile(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        flags: OpenFlags,
        mode: u16,
    ) -> Result<Arc<OpenFile>> {
        if !flags.writable() || flags.contains(OpenFlags::CREAT) {
            return Err(Errno::EINVAL);
        }
        check_path(path)?;
        let dir = self.walk(ctx, cred).resolve(Self::start(ctx, at), path, true)?;
        if dir.attr()?.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let dir_attr = self.may_change(cred, &dir)?;
        let (uid, gid, perm) = Self::new_owner(cred, &dir_attr, mode & !ctx.umask, false);
        let node = NewNode { kind: FileType::Regular, perm, uid, gid, rdev: DeviceId::default() };
        let ino = dir.mount.sb.fs.create_tmpfile(dir.ino, &node)?;
        let loc = Location { mount: dir.mount.clone(), ino };
        let file = OpenFile::new(loc.pin()?, FileType::Regular, Backend::Inode, flags);
        Ok(Arc::new(if flags.contains(OpenFlags::EXCL) { file.unnameable() } else { file }))
    }

    /// The `O_CREAT` walk: the last component is created if missing, and
    /// followed if it is a symbolic link (unless `O_EXCL` or `O_NOFOLLOW`),
    /// including a dangling one, whose target is then created.
    pub(super) fn open_creat(
        &self,
        walk: &mut Walk<'_>,
        ctx: &FsContext,
        start: Location,
        path: &[u8],
        flags: OpenFlags,
        mode: u16,
    ) -> Result<(Found, bool)> {
        let mut p = walk.parent(start, path)?;
        loop {
            let Last::Name(name) = &p.last else { return Err(Errno::EISDIR) };
            match walk.child(&p.dir, name) {
                Ok(_) if flags.contains(OpenFlags::EXCL) => return Err(Errno::EEXIST),
                Ok(loc) => {
                    let kind = loc.attr()?.kind;
                    if kind != FileType::Symlink {
                        // "file/" asks for a directory to be made: EISDIR
                        // for a regular file, as for a missing name (the
                        // spec of open).
                        if p.trailing_slash && kind == FileType::Regular {
                            return Err(Errno::EISDIR);
                        }
                        if p.trailing_slash && kind != FileType::Directory {
                            return Err(Errno::ENOTDIR);
                        }
                        return Ok(((loc, Some((p.dir, name.clone()))), false));
                    }
                    if flags.contains(OpenFlags::NOFOLLOW) {
                        return Err(Errno::ELOOP);
                    }
                    let mut target = match walk.link_target(&loc)? {
                        crate::path::LinkTarget::Path(t) => t,
                        crate::path::LinkTarget::Jump(found) => {
                            if p.trailing_slash && found.0.attr()?.kind != FileType::Directory {
                                return Err(Errno::ENOTDIR);
                            }
                            return Ok((found, false));
                        }
                    };
                    if p.trailing_slash {
                        target.push(b'/');
                    }
                    p = walk.parent(p.dir.clone(), &target)?;
                }
                Err(Errno::ENOENT) => {
                    if p.trailing_slash {
                        return Err(Errno::EISDIR);
                    }
                    let dir_attr = self.may_change(walk.cred, &p.dir)?;
                    let (uid, gid, perm) = Self::new_owner(walk.cred, &dir_attr, mode & !ctx.umask, false);
                    let node = NewNode { kind: FileType::Regular, perm, uid, gid, rdev: DeviceId::default() };
                    let ino = self.name_changed(p.dir.mount.sb.fs.create(p.dir.ino, name, &node))?;
                    inotify::entry_event(&p.dir, name, inotify::mask::IN_CREATE, false, 0);
                    let loc = Location { mount: p.dir.mount.clone(), ino };
                    return Ok(((loc, Some((p.dir, name.clone()))), true));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Opens a file for `execve`: a regular file (`EACCES` otherwise) the
    /// caller may execute, not on a `noexec` mount (`EACCES`).
    pub fn open_exec(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Arc<OpenFile>> {
        self.open_exec_with(ctx, cred, at, path, true)
    }

    /// [`open_exec`](Self::open_exec), following a final symbolic link
    /// only with `follow` (execveat(2) `AT_SYMLINK_NOFOLLOW`: `ELOOP` for
    /// a link).
    pub fn open_exec_with(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        follow: bool,
    ) -> Result<Arc<OpenFile>> {
        check_path(path)?;
        let (loc, named) = self.walk(ctx, cred).resolve_named(Self::start(ctx, at), path, follow)?;
        let origin = named.map(|(dir, name)| Origin {
            mount: Arc::downgrade(&dir.mount),
            dir: dir.ino,
            name: name.into_boxed_slice(),
        });
        Self::exec_file(cred, loc, origin)
    }

    /// Opens the file `file` is open on for `execve` (execveat(2) with
    /// `AT_EMPTY_PATH`): the checks of [`open_exec`](Self::open_exec); a
    /// descriptor of a symbolic link (`O_PATH | O_NOFOLLOW`) is `ELOOP`.
    pub fn open_exec_file(&self, cred: &Cred, file: &OpenFile) -> Result<Arc<OpenFile>> {
        Self::exec_file(cred, file.node().loc().clone(), file.origin().cloned())
    }

    pub(super) fn exec_file(cred: &Cred, loc: Location, origin: Option<Origin>) -> Result<Arc<OpenFile>> {
        let attr = loc.attr()?;
        if attr.kind == FileType::Symlink {
            return Err(Errno::ELOOP);
        }
        if attr.kind != FileType::Regular || loc.mount.flags().no_exec {
            return Err(Errno::EACCES);
        }
        cred.check(&attr, Access::EXEC)?;
        // execve(2): "ETXTBSY The specified executable was open for
        // writing by one or more processes."
        if loc.mount.sb.has_text_writers(loc.ino) {
            return Err(Errno::ETXTBSY);
        }
        let file = OpenFile::new(loc.pin()?, attr.kind, Backend::Inode, OpenFlags::RDONLY).with_origin(origin);
        Ok(Arc::new(file.denying_write()))
    }

    /// `mkdir(2)`: `mode` is masked by the umask.
    pub fn mkdir(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], mode: u16) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = self.new_name(&p)?;
        let dir_attr = self.may_change(cred, &p.dir)?;
        let (uid, gid, perm) = Self::new_owner(cred, &dir_attr, mode & !ctx.umask & 0o1777, true);
        self.name_changed(p.dir.mount.sb.fs.mkdir(p.dir.ino, name, perm, uid, gid))?;
        inotify::entry_event(&p.dir, name, inotify::mask::IN_CREATE, true, 0);
        Ok(())
    }

    /// `mknod(2)`: regular files, FIFOs and sockets for anyone with write
    /// permission on the directory, device nodes only with `CAP_MKNOD`
    /// (`EPERM`);
    /// directories and symbolic links are `EINVAL`.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn mknod(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        kind: FileType,
        mode: u16,
        rdev: DeviceId,
    ) -> Result<()> {
        if matches!(kind, FileType::Directory | FileType::Symlink) {
            return Err(Errno::EINVAL);
        }
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = self.new_name(&p)?;
        if p.trailing_slash {
            return Err(Errno::ENOENT);
        }
        let dir_attr = self.may_change(cred, &p.dir)?;
        // A device node needs CAP_MKNOD, checked after the lookup and the
        // directory's permission (mknod's spec sheet).
        if matches!(kind, FileType::CharDevice | FileType::BlockDevice) && !cred.has(cap::MKNOD) {
            return Err(Errno::EPERM);
        }
        let (uid, gid, perm) = Self::new_owner(cred, &dir_attr, mode & !ctx.umask, false);
        let rdev =
            if matches!(kind, FileType::CharDevice | FileType::BlockDevice) { rdev } else { DeviceId::default() };
        self.name_changed(p.dir.mount.sb.fs.create(p.dir.ino, name, &NewNode { kind, perm, uid, gid, rdev }))?;
        inotify::entry_event(&p.dir, name, inotify::mask::IN_CREATE, false, 0);
        Ok(())
    }

    /// `symlink(2)`: a link at `path` to `target` (stored as given).
    pub fn symlink(&self, ctx: &FsContext, cred: &Cred, target: &[u8], at: At<'_>, path: &[u8]) -> Result<()> {
        if target.is_empty() {
            return Err(Errno::ENOENT);
        }
        if target.len() >= PATH_MAX {
            return Err(Errno::ENAMETOOLONG);
        }
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = match self.new_name(&p) {
            // "file/": a name that exists used as a directory is ENOTDIR
            // (a link to one is followed), anything else there EEXIST.
            Err(Errno::EEXIST) if p.trailing_slash => {
                return match self.resolve(ctx, cred, at, path, true) {
                    Err(Errno::ENOTDIR) => Err(Errno::ENOTDIR),
                    _ => Err(Errno::EEXIST),
                };
            }
            r => r?,
        };
        if p.trailing_slash {
            return Err(Errno::ENOENT);
        }
        let dir_attr = self.may_change(cred, &p.dir)?;
        let (uid, gid, _) = Self::new_owner(cred, &dir_attr, 0o777, false);
        self.name_changed(p.dir.mount.sb.fs.symlink(p.dir.ino, name, target, uid, gid))?;
        inotify::entry_event(&p.dir, name, inotify::mask::IN_CREATE, false, 0);
        Ok(())
    }

    /// `linkat(2)`: a new name `new` for what `old` names. `follow` is
    /// `AT_SYMLINK_FOLLOW` (`link(2)` itself does not follow on Linux).
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn link(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        old_at: At<'_>,
        old: &[u8],
        new_at: At<'_>,
        new: &[u8],
        follow: bool,
    ) -> Result<()> {
        self.link_paths(ctx, cred, PathArg::new(old_at, old), PathArg::new(new_at, new), follow)
    }

    /// [`link`](Self::link) with each path's errors in their place
    /// (linkat's `path_order_two_paths`): `old` is resolved completely
    /// first (its string, start and every component, the last one
    /// included), so any error of it wins over every error of `new`; then
    /// `new` (string, start, components); then the new name's own errors
    /// (`EEXIST`, ...).
    pub fn link_paths(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        old: PathArg<'_>,
        new: PathArg<'_>,
        follow: bool,
    ) -> Result<()> {
        let (path, at) = old.begin()?;
        let src = self.walk(ctx, cred).resolve(Self::start(ctx, at), path, follow)?;
        self.link_location(ctx, cred, src, new, true)
    }

    /// `linkat(2)` with `AT_EMPTY_PATH`: a new name `new` for the file
    /// `src` (an open file's node), which may be an `O_TMPFILE` file that
    /// has no name yet; another file without names is `ENOENT`. `new`'s
    /// errors come after whatever the caller found out about `src`.
    pub fn link_node(&self, ctx: &FsContext, cred: &Cred, src: &NodeRef, new: PathArg<'_>) -> Result<()> {
        self.link_location(ctx, cred, src.loc().clone(), new, true)
    }

    /// [`link_node`](Self::link_node) for the node `file` is open on; an
    /// `O_TMPFILE | O_EXCL` file is `ENOENT`, as a file without names.
    pub fn link_file(&self, ctx: &FsContext, cred: &Cred, file: &OpenFile, new: PathArg<'_>) -> Result<()> {
        self.link_location(ctx, cred, file.node().loc().clone(), new, !file.is_unnameable())
    }

    pub(super) fn link_location(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        src: Location,
        new: PathArg<'_>,
        nameable: bool,
    ) -> Result<()> {
        let p = self.parent_arg(ctx, cred, new)?;
        let name = self.new_name(&p)?;
        if p.trailing_slash {
            return Err(Errno::ENOENT);
        }
        if !Arc::ptr_eq(&src.mount, &p.dir.mount) {
            return Err(Errno::EXDEV);
        }
        // A directory is EPERM before the new parent's permissions
        // (link("/", "/x") by a normal user); the protected_hardlinks EPERM
        // comes after EROFS and EACCES (the spec of link: `order`).
        let attr = src.attr()?;
        if attr.kind == FileType::Directory {
            return Err(Errno::EPERM);
        }
        self.may_change(cred, &p.dir)?;
        if !Self::protected_hardlinks_allow(cred, &attr) {
            return Err(Errno::EPERM);
        }
        if !nameable {
            return Err(Errno::ENOENT);
        }
        self.name_changed(p.dir.mount.sb.fs.link(p.dir.ino, name, src.ino))?;
        // inotify(7): the link count changed; a new name appeared.
        inotify::self_event(&src, inotify::mask::IN_ATTRIB, false);
        inotify::entry_event(&p.dir, name, inotify::mask::IN_CREATE, false, 0);
        Ok(())
    }

    /// The `fs.protected_hardlinks = 1` rule (the default; `proc(5)`,
    /// `/proc/sys/fs/protected_hardlinks`): a caller without `CAP_FOWNER`
    /// who does not own the file may only link a regular file it may both
    /// read and write and that is neither set-user-ID nor set-group-ID
    /// with group execute.
    pub(super) fn protected_hardlinks_allow(cred: &Cred, attr: &Attr) -> bool {
        if cred.has(cap::FOWNER) || cred.uid == attr.uid {
            return true;
        }
        let setid = attr.perm & S_ISUID != 0 || (attr.perm & S_ISGID != 0 && attr.perm & S_IXGRP != 0);
        attr.kind == FileType::Regular && !setid && cred.check(attr, Access::READ | Access::WRITE).is_ok()
    }
}
