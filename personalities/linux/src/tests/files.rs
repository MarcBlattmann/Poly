//! The file system calls (openat, stat, getdents64, fcntl, ...) and the
//! resource limits, against a [`LinuxFs`] over a real `poly-vfs` tmpfs.

extern crate std;

use super::fake::*;
use crate::abi::{le_u64, makedev};
use crate::fs::{
    DirEntry, FileStatus, LinuxFs, LockHolder, StatFs, SysInfo, TimeChange, XattrTarget, at::AT_FDCWD, dirent64_len,
    encode_dirent64, major, minor, rlimit::*,
};
use crate::kernel::Target;
use crate::rlimit::Rlimit;
use crate::{Errno, nr};
use poly_vfs::locks::{LockKind, LockOwner};
use poly_vfs::{At, Cred, FdTable, FsContext, OpenFlags, PathArg, RenameFlags, Tmpfs, Vfs, Whence};
use std::sync::{Arc, Mutex};
use std::vec::Vec;

#[derive(Debug, Default)]
struct Clock;

impl poly_vfs::Clock for Clock {
    fn now(&self) -> poly_vfs::Timespec {
        poly_vfs::Timespec::new(1000, 0)
    }
}

fn errno(e: poly_vfs::Errno) -> Errno {
    Errno::new(e as u16).unwrap()
}

fn vfs_errno(e: Errno) -> poly_vfs::Errno {
    poly_vfs::Errno::from_code(i32::from(e.get())).unwrap()
}

/// [`VfsFs::start`] as where a relative path starts.
fn at_of(start: &Result<Option<poly_vfs::NodeRef>, Errno>) -> poly_vfs::Result<At<'_>> {
    match start {
        Ok(node) => Ok(node.as_ref().map_or(At::Cwd, At::Node)),
        Err(e) => Err(vfs_errno(*e)),
    }
}

/// `LinuxFs` the way the kernel implements it, over a VFS, one context and
/// one descriptor table.
struct VfsFs {
    vfs: Arc<Vfs>,
    ctx: FsContext,
    fds: FdTable,
    cred: Cred,
    /// set_fd_limit calls.
    limits: Arc<Mutex<Vec<(Target, u64)>>>,
    synced: Arc<Mutex<u32>>,
    /// Advisory locks, owned by process 1 or by descriptions.
    locks: poly_vfs::locks::LockTable,
}

impl VfsFs {
    fn node(&self, fd: i32) -> Result<poly_vfs::NodeRef, Errno> {
        if fd == AT_FDCWD {
            return Ok(self.ctx.cwd().clone());
        }
        self.fds.get(fd).map(|f| f.node().clone()).map_err(errno)
    }

    /// The node a relative path from `dirfd` starts at (`None`: the working
    /// directory), or `EBADF`.
    fn start(&self, dirfd: i32) -> Result<Option<poly_vfs::NodeRef>, Errno> {
        if dirfd == AT_FDCWD { Ok(None) } else { self.node(dirfd).map(Some) }
    }

    fn at<R>(&self, dirfd: i32, path: &[u8], f: impl FnOnce(&Vfs, At<'_>) -> poly_vfs::Result<R>) -> Result<R, Errno> {
        if dirfd == AT_FDCWD || path.first() == Some(&b'/') {
            f(&self.vfs, At::Cwd).map_err(errno)
        } else {
            let node = self.node(dirfd)?;
            f(&self.vfs, At::Node(&node)).map_err(errno)
        }
    }

    /// The lock table's file and owner for `fd`: process 1, or (`ofd`) the
    /// open file description.
    fn lock_key(&self, fd: i32, ofd: bool) -> Result<(poly_vfs::locks::FileKey, LockOwner), Errno> {
        let file = self.fds.get(fd).map_err(errno)?;
        let owner = if ofd { LockOwner::Description(Arc::as_ptr(file) as usize) } else { LockOwner::Process(1) };
        Ok(((0, file.node().ino()), owner))
    }

    fn status(st: poly_vfs::Stat) -> FileStatus {
        let stat = crate::abi::FileStat {
            dev: makedev(st.dev.major, st.dev.minor),
            ino: st.ino,
            nlink: u64::from(st.nlink),
            mode: st.mode,
            uid: st.uid,
            gid: st.gid,
            rdev: makedev(st.rdev.major, st.rdev.minor),
            size: st.size as i64,
            blksize: i64::from(st.blksize),
            blocks: st.blocks as i64,
            atime: crate::abi::Timespec { sec: st.atime.sec, nsec: i64::from(st.atime.nsec) },
            mtime: crate::abi::Timespec { sec: st.mtime.sec, nsec: i64::from(st.mtime.nsec) },
            ctime: crate::abi::Timespec { sec: st.ctime.sec, nsec: i64::from(st.ctime.nsec) },
        };
        FileStatus { stat, mount_id: 1, mount_root: st.ino == 1 }
    }
}

impl LinuxFs for VfsFs {
    fn openat(&mut self, dirfd: i32, path: &[u8], flags: u32, mode: u32) -> Result<i32, Errno> {
        let flags = OpenFlags::from_bits(flags);
        let file = self.at(dirfd, path, |v, at| v.open(&self.ctx, &self.cred, at, path, flags, mode as u16))?;
        self.fds.install(file, flags.contains(OpenFlags::CLOEXEC)).map_err(errno)
    }

    fn check_fd_free(&mut self) -> Result<(), Errno> {
        self.fds.check_free().map_err(errno)
    }

    fn statat(&mut self, dirfd: i32, path: &[u8], follow: bool) -> Result<FileStatus, Errno> {
        if path.is_empty() {
            return Ok(Self::status(self.node(dirfd)?.stat().map_err(errno)?));
        }
        self.at(dirfd, path, |v, at| v.stat(&self.ctx, &self.cred, at, path, follow)).map(Self::status)
    }

    fn readlinkat(&mut self, dirfd: i32, path: &[u8]) -> Result<Vec<u8>, Errno> {
        if path.is_empty() {
            return self.node(dirfd)?.readlink().map_err(|_| Errno::ENOENT);
        }
        self.at(dirfd, path, |v, at| v.readlink(&self.ctx, &self.cred, at, path))
    }

