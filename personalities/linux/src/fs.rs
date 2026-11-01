//! File system services the personality asks the kernel for
//! ([`LinuxFs`], reached through [`LinuxKernel::fs`]), and the user ABI of
//! the file system calls: `AT_*` and `fcntl` constants, `struct statx`,
//! `struct statfs`, `struct linux_dirent64`, `struct sysinfo`.
//!
//! Like [`LinuxKernel`], the trait works on kernel buffers: the handlers in
//! `sys::path` and `sys::fd` copy paths and results from and to user
//! memory, and the kernel never sees a user pointer. Paths are the bytes
//! the caller passed, without the NUL; flags are Linux's bits. Errors are
//! the ones the matching manual page lists. See ADR 0043.
//!
//! [`LinuxKernel`]: crate::kernel::LinuxKernel
//! [`LinuxKernel::fs`]: crate::kernel::LinuxKernel::fs

use crate::abi::{FileStat, Timespec};
use crate::errno::Errno;
use crate::kernel::Target;
use alloc::vec::Vec;

/// `AT_*` values (Linux UAPI `linux/fcntl.h`).
pub mod at {
    /// "Use the current working directory" in place of a directory fd.
    pub const AT_FDCWD: i32 = -100;
    pub const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
    /// faccessat2(2): check with the effective ids.
    pub const AT_EACCESS: u32 = 0x200;
    /// unlinkat(2): remove a directory.
    pub const AT_REMOVEDIR: u32 = 0x200;
    pub const AT_SYMLINK_FOLLOW: u32 = 0x400;
    pub const AT_NO_AUTOMOUNT: u32 = 0x800;
    pub const AT_EMPTY_PATH: u32 = 0x1000;
    /// statx(2) synchronisation: the two bits together are invalid.
    pub const AT_STATX_SYNC_TYPE: u32 = 0x6000;
}

/// `fcntl` commands and flags (`asm-generic/fcntl.h`, `linux/fcntl.h`).
pub mod fcntl {
    pub const F_DUPFD: u32 = 0;
    pub const F_GETFD: u32 = 1;
    pub const F_SETFD: u32 = 2;
    pub const F_GETFL: u32 = 3;
    pub const F_SETFL: u32 = 4;
    /// `F_LINUX_SPECIFIC_BASE + 6`.
    pub const F_DUPFD_CLOEXEC: u32 = 1030;
    /// `F_LINUX_SPECIFIC_BASE + 7` and `+ 8`: a pipe's capacity.
    pub const F_SETPIPE_SZ: u32 = 1031;
    pub const F_GETPIPE_SZ: u32 = 1032;
    /// `F_LINUX_SPECIFIC_BASE + 9` and `+ 10`: file seals (memfd).
    pub const F_ADD_SEALS: u32 = 1033;
    pub const F_GET_SEALS: u32 = 1034;
    /// The close-on-exec descriptor flag.
    pub const FD_CLOEXEC: u64 = 1;
    /// `O_CLOEXEC`, the only flag dup3(2) takes.
    pub const O_CLOEXEC: u32 = 0o2000000;
    /// The `open` flags for creat(2): `O_CREAT | O_WRONLY | O_TRUNC`.
    pub const CREAT_FLAGS: u32 = 0o100 | 0o1 | 0o1000;
}

/// access(2) modes (`F_OK`, `R_OK`, `W_OK`, `X_OK`, POSIX `<unistd.h>`).
pub mod access {
    pub const F_OK: u32 = 0;
    pub const X_OK: u32 = 1;
    pub const W_OK: u32 = 2;
    pub const R_OK: u32 = 4;
}

/// statx(2) masks and attributes (`linux/stat.h`).
pub mod statx {
    pub const STATX_BASIC_STATS: u32 = 0x7ff;
    pub const STATX_MNT_ID: u32 = 0x1000;
    /// The unique 64-bit mount id instead of the legacy one (`stx_mnt_id`).
    pub const STATX_MNT_ID_UNIQUE: u32 = 0x4000;
    pub const STATX_RESERVED: u32 = 0x8000_0000;
    pub const STATX_ATTR_MOUNT_ROOT: u64 = 0x2000;
}

