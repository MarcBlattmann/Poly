//! Buddy allocator for physical frames.
//!
//! Memory is handed out in blocks of `2^order` frames (4 KiB each), always
//! aligned to their own size. A free block of order `k` that is not the
//! largest order has a *buddy*: the other half of the order `k + 1` block it
//! was split from. When both halves are free they merge back, so freeing
//! everything always restores the largest blocks the memory map allows.
//!
//! # Metadata
//!
//! The allocator never reads or writes the memory it manages. Free-list
//! links stored inside free frames would need every frame mapped, which the
//! kernel cannot promise for RAM above the boot mapping, and would make the
//! allocator untestable on the host. Instead all state lives in a word buffer
//! the caller provides, sized up front by [`BuddyAllocator::metadata_words`].
//! It covers the *span* from the lowest to the highest usable address,
//! rounded out to [`MAX_ORDER`] blocks, and holds two bitmaps per order:
//!
//! - **free**: one bit per block slot, set when that block is free and not
//!   part of a larger free block. Each free bitmap has summary levels on top
//!   (bit `i` of level `l + 1` is set when word `i` of level `l` is non-zero)
//!   up to a single word, so the lowest free block is found with one
//!   `trailing_zeros` per level instead of a scan.
//! - **allocated**: one bit per block slot, set while a block of exactly
//!   that order is handed out from there. [`BuddyAllocator::free`] requires
//!   it, which catches double frees, frees with the wrong order and frees of
//!   memory that was never handed out, without corrupting any state.
//!
//! That is about 4 bits per 4 KiB frame of span (1/8192 of the span: 256 KiB
//! of metadata for 2 GiB).
//!
//! # Complexity
//!
//! With `L` summary levels (`L <= 4` up to 64 GiB of span, `L <= 7` for the
//! whole 52-bit physical address space), [`allocate`](BuddyAllocator::allocate)
//! and [`free`](BuddyAllocator::free) are `O(MAX_ORDER * L)`: at most one
//! split or merge per order, each touching one word per level. Nothing is
//! proportional to the amount of memory.

use crate::{MemoryMap, PAGE_SIZE, RegionKind, align_down, align_up};
use core::fmt;

/// Largest block order. Order 10 is 1024 frames = 4 MiB: big enough for any
/// physically contiguous buffer the kernel plans to need (2 MiB huge pages are
/// order 9), small enough that the split/merge loops stay short and that the
/// span, which is rounded out to this size, wastes little metadata.
pub const MAX_ORDER: usize = 10;

const ORDERS: usize = MAX_ORDER + 1;

/// Frames per largest block.
const MAX_BLOCK_FRAMES: u64 = 1 << MAX_ORDER;

/// x86-64 physical addresses are at most 52 bits wide; usable RAM reported
/// above that is ignored. This also bounds the number of summary levels.
const MAX_PHYS: u64 = 1 << 52;

/// Levels of a free bitmap for `2^40` bits (all frames below [`MAX_PHYS`]):
/// `2^34`, `2^28`, ... `2^4`, then 1 word.
const MAX_LEVELS: usize = 7;

/// The metadata buffer passed to [`BuddyAllocator::new`] is too short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataTooSmall {
    /// Words the memory map needs, as [`BuddyAllocator::metadata_words`] says.
    pub needed: usize,
}

/// Why [`BuddyAllocator::free`] refused a block. The allocator is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeError {
    /// `order` is above [`MAX_ORDER`].
    BadOrder,
    /// The address is not aligned to the block size of `order`.
    Misaligned,
    /// The address is outside the memory this allocator manages.
    OutOfRange,
    /// No block of this order is allocated at this address: a double free,
    /// a free with the wrong order, or memory that was never handed out.
    NotAllocated,
}

/// A bitmap with summary levels, stored in the metadata buffer.
#[derive(Clone, Copy, Debug)]
struct BitTree {
    /// Word offset of each level, leaf level first. The last level is one word.
    level: [usize; MAX_LEVELS],
    levels: usize,
}

