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
//! ```
//!
//! - [`table`]: all ~370 x86-64 calls by number and name (generated from the
//!   coverage map, kept in sync by a test) and the handlers of implemented ones.
//! - [`kernel::LinuxKernel`]: the services the kernel provides.
//! - [`args`], [`abi`], [`errno`]: argument decoding, UAPI constants and
//!   structure layouts, error numbers.
//! - [`sys`]: the handlers.
//!
//! Implementation status lives in `coverage/linux/syscall/x86_64.json`, not here.

#![no_std]

// Bounce buffers between user memory and the VFS (`sys::io`) and iovec
// arrays are sized by the caller's arguments, up to 64 KiB, which is too
// much for a kernel stack.
extern crate alloc;

pub mod abi;
pub mod args;
pub mod errno;
pub mod kernel;
mod numbers;
pub mod sys;
pub mod table;

#[cfg(test)]
mod tests;

pub use args::Args;
pub use errno::{Errno, SysResult};
pub use kernel::{LinuxKernel, UserPtr};
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
    pub fn dispatch_with(kernel: &mut dyn LinuxKernel, regs: &mut SyscallRegs) -> Outcome {
        let number = regs.rax;
        match table::handler(number) {
            Some(handler) => {
                let args = Args::from_regs(regs);
                regs.rax = errno::encode(handler(kernel, &args));
                Outcome::Handled
            }
            None => {
                regs.rax = Errno::ENOSYS.to_rax();
                Outcome::Unimplemented { table: TABLE, number }
            }
        }
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
