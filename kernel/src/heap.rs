//! The kernel heap: [`poly_heap`] as the `#[global_allocator]`, so the
//! kernel can use `alloc` (`Box`, `Vec`, `String`, `BTreeMap`, ...).
//!
//! # Where the pages come from
//!
//! [`FramePages`] takes runs of frames from the buddy allocator in
//! [`FRAMES`] with `allocate_run`, which frees the tail of the block it
//! carves a run from, so a 5-page allocation costs 5 frames, not 8. Buddy
//! blocks are aligned to their size, which gives slabs the alignment they
//! ask for (up to 32 KiB) and large allocations theirs. The kernel reaches
//! the frames through [`arch::phys_to_virt`]. Two limits follow:
//!
//! - One allocation is at most one buddy block: 4 MiB
//!   ([`MAX_ORDER`](poly_mem::MAX_ORDER)), in size and in alignment. Larger
//!   requests fail like any other out-of-memory. Bigger buffers will need
//!   virtually contiguous mappings of scattered frames.
//! - Frames are reached through the direct map. The page source still checks
//!   that `phys_to_virt` reaches each run, so a frame outside the map fails
//!   the allocation cleanly instead of being used.
//!
//! # Locking
//!
//! [`HEAP`]'s lock is taken first, then [`FRAMES`] inside the page source.
//! Code holding [`FRAMES`] must therefore never allocate: it would wait for
//! itself. Neither lock masks interrupts, which is fine while interrupts are
//! off; once an interrupt handler can allocate, [`HEAP`] needs a lock that
//! masks them (`LockedHeap::with_lock`).
//!
//! # Running out of memory
//!
//! The allocator returns null. Fallible APIs such as `Vec::try_reserve`
//! report that to the caller; everything else calls
//! `alloc::alloc::handle_alloc_error`. Stable Rust has no
//! `#[alloc_error_handler]`, and in a `no_std` binary the default handler
//! panics with "memory allocation of N bytes failed" (without unwinding),
//! so an out-of-memory ends in the kernel's panic handler with that message
//! and a backtrace through the allocating code. `cargo xtask test` checks
//! this with `poly.selftest.oom`.

use crate::arch;
use crate::frames::FRAMES;
use crate::ktest::TestResult;
use crate::{kernel_test, kprintln};
use alloc::alloc::{alloc, dealloc};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::ptr::NonNull;
use poly_heap::{CLASS_SIZES, LockedHeap, MAX_SMALL, PAGE_SIZE, PageSource, Stats};
use poly_mem::{BuddyAllocator, MAX_ORDER};

#[global_allocator]
pub static HEAP: LockedHeap<FramePages> = LockedHeap::new(FramePages { offset: None });

/// The heap's page source: runs of frames from [`FRAMES`].
#[derive(Debug)]
pub struct FramePages {
    /// `virt - phys` of the pages handed out. Every mapping `phys_to_virt`
    /// knows (the boot mapping, later the direct map) is a fixed offset, so
    /// this turns a freed pointer back into its frame; `free_pages` checks
    /// the round trip.
    offset: Option<u64>,
}

// SAFETY: a run comes from the frame allocator, which hands each frame out
// once until it is freed, so nothing else uses it; `phys_to_virt` maps all
// of it writable (`reachable` checks both ends of a run inside one linear
// mapping).
unsafe impl PageSource for FramePages {
    fn alloc_pages(&mut self, count: usize, align: usize) -> Option<NonNull<u8>> {
        let frames = count as u64;
        let align_order = (align / PAGE_SIZE).trailing_zeros() as usize;
        let phys = {
            let mut guard = FRAMES.lock();
            allocate_reachable(guard.as_mut()?, frames, align_order)?
        };
        let virt = arch::phys_to_virt(phys).expect("allocate_reachable checked the mapping");
        let offset = virt.wrapping_sub(phys);
        assert_eq!(*self.offset.get_or_insert(offset), offset, "phys_to_virt is not one linear mapping");
        NonNull::new(virt as *mut u8)
    }

