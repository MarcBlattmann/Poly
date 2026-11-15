//! Vectored exception handlers (Learn: `AddVectoredExceptionHandler`,
//! `RemoveVectoredExceptionHandler`, "Vectored Exception Handling"): a
//! process-wide list that `KiUserExceptionDispatcher` calls, first to last,
//! before any frame-based handler. A handler gets an `EXCEPTION_POINTERS`
//! (`winnt.h`: the record pointer, then the context pointer) and returns
//! `EXCEPTION_CONTINUE_EXECUTION` (-1) to resume the (perhaps changed)
//! context or `EXCEPTION_CONTINUE_SEARCH` (0) to go on.

use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// `EXCEPTION_CONTINUE_EXECUTION` (`excpt.h`).
#[cfg_attr(target_arch = "x86", allow(dead_code))] // 32-bit dispatch is a later stage (ADR 0186)
pub const CONTINUE_EXECUTION: i32 = -1;

struct List {
    flag: AtomicBool,
    /// (handle, handler), in call order.
    entries: UnsafeCell<Vec<(u64, u64)>>,
}

// SAFETY: access is serialized by `flag`.
unsafe impl Sync for List {}

static LIST: List = List { flag: AtomicBool::new(false), entries: UnsafeCell::new(Vec::new()) };
/// The vectored continue handlers (`AddVectoredContinueHandler`): a list of their own.
static CONTINUE: List = List { flag: AtomicBool::new(false), entries: UnsafeCell::new(Vec::new()) };
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn with_list<R>(list: &List, f: impl FnOnce(&mut Vec<(u64, u64)>) -> R) -> R {
    while list.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only accessor.
    let r = f(unsafe { &mut *list.entries.get() });
    list.flag.store(false, Ordering::Release);
    r
}

fn with<R>(f: impl FnOnce(&mut Vec<(u64, u64)>) -> R) -> R {
    with_list(&LIST, f)
}

/// Add `handler` to `list`, first (`first` non-zero) or last; returns a
/// handle for the removal (an opaque non-null value, distinct for every call
/// of either list).
fn add(list: &List, first: u32, handler: *mut u8) -> *mut u8 {
    if handler.is_null() {
        return core::ptr::null_mut();
    }
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) << 4;
    with_list(list, |l| {
        let e = (handle, handler as u64);
        if first != 0 { l.insert(0, e) } else { l.push(e) }
    });
    handle as *mut u8
}

/// Remove the handler `handle` names from `list`: non-zero if it was there.
fn remove(list: &List, handle: *mut u8) -> u32 {
    with_list(list, |l| match l.iter().position(|&(h, _)| h == handle as u64) {
        Some(i) => {
            l.remove(i);
            1
        }
        None => 0,
    })
}

/// `RtlAddVectoredExceptionHandler(First, Handler)`: add `handler` first
/// (`first` non-zero) or last; returns a handle for the removal (an opaque
/// non-null value, distinct for every call).
#[unsafe(no_mangle)]
pub extern "system" fn RtlAddVectoredExceptionHandler(first: u32, handler: *mut u8) -> *mut u8 {
    add(&LIST, first, handler)
}

/// `RtlRemoveVectoredExceptionHandler(Handle)`: non-zero if the handle was
/// registered.
#[unsafe(no_mangle)]
pub extern "system" fn RtlRemoveVectoredExceptionHandler(handle: *mut u8) -> u32 {
    remove(&LIST, handle)
}

/// `RtlAddVectoredContinueHandler(First, Handler)` (Learn:
/// `AddVectoredContinueHandler`): registered first or last like an exception
/// handler, in a list of its own. They are called after an exception has been
/// resolved by a handler that continued execution (and not when none did,
/// `specs/windows/kernel32.dll/AddVectoredContinueHandler.yaml`); their results
/// change nothing.
#[unsafe(no_mangle)]
pub extern "system" fn RtlAddVectoredContinueHandler(first: u32, handler: *mut u8) -> *mut u8 {
    add(&CONTINUE, first, handler)
}

/// `RtlRemoveVectoredContinueHandler(Handle)`: non-zero if the handle was a
/// registered continue handler.
#[unsafe(no_mangle)]
pub extern "system" fn RtlRemoveVectoredContinueHandler(handle: *mut u8) -> u32 {
    remove(&CONTINUE, handle)
}

/// Call the vectored continue handlers, first to last, on the exception at `record` / `context`
/// that was resolved: a handler continued execution.
///
/// # Safety
/// `record` and `context` are a valid `EXCEPTION_RECORD` and `CONTEXT`.
#[cfg_attr(target_arch = "x86", allow(dead_code))]
pub unsafe fn call_continue_handlers(record: *mut u8, context: *mut u8) {
    let handlers: Vec<u64> = with_list(&CONTINUE, |l| l.iter().map(|&(_, h)| h).collect());
    let mut pointers: [*mut u8; 2] = [record, context];
    type Handler = unsafe extern "system" fn(*mut [*mut u8; 2]) -> i32;
    for h in handlers {
        // SAFETY: a handler the program registered, with the documented
        // `PVECTORED_EXCEPTION_HANDLER` signature.
        unsafe { core::mem::transmute::<usize, Handler>(h as usize)(&mut pointers) };
    }
}

/// Call the vectored handlers on the exception at `record` / `context`
/// (the kernel's copies on this thread's stack); true if one asked to
/// continue execution. The list is copied first, so a handler may add or
/// remove handlers.
///
/// # Safety
/// `record` and `context` are a valid `EXCEPTION_RECORD` and `CONTEXT`.
#[cfg_attr(target_arch = "x86", allow(dead_code))]
pub unsafe fn call_handlers(record: *mut u8, context: *mut u8) -> bool {
    let handlers: Vec<u64> = with(|l| l.iter().map(|&(_, h)| h).collect());
    let mut pointers: [*mut u8; 2] = [record, context];
    type Handler = unsafe extern "system" fn(*mut [*mut u8; 2]) -> i32;
    for h in handlers {
        // SAFETY: a handler the program registered, with the documented
        // `PVECTORED_EXCEPTION_HANDLER` signature.
        if unsafe { core::mem::transmute::<usize, Handler>(h as usize)(&mut pointers) } == CONTINUE_EXECUTION {
            return true;
        }
    }
    false
}
