//! The ioctl(2) requests every descriptor answers, whatever its file:
//! `FIONBIO` (set or clear `O_NONBLOCK` from an int argument; ioctl's
//! spec sheet: on a pty slave too), `FIOCLEX` / `FIONCLEX` (set or clear
//! close-on-exec). Request numbers from `asm-generic/ioctls.h`.

use crate::args::Args;
use crate::errno::SysResult;
use crate::fileio::flags::O_NONBLOCK;
use crate::kernel::LinuxKernel;
use crate::sys::path::fs;

pub const FIONBIO: u32 = 0x5421;
pub const FIONCLEX: u32 = 0x5450;
pub const FIOCLEX: u32 = 0x5451;

/// Answer `request` on `fd` if it is one of these; `None` otherwise.
pub fn generic(k: &mut dyn LinuxKernel, fd: i32, request: u32, a: &Args) -> Option<SysResult> {
    Some(match request {
        FIONBIO => fionbio(k, fd, a),
        FIOCLEX | FIONCLEX => fs(k).and_then(|fs| fs.set_cloexec(fd, request == FIOCLEX)).map(|()| 0),
        _ => return None,
    })
}

fn fionbio(k: &mut dyn LinuxKernel, fd: i32, a: &Args) -> SysResult {
    let mut b = [0u8; 4];
    k.copy_from_user(a.ptr(2), &mut b)?;
    let on = i32::from_le_bytes(b) != 0;
    let fs = fs(k)?;
    let flags = fs.status_flags(fd)?;
    let flags = if on { flags | O_NONBLOCK } else { flags & !O_NONBLOCK };
    fs.set_status_flags(fd, flags).map(|()| 0)
}
