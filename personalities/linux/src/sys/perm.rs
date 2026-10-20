//! Ownership and times (ADR 0045): chown(2), fchown(2), lchown(2),
//! fchownat(2) and utimensat(2). Like the other file system calls (ADR
//! 0043, `sys::path`), the handlers check flags, read the path and call the
//! kernel's [`LinuxFs`](crate::fs::LinuxFs), which checks as the calling
//! thread. (access(2), chmod(2) and umask(2) are in `sys::path`.)

use crate::abi::Timespec;
use crate::args::Args;
use crate::cred::optional_id;
use crate::errno::{Errno, SysResult};
use crate::fs::TimeChange;
use crate::fs::at::{AT_EMPTY_PATH, AT_FDCWD, AT_SYMLINK_NOFOLLOW};
use crate::kernel::LinuxKernel;
use crate::sys::path::{fs, nonempty_path, path_arg};

/// utimensat(2)'s special nanosecond values (`linux/stat.h`).
pub const UTIME_NOW: i64 = (1 << 30) - 1;
pub const UTIME_OMIT: i64 = (1 << 30) - 2;

fn ids(a: &Args, i: usize) -> (Option<u32>, Option<u32>) {
    (optional_id(a.uint(i)), optional_id(a.uint(i + 1)))
}

/// chown(path, owner, group): `-1` leaves an id alone; follows a final
/// symbolic link.
pub fn chown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    let (uid, gid) = ids(a, 1);
    fs(k)?.fchownat(AT_FDCWD, &path, uid, gid, true).map(|()| 0)
}

/// lchown(path, owner, group): the link itself.
pub fn lchown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = nonempty_path(k, a.ptr(0))?;
    let (uid, gid) = ids(a, 1);
    fs(k)?.fchownat(AT_FDCWD, &path, uid, gid, false).map(|()| 0)
}

/// fchown(fd, owner, group).
pub fn fchown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (uid, gid) = ids(a, 1);
    fs(k)?.fchown(a.fd(0), uid, gid).map(|()| 0)
}

/// fchownat(dirfd, path, owner, group, flags): `AT_SYMLINK_NOFOLLOW` and
/// `AT_EMPTY_PATH`; other flags `EINVAL` (checked before the path is read).
pub fn fchownat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(4);
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(Errno::EINVAL);
    }
    let path = if flags & AT_EMPTY_PATH != 0 { path_arg(k, a.ptr(1))? } else { nonempty_path(k, a.ptr(1))? };
    let (uid, gid) = ids(a, 2);
    fs(k)?.fchownat(a.fd(0), &path, uid, gid, flags & AT_SYMLINK_NOFOLLOW == 0).map(|()| 0)
}

/// One `struct timespec` of utimensat(2): `UTIME_OMIT` is `None`,
/// `UTIME_NOW` the current time (`tv_sec` ignored for both), else
/// nanoseconds must be 0-999999999 (`EINVAL`).
fn time_change(t: Timespec) -> Result<Option<TimeChange>, Errno> {
    match t.nsec {
        UTIME_OMIT => Ok(None),
        UTIME_NOW => Ok(Some(TimeChange::Now)),
        0..=999_999_999 => Ok(Some(TimeChange::At(t))),
        _ => Err(Errno::EINVAL),
    }
}

/// utimensat(dirfd, path, times, flags). `times` null means both "now".
/// "If both tv_nsec fields are specified as UTIME_OMIT, then the Linux
/// implementation of utimensat() succeeds even if the file referred to by
/// dirfd and pathname does not exist." A null `path` is the Linux
/// extension futimens(3) uses: the file `dirfd` is open on (`EFAULT` for
/// `AT_FDCWD`). Flags: `AT_SYMLINK_NOFOLLOW` and `AT_EMPTY_PATH`, else
/// `EINVAL`. Who may change what is the kernel's check (owner or
/// `CAP_FOWNER`; for "now" on both, write access is enough).
pub fn utimensat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let dirfd = a.fd(0);
    let flags = a.uint(3);
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(Errno::EINVAL);
    }
    let (atime, mtime) = if a.ptr(2).is_null() {
        (Some(TimeChange::Now), Some(TimeChange::Now))
    } else {
        let mut b = [0u8; 2 * Timespec::SIZE];
        k.copy_from_user(a.ptr(2), &mut b)?;
        let at = |i: usize| {
            let mut t = [0u8; Timespec::SIZE];
            t.copy_from_slice(&b[i * Timespec::SIZE..(i + 1) * Timespec::SIZE]);
            time_change(Timespec::from_bytes(&t))
        };
        (at(0)?, at(1)?)
    };
    if atime.is_none() && mtime.is_none() {
        return Ok(0);
    }
    let follow = flags & AT_SYMLINK_NOFOLLOW == 0;
    if a.ptr(1).is_null() {
        if dirfd == AT_FDCWD {
            return Err(Errno::EFAULT);
        }
        return fs(k)?.utimensat(dirfd, None, atime, mtime, follow).map(|()| 0);
    }
    let path = if flags & AT_EMPTY_PATH != 0 { path_arg(k, a.ptr(1))? } else { nonempty_path(k, a.ptr(1))? };
    fs(k)?.utimensat(dirfd, Some(&path), atime, mtime, follow).map(|()| 0)
}