    fn getdents(&mut self, fd: i32, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<usize, Errno> {
        let file = self.fds.get(fd).map_err(errno)?.clone();
        file.read_dir(&mut |e| emit(&DirEntry { ino: e.ino, next: e.next, kind: e.kind.dirent_type(), name: e.name }))
            .map_err(errno)
    }

    fn accessat(&mut self, dirfd: i32, path: &[u8], mode: u32, _: bool, follow: bool) -> Result<(), Errno> {
        let want = poly_vfs::Access::from_bits(mode).map_err(errno)?;
        self.at(dirfd, path, |v, at| v.access(&self.ctx, &self.cred, at, path, want, follow))
    }

    fn getcwd(&mut self) -> Result<Vec<u8>, Errno> {
        self.vfs.getcwd(&self.ctx).map_err(errno)
    }

    fn chdir(&mut self, path: &[u8]) -> Result<(), Errno> {
        let vfs = self.vfs.clone();
        vfs.chdir(&mut self.ctx, &self.cred, At::Cwd, path).map_err(errno)
    }

    fn fchdir(&mut self, fd: i32) -> Result<(), Errno> {
        let node = self.node(fd)?;
        self.ctx.set_cwd(&self.cred, node).map_err(errno)
    }

    fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, Errno> {
        let whence = Whence::from_linux(whence).map_err(errno)?;
        self.fds.get(fd).map_err(errno)?.seek(offset, whence).map_err(errno)
    }

    fn pread(&mut self, fd: i32, buf: &mut [u8], offset: u64) -> Result<usize, Errno> {
        self.fds.get(fd).map_err(errno)?.read_at(offset, buf).map_err(errno)
    }

    fn pwrite(&mut self, fd: i32, buf: &[u8], offset: u64) -> Result<usize, Errno> {
        self.fds.get(fd).map_err(errno)?.write_at(offset, buf).map_err(errno)
    }

    fn dup(&mut self, fd: i32) -> Result<i32, Errno> {
        self.fds.dup(fd).map_err(errno)
    }

    fn dup3(&mut self, old: i32, new: i32, cloexec: bool) -> Result<i32, Errno> {
        let flags = if cloexec { OpenFlags::CLOEXEC } else { OpenFlags::RDONLY };
        self.fds.dup3(old, new, flags).map_err(errno)
    }

    fn dup_min(&mut self, fd: i32, min: i32, cloexec: bool) -> Result<i32, Errno> {
        self.fds.dup_from(fd, min, cloexec).map_err(errno)
    }

    fn cloexec(&mut self, fd: i32) -> Result<bool, Errno> {
        self.fds.cloexec(fd).map_err(errno)
    }

    fn set_cloexec(&mut self, fd: i32, cloexec: bool) -> Result<(), Errno> {
        self.fds.set_cloexec(fd, cloexec).map_err(errno)
    }

    fn memfd_create(&mut self, name: &[u8], perm: u32, seals: u32, cloexec: bool) -> Result<i32, Errno> {
        let mount = self.vfs.detached_mount(Arc::new(Tmpfs::new(Arc::new(Clock))));
        let memfd = poly_vfs::memfd::Memfd::new(name, seals);
        let file = self.vfs.memfd_file(&mount, &self.cred, perm as u16, memfd).map_err(errno)?;
        self.fds.install(file, cloexec).map_err(errno)
    }

    fn add_seals(&mut self, fd: i32, seals: u32) -> Result<(), Errno> {
        let file = self.fds.get(fd).map_err(errno)?;
        let memfd = file.memfd().ok_or(Errno::EINVAL)?;
        if !file.writable() {
            return Err(Errno::EPERM);
        }
        memfd.add_seals(seals).map_err(errno)
    }

    fn get_seals(&mut self, fd: i32) -> Result<u32, Errno> {
        self.fds.get(fd).map_err(errno)?.memfd().map(|m| m.seals()).ok_or(Errno::EINVAL)
    }

    fn close_range(&mut self, first: u32, last: u32, cloexec: bool) -> Result<(), Errno> {
        self.fds.close_range(first, last, cloexec);
        Ok(())
    }

    fn test_lock(
        &mut self,
        fd: i32,
        ofd: bool,
        write: bool,
        start: u64,
        end: u64,
    ) -> Result<Option<LockHolder>, Errno> {
        let (key, owner) = self.lock_key(fd, ofd)?;
        let kind = if write { LockKind::Write } else { LockKind::Read };
        Ok(self.locks.test(key, owner, kind, start, end).map(|l| LockHolder {
            write: l.kind == LockKind::Write,
            start: l.start,
            end: l.end,
            pid: l.pid,
        }))
    }

    /// No other thread can unlock here: a conflict is `EAGAIN` even with
    /// `wait`.
    fn set_lock(
        &mut self,
        fd: i32,
        ofd: bool,
        write: Option<bool>,
        start: u64,
        end: u64,
        _wait: bool,
    ) -> Result<(), Errno> {
        let (key, owner) = self.lock_key(fd, ofd)?;
        let kind = write.map(|w| if w { LockKind::Write } else { LockKind::Read });
        let pid = if ofd { -1 } else { 1 };
        self.locks.set(key, owner, pid, kind, start, end).map_err(|_| Errno::EAGAIN)
    }

    fn flock(&mut self, fd: i32, exclusive: Option<bool>, _wait: bool) -> Result<(), Errno> {
        let (key, owner) = self.lock_key(fd, true)?;
        match exclusive {
            Some(x) => self.locks.flock(key, owner, x).map_err(|_| Errno::EAGAIN),
            None => {
                self.locks.unflock(key, owner);
                Ok(())
            }
        }
    }

    fn status_flags(&mut self, fd: i32) -> Result<u32, Errno> {
        Ok(self.fds.get(fd).map_err(errno)?.flags().bits())
    }

    fn set_status_flags(&mut self, fd: i32, flags: u32) -> Result<(), Errno> {
        self.fds.get(fd).map_err(errno)?.set_flags(OpenFlags::from_bits(flags));
        Ok(())
    }

    fn umask(&mut self, mask: u32) -> u32 {
        u32::from(self.ctx.set_umask(mask as u16))
    }

    fn mkdirat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno> {
        self.at(dirfd, path, |v, at| v.mkdir(&self.ctx, &self.cred, at, path, mode as u16))
    }

    fn unlinkat(&mut self, dirfd: i32, path: &[u8], remove_dir: bool) -> Result<(), Errno> {
        if remove_dir {
            self.at(dirfd, path, |v, at| v.rmdir(&self.ctx, &self.cred, at, path))
        } else {
            self.at(dirfd, path, |v, at| v.unlink(&self.ctx, &self.cred, at, path))
        }
    }

    fn renameat(
        &mut self,
        olddirfd: i32,
        old: &[u8],
        newdirfd: i32,
        new: Result<&[u8], Errno>,
        flags: u32,
    ) -> Result<(), Errno> {
        let flags = RenameFlags::from_linux(flags).map_err(errno)?;
        let (o, n) = (self.start(olddirfd), self.start(newdirfd));
        let old = PathArg { path: Ok(old), at: at_of(&o) };
        let new = PathArg { path: new.map_err(vfs_errno), at: at_of(&n) };
        self.vfs.rename_paths(&self.ctx, &self.cred, old, new, flags).map_err(errno)
    }

    fn symlinkat(&mut self, target: &[u8], dirfd: i32, path: &[u8]) -> Result<(), Errno> {
        self.at(dirfd, path, |v, at| v.symlink(&self.ctx, &self.cred, target, at, path))
    }

    fn linkat(
        &mut self,
        olddirfd: i32,
        old: &[u8],
        newdirfd: i32,
        new: Result<&[u8], Errno>,
        follow: bool,
    ) -> Result<(), Errno> {
        let (o, n) = (self.start(olddirfd), self.start(newdirfd));
        let old = PathArg { path: Ok(old), at: at_of(&o) };
        let new = PathArg { path: new.map_err(vfs_errno), at: at_of(&n) };
        self.vfs.link_paths(&self.ctx, &self.cred, old, new, follow).map_err(errno)
    }

    fn fchmod(&mut self, fd: i32, mode: u32) -> Result<(), Errno> {
        self.fds.get(fd).map_err(errno)?.chmod(&self.cred, mode as u16).map_err(errno)
    }

    fn fchmodat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno> {
        self.at(dirfd, path, |v, at| v.chmod(&self.ctx, &self.cred, at, path, mode as u16, true))
    }

    fn fchownat(
        &mut self,
        dirfd: i32,
        path: &[u8],
        uid: Option<u32>,
        gid: Option<u32>,
        follow: bool,
    ) -> Result<(), Errno> {
        let node = if path.is_empty() {
            self.node(dirfd)?
        } else {
            self.at(dirfd, path, |v, at| v.lookup(&self.ctx, &self.cred, at, path, follow))?
        };
        node.chown(&self.cred, uid, gid).map_err(errno)
    }

    fn fchown(&mut self, fd: i32, uid: Option<u32>, gid: Option<u32>) -> Result<(), Errno> {
        let file = self.fds.get(fd).map_err(errno)?;
        if file.is_path_only() {
            return Err(Errno::EBADF);
        }
        file.node().chown(&self.cred, uid, gid).map_err(errno)
    }

    fn utimensat(
        &mut self,
        dirfd: i32,
        path: Option<&[u8]>,
        atime: Option<TimeChange>,
        mtime: Option<TimeChange>,
        follow: bool,
    ) -> Result<(), Errno> {
        let set = |t: Option<TimeChange>| {
            t.map(|t| match t {
                TimeChange::Now => poly_vfs::SetTime::Now,
                TimeChange::At(ts) => poly_vfs::SetTime::At(poly_vfs::Timespec::new(ts.sec, ts.nsec as u32)),
            })
        };
        let node = match path {
            None => {
                let file = self.fds.get(dirfd).map_err(errno)?;
                if file.is_path_only() {
                    return Err(Errno::EBADF);
                }
                file.node().clone()
            }
            Some([]) => self.node(dirfd)?,
            Some(p) => self.at(dirfd, p, |v, at| v.lookup(&self.ctx, &self.cred, at, p, follow))?,
        };
        node.set_times(&self.cred, set(atime), set(mtime)).map_err(errno)
    }

    fn ftruncate(&mut self, fd: i32, len: u64) -> Result<(), Errno> {
        self.fds.get(fd).map_err(errno)?.truncate(len).map_err(errno)
    }

    fn truncate(&mut self, path: &[u8], len: u64) -> Result<(), Errno> {
        self.vfs.truncate(&self.ctx, &self.cred, At::Cwd, path, len).map_err(errno)
    }

    fn fsync(&mut self, fd: i32, _: bool) -> Result<(), Errno> {
        self.fds.get(fd).map_err(errno)?.sync().map_err(errno)
    }

    fn sync(&mut self) {
        *self.synced.lock().unwrap() += 1;
    }

    fn syncfs(&mut self, fd: i32) -> Result<(), Errno> {
        self.fds.get(fd).map_err(errno)?;
        Ok(())
    }

    fn statfs(&mut self, path: &[u8]) -> Result<StatFs, Errno> {
        let n = self.vfs.lookup(&self.ctx, &self.cred, At::Cwd, path, true).map_err(errno)?;
        let s = n.statfs().map_err(errno)?;
        Ok(StatFs { fs_type: s.magic, bsize: s.block_size, namelen: s.name_max, flags: s.flags, ..StatFs::default() })
    }

    fn fstatfs(&mut self, fd: i32) -> Result<StatFs, Errno> {
        let s = self.node(fd)?.statfs().map_err(errno)?;
        Ok(StatFs { fs_type: s.magic, bsize: s.block_size, namelen: s.name_max, flags: s.flags, ..StatFs::default() })
    }

    fn set_fd_limit(&mut self, target: Target, limit: u64) -> Result<(), Errno> {
        self.limits.lock().unwrap().push((target, limit));
        if target == Target::Current {
            self.fds.set_limit(limit as usize);
        }
        Ok(())
    }

    fn getxattr(&mut self, target: XattrTarget<'_>, name: &[u8]) -> Result<Vec<u8>, Errno> {
        self.xattr_node(target)?.getxattr(&self.cred, name).map_err(errno)
    }

    fn setxattr(&mut self, target: XattrTarget<'_>, name: &[u8], value: &[u8], flags: u32) -> Result<(), Errno> {
        self.xattr_node(target)?.setxattr(&self.cred, name, value, flags).map_err(errno)
    }

    fn listxattr(&mut self, target: XattrTarget<'_>) -> Result<Vec<u8>, Errno> {
        self.xattr_node(target)?.listxattr(&self.cred).map_err(errno)
    }

    fn removexattr(&mut self, target: XattrTarget<'_>, name: &[u8]) -> Result<(), Errno> {
        self.xattr_node(target)?.removexattr(&self.cred, name).map_err(errno)
    }
}

impl VfsFs {
    fn xattr_node(&self, target: XattrTarget<'_>) -> Result<poly_vfs::NodeRef, Errno> {
        match target {
            XattrTarget::Path { path, follow } => {
                self.vfs.lookup(&self.ctx, &self.cred, At::Cwd, path, follow).map_err(errno)
            }
            XattrTarget::Fd(fd) => {
                let file = self.fds.get(fd).map_err(errno)?;
                if file.is_path_only() { Err(Errno::EBADF) } else { Ok(file.node().clone()) }
            }
        }
    }
}

pub(super) struct Env {
    pub(super) k: FakeKernel,
    pub(super) vfs: Arc<Vfs>,
    limits: Arc<Mutex<Vec<(Target, u64)>>>,
    synced: Arc<Mutex<u32>>,
}

/// A fake kernel whose file system is a tmpfs with `/dir/file`
/// ("contents"), `/dir/sub/`, `/link` -> `dir/file`; descriptors 0-2 are
/// not open in the VFS table (the fake's own read/write files stay).
pub(super) fn env() -> Env {
    let vfs = Arc::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(Clock)))));
    let ctx = vfs.root_context().unwrap();
    let cred = Cred::root();
    vfs.mkdir(&ctx, &cred, At::Cwd, b"/dir", 0o755).unwrap();
    vfs.mkdir(&ctx, &cred, At::Cwd, b"/dir/sub", 0o755).unwrap();
    let f = vfs.open(&ctx, &cred, At::Cwd, b"/dir/file", OpenFlags::CREAT | OpenFlags::WRONLY, 0o644).unwrap();
    f.write(b"contents").unwrap();
    vfs.symlink(&ctx, &cred, b"dir/file", At::Cwd, b"/link").unwrap();
    let mut fds = FdTable::new();
    // Hold 0-2 so new descriptors start at 3, as in a process.
    for _ in 0..3 {
        let null = vfs.open(&ctx, &cred, At::Cwd, b"/dir", OpenFlags::RDONLY, 0).unwrap();
        fds.install(null, false).unwrap();
    }
    let limits = Arc::new(Mutex::new(Vec::new()));
    let synced = Arc::new(Mutex::new(0));
    let mut k = FakeKernel::new();
    k.fs = Some(std::boxed::Box::new(VfsFs {
        vfs: vfs.clone(),
        ctx,
        fds,
        cred,
        limits: limits.clone(),
        synced: synced.clone(),
        locks: poly_vfs::locks::LockTable::new(),
    }));
    Env { k, vfs, limits, synced }
}