/// Resource limits (`asm-generic/resource.h`).
pub mod rlimit {
    pub const RLIMIT_CPU: u32 = 0;
    pub const RLIMIT_FSIZE: u32 = 1;
    pub const RLIMIT_DATA: u32 = 2;
    pub const RLIMIT_STACK: u32 = 3;
    pub const RLIMIT_CORE: u32 = 4;
    pub const RLIMIT_RSS: u32 = 5;
    pub const RLIMIT_NPROC: u32 = 6;
    pub const RLIMIT_NOFILE: u32 = 7;
    pub const RLIMIT_MEMLOCK: u32 = 8;
    pub const RLIMIT_AS: u32 = 9;
    pub const RLIMIT_LOCKS: u32 = 10;
    pub const RLIMIT_SIGPENDING: u32 = 11;
    pub const RLIMIT_MSGQUEUE: u32 = 12;
    pub const RLIMIT_NICE: u32 = 13;
    pub const RLIMIT_RTPRIO: u32 = 14;
    pub const RLIMIT_RTTIME: u32 = 15;
    pub const RLIM_NLIMITS: u32 = 16;
    /// `RLIM_INFINITY`: `~0UL`.
    pub const RLIM_INFINITY: u64 = u64::MAX;
}

/// The major number of a `dev_t` in the encoding of
/// [`makedev`](crate::abi::makedev) (makedev(3)).
pub const fn major(dev: u64) -> u32 {
    (((dev >> 32) & 0xffff_f000) | ((dev >> 8) & 0xfff)) as u32
}

/// The minor number of a `dev_t` (makedev(3)).
pub const fn minor(dev: u64) -> u32 {
    (((dev >> 12) & 0xffff_ff00) | (dev & 0xff)) as u32
}

/// What the kernel reports about a file for stat(2) and statx(2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileStatus {
    pub stat: FileStat,
    /// The mount's id, as `/proc/<pid>/mountinfo` shows it (`stx_mnt_id`).
    pub mount_id: u64,
    /// The file is the root of a mount (`STATX_ATTR_MOUNT_ROOT`).
    pub mount_root: bool,
}

impl FileStatus {
    /// `sizeof(struct statx)`.
    pub const STATX_SIZE: usize = 0x100;

    /// Encode as `struct statx` (`linux/stat.h`) with `stx_mask` =
    /// `mask`: `stx_mask, stx_blksize` (u32), `stx_attributes` (u64) at 0;
    /// `stx_nlink, stx_uid, stx_gid` (u32), `stx_mode` (u16) at 0x10;
    /// `stx_ino, stx_size, stx_blocks, stx_attributes_mask` (u64) at 0x20;
    /// four timestamps (`tv_sec` i64, `tv_nsec` u32, 4 reserved bytes) at
    /// 0x40: atime, btime, ctime, mtime; `stx_rdev_major, stx_rdev_minor,
    /// stx_dev_major, stx_dev_minor` (u32) at 0x80; `stx_mnt_id` (u64) at
    /// 0x90; the rest zero.
    pub fn to_statx(&self, mask: u32) -> [u8; Self::STATX_SIZE] {
        let st = &self.stat;
        let mut b = [0u8; Self::STATX_SIZE];
        let mut put = |at: usize, bytes: &[u8]| b[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x00, &mask.to_le_bytes());
        put(0x04, &(st.blksize as u32).to_le_bytes());
        let attributes = if self.mount_root { statx::STATX_ATTR_MOUNT_ROOT } else { 0 };
        put(0x08, &attributes.to_le_bytes());
        put(0x10, &(st.nlink as u32).to_le_bytes());
        put(0x14, &st.uid.to_le_bytes());
        put(0x18, &st.gid.to_le_bytes());
        put(0x1c, &(st.mode as u16).to_le_bytes());
        put(0x20, &st.ino.to_le_bytes());
        put(0x28, &(st.size as u64).to_le_bytes());
        put(0x30, &(st.blocks as u64).to_le_bytes());
        put(0x38, &statx::STATX_ATTR_MOUNT_ROOT.to_le_bytes());
        let mut time = |at: usize, t: Timespec| {
            put(at, &t.sec.to_le_bytes());
            put(at + 8, &(t.nsec as u32).to_le_bytes());
        };
        time(0x40, st.atime);
        // 0x50: btime, not reported (STATX_BTIME is not in the mask).
        time(0x60, st.ctime);
        time(0x70, st.mtime);
        put(0x80, &major(st.rdev).to_le_bytes());
        put(0x84, &minor(st.rdev).to_le_bytes());
        put(0x88, &major(st.dev).to_le_bytes());
        put(0x8c, &minor(st.dev).to_le_bytes());
        put(0x90, &self.mount_id.to_le_bytes());
        b
    }
}

