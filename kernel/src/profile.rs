//! `poly.debug.profile`: a sampling profile of the kernel, for finding
//! where system time goes (docs/DEBUGGING.md). Every timer tick that
//! interrupts kernel code counts its instruction address and the return
//! address of its frame (the caller); at power-off the most frequent ones
//! are printed, to be looked up in the kernel's symbols
//! (`addr2line -f -C -e <kernel> <address>`):
//!
//! ```text
//! profile: 12345 ticks: 10000 kernel, 2345 user
//! profile: pc 0xffffffff80123456 4000
//! profile: caller 0xffffffff80123abc 3900
//! ```
//!
//! Off unless the flag is given: then each sample is a few atomic
//! operations in the timer interrupt. The tables are fixed-size open
//! addressing hash tables; a full table drops samples (counted).

use crate::boot_info::BootInfo;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Kernel argument that turns the profile on.
pub const PROFILE_FLAG: &str = "poly.debug.profile";

static ON: AtomicBool = AtomicBool::new(false);

const SLOTS: usize = 4096;

/// One table: addresses (0 for a free slot) and their counts.
struct Table {
    keys: [AtomicU64; SLOTS],
    counts: [AtomicU64; SLOTS],
}

impl Table {
    const fn new() -> Table {
        Table { keys: [const { AtomicU64::new(0) }; SLOTS], counts: [const { AtomicU64::new(0) }; SLOTS] }
    }

    fn add(&self, key: u64) {
        if key == 0 {
            return;
        }
        let mut i = (key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 52) as usize % SLOTS;
        for _ in 0..64 {
            match self.keys[i].compare_exchange(0, key, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    self.counts[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(k) if k == key => {
                    self.counts[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => i = (i + 1) % SLOTS,
            }
        }
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }

    /// The `n` most frequent entries, most frequent first.
    fn top(&self, out: &mut [(u64, u64)]) {
        for (k, c) in self.keys.iter().zip(&self.counts) {
            let (k, c) = (k.load(Ordering::Relaxed), c.load(Ordering::Relaxed));
            if k == 0 {
                continue;
            }
            if let Some(min) = out.iter_mut().min_by_key(|e| e.1).filter(|e| e.1 < c) {
                *min = (k, c);
            }
        }
        out.sort_unstable_by_key(|e| core::cmp::Reverse(e.1));
    }
}

static PCS: Table = Table::new();
static CALLERS: Table = Table::new();
static KERNEL: AtomicU64 = AtomicU64::new(0);
static USER: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Read the flag (once, at boot).
pub fn init(boot: &BootInfo) {
    ON.store(boot.has_flag(PROFILE_FLAG), Ordering::Relaxed);
}

/// A timer tick interrupted code at `pc` with frame pointer `fp`.
/// `caller` reads the word at an address on the interrupted stack (the
/// frame's return address), `None` if it is not readable.
pub fn sample(user: bool, pc: u64, caller: impl FnOnce() -> Option<u64>) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    if user {
        USER.fetch_add(1, Ordering::Relaxed);
        return;
    }
    KERNEL.fetch_add(1, Ordering::Relaxed);
    PCS.add(pc);
    if let Some(c) = caller() {
        CALLERS.add(c);
    }
}

/// Print the profile (at power-off).
pub fn dump(out: &mut dyn Write) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let (k, u) = (KERNEL.load(Ordering::Relaxed), USER.load(Ordering::Relaxed));
    let _ =
        writeln!(out, "profile: {} ticks: {k} kernel, {u} user, {} dropped", k + u, DROPPED.load(Ordering::Relaxed));
    let mut top = [(0u64, 0u64); 60];
    PCS.top(&mut top);
    for (pc, n) in top.iter().filter(|e| e.1 > 0) {
        let _ = writeln!(out, "profile: pc {pc:#x} {n}");
    }
    let mut top = [(0u64, 0u64); 60];
    CALLERS.top(&mut top);
    for (pc, n) in top.iter().filter(|e| e.1 > 0) {
        let _ = writeln!(out, "profile: caller {pc:#x} {n}");
    }
}
