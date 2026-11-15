//! Our `kernel32.dll` (ADR 0079). Most of its exports are forwarders to
//! `kernelbase` through API-set contracts (written by `build.rs` from
//! `poly_kernelbase::exports::KERNEL32_FORWARDS`), as on Windows since
//! Windows 7. What it implements itself are the functions whose Learn
//! pages name no API set: the global-memory functions, over `kernelbase`'s
//! local-memory ones (Learn: "The global functions ... are slower than
//! other memory management functions"; in a 32/64-bit process global and
//! local memory are the same heap).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_rt as _;

extern crate alloc;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn LocalFree(mem: *mut u8) -> *mut u8;
    fn LocalSize(mem: *mut u8) -> usize;
    fn SetLastError(error: u32);
}

/// `poly-kernelbase` links `alloc`, so the DLL needs an allocator (and
/// `GetStartupInfoA` keeps its ANSI strings in it): the process heap, through
/// `kernelbase`'s local-memory functions (16-byte aligned blocks).
struct LocalHeap;

// SAFETY: LocalAlloc returns 16-byte aligned blocks of the process heap;
// stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for LocalHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: no preconditions.
        unsafe { LocalAlloc(0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { LocalFree(ptr) };
    }
}

#[global_allocator]
static ALLOC: LocalHeap = LocalHeap;

/// `GlobalAlloc`: `GMEM_*` flags checked (`GMEM_VALID_FLAGS`), then a
/// local block (`GMEM_ZEROINIT` is `LMEM_ZEROINIT`; moveable blocks are
/// fixed in Poly, so the handle is the address).
#[unsafe(no_mangle)]
pub extern "system" fn GlobalAlloc(flags: u32, bytes: usize) -> *mut u8 {
    match poly_kernelbase::memory::global_alloc_flags(flags) {
        Ok(heap_flags) => {
            let local = if heap_flags != 0 { poly_kernelbase::memory::LMEM_ZEROINIT } else { 0 };
            // SAFETY: a kernelbase export without memory preconditions.
            unsafe { LocalAlloc(local, bytes) }
        }
        Err(e) => {
            // SAFETY: as above.
            unsafe { SetLastError(e) };
            core::ptr::null_mut()
        }
    }
}

/// `GlobalFree`: NULL on success, the handle on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GlobalFree(mem: *mut u8) -> *mut u8 {
    // SAFETY: the caller's block.
    unsafe { LocalFree(mem) }
}

/// `GlobalSize`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GlobalSize(mem: *mut u8) -> usize {
    // SAFETY: the caller's block.
    unsafe { LocalSize(mem) }
}

/// `GetUserDefaultLCID`: the user's locale, English (United States)
/// (`poly_nls`'s default; `GetClipboardData.yaml` compares `CF_LOCALE`
/// with it).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultLCID() -> u32 {
    0x0409
}

/// `GlobalLock`: the block's address (blocks never move in Poly).
#[unsafe(no_mangle)]
pub extern "system" fn GlobalLock(mem: *mut u8) -> *mut u8 {
    mem
}

/// `GlobalUnlock`: FALSE with `NO_ERROR`, the block being unlocked.
#[unsafe(no_mangle)]
pub extern "system" fn GlobalUnlock(_mem: *mut u8) -> i32 {
    // SAFETY: as above.
    unsafe { SetLastError(0) };
    0
}

// ----- the `lstr*` functions (ADR 0079, second slice) ---------------------------------------

/// A NUL-terminated argument; null is treated as empty (Learn: `lstrlenW`
/// "If lpString is NULL, the function returns 0").
///
/// # Safety
/// `p` is null or a NUL-terminated UTF-16 string.
unsafe fn wide<'a>(p: *const u16) -> &'a [u16] {
    // SAFETY: the caller's contract.
    if p.is_null() { &[] } else { unsafe { poly_kernelbase::text::wide_at(p) } }
}

