//! `SetErrorMode`, `GetErrorMode`, `SetThreadErrorMode`, `GetThreadErrorMode` (the sheet
//! `kernel32.dll/SetErrorMode`; the rules are `poly_kernelbase::errmode`'s).
//!
//! The process mode is the kernel's `ProcessDefaultHardErrorMode` (`NtSetInformationProcess`
//! class 12): a child created by `CreateProcessW` starts with the creator's value (`spawn.rs`
//! passes it as Poly's attribute 5). The thread mode is this DLL's, in a TLS index it allocates
//! the first time a thread sets one.

use crate::base::*;
use crate::nt::{self, Handle};
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::errmode;
use poly_ntobj::NtStatus;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtSetInformationProcess(process: Handle, class: u32, info: *const u8, length: u32) -> NtStatus;
}

/// `ProcessDefaultHardErrorMode` (`ntddk.h`).
const PROCESS_DEFAULT_HARD_ERROR_MODE: u32 = 12;

/// The process's error mode now.
pub(crate) fn process_mode() -> u32 {
    let mut mode = 0u32;
    // SAFETY: a 4-byte buffer of the class's size.
    let st = unsafe {
        nt::NtQueryInformationProcess(
            nt::CURRENT_PROCESS,
            PROCESS_DEFAULT_HARD_ERROR_MODE,
            (&raw mut mode).cast(),
            4,
            core::ptr::null_mut(),
        )
    };
    if st.is_success() { mode } else { 0 }
}

/// `SetErrorMode(uMode)`: the previous mode.
#[unsafe(no_mangle)]
pub extern "system" fn SetErrorMode(mode: u32) -> u32 {
    let old = process_mode();
    let new = errmode::process_mode_after(old, mode);
    // SAFETY: a 4-byte value.
    let _ = unsafe {
        NtSetInformationProcess(nt::CURRENT_PROCESS, PROCESS_DEFAULT_HARD_ERROR_MODE, (&raw const new).cast(), 4)
    };
    old
}

/// `GetErrorMode`.
#[unsafe(no_mangle)]
pub extern "system" fn GetErrorMode() -> u32 {
    process_mode()
}

/// The TLS index of the thread modes (`u32::MAX`: not allocated yet).
static THREAD_MODE_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);

fn thread_index(create: bool) -> Option<u32> {
    let i = THREAD_MODE_INDEX.load(Ordering::Acquire);
    if i != u32::MAX || !create {
        return (i != u32::MAX).then_some(i);
    }
    let n = crate::tls::TlsAlloc();
    if n == u32::MAX {
        return None;
    }
    match THREAD_MODE_INDEX.compare_exchange(u32::MAX, n, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(n),
        Err(other) => {
            crate::tls::TlsFree(n);
            Some(other)
        }
    }
}

/// The calling thread's mode.
fn thread_mode() -> u32 {
    let Some(i) = thread_index(false) else { return 0 };
    // The slot is 0 until a mode is stored (and for a thread that never stored one).
    let v = crate::tls::TlsGetValue(i) as usize as u32;
    // `TlsGetValue` sets the last error to 0; the sheet leaves the last error alone.
    v
}

/// `SetThreadErrorMode(dwNewMode, lpOldMode)`: three bits only (87 otherwise).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadErrorMode(mode: u32, old: *mut u32) -> i32 {
    if let Err(e) = errmode::thread_mode_check(mode) {
        return fail(e, FALSE);
    }
    let previous = thread_mode();
    if mode != previous {
        let Some(i) = thread_index(true) else { return fail(8, FALSE) }; // ERROR_NOT_ENOUGH_MEMORY
        if crate::tls::TlsSetValue(i, mode as usize as *mut u8) == 0 {
            return FALSE;
        }
    }
    if !old.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { old.write_unaligned(previous) };
    }
    TRUE
}

/// `GetThreadErrorMode`.
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadErrorMode() -> u32 {
    let last = last_error();
    let m = thread_mode();
    set_last_error(last);
    m
}
