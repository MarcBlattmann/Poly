//! Kernel virtual memory, the architecture-neutral side.
//!
//! The architecture builds the kernel page tables (on x86-64:
//! `arch/x86_64/paging.rs`, which documents the layout): a direct map of all
//! physical memory, through which [`arch::phys_to_virt`] reaches any frame,
//! the kernel image mapped W^X, guard pages below the kernel stacks, an
//! area for the kernel thread stacks ([`map_thread_stack`]), and
//! uncached mappings of device memory for drivers ([`map_mmio`]), and
//! write-combining ones for framebuffers ([`map_mmio_wc`]).

use crate::arch;
use poly_mem::MemoryMap;

pub use crate::arch::paging::{
    Caching, THREAD_STACKS_BASE, THREAD_STACKS_SIZE, caching, map_mmio, map_mmio_wc, map_thread_stack,
    unmap_thread_stack,
};

/// Switch to the kernel page tables. Returns `memory` with the frames the
/// page tables took marked as kernel memory; build the frame allocator from
/// that map.
///
/// # Safety
/// Call once on the boot CPU, before the frame allocator is set up and before
/// anything keeps a pointer from [`arch::phys_to_virt`] (the boot mapping it
/// points into goes away).
pub unsafe fn init(memory: &MemoryMap) -> MemoryMap {
    // SAFETY: forwarded from the caller.
    unsafe { arch::paging::init(memory) }
}

/// Fill `[start, start + len)` with [`poly_mem::STACK_PAINT`].
///
/// # Safety
/// The range is mapped, writable, 8-byte aligned and not in use.
pub unsafe fn paint_stack(start: u64, len: u64) {
    for i in 0..len / 8 {
        // SAFETY: inside the range the caller vouches for.
        unsafe { ((start + 8 * i) as *mut u64).write_volatile(poly_mem::STACK_PAINT) };
    }
}

/// The bytes of the painted stack `[start, start + len)` ever used: from
/// its top down to the lowest word that lost its paint.
///
/// # Safety
/// The range is mapped and 8-byte aligned. It may be in use (by another
/// CPU, even): the words are read one at a time, volatile.
pub unsafe fn stack_used(start: u64, len: u64) -> u64 {
    // SAFETY: inside the range the caller vouches for.
    let untouched = poly_mem::untouched_words((len / 8) as usize, |i| unsafe {
        ((start + 8 * i as u64) as *const u64).read_volatile()
    });
    len - 8 * untouched as u64
}

crate::kernel_test! {
    fn direct_map_covers_all_ram(env: &TestEnv) -> TestResult {
        use crate::frames::FRAMES;
        use crate::selftest::{Pass, pattern};
        use poly_mem::{BuddyAllocator, MAX_ORDER, PAGE_SIZE, RegionKind};

        // Every region of the memory map is reachable, up to the highest one.
        for r in env.memory.regions().iter().filter(|r| r.kind != RegionKind::Bad) {
            let start = r.start & !(PAGE_SIZE - 1);
            if arch::phys_range_to_virt(start, r.end - start).is_none() {
                crate::kprintln!("mm: {:#x}-{:#x} {:?} is not direct-mapped", r.start, r.end, r.kind);
                return Err("memory map region outside the direct map");
            }
        }

        // Write and read back a pattern in RAM above the old 1 GiB boot
        // mapping, when there is any. The buddy allocator hands out the lowest
        // free block first, and at most 256 blocks of 4 MiB fit below 1 GiB.
        const GIB: u64 = 1 << 30;
        let high_ram = env.memory.regions().iter().any(|r| r.kind == RegionKind::Usable && r.end > GIB);
        let mut guard = FRAMES.lock();
        let alloc = guard.as_mut().ok_or("frame allocator not initialised")?;
        let block = BuddyAllocator::block_size(MAX_ORDER);
        let mut held = [0u64; 257];
        let mut n = 0;
        let target = loop {
            let Some(b) = alloc.allocate(MAX_ORDER) else { break None };
            held[n] = b;
            n += 1;
            if !high_ram || b >= GIB || n == held.len() {
                break Some(b);
            }
        };
        let result = (|| {
            let frame = target.ok_or("no free 4 MiB block")? + block - PAGE_SIZE;
            if high_ram && frame < GIB {
                return Err("no free block above 1 GiB");
            }
            let virt = arch::phys_to_virt(frame).ok_or("allocated frame outside the direct map")?;
            pattern(virt as *mut u64, frame, Pass::Write)?;
            pattern(virt as *mut u64, frame, Pass::Check)?;
            crate::kprintln!("mm: pattern written and read back at {frame:#x} through {virt:#x}");
            Ok(())
        })();
        for &b in &held[..n] {
            alloc.free(b, MAX_ORDER).map_err(|_| "free refused a live block")?;
        }
        result
    }
}
