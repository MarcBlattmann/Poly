//! Vectored I/O at an offset: preadv(2), pwritev(2), preadv2(2),
//! pwritev2(2).
//!
//! preadv and pwritev are readv and writev at an explicit offset (`pos_l`,
//! a signed 64-bit value on x86-64; `pos_h` is ignored), the file offset
//! neither used nor moved. The `2` forms add an offset of -1 for "the file
//! offset" (used and advanced, so pipes work) and `RWF_*` flags for one
//! call. The order of the errors (the spec sheets): (1) the offset
//! (`EINVAL`), (2) the descriptor (`EBADF`: not open, wrong access mode,
//! `O_PATH`), (3) for the `2` forms unknown flags (`EOPNOTSUPP`), (4) an
//! explicit offset on a pipe or socket (`ESPIPE`), (5) the iovec array
//! (`EINVAL`, `EFAULT`), (6) a directory (`EISDIR`, reads), then the
//! transfer (`EFBIG` for `RLIMIT_FSIZE`, `EFAULT` for buffers).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::sys::io::{How, load_iovecs, read_into_how, sigpipe_on_epipe, write_from_how};
use crate::sys::path::{O_PATH, fs};
use crate::table::{Handler, nr};

/// The calls of this module, registered like `table::IMPLEMENTED`.
pub const CALLS: &[(u64, Handler)] =
    &[(nr::PREADV, preadv), (nr::PWRITEV, pwritev), (nr::PREADV2, preadv2), (nr::PWRITEV2, pwritev2)];

/// `RWF_*` (Linux UAPI `linux/fs.h`).
pub const RWF_HIPRI: u32 = 0x1;
pub const RWF_DSYNC: u32 = 0x2;
pub const RWF_SYNC: u32 = 0x4;
pub const RWF_NOWAIT: u32 = 0x8;
pub const RWF_APPEND: u32 = 0x10;
pub const RWF_NOAPPEND: u32 = 0x20;
pub const RWF_ATOMIC: u32 = 0x40;
pub const RWF_DONTCACHE: u32 = 0x80;
pub const RWF_NOSIGNAL: u32 = 0x100;

/// The flags Poly carries out or may ignore. `RWF_ATOMIC` and
/// `RWF_DONTCACHE` are defined but not supported (`EOPNOTSUPP`, as tmpfs
/// answers them; the spec of preadv2: `newer_flags`).
const RWF_SUPPORTED: u32 = RWF_HIPRI | RWF_DSYNC | RWF_SYNC | RWF_NOWAIT | RWF_APPEND | RWF_NOAPPEND | RWF_NOSIGNAL;

/// `O_ACCMODE` values (`asm-generic/fcntl.h`).
const O_RDONLY: u32 = 0;
const O_WRONLY: u32 = 1;
const O_RDWR: u32 = 2;
/// `SEEK_CUR` (`linux/fs.h`).
const SEEK_CUR: u32 = 1;

/// The offset argument: `None` for -1 where it means the file offset
/// (`current_ok`), `EINVAL` for any other negative value.
fn offset_arg(a: &Args, current_ok: bool) -> Result<Option<u64>, Errno> {
    match a.long(3) {
        -1 if current_ok => Ok(None),
        o if o < 0 => Err(Errno::EINVAL),
        o => Ok(Some(o as u64)),
    }
}

/// `EBADF` unless `fd` is open for reading (`write` false) or writing, and
/// not `O_PATH`.
fn check_fd(k: &mut dyn LinuxKernel, fd: i32, write: bool) -> Result<(), Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let flags = fs(k)?.status_flags(fd)?;
    let mode_ok = match flags & 3 {
        O_RDWR => true,
        O_RDONLY => !write,
        O_WRONLY => write,
        _ => false,
    };
    if flags & O_PATH != 0 || !mode_ok { Err(Errno::EBADF) } else { Ok(()) }
}

/// `EOPNOTSUPP` for flags Poly does not support (unknown bits included).
fn check_flags(flags: u32) -> Result<(), Errno> {
    if flags & !RWF_SUPPORTED != 0 { Err(Errno::EOPNOTSUPP) } else { Ok(()) }
}

/// `ESPIPE` when `fd` cannot seek (a pipe, a socket) and the call names an
/// offset.
fn check_seekable(k: &mut dyn LinuxKernel, fd: i32, at: Option<u64>) -> Result<(), Errno> {
    if at.is_some() && fs(k)?.lseek(fd, 0, SEEK_CUR) == Err(Errno::ESPIPE) {
        return Err(Errno::ESPIPE);
    }
    Ok(())
}

fn read_common(k: &mut dyn LinuxKernel, a: &Args, at: Option<u64>, flags: u32) -> SysResult {
    let fd = a.fd(0);
    check_fd(k, fd, false)?;
    check_flags(flags)?;
    check_seekable(k, fd, at)?;
    let count = a.int(2);
    let (iovs, total) = load_iovecs(k, a.ptr(1), count)?;
    if count == 0 {
        return Ok(0);
    }
    if k.fstat(fd)?.file_type() == crate::abi::mode::S_IFDIR {
        return Err(Errno::EISDIR);
    }
    if total == 0 {
        return Ok(0);
    }
    // RWF_HIPRI, RWF_DSYNC, RWF_SYNC, RWF_APPEND and RWF_NOAPPEND change
    // nothing about a read.
    read_into_how(k, fd, &iovs, total, How { at, append: None, nowait: flags & RWF_NOWAIT != 0 })
}

fn write_common(k: &mut dyn LinuxKernel, a: &Args, at: Option<u64>, flags: u32) -> SysResult {
    let fd = a.fd(0);
    check_fd(k, fd, true)?;
    check_flags(flags)?;
    check_seekable(k, fd, at)?;
    let (iovs, total) = load_iovecs(k, a.ptr(1), a.int(2))?;
    if total == 0 {
        return Ok(0);
    }
    let append = if flags & RWF_NOAPPEND != 0 {
        Some(false)
    } else if flags & RWF_APPEND != 0 {
        Some(true)
    } else {
        None
    };
    let how = How { at, append, nowait: flags & RWF_NOWAIT != 0 };
    let result = write_from_how(k, fd, &iovs, total, how);
    // A write with RWF_DSYNC / RWF_SYNC is O_DSYNC / O_SYNC for this call;
    // on files that cannot be synced (pipes, sockets) the flags mean
    // nothing, so the write's result stands.
    if result.is_ok_and(|n| n > 0) && flags & (RWF_DSYNC | RWF_SYNC) != 0 {
        let _ = fs(k)?.fsync(fd, flags & RWF_SYNC == 0);
    }
    if flags & RWF_NOSIGNAL != 0 { result } else { sigpipe_on_epipe(k, result) }
}

/// preadv(fd, iov, iovcnt, pos_l, pos_h)
pub fn preadv(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let at = offset_arg(a, false)?;
    read_common(k, a, at, 0)
}

/// pwritev(fd, iov, iovcnt, pos_l, pos_h): with `O_APPEND` the data goes
/// to the end whatever the offset (as pwrite(2)'s BUGS describe).
pub fn pwritev(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let at = offset_arg(a, false)?;
    write_common(k, a, at, 0)
}

/// preadv2(fd, iov, iovcnt, pos_l, pos_h, flags)
pub fn preadv2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let at = offset_arg(a, true)?;
    read_common(k, a, at, a.uint(5))
}

/// pwritev2(fd, iov, iovcnt, pos_l, pos_h, flags)
pub fn pwritev2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let at = offset_arg(a, true)?;
    write_common(k, a, at, a.uint(5))
}
