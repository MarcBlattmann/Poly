//! Pre-zeroed frames (ADR 0263): idle CPUs zero frames ahead of time, and a fault for a fresh
//! anonymous page (Linux `mmap`, an NT commit's first touch, a Mach `vm_allocate`) takes one
//! instead of zeroing 4 KiB while the program waits (`UserFrame::zeroed`). The rules are
//! `poly_mem::zeropool`'s; this is the pool itself and its place in the kernel:
//!
//! - **Refill** happens in the idle loop ([`refill_one`], one frame per pass, interrupts on): a CPU
//!   with nothing to run zeroes a frame and looks again, so a thread made runnable meanwhile (its
//!   reschedule interrupt) runs at once. Not while the kernel tests run (they count free frames).
//! - **Use**: [`take`], under ADR 0250's reserve rule with the pool's frames counted as free.
//! - **Drain**: [`drain`] at the low watermark (`frames::after_user_allocation`): the pool goes
//!   back to the allocator for the kernel's own allocations.
//!
//! The allocator's free frames are the dirty list: a freed frame (ADR 0254's deferred frees
//! included) goes back there as it was, and only [`refill_one`] moves a frame into the pool,
//! after zeroing it. Frames in the pool belong to no one; a frame taken from it is the taker's.
//!

use crate::frames::{self, FRAMES};
use crate::sync::SpinLock;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use poly_mem::PAGE_SIZE;
use poly_mem::zeropool::{self as policy, Stack};

static POOL: SpinLock<Stack> = SpinLock::new(Stack::new());
/// The pool's size limit; 0 until [`init`] (no pool).
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
/// Frames in the pool (read without the lock, by the idle loop's quick look).
static LEN: AtomicUsize = AtomicUsize::new(0);
/// Frames taken from the pool and frames zeroed into it since boot (`/proc`-style statistics).
static TAKEN: AtomicU64 = AtomicU64::new(0);
static ZEROED: AtomicU64 = AtomicU64::new(0);
/// TEMP-AB (prezeroed-pages, removed before the PR): the pool in use (Scroll Lock switches).
pub static TEMP_AB: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Size the pool for this machine. Call once, after the frame allocator and its reserve are set.
pub fn init() {
    let total = FRAMES.lock().as_ref().map_or(0, |f| f.total_frames());
    CAPACITY.store(policy::capacity(total), Ordering::Relaxed);
}

/// Frames in the pool now.
pub fn len() -> usize {
    LEN.load(Ordering::Relaxed)
}

/// (taken, zeroed into the pool) since boot.
pub fn stats() -> (u64, u64) {
    (TAKEN.load(Ordering::Relaxed), ZEROED.load(Ordering::Relaxed))
}

fn allocator_free() -> u64 {
    FRAMES.lock().as_ref().map_or(0, |f| f.free_frames())
}

/// Whether the idle loop should zero a frame: the pool has room, the kernel tests are not running,
/// and the allocator has more than the watermark plus a pool free.
pub fn wants() -> bool {
    let capacity = CAPACITY.load(Ordering::Relaxed);
    capacity != 0
        && TEMP_AB.load(Ordering::Relaxed)
        && len() < capacity
        && !crate::ktest::running()
        && policy::may_refill(allocator_free(), len(), capacity, frames::low_watermark())
}

/// Zero one frame from the allocator into the pool, if the rules allow it. Returns whether it did.
/// Called with interrupts on (it takes a few microseconds).
pub fn refill_one() -> bool {
    let capacity = CAPACITY.load(Ordering::Relaxed);
    if capacity == 0 {
        return false;
    }
    let phys = {
        let mut f = FRAMES.lock();
        let Some(alloc) = f.as_mut() else { return false };
        if !policy::may_refill(alloc.free_frames(), len(), capacity, frames::low_watermark()) {
            return false;
        }
        match alloc.allocate(0) {
            Some(p) => p,
            None => return false,
        }
    };
    zero(phys);
    let kept = {
        let mut pool = POOL.lock();
        let kept = pool.len() < capacity && pool.push(phys);
        LEN.store(pool.len(), Ordering::Relaxed);
        kept
    };
    if kept {
        ZEROED.fetch_add(1, Ordering::Relaxed);
    } else {
        give_back(phys);
    }
    kept
}

/// A zeroed frame from the pool for user memory or the page cache, if it has one and the reserve
/// rule lets user memory have it (`floor`: `frames::user_floor`).
pub fn take(floor: u64) -> Option<u64> {
    if len() == 0 || !TEMP_AB.load(Ordering::Relaxed) {
        return None;
    }
    let free = allocator_free();
    let phys = {
        let mut pool = POOL.lock();
        if !policy::user_may_take(free, pool.len(), floor) {
            return None;
        }
        let phys = pool.pop();
        LEN.store(pool.len(), Ordering::Relaxed);
        phys
    }?;
    TAKEN.fetch_add(1, Ordering::Relaxed);
    Some(phys)
}

/// Give every pooled frame back to the allocator (memory is low).
pub fn drain() {
    loop {
        let phys = {
            let mut pool = POOL.lock();
            let p = pool.pop();
            LEN.store(pool.len(), Ordering::Relaxed);
            p
        };
        match phys {
            Some(p) => give_back(p),
            None => break,
        }
    }
}