    unsafe fn free_pages(&mut self, ptr: NonNull<u8>, count: usize) {
        let virt = ptr.addr().get() as u64;
        let offset = self.offset.expect("heap freed pages before allocating any");
        let phys = virt.wrapping_sub(offset);
        assert_eq!(arch::phys_to_virt(phys), Some(virt), "heap page outside the physical mapping");
        let mut guard = FRAMES.lock();
        let frames = guard.as_mut().expect("heap pages without a frame allocator");
        frames.free_run(phys, count as u64).expect("heap freed frames the frame allocator did not hand out");
    }
}

/// A run of `count` frames the kernel can reach, aligned to
/// `2^align_order` frames.
fn allocate_reachable(frames: &mut BuddyAllocator<'_>, count: u64, align_order: usize) -> Option<u64> {
    let phys = frames.allocate_run(count, align_order, u64::MAX)?;
    if reachable(phys, count * PAGE_SIZE as u64) {
        return Some(phys);
    }
    frames.free_run(phys, count).expect("run just allocated");
    None
}

/// Whether `phys_to_virt` maps all of `[phys, phys + bytes)` in one piece.
fn reachable(phys: u64, bytes: u64) -> bool {
    match (arch::phys_to_virt(phys), arch::phys_to_virt(phys + bytes - 1)) {
        (Some(first), Some(last)) => last - first == bytes - 1,
        _ => false,
    }
}

/// A snapshot of the heap's bookkeeping.
pub fn stats() -> Stats {
    HEAP.lock().stats()
}

/// Give the heap's cached empty slabs back to the frame allocator. Returns
/// the number of pages released.
pub fn trim() -> usize {
    HEAP.lock().trim()
}

/// One line of heap numbers for the boot log.
pub fn log_stats() {
    let s = stats();
    kprintln!(
        "heap: {} bytes in use, {} pages held ({} in {} large runs), {} failed allocations",
        s.bytes_in_use,
        s.pages_held,
        s.large_pages,
        s.large_allocations,
        s.failed_allocations
    );
}

/// `alloc(layout)` for the tests, through `black_box`: the optimiser may
/// drop an allocation whose pointer it never sees escape and assume it
/// succeeded, which would hide the failures the tests look for.
fn heap_alloc(layout: Layout) -> *mut u8 {
    assert!(layout.size() > 0, "GlobalAlloc does not take zero-size layouts");
    // SAFETY: the layout has a non-zero size.
    core::hint::black_box(unsafe { alloc(layout) })
}

/// Deterministic xorshift64, so the tests need no entropy.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

fn free_frames() -> u64 {
    FRAMES.lock().as_ref().map_or(0, |f| f.free_frames())
}

/// Byte `i` of a block filled for `seed`.
fn fill_byte(seed: usize, i: usize) -> u8 {
    (seed.wrapping_mul(31) ^ i.wrapping_mul(7)) as u8
}

/// # Safety
///
/// `p` is valid for writes of `len` bytes.
unsafe fn fill(p: *mut u8, len: usize, seed: usize) {
    for i in 0..len {
        // SAFETY: `i < len`, forwarded from the caller.
        unsafe { p.add(i).write(fill_byte(seed, i)) };
    }
}

/// # Safety
///
/// `p` is valid for reads of `len` bytes.
unsafe fn holds_fill(p: *const u8, len: usize, seed: usize) -> bool {
    // SAFETY: `i < len`, forwarded from the caller.
    (0..len).all(|i| unsafe { p.add(i).read() } == fill_byte(seed, i))
}

kernel_test! {
    fn box_and_string() -> TestResult {
        let b = Box::new(0x1234_5678_9abc_def0u64);
        if *b != 0x1234_5678_9abc_def0 || !(&raw const *b).addr().is_multiple_of(align_of::<u64>()) {
            return Err("Box lost its value or alignment");
        }
        let s = format!("{}-{:#x}-{:>6}", 42, *b, "poly");
        if s != "42-0x123456789abcdef0-  poly" {
            return Err("format! produced the wrong string");
        }
        let mut grown = String::new();
        for i in 0..500 {
            grown.push_str(&format!("{i},"));
        }
        if !grown.starts_with("0,1,2,") || !grown.ends_with(",498,499,") || grown.len() != 1890 {
            return Err("String did not grow correctly");
        }
        Ok(())
    }
}

