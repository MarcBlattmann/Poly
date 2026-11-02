//! The global allocator: `poly-heap`'s size-class slabs (the kernel's own
//! heap) over pages from the `map` call.
//!
//! The heap asks for runs aligned to more than a page (slabs are aligned
//! to their size, up to 32 KiB); `map` only promises page alignment, so
//! such a run is mapped with room to spare and the ends past the aligned
//! run are unmapped again ([`trim`]).

use crate::abi::{PAGE_SIZE, prot};
use crate::sys;
use core::ptr::NonNull;
use poly_heap::PageSource;

/// Pages from `map`, given back with `unmap`.
#[derive(Debug, Default)]
pub struct MapPages;

/// Where the aligned run of `len` bytes starts in a mapping of `total`
/// bytes at `base`, and how much to unmap before it and after it.
/// `align` is a power of two; `total >= len + align - PAGE_SIZE`.
pub fn trim(base: u64, len: u64, align: u64, total: u64) -> (u64, u64, u64) {
    let start = (base + align - 1) & !(align - 1);
    (start, start - base, base + total - (start + len))
}

// SAFETY: every run comes from a fresh private mapping of at least
// `count` pages that nothing else knows of, and is only unmapped when the
// heap gives it back.
unsafe impl PageSource for MapPages {
    fn alloc_pages(&mut self, count: usize, align: usize) -> Option<NonNull<u8>> {
        let len = (count as u64).checked_mul(PAGE_SIZE)?;
        let align = align as u64;
        let rw = prot::READ | prot::WRITE;
        if align <= PAGE_SIZE {
            return NonNull::new(sys::map(0, len, rw).ok()? as *mut u8);
        }
        let total = len.checked_add(align - PAGE_SIZE)?;
        let base = sys::map(0, total, rw).ok()?;
        let (start, head, tail) = trim(base, len, align, total);
        if head > 0 {
            let _ = sys::unmap(base, head);
        }
        if tail > 0 {
            let _ = sys::unmap(start + len, tail);
        }
        NonNull::new(start as *mut u8)
    }

    unsafe fn free_pages(&mut self, ptr: NonNull<u8>, count: usize) {
        let _ = sys::unmap(ptr.as_ptr() as u64, count as u64 * PAGE_SIZE);
    }
}

#[cfg(all(target_os = "none", not(test)))]
#[global_allocator]
static HEAP: poly_heap::LockedHeap<MapPages> = poly_heap::LockedHeap::new(MapPages);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_an_aligned_run_inside_the_mapping() {
        for base in (0x1000..0x40000).step_by(0x1000) {
            for (len, align) in [(0x8000, 0x8000), (0x1000, 0x2000), (0x3000, 0x4000), (0x10000, 0x8000)] {
                let total = len + align - PAGE_SIZE;
                let (start, head, tail) = trim(base, len, align, total);
                assert_eq!(start % align, 0);
                assert!(start >= base && start + len <= base + total);
                assert_eq!(head + len + tail, total);
                assert_eq!(head % PAGE_SIZE, 0);
                assert_eq!(tail % PAGE_SIZE, 0);
            }
        }
    }

    #[test]
    fn no_pages_on_the_host() {
        assert!(MapPages.alloc_pages(1, 4096).is_none());
        assert!(MapPages.alloc_pages(2, 8192).is_none());
        assert!(MapPages.alloc_pages(usize::MAX, 4096).is_none());
    }
}
