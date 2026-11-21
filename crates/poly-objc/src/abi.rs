//! The layout of the Objective-C metadata the compiler and linker put into
//! an image, for 64-bit code (x86-64 and arm64).
//!
//! Learned from the output of clang and the linkers on our own sources
//! (black boxes: `clang -S` of an `@implementation`, `otool` and `nm` of
//! the linked result; ADR 0261): no runtime source was read. Every record
//! is reached from the image's `__objc_*` list sections:
//!
//! - **Class object** (`_OBJC_CLASS_$_Name`, `_OBJC_METACLASS_$_Name`):
//!   isa, superclass, cache, vtable, data (five pointers). `data` points to
//!   the read-only record; its low three bits are flags, not address.
//! - **Read-only record** (`_OBJC_CLASS_RO_$_Name`): flags (`u32`, bit 0
//!   set for a metaclass), instance start, instance size, a reserved word,
//!   then pointers: ivar layout, name, base methods, base protocols, ivars,
//!   weak ivar layout, base properties.
//! - **Method list**: `u32` entry size and flags, `u32` count, entries.
//!   Plain entries are three pointers (name string, type string, IMP; 24
//!   bytes). With bit 31 of the first word set, the list is relative:
//!   12-byte entries of three `i32` offsets, each from its own field: the
//!   name offset reaches a selector reference (a pointer to the name), the
//!   type offset the type string, the IMP offset the code (0: none). Apple's
//!   linker writes relative lists into `__TEXT,__objc_methlist`.
//! - **Category**: name, class, instance methods, class methods,
//!   protocols, instance properties, class properties (pointers), then its
//!   size (`u32`).
//! - **Protocol list**: a `u64` count, then that many protocol pointers.
//! - **Protocol**: isa, name, protocols, instance methods, class methods,
//!   optional instance methods, optional class methods, properties, size
//!   and flags (`u32` each), ...
//! - **Ivar list**: `u32` entry size, `u32` count, entries of offset
//!   pointer, name, type (pointers), alignment and size (`u32`).

use crate::Mem;
use alloc::vec::Vec;

pub const CLASS_ISA: u64 = 0;
pub const CLASS_SUPERCLASS: u64 = 8;
pub const CLASS_CACHE: u64 = 16;
pub const CLASS_VTABLE: u64 = 24;
pub const CLASS_DATA: u64 = 32;
/// The size of a class object.
pub const CLASS_SIZE: u64 = 40;
/// The bits of `data` that are flags.
pub const DATA_FLAG_BITS: u64 = 7;

pub const RO_FLAGS: u64 = 0;
pub const RO_INSTANCE_START: u64 = 4;
pub const RO_INSTANCE_SIZE: u64 = 8;
pub const RO_NAME: u64 = 24;
pub const RO_BASE_METHODS: u64 = 32;
pub const RO_BASE_PROTOCOLS: u64 = 40;
pub const RO_IVARS: u64 = 48;
pub const RO_BASE_PROPERTIES: u64 = 64;
/// The size of a read-only record.
pub const RO_SIZE: u64 = 72;
/// `flags` bit of a metaclass's read-only record.
pub const RO_META: u32 = 1;
/// `flags` bit of a root class (no superclass).
pub const RO_ROOT: u32 = 2;

pub const CAT_NAME: u64 = 0;
pub const CAT_CLASS: u64 = 8;
pub const CAT_INSTANCE_METHODS: u64 = 16;
pub const CAT_CLASS_METHODS: u64 = 24;
pub const CAT_PROTOCOLS: u64 = 32;

pub const PROTO_NAME: u64 = 8;
pub const PROTO_PROTOCOLS: u64 = 16;

/// Bit 31 of a method list's first word: relative entries.
pub const METHOD_LIST_RELATIVE: u32 = 0x8000_0000;

/// A method as a list in an image describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodEntry {
    /// The address of the name string.
    pub name: u64,
    /// The address of the type string (0: none).
    pub types: u64,
    /// The implementation (0: none, as in protocols).
    pub imp: u64,
}

/// The entries of the method list at `list` (0: none). Relative or plain.
pub fn method_list(m: &dyn Mem, list: u64) -> Vec<MethodEntry> {
    let mut out = Vec::new();
    if list == 0 {
        return out;
    }
    let first = m.u32(list);
    let count = u64::from(m.u32(list + 4)).min(1 << 16);
    let relative = first & METHOD_LIST_RELATIVE != 0;
    let entsize = u64::from(first & 0xfffc);
    for i in 0..count {
        let e = list + 8 + i * entsize;
        if relative {
            let rel = |field: u64| -> u64 {
                let off = m.u32(field) as i32;
                if off == 0 { 0 } else { field.wrapping_add(off as i64 as u64) }
            };
            let selref = rel(e);
            let name = if selref == 0 { 0 } else { m.u64(selref) };
            out.push(MethodEntry { name, types: rel(e + 4), imp: rel(e + 8) });
        } else {
            out.push(MethodEntry { name: m.u64(e), types: m.u64(e + 8), imp: m.u64(e + 16) });
        }
    }
    out
}

/// The protocols of the protocol list at `list` (0: none).
pub fn protocol_list(m: &dyn Mem, list: u64) -> Vec<u64> {
    if list == 0 {
        return Vec::new();
    }
    let n = m.u64(list).min(1 << 12);
    (0..n).map(|i| m.u64(list + 8 + 8 * i)).filter(|&p| p != 0).collect()
}

/// An instance variable as an image describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IvarEntry {
    /// The address of the word holding the ivar's offset.
    pub offset_ptr: u64,
    pub name: u64,
    pub types: u64,
    pub size: u32,
}

/// The entries of the ivar list at `list` (0: none).
pub fn ivar_list(m: &dyn Mem, list: u64) -> Vec<IvarEntry> {
    let mut out = Vec::new();
    if list == 0 {
        return out;
    }
    let entsize = u64::from(m.u32(list)).max(32);
    let count = u64::from(m.u32(list + 4)).min(1 << 16);
    for i in 0..count {
        let e = list + 8 + i * entsize;
        out.push(IvarEntry { offset_ptr: m.u64(e), name: m.u64(e + 8), types: m.u64(e + 16), size: m.u32(e + 28) });
    }
    out
}

/// The read-only record of the class object at `cls`.
pub fn class_ro(m: &dyn Mem, cls: u64) -> u64 {
    m.u64(cls + CLASS_DATA) & !DATA_FLAG_BITS
}
