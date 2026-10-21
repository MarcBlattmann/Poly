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
pub const CONTINUE_EXECUTION: i32 = -1;

struct List {
    flag: AtomicBool,
    /// (handle, handler), in call order.
    entries: UnsafeCell<Vec<(u64, u64)>>,
}

// SAFETY: access is serialized by `flag`.
unsafe impl Sync for List {}

static LIST: List = List { flag: AtomicBool::new(false), entries: UnsafeCell::new(Vec::new()) };
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn with<R>(f: impl FnOnce(&mut Vec<(u64, u64)>) -> R) -> R {
    while LIST.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only accessor.
    let r = f(unsafe { &mut *LIST.entries.get() });
    LIST.flag.store(false, Ordering::Release);
    r
}

/// `RtlAddVectoredExceptionHandler(First, Handler)`: add `handler` first
/// (`first` non-zero) or last; returns a handle for the removal (an opaque
/// non-null value, distinct for every call).
#[unsafe(no_mangle)]
pub extern "C" fn RtlAddVectoredExceptionHandler(first: u32, handler: *mut u8) -> *mut u8 {
    if handler.is_null() {
        return core::ptr::null_mut();
    }
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) << 4;
    with(|l| {
        let e = (handle, handler as u64);
        if first != 0 { l.insert(0, e) } else { l.push(e) }
    });
    handle as *mut u8
}

/// `RtlRemoveVectoredExceptionHandler(Handle)`: non-zero if the handle was
/// registered.
#[unsafe(no_mangle)]
pub extern "C" fn RtlRemoveVectoredExceptionHandler(handle: *mut u8) -> u32 {
    with(|l| match l.iter().position(|&(h, _)| h == handle as u64) {
        Some(i) => {
            l.remove(i);
            1
        }
        None => 0,
    })
}

/// Call the vectored handlers on the exception at `record` / `context`
/// (the kernel's copies on this thread's stack); true if one asked to
/// continue execution. The list is copied first, so a handler may add or
/// remove handlers.
///
/// # Safety
/// `record` and `context` are a valid `EXCEPTION_RECORD` and `CONTEXT`.
pub unsafe fn call_handlers(record: *mut u8, context: *mut u8) -> bool {
    let handlers: Vec<u64> = with(|l| l.iter().map(|&(_, h)| h).collect());
    let mut pointers: [*mut u8; 2] = [record, context];
    type Handler = unsafe extern "system" fn(*mut [*mut u8; 2]) -> i32;
    for h in handlers {
        // SAFETY: a handler the program registered, with the documented
        // `PVECTORED_EXCEPTION_HANDLER` signature.
        if unsafe { core::mem::transmute::<u64, Handler>(h)(&mut pointers) } == CONTINUE_EXECUTION {
            return true;
        }
    }
    false
}