/// Puts a NUL-terminated string in the data area at `offset`.
pub(super) fn s(k: &mut FakeKernel, offset: u64, text: &[u8]) -> u64 {
    let mut b = text.to_vec();
    b.push(0);
    k.poke(scratch(offset), &b);
    scratch(offset)
}

const OUT: u64 = 0x8000;
pub(super) const AT: u64 = AT_FDCWD as i64 as u64;

#[test]
fn without_kernel_services_file_calls_are_enosys() {
    let mut k = FakeKernel::new();
    let p = s(&mut k, 0, b"/x");
    assert_eq!(sys(&mut k, nr::OPENAT, &[AT, p, 0, 0]), Err(Errno::ENOSYS));
    assert_eq!(sys(&mut k, nr::GETCWD, &[scratch(OUT), 100]), Err(Errno::ENOSYS));
    assert_eq!(sys(&mut k, nr::SYSINFO, &[scratch(OUT)]), Err(Errno::ENOSYS));
}

#[test]
fn openat_open_and_creat() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::OPENAT, &[AT, p, 0, 0]), Ok(3));
    assert_eq!(sys(k, nr::OPEN, &[p, 0, 0]), Ok(4));
    let n = s(k, 0, b"/dir/new");
    // O_CREAT | O_EXCL | O_WRONLY, mode 0640 with the 022 umask.
    assert_eq!(sys(k, nr::OPENAT, &[AT, n, 0o301, 0o640]), Ok(5));
    assert_eq!(sys(k, nr::OPENAT, &[AT, n, 0o301, 0o640]), Err(Errno::EEXIST));
    let st = e.vfs.stat(&e.vfs.root_context().unwrap(), &Cred::root(), At::Cwd, b"/dir/new", true).unwrap();
    assert_eq!(st.mode, 0o100640);
    let c = s(&mut e.k, 0, b"/dir/created");
    assert_eq!(sys(&mut e.k, nr::CREAT, &[c, 0o600]), Ok(6));
    // Relative to a directory descriptor.
    let d = s(&mut e.k, 0, b"/dir");
    let dirfd = sys(&mut e.k, nr::OPEN, &[d, 0o200000, 0]).unwrap();
    let rel = s(&mut e.k, 0, b"file");
    assert_eq!(sys(&mut e.k, nr::OPENAT, &[dirfd, rel, 0, 0]), Ok(dirfd + 1));
    assert_eq!(sys(&mut e.k, nr::OPENAT, &[99, rel, 0, 0]), Err(Errno::EBADF));
}

