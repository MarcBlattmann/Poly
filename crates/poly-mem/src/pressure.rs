//! Memory pressure (ADR 0250): how many frames user memory leaves to the kernel, when reclaim
//! starts, and which process an out-of-memory ends.
//!
//! The kernel heap, page tables and DMA buffers take their frames from the same allocator as user
//! pages and the page cache. A kernel allocation that fails ends in a panic (there is no recovering
//! from every `Vec` push), so user memory and the page cache must never take the last frames:
//!
//! - **Reserve.** User pages and page cache pages are only handed out while more than
//!   [`reserve_frames`] frames are free. The rest is the kernel's.
//! - **Low watermark.** Below [`low_watermark`] free frames, the page cache gives back clean pages
//!   in the background, so the reserve is rarely reached while cache is still there to drop.
//! - **Out of memory.** When user memory has been refused and free frames stay at the reserve, the
//!   process holding the most memory is ended ([`choose_victim`]), never one of the desktop's.

/// Frames kept for the kernel: 1/32 of RAM, at least 8 MiB and at most 128 MiB (4 KiB frames).
pub fn reserve_frames(total: u64) -> u64 {
    const MIN: u64 = 2048;
    const MAX: u64 = 32768;
    (total / 32).clamp(MIN, MAX).min(total / 2)
}

/// Below this many free frames the page cache gives back clean pages: twice the reserve.
pub fn low_watermark(reserve: u64) -> u64 {
    reserve.saturating_mul(2)
}

/// Whether a frame may go to user memory or the page cache with `free` frames free.
pub fn user_may_take(free: u64, reserve: u64) -> bool {
    free > reserve
}

/// A process the out-of-memory policy may end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub pid: u32,
    /// Frames its memory holds.
    pub resident: u64,
    /// One the policy never ends: the desktop's own programs, init.
    pub protected: bool,
    /// It is ending already: ending it again frees nothing more.
    pub ending: bool,
}

/// The process to end when memory has run out: the one holding the most frames that is neither
/// protected nor ending already (the youngest of equals: the larger id). `None` if there is none,
/// or if none holds any memory.
pub fn choose_victim(candidates: &[Candidate]) -> Option<u32> {
    candidates
        .iter()
        .filter(|c| !c.protected && !c.ending && c.resident > 0)
        .max_by_key(|c| (c.resident, c.pid))
        .map(|c| c.pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reserve_grows_with_ram_within_bounds() {
        assert_eq!(reserve_frames(65536), 2048, "256 MiB: the 8 MiB floor");
        assert_eq!(reserve_frames(1 << 20), 32768, "4 GiB: 1/32, 128 MiB");
        assert_eq!(reserve_frames(1 << 22), 32768, "16 GiB: the 128 MiB ceiling");
        assert_eq!(reserve_frames(3000), 1500, "a tiny machine keeps half for user memory");
        assert_eq!(low_watermark(2048), 4096);
        assert!(user_may_take(2049, 2048) && !user_may_take(2048, 2048));
    }

    #[test]
    fn the_victim_is_the_largest_process_that_may_go() {
        let c = |pid, resident, protected, ending| Candidate { pid, resident, protected, ending };
        assert_eq!(choose_victim(&[]), None);
        assert_eq!(choose_victim(&[c(1, 900, true, false), c(7, 10, false, false)]), Some(7));
        assert_eq!(choose_victim(&[c(5, 300, false, true), c(6, 200, false, false)]), Some(6));
        assert_eq!(choose_victim(&[c(8, 100, false, false), c(9, 100, false, false)]), Some(9));
        assert_eq!(choose_victim(&[c(3, 0, false, false), c(4, 50, true, false)]), None);
    }
}