/// A record lock [`LinuxFs::test_lock`] found in the way: write or read,
/// its range `[start, end)` (`end` `u64::MAX`: to the end of the file) and
/// the owning process (-1 for an open file description's lock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockHolder {
    pub write: bool,
    pub start: u64,
    pub end: u64,
    pub pid: i32,
}

/// `struct statfs` (`asm-generic/statfs.h`, x86-64: every field a
/// `long`, `f_fsid` two `int`s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatFs {
    pub fs_type: u64,
    pub bsize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub fsid: [u32; 2],
    pub namelen: u64,
    pub frsize: u64,
    pub flags: u64,
}

impl StatFs {
    /// `sizeof(struct statfs)` on x86-64: 11 words, `f_fsid`, 4 spare words.
    pub const SIZE: usize = 120;

    /// Encode in field order; the spare words are zero.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let words = [self.fs_type, self.bsize, self.blocks, self.bfree, self.bavail, self.files, self.ffree];
        for (i, w) in words.iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        b[56..60].copy_from_slice(&self.fsid[0].to_le_bytes());
        b[60..64].copy_from_slice(&self.fsid[1].to_le_bytes());
        b[64..72].copy_from_slice(&self.namelen.to_le_bytes());
        b[72..80].copy_from_slice(&self.frsize.to_le_bytes());
        b[80..88].copy_from_slice(&self.flags.to_le_bytes());
        b
    }
}

/// `struct sysinfo` (`linux/sysinfo.h`) as sysinfo(2) fills it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SysInfo {
    /// Seconds since boot.
    pub uptime: i64,
    /// 1, 5 and 15 minute load averages, scaled by 65536.
    pub loads: [u64; 3],
    pub totalram: u64,
    pub freeram: u64,
    pub sharedram: u64,
    pub bufferram: u64,
    pub totalswap: u64,
    pub freeswap: u64,
    pub procs: u16,
    pub totalhigh: u64,
    pub freehigh: u64,
    /// The unit of the memory sizes, in bytes.
    pub mem_unit: u32,
}

impl SysInfo {
    /// `sizeof(struct sysinfo)` on x86-64: `uptime` and `loads[3]`, six
    /// `unsigned long`s, `procs` and `pad` (u16) at 80, `totalhigh` and
    /// `freehigh` at 88 and 96 (aligned), `mem_unit` (u32) at 104, then
    /// `_f[0]` and padding to 112.
    pub const SIZE: usize = 112;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let words = [
            self.uptime as u64,
            self.loads[0],
            self.loads[1],
            self.loads[2],
            self.totalram,
            self.freeram,
            self.sharedram,
            self.bufferram,
            self.totalswap,
            self.freeswap,
        ];
        for (i, w) in words.iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        b[80..82].copy_from_slice(&self.procs.to_le_bytes());
        b[88..96].copy_from_slice(&self.totalhigh.to_le_bytes());
        b[96..104].copy_from_slice(&self.freehigh.to_le_bytes());
        b[104..108].copy_from_slice(&self.mem_unit.to_le_bytes());
        b
    }
}

/// One directory entry as the kernel lists it for getdents64(2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry<'a> {
    pub ino: u64,
    /// Where the next entry is (`d_off`): seeking there continues after
    /// this one.
    pub next: u64,
    /// `d_type`: `DT_REG`, `DT_DIR`, ... (getdents(2)).
    pub kind: u8,
    pub name: &'a [u8],
}

/// Bytes a `struct linux_dirent64` record of `name` takes: `d_ino` (u64),
/// `d_off` (i64), `d_reclen` (u16), `d_type` (u8), the name and its NUL,
/// padded to 8 bytes (getdents(2)).
pub const fn dirent64_len(name: &[u8]) -> usize {
    (19 + name.len() + 1).next_multiple_of(8)
}

