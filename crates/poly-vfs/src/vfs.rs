//! The VFS: mount table, device registry and path-based operations.

use crate::cred::{Access, Cred, cap};
use crate::device::CharDevice;
use crate::fdtable::At;
use crate::file::{Backend, OpenFile, OpenFlags, Origin};
use crate::fs::{FileSystem, NewNode, RenameFlags};
use crate::mount::{Mount, MountFlags, MountId, MountInfo, MountTable};
use crate::node::{FsContext, Location, NodeRef, is_fs_object};
use crate::object::AnonFs;
use crate::path::{Found, Last, PATH_MAX, Parent, PathArg, Walk, check_path};
use crate::sync::SpinMutex;
use crate::{Attr, DeviceId, Errno, FileType, Ino, PERM_MASK, Result, S_ISGID, S_ISVTX, Stat};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The virtual file system of one machine: what is mounted where, which
/// character devices exist, and every path-based operation.
///
/// Methods take the caller's [`FsContext`] (root, working directory,
/// umask) and [`Cred`], and an [`At`] saying where a relative path starts.
pub struct Vfs {
    pub(crate) mounts: SpinMutex<MountTable>,
    devices: SpinMutex<BTreeMap<DeviceId, Arc<dyn CharDevice>>>,
    /// The inodes of file objects (pipes, event files, epoll sets), on a
    /// mount no path reaches (`object.rs`).
    anon: (Arc<AnonFs>, Arc<Mount>),
    /// The pipes of open FIFO inodes (`fifo.rs`).
    fifos: crate::fifo::Fifos,
    /// Counts every change of a name ([`namespace_generation`](Self::namespace_generation)).
    names_changed: core::sync::atomic::AtomicU64,
}

impl core::fmt::Debug for Vfs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vfs").field("mounts", &self.mounts.lock().mounts).finish_non_exhaustive()
    }
}

impl Vfs {
    /// A VFS with `root` mounted at `/`.
    pub fn new(root: Arc<dyn FileSystem>) -> Vfs {
        Vfs::with_root_flags(root, MountFlags::default())
    }

    pub fn with_root_flags(root: Arc<dyn FileSystem>, flags: MountFlags) -> Vfs {
        let mut table = MountTable::new();
        let root_ino = root.root();
        let info = root.info();
        let sb = table.superblock(root, info);
        table.add(sb, root_ino, None, flags);
        let anon_fs = Arc::new(AnonFs::default());
        let anon_info = anon_fs.info();
        let anon_sb = table.superblock(anon_fs.clone(), anon_info);
        let anon_mount = table.detached(anon_sb, anon_fs.root());
        Vfs {
            mounts: SpinMutex::new(table),
            devices: SpinMutex::new(BTreeMap::new()),
            anon: (anon_fs, anon_mount),
            fifos: crate::fifo::Fifos::default(),
            names_changed: core::sync::atomic::AtomicU64::new(0),
        }
    }

    /// A number that grows with every change of a name anywhere in this VFS
    /// (a file, directory, node or link made, a name removed, a rename),
    /// and never otherwise: a cache of directory contents made at one value
    /// is still right while it has not moved (the NT personality's
    /// case-insensitive lookup, ADR 0091).
    pub fn namespace_generation(&self) -> u64 {
        self.names_changed.load(core::sync::atomic::Ordering::Acquire)
    }

    /// Count a change of a name if `r` says it happened.
    fn name_changed<T>(&self, r: Result<T>) -> Result<T> {
        if r.is_ok() {
            self.names_changed.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
        }
        r
    }

    /// A new inode for a file object (`kind` and `perm` as `fstat` will
    /// report them, owned by `cred`), pinned by the returned reference and
    /// gone with its last one. Open files on the object take clones
    /// ([`OpenFile::on_object`](crate::OpenFile::on_object)); a pipe's two ends
    /// share one inode.
    pub fn anon_node(&self, kind: FileType, perm: u16, cred: &Cred) -> Result<NodeRef> {
        let (fs, mount) = &self.anon;
        let ino = fs.alloc(kind, perm & PERM_MASK, cred.uid, cred.gid);
        Location { mount: mount.clone(), ino }.pin()
    }

