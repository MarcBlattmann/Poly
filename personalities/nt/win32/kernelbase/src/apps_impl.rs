//! The small functions of the Windows applications' import lists that Learn documents fully
//! (ADR 0155 addendum, step 10; logic and tests in `poly_kernelbase::apps`): `MulDiv`,
//! `EncodePointer` / `DecodePointer`, `InterlockedIncrement` and
//! `InterlockedDecrement`. `kernel32.dll` forwards its exports of these names here.

use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use poly_kernelbase::apps;

/// `MulDiv`.
#[unsafe(no_mangle)]
pub extern "system" fn MulDiv(number: i32, numerator: i32, denominator: i32) -> i32 {
    apps::mul_div(number, numerator, denominator)
}

#[cfg(target_arch = "x86")]
use core::arch::x86::_rdtsc;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::_rdtsc;

/// The process's pointer-encoding cookie: made on first use from the time-stamp counter and the
/// stack's address, never 0 (and, for a 32-bit process, never 0 in its low 32 bits).
fn cookie() -> u64 {
    static COOKIE: AtomicU64 = AtomicU64::new(0);
    let c = COOKIE.load(Ordering::Relaxed);
    if c != 0 {
        return c;
    }
    // SAFETY: rdtsc has no preconditions.
    let tsc = unsafe { _rdtsc() };
    let marker = 0u8;
    let fresh = (tsc.rotate_left(17) ^ (core::ptr::addr_of!(marker) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
    match COOKIE.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => fresh,
        Err(existing) => existing,
    }
}

/// `EncodePointer`.
#[unsafe(no_mangle)]
pub extern "system" fn EncodePointer(ptr: usize) -> usize {
    if cfg!(target_arch = "x86") {
        apps::encode_pointer32(ptr as u32, cookie() as u32) as usize
    } else {
        apps::encode_pointer(ptr as u64, cookie()) as usize
    }
}

/// `DecodePointer`.
#[unsafe(no_mangle)]
pub extern "system" fn DecodePointer(ptr: usize) -> usize {
    if cfg!(target_arch = "x86") {
        apps::decode_pointer32(ptr as u32, cookie() as u32) as usize
    } else {
        apps::decode_pointer(ptr as u64, cookie()) as usize
    }
}

/// `InterlockedIncrement`.
///
/// # Safety
/// `value` points to a 4-byte aligned, writable `LONG`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InterlockedIncrement(value: *mut i32) -> i32 {
    // SAFETY: the caller's contract; an `AtomicI32` has the layout of an `i32`.
    apps::interlocked_add(unsafe { &*(value as *const AtomicI32) }, 1)
}

/// `InterlockedDecrement`.
///
/// # Safety
/// As [`InterlockedIncrement`].
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InterlockedDecrement(value: *mut i32) -> i32 {
    // SAFETY: as above.
    apps::interlocked_add(unsafe { &*(value as *const AtomicI32) }, -1)
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LdrDisableThreadCalloutsForDll(handle: *mut u8) -> i32;
}

/// `DisableThreadLibraryCalls(hLibModule)`: "Disables the DLL_THREAD_ATTACH and DLL_THREAD_DETACH
/// notifications for the specified DLL" (Learn, `libloaderapi.h`); nonzero on success, zero with the last
/// error on failure (a handle that is no loaded module). The loader keeps the flag
/// (`LdrDisableThreadCalloutsForDll`). Doc-derived, ADR 0155 addendum, step 10.
#[unsafe(no_mangle)]
pub extern "system" fn DisableThreadLibraryCalls(module: *mut u8) -> i32 {
    // SAFETY: the loader looks the handle up; any value is fine.
    let status = unsafe { LdrDisableThreadCalloutsForDll(module) };
    if status >= 0 { 1 } else { crate::base::fail_status(poly_ntobj::NtStatus(status as u32), 0) }
}
