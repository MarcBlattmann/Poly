//! fadvise64(2) (posix_fadvise(2)): advice about how a file will be read
//! (ADR 0057; GNU `sort` and `cat` give it).
//!
//! The advice is only a hint for the page cache, whose read-ahead does not
//! take hints yet, so a valid call does nothing and succeeds. The checks
//! are posix_fadvise(2)'s ERRORS: "EBADF The fd argument was not a valid
//! file descriptor", "EINVAL An invalid value was specified for advice",
//! "ESPIPE The specified file descriptor refers to a pipe or FIFO". The
//! order (descriptor, then file type, then advice) is ours, to be
//! confirmed by the diff runner. A negative `len` is accepted: the page
//! says nothing about it; the offset and length only bound the range the
//! hint is about.

use crate::abi::mode;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

/// The advice values (`linux/fadvise.h`, the x86-64 values).
pub const POSIX_FADV_NORMAL: u32 = 0;
pub const POSIX_FADV_RANDOM: u32 = 1;
pub const POSIX_FADV_SEQUENTIAL: u32 = 2;
pub const POSIX_FADV_WILLNEED: u32 = 3;
pub const POSIX_FADV_DONTNEED: u32 = 4;
pub const POSIX_FADV_NOREUSE: u32 = 5;

/// fadvise64(fd, offset, len, advice)
pub fn fadvise64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let st = k.fstat(fd)?;
    if st.file_type() == mode::S_IFIFO {
        return Err(Errno::ESPIPE);
    }
    match a.uint(3) {
        POSIX_FADV_NORMAL | POSIX_FADV_RANDOM | POSIX_FADV_SEQUENTIAL | POSIX_FADV_WILLNEED
        | POSIX_FADV_DONTNEED | POSIX_FADV_NOREUSE => Ok(0),
        _ => Err(Errno::EINVAL),
    }
}

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::FADVISE64, fadvise64)];
