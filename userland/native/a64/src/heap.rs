//! The runtime's allocator: `poly-heap` over the heap area the kernel maps
//! at exec (`poly_a64_xnu::layout::RUNTIME_HEAP`). There is no `mmap` to
//! ask for more, so runs of pages are carved from the area and freed runs
//! are kept for reuse (first fit).

use core::ptr::NonNull;
use poly_a64_xnu::layout::{RUNTIME_HEAP, RUNTIME_HEAP_SIZE};
use poly_heap::{LockedHeap, PAGE_SIZE, PageSource};

/// Freed runs kept for reuse; more are dropped (leaked).
const FREE_RUNS: usize = 256;

/// Pages of the heap area.
pub struct AreaPages {
    next: u64,
    end: u64,
    /// Freed runs: (address, pages); a count of 0 is an empty slot.
    free: [(u64, usize); FREE_RUNS],
}

impl AreaPages {
    const fn new() -> AreaPages {
        AreaPages { next: RUNTIME_HEAP, end: RUNTIME_HEAP + RUNTIME_HEAP_SIZE, free: [(0, 0); FREE_RUNS] }
    }
}

// SAFETY: every run is inside the heap area the kernel mapped readable and
// writable for this process, and is handed out once until it is freed.
unsafe impl PageSource for AreaPages {
    fn alloc_pages(&mut self, count: usize, align: usize) -> Option<NonNull<u8>> {
        let align = align.max(PAGE_SIZE) as u64;
        let len = (count as u64).checked_mul(PAGE_SIZE as u64)?;
        // A freed run that fits exactly and is aligned.
        if let Some(slot) = self.free.iter_mut().find(|(a, n)| *n == count && a % align == 0) {
            let at = slot.0;
            *slot = (0, 0);
            return NonNull::new(at as *mut u8);
        }
        let at = self.next.checked_add(align - 1)? & !(align - 1);
        let end = at.checked_add(len)?;
        if end > self.end {
            return None;
        }
        // Pages skipped for alignment are kept as a free run.
        if at > self.next {
            let skipped = ((at - self.next) / PAGE_SIZE as u64) as usize;
            if let Some(slot) = self.free.iter_mut().find(|(_, n)| *n == 0) {
                *slot = (self.next, skipped);
            }
        }
        self.next = end;
        NonNull::new(at as *mut u8)
    }

    unsafe fn free_pages(&mut self, ptr: NonNull<u8>, count: usize) {
        if let Some(slot) = self.free.iter_mut().find(|(_, n)| *n == 0) {
            *slot = (ptr.as_ptr() as u64, count);
        }
    }
}

#[global_allocator]
static HEAP: LockedHeap<AreaPages> = LockedHeap::new(AreaPages::new());