#[test]
fn paths_are_checked() {
    let mut e = env();
    let k = &mut e.k;
    let empty = s(k, 0, b"");
    assert_eq!(sys(k, nr::OPENAT, &[AT, empty, 0, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::OPEN, &[UNMAPPED, 0, 0]), Err(Errno::EFAULT));
    let long = std::vec![b'a'; 5000];
    k.poke(scratch(0), &long);
    assert_eq!(sys(k, nr::OPEN, &[scratch(0), 0, 0]), Err(Errno::ENAMETOOLONG));
    let missing = s(k, 0, b"/nope");
    assert_eq!(sys(k, nr::OPEN, &[missing, 0, 0]), Err(Errno::ENOENT));
}

#[test]
fn open_reports_the_descriptor_limit_before_the_path() {
    let mut e = env();
    let k = &mut e.k;
    // 0-2 are open: a soft limit of 3 leaves no number free.
    k.poke(scratch(0), &Rlimit { cur: 3, max: 4096 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), 0]), Ok(0));
    let new = s(k, 0x100, b"/dir/new");
    let empty = s(k, 0x200, b"");
    let long = std::vec![b'a'; 5000];
    k.poke(scratch(0x300), &long);
    assert_eq!(sys(k, nr::OPEN, &[new, 0o102, 0o644]), Err(Errno::EMFILE), "O_CREAT creates nothing");
    assert_eq!(sys(k, nr::OPEN, &[UNMAPPED, 0, 0]), Err(Errno::EMFILE), "before EFAULT");
    assert_eq!(sys(k, nr::OPEN, &[empty, 0, 0]), Err(Errno::EMFILE), "before ENOENT");
    assert_eq!(sys(k, nr::OPEN, &[scratch(0x300), 0, 0]), Err(Errno::EMFILE), "before ENAMETOOLONG");
    assert_eq!(sys(k, nr::OPENAT, &[AT, new, 0o102, 0o644]), Err(Errno::EMFILE));
    let st = e.vfs.stat(&e.vfs.root_context().unwrap(), &Cred::root(), At::Cwd, b"/dir/new", true);
    assert_eq!(st.map(|_| ()), Err(poly_vfs::Errno::ENOENT));
    // One more number and the same open works.
    let k = &mut e.k;
    k.poke(scratch(0), &Rlimit { cur: 4, max: 4096 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), 0]), Ok(0));
    assert_eq!(sys(k, nr::OPEN, &[new, 0o102, 0o644]), Ok(3));
    assert_eq!(sys(k, nr::OPEN, &[new, 0, 0]), Err(Errno::EMFILE));
}

#[test]
fn open_flags_and_mode_are_32_bit() {
    let mut e = env();
    let k = &mut e.k;
    let f = s(k, 0, b"/dir/file");
    // The upper half of the flags register is ignored: O_RDWR.
    let fd = sys(k, nr::OPEN, &[f, 0xffff_ffff_0000_0002, 0]).unwrap();
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]), Ok(0o100002), "F_GETFL: O_RDWR | O_LARGEFILE");
    // Unknown flag bits are ignored too.
    let fd = sys(k, nr::OPEN, &[f, 0x4000_0000, 0]).unwrap();
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]), Ok(0o100000));
    // The upper half of mode and bits above 07777 are ignored.
    let g = s(k, 0x100, b"/dir/g");
    sys(k, nr::OPEN, &[g, 0o301, (0x1234_5678 << 32) | 0o600]).unwrap();
    let h = s(k, 0x200, b"/dir/h");
    sys(k, nr::OPEN, &[h, 0o101, 0o170644]).unwrap();
    let ctx = e.vfs.root_context().unwrap();
    let mode = |p: &[u8]| e.vfs.stat(&ctx, &Cred::root(), At::Cwd, p, true).unwrap().mode;
    assert_eq!(mode(b"/dir/g"), 0o100600);
    assert_eq!(mode(b"/dir/h"), 0o100644);
}

