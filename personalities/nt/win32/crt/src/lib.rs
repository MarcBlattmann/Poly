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

/// An exported variadic function: spill the register arguments into their
/// home slots (so all arguments form one array), point at the first
/// variadic one, and call `$t` with that pointer as the argument after the
/// `$fixed` fixed ones (Learn, "x64 calling convention": "the caller
/// reserves ... space for the four register parameters"; varargs are
/// passed as for an unprototyped function). The thunk has unwind data.
/// Each DLL's `build.rs` exports the names (`poly_crt::exports::SHARED_VARARGS`
/// for the ones here).
#[macro_export]
macro_rules! va_thunk {
    ($name:literal, 1, $t:path) => {
        $crate::va_thunk!(@ $name, "72", "mov rdx, rax", $t);
    };
    ($name:literal, 2, $t:path) => {
        $crate::va_thunk!(@ $name, "80", "mov r8, rax", $t);
    };
    ($name:literal, 3, $t:path) => {
        $crate::va_thunk!(@ $name, "88", "mov r9, rax", $t);
    };
    (@ $name:literal, $off:literal, $store:literal, $t:path) => {
        core::arch::global_asm!(
            ".text",
            concat!(".globl ", $name),
            ".p2align 4",
            concat!(".seh_proc ", $name),
            concat!($name, ":"),
            "mov qword ptr [rsp + 8], rcx",
            "mov qword ptr [rsp + 16], rdx",
            "mov qword ptr [rsp + 24], r8",
            "mov qword ptr [rsp + 32], r9",
            "sub rsp, 56",
            ".seh_stackalloc 56",
            ".seh_endprologue",
            concat!("lea rax, [rsp + ", $off, "]"),
            $store,
            "call {t}",
            "add rsp, 56",
            "ret",
            ".seh_endproc",
            t = sym $t,
        );
    };
}

pub mod conio;
pub mod conv;
pub mod env;
pub mod find;
pub mod global;
pub mod heap;
pub mod io;
pub mod jmp;
pub mod lctype;
pub mod locale;
pub mod lowio;
pub mod mathfn;
pub mod mbconv;
pub mod mbstring;
pub mod misc;
pub mod spawn;
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
/// `terminating`: `DllMain`'s `lpReserved` is non-NULL at
/// `DLL_PROCESS_DETACH`, the process is ending (Learn, "DllMain entry
/// point"). Streams are written when the DLL is unloaded, not when the
/// process ends without `exit` (other threads may hold their locks; Poly's
/// reading of what the runtime does, to be confirmed).
pub fn dll_main(reason: u32, terminating: bool) -> i32 {
    match reason {
        DLL_PROCESS_ATTACH => {
            global::init_tls();
            startup::init();
            lowio::init();
            stdio::init();
        }
        DLL_THREAD_DETACH => {
            locale::thread_detach();
            global::thread_detach();
        }
        DLL_PROCESS_DETACH if !terminating => {
            stdio::flush_all();
        }
        _ => {}
    }
    1
}
