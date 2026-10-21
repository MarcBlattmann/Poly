//! The process heap and the global allocator, over [`poly_rtl::heap`].

use crate::sys::SysPlatform;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU64, Ordering};
use poly_rtl::heap::{Heap, flags};

static PROCESS_HEAP: AtomicU64 = AtomicU64::new(0);

/// The process heap (`RtlProcessHeap`, `GetProcessHeap`). Valid after
/// [`init`].
pub fn process_heap() -> Heap {
    Heap(PROCESS_HEAP.load(Ordering::Acquire) as *mut u8)
}

/// Create the process heap. Called once, first thing in `LdrInitializeThunk`.
pub fn init() {
    if PROCESS_HEAP.load(Ordering::Acquire) != 0 {
        return;
    }
    // SAFETY: platform memory; called once before any allocation.
    let heap = unsafe { Heap::create(&SysPlatform, flags::GROWABLE, None, 0) };
    if let Some(h) = heap {
        PROCESS_HEAP.store(h.0 as u64, Ordering::Release);
    }
}

/// The global allocator: the process heap.
pub struct ProcessHeapAlloc;

// SAFETY: the process heap is thread-safe (serialized), and returns aligned
// blocks; `dealloc` gets a pointer the heap returned.
unsafe impl GlobalAlloc for ProcessHeapAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // The heap aligns to 16 bytes; a stricter request over-allocates and
        // the aligned pointer is within the block (the block header is not
        // consulted on free for aligned sub-blocks, so only 16-byte-aligned
        // requests use the fast path; larger alignments fall back to a
        // header-free page run in the heap's big path).
        let size = layout.size().max(1);
        // SAFETY: the heap is valid.
        let p = unsafe { process_heap().allocate(&SysPlatform, 0, size.max(layout.align())) };
        p.unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: `ptr` came from `alloc`.
        unsafe { process_heap().free(&SysPlatform, 0, ptr) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, _layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` came from `alloc`.
        unsafe { process_heap().reallocate(&SysPlatform, 0, ptr, new_size.max(1)) }.unwrap_or(core::ptr::null_mut())
    }
}