#[test]
fn stat_lstat_newfstatat_and_fstat_by_empty_path() {
    let mut e = env();
    let k = &mut e.k;
    let link = s(k, 0, b"/link");
    assert_eq!(sys(k, nr::STAT, &[link, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u32(scratch(OUT) + 24), 0o100644, "st_mode of the target");
    assert_eq!(le_u64(&k.peek(scratch(OUT) + 48, 8)), 8, "st_size");
    assert_eq!(sys(k, nr::LSTAT, &[link, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u32(scratch(OUT) + 24) & 0o170000, 0o120000);
    // newfstatat with AT_SYMLINK_NOFOLLOW, and bad flags.
    assert_eq!(sys(k, nr::NEWFSTATAT, &[AT, link, scratch(OUT), 0x100]), Ok(0));
    assert_eq!(k.peek_u32(scratch(OUT) + 24) & 0o170000, 0o120000);
    assert_eq!(sys(k, nr::NEWFSTATAT, &[AT, link, scratch(OUT), 0x1]), Err(Errno::EINVAL));
    // AT_EMPTY_PATH: the descriptor itself (and NULL is accepted).
    let p = s(k, 0x100, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[p, 0, 0]).unwrap();
    let empty = s(k, 0x200, b"");
    assert_eq!(sys(k, nr::NEWFSTATAT, &[fd, empty, scratch(OUT), 0x1000]), Ok(0));
    assert_eq!(k.peek_u32(scratch(OUT) + 24), 0o100644);
    assert_eq!(sys(k, nr::NEWFSTATAT, &[fd, 0, scratch(OUT), 0x1000]), Ok(0));
    assert_eq!(sys(k, nr::NEWFSTATAT, &[fd, empty, scratch(OUT), 0]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::STAT, &[link, UNMAPPED]), Err(Errno::EFAULT));
}

/// lstat (the spec sheet's `path_order` and `bad_buffer`): the path string
/// is checked first, then its walk, and only a path that resolves is
/// checked against the buffer, which an error leaves unchanged.
#[test]
fn lstat_checks_the_path_before_the_buffer() {
    let mut e = env();
    let k = &mut e.k;
    let link = s(k, 0, b"/link");
    let missing = s(k, 0x100, b"/dir/nope");
    let through_file = s(k, 0x200, b"/dir/file/x");
    let empty = s(k, 0x300, b"");
    assert_eq!(sys(k, nr::LSTAT, &[link, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u32(scratch(OUT) + 24), 0o120777, "S_IFLNK | 0777");
    assert_eq!(le_u64(&k.peek(scratch(OUT) + 48, 8)), 8, "the length of \"dir/file\"");
    assert_eq!(sys(k, nr::LSTAT, &[missing, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::LSTAT, &[through_file, 0]), Err(Errno::ENOTDIR));
    assert_eq!(sys(k, nr::LSTAT, &[empty, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::LSTAT, &[0, scratch(OUT)]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::LSTAT, &[1, scratch(OUT)]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::LSTAT, &[8, 8]), Err(Errno::EFAULT));
    // 4096 bytes without a NUL: too long before any component is looked at.
    let long = std::vec![b'a'; 4096];
    k.poke(scratch(0x1000), b"/nope/");
    k.poke(scratch(0x1006), &long[6..]);
    k.poke(scratch(0x2000), &[0]);
    assert_eq!(sys(k, nr::LSTAT, &[scratch(0x1000), scratch(OUT)]), Err(Errno::ENAMETOOLONG));

    // Bad buffers: NULL, read-only, running into an unmapped page; 144
    // bytes ending exactly at the end of the mapping are fine.
    assert_eq!(sys(k, nr::LSTAT, &[link, 0]), Err(Errno::EFAULT));
    let ro = read_only_page(k);
    assert_eq!(sys(k, nr::LSTAT, &[link, ro]), Err(Errno::EFAULT));
    let end = scratch(16 * 4096);
    assert_eq!(sys(k, nr::LSTAT, &[link, end - 144]), Ok(0));
    k.poke(end - 100, &[0xa5; 100]);
    assert_eq!(sys(k, nr::LSTAT, &[link, end - 100]), Err(Errno::EFAULT));
    assert_eq!(k.peek(end - 100, 100), [0xa5; 100], "the buffer is left unchanged");
    // On a path error the buffer is not touched either.
    k.poke(scratch(OUT), &[0x5a; 144]);
    assert_eq!(sys(k, nr::LSTAT, &[missing, scratch(OUT)]), Err(Errno::ENOENT));
    assert_eq!(k.peek(scratch(OUT), 144), [0x5a; 144]);
}

#[test]
fn statx_fills_the_basic_fields_and_the_mount_id() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::STATX, &[AT, p, 0, 0x7ff, scratch(OUT)]), Ok(0));
    let b = k.peek(scratch(OUT), 0x100);
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    assert_eq!(u32_at(0), 0x7ff | 0x1000, "stx_mask");
    assert_eq!(u16::from_le_bytes([b[0x1c], b[0x1d]]), 0o100644, "stx_mode");
    assert_eq!(u64_at(0x28), 8, "stx_size");
    assert_eq!(u32_at(0x10), 1, "stx_nlink");
    assert_eq!(u64_at(0x40), 1000, "stx_atime.tv_sec");
    assert_eq!(u64_at(0x50), 0, "no birth time");
    assert_eq!(u64_at(0x90), 1, "stx_mnt_id");
    assert_eq!(u64_at(0x38), 0x2000, "STATX_ATTR_MOUNT_ROOT is supported");
    assert_eq!(u64_at(0x08), 0, "not a mount root");
    let root = s(k, 0, b"/");
    assert_eq!(sys(k, nr::STATX, &[AT, root, 0, 0x7ff, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u64(scratch(OUT) + 8), 0x2000);
    // Bad flags, both sync types, the reserved mask bit.
    assert_eq!(sys(k, nr::STATX, &[AT, p, 0x1, 0x7ff, scratch(OUT)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::STATX, &[AT, p, 0x6000, 0x7ff, scratch(OUT)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::STATX, &[AT, p, 0, 0x8000_0000, scratch(OUT)]), Err(Errno::EINVAL));
}

#[test]
fn device_numbers_round_trip() {
    for (ma, mi) in [(0, 0), (1, 3), (5, 1), (8, 2), (0xfff, 0xff), (0x1234, 0x56789), (u32::MAX, u32::MAX)] {
        let dev = makedev(ma, mi);
        assert_eq!((major(dev), minor(dev)), (ma, mi));
    }
}

#[test]
fn readlink_and_readlinkat() {
    let mut e = env();
    let k = &mut e.k;
    let link = s(k, 0, b"/link");
    assert_eq!(sys(k, nr::READLINK, &[link, scratch(OUT), 100]), Ok(8));
    assert_eq!(k.peek(scratch(OUT), 8), b"dir/file");
    // Truncated silently, no NUL.
    assert_eq!(sys(k, nr::READLINKAT, &[AT, link, scratch(OUT), 3]), Ok(3));
    assert_eq!(sys(k, nr::READLINK, &[link, scratch(OUT), 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::READLINK, &[link, scratch(OUT), u64::from(u32::MAX)]), Err(Errno::EINVAL), "int -1");
    let file = s(k, 0x100, b"/dir/file");
    assert_eq!(sys(k, nr::READLINK, &[file, scratch(OUT), 100]), Err(Errno::EINVAL), "not a link");
    // Empty path: the link the descriptor is open on (O_PATH | O_NOFOLLOW).
    let fd = sys(k, nr::OPEN, &[link, 0o10000000 | 0o400000, 0]).unwrap();
    let empty = s(k, 0x200, b"");
    assert_eq!(sys(k, nr::READLINKAT, &[fd, empty, scratch(OUT), 100]), Ok(8));
    assert_eq!(sys(k, nr::READLINKAT, &[AT, empty, scratch(OUT), 100]), Err(Errno::ENOENT));
}

/// The records of a getdents64 buffer: (inode, d_off, type, name).
fn dirents(buf: &[u8]) -> Vec<(u64, u64, u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < buf.len() {
        let reclen = u16::from_le_bytes([buf[at + 16], buf[at + 17]]) as usize;
        assert_eq!(reclen % 8, 0);
        let name = &buf[at + 19..at + reclen];
        let name = name[..name.iter().position(|&b| b == 0).unwrap()].to_vec();
        out.push((le_u64(&buf[at..at + 8]), le_u64(&buf[at + 8..at + 16]), buf[at + 18], name));
        at += reclen;
    }
    out
}

#[test]
fn getdents64_lists_in_pieces_and_resumes() {
    let mut e = env();
    let k = &mut e.k;
    let d = s(k, 0, b"/dir");
    let fd = sys(k, nr::OPEN, &[d, 0o200000, 0]).unwrap();
    // Room for exactly two 24-byte records.
    assert_eq!(sys(k, nr::GETDENTS64, &[fd, scratch(OUT), 48]), Ok(48));
    let first = dirents(&k.peek(scratch(OUT), 48));
    assert_eq!(first.iter().map(|e| e.3.as_slice()).collect::<Vec<_>>(), [&b"."[..], b".."]);
    assert_eq!(first[0].2, 4, "DT_DIR");
    let n = sys(k, nr::GETDENTS64, &[fd, scratch(OUT), 4096]).unwrap();
    let rest = dirents(&k.peek(scratch(OUT), n as usize));
    // tmpfs lists newest first (the getdents64 spec, `directory_order`):
    // the file was made after the directory.
    assert_eq!(rest.iter().map(|e| e.3.as_slice()).collect::<Vec<_>>(), [&b"file"[..], b"sub"]);
    assert_eq!(rest[0].2, 8, "DT_REG");
    // d_off: "." 1, ".." the first entry's cookie, the last INT32_MAX.
    assert_eq!(first[0].1, 1);
    assert_eq!(first[1].1, rest[0].1 + 1, "file's cookie is one above sub's");
    assert_eq!(rest[1].1, i32::MAX as u64);
    assert_eq!(sys(k, nr::GETDENTS64, &[fd, scratch(OUT), 4096]), Ok(0), "the end");
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 1]), Ok(i32::MAX as u64), "SEEK_CUR at the end");
    // Seeking back to a d_off continues after that entry.
    assert_eq!(sys(k, nr::LSEEK, &[fd, first[0].1, 0]), Ok(first[0].1));
    let n = sys(k, nr::GETDENTS64, &[fd, scratch(OUT), 4096]).unwrap();
    assert_eq!(dirents(&k.peek(scratch(OUT), n as usize)).len(), 3);
    // Too small for one entry; not a directory; bad fd.
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 0]), Ok(0));
    assert_eq!(sys(k, nr::GETDENTS64, &[fd, scratch(OUT), 10]), Err(Errno::EINVAL));
    let f = s(k, 0, b"/dir/file");
    let file = sys(k, nr::OPEN, &[f, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::GETDENTS64, &[file, scratch(OUT), 4096]), Err(Errno::ENOTDIR));
    assert_eq!(sys(k, nr::GETDENTS64, &[77, scratch(OUT), 4096]), Err(Errno::EBADF));
}

#[test]
fn legacy_getdents_puts_the_type_last() {
    let mut e = env();
    let k = &mut e.k;
    let d = s(k, 0, b"/dir/sub");
    let fd = sys(k, nr::OPEN, &[d, 0o200000, 0]).unwrap();
    assert_eq!(sys(k, nr::GETDENTS, &[fd, scratch(OUT), 4096]), Ok(48));
    let b = k.peek(scratch(OUT), 48);
    assert_eq!(u16::from_le_bytes([b[16], b[17]]), 24);
    assert_eq!(&b[18..20], b".\0");
    assert_eq!(b[23], 4, "d_type in the record's last byte");
}

#[test]
fn dirent_records_are_8_byte_aligned() {
    assert_eq!(dirent64_len(b"a"), 24);
    assert_eq!(dirent64_len(b"abcd"), 24);
    assert_eq!(dirent64_len(b"abcde"), 32);
    let mut buf = [0xffu8; 32];
    let e = DirEntry { ino: 7, next: 9, kind: 8, name: b"abcde" };
    assert_eq!(encode_dirent64(&e, &mut buf), 32);
    assert_eq!(&buf[19..25], b"abcde\0");
    assert!(buf[25..].iter().all(|&b| b == 0), "padding is zero");
}

#[test]
fn access_and_faccessat2() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::ACCESS, &[p, 0]), Ok(0), "F_OK");
    assert_eq!(sys(k, nr::ACCESS, &[p, 4 | 2]), Ok(0));
    assert_eq!(sys(k, nr::ACCESS, &[p, 1]), Err(Errno::EACCES), "root needs one x bit");
    assert_eq!(sys(k, nr::ACCESS, &[p, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FACCESSAT, &[AT, p, 4]), Ok(0));
    assert_eq!(sys(k, nr::FACCESSAT2, &[AT, p, 4, 0x200]), Ok(0));
    assert_eq!(sys(k, nr::FACCESSAT2, &[AT, p, 4, 0x1]), Err(Errno::EINVAL));
    let missing = s(k, 0, b"/none");
    assert_eq!(sys(k, nr::ACCESS, &[missing, 0]), Err(Errno::ENOENT));
}

#[test]
fn getcwd_chdir_fchdir() {
    let mut e = env();
    let k = &mut e.k;
    assert_eq!(sys(k, nr::GETCWD, &[scratch(OUT), 100]), Ok(2));
    assert_eq!(k.peek(scratch(OUT), 2), b"/\0");
    let d = s(k, 0, b"/dir/sub");
    assert_eq!(sys(k, nr::CHDIR, &[d]), Ok(0));
    assert_eq!(sys(k, nr::GETCWD, &[scratch(OUT), 100]), Ok(9));
    assert_eq!(k.peek(scratch(OUT), 9), b"/dir/sub\0");
    assert_eq!(sys(k, nr::GETCWD, &[scratch(OUT), 8]), Err(Errno::ERANGE));
    assert_eq!(sys(k, nr::GETCWD, &[scratch(OUT), 0]), Err(Errno::ERANGE));
    let f = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::CHDIR, &[f]), Err(Errno::ENOTDIR));
    let up = s(k, 0, b"/");
    let fd = sys(k, nr::OPEN, &[up, 0o200000, 0]).unwrap();
    assert_eq!(sys(k, nr::FCHDIR, &[fd]), Ok(0));
    assert_eq!(sys(k, nr::GETCWD, &[scratch(OUT), 100]), Ok(2));
    assert_eq!(sys(k, nr::FCHDIR, &[u64::MAX]), Err(Errno::EBADF));
}