/// `lstrlenW`: the length in characters, without the NUL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn lstrlenW(s: *const u16) -> i32 {
    // SAFETY: the caller's string.
    unsafe { wide(s) }.len() as i32
}

/// `lstrlenA`: the length in bytes, without the NUL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn lstrlenA(s: *const u8) -> i32 {
    // SAFETY: the caller's string.
    if s.is_null() { 0 } else { unsafe { poly_kernelbase::text::strlen(s) as i32 } }
}

/// `lstrcmpW`: negative, zero or positive; Poly compares ordinally
/// (`poly_kernelbase::nls::lstrcmp`, to be confirmed against the user
/// locale's linguistic order).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn lstrcmpW(a: *const u16, b: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { poly_kernelbase::nls::lstrcmp(wide(a), wide(b), false) }
}

/// `lstrcmpiW`: as `lstrcmpW` without case.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn lstrcmpiW(a: *const u16, b: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { poly_kernelbase::nls::lstrcmp(wide(a), wide(b), true) }
}

/// `lstrcmpA`, `lstrcmpiA`: the strings in the ANSI code page, compared as
/// UTF-16 (`lstrcmpW` / `lstrcmpiW`); a null string is less than any other.
unsafe fn ansi_cmp(a: *const u8, b: *const u8, ignore_case: bool) -> i32 {
    // SAFETY: the caller's NUL-terminated strings, or null.
    let text = |p: *const u8| {
        (!p.is_null()).then(|| {
            poly_kernelbase::text::from_ansi(unsafe {
                core::slice::from_raw_parts(p, poly_kernelbase::text::strlen(p))
            })
        })
    };
    let (a, b) = (text(a), text(b));
    match (a, b) {
        (None, None) => 0,
        (None, Some(_)) => -1,
        (Some(_), None) => 1,
        (Some(a), Some(b)) => poly_kernelbase::nls::lstrcmp(&a, &b, ignore_case),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn lstrcmpA(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { ansi_cmp(a, b, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn lstrcmpiA(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { ansi_cmp(a, b, true) }
}

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetStartupInfoW(info: *mut u8);
}

/// `GetStartupInfoA`: `GetStartupInfoW`'s structure (`STARTUPINFOA` has
/// the same layout, `processthreadsapi.h`) with `lpReserved`, `lpDesktop`
/// and `lpTitle` in the ANSI code page, converted once and kept for the
/// life of the process (ADR 0089).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetStartupInfoA(info: *mut u8) {
    use core::sync::atomic::{AtomicPtr, Ordering};
    // This process's own `STARTUPINFO` layout (68 bytes in a 32-bit process, ADR 0186).
    use poly_kernelbase::process::{SI_NATIVE as SL, STARTUPINFO_SIZE};
    static ANSI: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
    let mut p = ANSI.load(Ordering::Acquire);
    if p.is_null() {
        let mut si = alloc::boxed::Box::new([0u8; STARTUPINFO_SIZE]);
        // SAFETY: a buffer at least as large as this process's STARTUPINFOW.
        unsafe { GetStartupInfoW(si.as_mut_ptr()) };
        for field in [SL.reserved, SL.desktop, SL.title] {
            let w = SL.pointer(&si[..], field) as usize as *const u16;
            if w.is_null() {
                continue;
            }
            // SAFETY: a NUL-terminated string in the process parameters.
            let mut a = poly_kernelbase::text::to_ansi(unsafe { poly_kernelbase::text::wide_at(w) });
            a.push(0);
            let at = alloc::boxed::Box::leak(a.into_boxed_slice()).as_ptr() as usize;
            si[field..field + SL.ptr].copy_from_slice(&at.to_le_bytes());
        }
        let leaked = alloc::boxed::Box::leak(si).as_mut_ptr();
        p = match ANSI.compare_exchange(core::ptr::null_mut(), leaked, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => leaked,
            Err(other) => other,
        };
    }
    // SAFETY: the caller's STARTUPINFOA; `p` holds one.
    unsafe { core::ptr::copy_nonoverlapping(p, info, SL.size) };
}

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetSystemInfo(info: *mut u8);
}

/// `GetActiveProcessorGroupCount` (Learn names no API set: `kernel32`'s
/// own, ADR 0114).
#[unsafe(no_mangle)]
pub extern "system" fn GetActiveProcessorGroupCount() -> u16 {
    // SAFETY: no pointers.
    unsafe { SetLastError(poly_kernelbase::system::PROCESSOR_QUERY_LAST_ERROR) };
    poly_kernelbase::system::ACTIVE_PROCESSOR_GROUPS
}

/// `GetActiveProcessorCount`: the processors of a group, from
/// `GetSystemInfo`'s `dwNumberOfProcessors` (ADR 0114).
#[unsafe(no_mangle)]
pub extern "system" fn GetActiveProcessorCount(group: u16) -> u32 {
    let mut info = [0u8; 48];
    // SAFETY: a SYSTEM_INFO-sized buffer (48 bytes, sysinfoapi.h).
    unsafe { GetSystemInfo(info.as_mut_ptr()) };
    let cpus = u32::from_le_bytes([info[32], info[33], info[34], info[35]]);
    match poly_kernelbase::system::active_processor_count(group, cpus) {
        Ok(n) => {
            // SAFETY: no pointers.
            unsafe { SetLastError(poly_kernelbase::system::PROCESSOR_QUERY_LAST_ERROR) };
            n
        }
        Err(e) => {
            // SAFETY: no pointers.
            unsafe { SetLastError(e) };
            0
        }
    }
}

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const u8,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn ReadFile(file: isize, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn WriteFile(file: isize, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

/// `CopyFileW(lpExistingFileName, lpNewFileName, bFailIfExists)` (Learn names no API set:
/// `kernel32`'s own): the bytes of the file are written to the new one, which is made (`CREATE_NEW`
/// when `bFailIfExists`, so an existing file is `ERROR_FILE_EXISTS`) or replaced. Attributes and
/// streams are not copied.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopyFileW(existing: *const u16, new: *const u16, fail_if_exists: i32) -> i32 {
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 1;
    const CREATE_NEW: u32 = 1;
    const CREATE_ALWAYS: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    // SAFETY: the caller's NUL-terminated names; handles are closed on every path.
    unsafe {
        let src = CreateFileW(existing, GENERIC_READ, FILE_SHARE_READ, core::ptr::null(), OPEN_EXISTING, 0, 0);
        if src == -1 {
            return 0;
        }
        let disposition = if fail_if_exists != 0 { CREATE_NEW } else { CREATE_ALWAYS };
        let dst = CreateFileW(new, GENERIC_WRITE, 0, core::ptr::null(), disposition, FILE_ATTRIBUTE_NORMAL, 0);
        if dst == -1 {
            let e = poly_kernelbase_last_error();
            CloseHandle(src);
            SetLastError(e);
            return 0;
        }
        let mut buf = alloc::vec![0u8; 64 * 1024];
        let mut ok = 1;
        loop {
            let mut got = 0u32;
            if ReadFile(src, buf.as_mut_ptr(), buf.len() as u32, &mut got, core::ptr::null_mut()) == 0 {
                ok = 0;
                break;
            }
            if got == 0 {
                break;
            }
            let mut put = 0u32;
            if WriteFile(dst, buf.as_ptr(), got, &mut put, core::ptr::null_mut()) == 0 || put != got {
                ok = 0;
                break;
            }
        }
        let e = poly_kernelbase_last_error();
        CloseHandle(src);
        CloseHandle(dst);
        if ok == 0 {
            SetLastError(e);
        }
        ok
    }
}

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    #[link_name = "GetLastError"]
    fn poly_kernelbase_last_error() -> u32;
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn kernel32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

#[cfg(target_arch = "x86_64")]
mod apps;