/// Encode `e` as a `struct linux_dirent64` at the start of `out`, which
/// must hold [`dirent64_len`] bytes.
pub fn encode_dirent64(e: &DirEntry<'_>, out: &mut [u8]) -> usize {
    let len = dirent64_len(e.name);
    let rec = &mut out[..len];
    rec.fill(0);
    rec[0..8].copy_from_slice(&e.ino.to_le_bytes());
    rec[8..16].copy_from_slice(&e.next.to_le_bytes());
    rec[16..18].copy_from_slice(&(len as u16).to_le_bytes());
    rec[18] = e.kind;
    rec[19..19 + e.name.len()].copy_from_slice(e.name);
    len
}

/// Bytes a legacy `struct linux_dirent` record of `name` takes: `d_ino`,
/// `d_off` (unsigned long), `d_reclen` (u16), the name and its NUL, a
/// padding byte and `d_type` as the record's last byte, padded to 8.
pub const fn dirent_len(name: &[u8]) -> usize {
    (18 + name.len() + 2).next_multiple_of(8)
}

/// Encode `e` as a legacy `struct linux_dirent` (getdents(2)).
pub fn encode_dirent(e: &DirEntry<'_>, out: &mut [u8]) -> usize {
    let len = dirent_len(e.name);
    let rec = &mut out[..len];
    rec.fill(0);
    rec[0..8].copy_from_slice(&e.ino.to_le_bytes());
    rec[8..16].copy_from_slice(&e.next.to_le_bytes());
    rec[16..18].copy_from_slice(&(len as u16).to_le_bytes());
    rec[18..18 + e.name.len()].copy_from_slice(e.name);
    rec[len - 1] = e.kind;
    len
}

/// A time for utimensat(2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeChange {
    /// `UTIME_NOW`.
    Now,
    /// An explicit time (nanoseconds already checked to be 0-999999999).
    At(Timespec),
}

/// The file system services of the kernel for the calling thread's
/// process. `dirfd` arguments are descriptors or [`at::AT_FDCWD`]; an
/// absolute path ignores `dirfd`. Errors are the manual pages' (`EBADF`,
/// `ENOENT`, `ENOTDIR`, `EACCES`, `EEXIST`, `EROFS`, `ELOOP`, ...).
pub trait LinuxFs {
    /// openat(2): `flags` are the caller's (unknown bits ignored), `mode`
    /// the permission bits for a new file. Returns the new descriptor.
    fn openat(&mut self, dirfd: i32, path: &[u8], flags: u32, mode: u32) -> Result<i32, Errno>;

    /// openat2(2): [`openat`](Self::openat) with the walk restricted by
    /// `resolve` (the `RESOLVE_*` bits, already checked): `NO_XDEV`
    /// (crossing a mount `EXDEV`), `NO_MAGICLINKS` / `NO_SYMLINKS`
    /// (following such a link `ELOOP`), `BENEATH` (leaving `dirfd`'s
    /// directory `EXDEV`), `IN_ROOT` (`dirfd`'s directory is `/`, also for
    /// an absolute path), `CACHED` (no effect). Without restrictions it is
    /// openat.
    fn openat2(&mut self, dirfd: i32, path: &[u8], flags: u32, mode: u32, resolve: u64) -> Result<i32, Errno> {
        const CACHED: u64 = 0x20;
        if resolve & !CACHED == 0 { self.openat(dirfd, path, flags, mode) } else { Err(Errno::ENOSYS) }
    }

    /// `EMFILE` when every descriptor below the soft `RLIMIT_NOFILE` is in
    /// use: open(2) reports it before it looks at the path.
    fn check_fd_free(&mut self) -> Result<(), Errno>;

    /// stat(2) of `path`; an empty `path` means the file `dirfd` is open
    /// on (`AT_EMPTY_PATH`; the working directory for `AT_FDCWD`).
    fn statat(&mut self, dirfd: i32, path: &[u8], follow: bool) -> Result<FileStatus, Errno>;

    /// readlinkat(2): the whole target. An empty `path` means the link
    /// `dirfd` is open on (with `O_PATH | O_NOFOLLOW`).
    fn readlinkat(&mut self, dirfd: i32, path: &[u8]) -> Result<Vec<u8>, Errno>;

