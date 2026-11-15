//! ANSI forms of kernel32 functions that Poly implements in UTF-16 (ADR 0186, stage 3: the 32-bit
//! programs of the survey are mostly ANSI-minded: Borland's runtime, MSYS2, Python). Each makes
//! the wide call and converts the result to the ANSI code page (1252, `poly_kernelbase::text`);
//! the buffer conventions are Learn's for the function.

use crate::base::*;
use crate::nt;
use crate::process::{FreeEnvironmentStringsW, GetCurrentDirectoryW, GetEnvironmentStringsW, GetModuleFileNameW};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::text;

/// Room for a path in the wide call (`MAX_PATH` times a few; Poly's paths are shorter).
const PATH_UNITS: usize = 1024;

/// Copy `bytes` and a NUL to `buf` (`n` bytes of `bytes`, `n < capacity`).
///
/// # Safety
/// `buf` holds `n + 1` bytes.
unsafe fn put_ansi(buf: *mut u8, bytes: &[u8], n: usize) {
    // SAFETY: the caller's buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n);
        buf.add(n).write(0);
    }
}

/// `GetModuleFileNameA`: the module's path in the ANSI code page, with the "truncate" convention
/// (Learn: a too-small buffer gets `nSize - 1` characters and a NUL, the function returns `nSize`
/// and sets `ERROR_INSUFFICIENT_BUFFER`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetModuleFileNameA(module: *mut u8, buf: *mut u8, size: u32) -> u32 {
    let mut wide = [0u16; PATH_UNITS];
    // SAFETY: the buffer holds `PATH_UNITS` units.
    let n = unsafe { GetModuleFileNameW(module, wide.as_mut_ptr(), PATH_UNITS as u32) } as usize;
    if n == 0 {
        return 0;
    }
    let ansi = text::to_ansi(&wide[..n.min(PATH_UNITS)]);
    let (copy, ret, short) = text::truncated(ansi.len(), size);
    if size > 0 {
        // SAFETY: the caller's buffer of `size` bytes; `copy < size`.
        unsafe { put_ansi(buf, &ansi, copy) };
    }
    if short {
        set_last_error(ERROR_INSUFFICIENT_BUFFER);
    }
    ret
}

/// `GetCurrentDirectoryA`: the "size" convention (the length without the NUL, or the size
/// needed with it).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCurrentDirectoryA(size: u32, buf: *mut u8) -> u32 {
    let mut wide = [0u16; PATH_UNITS];
    // SAFETY: the buffer holds `PATH_UNITS` units.
    let n = unsafe { GetCurrentDirectoryW(PATH_UNITS as u32, wide.as_mut_ptr()) } as usize;
    if n == 0 || n >= PATH_UNITS {
        return 0;
    }
    let ansi = text::to_ansi(&wide[..n]);
    match text::sized(ansi.len(), size) {
        text::Sized::Copy(len) => {
            // SAFETY: the caller's buffer holds more than `len` bytes.
            unsafe { put_ansi(buf, &ansi, len as usize) };
            len
        }
        text::Sized::Needed(need) => need,
    }
}

/// `GetEnvironmentStrings` (`GetEnvironmentStringsA`): the environment block in the ANSI code
/// page (`NAME=value` strings, each NUL-terminated, then one more NUL), a copy freed with
/// `FreeEnvironmentStringsA`.
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentStrings() -> *mut u8 {
    let wide = GetEnvironmentStringsW();
    if wide.is_null() {
        return core::ptr::null_mut();
    }
    // The block ends at the empty string: two NULs in a row.
    let mut len = 0usize;
    // SAFETY: the block `GetEnvironmentStringsW` returned ends with an empty string.
    unsafe {
        while !(wide.add(len).read() == 0 && wide.add(len + 1).read() == 0) {
            len += 1;
        }
    }
    len += 2;
    // SAFETY: `len` units of the block.
    let units: Vec<u16> = unsafe { core::slice::from_raw_parts(wide, len) }.to_vec();
    // SAFETY: the block we got above.
    unsafe { FreeEnvironmentStringsW(wide) };
    let ansi = text::to_ansi(&units);
    // SAFETY: a valid heap.
    let p = unsafe { nt::RtlAllocateHeap(process_heap(), 0, ansi.len()) } as *mut u8;
    if p.is_null() {
        return fail(ERROR_NOT_ENOUGH_MEMORY, p);
    }
    // SAFETY: `p` holds `ansi.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(ansi.as_ptr(), p, ansi.len()) };
    p
}

/// `GetEnvironmentStringsA` is the same function by its other name.
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentStringsA() -> *mut u8 {
    GetEnvironmentStrings()
}

/// `FreeEnvironmentStringsA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeEnvironmentStringsA(block: *mut u8) -> i32 {
    // SAFETY: a block `GetEnvironmentStrings` returned.
    unsafe { nt::RtlFreeHeap(process_heap(), 0, block) as i32 }
}
