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

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn LocalFree(mem: *mut u8) -> *mut u8;
    fn LocalSize(mem: *mut u8) -> usize;
    fn SetLastError(error: u32);
}

/// `poly-kernelbase` links `alloc`, so the DLL needs an allocator, though
/// `kernel32` itself never allocates from Rust: the process heap, through
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

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn kernel32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
