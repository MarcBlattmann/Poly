//! What Poly's GUI DLLs share (ADR 0128).
//!
//! Until the spec sheets of a function's slice arrive, `user32.dll` and
//! `gdi32.dll` export it as a **stub**: it sets the last error to
//! `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`) and returns the failure
//! value its Learn page documents, never guessed behaviour.
//! [`not_implemented_stubs!`] writes them; `grep -rn not_implemented` finds
//! them all. The export lists `cargo xtask win32` checks are
//! `poly_win32k::exports`.

#![no_std]

/// `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`).
pub const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(error: u32);
}

/// What every stub does before returning its failure value.
pub fn not_implemented() {
    // SAFETY: an ntdll export without preconditions (it writes the calling
    // thread's TEB).
    unsafe { RtlSetLastWin32Error(ERROR_CALL_NOT_IMPLEMENTED) }
}

/// Exported stubs: `Name(arguments...) => failure value;` each. Every
/// argument is taken as a pointer-sized integer, which the x64 calling
/// convention makes the same as the documented integer, handle and pointer
/// parameters (none of these functions takes a structure or a float by
/// value); the value is returned in `rax`, of which callers read the
/// documented type's width (`-1` as `BOOL`, `CLR_INVALID` as `COLORREF`).
#[macro_export]
macro_rules! not_implemented_stubs {
    ($( $(#[$meta:meta])* $name:ident ( $($arg:ident),* ) => $fail:expr ; )*) => {
        $(
            $(#[$meta])*
            #[unsafe(no_mangle)]
            #[allow(unused_variables, non_snake_case)]
            pub extern "system" fn $name($($arg: usize),*) -> usize {
                $crate::not_implemented();
                $fail
            }
        )*
    };
}