impl BitTree {
    /// Lay out a tree of `bits` bits starting at word `at`. Returns the tree
    /// and the first word after it.
    fn layout(bits: u64, mut at: usize) -> (BitTree, usize) {
        let mut tree = BitTree { level: [0; MAX_LEVELS], levels: 0 };
        let mut words = words_for(bits);
        loop {
            tree.level[tree.levels] = at;
            tree.levels += 1;
            at += words;
            if words == 1 {
                return (tree, at);
            }
            words = words.div_ceil(64);
        }
    }

    fn get(&self, m: &[u64], i: u64) -> bool {
        bit(m, self.level[0], i)
    }

    fn set(&self, m: &mut [u64], mut i: u64) {
        for &off in &self.level[..self.levels] {
            let w = &mut m[off + (i / 64) as usize];
            let was_empty = *w == 0;
            *w |= 1 << (i % 64);
            if !was_empty {
                // The level above already knows this word is non-empty.
                break;
            }
            i /= 64;
        }
    }

    fn clear(&self, m: &mut [u64], mut i: u64) {
        for &off in &self.level[..self.levels] {
            let w = &mut m[off + (i / 64) as usize];
            *w &= !(1 << (i % 64));
            if *w != 0 {
                break;
            }
            i /= 64;
        }
    }

    /// Lowest set bit, found by walking down from the top word.
    fn first(&self, m: &[u64]) -> Option<u64> {
        let mut i = 0u64;
        for &off in self.level[..self.levels].iter().rev() {
            let w = m[off + i as usize];
            if w == 0 {
                return None;
            }
            i = i * 64 + u64::from(w.trailing_zeros());
        }
        Some(i)
    }
}

/// Words for a flat bitmap of `bits` bits; at least one so every order has
/// a top word, even for an empty span.
fn words_for(bits: u64) -> usize {
    bits.div_ceil(64).max(1) as usize
}

fn bit(m: &[u64], off: usize, i: u64) -> bool {
    m[off + (i / 64) as usize] & (1 << (i % 64)) != 0
}

fn set_bit(m: &mut [u64], off: usize, i: u64, value: bool) {
    let w = &mut m[off + (i / 64) as usize];
    if value {
        *w |= 1 << (i % 64);
    } else {
        *w &= !(1 << (i % 64));
    }
}

/// Usable regions as page-aligned `[start, end)` ranges below [`MAX_PHYS`].
fn usable(map: &MemoryMap) -> impl Iterator<Item = (u64, u64)> + '_ {
    map.regions()
        .iter()
        .filter(|r| r.kind == RegionKind::Usable)
        .map(|r| (align_up(r.start.min(MAX_PHYS), PAGE_SIZE), align_down(r.end.min(MAX_PHYS), PAGE_SIZE)))
        .filter(|(s, e)| s < e)
}

/// Where everything lives; depends only on the memory map.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// Physical address of frame index 0, aligned to the largest block.
    base: u64,
    /// Frames in the span, a multiple of [`MAX_BLOCK_FRAMES`].
    frames: u64,
    free: [BitTree; ORDERS],
    /// Word offset of each order's allocated bitmap.
    allocated: [usize; ORDERS],
    words: usize,
}

impl Geometry {
    fn of(map: &MemoryMap) -> Geometry {
        let (lo, hi) = usable(map).fold((u64::MAX, 0), |(lo, hi), (s, e)| (lo.min(s), hi.max(e)));
        let (base, frames) = if lo < hi {
            let max_block = MAX_BLOCK_FRAMES * PAGE_SIZE;
            let base = align_down(lo, max_block);
            (base, (align_up(hi, max_block) - base) / PAGE_SIZE)
        } else {
            (0, 0)
        };

        let mut free = [BitTree { level: [0; MAX_LEVELS], levels: 0 }; ORDERS];
        let mut allocated = [0; ORDERS];
        let mut at = 0;
        for order in 0..ORDERS {
            let slots = frames >> order;
            (free[order], at) = BitTree::layout(slots, at);
            allocated[order] = at;
            at += words_for(slots);
        }
        Geometry { base, frames, free, allocated, words: at }
    }
}

