//! Our `vcruntime140_1.dll` (ADR 0199): `__CxxFrameHandler4`, the language-specific handler
//! of functions whose C++ exception tables are in Visual C++'s compressed form ("FH4", the
//! default of Visual C++ 2019 and later on x64), and the debugger hooks.
//!
//! The handler (the spec `vcruntime140_1.dll/__CxxFrameHandler4.yaml`) is in Poly's
//! `vcruntime140.dll` (`poly-win32-cxxrt`, tables decoded by `poly_cxxeh::fh4`) and this DLL's
//! export forwards to it (`build.rs`): the catch blocks in progress, the current exception and
//! the count of uncaught exceptions are per-thread state that `_CxxThrowException` and
//! `__CxxFrameHandler3` share with it, so it must live in the one DLL that has them.

#![no_std]

// The panic handler and the runtime support every Poly DLL links.
use poly_win32_rt as _;

/// Marks this file as Poly's own copy (ADR 0199): at boot Poly refreshes a
/// `System32` copy that holds these bytes and keeps any other (Microsoft's
/// runtime a user installed). The same text as the kernel's
/// `OWN_COPY_MARKER` (`personalities/nt/src/layout/create.rs`); the kernel's
/// `build.rs` checks it is in the file.
static OWN_COPY_MARKER: &[u8] = b"Poly's own copy of a Visual C++ runtime DLL (ADR 0199)";

/// The entry point: nothing to set up (it keeps the marker in the file).
#[unsafe(no_mangle)]
pub extern "system" fn vcruntime140_1_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    core::hint::black_box(OWN_COPY_MARKER.as_ptr());
    1
}

// The debugger's non-local-goto hooks: places a debugger sets breakpoints on.
core::arch::global_asm!(
    r#"
    .text
    .globl poly_vcrt1_nlg
    .p2align 4
poly_vcrt1_nlg:
    ret
"#
);
