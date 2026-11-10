//! Our `winmm.dll` (ADR 0155): the timer functions Chrome calls at start-up. `timeGetTime` is
//! the milliseconds since the system started (the same clock as `GetTickCount`, but 1 ms
//! resolution) and wraps like a `DWORD`; `timeBeginPeriod` and `timeEndPeriod` accept any period
//! from 1 ms (Learn: both return `TIMERR_NOERROR` on success). The logic is
//! `poly_kernelbase::winmm`. Every other winmm function is bound to polystub by the loader.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_kernelbase::winmm;
use poly_win32_rt as _;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTickCount64() -> u64;
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
}

/// The process heap (16-byte aligned) for `alloc` (poly-kernelbase links it).
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

/// `timeGetTime`.
#[unsafe(no_mangle)]
pub extern "system" fn timeGetTime() -> u32 {
    // SAFETY: a plain call.
    winmm::time_get_time(unsafe { GetTickCount64() })
}

/// `timeBeginPeriod`.
#[unsafe(no_mangle)]
pub extern "system" fn timeBeginPeriod(period: u32) -> u32 {
    winmm::period_result(period)
}

/// `timeEndPeriod`.
#[unsafe(no_mangle)]
pub extern "system" fn timeEndPeriod(period: u32) -> u32 {
    winmm::period_result(period)
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn winmm_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
