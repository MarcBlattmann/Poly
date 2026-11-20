//! Pre-zeroed frames (ADR 0263): a bounded pool of frames that idle CPUs zeroed ahead of time, so
//! that a page fault for a fresh anonymous page (or a commit's first touch) takes one instead of
//! zeroing 4 KiB while the program waits.
//!
//! The pool's frames come out of the frame allocator, whose free frames are the "dirty" list:
//! every frame freed there may hold an old page's contents, and only the zeroing in the idle loop
//! moves a frame into the pool. The rules here keep the pool out of the way of ADR 0250's
//! reserve:
//!
//! - **Size.** At most [`capacity`] frames: 1/256 of RAM, between 64 frames (256 KiB) and 4096
//!   (16 MiB).
//! - **Refill.** Only while the allocator has more than the low watermark plus a whole pool free
//!   ([`may_refill`]): filling the pool never brings free memory near the watermark.
//! - **Use.** A user frame may come from the pool under the same rule as from the allocator, with
//!   the pool's frames counted as free ([`user_may_take`]): they were free memory.
//! - **Drain.** At the low watermark the pool goes back to the allocator ([`must_drain`]), so the
//!   kernel's own allocations (heap, page tables, DMA), which take from the allocator only, never
//!   find its frames parked in the pool.

/// The most frames the pool holds for a machine of `total` frames.
pub fn capacity(total: u64) -> usize {
    const MIN: u64 = 64;
    const MAX: u64 = MAX_CAPACITY as u64;
    (total / 256).clamp(MIN, MAX).min(total / 16) as usize
}

/// The largest [`capacity`]: the size of the pool's storage.
pub const MAX_CAPACITY: usize = 4096;

/// Whether an idle CPU may take one more frame from the allocator to zero for the pool: the pool
/// is not full, and the allocator keeps more than the low watermark plus a whole pool free.
pub fn may_refill(allocator_free: u64, pooled: usize, capacity: usize, low_watermark: u64) -> bool {
    pooled < capacity && allocator_free > low_watermark.saturating_add(capacity as u64)
}

/// Whether a user frame may come from the pool: the reserve rule of ADR 0250
/// (`pressure::user_may_take`) with the pool's frames counted as free.
pub fn user_may_take(allocator_free: u64, pooled: usize, floor: u64) -> bool {
    pooled > 0 && crate::pressure::user_may_take(allocator_free.saturating_add(pooled as u64), floor)
}

/// Whether the pool must give its frames back: the allocator is at or below the low watermark.
pub fn must_drain(allocator_free: u64, low_watermark: u64) -> bool {
    allocator_free <= low_watermark
}

/// Physical addresses of zeroed frames, last in first out (the most recently zeroed frame is the
/// likeliest still in the cache).
pub struct Stack {
    frames: [u64; MAX_CAPACITY],
    len: usize,
}

impl Stack {
    pub const fn new() -> Stack {
        Stack { frames: [0; MAX_CAPACITY], len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Add a zeroed frame; `false` (and nothing kept) if the storage is full.
    pub fn push(&mut self, phys: u64) -> bool {
        if self.len == MAX_CAPACITY {
            return false;
        }
        self.frames[self.len] = phys;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> Option<u64> {
        self.len = self.len.checked_sub(1)?;
        Some(self.frames[self.len])
    }
}

impl core::fmt::Debug for Stack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stack").field("len", &self.len).finish_non_exhaustive()
    }
}

impl Default for Stack {
    fn default() -> Self {
        Stack::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_is_a_small_bounded_share_of_ram() {
        assert_eq!(capacity(65536), 256, "256 MiB: 1 MiB of zeroed frames");
        assert_eq!(capacity(1 << 20), 4096, "4 GiB: the 16 MiB ceiling");
        assert_eq!(capacity(1 << 22), 4096);
        assert_eq!(capacity(8192), 64, "32 MiB: the floor");
        assert_eq!(capacity(512), 32, "a tiny machine: at most 1/16 of it");
    }

    #[test]
    fn refilling_stops_well_above_the_watermark() {
        // Watermark 4096, pool of 256: refill only above 4352 free.
        assert!(may_refill(10_000, 0, 256, 4096));
        assert!(may_refill(4353, 255, 256, 4096));
        assert!(!may_refill(4352, 0, 256, 4096), "would bring free memory to the watermark");
        assert!(!may_refill(10_000, 256, 256, 4096), "full");
    }

    #[test]
    fn pooled_frames_count_as_free_for_the_reserve_rule() {
        assert!(!user_may_take(100_000, 0, 2048), "an empty pool gives nothing");
        assert!(user_may_take(2040, 10, 2048), "2050 free with the pool: above the floor");
        assert!(!user_may_take(2040, 8, 2048), "2048 with the pool: the floor itself");
    }

    #[test]
    fn the_pool_drains_at_the_watermark() {
        assert!(must_drain(4096, 4096) && must_drain(100, 4096));
        assert!(!must_drain(4097, 4096));
    }

    #[test]
    fn the_stack_is_last_in_first_out_and_bounded() {
        let mut s = Stack::new();
        assert!(s.is_empty() && s.pop().is_none());
        assert!(s.push(0x1000) && s.push(0x2000));
        assert_eq!((s.len(), s.pop(), s.pop(), s.pop()), (2, Some(0x2000), Some(0x1000), None));
        for i in 0..MAX_CAPACITY as u64 {
            assert!(s.push(i << 12));
        }
        assert!(!s.push(0x5000), "full");
        assert_eq!(s.len(), MAX_CAPACITY);
    }
}
