//! Placement: where a new mapping goes.

use super::*;

impl<F: Clone + PartialEq> VmMap<F> {
    /// Where a mapping of `len` bytes goes: the hint (rounded up to a page,
    /// and raised to [`Layout::min_addr`]: "a nearby page boundary, but
    /// always above or equal to `mmap_min_addr`", mmap(2)) if the whole
    /// range lies in the address space and is free, else the first hole in
    /// search order ("If another mapping already exists there, the kernel
    /// picks a new address").
    pub(super) fn place(&self, hint: u64, len: u64) -> Option<u64> {
        let l = &self.layout;
        if let Some(h) = self.hint_fits(hint, len, l.max_addr) {
            return Some(h);
        }
        // The fallback covers the whole space, so it also finds a hole that
        // straddles mmap_base.
        match l.search {
            Search::TopDown => self.top_down(len, l.mmap_base).or_else(|| self.bottom_up(len, l.min_addr)),
            Search::BottomUp => self.bottom_up(len, l.mmap_base).or_else(|| self.top_down(len, l.max_addr)),
        }
    }

    /// Where a mapping of `len` bytes that must end at or below `limit`
    /// goes (`MAP_32BIT`, with `limit` 2 GiB): the hint if it fits as for
    /// [`VmMap::place`], else the lowest hole at or above [`LOW_SEARCH_BASE`],
    /// else the lowest hole anywhere below `limit`.
    pub(super) fn place_below(&self, hint: u64, len: u64, limit: u64) -> Option<u64> {
        let limit = limit.min(self.layout.max_addr);
        if let Some(h) = self.hint_fits(hint, len, limit) {
            return Some(h);
        }
        let base = LOW_SEARCH_BASE.max(self.layout.min_addr);
        self.bottom_up_in(len, base, limit).or_else(|| self.bottom_up_in(len, self.layout.min_addr, limit))
    }

    /// The hint rounded up to a page and raised to `min_addr` (0 is no
    /// hint), if the range from there ends at or below `limit` and is free.
    fn hint_fits(&self, hint: u64, len: u64, limit: u64) -> Option<u64> {
        let h = page_up(hint).filter(|&h| h != 0)?.max(self.layout.min_addr);
        let end = h.checked_add(len)?;
        (end <= limit && self.is_free(h, end)).then_some(h)
    }

    /// Whether `[start, end)` is free of regions and of stack guard gaps.
    pub(super) fn is_free(&self, start: u64, end: u64) -> bool {
        let gap = self.layout.stack_guard_gap;
        for v in self.vmas.range(..end.saturating_add(gap)).rev().map(|(_, v)| v) {
            if v.end <= start {
                // Ends are sorted: nothing further down reaches the range.
                return true;
            }
            let (bs, be) = v.blocked(gap);
            if bs < end && be > start {
                return false;
            }
        }
        true
    }

    /// The highest `len`-byte hole that ends at or below `limit`.
    pub(super) fn top_down(&self, len: u64, limit: u64) -> Option<u64> {
        let gap = self.layout.stack_guard_gap;
        let min = self.layout.min_addr;
        let mut top = limit;
        for v in self.vmas.range(..limit.saturating_add(gap)).rev().map(|(_, v)| v) {
            let (bs, be) = v.blocked(gap);
            let floor = be.max(min);
            if top >= floor && top - floor >= len {
                return Some(top - len);
            }
            top = top.min(bs);
            if top < min || top - min < len {
                return None;
            }
        }
        (top >= min && top - min >= len).then(|| top - len)
    }

    /// The lowest `len`-byte hole that starts at or above `base`.
    pub(super) fn bottom_up(&self, len: u64, base: u64) -> Option<u64> {
        self.bottom_up_in(len, base, self.layout.max_addr)
    }

    /// The lowest `len`-byte hole that starts at or above `base` and ends at
    /// or below `max`.
    fn bottom_up_in(&self, len: u64, base: u64, max: u64) -> Option<u64> {
        let gap = self.layout.stack_guard_gap;
        let mut lo = base;
        let straddling = self.vmas.range(..base).next_back().map(|(_, v)| v).filter(|v| v.end > base);
        for v in straddling.into_iter().chain(self.vmas.range(base..).map(|(_, v)| v)) {
            let (bs, be) = v.blocked(gap);
            if bs >= lo && bs - lo >= len {
                return (lo <= max && max - lo >= len).then_some(lo);
            }
            lo = lo.max(be);
            if lo > max || max - lo < len {
                return None;
            }
        }
        (lo <= max && max - lo >= len).then_some(lo)
    }
}