    /// getdents64(2) without the encoding: lists the directory open as
    /// `fd` from its offset, calling `emit` per entry until it returns
    /// `false` (no room; the entry comes again next time) or the end.
    /// Returns the entries accepted. `ENOTDIR` for other files.
    fn getdents(&mut self, fd: i32, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<usize, Errno>;

    /// faccessat2(2): `mode` is `F_OK` or a mix of `R_OK`, `W_OK`, `X_OK`;
    /// `effective` checks with the effective ids (`AT_EACCESS`). An empty
    /// `path` means `dirfd`'s file.
    fn accessat(&mut self, dirfd: i32, path: &[u8], mode: u32, effective: bool, follow: bool) -> Result<(), Errno>;

    /// getcwd(2): the working directory's absolute path.
    fn getcwd(&mut self) -> Result<Vec<u8>, Errno>;
    fn chdir(&mut self, path: &[u8]) -> Result<(), Errno>;
    fn fchdir(&mut self, fd: i32) -> Result<(), Errno>;

    /// lseek(2): `whence` as given (`EINVAL` for unknown values).
    fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, Errno>;
    /// pread64(2), pwrite64(2): at `offset` (not negative), the file
    /// offset unchanged. `ESPIPE` for unseekable files.
    fn pread(&mut self, fd: i32, buf: &mut [u8], offset: u64) -> Result<usize, Errno>;
    fn pwrite(&mut self, fd: i32, buf: &[u8], offset: u64) -> Result<usize, Errno>;
    /// fallocate(2) on `fd` (open for writing, a regular file; the range
    /// checked): `ENOSPC`, and `EOPNOTSUPP` for modes the file system does
    /// not support.
    fn fallocate(&mut self, _fd: i32, _mode: u32, _offset: u64, _len: u64) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    /// preadv2(2) with `RWF_NOWAIT`: one read that never waits (`EAGAIN`
    /// where it would), at `at` or (`None`) at the file offset, which
    /// advances. `EOPNOTSUPP` for a file on a file system (tmpfs has no
    /// non-blocking buffered reads; the spec of preadv2).
    fn read_nowait(&mut self, _fd: i32, _buf: &mut [u8], _at: Option<u64>) -> Result<usize, Errno> {
        Err(Errno::EOPNOTSUPP)
    }
    /// pwritev2(2) with `RWF_APPEND` (`append: Some(true)`), `RWF_NOAPPEND`
    /// (`Some(false)`) or `RWF_NOWAIT` (`nowait`, as for
    /// [`read_nowait`](Self::read_nowait)): one write at `at` or (`None`)
    /// at the file offset, which advances (to the new end after an
    /// append). The descriptor's own flags do not change.
    fn write_flags(
        &mut self,
        _fd: i32,
        _buf: &[u8],
        _at: Option<u64>,
        _append: Option<bool>,
        _nowait: bool,
    ) -> Result<usize, Errno> {
        Err(Errno::EOPNOTSUPP)
    }

    /// dup(2).
    fn dup(&mut self, fd: i32) -> Result<i32, Errno>;
    /// dup2(2) / dup3(2) with `old != new`: `new` replaced silently.
    fn dup3(&mut self, old: i32, new: i32, cloexec: bool) -> Result<i32, Errno>;
    /// `F_DUPFD` / `F_DUPFD_CLOEXEC`: the lowest free descriptor `>= min`.
    fn dup_min(&mut self, fd: i32, min: i32, cloexec: bool) -> Result<i32, Errno>;
    /// `F_GETFD` / `F_SETFD`.
    fn cloexec(&mut self, fd: i32) -> Result<bool, Errno>;
    fn set_cloexec(&mut self, fd: i32, cloexec: bool) -> Result<(), Errno>;
    /// close_range(2): close (with `cloexec`: mark close-on-exec) the open
    /// descriptors from `first` to `last`, both included (ADR 0059).
    fn close_range(&mut self, _first: u32, _last: u32, _cloexec: bool) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    /// close_range(2) `CLOSE_RANGE_UNSHARE`, unshare(2) `CLONE_FILES`: give
    /// the calling process a private copy of its descriptor table if
    /// another process shares it (clone(2) `CLONE_FILES`); nothing to do
    /// otherwise (ADR 0111).
    fn unshare_fd_table(&mut self) -> Result<(), Errno> {
        Ok(())
    }
    /// unshare(2) `CLONE_FS`: give the calling process a private
    /// copy of its root, working directory and umask if another process
    /// shares them (clone(2) `CLONE_FS`); nothing to do otherwise
    /// (ADR 0121).
    fn unshare_fs_context(&mut self) -> Result<(), Errno> {
        Ok(())
    }
    /// fcntl(2) `F_GETLK` (`ofd`: `F_OFD_GETLK`): the lock of another
    /// owner that keeps the caller from a read (`write` false) or write
    /// lock on `[start, end)` of `fd`'s file (`end` `u64::MAX`: to the end
    /// of the file), if any (ADR 0059).
    fn test_lock(
        &mut self,
        _fd: i32,
        _ofd: bool,
        _write: bool,
        _start: u64,
        _end: u64,
    ) -> Result<Option<LockHolder>, Errno> {
        Err(Errno::ENOSYS)
    }
    /// fcntl(2) `F_SETLK` / `F_SETLKW` (`ofd`: the `F_OFD_` forms): take a
    /// read or write lock (`Some(write)`) on the range, or unlock it
    /// (`None`). A conflict is `EAGAIN` without `wait`; with it the call
    /// waits, and a signal ends the wait with `RESTART_IF_SA_RESTART`.
    fn set_lock(
        &mut self,
        _fd: i32,
        _ofd: bool,
        _write: Option<bool>,
        _start: u64,
        _end: u64,
        _wait: bool,
    ) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    /// flock(2): a shared or exclusive lock (`Some(exclusive)`) on `fd`'s
    /// file, or unlock (`None`); a conflict as for [`set_lock`](Self::set_lock).
    /// memfd_create(2): a new unnamed regular file in memory, mode `perm`
    /// (no umask), owned by the caller's file system ids, open
    /// `O_RDWR | O_LARGEFILE` on the lowest free descriptor, starting with
    /// `seals`; its `/proc` name is `/memfd:<name> (deleted)`. `EMFILE`.
    fn memfd_create(&mut self, _name: &[u8], _perm: u32, _seals: u32, _cloexec: bool) -> Result<i32, Errno> {
        Err(Errno::ENOSYS)
    }

    /// fcntl `F_ADD_SEALS`: `EINVAL` if `fd` is not a memfd or `seals` has
    /// unknown bits, `EPERM` if it is not open for writing or sealing is
    /// sealed, `EBUSY` for `F_SEAL_WRITE` while a writable shared mapping
    /// exists.
    fn add_seals(&mut self, _fd: i32, _seals: u32) -> Result<(), Errno> {
        Err(Errno::EINVAL)
    }

    /// fcntl `F_GET_SEALS`: `EINVAL` if `fd` is not a memfd.
    fn get_seals(&mut self, _fd: i32) -> Result<u32, Errno> {
        Err(Errno::EINVAL)
    }

    fn flock(&mut self, _fd: i32, _exclusive: Option<bool>, _wait: bool) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    /// `F_GETFL` / `F_SETFL` (only `O_APPEND`, `O_ASYNC`, `O_DIRECT`,
    /// `O_NOATIME`, `O_NONBLOCK` change).
    fn status_flags(&mut self, fd: i32) -> Result<u32, Errno>;
    fn set_status_flags(&mut self, fd: i32, flags: u32) -> Result<(), Errno>;

    /// umask(2): sets the mask, returns the old one.
    fn umask(&mut self, mask: u32) -> u32;

    fn mkdirat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno>;
    /// unlinkat(2); `remove_dir` is `AT_REMOVEDIR` (rmdir).
    fn unlinkat(&mut self, dirfd: i32, path: &[u8], remove_dir: bool) -> Result<(), Errno>;
    /// renameat2(2): `flags` as given (`EINVAL` for unknown or conflicting
    /// ones). `new` is the second path as copied, or why it could not be
    /// (`EFAULT`, `ENAMETOOLONG`, `ENOENT` when empty): that error, like
    /// a bad `newdirfd`, comes after every error of `old`'s directories
    /// (renameat2's `path_order_two_paths`; `poly_vfs::Vfs::rename_paths`).
    fn renameat(
        &mut self,
        olddirfd: i32,
        old: &[u8],
        newdirfd: i32,
        new: Result<&[u8], Errno>,
        flags: u32,
    ) -> Result<(), Errno>;
    fn symlinkat(&mut self, target: &[u8], dirfd: i32, path: &[u8]) -> Result<(), Errno>;
    /// linkat(2); an empty `old` means `olddirfd`'s file (`AT_EMPTY_PATH`).
    /// `new` as for [`renameat`](Self::renameat): its errors, and a bad
    /// `newdirfd`, come after every error of `old` (linkat's
    /// `path_order_two_paths`; `poly_vfs::Vfs::link_paths`).
    fn linkat(
        &mut self,
        olddirfd: i32,
        old: &[u8],
        newdirfd: i32,
        new: Result<&[u8], Errno>,
        follow: bool,
    ) -> Result<(), Errno>;

    fn fchmod(&mut self, fd: i32, mode: u32) -> Result<(), Errno>;
    fn fchmodat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno>;
    /// fchmodat2(2): fchmodat with `follow` false for
    /// `AT_SYMLINK_NOFOLLOW` (a symbolic link found is `EOPNOTSUPP`) and an
    /// empty `path` for `AT_EMPTY_PATH`: the file `dirfd` refers to (any
    /// descriptor, `O_PATH` ones too, or the working directory for
    /// `AT_FDCWD`).
    fn fchmodat2(&mut self, dirfd: i32, path: &[u8], mode: u32, follow: bool) -> Result<(), Errno> {
        if follow && !path.is_empty() { self.fchmodat(dirfd, path, mode) } else { Err(Errno::ENOSYS) }
    }

    /// fchownat(2) (chown, lchown): `None` for `-1`, "unchanged". An empty
    /// `path` means `dirfd`'s file (`AT_EMPTY_PATH`, `O_PATH` descriptors
    /// included). Checked as the calling thread (ADR 0045): `CAP_CHOWN` to
    /// give a file away, the owner may pick one of its groups.
    fn fchownat(
        &mut self,
        dirfd: i32,
        path: &[u8],
        uid: Option<u32>,
        gid: Option<u32>,
        follow: bool,
    ) -> Result<(), Errno>;
    /// fchown(2): `EBADF` for `O_PATH` descriptors.
    fn fchown(&mut self, fd: i32, uid: Option<u32>, gid: Option<u32>) -> Result<(), Errno>;
    /// utimensat(2): `None` for `UTIME_OMIT` (never both). `path` `None`
    /// is the null path futimens(3) uses: the file `dirfd` is open on
    /// (`EBADF` for `O_PATH`); an empty path is `AT_EMPTY_PATH`'s.
    fn utimensat(
        &mut self,
        dirfd: i32,
        path: Option<&[u8]>,
        atime: Option<TimeChange>,
        mtime: Option<TimeChange>,
        follow: bool,
    ) -> Result<(), Errno>;

    fn ftruncate(&mut self, fd: i32, len: u64) -> Result<(), Errno>;
    fn truncate(&mut self, path: &[u8], len: u64) -> Result<(), Errno>;

    /// fsync(2) / fdatasync(2) (`data_only`).
    fn fsync(&mut self, fd: i32, data_only: bool) -> Result<(), Errno>;
    /// sync(2): every file system.
    fn sync(&mut self);
    /// syncfs(2): the file system `fd` is on.
    fn syncfs(&mut self, fd: i32) -> Result<(), Errno>;

    fn statfs(&mut self, path: &[u8]) -> Result<StatFs, Errno>;
    fn fstatfs(&mut self, fd: i32) -> Result<StatFs, Errno>;

    /// The soft `RLIMIT_NOFILE` of process `target` changed: descriptors
    /// from `limit` on can no longer be opened there.
    fn set_fd_limit(&mut self, target: Target, limit: u64) -> Result<(), Errno>;

    /// getxattr(2) family: the value of extended attribute `name` (checked
    /// to be 1 to 255 bytes) of `target`. The kernel checks the namespace
    /// and permissions (xattr(7)): `ENODATA`, `EOPNOTSUPP`, `EACCES`, ...
    /// The defaults are a kernel without extended attributes (`ENOSYS`).
    fn getxattr(&mut self, target: XattrTarget<'_>, name: &[u8]) -> Result<Vec<u8>, Errno> {
        let _ = (target, name);
        Err(Errno::ENOSYS)
    }

    /// setxattr(2) family; `flags` as the caller gave them.
    fn setxattr(&mut self, target: XattrTarget<'_>, name: &[u8], value: &[u8], flags: u32) -> Result<(), Errno> {
        let _ = (target, name, value, flags);
        Err(Errno::ENOSYS)
    }

    /// listxattr(2) family: the names the caller may see, each followed by
    /// a NUL.
    fn listxattr(&mut self, target: XattrTarget<'_>) -> Result<Vec<u8>, Errno> {
        let _ = target;
        Err(Errno::ENOSYS)
    }

    /// removexattr(2) family.
    fn removexattr(&mut self, target: XattrTarget<'_>, name: &[u8]) -> Result<(), Errno> {
        let _ = (target, name);
        Err(Errno::ENOSYS)
    }

    /// file_getattr(2): the inode flags of `(dirfd, path)` (an empty path:
    /// `dirfd`'s file, any descriptor); `EOPNOTSUPP` for a file without
    /// them.
    fn file_getattr(&mut self, dirfd: i32, path: &[u8], follow: bool) -> Result<crate::sys::fileattr::FileAttr, Errno> {
        let _ = (dirfd, path, follow);
        Err(Errno::ENOSYS)
    }

    /// file_setattr(2): replace the inode flags of `(dirfd, path)`.
    fn file_setattr(
        &mut self,
        dirfd: i32,
        path: &[u8],
        follow: bool,
        attr: &crate::sys::fileattr::FileAttr,
    ) -> Result<(), Errno> {
        let _ = (dirfd, path, follow, attr);
        Err(Errno::ENOSYS)
    }

    /// name_to_handle_at(2): the handle of `(dirfd, path)` (an empty path:
    /// `dirfd`'s file) and the id of its mount, as `/proc/<pid>/mountinfo`
    /// shows it; `EOPNOTSUPP` if its file system makes none.
    fn name_to_handle(
        &mut self,
        dirfd: i32,
        path: &[u8],
        follow: bool,
    ) -> Result<(crate::sys::handle::FileHandle, u64), Errno> {
        let _ = (dirfd, path, follow);
        Err(Errno::ENOSYS)
    }

    /// open_by_handle_at(2): open the file `handle` names on the file
    /// system of `mount_fd`'s file (`AT_FDCWD`: the working directory's)
    /// with open(2) `flags`; `ESTALE` if it names no live file there.
    fn open_by_handle(
        &mut self,
        mount_fd: i32,
        handle: &crate::sys::handle::FileHandle,
        flags: u32,
    ) -> Result<i32, Errno> {
        let _ = (mount_fd, handle, flags);
        Err(Errno::ENOSYS)
    }

    /// Landlock (`crate::landlock`): the chain of the object `path` names
    /// from `dirfd` (the object, then each directory above it up to the
    /// root) and whether it is a directory. Errors are the lookup's.
    fn path_ancestry(
        &mut self,
        dirfd: i32,
        path: &[u8],
        follow: bool,
    ) -> Result<(Vec<crate::landlock::ObjectKey>, bool), Errno> {
        let _ = (dirfd, path, follow);
        Err(Errno::ENOSYS)
    }

    /// Landlock: the chain of the directory the last component of `path`
    /// is in.
    fn parent_ancestry(&mut self, dirfd: i32, path: &[u8]) -> Result<Vec<crate::landlock::ObjectKey>, Errno> {
        let _ = (dirfd, path);
        Err(Errno::ENOSYS)
    }

    /// Landlock: the object `fd` is open on and whether it is a directory;
    /// `None` if it is no file of a file system (a pipe, a socket, an
    /// event file, a ruleset). `EBADF` if `fd` is not open.
    fn fd_object(&mut self, fd: i32) -> Result<Option<(crate::landlock::ObjectKey, bool)>, Errno> {
        let _ = fd;
        Err(Errno::ENOSYS)
    }
}

/// The file an extended attribute call is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XattrTarget<'a> {
    /// A path from the working directory (not empty); `follow`: a final
    /// symbolic link is followed (`getxattr`, not `lgetxattr`).
    Path { path: &'a [u8], follow: bool },
    /// An open descriptor (`fgetxattr`); `EBADF` for an `O_PATH` one.
    Fd(i32),
    /// The *xattrat calls: `path` from `dirfd` (`AT_FDCWD`: the working
    /// directory); an empty path (`AT_EMPTY_PATH`) is `dirfd`'s file, as
    /// [`XattrTarget::Fd`] (the working directory for `AT_FDCWD`).
    At { dirfd: i32, path: &'a [u8], follow: bool },
}
