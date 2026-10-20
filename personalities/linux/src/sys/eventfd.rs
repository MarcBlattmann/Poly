//! eventfd(2), eventfd2(2): an event counter as a file. Reads, writes,
//! blocking and readiness are the kernel's (`poly_vfs::eventfd`).

use super::pipe::file_io;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{NewFd, flags};
use crate::kernel::LinuxKernel;

/// `EFD_SEMAPHORE` (`sys/eventfd.h`; `EFD_CLOEXEC` and `EFD_NONBLOCK` are
/// `O_CLOEXEC` and `O_NONBLOCK`).
pub const EFD_SEMAPHORE: u32 = 1;

/// eventfd(initval): eventfd2 without flags.
pub fn eventfd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make(k, a.uint(0), 0)
}

/// eventfd2(initval, flags): eventfd(2) "EINVAL An unsupported value was
/// specified in flags".
pub fn eventfd2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make(k, a.uint(0), a.uint(1))
}

fn make(k: &mut dyn LinuxKernel, initval: u32, bits: u32) -> SysResult {
    if bits & !(EFD_SEMAPHORE | flags::O_CLOEXEC | flags::O_NONBLOCK) != 0 {
        return Err(Errno::EINVAL);
    }
    file_io(k)?.eventfd(initval, bits & EFD_SEMAPHORE != 0, NewFd::from_flags(bits)).map(|fd| fd as u64)
}
