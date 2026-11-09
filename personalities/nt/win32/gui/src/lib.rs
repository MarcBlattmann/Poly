//! What Poly's GUI DLLs share (ADR 0128):
//!
//! - the global allocator: the process heap (`HeapAlloc`, 16-byte aligned);
//! - [`SpinLock`] for `gdi32`'s object table;
//! - the last error, and the explicit not-implemented stubs
//!   ([`not_implemented_stubs!`]: `ERROR_CALL_NOT_IMPLEMENTED` and the
//!   failure value of the function's Learn page) for exports whose slice
//!   has not arrived; `grep -rn not_implemented` finds them.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`).
pub const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
}

struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments
// are refused.
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

/// Set the calling thread's last error.
pub fn set_last_error(e: u32) {
    // SAFETY: a kernel32 export without preconditions.
    unsafe { SetLastError(e) }
}

/// The calling thread's last error.
pub fn last_error() -> u32 {
    // SAFETY: a kernel32 export without preconditions.
    unsafe { GetLastError() }
}

/// What every stub does before returning its failure value.
pub fn not_implemented() {
    set_last_error(ERROR_CALL_NOT_IMPLEMENTED);
}

/// A spin lock around a value.
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only while holding the lock.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    /// Run `f` with the value locked.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held, so this is the only reference.
        let r = f(unsafe { &mut *self.value.get() });
        self.locked.store(false, Ordering::Release);
        r
    }
}

/// A NUL-terminated UTF-16 string, or `n` units of it when `n >= 0`.
///
/// # Safety
/// `p` points at such a string (or `n` readable units).
pub unsafe fn wide(p: *const u16, n: i32) -> alloc::vec::Vec<u16> {
    let mut v = alloc::vec::Vec::new();
    if p.is_null() {
        return v;
    }
    let mut i = 0usize;
    loop {
        if n >= 0 && i >= n as usize {
            break;
        }
        // SAFETY: the caller's promise.
        let c = unsafe { *p.add(i) };
        if n < 0 && c == 0 {
            break;
        }
        v.push(c);
        i += 1;
    }
    v
}

/// Exported stubs: `Name(arguments...) => failure value;` each. Every
/// argument is taken as a pointer-sized integer, which the x64 calling
/// convention makes the same as the documented integer, handle and pointer
/// parameters (none of these functions takes a structure or a float by
/// value); the value is returned in `rax`, of which callers read the
/// documented type's width (`-1` as `BOOL`, `CLR_INVALID` as `COLORREF`).
#[macro_export]
macro_rules! not_implemented_stubs {
    ($( $(#[$meta:meta])* $name:ident ( $($arg:ident),* ) => $fail:expr ; )*) => {
        $(
            $(#[$meta])*
            #[unsafe(no_mangle)]
            #[allow(unused_variables, non_snake_case)]
            pub extern "system" fn $name($($arg: usize),*) -> usize {
                $crate::not_implemented();
                $fail
            }
        )*
    };
}
