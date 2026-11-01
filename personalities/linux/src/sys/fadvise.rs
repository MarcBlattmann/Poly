//! Page-cache hints: fadvise64(2) (posix_fadvise(2); ADR 0059, GNU `sort`
//! and `cat` give it), readahead(2) and sync_file_range(2) (ADR 0121).
//!
//! The page cache does not take hints and Poly writes back on its own
//! schedule, so a valid call does nothing visible and returns 0: exactly
//! what the three spec sheets observed (data, size and file offset never
//! change). What remains are the checks, in the spec sheets' order:
//!
//! - fadvise64: the descriptor (not open or `O_PATH`: `EBADF`), a pipe
//!   or FIFO (`ESPIPE`), then a negative `len` or an advice outside 0..5
//!   (`EINVAL`; `advice` is a 32-bit int). Any other descriptor works,
//!   whatever its access mode.
//! - readahead: the descriptor (not open, `O_PATH` or not open for
//!   reading: `EBADF`), then anything but a regular file or a `count` of
//!   2^63 or more (`EINVAL`).
//! - sync_file_range: the descriptor (`EBADF`), the arguments (unknown
//!   flag bits, a negative offset or length, an end past `i64::MAX`:
//!   `EINVAL`), then anything but a regular file, a directory or a block
//!   device (`ESPIPE`).

use crate::abi::mode;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::sys::path::O_PATH;
use crate::table::{Handler, nr};

/// The advice values (`linux/fadvise.h`, the x86-64 values).
pub const POSIX_FADV_NORMAL: u32 = 0;
pub const POSIX_FADV_RANDOM: u32 = 1;
pub const POSIX_FADV_SEQUENTIAL: u32 = 2;
pub const POSIX_FADV_WILLNEED: u32 = 3;
pub const POSIX_FADV_DONTNEED: u32 = 4;
pub const POSIX_FADV_NOREUSE: u32 = 5;

/// sync_file_range(2)'s flags (`linux/fs.h`).
pub const SYNC_FILE_RANGE_WAIT_BEFORE: u32 = 1;
pub const SYNC_FILE_RANGE_WRITE: u32 = 2;
pub const SYNC_FILE_RANGE_WAIT_AFTER: u32 = 4;

/// The file type of `fd` after the descriptor check every call starts
/// with: not open (negative numbers included) or `O_PATH` is `EBADF`.
/// Also returns the status flags.
fn open_file(k: &mut dyn LinuxKernel, fd: i32) -> Result<(u32, u32), Errno> {
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let kind = k.fstat(fd)?.file_type();
    // Without the file system services (pipes and sockets only) there
    // are no O_PATH descriptors; take them as open for reading and writing.
    let flags = match k.fs() {
        Some(f) => f.status_flags(fd)?,
        None => 2,
    };
    if flags & O_PATH != 0 {
        return Err(Errno::EBADF);
    }
    Ok((kind, flags))
}

/// Whether `advice` (the low 32 bits) and `len` are valid fadvise64
/// arguments.
pub fn fadvise_args_valid(len: i64, advice: u32) -> bool {
    len >= 0
        && matches!(
            advice,
            POSIX_FADV_NORMAL
                | POSIX_FADV_RANDOM
                | POSIX_FADV_SEQUENTIAL
                | POSIX_FADV_WILLNEED
                | POSIX_FADV_DONTNEED
                | POSIX_FADV_NOREUSE
        )
}

/// fadvise64(fd, offset, len, advice)
pub fn fadvise64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (kind, _) = open_file(k, a.fd(0))?;
    if kind == mode::S_IFIFO {
        return Err(Errno::ESPIPE);
    }
    if !fadvise_args_valid(a.long(2), a.uint(3)) {
        return Err(Errno::EINVAL);
    }
    Ok(0)
}

/// readahead(fd, offset, count)
pub fn readahead(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (kind, flags) = open_file(k, a.fd(0))?;
    // Open for reading: O_RDONLY (0) or O_RDWR (2).
    if !matches!(flags & 3, 0 | 2) {
        return Err(Errno::EBADF);
    }
    if kind != mode::S_IFREG || a.ulong(2) > i64::MAX as u64 {
        return Err(Errno::EINVAL);
    }
    Ok(0)
}

/// Whether sync_file_range's `flags`, `offset` and `nbytes` are valid:
/// known flags only, neither number negative, and the end of the range
/// within `i64::MAX`.
pub fn sync_range_args_valid(offset: i64, nbytes: i64, flags: u32) -> bool {
    let known = SYNC_FILE_RANGE_WAIT_BEFORE | SYNC_FILE_RANGE_WRITE | SYNC_FILE_RANGE_WAIT_AFTER;
    flags & !known == 0 && offset >= 0 && nbytes >= 0 && offset.checked_add(nbytes).is_some()
}

/// sync_file_range(fd, offset, nbytes, flags)
pub fn sync_file_range(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (kind, _) = open_file(k, a.fd(0))?;
    if !sync_range_args_valid(a.long(1), a.long(2), a.uint(3)) {
        return Err(Errno::EINVAL);
    }
    if !matches!(kind, mode::S_IFREG | mode::S_IFDIR | mode::S_IFBLK) {
        return Err(Errno::ESPIPE);
    }
    Ok(0)
}

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] =
    &[(nr::FADVISE64, fadvise64), (nr::READAHEAD, readahead), (nr::SYNC_FILE_RANGE, sync_file_range)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fadvise_arguments() {
        for advice in 0..=5 {
            assert!(fadvise_args_valid(0, advice));
        }
        assert!(fadvise_args_valid(i64::MAX, POSIX_FADV_DONTNEED));
        assert!(!fadvise_args_valid(-1, POSIX_FADV_NORMAL));
        for bad in [6, 7, 9, 100, u32::MAX] {
            assert!(!fadvise_args_valid(0, bad));
        }
    }

    #[test]
    fn sync_range_arguments() {
        for flags in 0..=7 {
            assert!(sync_range_args_valid(0, 0, flags));
        }
        assert!(sync_range_args_valid(0x7fff_ffff_ffff_0000, 0xffff, 2));
        assert!(sync_range_args_valid(0, i64::MAX, 0));
        assert!(!sync_range_args_valid(0x7fff_ffff_ffff_0000, 0x10_0000, 0));
        assert!(!sync_range_args_valid(1, i64::MAX, 0));
        assert!(!sync_range_args_valid(-1, 0, 0));
        assert!(!sync_range_args_valid(0, -1, 0));
        for bad in [8, 0x10, 0x8000_0000, u32::MAX] {
            assert!(!sync_range_args_valid(0, 0, bad));
        }
    }
}