#[test]
fn lseek_pread_pwrite() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[p, 2, 0]).unwrap();
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 2]), Ok(8), "SEEK_END");
    assert_eq!(sys(k, nr::LSEEK, &[fd, u64::MAX, 0]), Err(Errno::EINVAL), "negative");
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 9]), Err(Errno::EINVAL), "whence");
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(OUT), 100, 3]), Ok(5));
    assert_eq!(k.peek(scratch(OUT), 5), b"tents");
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 1]), Ok(8), "pread leaves the offset");
    let data = s(k, 0, b"XY");
    assert_eq!(sys(k, nr::PWRITE64, &[fd, data, 2, 1]), Ok(2));
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(OUT), 100, 0]), Ok(8));
    assert_eq!(k.peek(scratch(OUT), 8), b"cXYtents");
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(OUT), 1, u64::MAX]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::PWRITE64, &[fd, UNMAPPED, 1, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(OUT), 0, 0]), Ok(0));
}

#[test]
fn dup_dup2_dup3() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[p, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::DUP, &[fd]), Ok(fd + 1));
    assert_eq!(sys(k, nr::DUP2, &[fd, 10]), Ok(10));
    assert_eq!(sys(k, nr::DUP2, &[fd, fd]), Ok(fd), "dup2 onto itself");
    assert_eq!(sys(k, nr::DUP2, &[50, 50]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::DUP3, &[fd, fd, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::DUP3, &[fd, 11, 1]), Err(Errno::EINVAL), "bad flags");
    assert_eq!(sys(k, nr::DUP3, &[fd, 11, 0o2000000]), Ok(11));
    assert_eq!(sys(k, nr::FCNTL, &[11, 1, 0]), Ok(1), "FD_CLOEXEC from dup3");
    assert_eq!(sys(k, nr::FCNTL, &[10, 1, 0]), Ok(0));
    assert_eq!(sys(k, nr::DUP, &[u64::MAX]), Err(Errno::EBADF));
}

#[test]
fn fcntl_commands() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[p, 0o2002, 0]).unwrap();
    // Every open on x86-64 is a large-file open (O_LARGEFILE, 0o100000).
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]), Ok(0o102002), "F_GETFL: O_RDWR | O_APPEND | O_LARGEFILE");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 4, 0o4000]), Ok(0), "F_SETFL O_NONBLOCK");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]), Ok(0o104002), "O_APPEND cleared, access mode kept");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 2, 1]), Ok(0), "F_SETFD FD_CLOEXEC");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 1, 0]), Ok(1));
    assert_eq!(sys(k, nr::FCNTL, &[fd, 0, 20]), Ok(20), "F_DUPFD");
    assert_eq!(sys(k, nr::FCNTL, &[20, 1, 0]), Ok(0), "F_DUPFD clears close-on-exec");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 1030, 20]), Ok(21), "F_DUPFD_CLOEXEC");
    assert_eq!(sys(k, nr::FCNTL, &[21, 1, 0]), Ok(1));
    assert_eq!(sys(k, nr::FCNTL, &[fd, 0, u64::MAX]), Err(Errno::EINVAL), "negative");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 0, 5000]), Err(Errno::EINVAL), "beyond RLIMIT_NOFILE");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 5, 0]), Err(Errno::EFAULT), "F_GETLK reads a struct flock");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 8, 0]), Err(Errno::EINVAL), "F_SETOWN not supported");
    assert_eq!(sys(k, nr::FCNTL, &[99, 3, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::FCNTL, &[99, 12345, 0]), Err(Errno::EBADF), "the fd is checked first");
}

