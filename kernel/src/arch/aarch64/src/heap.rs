//! The heap: `poly_heap` over the frames of `poly_mem`'s buddy allocator,
//! the same two crates the x86 kernel uses (`kernel/src/heap.rs`,
//! `kernel/src/frames.rs`). RAM is identity mapped, so a frame's physical
//! address is its virtual address.

use core::cell::UnsafeCell;
use core::ptr::NonNull;
use poly_heap::{LockedHeap, PAGE_SIZE, PageSource, RawLock, RawSpinLock};
use poly_mem::{BuddyAllocator, MemoryMap};

#[global_allocator]
static HEAP: LockedHeap<FramePages> = LockedHeap::new(FramePages);

/// Words of buddy allocator metadata: enough for the RAM of QEMU's `virt`
/// up to a few gigabytes (`init` checks).
const METADATA_WORDS: usize = 1 << 16;

struct Frames {
    lock: RawSpinLock,
    allocator: UnsafeCell<Option<BuddyAllocator<'static>>>,
    metadata: UnsafeCell<[u64; METADATA_WORDS]>,
}

// SAFETY: `allocator` is only reached through `with`, which holds `lock`;
// `metadata` is lent out once, to the allocator `init` makes.
unsafe impl Sync for Frames {}

static FRAMES: Frames = Frames {
    lock: RawSpinLock::new(),
    allocator: UnsafeCell::new(None),
    metadata: UnsafeCell::new([0; METADATA_WORDS]),
};

/// Run `f` on the frame allocator, if `init` made it. The heap's lock is
/// taken before this one, so `f` must never allocate from the heap.
fn with<R>(f: impl FnOnce(&mut BuddyAllocator<'static>) -> R) -> Option<R> {
    FRAMES.lock.lock();
    // SAFETY: the lock is held, so this is the only reference.
    let result = unsafe { (*FRAMES.allocator.get()).as_mut().map(f) };
    // SAFETY: locked above, by this thread.
    unsafe { FRAMES.lock.unlock() };
    result
}

struct FramePages;

// SAFETY: a run comes from the frame allocator, which hands each frame out
// once until it is freed; the frames are RAM, identity mapped writable, and
// the memory map `init` gave the allocator excludes the stub's image and the
// devicetree.
unsafe impl PageSource for FramePages {
    fn alloc_pages(&mut self, count: usize, align: usize) -> Option<NonNull<u8>> {
        let align_order = (align / PAGE_SIZE).trailing_zeros() as usize;
        let phys = with(|f| f.allocate_run(count as u64, align_order, u64::MAX))??;
        NonNull::new(phys as usize as *mut u8)
    }

    unsafe fn free_pages(&mut self, ptr: NonNull<u8>, count: usize) {
        let freed = with(|f| f.free_run(ptr.addr().get() as u64, count as u64));
        debug_assert!(matches!(freed, Some(Ok(()))), "the heap freed a run the frame allocator did not hand out");
    }
}

/// Why [`init`] failed.
#[derive(Debug)]
pub enum InitError {
    /// More than once.
    Twice,
    /// The memory map needs more metadata than the stub reserves.
    #[allow(dead_code)] // shown by the panic message's `Debug`
    MetadataTooSmall { words: usize },
    /// `BuddyAllocator::new` refused.
    Allocator,
}

/// Start the frame allocator over `map` (usable RAM minus the stub's image
/// and the devicetree) and so the heap.
pub fn init(map: &MemoryMap) -> Result<(), InitError> {
    let words = BuddyAllocator::metadata_words(map);
    if words > METADATA_WORDS {
        return Err(InitError::MetadataTooSmall { words });
    }
    FRAMES.lock.lock();
    // SAFETY: the lock is held.
    let slot = unsafe { &mut *FRAMES.allocator.get() };
    let result = if slot.is_some() {
        Err(InitError::Twice)
    } else {
        // SAFETY: `metadata` is a static used nowhere else, and the slot
        // being empty (checked under the lock) means no slice was made
        // from it before.
        let metadata: &'static mut [u64] =
            unsafe { core::slice::from_raw_parts_mut(FRAMES.metadata.get().cast::<u64>(), METADATA_WORDS) };
        BuddyAllocator::new(map, metadata).map(|a| *slot = Some(a)).map_err(|_| InitError::Allocator)
    };
    // SAFETY: locked above.
    unsafe { FRAMES.lock.unlock() };
    result
}

/// Free frames in the allocator (0 before `init`).
pub fn free_frames() -> u64 {
    with(|f| f.free_frames()).unwrap_or(0)
}

/// Take one frame, or give one back (the self-test of the allocator itself).
pub fn allocate_frame() -> Option<u64> {
    with(|f| f.allocate(0))?
}

/// Give back a frame from [`allocate_frame`].
pub fn free_frame(addr: u64) -> bool {
    matches!(with(|f| f.free(addr, 0)), Some(Ok(())))
}

/// Bytes the heap's callers have asked for and not freed.
pub fn bytes_in_use() -> usize {
    HEAP.lock().stats().bytes_in_use
}