/// Physical frame allocator over the usable RAM of a memory map.
pub struct BuddyAllocator<'a> {
    meta: &'a mut [u64],
    geo: Geometry,
    total: u64,
    free: u64,
    free_blocks: [u64; ORDERS],
}

impl fmt::Debug for BuddyAllocator<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuddyAllocator")
            .field("base", &self.geo.base)
            .field("span_frames", &self.geo.frames)
            .field("total", &self.total)
            .field("free", &self.free)
            .field("free_blocks", &self.free_blocks)
            .finish_non_exhaustive()
    }
}

impl<'a> BuddyAllocator<'a> {
    /// Bytes in a block of `order`.
    pub const fn block_size(order: usize) -> u64 {
        PAGE_SIZE << order
    }

    /// Length of the metadata buffer [`new`](Self::new) needs for `map`.
    pub fn metadata_words(map: &MemoryMap) -> usize {
        Geometry::of(map).words
    }

    /// Manage every usable frame of `map`, which must be normalized (see
    /// [`MemoryMap::normalized`]). Only the first
    /// [`metadata_words`](Self::metadata_words) words of `meta` are used; they
    /// are cleared here, so the buffer's previous contents do not matter.
    pub fn new(map: &MemoryMap, meta: &'a mut [u64]) -> Result<Self, MetadataTooSmall> {
        let geo = Geometry::of(map);
        if meta.len() < geo.words {
            return Err(MetadataTooSmall { needed: geo.words });
        }
        let meta = &mut meta[..geo.words];
        meta.fill(0);
        let mut a = BuddyAllocator { meta, geo, total: 0, free: 0, free_blocks: [0; ORDERS] };
        for (start, end) in usable(map) {
            let mut f = (start - geo.base) / PAGE_SIZE;
            let end = (end - geo.base) / PAGE_SIZE;
            a.total += end - f;
            while f < end {
                // Largest block that starts here and still fits.
                let mut order = (f.trailing_zeros() as usize).min(MAX_ORDER);
                while f + (1 << order) > end {
                    order -= 1;
                }
                a.insert(f, order);
                f += 1 << order;
            }
        }
        a.free = a.total;
        Ok(a)
    }

    /// Frames of usable RAM under management.
    pub fn total_frames(&self) -> u64 {
        self.total
    }

    /// Frames not currently allocated.
    pub fn free_frames(&self) -> u64 {
        self.free
    }

    /// Free blocks of exactly `order` (not counting those inside larger free
    /// blocks). Zero for orders above [`MAX_ORDER`].
    pub fn free_blocks(&self, order: usize) -> u64 {
        self.free_blocks.get(order).copied().unwrap_or(0)
    }

    /// Physical address of a free block of `2^order` frames, aligned to its
    /// size. Takes the lowest-addressed block of the smallest order that has
    /// one, splitting it down as needed. `None` if no block is large enough.
    pub fn allocate(&mut self, order: usize) -> Option<u64> {
        self.allocate_below(order, u64::MAX)
    }

    /// Like [`allocate`](Self::allocate), but the block must end at or below
    /// physical address `limit`: for memory that has to be reachable through
    /// a mapping that does not cover all RAM, or by a device that cannot
    /// address all of it. Takes the smallest order whose lowest-addressed
    /// free block starts low enough. Because that block is the lowest of its
    /// order, `None` means no free block of any order has `2^order` frames
    /// below `limit`.
    pub fn allocate_below(&mut self, order: usize, limit: u64) -> Option<u64> {
        if order > MAX_ORDER {
            return None;
        }
        let size = Self::block_size(order);
        let (from, slot) = (order..ORDERS).find_map(|k| {
            if self.free_blocks[k] == 0 {
                return None;
            }
            let slot = self.geo.free[k].first(self.meta)?;
            // Splitting keeps the lower half, so the block handed out starts
            // where this one does.
            let start = self.geo.base + (slot << k) * PAGE_SIZE;
            (start + size <= limit).then_some((k, slot))
        })?;
        self.geo.free[from].clear(self.meta, slot);
        self.free_blocks[from] -= 1;

        // Keep the lower half at every split, free the upper half.
        let frame = slot << from;
        for k in (order..from).rev() {
            self.geo.free[k].set(self.meta, (frame >> k) + 1);
            self.free_blocks[k] += 1;
        }
        set_bit(self.meta, self.geo.allocated[order], frame >> order, true);
        self.free -= 1 << order;
        Some(self.geo.base + frame * PAGE_SIZE)
    }

