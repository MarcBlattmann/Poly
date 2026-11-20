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
    /// How much more (or less) expendable it is, from -1000 to 1000, as Linux's `oom_score_adj`
    /// (proc_pid_oom_score_adj(5)): a Linux process's own value; for a Windows process what its
    /// relations say (`poly_personality_nt::oomadj`). -1000: never ended.
    pub adj: i32,
}

/// `oom_score_adj`'s range.
pub const OOM_SCORE_ADJ_MIN: i32 = -1000;
pub const OOM_SCORE_ADJ_MAX: i32 = 1000;

/// A candidate's score, as Linux's: its frames plus `adj` thousandths of all `total` frames
/// (proc_pid_oom_score_adj(5): "the value is added to the badness score ... in units of
/// one-thousandth of the memory available"). `None` for one with `adj` -1000 (never ended).
pub fn badness(c: &Candidate, total: u64) -> Option<i64> {
    let adj = c.adj.clamp(OOM_SCORE_ADJ_MIN, OOM_SCORE_ADJ_MAX);
    if adj == OOM_SCORE_ADJ_MIN {
        return None;
    }
    Some(c.resident as i64 + i64::from(adj) * (total.min(i64::MAX as u64 / 2000) as i64) / 1000)
}

/// The process to end when memory has run out: the one with the highest [`badness`] that is
/// neither protected nor ending already and holds memory (the youngest of equals: the larger
/// id). `None` if there is none. `total`: the machine's frames.
pub fn choose_victim(candidates: &[Candidate], total: u64) -> Option<u32> {
    candidates
        .iter()
        .filter(|c| !c.protected && !c.ending && c.resident > 0)
        .filter_map(|c| badness(c, total).map(|b| (b, c.pid)))
        .max()
        .map(|(_, pid)| pid)
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
        let c = |pid, resident, protected, ending| Candidate { pid, resident, protected, ending, adj: 0 };
        let t = 1 << 20;
        assert_eq!(choose_victim(&[], t), None);
        assert_eq!(choose_victim(&[c(1, 900, true, false), c(7, 10, false, false)], t), Some(7));
        assert_eq!(choose_victim(&[c(5, 300, false, true), c(6, 200, false, false)], t), Some(6));
        assert_eq!(choose_victim(&[c(8, 100, false, false), c(9, 100, false, false)], t), Some(9));
        assert_eq!(choose_victim(&[c(3, 0, false, false), c(4, 50, true, false)], t), None);
    }

    #[test]
    fn the_score_adjustment_makes_children_go_first() {
        let c = |pid, resident, adj| Candidate { pid, resident, protected: false, ending: false, adj };
        let total = 1 << 20; // 4 GiB
        // A browser of 270 MiB and a renderer of 100 MiB that says it is expendable (300, as
        // Linux Chromium writes): the renderer goes.
        let browser = c(10, 270 << 8, 0);
        let renderer = c(11, 100 << 8, 300);
        assert_eq!(choose_victim(&[browser, renderer], total), Some(11));
        // Without the adjustment the larger one goes.
        assert_eq!(choose_victim(&[browser, c(11, 100 << 8, 0)], total), Some(10));
        // -1000: never, even when it is the only one.
        assert_eq!(choose_victim(&[c(12, 500, -1000)], total), None);
        // A negative adjustment protects a smaller process less than -1000 would.
        assert_eq!(badness(&c(13, 1000, -500), 2000), Some(0));
        // Values beyond the range count as its ends.
        assert_eq!(badness(&c(14, 10, 5000), 1000), Some(1010));
    }
}
