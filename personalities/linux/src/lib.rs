//! Linux personality: the x86-64 Linux system call ABI.
//!
//! Calling convention (from the public x86-64 psABI and the kernel's
//! documented user ABI): number in `rax`, arguments in `rdi, rsi, rdx, r10,
//! r8, r9`, result in `rax`, errors returned as `-errno`.
//!
//! How a call flows (ADR 0025):
//!
//! ```text
//! kernel syscall entry ── SyscallRegs ──► Linux::dispatch_with(&mut kernel, regs)
//!     table::handler(rax) ── none ──► rax = -ENOSYS, Outcome::Unimplemented
//!         │ some
//!         ▼
//!     handler(&mut dyn LinuxKernel, &Args) -> Result<u64, Errno>
//!         decode + validate per man page, then call LinuxKernel
//!         (user memory, VFS, VMA manager, thread state, clocks, RNG)
//!         │
//!         ▼
//!     rax = value or -errno, Outcome::Handled
//!     (interrupted by a signal: rax = -EINTR, restart decided below)
//!
//! kernel, before every return to user mode ──► Linux::exit_to_user(&mut kernel)
//!     pending signals: ignore / stop / kill / handler frame; restart or EINTR
//! kernel, when a thread exits ──► Linux::thread_exit(&mut kernel, shares_memory)
//!     robust futex list, clear_child_tid
//! ```
//!
//! - [`table`]: all ~370 x86-64 calls by number and name (generated from the
//!   coverage map, kept in sync by a test) and the handlers of implemented ones.
//! - [`kernel::LinuxKernel`]: the services the kernel provides.
//! - [`args`], [`abi`], [`errno`]: argument decoding, UAPI constants and
//!   structure layouts, error numbers.
//! - [`sys`]: the handlers.
//! - [`signal`]: signal state, delivery, frames (ADR 0031).
//! - [`futex`]: futex wait queues, robust lists (ADR 0031).
//! - [`cred`]: per-thread credentials and capabilities, and the rules that
//!   change them (ADR 0045).
//! - [`fs`]: the file system services the kernel provides
//!   ([`fs::LinuxFs`], through [`LinuxKernel::fs`]) and the file system
//!   calls' structures; [`rlimit`]: resource limits (ADR 0043).
//! - [`fileio`]: what pipes, `poll` / `select`, `epoll` and the event files
//!   (`eventfd`, `timerfd`, `signalfd`) need from the kernel (ADR 0042).
//! - [`time`]: time structures and process timers (ADR 0044).
//! - [`seccomp`]: strict mode and BPF filters checked before every call
//!   (ADR 0054).
//! - [`net`]: what the socket calls need from the kernel
//!   ([`net::LinuxNet`]), socket structures and `NETLINK_ROUTE` (ADR 0048).
//!
//! Implementation status lives in `coverage/linux/syscall/x86_64.json`, not here.

#![no_std]

// Bounce buffers between user memory and the VFS (`sys::io`) and iovec
// arrays are sized by the caller's arguments, up to 64 KiB, which is too
// much for a kernel stack.
extern crate alloc;

pub mod abi;
pub mod admin;
pub mod args;
pub mod cred;
pub mod errno;
pub mod fileio;
pub mod fs;
pub mod futex;
pub mod kernel;
pub mod net;
mod numbers;
pub mod rlimit;
pub mod seccomp;
pub mod signal;
pub mod sys;
pub mod table;
pub mod task;
pub mod time;

#[cfg(test)]
mod tests;

pub use args::Args;
pub use errno::{Errno, SysResult};
pub use kernel::{LinuxKernel, UserPtr};
pub use signal::{ExitToUser, LinuxProcess};
pub use table::nr;

use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};

/// `ENOSYS` from the Linux UAPI headers (`asm-generic/errno.h`).
pub const ENOSYS: u64 = 38;

/// Encode an errno the way Linux returns it in `rax`.
pub const fn err(errno: u64) -> u64 {
    errno.wrapping_neg()
}