#[test]
fn names_change() {
    let mut e = env();
    let k = &mut e.k;
    let d = s(k, 0, b"/made");
    assert_eq!(sys(k, nr::MKDIR, &[d, 0o777]), Ok(0));
    assert_eq!(sys(k, nr::MKDIR, &[d, 0o777]), Err(Errno::EEXIST));
    let d2 = s(k, 0x100, b"made2");
    assert_eq!(sys(k, nr::MKDIRAT, &[AT, d2, 0o700]), Ok(0));
    assert_eq!(sys(k, nr::RMDIR, &[d2]), Ok(0));
    let old = s(k, 0x200, b"/dir/file");
    let new = s(k, 0x300, b"/made/moved");
    assert_eq!(sys(k, nr::RENAME, &[old, new]), Ok(0));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, new, AT, old, 1]), Ok(0), "RENAME_NOREPLACE to a free name");
    let other = s(k, 0x400, b"/dir/other");
    assert_eq!(sys(k, nr::SYMLINKAT, &[old, AT, other]), Ok(0));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, old, AT, other, 1]), Err(Errno::EEXIST));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, old, AT, other, 3]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::RENAMEAT, &[AT, old, AT, other]), Ok(0));
    let hard = s(k, 0x500, b"/made/hard");
    assert_eq!(sys(k, nr::LINK, &[other, hard]), Ok(0));
    assert_eq!(sys(k, nr::STAT, &[hard, scratch(OUT)]), Ok(0));
    assert_eq!(le_u64(&k.peek(scratch(OUT) + 16, 8)), 2, "two links");
    let hard2 = s(k, 0x600, b"/made/hard2");
    assert_eq!(sys(k, nr::LINKAT, &[AT, other, AT, hard2, 0x1]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::LINKAT, &[AT, other, AT, hard2, 0x400]), Ok(0));
    assert_eq!(sys(k, nr::UNLINK, &[hard2]), Ok(0));
    assert_eq!(sys(k, nr::UNLINKAT, &[AT, hard, 0]), Ok(0));
    assert_eq!(sys(k, nr::UNLINKAT, &[AT, d, 0x200]), Ok(0), "AT_REMOVEDIR");
    assert_eq!(sys(k, nr::UNLINKAT, &[AT, d, 0x1]), Err(Errno::EINVAL));
    let sl = s(k, 0x700, b"/sl");
    assert_eq!(sys(k, nr::SYMLINK, &[old, sl]), Ok(0));
    let empty = s(k, 0x800, b"");
    assert_eq!(sys(k, nr::SYMLINK, &[empty, sl]), Err(Errno::ENOENT));
}

/// linkat and renameat2: the second path's string errors (an unreadable
/// pointer, PATH_MAX bytes or more, empty) and a bad newdirfd come after
/// the first path's errors (the spec sheets' `path_order_two_paths`).
#[test]
fn second_path_errors_come_in_their_place() {
    let mut e = env();
    let k = &mut e.k;
    let missing = s(k, 0, b"/dir/missing");
    let nope = s(k, 0x100, b"/dir/nope/x");
    let file = s(k, 0x200, b"/dir/file");
    let rel = s(k, 0x300, b"x");
    let empty = s(k, 0x400, b"");
    let too_long = s(k, 0x1000, &[b'/'; crate::sys::path::PATH_MAX]);
    let bad_fd = 99;
    for new in [UNMAPPED, too_long, empty] {
        assert_eq!(sys(k, nr::LINKAT, &[AT, missing, AT, new, 0]), Err(Errno::ENOENT), "oldpath first");
        assert_eq!(sys(k, nr::RENAMEAT2, &[AT, nope, AT, new, 0]), Err(Errno::ENOENT), "old's directories first");
    }
    assert_eq!(sys(k, nr::LINKAT, &[AT, missing, bad_fd, rel, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, nope, bad_fd, rel, 0]), Err(Errno::ENOENT));
    // Then the second path's own errors, in their order.
    assert_eq!(sys(k, nr::LINKAT, &[AT, file, bad_fd, UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::LINKAT, &[AT, file, bad_fd, too_long, 0]), Err(Errno::ENAMETOOLONG));
    assert_eq!(sys(k, nr::LINKAT, &[AT, file, bad_fd, empty, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(k, nr::LINKAT, &[AT, file, bad_fd, rel, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, missing, AT, too_long, 0]), Err(Errno::ENAMETOOLONG));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, missing, bad_fd, rel, 0]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, missing, AT, rel, 0]), Err(Errno::ENOENT));
    // The first path's string errors still come before everything.
    assert_eq!(sys(k, nr::LINKAT, &[AT, UNMAPPED, bad_fd, UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::RENAMEAT2, &[AT, empty, bad_fd, UNMAPPED, 0]), Err(Errno::ENOENT));
}

#[test]
fn modes_sizes_and_sync() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::CHMOD, &[p, 0o100600]), Ok(0), "only the permission bits count");
    assert_eq!(sys(k, nr::STAT, &[p, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u32(scratch(OUT) + 24), 0o100600);
    assert_eq!(sys(k, nr::FCHMODAT, &[AT, p, 0o640]), Ok(0));
    let fd = sys(k, nr::OPEN, &[p, 2, 0]).unwrap();
    assert_eq!(sys(k, nr::FCHMOD, &[fd, 0o444]), Ok(0));
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, 3]), Ok(0));
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, u64::MAX]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::TRUNCATE, &[p, 5]), Ok(0));
    assert_eq!(sys(k, nr::TRUNCATE, &[p, u64::MAX]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::FSTAT, &[1, scratch(OUT)]), Ok(0));
    assert_eq!(sys(k, nr::PREAD64, &[fd, scratch(OUT), 100, 0]), Ok(5));
    assert_eq!(k.peek(scratch(OUT), 5), b"con\0\0");
    assert_eq!(sys(k, nr::FSYNC, &[fd]), Ok(0));
    assert_eq!(sys(k, nr::FDATASYNC, &[fd]), Ok(0));
    assert_eq!(sys(k, nr::FSYNC, &[77]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::SYNC, &[]), Ok(0));
    assert_eq!(*e.synced.lock().unwrap(), 1);
    assert_eq!(sys(&mut e.k, nr::SYNCFS, &[fd]), Ok(0));
}

#[test]
fn umask_returns_the_old_mask() {
    let mut e = env();
    let k = &mut e.k;
    assert_eq!(sys(k, nr::UMASK, &[0o077]), Ok(0o022));
    assert_eq!(sys(k, nr::UMASK, &[0o7777]), Ok(0o077));
    assert_eq!(sys(k, nr::UMASK, &[0]), Ok(0o777), "only 0777 is kept");
}

