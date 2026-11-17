//! Our `winmm.dll` (ADR 0155): the timer functions Chrome calls at start-up. `timeGetTime` is
//! the milliseconds since the system started (the same clock as `GetTickCount`, but 1 ms
//! resolution) and wraps like a `DWORD`; `timeBeginPeriod` and `timeEndPeriod` accept any period
//! from 1 ms (Learn: both return `TIMERR_NOERROR` on success). The logic is
//! `poly_kernelbase::winmm`. Every other winmm function is bound to polystub by the loader.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_kernelbase::winmm;
use poly_win32_rt as _;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetTickCount64() -> u64;
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, out: *mut isize) -> u32;
    fn RegCloseKey(key: isize) -> u32;
}

/// `HKEY_LOCAL_MACHINE` and `KEY_READ` (`winreg.h`, `winnt.h`).
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
const KEY_READ: u32 = 0x2_0019;

/// `Software\Microsoft\Windows NT\CurrentVersion\Drivers32`, the key of the installed
/// multimedia drivers, NUL-terminated.
const DRIVERS32: [u16; 55] = {
    let text = b"Software\\Microsoft\\Windows NT\\CurrentVersion\\Drivers32\0";
    let mut out = [0u16; 55];
    let mut i = 0;
    while i < text.len() {
        out[i] = text[i] as u16;
        i += 1;
    }
    out
};

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

/// The entry point (`DllMain`): when the process loads the library it looks at the key of the
/// installed multimedia drivers, as `winmm` does (it depends on `advapi32` for it, and the loader
/// tells whoever registered for DLL notifications about that dependency too, ADR 0196). Poly has
/// no multimedia drivers: the key is only opened.
#[unsafe(no_mangle)]
pub extern "system" fn winmm_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        let mut key = 0isize;
        // SAFETY: a NUL-terminated path and a local out variable.
        unsafe {
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, DRIVERS32.as_ptr(), 0, KEY_READ, &mut key) == 0 {
                RegCloseKey(key);
            }
        }
    }
    1
}

#[cfg(target_arch = "x86_64")]
mod apps;
