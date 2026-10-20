//! The VFS: mount table, device registry and path-based operations.

use crate::cred::{Access, Cred};
use crate::device::CharDevice;
use crate::fdtable::At;
use crate::file::{Backend, OpenFile, OpenFlags};
use crate::fs::{FileSystem, NewNode, RenameFlags};
use crate::mount::{Mount, MountFlags, MountId, MountInfo, MountTable};
use crate::node::{FsContext, Location, NodeRef, is_fs_object};
use crate::path::{Last, PATH_MAX, Parent, Walk, check_path};
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
        Vfs { mounts: SpinMutex::new(table), devices: SpinMutex::new(BTreeMap::new()) }
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
        check_path(path)?;
        self.walk(ctx, cred).parent(Self::start(ctx, at), path)
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
    /// (`unlink(2)`, `rename(2)`: `EPERM`).
    fn sticky_allows(cred: &Cred, dir: &Attr, victim: &Attr) -> Result<()> {
        if dir.perm & S_ISVTX != 0 && !cred.is_root() && cred.uid != dir.uid && cred.uid != victim.uid {
            Err(Errno::EPERM)
        } else {
            Ok(())
        }
    }

    /// Owner, group and permission bits of a node created in a directory
    /// with attributes `dir`: the caller's ids, or the directory's group if
    /// it is set-group-ID (a new directory then inherits the bit), as
    /// `open(2)` and `mkdir(2)` describe for Linux.
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
        if !is_dir && perm & S_ISGID != 0 && !cred.is_root() && !cred.in_group(gid) {
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
            if flags.intersection(OpenFlags::ACCMODE) == OpenFlags::ACCMODE {
                return Err(Errno::EINVAL);
            }
            if flags.contains(OpenFlags::TMPFILE) {
                return Err(Errno::EOPNOTSUPP);
            }
            if flags.contains(OpenFlags::CREAT) && flags.contains(OpenFlags::DIRECTORY) {
                return Err(Errno::EINVAL);
            }
        }
        let follow = !flags.contains(OpenFlags::NOFOLLOW);
        check_path(path)?;
        let mut walk = self.walk(ctx, cred);
        let start = Self::start(ctx, at);
        let (loc, created) = if !flags.contains(OpenFlags::CREAT) {
            (walk.resolve(start, path, follow)?, false)
        } else {
            self.open_creat(&mut walk, ctx, start, path, flags, mode)?
        };
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
            return Ok(Arc::new(OpenFile::new(loc.pin()?, attr.kind, Backend::Path, flags)));
        }
        let write = flags.writable();
        if attr.kind == FileType::Directory && (write || flags.contains(OpenFlags::CREAT)) {
            return Err(Errno::EISDIR);
        }
        if !created {
            if (write || flags.contains(OpenFlags::TRUNC)) && is_fs_object(attr.kind) && loc.mount.read_only() {
                return Err(Errno::EROFS);
            }
            let mut want = Access::NONE;
            if flags.readable() {
                want = want | Access::READ;
            }
            if write || flags.contains(OpenFlags::TRUNC) {
                want = want | Access::WRITE;
            }
            cred.check(&attr, want)?;
        }
        let backend = match attr.kind {
            FileType::Regular | FileType::Directory => Backend::Inode,
            FileType::CharDevice => {
                if loc.mount.flags.no_dev {
                    return Err(Errno::EACCES);
                }
                Backend::Device(self.char_device(attr.rdev).ok_or(Errno::ENXIO)?)
            }
            // Block devices are reached through the block layer, FIFOs come
            // with pipes (Phase 4), and a socket's name cannot be opened
            // (`open(2)`: ENXIO).
            FileType::BlockDevice | FileType::Fifo | FileType::Socket | FileType::Symlink => {
                return Err(Errno::ENXIO);
            }
        };
        let node = loc.pin()?;
        if flags.contains(OpenFlags::TRUNC) && attr.kind == FileType::Regular && !created && attr.size != 0 {
            node.truncate_unchecked(0)?;
        }
        Ok(Arc::new(OpenFile::new(node, attr.kind, backend, flags)))
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
    ) -> Result<(Location, bool)> {
        let mut p = walk.parent(start, path)?;
        loop {
            let Last::Name(name) = &p.last else { return Err(Errno::EISDIR) };
            match self.child(&p.dir, name) {
                Ok(_) if flags.contains(OpenFlags::EXCL) => return Err(Errno::EEXIST),
                Ok(loc) => {
                    if loc.attr()?.kind != FileType::Symlink {
                        if p.trailing_slash && loc.attr()?.kind != FileType::Directory {
                            return Err(Errno::ENOTDIR);
                        }
                        return Ok((loc, false));
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
                    let ino = p.dir.mount.sb.fs.create(p.dir.ino, name, &node)?;
                    return Ok((Location { mount: p.dir.mount.clone(), ino }, true));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Opens a file for `execve`: a regular file (`EACCES` otherwise) the
    /// caller may execute, not on a `noexec` mount (`EACCES`).
    pub fn open_exec(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<Arc<OpenFile>> {
        let loc = self.resolve(ctx, cred, at, path, true)?;
        let attr = loc.attr()?;
        if attr.kind != FileType::Regular || loc.mount.flags.no_exec {
            return Err(Errno::EACCES);
        }
        cred.check(&attr, Access::EXEC)?;
        Ok(Arc::new(OpenFile::new(loc.pin()?, attr.kind, Backend::Inode, OpenFlags::RDONLY)))
    }

    /// `mkdir(2)`: `mode` is masked by the umask.
    pub fn mkdir(&self, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8], mode: u16) -> Result<()> {
        let p = self.parent_of(ctx, cred, at, path)?;
        let name = self.new_name(&p)?;
        let dir_attr = self.may_change(cred, &p.dir)?;
        let (uid, gid, perm) = Self::new_owner(cred, &dir_attr, mode & !ctx.umask & 0o1777, true);
        p.dir.mount.sb.fs.mkdir(p.dir.ino, name, perm, uid, gid).map(|_| ())
    }

    /// `mknod(2)`: regular files, FIFOs and sockets for anyone with write
    /// permission on the directory, device nodes for root only (`EPERM`);
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
            FileType::CharDevice | FileType::BlockDevice if !cred.is_root() => return Err(Errno::EPERM),
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
        p.dir.mount.sb.fs.create(p.dir.ino, name, &NewNode { kind, perm, uid, gid, rdev }).map(|_| ())
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
        let name = self.new_name(&p)?;
        if p.trailing_slash {
            return Err(Errno::ENOENT);
        }
        let dir_attr = self.may_change(cred, &p.dir)?;
        let (uid, gid, _) = Self::new_owner(cred, &dir_attr, 0o777, false);
        p.dir.mount.sb.fs.symlink(p.dir.ino, name, target, uid, gid).map(|_| ())
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
        let src = self.resolve(ctx, cred, old_at, old, follow)?;
        let p = self.parent_of(ctx, cred, new_at, new)?;
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
        p.dir.mount.sb.fs.link(p.dir.ino, name, src.ino)
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
        sb.fs.unlink(p.dir.ino, name)
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
        sb.fs.rmdir(p.dir.ino, name)
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
        if flags.noreplace && flags.exchange {
            return Err(Errno::EINVAL);
        }
        let op = self.parent_of(ctx, cred, old_at, old)?;
        let np = self.parent_of(ctx, cred, new_at, new)?;
        let (Last::Name(old_name), Last::Name(new_name)) = (&op.last, &np.last) else {
            return Err(Errno::EBUSY);
        };
        if !Arc::ptr_eq(&op.dir.mount, &np.dir.mount) {
            return Err(Errno::EXDEV);
        }
        let src = self.raw_child(&op.dir, old_name)?;
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
            if dattr.ino == sattr.ino {
                // Two names of one inode (or the same name): nothing to do.
                return Ok(());
            }
            if flags.noreplace {
                return Err(Errno::EEXIST);
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
        sb.fs.rename(op.dir.ino, old_name, np.dir.ino, new_name, flags)
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

    /// `mount(2)`: puts `fs` on the directory `path` names (root only,
    /// `EPERM`; a directory, `ENOTDIR`).
    pub fn mount(
        &self,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
        fs: Arc<dyn FileSystem>,
        flags: MountFlags,
    ) -> Result<MountId> {
        if !cred.is_root() {
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
        if !cred.is_root() {
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
        if !cred.is_root() {
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

    /// Where mount `id` is mounted, as an absolute path from `ctx`'s root
    /// (what `/proc/mounts` and `mount` show). `ENOENT` if there is no such
    /// mount or it cannot be reached from that root.
    pub fn mount_path(&self, ctx: &FsContext, id: MountId) -> Result<Vec<u8>> {
        let mount = self.mounts.lock().mounts.iter().find(|m| m.id == id).cloned().ok_or(Errno::ENOENT)?;
        let root = Location { ino: mount.root, mount }.pin()?;
        self.path_of(ctx, &root)
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
                flags: m.flags,
            })
            .collect()
    }
}