    /// Return a block from [`allocate`](Self::allocate) with the same
    /// `order`, merging it with free buddies.
    pub fn free(&mut self, addr: u64, order: usize) -> Result<(), FreeError> {
        if order > MAX_ORDER {
            return Err(FreeError::BadOrder);
        }
        if !addr.is_multiple_of(Self::block_size(order)) {
            return Err(FreeError::Misaligned);
        }
        let frame = addr.checked_sub(self.geo.base).ok_or(FreeError::OutOfRange)? / PAGE_SIZE;
        if frame >= self.geo.frames {
            return Err(FreeError::OutOfRange);
        }
        let slot = frame >> order;
        if !bit(self.meta, self.geo.allocated[order], slot) {
            return Err(FreeError::NotAllocated);
        }
        set_bit(self.meta, self.geo.allocated[order], slot, false);
        self.free += 1 << order;
        self.insert(frame, order);
        Ok(())
    }

    /// Mark the block at `frame` free, merging upwards while its buddy is
    /// free too.
    fn insert(&mut self, mut frame: u64, mut order: usize) {
        while order < MAX_ORDER {
            // The span is a whole number of largest blocks, so the buddy
            // slot always exists.
            let buddy = (frame >> order) ^ 1;
            if !self.geo.free[order].get(self.meta, buddy) {
                break;
            }
            self.geo.free[order].clear(self.meta, buddy);
            self.free_blocks[order] -= 1;
            frame &= !(1 << order);
            order += 1;
        }
        self.geo.free[order].set(self.meta, frame >> order);
        self.free_blocks[order] += 1;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::Region;
    use RegionKind::*;
    use std::collections::HashSet;
    use std::vec;
    use std::vec::Vec;

    const MIB: u64 = 1 << 20;
    const BLOCK: u64 = BuddyAllocator::block_size(MAX_ORDER);

    fn map(rs: &[(u64, u64, RegionKind)]) -> MemoryMap {
        let mut m = MemoryMap::new();
        for &(s, e, k) in rs {
            m.push(Region::new(s, e, k)).unwrap();
        }
        m.normalized().unwrap()
    }

    fn meta_for(m: &MemoryMap) -> Vec<u64> {
        // Garbage, to prove `new` clears what it uses.
        vec![0xdead_beef_dead_beef; BuddyAllocator::metadata_words(m)]
    }

    fn usable_contains(m: &MemoryMap, addr: u64, len: u64) -> bool {
        m.regions().iter().any(|r| r.kind == Usable && r.start <= addr && addr + len <= r.end)
    }

    /// Allocate order-0 frames until exhausted.
    fn drain(a: &mut BuddyAllocator<'_>) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some(f) = a.allocate(0) {
            out.push(f);
        }
        out
    }

