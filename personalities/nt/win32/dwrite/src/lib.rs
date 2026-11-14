//! Our `dwrite.dll` (ADR 0170): DirectWrite for Chrome and Skia. One factory
//! object answers for `IDWriteFactory` .. `IDWriteFactory7`; fonts come from
//! application loaders, from files, from in-memory data and from the
//! system collection (the fonts of `C:\Windows\Fonts`, also under the
//! Windows family names that stand for them). The logic is
//! `poly-dwrite`'s, host-tested; this crate is the COM glue.

#![no_std]
#![allow(clippy::missing_safety_doc, clippy::missing_transmute_annotations, clippy::upper_case_acronyms)]

extern crate alloc;

use core::ffi::c_void;

use poly_win32_gui as _;
use poly_win32_rt as _;

mod abi;
mod coll;
mod com;
mod face;
mod factory;
mod fallback;
mod file;
mod fontset;
mod io;
mod numsub;
mod render;
mod strings;
mod system;

/// `DWriteCreateFactory(factoryType, iid, factory)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DWriteCreateFactory(kind: u32, iid: *const abi::GUID, factory: *mut *mut c_void) -> i32 {
    // SAFETY: the caller's arguments, checked inside.
    unsafe { factory::dwrite_create_factory(kind, iid, factory) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dwrite_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
