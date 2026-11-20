//! Poly's Objective-C runtime, the logic (ADR 0261): what `libobjc.A.dylib`
//! (`personalities/xnu/libobjc`) does with the metadata of the program's
//! images and with its objects, host-tested.
//!
//! - [`abi`]: the layout of the compiled metadata (class objects, read-only
//!   records, method lists plain and relative, categories, protocols, ivar
//!   lists).
//! - [`image`]: an image's `__objc_*` sections, from its Mach-O header in
//!   memory.
//! - [`runtime`]: selectors, classes, methods, categories, protocols, method
//!   lookup, run-time classes and the `+load` order.
//! - [`arc`]: reference counts, weak references, associated objects,
//!   autorelease pools and `@synchronized` locks.
//!
//! The memory the metadata lives in is reached through [`Mem`]: raw
//! pointers in the dylib, a simulated address space in the tests.
//!
//! Sources: the spec sheets of `specs/macos/libobjc.A.dylib` and their
//! visible tests, `<objc/runtime.h>` and `<objc/message.h>` of the macOS
//! SDK, and the output of clang and the linkers for our own Objective-C
//! sources (black boxes; ADR 0261). No source of Apple's runtime (objc4) or
//! of any other Objective-C runtime was read, and no Apple binary was
//! disassembled.

#![no_std]

extern crate alloc;

pub mod abi;
pub mod arc;
pub mod image;
pub mod runtime;

use alloc::vec::Vec;

/// The memory of the process, as the runtime reads and writes it.
pub trait Mem {
    /// The byte at `addr` (0 where nothing is mapped).
    fn u8(&self, addr: u64) -> u8;

    fn u32(&self, addr: u64) -> u32 {
        u32::from_le_bytes(core::array::from_fn(|i| self.u8(addr.wrapping_add(i as u64))))
    }

    fn u64(&self, addr: u64) -> u64 {
        u64::from_le_bytes(core::array::from_fn(|i| self.u8(addr.wrapping_add(i as u64))))
    }

    /// Write a pointer-sized word (selector references, class objects the
    /// runtime makes).
    fn write_u64(&mut self, addr: u64, value: u64);

    /// Copy `bytes` to new memory that lives as long as the program and
    /// return its address (names and type strings the runtime keeps).
    fn alloc(&mut self, bytes: &[u8]) -> u64;

    /// The NUL-terminated string at `addr` (at most 64 KiB of it).
    fn cstr(&self, addr: u64) -> Vec<u8> {
        let mut out = Vec::new();
        if addr == 0 {
            return out;
        }
        loop {
            let b = self.u8(addr.wrapping_add(out.len() as u64));
            if b == 0 || out.len() >= 1 << 16 {
                return out;
            }
            out.push(b);
        }
    }
}

#[cfg(test)]
mod tests;
