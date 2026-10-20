//! The kernel's physical frame allocator: a [`BuddyAllocator`] over all
//! usable RAM, behind a spinlock.
//!
//! The allocator keeps its metadata outside the memory it manages, so the
//! only memory it needs to touch is that metadata buffer. It is carved from
//! usable RAM and reached through the direct map, so [`init`] runs after the
//! kernel page tables are loaded (`mm::init`).
//!
//! The kernel heap takes its pages from here while holding its own lock, so
//! nothing may allocate from the heap while holding [`FRAMES`].

use crate::arch;
use crate::sync::SpinLock;
use poly_mem::{BuddyAllocator, FreeError, MAX_ORDER, MemoryMap, PAGE_SIZE, Region, RegionKind, align_up};

pub static FRAMES: SpinLock<Option<BuddyAllocator<'static>>> = SpinLock::new(None);

/// Where the allocator's metadata lives, for the boot log.
#[derive(Clone, Copy, Debug)]
pub struct Metadata {
    pub phys: u64,
    pub bytes: u64,
}

/// Hand every usable frame of the normalized map `memory` to [`FRAMES`],
/// except the pages that hold the allocator's own metadata. Returns the map
/// the allocator was built from: `memory` with the metadata marked
/// [`RegionKind::Kernel`].
pub fn init(memory: &MemoryMap) -> (MemoryMap, Metadata) {
    let mut frames = FRAMES.lock();
    assert!(frames.is_none(), "frame allocator initialised twice");

    // Taking pages out of the map can only shrink the span it covers, so
    // this is enough for the final map too.
    let words = BuddyAllocator::metadata_words(memory);
    let bytes = align_up((words * size_of::<u64>()) as u64, PAGE_SIZE);
    let phys = memory.find_usable(bytes, u64::MAX).expect("no memory for frame metadata");

    let mut map = memory.clone();
    map.push(Region::new(phys, phys + bytes, RegionKind::Kernel)).expect("memory map full");
    let map = map.normalized().expect("memory map full");

    let virt = arch::phys_range_to_virt(phys, bytes).expect("frame metadata outside the direct map");
    // SAFETY: [phys, phys + bytes) is usable RAM, which the direct map maps
    // writable at `virt` (checked above) for the kernel's lifetime.
    // Nothing else uses it: it was free RAM, it is marked Kernel in the map
    // the allocator manages, and the assert above guarantees this runs only
    // once, so this is the only reference ever made to it. It is never freed,
    // so it lives for 'static. It is page-aligned, which satisfies u64, and
    // it is zeroed before the slice is formed.
    let meta = unsafe {
        let ptr = virt as *mut u64;
        core::ptr::write_bytes(ptr, 0, words);
        core::slice::from_raw_parts_mut(ptr, words)
    };
    *frames = Some(BuddyAllocator::new(&map, meta).expect("metadata sized by metadata_words"));
    (map, Metadata { phys, bytes })
}

/// Frames in the largest buddy block (4 MiB).
const BLOCK_FRAMES: u64 = 1 << MAX_ORDER;

/// Most blocks [`allocate_contiguous`] holds while it looks for a run.
const SEARCH_BLOCKS: usize = 64;

/// `frames` physically contiguous frames (so contiguous in the direct map
/// too), not zeroed, possibly more than one buddy block holds: for large
/// buffers such as a display's back buffer. Up to 4 MiB this is one run
/// ([`BuddyAllocator::allocate_run`]). Beyond, it takes 4 MiB blocks, lowest
/// first, until enough of them lie back to back, and gives the others back;
/// the last block is taken whole, so up to 4 MiB less one frame is wasted.
/// `None` if no such run turns up among the first 64 blocks. Give the frames
/// back with [`free_contiguous`] and the same `frames`.
pub fn allocate_contiguous(frames: u64) -> Option<u64> {
    let mut guard = FRAMES.lock();
    let alloc = guard.as_mut()?;
    if frames <= BLOCK_FRAMES {
        return alloc.allocate_run(frames, 0, u64::MAX);
    }
    let blocks = frames.div_ceil(BLOCK_FRAMES);
    let block_bytes = BLOCK_FRAMES * PAGE_SIZE;
    // No heap here: the heap takes FRAMES itself.
    let mut held = [0u64; SEARCH_BLOCKS];
    let (mut n, mut start, mut len) = (0, 0, 0);
    let found = loop {
        let Some(block) = held.get_mut(n).and_then(|slot| {
            *slot = alloc.allocate(MAX_ORDER)?;
            Some(*slot)
        }) else {
            break None;
        };
        n += 1;
        if len > 0 && block == start + len * block_bytes {
            len += 1;
        } else {
            (start, len) = (block, 1);
        }
        if len == blocks {
            break Some(start);
        }
    };
    let in_run = |b: u64| found.is_some_and(|s| (s..s + blocks * block_bytes).contains(&b));
    for &block in held[..n].iter().filter(|&&b| !in_run(b)) {
        let freed = alloc.free(block, MAX_ORDER);
        debug_assert!(freed.is_ok(), "block {block:#x} was just allocated");
    }
    found
}

/// Give back frames from [`allocate_contiguous`] with the same `frames`.
pub fn free_contiguous(phys: u64, frames: u64) -> Result<(), FreeError> {
    let mut guard = FRAMES.lock();
    let alloc = guard.as_mut().ok_or(FreeError::OutOfRange)?;
    if frames <= BLOCK_FRAMES {
        return alloc.free_run(phys, frames);
    }
    for i in 0..frames.div_ceil(BLOCK_FRAMES) {
        alloc.free(phys + i * BLOCK_FRAMES * PAGE_SIZE, MAX_ORDER)?;
    }
    Ok(())
}

crate::kernel_test! {
    fn contiguous_runs_beyond_one_block() -> TestResult {
        let free = || FRAMES.lock().as_ref().map_or(0, |f| f.free_frames());
        let before = free();
        // 2.5 blocks: three whole blocks, back to back.
        let frames = BLOCK_FRAMES * 2 + BLOCK_FRAMES / 2;
        let phys = allocate_contiguous(frames).ok_or("no 10 MiB run of frames")?;
        let bytes = frames * PAGE_SIZE;
        let result = (|| {
            if before - free() != 3 * BLOCK_FRAMES || !phys.is_multiple_of(BLOCK_FRAMES * PAGE_SIZE) {
                return Err("the run is not three whole, aligned blocks");
            }
            let virt = arch::phys_range_to_virt(phys, bytes).ok_or("the run is not in the direct map")?;
            // Touch the first and last frame of every block.
            for offset in (0..bytes).step_by((BLOCK_FRAMES * PAGE_SIZE) as usize) {
                for frame in [offset, (offset + (BLOCK_FRAMES - 1) * PAGE_SIZE).min(bytes - PAGE_SIZE)] {
                    let p = (virt + frame) as *mut u64;
                    use crate::selftest::{Pass, pattern};
                    pattern(p, phys + frame, Pass::Write)?;
                    pattern(p, phys + frame, Pass::Check)?;
                }
            }
            Ok(())
        })();
        free_contiguous(phys, frames).map_err(|_| "free_contiguous refused the run")?;
        result?;
        if free() != before {
            return Err("frames lost");
        }
        // Small runs are exact.
        let phys = allocate_contiguous(3).ok_or("no 3 frames")?;
        let used = before - free();
        free_contiguous(phys, 3).map_err(|_| "free_contiguous refused a small run")?;
        if used != 3 || free() != before { Err("a 3-frame run was not exactly 3 frames") } else { Ok(()) }
    }
}