kernel_test! {
    fn vec_grows_to_mebibytes() -> TestResult {
        // 1 Mi u32 = 4 MiB, the largest allocation the heap can make; the
        // last few doublings are large allocations moved by realloc.
        let mut v = Vec::new();
        for i in 0..(1u32 << 20) {
            v.push(i.wrapping_mul(2_654_435_761));
        }
        if v.capacity() * size_of::<u32>() != 4 << 20 {
            return Err("Vec did not double up to 4 MiB");
        }
        if v.iter().enumerate().any(|(i, &x)| x != (i as u32).wrapping_mul(2_654_435_761)) {
            return Err("Vec contents changed while it grew");
        }
        drop(v);

        // Several large allocations alive at once must not overlap.
        let before = stats().large_allocations;
        let bufs: Vec<Vec<u8>> = (0..4).map(|i| vec![i as u8 + 1; (1 << 20) + i * 4096 + 1]).collect();
        if stats().large_allocations != before + 4 {
            return Err("1 MiB buffers were not large allocations");
        }
        for (i, b) in bufs.iter().enumerate() {
            if b.iter().any(|&x| x != i as u8 + 1) {
                return Err("large buffers overlap");
            }
        }
        Ok(())
    }
}

kernel_test! {
    fn btree_map_with_thousands_of_entries() -> TestResult {
        let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
        let mut keys: Vec<u32> = (0..5000).collect();
        rng.shuffle(&mut keys);
        let mut map = BTreeMap::new();
        for &k in &keys {
            map.insert(k, format!("value {k}"));
        }
        if map.len() != 5000 || !map.iter().enumerate().all(|(i, (&k, v))| k == i as u32 && *v == format!("value {k}")) {
            return Err("BTreeMap lost or misordered entries");
        }
        for &k in keys.iter().filter(|&&k| k % 3 != 0) {
            map.remove(&k);
        }
        if map.len() != 1667 || map.keys().any(|k| k % 3 != 0) {
            return Err("BTreeMap removals went wrong");
        }
        Ok(())
    }
}

kernel_test! {
    fn small_objects_of_every_class_freed_in_random_order() -> TestResult {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let in_use = |s: &Stats| s.classes.iter().map(|c| c.objects_in_use).sum::<usize>();
        let start = in_use(&stats());

        // Every class size exactly, enough to fill several slabs of the big
        // classes, then random sizes and alignments that land in between.
        let mut layouts = Vec::new();
        for &size in &CLASS_SIZES {
            layouts.extend((0..64).map(|_| Layout::from_size_align(size, 1 << size.trailing_zeros().min(11))));
        }
        for _ in 0..3000 {
            let size = 1 + rng.below(MAX_SMALL);
            let align = 1 << rng.below(7);
            layouts.push(Layout::from_size_align(size, align));
        }

        let mut live: Vec<(*mut u8, Layout)> = Vec::with_capacity(layouts.len());
        for (seed, layout) in layouts.into_iter().enumerate() {
            let layout = layout.map_err(|_| "bad test layout")?;
            let p = heap_alloc(layout);
            if p.is_null() {
                return Err("small allocation failed");
            }
            if !p.addr().is_multiple_of(layout.align()) {
                return Err("small allocation misaligned");
            }
            // SAFETY: `p` is a fresh allocation of `layout.size()` bytes.
            unsafe { fill(p, layout.size(), seed) };
            live.push((p, layout));
        }
        if in_use(&stats()) != start + live.len() {
            return Err("objects in use do not match the allocations");
        }

        let mut order: Vec<usize> = (0..live.len()).collect();
        rng.shuffle(&mut order);
        let mut intact = true;
        for i in order {
            let (p, layout) = live[i];
            // SAFETY: `p` is a live allocation of `layout`, freed exactly
            // once here.
            unsafe {
                intact &= holds_fill(p, layout.size(), i);
                dealloc(p, layout);
            }
        }
        if !intact {
            return Err("an object was overwritten while the others lived");
        }
        if in_use(&stats()) != start {
            return Err("objects still in use after freeing them all");
        }
        Ok(())
    }
}

