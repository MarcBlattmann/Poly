//! Our `bcryptprimitives.dll` (ADR 0155). Its one export that programs
//! import is `ProcessPrng(pbData, cbData)`: it fills the buffer with
//! cryptographically random bytes and returns `TRUE`. Microsoft's own
//! library on Windows 10 and later uses it for the system-preferred
//! generator; Chromium's base library calls it for every random number and
//! stops the process if the DLL is missing. The bytes come from the same
//! source as `BCryptGenRandom` and `RtlGenRandom` here (the kernel's
//! `/dev/urandom`). The return value is `BOOL` and is `TRUE` unless the
//! random device fails; the function takes no flags.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_cryptrt as rt;

/// `ProcessPrng`: fill `buf[..len]` with random bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ProcessPrng(buf: *mut u8, len: usize) -> i32 {
    if len == 0 {
        return 1;
    }
    if buf.is_null() {
        return 0;
    }
    // SAFETY: the caller's buffer of `len` bytes.
    i32::from(rt::random(unsafe { rt::bytes_mut(buf, len) }))
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn bcryptprimitives_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