/// After a user allocation: at the low watermark the pool goes back to the allocator.
pub fn after_user_allocation(allocator_free: u64) {
    if len() > 0 && policy::must_drain(allocator_free, frames::low_watermark()) {
        drain();
    }
}

fn give_back(phys: u64) {
    if let Some(f) = FRAMES.lock().as_mut() {
        f.free(phys, 0).expect("a pooled frame freed twice");
    }
}

fn zero(phys: u64) {
    let virt = crate::arch::phys_to_virt(phys).expect("frame outside the direct map");
    // SAFETY: `phys` was just taken from the allocator for the pool alone: nothing else maps or
    // reaches it; it is RAM reached through the direct map, page-aligned and writable.
    unsafe { core::ptr::write_bytes(virt as *mut u8, 0, PAGE_SIZE as usize) };
}

mod kernel_tests {
    use super::*;
    use crate::kernel_test;

    /// Fill the pool with `n` frames (the idle loop does not while the tests run).
    fn fill(n: usize) -> Result<usize, &'static str> {
        let mut got = 0;
        while len() < n {
            if !refill_one() {
                break;
            }
            got += 1;
        }
        if len() < n.min(CAPACITY.load(Ordering::Relaxed)) {
            return Err("the pool did not fill (is memory short?)");
        }
        Ok(got)
    }

    fn page(phys: u64) -> &'static [u8] {
        let virt = crate::arch::phys_to_virt(phys).expect("direct map");
        // SAFETY: a frame the test holds; read through the direct map.
        unsafe { core::slice::from_raw_parts(virt as *const u8, PAGE_SIZE as usize) }
    }

    kernel_test! {
        /// Frames freed with old contents come back from the pool zeroed: dirty frames go to the
        /// allocator, and the pool only holds frames it zeroed.
        fn pooled_frames_are_zero_even_after_dirty_frees() -> TestResult {
            let _only = crate::smp::exclusive();
            drain();
            // Dirty 64 frames and free them.
            let mut held = alloc::vec::Vec::new();
            for _ in 0..64 {
                let p = frames::allocate_user_frame().ok_or("no frame")?;
                let virt = crate::arch::phys_to_virt(p).ok_or("direct map")?;
                // SAFETY: a frame this test holds.
                unsafe { core::ptr::write_bytes(virt as *mut u8, 0xA5, PAGE_SIZE as usize) };
                held.push(p);
            }
            for p in held {
                give_back(p);
            }
            fill(64)?;
            for _ in 0..64 {
                let p = take(0).ok_or("the filled pool gave nothing")?;
                let dirty = page(p).iter().any(|&b| b != 0);
                give_back(p);
                if dirty {
                    return Err("a frame from the pool was not zero");
                }
            }
            drain();
            Ok(())
        }
    }

    kernel_test! {
        /// What one process wrote never shows in another's fresh pages, with the pool in use.
        fn a_process_never_sees_another_ones_old_pages() -> TestResult {
            use crate::user::Memory;
            use crate::user::memory::{Layout, Placement, Prot};
            let _only = crate::smp::exclusive();
            const P: u64 = PAGE_SIZE;
            const PAGES: u64 = 32;
            {
                let a = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
                let base = a.map_anonymous(Placement::Hint(0), PAGES * P, Prot::RW).map_err(|_| "mmap failed")?;
                for i in 0..PAGES {
                    a.write(base + i * P, &[0x5Au8; 4096]).map_err(|_| "write failed")?;
                }
            }
            // A's frames are free again (dirty); the pool is refilled from the allocator.
            fill(PAGES as usize)?;
            let b = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
            let base = b.map_anonymous(Placement::Hint(0), PAGES * P, Prot::RW).map_err(|_| "mmap failed")?;
            let mut buf = [0xFFu8; 4096];
            for i in 0..PAGES {
                b.read(base + i * P, &mut buf).map_err(|_| "read failed")?;
                if buf.iter().any(|&x| x != 0) {
                    return Err("a fresh page showed another process's data");
                }
            }
            drain();
            Ok(())
        }
    }

    kernel_test! {
        /// The pool fills only above the watermark plus a pool, gives way at the watermark, and
        /// fills again afterwards; its frames go back to the allocator, none lost.
        fn the_pool_drains_under_pressure_and_refills() -> TestResult {
            let _only = crate::smp::exclusive();
            drain();
            let before = allocator_free();
            let n = fill(32)?;
            if allocator_free() + len() as u64 != before || n != len() {
                return Err("filling the pool lost or made frames");
            }
            // At the watermark the pool drains: everything is back.
            after_user_allocation(frames::low_watermark());
            if len() != 0 || allocator_free() != before {
                return Err("the pool did not give its frames back at the watermark");
            }
            // Refilling is refused while the allocator is too low...
            let capacity = CAPACITY.load(Ordering::Relaxed);
            if policy::may_refill(frames::low_watermark() + capacity as u64, 0, capacity, frames::low_watermark()) {
                return Err("the pool would refill at the watermark plus a pool");
            }
            // ... and works again with memory to spare.
            fill(8)?;
            drain();
            if allocator_free() != before {
                return Err("frames went missing over drain and refill");
            }
            Ok(())
        }
    }
}