/// A page-aligned type, so `Box::new` asks for 4 KiB alignment.
#[repr(C, align(4096))]
struct Page([u8; 4096]);

kernel_test! {
    fn alignment_up_to_the_largest_block() -> TestResult {
        let page = Box::new(Page([7; 4096]));
        if !(&raw const *page).addr().is_multiple_of(4096) || page.0.iter().any(|&b| b != 7) {
            return Err("4 KiB aligned Box misplaced");
        }
        // Small sizes with large alignments, on both sides of the slab limit.
        for shift in [4, 9, 11, 12, 15, 16, 21, 22] {
            let layout = Layout::from_size_align(24, 1 << shift).map_err(|_| "bad test layout")?;
            let p = heap_alloc(layout);
            if p.is_null() || !p.addr().is_multiple_of(layout.align()) {
                return Err("aligned allocation failed or misaligned");
            }
            // SAFETY: `p` was just allocated with `layout`.
            unsafe { dealloc(p, layout) };
        }
        Ok(())
    }
}

kernel_test! {
    fn requests_beyond_one_block_fail_cleanly() -> TestResult {
        let block = PAGE_SIZE << MAX_ORDER;
        let failed = stats().failed_allocations;
        let largest = Layout::from_size_align(block, PAGE_SIZE).map_err(|_| "bad test layout")?;
        let p = heap_alloc(largest);
        if p.is_null() {
            return Err("a 4 MiB allocation failed");
        }
        // SAFETY: `p` was just allocated with `largest`.
        unsafe { dealloc(p, largest) };

        for (size, align) in [(block + 1, 8), (1, block * 2)] {
            let layout = Layout::from_size_align(size, align).map_err(|_| "bad test layout")?;
            let p = heap_alloc(layout);
            if !p.is_null() {
                // SAFETY: `p` was just allocated with `layout`.
                unsafe { dealloc(p, layout) };
                return Err("allocation larger than a buddy block succeeded");
            }
        }
        if stats().failed_allocations != failed + 2 {
            return Err("failed allocations not counted");
        }
        // Fallible APIs see the failure instead of panicking.
        let mut v: Vec<u8> = Vec::new();
        let reserved = v.try_reserve_exact(block * 4);
        core::hint::black_box(&mut v);
        if reserved.is_ok() {
            return Err("try_reserve beyond a block succeeded");
        }
        Ok(())
    }
}

kernel_test! {
    fn running_out_of_frames_fails_cleanly() -> TestResult {
        // Room for every block, reserved up front: pushing must not allocate
        // while the frames are gone (or while FRAMES is locked).
        let mut held: Vec<(u64, usize)> = Vec::with_capacity(8192);
        let start = free_frames();
        let outcome = exhaust_frames(&mut held);
        {
            let mut guard = FRAMES.lock();
            let frames = guard.as_mut().ok_or("frame allocator not initialised")?;
            for &(f, order) in &held {
                frames.free(f, order).map_err(|_| "could not give back a drained block")?;
            }
        }
        outcome?;
        if free_frames() != start {
            return Err("frames lost while exhausted");
        }
        Ok(())
    }
}

