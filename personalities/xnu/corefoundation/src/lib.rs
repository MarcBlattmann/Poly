//! Poly's own `CoreFoundation.framework` (ADR 0260): a Mach-O dylib with
//! Apple's install name (`/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation`),
//! so programs linked against the macOS SDK load it in place of Apple's.
//! `cargo xtask corefoundation` builds this crate for `x86_64-apple-darwin`
//! and links it with `rust-lld -flavor darwin` against Poly's libSystem;
//! the export list is `exports/CoreFoundation.txt`.
//!
//! Inputs: Apple's public CoreFoundation documentation, the SDK headers
//! `CoreFoundation/*.h` for constants, callback tables and the `CFSTR`
//! layout, the export names of the SDK's `.tbd`, and the spec sheets and
//! visible tests of `specs/macos/CoreFoundation.framework`. No part of
//! Apple's CoreFoundation, CF-Lite or swift-corelibs-foundation was read or
//! disassembled.
//!
//! Every object starts with a pointer-sized class word, so the Objective-C
//! runtime can later treat CF objects as objects (toll-free bridging); the
//! type is kept in CoreFoundation's own header word, never derived from the
//! class word (`base.rs`).
//!
//! v0 is single-threaded, like Poly's libSystem v0: reference counts are
//! atomic, everything else assumes one thread.

#![no_std]

extern crate alloc;

mod allocator;
mod array;
mod base;
mod bundle;
mod data;
mod date;
mod dict;
mod format;
mod number;
mod prefs;
mod runloop;
mod string;
mod sys;
mod url;

/// A library-wide variable. v0 has one thread, so nothing locks it; each
/// user takes it once per call and never across a callback into the
/// program.
struct Global<T>(core::cell::UnsafeCell<T>);

// SAFETY: one thread in v0 (see the crate documentation).
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(core::cell::UnsafeCell::new(v))
    }

    /// The value.
    ///
    /// # Safety
    /// No other reference to it is alive (one thread, one use per call, none
    /// held across a callback).
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self) -> &mut T {
        // SAFETY: the caller's contract.
        unsafe { &mut *self.0.get() }
    }
}

/// Rust's heap is libSystem's `malloc`.
struct Malloc;

// SAFETY: malloc returns 16-byte aligned blocks of at least the size asked;
// larger alignments go through posix_memalign.
unsafe impl core::alloc::GlobalAlloc for Malloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() <= 16 {
            // SAFETY: plain C call.
            unsafe { sys::malloc(layout.size().max(1)) as *mut u8 }
        } else {
            let mut p = core::ptr::null_mut();
            // SAFETY: plain C call with a valid out pointer.
            let r = unsafe { sys::posix_memalign(&mut p, layout.align(), layout.size().max(1)) };
            if r == 0 { p as *mut u8 } else { core::ptr::null_mut() }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        // SAFETY: `ptr` came from malloc or posix_memalign.
        unsafe { sys::free(ptr as *mut core::ffi::c_void) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: core::alloc::Layout, new_size: usize) -> *mut u8 {
        if layout.align() <= 16 {
            // SAFETY: `ptr` came from malloc.
            unsafe { sys::realloc(ptr as *mut core::ffi::c_void, new_size.max(1)) as *mut u8 }
        } else {
            // SAFETY: the default: allocate, copy, free.
            unsafe {
                let new = self.alloc(core::alloc::Layout::from_size_align_unchecked(new_size, layout.align()));
                if !new.is_null() {
                    core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                    self.dealloc(ptr, layout);
                }
                new
            }
        }
    }
}

#[global_allocator]
static HEAP: Malloc = Malloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    base::crash("internal error")
}

/// `core` refers to the 16-byte atomic load of the compiler's runtime from
/// code this library never runs (the `Debug` of 128-bit atomics; the
/// baseline x86-64 CPU has no `cmpxchg16b`). `-dead_strip` removes the
/// caller; this definition only lets the link resolve it.
#[unsafe(no_mangle)]
extern "C" fn __atomic_load_16(_p: *const u8, _order: i32) -> u128 {
    base::crash("16-byte atomics are not supported")
}