    #[test]
    fn one_block_splits_and_merges() {
        let m = map(&[(BLOCK, 2 * BLOCK, Usable)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        assert_eq!(a.total_frames(), 1024);
        assert_eq!(a.free_blocks(MAX_ORDER), 1);

        let f = a.allocate(0).unwrap();
        assert_eq!(f, BLOCK);
        assert_eq!(a.free_frames(), 1023);
        // One free buddy left behind at every order below the top.
        for k in 0..MAX_ORDER {
            assert_eq!(a.free_blocks(k), 1, "order {k}");
        }
        assert_eq!(a.free_blocks(MAX_ORDER), 0);

        // The next frame is the order-0 buddy; then the order-1 buddy.
        assert_eq!(a.allocate(0), Some(BLOCK + PAGE_SIZE));
        assert_eq!(a.allocate(1), Some(BLOCK + 2 * PAGE_SIZE));
        assert_eq!(a.free_blocks(0), 0);
        assert_eq!(a.free_blocks(1), 0);

        a.free(BLOCK + PAGE_SIZE, 0).unwrap();
        a.free(BLOCK, 0).unwrap();
        // The two frames merged into an order-1 block.
        assert_eq!(a.free_blocks(0), 0);
        assert_eq!(a.free_blocks(1), 1);
        a.free(BLOCK + 2 * PAGE_SIZE, 1).unwrap();
        assert_eq!(a.free_blocks(MAX_ORDER), 1);
        for k in 0..MAX_ORDER {
            assert_eq!(a.free_blocks(k), 0, "order {k}");
        }
        assert_eq!(a.free_frames(), 1024);
    }

    #[test]
    fn blocks_are_aligned_to_their_size() {
        let m = map(&[(0x10_0000, 64 * MIB + 0x3000, Usable)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        for round in 0..3 {
            for order in (0..=MAX_ORDER).rev().chain(0..=MAX_ORDER) {
                let f = a.allocate(order).unwrap();
                let size = BuddyAllocator::block_size(order);
                assert_eq!(f % size, 0, "round {round} order {order} at {f:#x}");
                assert!(usable_contains(&m, f, size));
            }
        }
    }

    #[test]
    fn unaligned_regions_are_used_exactly() {
        // Neither end is aligned to anything above a page.
        let m = map(&[(0x3000, 9 * MIB + 0x5000, Usable), (13 * MIB + 0x1000, 21 * MIB - 0x7000, Usable)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        let expected = m.total(Usable) / PAGE_SIZE;
        assert_eq!(a.total_frames(), expected);
        assert_eq!(a.free_frames(), expected);
        // There is room for some whole 4 MiB blocks in the middle.
        assert!(a.free_blocks(MAX_ORDER) >= 2);

        let frames = drain(&mut a);
        assert_eq!(frames.len() as u64, expected);
        assert_eq!(frames.iter().collect::<HashSet<_>>().len(), frames.len());
        for &f in &frames {
            assert!(usable_contains(&m, f, PAGE_SIZE), "{f:#x}");
        }
        assert_eq!(a.free_frames(), 0);
    }

    #[test]
    fn holes_and_reserved_ranges_are_never_handed_out() {
        let m = map(&[
            (0, 32 * MIB, Usable),
            (0, 0x10_0000, Reserved),
            (5 * MIB + 0x2000, 7 * MIB, Kernel),
            (12 * MIB, 12 * MIB + 0x1000, AcpiNvs),
            // Hole from 32 to 40 MiB.
            (40 * MIB, 48 * MIB, Usable),
            (44 * MIB, 45 * MIB, Bad),
        ]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        assert_eq!(a.total_frames(), m.total(Usable) / PAGE_SIZE);
        let mut seen = 0;
        for order in (0..=MAX_ORDER).cycle() {
            let Some(f) = a.allocate(order) else {
                if order == 0 {
                    break;
                }
                continue;
            };
            assert!(usable_contains(&m, f, BuddyAllocator::block_size(order)), "order {order} at {f:#x}");
            seen += 1 << order;
        }
        assert_eq!(seen, a.total_frames());
    }

    #[test]
    fn memory_above_4g_is_managed() {
        let m = map(&[(MIB, 2048 * MIB, Usable), (4096 * MIB, 6144 * MIB, Usable)]);
        let mut meta = meta_for(&m);
        // 6 GiB of span at 4 bits per frame, plus under 2% for the summaries.
        assert!(meta.len() * 8 <= (6 << 30) / 8192 * 102 / 100);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        assert_eq!(a.total_frames(), (2047 + 2048) * 256);
        let mut high = 0;
        while let Some(f) = a.allocate(MAX_ORDER) {
            assert!(usable_contains(&m, f, BLOCK));
            high += u64::from(f >= 4096 * MIB);
        }
        assert_eq!(high, 512);
    }

    #[test]
    fn exhaustion() {
        let m = map(&[(0, 3 * PAGE_SIZE, Usable)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        assert_eq!(a.allocate(2), None, "only 3 frames exist");
        assert_eq!(a.allocate(MAX_ORDER + 1), None);
        let one = a.allocate(1).unwrap();
        assert_eq!(one, 0);
        assert_eq!(a.allocate(1), None);
        assert_eq!(a.allocate(0), Some(2 * PAGE_SIZE));
        assert_eq!(a.allocate(0), None);
        assert_eq!(a.free_frames(), 0);
        a.free(one, 1).unwrap();
        assert_eq!(a.allocate(0), Some(0));
    }

    #[test]
    fn allocate_below_skips_small_blocks_above_the_limit() {
        // Like a machine where only the low half of RAM is mapped: RAM ends
        // in small leftover blocks, which plain `allocate` prefers.
        let (start, top, limit) = (MIB + 0x5000, 64 * MIB - 0x21000, 32 * MIB);
        let m = map(&[(start, top, Usable)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        let total = a.free_frames();
        let size = BuddyAllocator::block_size(2);
        let high = a.allocate(2).unwrap();
        assert!(high >= limit, "plain allocate takes a leftover block at the top");
        a.free(high, 2).unwrap();

        let mut low = Vec::new();
        while let Some(f) = a.allocate_below(2, limit) {
            assert_eq!(f % size, 0);
            assert!(f + size <= limit && usable_contains(&m, f, size), "{f:#x}");
            low.push((f, 2));
        }
        // Every aligned 16 KiB block from the first one in RAM up to the
        // limit, the last one ending exactly there.
        assert_eq!(low.len() as u64, (limit - align_up(start, size)) / size);
        // The frames before the first aligned block are still there.
        let blocks = low.len();
        while let Some(f) = a.allocate_below(0, limit) {
            assert!(f < align_up(start, size) && usable_contains(&m, f, PAGE_SIZE), "{f:#x}");
            low.push((f, 0));
        }
        assert_eq!((low.len() - blocks) as u64, (align_up(start, size) - start) / PAGE_SIZE);
        let high = a.allocate(2).unwrap();
        assert!(high >= limit, "memory above the limit is still there");
        low.push((high, 2));
        assert_eq!(a.allocate_below(0, 0), None);
        assert_eq!(a.allocate_below(MAX_ORDER + 1, u64::MAX), None);
        for (f, order) in low {
            a.free(f, order).unwrap();
        }
        assert_eq!(a.free_frames(), total);
    }

    #[test]
    fn bad_frees_are_rejected_without_damage() {
        let m = map(&[(BLOCK, 3 * BLOCK, Usable), (2 * BLOCK, 2 * BLOCK + MIB, Reserved)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        let start = a.free_frames();
        let f = a.allocate(2).unwrap();
        let g = a.allocate(0).unwrap();

        assert_eq!(a.free(f, MAX_ORDER + 1), Err(FreeError::BadOrder));
        assert_eq!(a.free(f + PAGE_SIZE, 2), Err(FreeError::Misaligned));
        assert_eq!(a.free(f + 1, 0), Err(FreeError::Misaligned));
        assert_eq!(a.free(0, 0), Err(FreeError::OutOfRange));
        assert_eq!(a.free(3 * BLOCK, 0), Err(FreeError::OutOfRange));
        assert_eq!(a.free(u64::MAX - PAGE_SIZE + 1, 0), Err(FreeError::OutOfRange));
        // Wrong order, both smaller and larger than allocated.
        assert_eq!(a.free(f, 1), Err(FreeError::NotAllocated));
        assert_eq!(a.free(f, 3), Err(FreeError::NotAllocated));
        // Inside the reserved range, and free but never allocated.
        assert_eq!(a.free(2 * BLOCK, 0), Err(FreeError::NotAllocated));
        assert_eq!(a.free(2 * BLOCK + 2 * MIB, 0), Err(FreeError::NotAllocated));

        a.free(f, 2).unwrap();
        assert_eq!(a.free(f, 2), Err(FreeError::NotAllocated), "double free");
        a.free(g, 0).unwrap();
        assert_eq!(a.free(g, 0), Err(FreeError::NotAllocated), "double free");
        assert_eq!(a.free_frames(), start);
        assert_eq!(a.free_blocks(MAX_ORDER), 1);
    }

    #[test]
    fn metadata_must_be_large_enough() {
        let m = map(&[(0, 64 * MIB, Usable)]);
        let needed = BuddyAllocator::metadata_words(&m);
        let mut meta = vec![0; needed - 1];
        assert_eq!(BuddyAllocator::new(&m, &mut meta).unwrap_err(), MetadataTooSmall { needed });
        // A larger buffer is fine; the tail is left alone.
        let mut meta = vec![7; needed + 3];
        {
            let a = BuddyAllocator::new(&m, &mut meta).unwrap();
            assert_eq!(a.total_frames(), 64 * MIB / PAGE_SIZE);
        }
        assert_eq!(&meta[needed..], &[7, 7, 7]);
    }

    #[test]
    fn empty_map() {
        let m = map(&[(0, 64 * MIB, Reserved)]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        assert_eq!(a.total_frames(), 0);
        assert_eq!(a.allocate(0), None);
        assert_eq!(a.free(0, 0), Err(FreeError::OutOfRange));
    }

    /// Deterministic xorshift64 so the stress test needs no dependencies.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn randomized_stress() {
        let m = map(&[
            (0x1000, 37 * MIB + 0x3000, Usable),
            (16 * MIB + 0x5000, 17 * MIB, Reserved),
            (40 * MIB + 0x7000, 61 * MIB, Usable),
            (130 * MIB, 131 * MIB + 0x2000, Usable),
        ]);
        let mut meta = meta_for(&m);
        let mut a = BuddyAllocator::new(&m, &mut meta).unwrap();
        let initial: Vec<u64> = (0..=MAX_ORDER).map(|k| a.free_blocks(k)).collect();
        let total = a.total_frames();
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let mut live: Vec<(u64, usize)> = Vec::new();
        let mut owned: HashSet<u64> = HashSet::new();
        let mut refused = 0;

        for step in 0..20_000 {
            if live.is_empty() || rng.below(100) < 60 {
                // Small orders are far more common, like real callers.
                let order = (rng.below(1 << MAX_ORDER).trailing_zeros() as usize).min(MAX_ORDER);
                match a.allocate(order) {
                    Some(f) => {
                        let size = BuddyAllocator::block_size(order);
                        assert_eq!(f % size, 0, "step {step}");
                        assert!(usable_contains(&m, f, size), "step {step}: {f:#x} order {order}");
                        for frame in (f..f + size).step_by(PAGE_SIZE as usize) {
                            assert!(owned.insert(frame), "step {step}: {frame:#x} handed out twice");
                        }
                        live.push((f, order));
                    }
                    None => {
                        assert!((order..=MAX_ORDER).all(|k| a.free_blocks(k) == 0), "step {step}");
                        refused += 1;
                    }
                }
            } else {
                let (f, order) = live.swap_remove(rng.below(live.len() as u64) as usize);
                a.free(f, order).unwrap();
                assert_eq!(a.free(f, order), Err(FreeError::NotAllocated), "step {step}");
                for frame in (f..f + BuddyAllocator::block_size(order)).step_by(PAGE_SIZE as usize) {
                    owned.remove(&frame);
                }
            }
            assert_eq!(a.free_frames() + owned.len() as u64, total, "step {step}");
        }

        // The mix is tuned so memory runs out now and then.
        assert!(refused > 0);
        while let Some((f, order)) = live.pop() {
            a.free(f, order).unwrap();
        }
        assert_eq!(a.free_frames(), total);
        let restored: Vec<u64> = (0..=MAX_ORDER).map(|k| a.free_blocks(k)).collect();
        assert_eq!(restored, initial);
    }
}