/// Take every free frame into `held` and check that the heap fails
/// cleanly, also when the only free memory is out of the kernel's reach
/// (with 2 GiB of RAM, until the direct map lands). Then free one reachable
/// 4 MiB block and check that the heap uses it.
fn exhaust_frames(held: &mut Vec<(u64, usize)>) -> TestResult {
    let two_pages = Layout::from_size_align(2 * PAGE_SIZE, PAGE_SIZE).map_err(|_| "bad test layout")?;
    let block = BuddyAllocator::block_size(MAX_ORDER);
    {
        let mut guard = FRAMES.lock();
        let frames = guard.as_mut().ok_or("frame allocator not initialised")?;
        for order in (0..=MAX_ORDER).rev() {
            while held.len() < held.capacity() {
                let Some(f) = frames.allocate(order) else { break };
                held.push((f, order));
            }
        }
        if frames.free_frames() != 0 {
            return Err("could not drain the frame allocator");
        }
    }

    let failed = stats().failed_allocations;
    if heap_round_trip(two_pages).is_some() {
        return Err("heap allocated pages with no free frames");
    }
    if stats().failed_allocations != failed + 1 {
        return Err("failed allocation not counted");
    }

    let unreachable = held.iter().position(|&(f, order)| order == MAX_ORDER && !reachable(f, block));
    if let Some(i) = unreachable {
        give_back(held, i)?;
        if heap_round_trip(two_pages).is_some() {
            return Err("heap allocated memory the kernel cannot reach");
        }
    }

    let reachable = held.iter().position(|&(f, order)| order == MAX_ORDER && reachable(f, block));
    give_back(held, reachable.ok_or("no reachable 4 MiB block")?)?;
    match heap_round_trip(two_pages) {
        Some(true) => Ok(()),
        Some(false) => Err("pages from a reachable block did not hold data"),
        None => Err("heap failed with a reachable block free"),
    }
}

/// Free `held[i]` to the frame allocator and forget it.
fn give_back(held: &mut Vec<(u64, usize)>, i: usize) -> TestResult {
    let (f, order) = held.swap_remove(i);
    let mut guard = FRAMES.lock();
    let frames = guard.as_mut().ok_or("frame allocator not initialised")?;
    frames.free(f, order).map_err(|_| "could not free a drained block")
}

/// Allocate `layout` from the heap, check the memory holds a pattern and
/// free it. `None` if the allocation failed.
fn heap_round_trip(layout: Layout) -> Option<bool> {
    let p = heap_alloc(layout);
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` is a fresh allocation of `layout`, freed once.
    unsafe {
        fill(p, layout.size(), 3);
        let intact = holds_fill(p, layout.size(), 3);
        dealloc(p, layout);
        Some(intact)
    }
}

kernel_test! {
    fn trim_gives_every_frame_back() -> TestResult {
        trim();
        let pages = stats().pages_held;
        let start = free_frames();
        {
            let mut boxes: Vec<Box<[u8]>> = Vec::new();
            for &size in &CLASS_SIZES {
                boxes.extend((0..40).map(|i| vec![i as u8; size].into_boxed_slice()));
            }
            boxes.push(vec![1; 5 * PAGE_SIZE + 3].into_boxed_slice());
            if stats().pages_held <= pages || free_frames() >= start {
                return Err("the heap took no frames");
            }
            if free_frames() + (stats().pages_held - pages) as u64 != start {
                return Err("frames taken do not match the pages the heap holds");
            }
        }
        if trim() == 0 {
            return Err("no empty slab was cached");
        }
        if stats().pages_held != pages || free_frames() != start {
            return Err("trim did not give every frame back");
        }
        Ok(())
    }
}

kernel_test! {
    /// Negative test (`poly.selftest.oom`): passes unless the boot asks for
    /// an allocation that cannot succeed, so the runner can check how an
    /// out-of-memory is reported.
    fn out_of_memory_on_request(env: &TestEnv) -> TestResult {
        if env.boot.has_flag("poly.selftest.oom") {
            allocate_too_much();
        }
        Ok(())
    }
}

/// Bytes `poly.selftest.oom` asks for: more than one buddy block.
const OOM_BYTES: usize = 64 << 20;

/// Kept out of line so the backtrace check has a frame to look for.
#[inline(never)]
fn allocate_too_much() {
    // black_box keeps the optimiser from proving the buffer unused and
    // dropping the allocation.
    let v: Vec<u8> = core::hint::black_box(Vec::with_capacity(OOM_BYTES));
    kprintln!("heap: unexpectedly allocated {} bytes", v.capacity());
}
