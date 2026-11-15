//! Thread and process priorities (Learn: `GetThreadPriority`, `SetThreadPriority`, `GetPriorityClass`,
//! `SetPriorityClass`, `GetThreadPriorityBoost`, `SetThreadPriorityBoost`; "Scheduling Priorities"): the
//! values are kept by the kernel (a thread's `ThreadBasePriority`, a process's `ProcessPriorityClass`);
//! Poly's scheduler does not use them yet.
//!
//! The Win32 thread priorities are -2 to 2 and the two ends `THREAD_PRIORITY_IDLE` (-15) and
//! `THREAD_PRIORITY_TIME_CRITICAL` (15); the kernel keeps the ends as -16 and 16. A process class is
//! `IDLE_PRIORITY_CLASS` ... `REALTIME_PRIORITY_CLASS` as `ProcessPriorityClass` numbers them.

use crate::base::*;
use crate::nt::{self, Handle};
use poly_kernelbase::error::*;

/// `THREAD_PRIORITY_ERROR_RETURN` (`winbase.h`).
const THREAD_PRIORITY_ERROR_RETURN: i32 = 0x7FFF_FFFF;
/// `ThreadBasePriority` of `NtSetInformationThread`; Poly's own query class for the same value.
const THREAD_BASE_PRIORITY: u32 = 3;
const THREAD_POLY_BASE_PRIORITY: u32 = 0x1000;
/// `ProcessPriorityClass`.
const PROCESS_PRIORITY_CLASS: u32 = 18;

/// The Win32 priority class of an NT one (`winbase.h`).
fn win32_class(nt_class: u8) -> u32 {
    match nt_class {
        1 => 0x40,
        3 => 0x80,
        4 => 0x100,
        5 => 0x4000,
        6 => 0x8000,
        _ => 0x20,
    }
}

/// The NT priority class of a Win32 one.
fn nt_class(class: u32) -> Option<u8> {
    Some(match class {
        0x40 => 1,
        0x20 => 2,
        0x80 => 3,
        0x100 => 4,
        0x4000 => 5,
        0x8000 => 6,
        _ => return None,
    })
}

/// `GetThreadPriority`: `THREAD_PRIORITY_ERROR_RETURN` and the error when the handle cannot be read.
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadPriority(thread: Handle) -> i32 {
    let mut base = 0i32;
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtQueryInformationThread(thread, THREAD_POLY_BASE_PRIORITY, (&mut base as *mut i32).cast(), 4, core::ptr::null_mut())
    };
    if !st.is_success() {
        return fail_status(st, THREAD_PRIORITY_ERROR_RETURN);
    }
    base.clamp(-15, 15)
}

/// `SetThreadPriority`: -2 to 2, 15 and -15 (and 16 and -16, which are the same ends) are valid.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadPriority(thread: Handle, priority: i32) -> i32 {
    let base = match priority {
        -2..=2 => priority,
        15 | 16 => 16,
        -15 | -16 => -16,
        _ => return fail(ERROR_INVALID_PARAMETER, FALSE),
    };
    // SAFETY: valid pointer.
    let st = unsafe { nt::NtSetInformationThread(thread, THREAD_BASE_PRIORITY, (&base as *const i32).cast(), 4) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `GetPriorityClass`: 0 and the error when the handle cannot be read.
#[unsafe(no_mangle)]
pub extern "system" fn GetPriorityClass(process: Handle) -> u32 {
    let mut info = [0u8; 2];
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtQueryInformationProcess(process, PROCESS_PRIORITY_CLASS, info.as_mut_ptr(), 2, core::ptr::null_mut()) };
    if !st.is_success() {
        return fail_status(st, 0);
    }
    win32_class(info[1])
}

/// `SetPriorityClass`.
#[unsafe(no_mangle)]
pub extern "system" fn SetPriorityClass(process: Handle, class: u32) -> i32 {
    let Some(c) = nt_class(class) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let info = [0u8, c];
    // SAFETY: valid pointer.
    let st = unsafe { nt::NtSetInformationProcess(process, PROCESS_PRIORITY_CLASS, info.as_ptr(), 2) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `GetThreadPriorityBoost`: Poly never turns the boost off.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadPriorityBoost(thread: Handle, disabled: *mut i32) -> i32 {
    if disabled.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let mut base = 0i32;
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtQueryInformationThread(thread, THREAD_POLY_BASE_PRIORITY, (&mut base as *mut i32).cast(), 4, core::ptr::null_mut())
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's out-pointer.
    unsafe { disabled.write(FALSE) };
    TRUE
}

/// `SetThreadPriorityBoost`: accepted for a thread that can be set, and ignored.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadPriorityBoost(thread: Handle, _disable: i32) -> i32 {
    let mut base = 0i32;
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtQueryInformationThread(thread, THREAD_POLY_BASE_PRIORITY, (&mut base as *mut i32).cast(), 4, core::ptr::null_mut())
    };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}
