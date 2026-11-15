//! C++ exception handling (ADR 0099): `msvcrt`'s names for the handlers of
//! `poly_win32_cxxrt`, which `vcruntime140.dll` shares (ADR 0199).
//! `_CxxThrowException` is that crate's assembly, exported by `build.rs`.

use poly_win32_cxxrt::frame_handler;

/// `__CxxFrameHandler3(record, establisher_frame, context, dispatcher)`:
/// the language-specific handler of functions with C++ try blocks or
/// objects to destroy (and of their funclets).
///
/// # Safety
/// Called by the exception dispatcher with valid arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxFrameHandler3(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { frame_handler(record, frame, context, dc) }
}

/// `__CxxFrameHandler`: the older name of the same handler; what depends on
/// the FuncInfo's magic number is read from it.
///
/// # Safety
/// As `__CxxFrameHandler3`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxFrameHandler(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { frame_handler(record, frame, context, dc) }
}

/// What the shared handlers call for `terminate()`: msvcrt's own.
fn terminate() -> ! {
    poly_win32_crt::startup::terminate()
}

/// Name msvcrt's `terminate()` to the shared handlers (process attach).
pub fn attach() {
    poly_win32_cxxrt::set_terminate_hook(terminate);
}
