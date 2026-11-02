//! The native personality: Poly's own system call interface, for programs
//! written for Poly itself, such as the desktop (ADR 0124, ADR 0125).
//!
//! A native program is an x86-64 ELF executable with Poly's ABI note
//! (`poly-binfmt`), built against `poly-rt`. Its calls follow
//! [`abi`]: number in `rax`, arguments in `rdi, rsi, rdx, r10, r8, r9`,
//! the value or `-error` back in `rax`.
//!
//! ```text
//! kernel syscall entry ── SyscallRegs ──► Native::dispatch_with(&mut kernel, regs)
//!     table::handler(rax) ── none ──► rax = -Unsupported, Outcome::Unimplemented
//!         │ some
//!         ▼
//!     handler(&mut dyn NativeKernel, &Args) -> Result<u64, Error>
//!         check and decode, copy user memory, call NativeKernel
//!         │
//!         ▼
//!     rax = value or -error, Outcome::Handled
//! ```
//!
//! - [`abi`]: numbers, errors, flags and structures, shared with `poly-rt`.
//! - [`kernel::NativeKernel`]: the services the kernel provides. Handles are
//!   the kernel's shared open files, so native and Linux processes can hold
//!   the same pipe, socket or device.
//! - [`sys`]: the handlers; [`table`]: numbers to handlers and names.

#![no_std]

extern crate alloc;

pub mod abi;
pub mod kernel;
pub mod sys;
pub mod table;

#[cfg(test)]
mod tests;

pub use abi::Error;
pub use kernel::NativeKernel;

use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};

/// The name of the call table, as reported in [`Outcome::Unimplemented`].
pub const TABLE: &str = "native";

#[derive(Debug)]
pub struct Native;

impl Native {
    /// Handle one system call for the thread `kernel` stands for. Only
    /// `rax` is written. After `exit` the kernel has been told to end the
    /// process; it must not resume the thread.
    pub fn dispatch_with(kernel: &mut dyn NativeKernel, regs: &mut SyscallRegs) -> Outcome {
        let number = regs.rax;
        match table::handler(number) {
            Some(handler) => {
                let args = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9];
                regs.rax = abi::encode(handler(kernel, &args));
                Outcome::Handled
            }
            None => {
                regs.rax = abi::encode(Err(Error::Unsupported));
                Outcome::Unimplemented { table: TABLE, number }
            }
        }
    }
}

/// Without the kernel's services (a bare process record in the kernel's
/// tests) no call can run: every one is unsupported.
impl SyscallDispatch for Native {
    const PERSONALITY: Personality = Personality::Native;

    fn dispatch(regs: &mut SyscallRegs) -> Outcome {
        let number = regs.rax;
        regs.rax = abi::encode(Err(Error::Unsupported));
        Outcome::Unimplemented { table: TABLE, number }
    }
}
