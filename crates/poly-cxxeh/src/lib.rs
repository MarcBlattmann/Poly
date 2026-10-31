//! C++ exception handling as msvcrt's `__CxxFrameHandler3`,
//! `__CxxFrameHandler` and `_CxxThrowException` do it on x64 (ADR 0099):
//! everything that does not touch the running process.
//!
//! The behaviour is the spec's (`specs/windows/msvcrt.dll/
//! {__CxxFrameHandler3,__CxxFrameHandler,_CxxThrowException}.yaml`, handed
//! over by `spec-cxxeh`), the table layouts are the ones the spec sheets
//! give (as the compiler writes them), the record and dispatcher layouts
//! and constants are `winnt.h`'s, `excpt.h`'s and `ntstatus.h`'s, and the
//! handler interface is Learn's "x64 exception handling". Nothing comes
//! from another implementation of these handlers.
//!
//! # Layout
//!
//! - [`consts`]: magic numbers, flags, adjectives and properties.
//! - [`tables`]: a function's `FuncInfo` and the tables it points at (the
//!   unwind map, the try block map, the handler arrays, the IP-to-state
//!   map), read through [`Memory`].
//! - [`throw`]: the exception record of a `throw`, its `ThrowInfo` and
//!   catchable types, matching a catch clause, the address adjustment to a
//!   base class, and what the catch object is made of.
//! - [`frame`]: the decisions for one frame: which kind of frame it is
//!   (the function, a catch block, an unwind action), which try blocks
//!   apply, whether a clause takes the exception (or `terminate` must be
//!   called), and which unwind actions run between two states.
//! - [`catches`]: the per-thread list of catch blocks in progress: the
//!   state of a frame whose catch block runs, the exception a rethrow
//!   means, and when an exception object is destroyed.
//!
//! The msvcrt glue (`personalities/nt/win32/msvcrt/src/cxx.rs`) reads the
//! real record and dispatcher context, calls these functions, and does what
//! they decide: copies, calls of funclets and destructors, and the unwind
//! through `ntdll`'s `RtlUnwindEx`.
//!
//! No input makes the crate panic: unreadable memory, impossible states,
//! counts and offsets are errors ([`Bad`]); every loop is bounded.

#![no_std]

extern crate alloc;

pub mod catches;
pub mod consts;
pub mod frame;
pub mod tables;
pub mod throw;

#[cfg(test)]
mod tests;

pub use poly_seh::{BadAddress, Memory, SliceMemory};

/// Why the tables or the exception could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// Memory could not be read here.
    Unreadable(u64),
    /// A FuncInfo magic number outside `0x19930520..=0x19930522`.
    Magic(u32),
    /// A state outside the unwind map.
    State(i32),
    /// An index outside its table (try block, handler, catchable type).
    Index(u32),
    /// A count or size that cannot be right (negative, or beyond the limits
    /// this crate accepts).
    Count(i64),
    /// Address arithmetic left the 64-bit space.
    Overflow,
    /// An unwind chain that does not end.
    Loop,
}

/// The most entries any table may have here (states, try blocks, handlers
/// of one try block, IP-to-state entries, catchable types).
pub const MAX_ENTRIES: u32 = 1 << 20;

/// `base + rva + off`, checked.
pub(crate) fn at(base: u64, rva: u32, off: u64) -> Result<u64, Bad> {
    base.checked_add(u64::from(rva)).and_then(|a| a.checked_add(off)).ok_or(Bad::Overflow)
}

/// `addr + off` for a signed offset, checked.
pub(crate) fn offset(addr: u64, off: i64) -> Result<u64, Bad> {
    addr.checked_add_signed(off).ok_or(Bad::Overflow)
}

pub(crate) fn read_bytes<const N: usize>(mem: &dyn Memory, addr: u64) -> Result<[u8; N], Bad> {
    let mut b = [0u8; N];
    mem.read(addr, &mut b).map_err(|e| Bad::Unreadable(e.0))?;
    Ok(b)
}

pub(crate) fn read_u8(mem: &dyn Memory, addr: u64) -> Result<u8, Bad> {
    Ok(read_bytes::<1>(mem, addr)?[0])
}

pub(crate) fn read_u32(mem: &dyn Memory, addr: u64) -> Result<u32, Bad> {
    Ok(u32::from_le_bytes(read_bytes(mem, addr)?))
}

pub(crate) fn read_i32(mem: &dyn Memory, addr: u64) -> Result<i32, Bad> {
    Ok(i32::from_le_bytes(read_bytes(mem, addr)?))
}

pub(crate) fn read_u64(mem: &dyn Memory, addr: u64) -> Result<u64, Bad> {
    Ok(u64::from_le_bytes(read_bytes(mem, addr)?))
}

/// A table count, refused above [`MAX_ENTRIES`].
pub(crate) fn count(n: u32) -> Result<u32, Bad> {
    if n > MAX_ENTRIES { Err(Bad::Count(i64::from(n))) } else { Ok(n) }
}
