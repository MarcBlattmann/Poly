//! File permissions and ownership (ADR 0045): chmod(2), chown(2),
//! access(2), umask(2) and utimensat(2) in all their forms. The handlers
//! decode paths, flags and ids and pick the credentials to check with
//! (the file system ids, or the real ids for access(2)); the kernel's VFS
//! applies the rules ([`LinuxKernel::change_attr`],
//! [`LinuxKernel::check_access`]).

use crate::abi::Timespec;
use crate::args::Args;
use crate::cred::{self, AttrChange, FileTarget, TimeChange, optional_id};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, UserPtr};
use crate::task::exec::PATH_MAX;
use alloc::vec::Vec;

/// `AT_FDCWD` and the `*at` flags (UAPI `linux/fcntl.h`).
pub const AT_FDCWD: i32 = -100;
pub const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
pub const AT_EACCESS: u32 = 0x200;
pub const AT_EMPTY_PATH: u32 = 0x1000;
/// utimensat(2)'s special nanosecond values (`linux/stat.h`).
pub const UTIME_NOW: i64 = (1 << 30) - 1;
pub const UTIME_OMIT: i64 = (1 << 30) - 2;

/// A path argument, which may be empty (for `AT_EMPTY_PATH`):
/// `ENAMETOOLONG` past `PATH_MAX`, `EFAULT` if unreadable.
fn read_path(k: &dyn LinuxKernel, ptr: UserPtr) -> Result<Vec<u8>, Errno> {
    let mut buf = alloc::vec![0u8; PATH_MAX];
    let n = k.strncpy_from_user(ptr, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// What a `*at` call names: the path from `dirfd`, or with
/// `AT_EMPTY_PATH` and an empty path the file `dirfd` is open on. An
/// empty path without the flag is `ENOENT` (path_resolution(7)).
fn target(dirfd: i32, path: &[u8], flags: u32) -> Result<FileTarget<'_>, Errno> {
    if path.is_empty() {
        return if flags & AT_EMPTY_PATH != 0 { Ok(FileTarget::EmptyPath(dirfd)) } else { Err(Errno::ENOENT) };
    }
    Ok(FileTarget::Path { dirfd, path, follow: flags & AT_SYMLINK_NOFOLLOW == 0 })
}

fn change(k: &mut dyn LinuxKernel, file: FileTarget<'_>, change: AttrChange) -> SysResult {
    let ids = cred::current(k).fs_ids();
    k.change_attr(file, &change, &ids).map(|()| 0)
}

fn mode_change(a: &Args, i: usize) -> AttrChange {
    AttrChange::Mode((a.uint(i) & 0o7777) as u16)
}

fn owner_change(a: &Args, i: usize) -> AttrChange {
    AttrChange::Owner { uid: optional_id(a.uint(i)), gid: optional_id(a.uint(i + 1)) }
}

/// chmod(path, mode): only the permission bits of `mode` (`07777`) count.
pub fn chmod(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = read_path(k, a.ptr(0))?;
    change(k, target(AT_FDCWD, &path, 0)?, mode_change(a, 1))
}

/// fchmod(fd, mode).
pub fn fchmod(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    change(k, FileTarget::Fd(a.fd(0)), mode_change(a, 1))
}

/// fchmodat(dirfd, path, mode): the system call has no flags argument (the
/// C library's `fchmodat()` handles `AT_SYMLINK_NOFOLLOW` itself).
pub fn fchmodat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = read_path(k, a.ptr(1))?;
    change(k, target(a.fd(0), &path, 0)?, mode_change(a, 2))
}

/// chown(path, owner, group): `-1` leaves an id alone; follows a final
/// symbolic link.
pub fn chown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = read_path(k, a.ptr(0))?;
    change(k, target(AT_FDCWD, &path, 0)?, owner_change(a, 1))
}

/// lchown(path, owner, group): the link itself.
pub fn lchown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let path = read_path(k, a.ptr(0))?;
    change(k, target(AT_FDCWD, &path, AT_SYMLINK_NOFOLLOW)?, owner_change(a, 1))
}

/// fchown(fd, owner, group).
pub fn fchown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    change(k, FileTarget::Fd(a.fd(0)), owner_change(a, 1))
}

/// fchownat(dirfd, path, owner, group, flags): `AT_SYMLINK_NOFOLLOW` and
/// `AT_EMPTY_PATH`; other flags `EINVAL` (checked before the path is read).
pub fn fchownat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(4);
    if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
        return Err(Errno::EINVAL);
    }
    let path = read_path(k, a.ptr(1))?;
    change(k, target(a.fd(0), &path, flags)?, owner_change(a, 2))
}

/// The access check shared by access(2), faccessat(2) and faccessat2(2):
/// `EINVAL` for mode bits other than `R_OK | W_OK | X_OK`; the real ids
/// unless `AT_EACCESS` ([`cred::Creds::access_ids`]).
fn access_at(k: &mut dyn LinuxKernel, dirfd: i32, ptr: UserPtr, mode: u32, flags: u32) -> SysResult {
    if flags & !(AT_EACCESS | AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 || mode & !7 != 0 {
        return Err(Errno::EINVAL);
    }
    let path = read_path(k, ptr)?;
    let file = target(dirfd, &path, flags)?;
    let creds = cred::current(k);
    let ids = if flags & AT_EACCESS != 0 { creds.fs_ids() } else { creds.access_ids() };
    k.check_access(file, mode, &ids).map(|()| 0)
}

/// access(path, mode).
pub fn access(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    access_at(k, AT_FDCWD, a.ptr(0), a.uint(1), 0)
}

/// faccessat(dirfd, path, mode): the old system call, without flags.
pub fn faccessat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    access_at(k, a.fd(0), a.ptr(1), a.uint(2), 0)
}

/// faccessat2(dirfd, path, mode, flags): `AT_EACCESS`,
/// `AT_SYMLINK_NOFOLLOW`, `AT_EMPTY_PATH`.
pub fn faccessat2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    access_at(k, a.fd(0), a.ptr(1), a.uint(2), a.uint(3))
}

/// umask(mask): umask(2) "always succeeds and the previous value of the
/// mask is returned"; only `mask & 0777` is used.
pub fn umask(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    k.umask(a.uint(0) & 0o777).map(u64::from)
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
/// `EINVAL`.
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
    let times = AttrChange::Times { atime, mtime };
    if a.ptr(1).is_null() {
        if dirfd == AT_FDCWD {
            return Err(Errno::EFAULT);
        }
        return change(k, FileTarget::Fd(dirfd), times);
    }
    let path = read_path(k, a.ptr(1))?;
    change(k, target(dirfd, &path, flags)?, times)
}
