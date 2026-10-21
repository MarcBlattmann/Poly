//! Our `kernelbase.dll` (ADR 0079): the Win32 base API — files, processes
//! and threads, memory, synchronisation, errors — on top of our `ntdll`
//! (ADR 0071). A real PE DLL built from Rust for `x86_64-pc-windows-msvc`
//! like `ntdll`; `kernel32.dll` forwards most of its exports here through
//! API-set contracts (`poly_kernelbase::exports`).
//!
//! It also holds the registry, token, SID, ACL and security-descriptor
//! functions `advapi32.dll` forwards here (ADR 0083: `registry`,
//! `security`).
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
mod mapping;
mod memory;
mod misc;
mod nls;
mod nt;
mod process;
mod registry;
mod security;
mod spawn;
mod sync;
mod tls;

#[global_allocator]
static ALLOC: base::ProcessHeap = base::ProcessHeap;

/// `DLL_PROCESS_ATTACH`, `DLL_THREAD_ATTACH`, `DLL_THREAD_DETACH`
/// (`winnt.h`).
const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_THREAD_ATTACH: u32 = 2;
const DLL_THREAD_DETACH: u32 = 3;

/// The entry point (`DllMain`): at `DLL_PROCESS_ATTACH`, the process's
/// current directory starts as its parameters say, else the executable's
/// directory (ADR 0079), and its standard handles and console are the ones
/// its parameters name (ADR 0080). Every thread is noted for TLS (`tls`).
#[unsafe(no_mangle)]
pub extern "system" fn kernelbase_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    match reason {
        DLL_PROCESS_ATTACH => {
            process::init_current_dir();
            let connection = process::init_std_handles();
            console::init(connection);
            tls::thread_attach();
        }
        DLL_THREAD_ATTACH => tls::thread_attach(),
        DLL_THREAD_DETACH => tls::thread_detach(),
        _ => {}
    }
    base::TRUE
}
