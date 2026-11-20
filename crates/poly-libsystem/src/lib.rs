//! The logic of Poly's own `libSystem.B.dylib` (ADR 0258), kept here so it
//! is tested on the host. The dylibs themselves (`personalities/xnu/libsystem`,
//! built for `x86_64-apple-darwin` and linked by `rust-lld`) export the C
//! functions and call into this crate.
//!
//! Inputs: the C standard, the System V x86-64 ABI, the macOS manual pages
//! and SDK headers (for constants and layouts), and the spec sheets of
//! `specs/macos/`. No part of Apple's libraries was read or disassembled.

#![no_std]

#[cfg(test)]
extern crate alloc;

pub mod atexit;
pub mod cstr;
pub mod ctype;
pub mod heap;
pub mod num;
pub mod printf;
pub mod scanf;
pub mod stdio;
pub mod sys;
pub mod time;
