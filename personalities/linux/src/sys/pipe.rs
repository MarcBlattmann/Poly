//! pipe(2), pipe2(2).
//!
//! The pipe itself (buffer, ends, `PIPE_BUF` atomicity, end of file,
//! `EPIPE`) is the kernel's ([`FileIo::pipe`](crate::fileio::FileIo::pipe));
//! `SIGPIPE` for a write without readers is raised in `sys::io`.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{FileIo, NewFd, flags};
use crate::kernel::{LinuxKernel, UserPtr};

/// The kernel's [`FileIo`], `ENOSYS` without one.
pub(crate) fn file_io(k: &mut dyn LinuxKernel) -> Result<&mut dyn FileIo, Errno> {
    k.file_io().ok_or(Errno::ENOSYS)
}

/// pipe(pipefd)
pub fn pipe(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make_pipe(k, a.ptr(0), 0)
}

/// pipe2(pipefd, flags): pipe(2): `O_CLOEXEC`, `O_DIRECT` (packet mode)
/// and `O_NONBLOCK` (on both open file descriptions); anything else is
/// `EINVAL` ("Invalid value in flags"). `O_NOTIFICATION_PIPE` is not
/// supported.
pub fn pipe2(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make_pipe(k, a.ptr(0), a.uint(1))
}

fn make_pipe(k: &mut dyn LinuxKernel, pipefd: UserPtr, bits: u32) -> SysResult {
    if bits & !(flags::O_CLOEXEC | flags::O_DIRECT | flags::O_NONBLOCK) != 0 {
        return Err(Errno::EINVAL);
    }
    let [r, w] = file_io(k)?.pipe(NewFd::from_flags(bits), bits & flags::O_DIRECT != 0)?;
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&r.to_le_bytes());
    out[4..].copy_from_slice(&w.to_le_bytes());
    if let Err(e) = k.copy_to_user(pipefd, &out) {
        // pipe(2): "EFAULT pipefd is not valid": no descriptors are left
        // behind (another thread may have seen them meanwhile).
        let _ = k.close(r);
        let _ = k.close(w);
        return Err(e);
    }
    Ok(0)
}
