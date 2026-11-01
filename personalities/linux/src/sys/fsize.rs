//! `RLIMIT_FSIZE` (ADR 0064): the largest file a process may make
//! (getrlimit(2): "attempts to extend a file beyond this limit result in
//! delivery of a SIGXFSZ signal"). Only regular files are limited.
//!
//! - A write (write, writev, pwrite64) that would take the file past the
//!   limit is cut short at it; one that starts at or past it writes
//!   nothing, fails with `EFBIG` and raises `SIGXFSZ` (to the writing
//!   thread, `SI_KERNEL`, like `SIGPIPE`).
//! - ftruncate past the limit fails with `EFBIG`, without a signal (the
//!   spec of setrlimit names only the writes); truncate also raises
//!   `SIGXFSZ` (the spec of truncate).

use crate::abi::SigInfo;
use crate::abi::mode;
use crate::abi::signal::SI_KERNEL;
use crate::errno::Errno;
use crate::fs::rlimit::{RLIM_INFINITY, RLIMIT_FSIZE};
use crate::kernel::{LinuxKernel, Target};
use crate::signal::deliver::{send, with_own};
use crate::sys::path::fs;

/// `SIGXFSZ` (x86-64 `asm/signal.h`).
pub const SIGXFSZ: u32 = 25;
/// `O_APPEND` (`asm-generic/fcntl.h`).
const O_APPEND: u32 = 0o2000;
/// `SEEK_CUR` (`linux/fs.h`).
const SEEK_CUR: u32 = 1;

/// The calling process's soft `RLIMIT_FSIZE`, if it is not infinite.
fn limit(k: &mut dyn LinuxKernel) -> Option<u64> {
    let cur = with_own(k, |p| p.limits.get(RLIMIT_FSIZE).map(|l| l.cur))?;
    (cur != RLIM_INFINITY).then_some(cur)
}

/// How many of `len` bytes a write to `fd` at `at` (`None`: at the file
/// offset, or at the end with `O_APPEND`) may write under the limit. With
/// none, `EFBIG` and `SIGXFSZ`.
pub fn allowed(k: &mut dyn LinuxKernel, fd: i32, at: Option<u64>, len: u64) -> Result<u64, Errno> {
    allowed_with(k, fd, at, len, None)
}

/// [`allowed`] for a write that appends (`append: Some(true)`, pwritev2's
/// `RWF_APPEND`) or does not (`Some(false)`, `RWF_NOAPPEND`) whatever the
/// descriptor's `O_APPEND` says.
pub fn allowed_with(
    k: &mut dyn LinuxKernel,
    fd: i32,
    at: Option<u64>,
    len: u64,
    append: Option<bool>,
) -> Result<u64, Errno> {
    let Some(limit) = limit(k) else { return Ok(len) };
    let Ok(st) = k.fstat(fd) else { return Ok(len) };
    if st.file_type() != mode::S_IFREG || len == 0 {
        return Ok(len);
    }
    let size = u64::try_from(st.size).unwrap_or(0);
    // A descriptor not open for writing fails as it would anyway (EBADF).
    let Some(flags) = fs(k).ok().and_then(|f| f.status_flags(fd).ok()) else { return Ok(len) };
    if !matches!(flags & 3, 1 | 2) {
        return Ok(len);
    }
    let append = append.unwrap_or(flags & O_APPEND != 0);
    let pos = match at {
        _ if append => size,
        Some(at) => at,
        None => match fs(k).ok().and_then(|f| f.lseek(fd, 0, SEEK_CUR).ok()) {
            Some(pos) => pos,
            None => return Ok(len),
        },
    };
    if pos >= limit {
        let tid = k.tid();
        let _ = send(k, Target::Thread(tid), Some(tid), None, SigInfo::new(SIGXFSZ, SI_KERNEL));
        return Err(Errno::EFBIG);
    }
    Ok(len.min(limit - pos))
}

/// truncate(2) or fallocate(2) of a file past the limit: `EFBIG`, and
/// `SIGXFSZ` to the calling thread as for a write (the specs of truncate
/// and fallocate: `rlimit_fsize`, `too_big`).
pub fn too_big_signal(k: &mut dyn LinuxKernel) -> Result<u64, Errno> {
    let tid = k.tid();
    let _ = send(k, Target::Thread(tid), Some(tid), None, SigInfo::new(SIGXFSZ, SI_KERNEL));
    Err(Errno::EFBIG)
}

/// A truncation of a file to `len` bytes: `EFBIG` past the limit.
pub fn check_truncate(k: &mut dyn LinuxKernel, len: u64) -> Result<(), Errno> {
    match limit(k) {
        Some(limit) if len > limit => Err(Errno::EFBIG),
        _ => Ok(()),
    }
}
