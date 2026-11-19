//! Poly's own `dyld`: loading Mach-O programs and their libraries
//! (ADR 0237).
//!
//! [`link`] reads a program through a [`FileSystem`], finds every library it
//! needs (`@executable_path`, `@loader_path`, `@rpath`, re-exports, weak
//! libraries; [`path`]), places every image (the program at its preferred
//! address or a chosen one, libraries one after another), and works out
//! each image's initializers and thread-local variables. The result, a
//! [`Program`], says what to map ([`Segment`]) and, through
//! [`Program::fixups`], every pointer to write: chained fixups (the 64-bit
//! and arm64e pointer formats; arm64e signatures are not applied) and the
//! classic rebase, bind, lazy bind and weak bind opcodes. Lazy binds are
//! bound at load, as `DYLD_BIND_AT_LAUNCH` would. Symbols are looked up in
//! export tries with the two-level namespace, flat lookups and weak
//! coalescing.
//!
//! Who maps memory is the caller's business: the kernel maps the segments
//! from the file, writes the fixups and builds the [`stack`]; the
//! [`start`] code then runs the initializers and `main` in user space.
//! Nothing here depends on the CPU but the start code (x86-64 now; the
//! loader itself reads arm64 files the same way, and the arm64 translator
//! will bring its own start code).
//!
//! Sources: the dyld(1) manual page (search paths), Apple's public Mach-O
//! headers through `poly-macho` (layouts and constants), and Poly's own
//! test programs (`tests/data`, built with clang and lld as black boxes).
//! No source of Apple's dyld or of any other loader was read.

#![no_std]

extern crate alloc;

mod error;
mod link;
pub mod path;
pub mod stack;
pub mod start;

pub use error::{Error, Result};
pub use link::*;
