//! signalfd(2), signalfd4(2): a file to read pending signals from, and the
//! `struct signalfd_siginfo` a read returns.
//!
//! The kernel's signal file takes the reading thread's pending signals of
//! its mask out of the personality's [`LinuxProcess`](crate::LinuxProcess)
//! (`dequeue_from`, as rt_sigtimedwait does: blocked or not) and encodes
//! each with [`siginfo_bytes`]. A signal queued for a process wakes the
//! threads waiting on signal files ([`LinuxKernel::signal_queued`]).

use super::pipe::file_io;
use crate::abi::SigInfo;
use crate::abi::signal::{SIGBUS, SIGCHLD, SIGFPE, SIGILL, SIGIO, SIGSEGV, SIGSET_SIZE, SIGSYS, SIGTRAP};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::{NewFd, flags};
use crate::kernel::{LinuxKernel, SigSet};

/// `sizeof(struct signalfd_siginfo)` (`linux/signalfd.h`).
pub const SIGNALFD_SIGINFO_SIZE: usize = 128;

/// `SI_TIMER` (`asm-generic/siginfo.h`).
const SI_TIMER: i32 = -2;

/// signalfd(fd, mask, sizemask): signalfd4 without flags.
pub fn signalfd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make(k, a, 0)
}

/// signalfd4(fd, mask, sizemask, flags): signalfd(2): `EINVAL` for flags
/// other than `SFD_CLOEXEC` / `SFD_NONBLOCK` or a `sizemask` other than
/// `sizeof(sigset_t)` (8), `EFAULT` for `mask`; `fd` -1 makes a new file,
/// otherwise it must be a signal file (`EBADF`, `EINVAL`) and gets the
/// new mask. "It is not possible to receive SIGKILL or SIGSTOP signals via
/// a signalfd file descriptor; these signals are silently ignored if
/// specified in mask."
pub fn signalfd4(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    make(k, a, a.uint(3))
}

fn make(k: &mut dyn LinuxKernel, a: &Args, bits: u32) -> SysResult {
    if bits & !(flags::O_CLOEXEC | flags::O_NONBLOCK) != 0 || a.ulong(2) != SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let fd = a.fd(0);
    if fd < -1 {
        return Err(Errno::EBADF);
    }
    let mut raw = [0; 8];
    k.copy_from_user(a.ptr(1), &mut raw)?;
    let mask = SigSet(u64::from_le_bytes(raw)).without(SigSet::UNBLOCKABLE);
    file_io(k)?.signalfd(fd, mask, NewFd::from_flags(bits)).map(|fd| fd as u64)
}

/// `struct signalfd_siginfo` (`linux/signalfd.h`) for a dequeued signal.
/// Which `siginfo_t` members carry information depends on the signal and
/// its code (sigaction(2), "The siginfo_t argument"): sent by a process
/// (code ≤ 0): `si_pid`, `si_uid` and the value (`ssi_int`, `ssi_ptr`), or
/// for `SI_TIMER` the timer id and overrun; from the kernel (code > 0):
/// `SIGCHLD`'s pid, uid, status and times, a fault's address (and
/// `si_addr_lsb`), `SIGIO`'s band and fd, `SIGSYS`'s call address, system
/// call and architecture.
pub fn siginfo_bytes(info: &SigInfo) -> [u8; SIGNALFD_SIGINFO_SIZE] {
    let f = &info.fields;
    let u32_at = |o: usize| u32::from_le_bytes([f[o], f[o + 1], f[o + 2], f[o + 3]]);
    let u64_at = |o: usize| crate::abi::le_u64(&f[o..o + 8]);
    let mut b = [0u8; SIGNALFD_SIGINFO_SIZE];
    put32(&mut b, 0, info.signo as u32);
    put32(&mut b, 4, info.errno as u32);
    put32(&mut b, 8, info.code as u32);
    let signal = info.signal();
    if info.code == SI_TIMER {
        // _timer: si_tid, si_overrun, si_value.
        put32(&mut b, 24, u32_at(0));
        put32(&mut b, 32, u32_at(4));
        put32(&mut b, 44, u32_at(8));
        put64(&mut b, 48, u64_at(8));
    } else if info.code <= 0 {
        // _kill / _rt: si_pid, si_uid, si_value.
        put32(&mut b, 12, u32_at(0));
        put32(&mut b, 16, u32_at(4));
        put32(&mut b, 44, u32_at(8));
        put64(&mut b, 48, u64_at(8));
    } else if signal == SIGCHLD {
        // _sigchld: si_pid, si_uid, si_status, si_utime, si_stime.
        put32(&mut b, 12, u32_at(0));
        put32(&mut b, 16, u32_at(4));
        put32(&mut b, 40, u32_at(8));
        put64(&mut b, 56, u64_at(16));
        put64(&mut b, 64, u64_at(24));
    } else if [SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP].contains(&signal) {
        // _sigfault: si_addr, then si_addr_lsb.
        put64(&mut b, 72, u64_at(0));
        b[80..82].copy_from_slice(&f[8..10]);
    } else if signal == SIGIO {
        // _sigpoll: si_band (a long), si_fd.
        put32(&mut b, 28, u64_at(0) as u32);
        put32(&mut b, 20, u32_at(8));
    } else if signal == SIGSYS {
        // _sigsys: si_call_addr, si_syscall, si_arch.
        put64(&mut b, 88, u64_at(0));
        put32(&mut b, 84, u32_at(8));
        put32(&mut b, 96, u32_at(12));
    }
    b
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