    #[cfg(test)]
    pub(crate) fn anon_inodes(&self) -> usize {
        self.anon.0.live()
    }

    /// A context with root and working directory at `/` and the default
    /// umask: what the first process starts with.
    pub fn root_context(&self) -> Result<FsContext> {
        let first = self.mounts.lock().mounts[0].clone();
        let root = Location { ino: first.root, mount: first }.pin()?;
        Ok(FsContext { cwd: root.clone(), root, umask: FsContext::DEFAULT_UMASK })
    }

    // ----- devices -----

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
        for _ in 0..=table.mounts.len() {
            match table.covering(loc.mount.id, loc.ino) {
                Some(m) => loc = Location { mount: m.clone(), ino: m.root },
                None => break,
            }
        }
        loc
    }

    /// Looks `name` up in `dir`, without following mounts.
    fn raw_child(&self, dir: &Location, name: &[u8]) -> Result<Location> {
        let ino = dir.mount.sb.lookup(dir.ino, name)?;
        Ok(Location { mount: dir.mount.clone(), ino })
    }

    /// Looks `name` up in `dir` and follows mounts on the result.
    pub(crate) fn child(&self, dir: &Location, name: &[u8]) -> Result<Location> {
        Ok(self.follow_mounts(self.raw_child(dir, name)?))
    }

    fn is_mount_point(&self, loc: &Location) -> bool {
        self.mounts.lock().covering(loc.mount.id, loc.ino).is_some()
    }

    fn start(ctx: &FsContext, at: At<'_>) -> Location {
        match at {
            At::Cwd => ctx.cwd.loc().clone(),
            At::Node(n) => n.loc().clone(),
        }
    }

    fn walk<'a>(&'a self, ctx: &FsContext, cred: &'a Cred) -> Walk<'a> {
        Walk::new(self, cred, ctx.root.loc().clone())
    }

    fn parent_of(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Parent> {
        self.parent_arg(ctx, cred, PathArg::new(at, path))
    }

    /// Steps 1 to 3 of the order of path errors (`path.rs`) for everything
    /// but the last name's own length: the string, the start, every
    /// directory on the way.
    fn dirs_of(&self, ctx: &FsContext, cred: &Cred, arg: PathArg<'_>) -> Result<Parent> {
        let (path, at) = arg.begin()?;
        self.walk(ctx, cred).dirs(Self::start(ctx, at), path)
    }

    /// [`dirs_of`](Self::dirs_of) and the last name's length.
    fn parent_arg(&self, ctx: &FsContext, cred: &Cred, arg: PathArg<'_>) -> Result<Parent> {
        let p = self.dirs_of(ctx, cred, arg)?;
        p.check_name()?;
        Ok(p)
    }

    fn resolve(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<Location> {
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
    fn may_change(&self, cred: &Cred, dir: &Location) -> Result<Attr> {
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
    fn sticky_allows(cred: &Cred, dir: &Attr, victim: &Attr) -> Result<()> {
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
    fn new_owner(cred: &Cred, dir: &Attr, perm: u16, is_dir: bool) -> (u32, u32, u16) {
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
    fn new_name<'p>(&self, p: &'p Parent) -> Result<&'p [u8]> {
        let Last::Name(name) = &p.last else { return Err(Errno::EEXIST) };
        match self.raw_child(&p.dir, name) {
            Ok(_) => Err(Errno::EEXIST),
            Err(Errno::ENOENT) => Ok(name),
            Err(e) => Err(e),
        }
    }

    // ----- creating -----

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
        let mut walk = self.walk(ctx, cred);
        let start = Self::start(ctx, at);
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
            // open(2): "EPERM The O_NOATIME flag was specified, but the
            // effective user ID of the caller did not match the owner of
            // the file and the caller was not privileged" (CAP_FOWNER).
            if flags.contains(OpenFlags::NOATIME) && !cred.owns(&attr) {
                return Err(Errno::EPERM);
            }
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
        Ok(Arc::new(OpenFile::new(node, attr.kind, backend, flags).with_origin(origin)))
    }

    /// `O_TMPFILE` (open(2)): an unnamed regular file in the directory
    /// `path` names, open for writing (`EINVAL` without write access or
    /// with `O_CREAT`; `ENOTDIR` if `path` is not a directory), with
    /// `mode & ~umask`; it has no links and goes when closed, unless
    /// `linkat` with `AT_EMPTY_PATH` names it, which `O_EXCL` rules out.
    /// `F_GETFL` reports `O_TMPFILE`.
    fn open_tmpfile(
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
    fn open_creat(
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
            match self.child(&p.dir, name) {
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
                    let mut target = walk.link_target(&loc)?;
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
        check_path(path)?;
        let (loc, named) = self.walk(ctx, cred).resolve_named(Self::start(ctx, at), path, true)?;
        let origin = named.map(|(dir, name)| Origin {
            mount: Arc::downgrade(&dir.mount),
            dir: dir.ino,
            name: name.into_boxed_slice(),
        });
        let attr = loc.attr()?;
        if attr.kind != FileType::Regular || loc.mount.flags().no_exec {
            return Err(Errno::EACCES);
        }
        cred.check(&attr, Access::EXEC)?;
        Ok(Arc::new(OpenFile::new(loc.pin()?, attr.kind, Backend::Inode, OpenFlags::RDONLY).with_origin(origin)))
    }

    /// `mkdir(2)`: `mode` is masked by the umask.
    pub fn mkdir(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], mode: u16) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = self.new_name(&p)?;
        let dir_attr = self.may_change(cred, &p.dir)?;
        let (uid, gid, perm) = Self::new_owner(cred, &dir_attr, mode & !ctx.umask & 0o1777, true);
        self.name_changed(p.dir.mount.sb.fs.mkdir(p.dir.ino, name, perm, uid, gid)).map(|_| ())
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
        match kind {
            FileType::Directory | FileType::Symlink => return Err(Errno::EINVAL),
            FileType::CharDevice | FileType::BlockDevice if !cred.has(cap::MKNOD) => return Err(Errno::EPERM),
            _ => {}
        }
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = self.new_name(&p)?;
        if p.trailing_slash {
            return Err(Errno::ENOENT);
        }
        let dir_attr = self.may_change(cred, &p.dir)?;
        let (uid, gid, perm) = Self::new_owner(cred, &dir_attr, mode & !ctx.umask, false);
        let rdev =
            if matches!(kind, FileType::CharDevice | FileType::BlockDevice) { rdev } else { DeviceId::default() };
        self.name_changed(p.dir.mount.sb.fs.create(p.dir.ino, name, &NewNode { kind, perm, uid, gid, rdev }))
            .map(|_| ())
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
        self.name_changed(p.dir.mount.sb.fs.symlink(p.dir.ino, name, target, uid, gid)).map(|_| ())
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

    fn link_location(
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
        if src.attr()?.kind == FileType::Directory {
            return Err(Errno::EPERM);
        }
        self.may_change(cred, &p.dir)?;
        if !nameable {
            return Err(Errno::ENOENT);
        }
        self.name_changed(p.dir.mount.sb.fs.link(p.dir.ino, name, src.ino))
    }

    // ----- removing and renaming -----

    /// `unlink(2)`.
    pub fn unlink(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let Last::Name(name) = &p.last else { return Err(Errno::EISDIR) };
        let victim = self.raw_child(&p.dir, name)?;
        let vattr = victim.attr()?;
        if vattr.kind == FileType::Directory {
            return Err(Errno::EISDIR);
        }
        if p.trailing_slash {
            return Err(Errno::ENOTDIR);
        }
        if self.is_mount_point(&victim) {
            return Err(Errno::EBUSY);
        }
        let dattr = self.may_change(cred, &p.dir)?;
        Self::sticky_allows(cred, &dattr, &vattr)?;
        let sb = &p.dir.mount.sb;
        sb.forget(p.dir.ino, name);
        self.name_changed(sb.fs.unlink(p.dir.ino, name))
    }

    /// `rmdir(2)`: `/` is `EBUSY`, a final `.` is `EINVAL`, `..` is
    /// `ENOTEMPTY`, a mount point or the caller's root is `EBUSY`.
    pub fn rmdir(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = match &p.last {
            Last::Root => return Err(Errno::EBUSY),
            Last::Dot => return Err(Errno::EINVAL),
            Last::DotDot => return Err(Errno::ENOTEMPTY),
            Last::Name(name) => name,
        };
        let victim = self.raw_child(&p.dir, name)?;
        let vattr = victim.attr()?;
        if vattr.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        if self.is_mount_point(&victim) || victim.same(ctx.root.loc()) {
            return Err(Errno::EBUSY);
        }
        let dattr = self.may_change(cred, &p.dir)?;
        Self::sticky_allows(cred, &dattr, &vattr)?;
        let sb = &p.dir.mount.sb;
        sb.forget(p.dir.ino, name);
        self.name_changed(sb.fs.rmdir(p.dir.ino, name))
    }

    /// `renameat2(2)`.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn rename(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        old_at: At<'_>,
        old: &[u8],
        new_at: At<'_>,
        new: &[u8],
        flags: RenameFlags,
    ) -> Result<()> {
        self.rename_paths(ctx, cred, PathArg::new(old_at, old), PathArg::new(new_at, new), flags)
    }

    /// [`rename`](Self::rename) with each path's errors in their place
    /// (renameat2's `path_order_two_paths`): the directories first, the
    /// last names after them. (1) `old`'s string, start and every component
    /// but the last; (2) the same for `new`; (3) `old`'s last name (over
    /// [`NAME_MAX`](crate::NAME_MAX) bytes `ENAMETOOLONG`, missing
    /// `ENOENT`); (4) `new`'s last name (`ENAMETOOLONG`); then the errors of
    /// the rename itself (`EXDEV`, `EISDIR`, ...). A last component that is
    /// `/`, `.` or `..` on either side is `EBUSY`, before (3).
    pub fn rename_paths(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        old: PathArg<'_>,
        new: PathArg<'_>,
        flags: RenameFlags,
    ) -> Result<()> {
        if flags.noreplace && flags.exchange {
            return Err(Errno::EINVAL);
        }
        let op = self.dirs_of(ctx, cred, old)?;
        let np = self.dirs_of(ctx, cred, new)?;
        let (Last::Name(old_name), Last::Name(new_name)) = (&op.last, &np.last) else {
            return Err(Errno::EBUSY);
        };
        op.check_name()?;
        let src = self.raw_child(&op.dir, old_name)?;
        np.check_name()?;
        if !Arc::ptr_eq(&op.dir.mount, &np.dir.mount) {
            return Err(Errno::EXDEV);
        }
        let sattr = src.attr()?;
        let dst = match self.raw_child(&np.dir, new_name) {
            Ok(d) => Some(d),
            Err(Errno::ENOENT) if !flags.exchange => None,
            Err(e) => return Err(e),
        };
        let dattr = dst.as_ref().map(Location::attr).transpose()?;
        let src_dir = sattr.kind == FileType::Directory;
        let dst_dir = dattr.is_some_and(|a| a.kind == FileType::Directory);
        // A trailing slash demands a directory on that side (for the
        // target: the source, unless exchanging).
        if (op.trailing_slash && !src_dir) || (np.trailing_slash && !(if flags.exchange { dst_dir } else { src_dir })) {
            return Err(Errno::ENOTDIR);
        }
        if self.is_mount_point(&src) || dst.as_ref().is_some_and(|d| self.is_mount_point(d)) {
            return Err(Errno::EBUSY);
        }
        if let Some(dattr) = &dattr {
            // RENAME_NOREPLACE: an existing target, even the source's own
            // name or another link of it, is EEXIST (the spec of renameat2).
            if flags.noreplace {
                return Err(Errno::EEXIST);
            }
            if dattr.ino == sattr.ino {
                // Two names of one inode (or the same name): nothing to do.
                return Ok(());
            }
            if !flags.exchange {
                if src_dir && !dst_dir {
                    return Err(Errno::ENOTDIR);
                }
                if !src_dir && dst_dir {
                    return Err(Errno::EISDIR);
                }
            }
        }
        // A directory cannot move below itself.
        if src_dir && self.is_within(&np.dir, src.ino)? {
            return Err(Errno::EINVAL);
        }
        if flags.exchange && dst_dir && self.is_within(&op.dir, dattr.as_ref().map_or(0, |a| a.ino))? {
            return Err(Errno::EINVAL);
        }
        let old_dattr = self.may_change(cred, &op.dir)?;
        let new_dattr = self.may_change(cred, &np.dir)?;
        Self::sticky_allows(cred, &old_dattr, &sattr)?;
        if let Some(dattr) = &dattr {
            Self::sticky_allows(cred, &new_dattr, dattr)?;
        }
        // Moving a directory to another parent rewrites its "..", which
        // needs write permission on it (`rename(2)`, EACCES).
        let moves_parent = op.dir.ino != np.dir.ino;
        if moves_parent && src_dir {
            cred.check(&sattr, Access::WRITE)?;
        }
        if moves_parent && flags.exchange && dst_dir {
            cred.check(dattr.as_ref().ok_or(Errno::ENOENT)?, Access::WRITE)?;
        }
        let sb = &op.dir.mount.sb;
        sb.forget(op.dir.ino, old_name);
        sb.forget(np.dir.ino, new_name);
        self.name_changed(sb.fs.rename(op.dir.ino, old_name, np.dir.ino, new_name, flags))
    }

    /// Whether `dir` is the directory `ancestor` or below it, within one
    /// file system (walking `..` up to the file system's root).
    fn is_within(&self, dir: &Location, ancestor: Ino) -> Result<bool> {
        let fs = &dir.mount.sb.fs;
        let root = fs.root();
        let mut cur = dir.ino;
        // Bounded so a corrupt file system cannot loop us forever.
        for _ in 0..1 << 16 {
            if cur == ancestor {
                return Ok(true);
            }
            if cur == root {
                return Ok(false);
            }
            let up = fs.lookup(cur, b"..")?;
            if up == cur {
                return Ok(false);
            }
            cur = up;
        }
        Err(Errno::ELOOP)
    }

    // ----- inspecting and changing attributes -----

    /// `stat(2)` (`follow`) and `lstat(2)`.
    pub fn stat(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<Stat> {
        let loc = self.resolve(ctx, cred, at, path, follow)?;
        Ok(Stat::new(loc.mount.dev(), &loc.attr()?))
    }

    /// `readlink(2)`: `EINVAL` unless `path` names a symbolic link.
    pub fn readlink(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Vec<u8>> {
        let loc = self.resolve(ctx, cred, at, path, false)?;
        loc.mount.sb.fs.readlink(loc.ino)
    }

    /// `faccessat(2)`. `cred` should hold the real ids unless the caller
    /// asked for `AT_EACCESS`.
    pub fn access(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        want: Access,
        follow: bool,
    ) -> Result<()> {
        self.lookup(ctx, cred, at, path, follow)?.access(cred, want)
    }

    /// `chmod(2)`.
    pub fn chmod(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], perm: u16, follow: bool) -> Result<()> {
        self.lookup(ctx, cred, at, path, follow)?.chmod(cred, perm)
    }

    /// `chown(2)` / `lchown(2)`; `None` leaves an id alone.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn chown(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        uid: Option<u32>,
        gid: Option<u32>,
        follow: bool,
    ) -> Result<()> {
        self.lookup(ctx, cred, at, path, follow)?.chown(cred, uid, gid)
    }

    /// `utimensat(2)` on a path: `None` leaves a time alone (`UTIME_OMIT`);
    /// see [`NodeRef::set_times`] for who may.
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn utimens(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        atime: Option<crate::SetTime>,
        mtime: Option<crate::SetTime>,
        follow: bool,
    ) -> Result<()> {
        self.lookup(ctx, cred, at, path, follow)?.set_times(cred, atime, mtime)
    }

    /// `truncate(2)`.
    pub fn truncate(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], size: u64) -> Result<()> {
        self.lookup(ctx, cred, at, path, true)?.truncate(cred, size)
    }

    // ----- the process context -----

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
    pub fn getcwd(&self, ctx: &FsContext) -> Result<Vec<u8>> {
        self.path_of(ctx, &ctx.cwd)
    }

    /// The absolute path of `node` as seen from `ctx`'s root, found by
    /// walking `..` and looking for each directory's name in its parent.
    /// `ENOENT` if the node has been removed or is not below the root, or
    /// if it is not a directory and has no name in the scanned parents.
    /// Meant for directories (getcwd, `/proc/self/cwd`).
    pub fn path_of(&self, ctx: &FsContext, node: &NodeRef) -> Result<Vec<u8>> {
        let cred = Cred::root();
        let walk = self.walk(ctx, &cred);
        let mut cur = self.follow_mounts(node.loc().clone());
        let mut names: Vec<Vec<u8>> = Vec::new();
        let mut total = 0;
        loop {
            if cur.same(walk.top_root()) || cur.same(ctx.root.loc()) {
                break;
            }
            if cur.is_mount_root() {
                match &cur.mount.parent {
                    Some((m, ino)) => {
                        // Name the mount point, not the mounted root.
                        cur = Location { mount: m.clone(), ino: *ino };
                        if cur.same(walk.top_root()) || cur.same(ctx.root.loc()) {
                            break;
                        }
                        if cur.is_mount_root() {
                            continue;
                        }
                    }
                    None => return Err(Errno::ENOENT),
                }
            }
            if cur.attr()?.nlink == 0 {
                return Err(Errno::ENOENT);
            }
            let parent = cur.mount.sb.fs.lookup(cur.ino, b"..")?;
            let name = Self::name_in(&cur.mount, parent, cur.ino)?;
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
        Ok(path)
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
            Self::name_in(dir.mount(), dir.ino(), node.ino()).ok()
        } else {
            None
        };
        path.extend_from_slice(name.as_deref().unwrap_or(&origin.name));
        if !linked {
            path.extend_from_slice(b" (deleted)");
        }
        Ok(path)
    }

    /// The name `child` has in directory `dir`.
    fn name_in(mount: &Mount, dir: Ino, child: Ino) -> Result<Vec<u8>> {
        let mut found = None;
        mount.sb.fs.readdir(dir, 0, &mut |e| {
            if e.ino == child && e.name != b"." && e.name != b".." {
                found = Some(e.name.to_vec());
                return false;
            }
            true
        })?;
        found.ok_or(Errno::ENOENT)
    }

    // ----- mounting -----

    /// `mount(2)`: puts `fs` on the directory `path` names (`CAP_SYS_ADMIN`,
    /// else `EPERM`; a directory, `ENOTDIR`).
    pub fn mount(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        fs: Arc<dyn FileSystem>,
        flags: MountFlags,
    ) -> Result<MountId> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let target = self.resolve(ctx, cred, at, path, true)?;
        if target.attr()?.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let root = fs.root();
        if fs.getattr(root)?.kind != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        let info = fs.info();
        let mut table = self.mounts.lock();
        let sb = table.superblock(fs, info);
        Ok(table.add(sb, root, Some((target.mount.clone(), target.ino)), flags).id)
    }

    /// A bind mount (`mount --bind`): what `source` names becomes visible at
    /// `target` too. Both must be directories, or both not (`ENOTDIR`).
    #[allow(clippy::too_many_arguments)] // the system call's own arguments
    pub fn bind(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        source_at: At<'_>,
        source: &[u8],
        target_at: At<'_>,
        target: &[u8],
        flags: MountFlags,
    ) -> Result<MountId> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let src = self.resolve(ctx, cred, source_at, source, true)?;
        let dst = self.resolve(ctx, cred, target_at, target, true)?;
        let src_dir = src.attr()?.kind == FileType::Directory;
        let dst_dir = dst.attr()?.kind == FileType::Directory;
        if src_dir != dst_dir {
            return Err(Errno::ENOTDIR);
        }
        let sb = src.mount.sb.clone();
        let mut table = self.mounts.lock();
        Ok(table.add(sb, src.ino, Some((dst.mount.clone(), dst.ino)), flags).id)
    }

    /// `umount2(2)`: `path` must be the root of a mount (`EINVAL`). Without
    /// `lazy`, the mount must have nothing mounted below it and nothing
    /// open, no working directory or root in it (`EBUSY`). With `lazy`
    /// (`MNT_DETACH`) it and everything below it are detached at once and
    /// live on for whoever still uses them. The first root cannot be
    /// unmounted (`EBUSY`).
    pub fn umount(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], lazy: bool) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = self.resolve(ctx, cred, at, path, true)?;
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        let mount = loc.mount.clone();
        drop(loc);
        if mount.parent.is_none() {
            return Err(Errno::EBUSY);
        }
        let mut table = self.mounts.lock();
        if !lazy {
            // References: the table's, ours, and one per child mount,
            // pinned node and walk in progress.
            if table.has_children(mount.id) || Arc::strong_count(&mount) > 2 {
                return Err(Errno::EBUSY);
            }
        }
        table.remove_tree(mount.id)
    }

    /// `mount(2)` with `MS_REMOUNT`: the mount whose root `path` names gets
    /// new per-mount flags (`CAP_SYS_ADMIN`, else `EPERM`; not the root of
    /// a mount, `EINVAL`). A file system that is read-only itself stays
    /// so whatever the flags say.
    pub fn remount(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], flags: MountFlags) -> Result<()> {
        if !cred.has(cap::SYS_ADMIN) {
            return Err(Errno::EPERM);
        }
        let loc = self.resolve(ctx, cred, at, path, true)?;
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        loc.mount.set_flags(flags);
        Ok(())
    }

    /// The mount whose root `path` names (`EINVAL` if `path` is not a
    /// mount point), as a `Vfs::mounts` entry: what `umount` callers check
    /// before unmounting a mount the kernel tracks itself (a disk).
    pub fn mount_at(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], follow: bool) -> Result<MountId> {
        let loc = self.resolve(ctx, cred, at, path, follow)?;
        if !loc.is_mount_root() {
            return Err(Errno::EINVAL);
        }
        Ok(loc.mount.id)
    }

    /// Where mount `id` is mounted, as an absolute path from `ctx`'s root
    /// (what `/proc/mounts` and `mount` show). `ENOENT` if there is no such
    /// mount or it cannot be reached from that root.
    pub fn mount_path(&self, ctx: &FsContext, id: MountId) -> Result<Vec<u8>> {
        let mount = self.mounts.lock().mounts.iter().find(|m| m.id == id).cloned().ok_or(Errno::ENOENT)?;
        let root = Location { ino: mount.root, mount }.pin()?;
        self.path_of(ctx, &root)
    }

    // ----- mount namespaces -----
    //
    // A mount namespace is a tree of mounts: the mounts below one first
    // mount (a mount without a mount point), which names it. The first
    // root's tree is the initial namespace. A copy duplicates every mount
    // of a tree, so mounting and unmounting in one copy does not show in
    // the others; the file systems (superblocks) are shared, so files are.

    /// The mount namespace `ctx`'s root is in.
    pub fn namespace_of(&self, ctx: &FsContext) -> MountId {
        crate::mount::base_of(&ctx.root.loc().mount)
    }

    /// Whether `ctx`'s root shows something else than its mount
    /// namespace's root (chroot(2) moved it). A root that is the
    /// namespace's first mount, or what is mounted over it, is not.
    pub fn is_chrooted(&self, ctx: &FsContext) -> bool {
        let mut first = ctx.root.loc().mount.clone();
        while let Some((p, _)) = first.parent.clone() {
            first = p;
        }
        let top = self.follow_mounts(Location { ino: first.root, mount: first });
        !self.follow_mounts(ctx.root.loc().clone()).same(&top)
    }

    /// `unshare(CLONE_NEWNS)`: copies the mount namespace `ctx` is in and
    /// moves `ctx`'s root and working directory to the same places in the
    /// copy (a working directory in another tree stays). Returns the new
    /// namespace. The flags of each mount are copied.
    pub fn copy_namespace(&self, ctx: &mut FsContext) -> Result<MountId> {
        let base = self.namespace_of(ctx);
        let mut copies: BTreeMap<MountId, Arc<Mount>> = BTreeMap::new();
        let new_base = {
            let mut table = self.mounts.lock();
            let mut new_base = None;
            for m in table.tree(base) {
                let parent = match &m.parent {
                    None => None,
                    // A parent that left the table (a lazy unmount) takes
                    // its subtree with it.
                    Some((p, ino)) => match copies.get(&p.id) {
                        Some(c) => Some((c.clone(), *ino)),
                        None => continue,
                    },
                };
                let is_base = parent.is_none();
                let copy = table.add(m.sb.clone(), m.root, parent, m.flags());
                if is_base {
                    new_base = Some(copy.id);
                }
                copies.insert(m.id, copy);
            }
            new_base.ok_or(Errno::EINVAL)?
        };
        let moved = |n: &NodeRef| -> Result<Option<NodeRef>> {
            match copies.get(&n.loc().mount.id) {
                Some(m) => Location { mount: m.clone(), ino: n.loc().ino }.pin().map(Some),
                None => Ok(None),
            }
        };
        let root = moved(&ctx.root)?;
        let cwd = moved(&ctx.cwd)?;
        if let Some(r) = root {
            ctx.root = r;
        }
        if let Some(c) = cwd {
            ctx.cwd = c;
        }
        Ok(new_base)
    }

    /// `setns(2)` into mount namespace `base`: `ctx`'s root and working
    /// directory become its root. `EINVAL` if there is no such namespace.
    pub fn enter_namespace(&self, ctx: &mut FsContext, base: MountId) -> Result<()> {
        let first = self
            .mounts
            .lock()
            .mounts
            .iter()
            .find(|m| m.id == base && m.parent.is_none())
            .cloned()
            .ok_or(Errno::EINVAL)?;
        let root = Location { ino: first.root, mount: first }.pin()?;
        ctx.cwd = root.clone();
        ctx.root = root;
        Ok(())
    }

    /// The last process left mount namespace `base`: its mounts go (files
    /// open on them keep them alive). The initial namespace stays
    /// (`EBUSY`).
    pub fn drop_namespace(&self, base: MountId) -> Result<()> {
        let mut table = self.mounts.lock();
        if table.mounts.first().is_some_and(|m| m.id == base) {
            return Err(Errno::EBUSY);
        }
        let doomed: Vec<MountId> = table.tree(base).iter().map(|m| m.id).collect();
        if doomed.is_empty() {
            return Err(Errno::EINVAL);
        }
        table.mounts.retain(|m| !doomed.contains(&m.id));
        Ok(())
    }

    /// The mounts of namespace `base`, in mount order.
    pub fn namespace_mounts(&self, base: MountId) -> Vec<MountInfo> {
        let ids: Vec<MountId> = self.mounts.lock().tree(base).iter().map(|m| m.id).collect();
        self.mounts().into_iter().filter(|m| ids.contains(&m.id)).collect()
    }

    /// Every mount, in mount order.
    pub fn mounts(&self) -> Vec<MountInfo> {
        self.mounts
            .lock()
            .mounts
            .iter()
            .map(|m| MountInfo {
                id: m.id,
                parent: m.parent.as_ref().map(|(p, ino)| (p.id, *ino)),
                fs_name: m.sb.info.name,
                dev: m.sb.dev,
                root: m.root,
                flags: m.flags(),
            })
            .collect()
    }
}
