//! A Mach-O reader: the file format of macOS programs and libraries.
//!
//! This crate only reads and checks. It maps nothing, applies no fixups and
//! resolves no symbols; that is the job of our own `dyld`, which builds on
//! it (ADR 0153). The same reader serves a translator that needs the code of
//! an arm64 program (ADR 0151) and a loader that places it in memory.
//!
//! # What it reads
//!
//! - **Universal files** ([`Fat`]): `FAT_MAGIC` and `FAT_MAGIC_64`, the
//!   slice table, picking a slice by CPU ([`select_slice`]).
//! - **Thin files** ([`MachO`]): the 64-bit little-endian header, all load
//!   commands with typed views ([`Command`]), segments and sections
//!   ([`Segment`], [`Section`]), symbols and strings ([`Symtab`]), the
//!   dynamic symbol table ([`Dysymtab`]), dependencies, run paths, entry
//!   points ([`Entry`]), versions, UUID, code signature and other
//!   `__LINKEDIT` blobs. 32-bit and big-endian files are recognised and
//!   refused with their own errors.
//! - **Export tries** ([`Exports`]): walking and looking up, with the
//!   re-export and stub-and-resolver forms.
//! - **Classic fixups** ([`DyldInfo`]): rebase, bind, weak bind and lazy
//!   bind opcode streams as iterators of records.
//! - **Chained fixups** ([`ChainedFixups`]): the header, segment starts,
//!   imports, and a walk of every chain for the 64-bit and arm64e pointer
//!   formats, with pointer authentication fields.
//!
//! # A loader's first look
//!
//! ```text
//! let bytes = select_slice(file, CpuKind::Arm64)?;   // thin or universal
//! let macho = MachO::parse(bytes)?;                  // everything checked once
//! let s = macho.summary()?;                          // entry, PIE, platform, ...
//! for lib in macho.dylibs() { /* load lib.name; its ordinal is its index + 1 */ }
//! for seg in macho.segment_table() { /* map seg.vm_range() from seg.file_range() */ }
//! if let Some(cf) = macho.chained_fixups()? {
//!     for fixup in cf.fixups() { /* rebase or bind, in place */ }
//! } else if let Some(info) = macho.dyld_info()? {
//!     for r in info.rebases() { /* ... */ }
//! }
//! ```
//!
//! # Design
//!
//! Everything borrows the file's bytes: no allocation, no copies except a
//! table of [`MAX_SEGMENTS`] small segment records inside [`MachO`]. The
//! crate is `no_std` and has no dependencies, so the kernel can use it as it
//! is. Lists are iterators; tables are checked when they are opened.
//!
//! # Robustness
//!
//! The input is untrusted. Whatever the bytes are, nothing here panics,
//! reads out of bounds, overflows or runs for long: every offset and size is
//! checked against the data it indexes (with checked arithmetic), every
//! loop either consumes input or has an explicit bound ([`MAX_FAT_ARCHES`],
//! [`MAX_SEGMENTS`], [`MAX_TRIE_DEPTH`], [`MAX_EXPORT_NAME_LEN`],
//! [`MAX_NAME_LEN`], 10 bytes per LEB128 number, a budget of records per
//! fixup stream), and failures are values of the typed [`Error`]. The
//! `tests/fuzz.rs` test mutates every test file hundreds of thousands of
//! times and runs every parser over the result.
//!
//! # Sources
//!
//! Layouts and constants come from the public SDK headers
//! `<mach-o/loader.h>`, `<mach-o/fat.h>`, `<mach-o/nlist.h>`,
//! `<mach-o/fixup-chains.h>`, `<mach/machine.h>`, `<mach/vm_prot.h>` and the
//! thread state headers `<mach/i386/thread_status.h>` and
//! `<mach/arm/thread_status.h>`; each constant group in [`consts`] names its
//! header. Behaviour that the headers' comments do not spell out (how the
//! `ADD_ADDR` opcodes advance, how a lazy stream is divided) was checked
//! against the output of `llvm-objdump` on files we built ourselves; no
//! Apple binary and no other implementation was read. See `CLEAN_ROOM.md`.

#![no_std]

mod bytes;
mod chained;
mod command;
pub mod consts;
mod dyld_info;
mod error;
mod exports;
mod fat;
mod header;
mod image;
mod segment;
mod symtab;

pub use chained::*;
pub use command::*;
pub use consts::*;
pub use dyld_info::*;
pub use error::{Error, Result};
pub use exports::*;
pub use fat::*;
pub use header::*;
pub use image::*;
pub use segment::*;
pub use symtab::*;
