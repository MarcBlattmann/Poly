//! The older calls that set a file's times by path (ADR 0064): utimes(2)
//! and futimesat(2) take two `struct timeval` (seconds and microseconds),
//! utime(2) a `struct utimbuf` (two seconds counts). They do what
//! utimensat(2) does (`sys::perm`) through the same kernel service: a null
//! times pointer sets both times to the current time; microseconds outside
//! `0..1_000_000` are `EINVAL` (utimes(2)). Final symbolic links are
//! followed; ctime becomes the current time.

use crate::abi::Timespec;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fs::TimeChange;
use crate::fs::at::AT_FDCWD;
use crate::kernel::LinuxKernel;
use crate::sys::path::{fs, nonempty_path};
use crate::table::{Handler, nr};
use crate::time::abi::Timeval;

/// A `struct timeval` of utimes(2) as a time to set.
fn from_timeval(tv: Timeval) -> Result<TimeChange, Errno> {
    if !(0..1_000_000).contains(&tv.usec) {
        return Err(Errno::EINVAL);
    }
    Ok(TimeChange::At(Timespec { sec: tv.sec, nsec: tv.usec * 1000 }))
}

/// The two times at `times` (two `struct timeval`), or "now" for both if
/// it is null.
fn timevals(k: &mut dyn LinuxKernel, a: &Args, i: usize) -> Result<(TimeChange, TimeChange), Errno> {
    if a.ptr(i).is_null() {
        return Ok((TimeChange::Now, TimeChange::Now));
    }
    let mut b = [0u8; 2 * Timeval::SIZE];
    k.copy_from_user(a.ptr(i), &mut b)?;
    let at = |n: usize| {
        let mut t = [0u8; Timeval::SIZE];
        t.copy_from_slice(&b[n * Timeval::SIZE..(n + 1) * Timeval::SIZE]);
        from_timeval(Timeval::from_bytes(&t))
    };
    Ok((at(0)?, at(1)?))
}

/// utimes(path, times[2]): access and modification time in microseconds.
pub fn utimes(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (atime, mtime) = timevals(k, a, 1)?;
    let path = nonempty_path(k, a.ptr(0))?;
    fs(k)?.utimensat(AT_FDCWD, Some(&path), Some(atime), Some(mtime), true).map(|()| 0)
}

/// futimesat(dirfd, path, times[2]): utimes(2) relative to `dirfd`. A
/// null `path` means the file `dirfd` refers to (any descriptor), but not
/// the working directory: with `AT_FDCWD` it is `EFAULT` (futimesat's
/// spec sheet). The times are read and checked first.
pub fn futimesat(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (atime, mtime) = timevals(k, a, 2)?;
    let dirfd = a.fd(0);
    if a.ptr(1).is_null() {
        if dirfd == AT_FDCWD {
            return Err(Errno::EFAULT);
        }
        return fs(k)?.utimensat(dirfd, None, Some(atime), Some(mtime), true).map(|()| 0);
    }
    let path = nonempty_path(k, a.ptr(1))?;
    fs(k)?.utimensat(dirfd, Some(&path), Some(atime), Some(mtime), true).map(|()| 0)
}

/// utime(path, times): `struct utimbuf` is `actime`, `modtime`, whole
/// seconds (`linux/utime.h`).
pub fn utime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (atime, mtime) = if a.ptr(1).is_null() {
        (TimeChange::Now, TimeChange::Now)
    } else {
        let mut b = [0u8; 16];
        k.copy_from_user(a.ptr(1), &mut b)?;
        let sec = |n: usize| i64::from_le_bytes(b[n * 8..n * 8 + 8].try_into().unwrap_or_default());
        (TimeChange::At(Timespec { sec: sec(0), nsec: 0 }), TimeChange::At(Timespec { sec: sec(1), nsec: 0 }))
    };
    let path = nonempty_path(k, a.ptr(0))?;
    fs(k)?.utimensat(AT_FDCWD, Some(&path), Some(atime), Some(mtime), true).map(|()| 0)
}

/// The calls of this module, merged into the table (`table::GROUPS`).
pub const CALLS: &[(u64, Handler)] = &[(nr::UTIMES, utimes), (nr::FUTIMESAT, futimesat), (nr::UTIME, utime)];
