//! x64 structured exception handling (SEH): the data an x64 PE image
//! describes its functions' stack frames with, the virtual unwinder that
//! reads it, and the walks exception dispatch and unwinding make over a
//! thread's stack.
//!
//! Everything here is from public Microsoft documentation (Learn: "x64
//! exception handling", "Structured Exception Handling (C/C++)", the
//! `RtlVirtualUnwind`, `RtlLookupFunctionEntry`, `RtlUnwindEx`,
//! `RtlCaptureContext` and `RaiseException` reference pages, the
//! `EXCEPTION_RECORD`, `CONTEXT` and `EXCEPTION_POINTERS` pages, and the PE
//! Format specification's ".pdata section") and the layouts and constants
//! of the public SDK headers `winnt.h` and `excpt.h`. Nothing comes from a
//! Windows binary or from another implementation. Where the documentation
//! leaves a behaviour open, the code says what it picks and why
//! (docs/adr/0068).
//!
//! # Layout
//!
//! - [`pdata`]: `RUNTIME_FUNCTION` entries of an image's exception
//!   directory, and finding the one that covers an address by binary search
//!   (`RtlLookupFunctionEntry`).
//! - [`info`]: `UNWIND_INFO` and its `UNWIND_CODE` array, decoded into
//!   [`info::UnwindOp`]s (every documented `UWOP_*`), with the exception
//!   handler and its language-specific data, or chained unwind info.
//! - [`unwind`]: the virtual unwinder (`RtlVirtualUnwind`): a [`Context`]
//!   in one function becomes the context of its caller, whether the address
//!   is in the prolog, the body or an epilog.
//! - [`context`], [`record`]: the x64 `CONTEXT`, `EXCEPTION_RECORD` and
//!   `DISPATCHER_CONTEXT` as plain values, and their `winnt.h` byte layouts.
//! - [`dispatch`]: the two passes of SEH over a stack — the search for a
//!   handler (`RtlDispatchException`), the unwind to a target frame
//!   (`RtlUnwindEx`) — and a plain stack walk for backtraces. Handlers are
//!   called through a trait, so the walks are tested on the host and our
//!   `ntdll` will call real code through the same trait.
//!
//! The crate never touches memory itself: everything it reads (unwind data,
//! code bytes, the stack) goes through [`Memory`], which the kernel
//! implements with its user-copy routines and the tests with byte maps. No
//! input makes it panic: malformed tables, truncated unwind codes,
//! unreadable memory and loops in chained unwind info are all errors.

#![no_std]

extern crate alloc;

pub mod consts;
pub mod context;
pub mod dispatch;
pub mod info;
pub mod pdata;
pub mod record;
pub mod unwind;

#[cfg(test)]
mod tests;

pub use context::Context;
pub use dispatch::{Disposition, FunctionEntry, Handlers, Module, Modules, StackLimits, Target};
pub use info::{UnwindInfo, UnwindOp};
pub use pdata::RuntimeFunction;
pub use record::{DispatcherContext, ExceptionRecord};
pub use unwind::{ContextPointers, Handler, Unwound, virtual_unwind};

/// An address that could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadAddress(pub u64);

/// Where the unwinder reads unwind data, code and the stack.
pub trait Memory {
    /// Fill `buf` from `addr`; fail if any of it cannot be read.
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress>;
}

impl<M: Memory + ?Sized> Memory for &M {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        (**self).read(addr, buf)
    }
}

/// A byte slice seen at address `base`: unwind data parsed straight from
/// an image file, or a test's memory.
#[derive(Clone, Copy, Debug)]
pub struct SliceMemory<'a> {
    pub base: u64,
    pub bytes: &'a [u8],
}

impl Memory for SliceMemory<'_> {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        let start = addr.checked_sub(self.base).ok_or(BadAddress(addr))?;
        let start = usize::try_from(start).map_err(|_| BadAddress(addr))?;
        let end = start.checked_add(buf.len()).ok_or(BadAddress(addr))?;
        let src = self.bytes.get(start..end).ok_or(BadAddress(addr))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

/// Little-endian reads through a [`Memory`].
pub(crate) fn read_u8(mem: &dyn Memory, addr: u64) -> Result<u8, UnwindError> {
    let mut b = [0u8; 1];
    mem.read(addr, &mut b).map_err(|e| UnwindError::Unreadable(e.0))?;
    Ok(b[0])
}

pub(crate) fn read_u16(mem: &dyn Memory, addr: u64) -> Result<u16, UnwindError> {
    let mut b = [0u8; 2];
    mem.read(addr, &mut b).map_err(|e| UnwindError::Unreadable(e.0))?;
    Ok(u16::from_le_bytes(b))
}

pub(crate) fn read_u32(mem: &dyn Memory, addr: u64) -> Result<u32, UnwindError> {
    let mut b = [0u8; 4];
    mem.read(addr, &mut b).map_err(|e| UnwindError::Unreadable(e.0))?;
    Ok(u32::from_le_bytes(b))
}

pub(crate) fn read_u64(mem: &dyn Memory, addr: u64) -> Result<u64, UnwindError> {
    let mut b = [0u8; 8];
    mem.read(addr, &mut b).map_err(|e| UnwindError::Unreadable(e.0))?;
    Ok(u64::from_le_bytes(b))
}

pub(crate) fn read_u128(mem: &dyn Memory, addr: u64) -> Result<u128, UnwindError> {
    let mut b = [0u8; 16];
    mem.read(addr, &mut b).map_err(|e| UnwindError::Unreadable(e.0))?;
    Ok(u128::from_le_bytes(b))
}

/// Why unwind data could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindError {
    /// Memory (unwind data, code or the stack) could not be read here.
    Unreadable(u64),
    /// `UNWIND_INFO.Version` is not 1, the only version Learn documents.
    BadVersion(u8),
    /// Undefined `UNWIND_INFO.Flags` bits, or `UNW_FLAG_CHAININFO` together
    /// with a handler flag (Learn: "If this flag is set, then the
    /// UNW_FLAG_EHANDLER and UNW_FLAG_UHANDLER flags must be cleared").
    BadFlags(u8),
    /// An unwind code with an undocumented operation or operation info, at
    /// this slot of the array.
    BadCode { slot: usize },
    /// An unwind code needs more slots than `CountOfCodes` has.
    TruncatedCodes,
    /// `UWOP_SET_FPREG`, or an epilog's `lea rsp`, in a function without a
    /// frame register.
    NoFrameRegister,
    /// Chained unwind info that does not end (a loop, or deeper than
    /// [`MAX_CHAIN`]).
    ChainTooDeep,
    /// `UNWIND_INFO` that is not DWORD aligned (Learn: "The UNWIND_INFO
    /// structure must be DWORD aligned in memory").
    Misaligned(u64),
    /// Address arithmetic left the 64-bit space.
    Overflow,
    /// The address is not in the function entry it was given.
    NotInFunction,
    /// An exception directory whose size is not a multiple of a
    /// `RUNTIME_FUNCTION`, or an entry that ends before it begins.
    BadTable,
}

/// The most chained `UNWIND_INFO` structures one unwind follows.
pub const MAX_CHAIN: usize = 32;