#[test]
fn statfs_and_fstatfs() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir");
    assert_eq!(sys(k, nr::STATFS, &[p, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u64(scratch(OUT)), 0x0102_1994, "TMPFS_MAGIC");
    assert_eq!(k.peek_u64(scratch(OUT) + 8), 4096, "f_bsize");
    assert_eq!(k.peek_u64(scratch(OUT) + 64), 255, "f_namelen");
    let fd = sys(k, nr::OPEN, &[p, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::FSTATFS, &[fd, scratch(OUT)]), Ok(0));
    assert_eq!(k.peek_u64(scratch(OUT)), 0x0102_1994);
    assert_eq!(sys(k, nr::FSTATFS, &[fd, UNMAPPED]), Err(Errno::EFAULT));
    let st = StatFs { fs_type: 1, fsid: [2, 3], flags: 9, ..StatFs::default() }.to_bytes();
    assert_eq!(st.len(), 120);
    assert_eq!(&st[56..64], &[2, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(st[80], 9);
}

fn rlimit_at(k: &FakeKernel, at: u64) -> Rlimit {
    Rlimit::from_bytes(&k.peek(at, 16).try_into().unwrap())
}

#[test]
fn prlimit64_reads_and_sets_limits() {
    let mut e = env();
    let k = &mut e.k;
    k.creds_mut().caps = crate::cred::Caps::ROOT;
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), 0, scratch(OUT)]), Ok(0));
    assert_eq!(rlimit_at(k, scratch(OUT)), Rlimit { cur: 1024, max: 4096 });
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_STACK), 0, scratch(OUT)]), Ok(0));
    assert_eq!(rlimit_at(k, scratch(OUT)), Rlimit { cur: 8 << 20, max: RLIM_INFINITY });
    // Set and get the old one in the same call.
    k.poke(scratch(0), &Rlimit { cur: 64, max: 128 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), scratch(OUT)]), Ok(0));
    assert_eq!(rlimit_at(k, scratch(OUT)), Rlimit { cur: 1024, max: 4096 });
    assert_eq!(k.me().limits.get(RLIMIT_NOFILE), Some(Rlimit { cur: 64, max: 128 }));
    assert_eq!(*e.limits.lock().unwrap(), [(Target::Current, 64)]);
    // The descriptor table follows: no descriptor at 64 or above.
    let k = &mut e.k;
    let p = s(k, 0x100, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[p, 0, 0]).unwrap();
    assert_eq!(sys(k, nr::DUP2, &[fd, 64]), Err(Errno::EBADF));
    assert_eq!(sys(k, nr::DUP2, &[fd, 63]), Ok(63));
}

#[test]
fn prlimit64_errors() {
    let mut e = env();
    let k = &mut e.k;
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, 16, 0, scratch(OUT)]), Err(Errno::EINVAL), "RLIM_NLIMITS");
    k.poke(scratch(0), &Rlimit { cur: 10, max: 5 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, 0, scratch(0), 0]), Err(Errno::EINVAL), "soft above hard");
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, 0, UNMAPPED, 0]), Err(Errno::EFAULT));
    // Raising a hard limit needs privilege (the fake runs as euid 1001).
    k.poke(scratch(0), &Rlimit { cur: 100, max: 5000 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), 0]), Err(Errno::EPERM));
    // Lowering it does not.
    k.poke(scratch(0), &Rlimit { cur: 100, max: 2000 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), 0]), Ok(0));
    // Root may raise, but not past nr_open.
    k.creds_mut().caps = crate::cred::Caps::ROOT;
    k.poke(scratch(0), &Rlimit { cur: 100, max: (1 << 20) + 1 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), 0]), Err(Errno::EPERM));
    k.poke(scratch(0), &Rlimit { cur: 1 << 20, max: 1 << 20 }.to_bytes());
    assert_eq!(sys(k, nr::PRLIMIT64, &[0, u64::from(RLIMIT_NOFILE), scratch(0), 0]), Ok(0));
    // Other processes.
    assert_eq!(sys(k, nr::PRLIMIT64, &[555, 0, 0, scratch(OUT)]), Err(Errno::ESRCH));
    assert_eq!(sys(k, nr::PRLIMIT64, &[u64::MAX, 0, 0, scratch(OUT)]), Err(Errno::ESRCH));
    k.add_process(200, 3000, 200, 200);
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(sys(k, nr::PRLIMIT64, &[200, 0, 0, scratch(OUT)]), Err(Errno::EPERM), "another user's");
    k.creds_mut().caps = crate::cred::Caps::ROOT;
    assert_eq!(sys(k, nr::PRLIMIT64, &[200, u64::from(RLIMIT_CORE), 0, scratch(OUT)]), Ok(0));
    assert_eq!(rlimit_at(k, scratch(OUT)), Rlimit { cur: 0, max: RLIM_INFINITY });
}

#[test]
fn getrlimit_and_setrlimit() {
    let mut e = env();
    let k = &mut e.k;
    assert_eq!(sys(k, nr::GETRLIMIT, &[u64::from(RLIMIT_CPU), scratch(OUT)]), Ok(0));
    assert_eq!(rlimit_at(k, scratch(OUT)), Rlimit::UNLIMITED);
    k.poke(scratch(0), &Rlimit { cur: 1 << 20, max: RLIM_INFINITY }.to_bytes());
    assert_eq!(sys(k, nr::SETRLIMIT, &[u64::from(RLIMIT_STACK), scratch(0)]), Ok(0));
    assert_eq!(sys(k, nr::GETRLIMIT, &[u64::from(RLIMIT_STACK), scratch(OUT)]), Ok(0));
    assert_eq!(rlimit_at(k, scratch(OUT)).cur, 1 << 20);
    assert_eq!(sys(k, nr::GETRLIMIT, &[16, scratch(OUT)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::GETRLIMIT, &[0, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::SETRLIMIT, &[0, UNMAPPED]), Err(Errno::EFAULT));
    // RLIMIT_SIGPENDING moves the signal queue limit.
    k.poke(scratch(0), &Rlimit { cur: 10, max: 4096 }.to_bytes());
    assert_eq!(sys(k, nr::SETRLIMIT, &[u64::from(RLIMIT_SIGPENDING), scratch(0)]), Ok(0));
    assert_eq!(k.me().queue_limit, 10);
}

#[test]
fn limits_are_inherited_by_fork() {
    let mut p = crate::LinuxProcess::new(1);
    p.limits.set(RLIMIT_NOFILE, Rlimit { cur: 7, max: 9 });
    let child = p.fork(1, 2);
    assert_eq!(child.limits.get(RLIMIT_NOFILE), Some(Rlimit { cur: 7, max: 9 }));
    let mut exec = child.clone();
    exec.exec(2);
    assert_eq!(exec.limits, child.limits, "execve keeps resource limits");
}

#[test]
fn sysinfo_encodes_struct_sysinfo() {
    let mut k = FakeKernel::new();
    k.sysinfo = Some(SysInfo {
        uptime: 42,
        loads: [65536, 32768, 0],
        totalram: 512 << 20,
        freeram: 100 << 20,
        procs: 3,
        mem_unit: 1,
        ..SysInfo::default()
    });
    assert_eq!(sys(&mut k, nr::SYSINFO, &[scratch(OUT)]), Ok(0));
    let b = k.peek(scratch(OUT), 112);
    assert_eq!(le_u64(&b[0..8]), 42);
    assert_eq!(le_u64(&b[8..16]), 65536);
    assert_eq!(le_u64(&b[32..40]), 512 << 20);
    assert_eq!(le_u64(&b[40..48]), 100 << 20);
    assert_eq!(u16::from_le_bytes([b[80], b[81]]), 3);
    assert_eq!(u32::from_le_bytes(b[104..108].try_into().unwrap()), 1);
    assert_eq!(sys(&mut k, nr::SYSINFO, &[UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn fionbio_and_fioclex_work_on_any_descriptor() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPEN, &[p, 0o2, 0]).unwrap();
    let arg = s(k, 0x100, b"");
    k.poke(arg, &1i32.to_le_bytes());
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x5421, arg]), Ok(0), "FIONBIO on a regular file");
    // (O_LARGEFILE, 0o100000: every x86-64 open.)
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]), Ok(0o104002));
    k.poke(arg, &0i32.to_le_bytes());
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x5421, arg]), Ok(0));
    assert_eq!(sys(k, nr::FCNTL, &[fd, 3, 0]), Ok(0o100002));
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x5451, 0]), Ok(0), "FIOCLEX");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 1, 0]), Ok(1));
    assert_eq!(sys(k, nr::IOCTL, &[fd, 0x5450, 0]), Ok(0), "FIONCLEX");
    assert_eq!(sys(k, nr::FCNTL, &[fd, 1, 0]), Ok(0));
    assert_eq!(sys(k, nr::IOCTL, &[99, 0x5421, arg]), Err(Errno::EBADF));
}
