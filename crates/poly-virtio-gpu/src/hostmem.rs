//! Placing blobs in the host-visible shared memory region (§5.7.4,
//! `RESOURCE_MAP_BLOB` takes the offset the driver chose): a first-fit
//! allocator of page-aligned ranges over the region's length. The device
//! never sees it; it only keeps two mappings from overlapping.

extern crate alloc;

use alloc::collections::BTreeMap;

/// Granularity of the ranges (a page: mappings are made of whole pages).
pub const ALIGN: u64 = 4096;

/// The ranges of a region of `len` bytes in use.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Ranges {
    len: u64,
    /// Offset -> length of each range in use.
    used: BTreeMap<u64, u64>,
}

impl Ranges {
    /// A region of `len` bytes (rounded down to whole pages), all free.
    pub fn new(len: u64) -> Ranges {
        Ranges { len: len / ALIGN * ALIGN, used: BTreeMap::new() }
    }

    /// The region's length.
    pub const fn len(&self) -> u64 {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bytes in use.
    pub fn used(&self) -> u64 {
        self.used.values().sum()
    }

    /// The lowest free offset with room for `size` bytes (whole pages),
    /// now in use; `None` if no gap is large enough.
    pub fn alloc(&mut self, size: u64) -> Option<u64> {
        let size = size.max(1).checked_next_multiple_of(ALIGN)?;
        let mut at = 0u64;
        for (&start, &n) in &self.used {
            if start - at >= size {
                break;
            }
            at = start + n;
        }
        if at.checked_add(size)? > self.len {
            return None;
        }
        self.used.insert(at, size);
        Some(at)
    }

    /// Give back the range at `offset`. False if none starts there.
    pub fn free(&mut self, offset: u64) -> bool {
        self.used.remove(&offset).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_fit_whole_pages_and_reuse() {
        let mut r = Ranges::new(10 * ALIGN + 100);
        assert_eq!(r.len(), 10 * ALIGN);
        assert_eq!(r.alloc(1), Some(0));
        assert_eq!(r.alloc(ALIGN + 1), Some(ALIGN));
        assert_eq!(r.alloc(3 * ALIGN), Some(3 * ALIGN));
        assert_eq!(r.used(), 6 * ALIGN);
        // The gap left by the second range is reused by what fits in it.
        assert!(r.free(ALIGN));
        assert!(!r.free(ALIGN));
        assert_eq!(r.alloc(3 * ALIGN), Some(6 * ALIGN));
        assert_eq!(r.alloc(2 * ALIGN), Some(ALIGN));
        // Full.
        assert_eq!(r.alloc(2 * ALIGN), None);
        assert_eq!(r.alloc(ALIGN), Some(9 * ALIGN));
        assert_eq!(r.alloc(1), None);
        assert_eq!(r.alloc(u64::MAX), None);
        assert!(Ranges::new(100).is_empty());
    }
}
