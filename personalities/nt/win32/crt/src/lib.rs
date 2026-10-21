//! Our C runtime's process side (ADR 0084), shared by `ucrtbase.dll` and
//! `msvcrt.dll`: the heap, file descriptors, streams, the program's
//! arguments and environment, start-up and exit, signals, and every
//! function both DLLs export with the same behaviour (its `#[no_mangle]`
//! items, which each DLL exports as they are). What only one DLL exports
//! is a plain function here that the DLL wraps; what can be decided
//! without a process is in `crates/poly-crt`.
//!
//! The runtime sits on `kernel32.dll` alone ([`sys`]).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;
use poly_win32_rt as _;

pub mod conv;
pub mod env;
pub mod global;
pub mod heap;
pub mod io;
pub mod lowio;
pub mod misc;
pub mod startup;
pub mod stdio;
pub mod strings;
pub mod sys;
pub mod text;
pub mod timefn;

#[global_allocator]
static ALLOC: heap::ProcessHeap = heap::ProcessHeap;

/// `DLL_PROCESS_ATTACH` …(`winnt.h`).
pub const DLL_PROCESS_DETACH: u32 = 0;
pub const DLL_PROCESS_ATTACH: u32 = 1;
pub const DLL_THREAD_ATTACH: u32 = 2;
pub const DLL_THREAD_DETACH: u32 = 3;

/// The DLLs' entry point's work. A DLL that binds its own data
/// ([`startup::bind`], [`stdio::bind_iob`]) does so before calling this.
pub fn dll_main(reason: u32) -> i32 {
    match reason {
        DLL_PROCESS_ATTACH => {
            global::init_tls();
            startup::init();
            lowio::init();
            stdio::init();
        }
        DLL_THREAD_DETACH => global::thread_detach(),
        DLL_PROCESS_DETACH => {
            stdio::flush_all();
        }
        _ => {}
    }
    1
}
