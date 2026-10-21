//! Our `kernelbase.dll` (ADR 0079): the Win32 base API — files, processes
//! and threads, memory, synchronisation, errors — on top of our `ntdll`
//! (ADR 0071). A real PE DLL built from Rust for `x86_64-pc-windows-msvc`
//! like `ntdll`; `kernel32.dll` forwards most of its exports here through
//! API-set contracts (`poly_kernelbase::exports`).
//!
//! Every export follows its Learn reference page; what a page leaves open
//! is decided, and host-tested, in `crates/poly-kernelbase`. This crate is
//! the glue: argument conversion, the `Nt*` / `Rtl*` / `Ldr*` calls, and
//! the last error.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;
use poly_win32_rt as _;

mod base;
mod console;
mod file;
mod memory;
mod misc;
mod nt;
mod process;
mod sync;

#[global_allocator]
static ALLOC: base::ProcessHeap = base::ProcessHeap;

/// `DLL_PROCESS_ATTACH` (`winnt.h`).
const DLL_PROCESS_ATTACH: u32 = 1;

/// The entry point (`DllMain`): at `DLL_PROCESS_ATTACH`, the process's
/// current directory starts as the executable's directory (ADR 0079), and
/// its standard handles and console are the ones its parameters name (ADR
/// 0080).
#[unsafe(no_mangle)]
pub extern "system" fn kernelbase_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        process::init_current_dir();
        let connection = process::init_std_handles();
        console::init(connection);
    }
    base::TRUE
}
