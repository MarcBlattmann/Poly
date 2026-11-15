//! Our `vcruntime140_1.dll` (ADR 0199): `__CxxFrameHandler4`, the language-specific handler
//! of functions whose C++ exception tables are in Visual C++'s compressed form ("FH4", the
//! default of Visual C++ 2019 and later on x64), and the debugger hooks.
//!
//! The compressed tables are not publicly documented and clang cannot emit them, so their
//! behaviour waits for a spec sheet observed with programs Microsoft's compiler builds on the
//! Windows oracle (ADR 0199). Until then the handler takes no exception and runs no
//! destructor: it logs `stub: vcruntime140_1!__CxxFrameHandler4` and lets the search go on
//! (`ExceptionContinueSearch`), so programs that throw nothing run, and a throw through such a
//! frame is seen in the log. That is what Poly's `msvcrt` did for every C++ frame before
//! ADR 0099.

#![no_std]

/// `ExceptionContinueSearch` (`excpt.h`).
const CONTINUE_SEARCH: u32 = 1;

/// The entry point: nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn vcruntime140_1_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

/// `__CxxFrameHandler4(record, establisher_frame, context, dispatcher)`: not yet (above).
#[unsafe(no_mangle)]
pub extern "C" fn __CxxFrameHandler4(_record: *mut u8, _frame: u64, _context: *mut u8, _dc: *mut u8) -> u32 {
    poly_win32_rt::unimplemented_export("stub: vcruntime140_1!__CxxFrameHandler4", false, 0);
    CONTINUE_SEARCH
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
