//! fallocate(2): allocate, or punch a hole in, a byte range of a regular
//! file open for writing.
//!
//! The order of the errors (the spec of fallocate): (1) `EBADF` for a
//! descriptor that is not open (`O_PATH` counts as not open); (2) `EINVAL`
//! for a negative offset or a length that is not positive; (3)
//! `EOPNOTSUPP` for bits no kernel defines and `PUNCH_HOLE` without
//! `KEEP_SIZE`; (4) `EBADF` for a descriptor not open for writing; (5)
//! `ESPIPE` on a pipe, `ENODEV` on anything else but a regular file; (6)
//! `EFBIG` past the largest file size or (allocating) past
//! `RLIMIT_FSIZE`, with `SIGXFSZ`; then the file system's `ENOSPC` and
//! `EOPNOTSUPP` for the modes it does not support (tmpfs: all but 0,
//! `KEEP_SIZE` and `KEEP_SIZE | PUNCH_HOLE`).

use crate::abi::mode;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::sys::path::{O_PATH, fs};
use crate::table::{Handler, nr};

/// The calls of this module, registered like `table::IMPLEMENTED`.
pub const CALLS: &[(u64, Handler)] = &[(nr::FALLOCATE, fallocate)];

/// `FALLOC_FL_*` (Linux UAPI `linux/falloc.h`).
pub const FALLOC_FL_KEEP_SIZE: u32 = 0x01;
pub const FALLOC_FL_PUNCH_HOLE: u32 = 0x02;
pub const FALLOC_FL_NO_HIDE_STALE: u32 = 0x04;
pub const FALLOC_FL_COLLAPSE_RANGE: u32 = 0x08;
pub const FALLOC_FL_ZERO_RANGE: u32 = 0x10;
pub const FALLOC_FL_INSERT_RANGE: u32 = 0x20;
pub const FALLOC_FL_UNSHARE_RANGE: u32 = 0x40;

/// Every mode bit a kernel defines.
const FALLOC_FL_KNOWN: u32 = FALLOC_FL_KEEP_SIZE
    | FALLOC_FL_PUNCH_HOLE
    | FALLOC_FL_NO_HIDE_STALE
    | FALLOC_FL_COLLAPSE_RANGE
    | FALLOC_FL_ZERO_RANGE
    | FALLOC_FL_INSERT_RANGE
    | FALLOC_FL_UNSHARE_RANGE;

/// The largest file size (`MAX_LFS_FILESIZE`, `linux/fs.h`: `LLONG_MAX`
/// on 64-bit), the same as `poly_vfs::MAX_FILE_SIZE`.
const MAX_FILE_SIZE: u64 = i64::MAX as u64;

/// fallocate(fd, mode, offset, len)
pub fn fallocate(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (fd, falloc_mode, offset, len) = (a.fd(0), a.uint(1), a.long(2), a.long(3));
    // (1) A descriptor that is not open, or open with O_PATH.
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let flags = fs(k)?.status_flags(fd)?;
    if flags & O_PATH != 0 {
        return Err(Errno::EBADF);
    }
    // (2) The range.
    if offset < 0 || len <= 0 {
        return Err(Errno::EINVAL);
    }
    let (offset, len) = (offset as u64, len as u64);
    // (3) Modes no file system can carry out.
    if falloc_mode & !FALLOC_FL_KNOWN != 0
        || (falloc_mode & FALLOC_FL_PUNCH_HOLE != 0 && falloc_mode & FALLOC_FL_KEEP_SIZE == 0)
    {
        return Err(Errno::EOPNOTSUPP);
    }
    // (4) Open for writing.
    if !matches!(flags & 3, 1 | 2) {
        return Err(Errno::EBADF);
    }
    // (5) The file's type.
    match k.fstat(fd)?.file_type() {
        mode::S_IFREG => {}
        mode::S_IFIFO => return Err(Errno::ESPIPE),
        _ => return Err(Errno::ENODEV),
    }
    // (6) The size.
    let end = offset.checked_add(len).filter(|&e| e <= MAX_FILE_SIZE).ok_or(Errno::EFBIG)?;
    if falloc_mode & FALLOC_FL_PUNCH_HOLE == 0 && crate::sys::fsize::check_truncate(k, end).is_err() {
        return crate::sys::fsize::too_big_signal(k);
    }
    fs(k)?.fallocate(fd, falloc_mode, offset, len).map(|()| 0)
}
