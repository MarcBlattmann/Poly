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
    /// descriptors from `first` to `last`, both included (ADR 0057).
    fn close_range(&mut self, _first: u32, _last: u32, _cloexec: bool) -> Result<(), Errno> {
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
    /// ones).
    fn renameat(&mut self, olddirfd: i32, old: &[u8], newdirfd: i32, new: &[u8], flags: u32) -> Result<(), Errno>;
    fn symlinkat(&mut self, target: &[u8], dirfd: i32, path: &[u8]) -> Result<(), Errno>;
    /// linkat(2); an empty `old` means `olddirfd`'s file (`AT_EMPTY_PATH`).
    fn linkat(&mut self, olddirfd: i32, old: &[u8], newdirfd: i32, new: &[u8], follow: bool) -> Result<(), Errno>;

    fn fchmod(&mut self, fd: i32, mode: u32) -> Result<(), Errno>;
    fn fchmodat(&mut self, dirfd: i32, path: &[u8], mode: u32) -> Result<(), Errno>;

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
}
