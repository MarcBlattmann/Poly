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

/// `OpenFileMappingA`: `OpenFileMappingW` with the name converted from the ANSI code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenFileMappingA(access: u32, inherit: i32, name: *const u8) -> crate::nt::Handle {
    if name.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: a NUL-terminated name.
    let mut wide = text::from_ansi(unsafe { core::slice::from_raw_parts(name, text::strlen(name)) });
    wide.push(0);
    // SAFETY: a NUL-terminated wide name.
    unsafe { crate::mapping::OpenFileMappingW(access, inherit, wide.as_ptr()) }
}

// ----- directory enumeration ------------------------------------------------------------------

/// `WIN32_FIND_DATAW` and `WIN32_FIND_DATAA` (`minwinbase.h`): the same first 44 bytes (attributes, three
/// times, the 64-bit size in two halves, the two reserved words), then `cFileName[260]` and
/// `cAlternateFileName[14]` of the character size: 520 + 28 bytes, or 260 + 14 bytes (320 with padding).
const FIND_PREFIX: usize = 44;
const FIND_DATA_W: usize = FIND_PREFIX + 520 + 28;

/// Convert a `WIN32_FIND_DATAW` to a `WIN32_FIND_DATAA`: the names go to the ANSI code page (the name
/// and the 8.3 name are cut to their fields; a name longer than 259 bytes cannot be told, as on Windows).
///
/// # Safety
/// `wide` holds a `WIN32_FIND_DATAW`, `ansi` has room for a `WIN32_FIND_DATAA` (320 bytes).
unsafe fn find_data_to_ansi(wide: &[u8; FIND_DATA_W], ansi: *mut u8) {
    // SAFETY: the caller's buffer of 320 bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(wide.as_ptr(), ansi, FIND_PREFIX);
        let units = |at: usize, max: usize| -> Vec<u16> {
            wide[at..at + 2 * max]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect()
        };
        let name = text::to_ansi(&units(FIND_PREFIX, 260));
        let short = text::to_ansi(&units(FIND_PREFIX + 520, 14));
        core::ptr::write_bytes(ansi.add(FIND_PREFIX), 0, 320 - FIND_PREFIX);
        core::ptr::copy_nonoverlapping(name.as_ptr(), ansi.add(FIND_PREFIX), name.len().min(259));
        core::ptr::copy_nonoverlapping(short.as_ptr(), ansi.add(FIND_PREFIX + 260), short.len().min(13));
    }
}

/// The wide pattern of an ANSI name (`None` for NULL).
///
/// # Safety
/// `name` is NULL or a NUL-terminated string.
unsafe fn wide_pattern(name: *const u8) -> Option<Vec<u16>> {
    if name.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated name.
    let mut w = text::from_ansi(unsafe { core::slice::from_raw_parts(name, text::strlen(name)) });
    w.push(0);
    Some(w)
}

/// `FindFirstFileA(lpFileName, lpFindFileData)`: `FindFirstFileW` with the name and the result in the ANSI code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstFileA(name: *const u8, data: *mut u8) -> isize {
    // SAFETY: the caller's name.
    let Some(pattern) = (unsafe { wide_pattern(name) }) else { return fail(ERROR_INVALID_PARAMETER, -1) };
    let mut wide = [0u8; FIND_DATA_W];
    // SAFETY: a wide pattern and a buffer of a WIN32_FIND_DATAW.
    let h = unsafe { crate::file::FindFirstFileW(pattern.as_ptr(), wide.as_mut_ptr()) };
    if h != -1 && !data.is_null() {
        // SAFETY: the caller's WIN32_FIND_DATAA.
        unsafe { find_data_to_ansi(&wide, data) };
    }
    h
}

/// `FindFirstFileExA`: the same for `FindFirstFileExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstFileExA(
    name: *const u8,
    info_level: u32,
    data: *mut u8,
    search_op: u32,
    filter: *const u8,
    flags: u32,
) -> isize {
    // SAFETY: the caller's name.
    let Some(pattern) = (unsafe { wide_pattern(name) }) else { return fail(ERROR_INVALID_PARAMETER, -1) };
    let mut wide = [0u8; FIND_DATA_W];
    // SAFETY: a wide pattern and a buffer of a WIN32_FIND_DATAW.
    let h = unsafe {
        crate::fileex::FindFirstFileExW(pattern.as_ptr(), info_level, wide.as_mut_ptr(), search_op, filter, flags)
    };
    if h != -1 && !data.is_null() {
        // SAFETY: the caller's WIN32_FIND_DATAA.
        unsafe { find_data_to_ansi(&wide, data) };
    }
    h
}

/// `FindNextFileA(hFindFile, lpFindFileData)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindNextFileA(handle: isize, data: *mut u8) -> i32 {
    let mut wide = [0u8; FIND_DATA_W];
    // SAFETY: a buffer of a WIN32_FIND_DATAW.
    let ok = unsafe { crate::file::FindNextFileW(handle, wide.as_mut_ptr()) };
    if ok != 0 && !data.is_null() {
        // SAFETY: the caller's WIN32_FIND_DATAA.
        unsafe { find_data_to_ansi(&wide, data) };
    }
    ok
}
