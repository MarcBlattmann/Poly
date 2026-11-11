//! Our `wtsapi32.dll` (ADR 0155, step 9): Remote Desktop Services as a machine with one
//! session at the console has them. Chrome's browser delay-loads four functions when it starts
//! (it registers its message window for session changes and asks about the session); without
//! the DLL the browser ends itself.
//!
//! The behaviour is Learn's (`wtsapi32.h`); the answers are `poly_kernelbase::session`'s,
//! host-tested. Nothing here was observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_kernelbase::session::{self, Session};

poly_win32_rt::process_heap_allocator!();

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetLastError(error: u32);
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn GetComputerNameW(buf: *mut u16, size: *mut u32) -> i32;
}

/// `ERROR_INVALID_HANDLE`, `ERROR_INVALID_PARAMETER`, `ERROR_NOT_ENOUGH_MEMORY` (`winerror.h`).
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;

fn fail(error: u32) -> i32 {
    // SAFETY: a plain call.
    unsafe { SetLastError(error) };
    0
}

/// This process's session (`PEB.SessionId`, PEB + 0x2C0, `winternl.h`; the PEB is at
/// `gs:[0x60]`).
fn session_id() -> u32 {
    let peb: u64;
    // SAFETY: GS is the TEB in every NT thread; TEB + 0x60 is the PEB pointer.
    unsafe {
        core::arch::asm!("mov {}, gs:[0x60]", out(reg) peb, options(nostack, pure, readonly));
        ((peb + 0x2C0) as *const u32).read()
    }
}

/// `WTSRegisterSessionNotification(hWnd, dwFlags)`: the window is to get
/// `WM_WTSSESSION_CHANGE`. Poly's session never changes state, so nothing is ever sent and
/// nothing has to be remembered.
#[unsafe(no_mangle)]
pub extern "system" fn WTSRegisterSessionNotification(hwnd: usize, flags: u32) -> i32 {
    match session::wts_register(hwnd, flags) {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}

/// `WTSUnRegisterSessionNotification(hWnd)`.
#[unsafe(no_mangle)]
pub extern "system" fn WTSUnRegisterSessionNotification(hwnd: usize) -> i32 {
    if hwnd == 0 { fail(ERROR_INVALID_PARAMETER) } else { 1 }
}

/// `WTSQuerySessionInformationW(hServer, SessionId, WTSInfoClass, ppBuffer, pBytesReturned)`:
/// the answer in a block the caller frees with `WTSFreeMemory`. The server is this computer
/// (`WTS_CURRENT_SERVER_HANDLE`, NULL): Poly has no other.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WTSQuerySessionInformationW(
    server: usize,
    session: u32,
    class: u32,
    buffer: *mut *mut u8,
    returned: *mut u32,
) -> i32 {
    if buffer.is_null() || returned.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's out-pointers.
    unsafe {
        buffer.write(core::ptr::null_mut());
        returned.write_unaligned(0);
    }
    if server != 0 {
        return fail(ERROR_INVALID_HANDLE);
    }
    let mut user = [0u16; 257];
    let name: Vec<u16> = "USERNAME\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated name; the buffer holds 257 characters.
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), user.as_mut_ptr(), 257) } as usize;
    let user = &user[..if n < 257 { n } else { 0 }];
    let mut computer = [0u16; 64];
    let mut len = 64u32;
    // SAFETY: the buffer holds `len` characters.
    let ok = unsafe { GetComputerNameW(computer.as_mut_ptr(), &mut len) };
    let computer = &computer[..if ok != 0 { (len as usize).min(63) } else { 0 }];
    let s = Session { id: session_id(), user, computer };
    let bytes = match session::wts_query(&s, session, class) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    // SAFETY: the process heap.
    let block = unsafe { process_heap::HeapAlloc(process_heap::GetProcessHeap(), 0, bytes.len().max(1)) };
    if block.is_null() {
        return fail(ERROR_NOT_ENOUGH_MEMORY);
    }
    // SAFETY: `block` holds `bytes.len()` bytes; the out-pointers are the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), block, bytes.len());
        buffer.write(block);
        returned.write_unaligned(bytes.len() as u32);
    }
    1
}

/// `WTSFreeMemory(pMemory)`: frees what a `WTS*` function allocated; NULL is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WTSFreeMemory(memory: *mut u8) {
    if !memory.is_null() {
        // SAFETY: a block `WTSQuerySessionInformationW` took from the process heap.
        unsafe { process_heap::HeapFree(process_heap::GetProcessHeap(), 0, memory) };
    }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn wtsapi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
