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

#[link(name = "kernelbase", kind = "raw-dylib")]
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

#[link(name = "kernelbase", kind = "raw-dylib")]
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
    use poly_kernelbase::process::{SI_DESKTOP, SI_RESERVED, SI_TITLE, STARTUPINFO_SIZE};
    static ANSI: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
    let mut p = ANSI.load(Ordering::Acquire);
    if p.is_null() {
        let mut si = alloc::boxed::Box::new([0u8; STARTUPINFO_SIZE]);
        // SAFETY: a STARTUPINFOW-sized buffer.
        unsafe { GetStartupInfoW(si.as_mut_ptr()) };
        for field in [SI_RESERVED, SI_DESKTOP, SI_TITLE] {
            let w = u64::from_le_bytes(si[field..field + 8].try_into().unwrap()) as *const u16;
            if w.is_null() {
                continue;
            }
            // SAFETY: a NUL-terminated string in the process parameters.
            let mut a = poly_kernelbase::text::to_ansi(unsafe { poly_kernelbase::text::wide_at(w) });
            a.push(0);
            let at = alloc::boxed::Box::leak(a.into_boxed_slice()).as_ptr() as u64;
            si[field..field + 8].copy_from_slice(&at.to_le_bytes());
        }
        let leaked = alloc::boxed::Box::leak(si).as_mut_ptr();
        p = match ANSI.compare_exchange(core::ptr::null_mut(), leaked, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => leaked,
            Err(other) => other,
        };
    }
    // SAFETY: the caller's STARTUPINFOA; `p` holds one.
    unsafe { core::ptr::copy_nonoverlapping(p, info, STARTUPINFO_SIZE) };
}

#[link(name = "kernelbase", kind = "raw-dylib")]
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

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn kernel32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
