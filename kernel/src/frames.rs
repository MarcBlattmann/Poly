//! The kernel's physical frame allocator: a [`BuddyAllocator`] over all
//! usable RAM, behind a spinlock.
//!
//! The allocator keeps its metadata outside the memory it manages, so the
//! only memory it needs to touch is that metadata buffer. It is carved from
//! usable RAM below [`arch::BOOT_MAPPED_PHYS`], the only RAM the kernel can
//! reach before it has its own page tables. Frames above that limit are
//! managed too; they are just not mapped yet.
//!
//! The kernel heap takes its pages from here while holding its own lock, so
//! nothing may allocate from the heap while holding [`FRAMES`].

use crate::arch;
use crate::sync::SpinLock;
use poly_mem::{BuddyAllocator, MemoryMap, PAGE_SIZE, Region, RegionKind, align_up};

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
    let phys = memory.find_usable(bytes, arch::BOOT_MAPPED_PHYS).expect("no low memory for frame metadata");

    let mut map = memory.clone();
    map.push(Region::new(phys, phys + bytes, RegionKind::Kernel)).expect("memory map full");
    let map = map.normalized().expect("memory map full");

    let virt = arch::phys_to_virt(phys).expect("frame metadata below the boot mapping limit");
    // SAFETY: [phys, phys + bytes) is usable RAM that ends below
    // BOOT_MAPPED_PHYS, so the boot page tables map it writable at `virt`.
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