/// The name of the call table, as reported in [`Outcome::Unimplemented`].
pub const TABLE: &str = "linux";

#[derive(Debug)]
pub struct Linux;

impl Linux {
    /// Handle one system call for the thread `kernel` stands for.
    ///
    /// Implemented calls run their handler, write the result (or `-errno`)
    /// to `rax` and report [`Outcome::Handled`]; every other number,
    /// including ones beyond the table, gets `-ENOSYS` and
    /// [`Outcome::Unimplemented`]. Only `rax` is written: the other
    /// registers go back to user mode unchanged.
    ///
    /// After `exit` / `exit_group` the kernel has been told to end the
    /// thread or process; it must not resume the thread.
    ///
    /// A call a signal interrupted returns `-EINTR` here; whether it stays
    /// that way or is restarted is decided by [`Linux::exit_to_user`],
    /// which the kernel must call before the thread returns to user mode.
    pub fn dispatch_with(kernel: &mut dyn LinuxKernel, regs: &mut SyscallRegs) -> Outcome {
        let number = regs.rax;
        // seccomp(2): the filters decide before the call runs (ADR 0054).
        if kernel.seccomp_active() && seccomp::check(kernel, regs) == seccomp::Verdict::Skip {
            return Outcome::Handled;
        }
        match table::handler(number) {
            Some(handler) => {
                let args = Args::from_regs(regs);
                let result = match handler(kernel, &args) {
                    Err(e) if e.is_restart() => {
                        signal::deliver::note_restart(kernel, number, e);
                        Err(Errno::EINTR)
                    }
                    other => other,
                };
                regs.rax = errno::encode(result);
                Outcome::Handled
            }
            None => {
                regs.rax = Errno::ENOSYS.to_rax();
                Outcome::Unimplemented { table: TABLE, number }
            }
        }
    }
}

impl Linux {
    /// Called by the kernel before a thread of a Linux process returns to
    /// user mode (after every system call, interrupt and exception): acts
    /// on pending signals and finishes interrupted system calls. See
    /// [`signal::deliver::exit_to_user`]. On [`ExitToUser::Killed`] the
    /// thread must not return to user mode.
    pub fn exit_to_user(kernel: &mut dyn LinuxKernel) -> ExitToUser {
        signal::exit_to_user(kernel)
    }

    /// Called by the kernel when a thread of a Linux process exits, with
    /// `shares_memory` set if its address space has other users: the
    /// robust futex list and `clear_child_tid`. See
    /// [`futex::thread_exit`]. The kernel then removes the thread from
    /// its [`LinuxProcess`].
    pub fn thread_exit(kernel: &mut dyn LinuxKernel, shares_memory: bool) {
        futex::thread_exit(kernel, shares_memory);
    }

    /// Called by the kernel for a fault in user mode that becomes a signal
    /// for the faulting thread (`SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`,
    /// `SIGTRAP`), with `trapno`, `err` and `cr2` set in the thread's
    /// [`kernel::UserContext`]. Delivered by the next
    /// [`exit_to_user`](Self::exit_to_user). [`signal::fault::fault_info`]
    /// says which signal and code a CPU exception raises.
    pub fn fault_signal(kernel: &mut dyn LinuxKernel, info: abi::SigInfo) {
        signal::force_signal(kernel, info);
    }
}

impl SyscallDispatch for Linux {
    const PERSONALITY: Personality = Personality::Linux;

    /// Dispatch without kernel services: there is nothing a call could act
    /// on, so every call returns `-ENOSYS` and reports
    /// [`Outcome::Unimplemented`]. The kernel switches to
    /// [`Linux::dispatch_with`] once it implements [`LinuxKernel`].
    fn dispatch(regs: &mut SyscallRegs) -> Outcome {
        let number = regs.rax;
        regs.rax = err(ENOSYS);
        Outcome::Unimplemented { table: TABLE, number }
    }
}
