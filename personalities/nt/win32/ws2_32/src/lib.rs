//! Our `ws2_32.dll` (ADR 0095): Winsock 2 for programs, over sockets that
//! are file objects on `\Device\Afd` (the NT personality's device in front
//! of the kernel's TCP/IP stack). A `SOCKET` is the endpoint's handle.
//!
//! - [`afd`]: opening endpoints and the device's requests
//!   (`NtDeviceIoControlFile` with `poly_winsock::afd`'s codes).
//! - [`sock`]: `WSAStartup` / `WSACleanup`, the socket calls (`socket` to
//!   `closesocket`, `send` / `recv` and their `WSA*` forms with overlapped
//!   I/O, `select`, `WSAPoll`, `ioctlsocket`, options, names) and the
//!   event functions.
//! - [`names`]: `getaddrinfo` / `GetAddrInfoW` with the stub resolver,
//!   `getnameinfo`, `gethostbyname`, `gethostname`, services and
//!   protocols, address text (`inet_pton` and friends), byte order.
//!
//! Errors are reported through `WSAGetLastError`, which is the thread's
//! last-error value (Learn: "WSAGetLastError": "the same as
//! GetLastError"); the codes come from `poly_winsock::error`. Every
//! decision that does not need a process is in the host-tested
//! `poly-winsock`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

pub mod afd;
pub mod names;
pub mod sock;

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;
use poly_winsock::consts::*;

pub type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetLastError() -> u32;
    pub fn SetLastError(e: u32);
    pub fn GetProcessHeap() -> Handle;
    pub fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut u8;
    pub fn HeapFree(heap: Handle, flags: u32, mem: *mut u8) -> i32;
    pub fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    pub fn SetEvent(h: Handle) -> i32;
    pub fn ResetEvent(h: Handle) -> i32;
    pub fn CloseHandle(h: Handle) -> i32;
    pub fn WaitForSingleObjectEx(h: Handle, ms: u32, alertable: i32) -> u32;
    pub fn WaitForMultipleObjectsEx(n: u32, handles: *const Handle, all: i32, ms: u32, alertable: i32) -> u32;
}

/// `alloc` (used by `poly-winsock`) on the process heap, 16-byte aligned.
struct ProcessHeap;

/// `HEAP_ZERO_MEMORY` (`winnt.h`).
pub const HEAP_ZERO_MEMORY: u32 = 0x8;

// SAFETY: HeapAlloc returns blocks aligned to 16 bytes on x64 (Learn:
// "HeapAlloc"); stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        heap_alloc(layout.size().max(1))
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block of `alloc`.
        unsafe { heap_free(ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

/// Zeroed bytes of the process heap (null on failure).
pub fn heap_alloc(n: usize) -> *mut u8 {
    // SAFETY: kernel32 exports without preconditions.
    unsafe { HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, n) }
}

/// Free a block of [`heap_alloc`].
pub unsafe fn heap_free(p: *mut u8) {
    if !p.is_null() {
        // SAFETY: the caller passes a block of the process heap.
        unsafe { HeapFree(GetProcessHeap(), 0, p) };
    }
}

/// Set the thread's WSA error.
pub fn set_error(e: u32) {
    // SAFETY: no preconditions.
    unsafe { SetLastError(e) }
}

/// Fail a call that returns `SOCKET_ERROR`.
pub fn fail(e: u32) -> i32 {
    set_error(e);
    SOCKET_ERROR
}

/// `WSAStartup` calls not yet matched by `WSACleanup`.
static STARTED: AtomicU32 = AtomicU32::new(0);

/// Fails with `WSANOTINITIALISED` before `WSAStartup` (Learn: every
/// function but a few needs it).
pub fn check_started() -> Result<(), u32> {
    if STARTED.load(Ordering::SeqCst) == 0 { Err(WSANOTINITIALISED) } else { Ok(()) }
}

/// `WSAGetLastError`.
#[unsafe(no_mangle)]
pub extern "system" fn WSAGetLastError() -> i32 {
    // SAFETY: no preconditions.
    unsafe { GetLastError() as i32 }
}

/// `WSASetLastError`.
#[unsafe(no_mangle)]
pub extern "system" fn WSASetLastError(e: i32) {
    set_error(e as u32);
}

/// `WSAStartup` (Learn): the version negotiated as
/// `poly_winsock::negotiate` says, `WSADATA` filled (the `_WIN64`
/// layout), the call counted. Errors are returned, not set.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAStartup(version: u16, data: *mut u8) -> i32 {
    if data.is_null() {
        return WSAEFAULT as i32;
    }
    let (used, high) = match poly_winsock::negotiate(version) {
        Ok(v) => v,
        Err(e) => {
            // Learn: wHighVersion still tells what is supported.
            // SAFETY: the caller's WSADATA.
            unsafe { write_wsadata(data, poly_winsock::HIGHEST_VERSION, poly_winsock::HIGHEST_VERSION) };
            return e as i32;
        }
    };
    // SAFETY: the caller's WSADATA.
    unsafe { write_wsadata(data, used, high) };
    STARTED.fetch_add(1, Ordering::SeqCst);
    0
}

unsafe fn write_wsadata(data: *mut u8, used: u16, high: u16) {
    // SAFETY: WSADATA_SIZE bytes the caller owns.
    unsafe {
        core::ptr::write_bytes(data, 0, WSADATA_SIZE);
        core::ptr::write_unaligned(data.cast::<u16>(), used);
        core::ptr::write_unaligned(data.add(2).cast::<u16>(), high);
        let desc = b"WinSock 2.0";
        core::ptr::copy_nonoverlapping(desc.as_ptr(), data.add(WSADATA_DESCRIPTION), desc.len());
        let status = b"Running";
        core::ptr::copy_nonoverlapping(status.as_ptr(), data.add(WSADATA_STATUS), status.len());
    }
}

/// `WSACleanup`: one `WSAStartup` undone; `WSANOTINITIALISED` without
/// one. Sockets stay open (Learn: they are reset when the last call ends
/// the use; Poly leaves that to `closesocket` and process exit, to be
/// confirmed by the spec agent).
#[unsafe(no_mangle)]
pub extern "system" fn WSACleanup() -> i32 {
    let r = STARTED.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
    if r.is_err() {
        return fail(WSANOTINITIALISED);
    }
    0
}

/// A NUL-terminated byte string as a `&str` (`None` for null or bad
/// UTF-8).
pub unsafe fn cstr<'a>(p: *const u8) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(p, n)).ok()
    }
}

/// A NUL-terminated UTF-16 string as a `String` (`None` for null).
pub unsafe fn wstr(p: *const u16) -> Option<alloc::string::String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string.
    let units = unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    };
    Some(char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect())
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn ws2_32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
