//! Placement: where a new mapping goes.

use super::*;

impl<F: Clone + PartialEq> VmMap<F> {
    /// Where a mapping of `len` bytes goes: the hint (rounded up to a page,
    /// and to `min_addr`) if that range is free, else the first hole in
    /// search order.
    pub(super) fn place(&self, hint: u64, len: u64) -> Option<u64> {
        let l = &self.layout;
        if let Some(h) = page_up(hint).filter(|&h| h != 0).map(|h| h.max(l.min_addr))
            && h.checked_add(len).is_some_and(|end| end <= l.max_addr && self.is_free(h, end))
        {
            return Some(h);
        }
        // The fallback covers the whole space, so it also finds a hole that
        // straddles mmap_base.
        match l.search {
            Search::TopDown => self.top_down(len, l.mmap_base).or_else(|| self.bottom_up(len, l.min_addr)),
            Search::BottomUp => self.bottom_up(len, l.mmap_base).or_else(|| self.top_down(len, l.max_addr)),
        }
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
        let gap = self.layout.stack_guard_gap;
        let max = self.layout.max_addr;
        let mut lo = base;
        let straddling = self.vmas.range(..base).next_back().map(|(_, v)| v).filter(|v| v.end > base);
        for v in straddling.into_iter().chain(self.vmas.range(base..).map(|(_, v)| v)) {
            let (bs, be) = v.blocked(gap);
            if bs >= lo && bs - lo >= len {
                return Some(lo);
            }
            lo = lo.max(be);
            if lo > max || max - lo < len {
                return None;
            }
        }
        (lo <= max && max - lo >= len).then_some(lo)
    }
}
