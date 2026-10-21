//! fadvise64(2) (`posix_fadvise(2)`): advice about how a file's data will
//! be used.
//!
//! The advice is a hint: posix_fadvise(2) says it "does not affect the
//! semantics of the application" and Poly takes all of it without acting
//! on it (the page cache's read-ahead does not change with it). What is
//! checked, as the man page's ERRORS list it:
//!
//! - `EBADF`: `fd` is not an open file descriptor (an `O_PATH` descriptor
//!   is not open for I/O, so it is `EBADF` too);
//! - `ESPIPE`: `fd` refers to a pipe or FIFO ("ESPIPE is the error
//!   specified by POSIX");
//! - `EINVAL`: an unknown `advice`, or a negative `len` (POSIX: "the value
//!   of len is less than zero").
//!
//! A `len` of 0 means "to the end of the file", whatever `offset` is; a
//! range past the end is fine.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::sys::path::fs;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::FADVISE64, fadvise64)];

/// The advice values (UAPI `linux/fadvise.h`, the values of every
/// architecture but s390).
pub mod advice {
    pub const POSIX_FADV_NORMAL: u32 = 0;
    pub const POSIX_FADV_RANDOM: u32 = 1;
    pub const POSIX_FADV_SEQUENTIAL: u32 = 2;
    pub const POSIX_FADV_WILLNEED: u32 = 3;
    pub const POSIX_FADV_DONTNEED: u32 = 4;
    pub const POSIX_FADV_NOREUSE: u32 = 5;
}

/// `O_PATH` (`asm-generic/fcntl.h`).
const O_PATH: u32 = 0o10000000;

/// Whether `value` is one of the advice values of [`advice`].
pub const fn valid_advice(value: i32) -> bool {
    matches!(value, 0..=5)
}

/// fadvise64(fd, offset, len, advice)
pub fn fadvise64(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = a.fd(0);
    if fd < 0 {
        return Err(Errno::EBADF);
    }
    let f = fs(k)?;
    let st = f.statat(fd, b"", true)?.stat;
    if f.status_flags(fd)? & O_PATH != 0 {
        return Err(Errno::EBADF);
    }
    if st.file_type() == crate::abi::mode::S_IFIFO {
        return Err(Errno::ESPIPE);
    }
    if !valid_advice(a.int(3)) || a.long(2) < 0 {
        return Err(Errno::EINVAL);
    }
    Ok(0)
}
