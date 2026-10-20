//! Memory shared with devices (DMA buffers).
//!
//! Devices see physical addresses: no IOMMU is set up, so the bus address
//! of a frame is its physical address. A [`DmaBuffer`] is a block of whole
//! frames from [`FRAMES`], reached by the CPU through the direct map. That
//! mapping is write-back cached, which is fine on x86-64: device accesses to
//! RAM snoop the CPU caches, so nothing needs flushing. Ordering between the
//! CPU and the device is the driver's job (the virtqueue code in
//! `poly-virtio` has the fences).
//!
//! # Buffers the caller owns
//!
//! The block layer hands drivers the caller's memory, so drivers need its
//! bus address: [`bus_address`] (and [`KernelDma`], the same for
//! `poly-virtio-blk`). A device needs the whole buffer at consecutive bus
//! addresses, and the kernel only knows that for the direct map, which maps
//! physical memory linearly: a range that lies entirely inside it is
//! physically contiguous, page boundaries or not. Everything else is
//! refused, and the request fails with `BadBuffer` instead of the device
//! scribbling over the wrong memory. What that means for the memory the
//! kernel has:
//!
//! - **Heap** (`Box`, `Vec`): accepted. The heap's pages are runs of frames
//!   from one buddy block each (`allocate_run`), reached through the direct
//!   map (`heap.rs`); a large allocation is one run, and a small object lies
//!   in one slot of one slab, which is one run too. So no heap allocation
//!   crosses from one run into another, and every byte of one is
//!   physically next to the one before.
//! - **[`DmaBuffer`]s** and **thread stacks**: frames reached through the
//!   direct map, accepted.
//! - **The kernel image** (statics, the boot stack, which the boot code and
//!   the in-kernel tests run on), **MMIO mappings**, holes and user
//!   addresses: refused. The image is linear too, but drivers should not
//!   DMA into it, and refusing keeps the rule simple: direct map or nothing.
//!
//! Once an IOMMU is in use, this is where bus addresses stop being physical
//! addresses.

use crate::arch;
use crate::frames::FRAMES;
use core::ptr::{self, NonNull};
use poly_block::IoBuf;
use poly_mem::{BuddyAllocator, MAX_ORDER, PAGE_SIZE};

/// The bus address of the `len` bytes at CPU address `virt`, if a device
/// can reach all of them at consecutive bus addresses: they must lie in the
/// direct map (see the module documentation).
pub fn bus_address(virt: u64, len: usize) -> Option<u64> {
    arch::virt_range_to_phys(virt, len as u64)
}

/// The kernel's address translation for `poly-virtio-blk`: [`bus_address`].
#[derive(Clone, Copy, Debug, Default)]
pub struct KernelDma;

impl poly_virtio_blk::Dma for KernelDma {
    fn bus_address(&self, buf: IoBuf) -> Option<u64> {
        bus_address(buf.ptr.as_ptr() as u64, buf.len)
    }
}

/// A zeroed, physically contiguous buffer for a device, freed on drop.
///
/// The owner must make sure the device no longer uses the buffer before
/// dropping it (reset the device or take the buffer back from it first);
/// if it cannot, it leaks the buffer with `core::mem::forget`.
#[derive(Debug)]
pub struct DmaBuffer {
    phys: u64,
    virt: NonNull<u8>,
    order: usize,
}

// SAFETY: the buffer is plain memory owned by this value; moving it to
// another CPU moves the only CPU-side owner with it.
unsafe impl Send for DmaBuffer {}

impl DmaBuffer {
    /// At least `bytes` bytes (rounded up to a power-of-two number of
    /// frames, at most 4 MiB), aligned to their size. `None` when out of
    /// frames.
    pub fn new(bytes: usize) -> Option<DmaBuffer> {
        let frames = bytes.max(1).div_ceil(PAGE_SIZE as usize).next_power_of_two();
        let order = frames.trailing_zeros() as usize;
        if order > MAX_ORDER {
            return None;
        }
        let phys = FRAMES.lock().as_mut()?.allocate(order)?;
        let len = BuddyAllocator::block_size(order);
        let Some(virt) = arch::phys_range_to_virt(phys, len).and_then(|v| NonNull::new(v as *mut u8)) else {
            // Every usable frame is direct-mapped, so this does not happen;
            // give the block back rather than use memory we cannot reach.
            let _ = FRAMES.lock().as_mut().map(|f| f.free(phys, order));
            return None;
        };
        // SAFETY: the block was just allocated, so nothing else uses it, and
        // the direct map makes all `len` bytes writable at `virt`.
        unsafe { ptr::write_bytes(virt.as_ptr(), 0, len as usize) };
        Some(DmaBuffer { phys, virt, order })
    }

    /// The bus address the device uses.
    pub const fn phys(&self) -> u64 {
        self.phys
    }

    pub const fn ptr(&self) -> NonNull<u8> {
        self.virt
    }

    pub fn len(&self) -> usize {
        BuddyAllocator::block_size(self.order) as usize
    }

    /// Copy `src` into the buffer at `offset`. Volatile, like every access
    /// to memory a device shares. Panics if it does not fit.
    pub fn write(&self, offset: usize, src: &[u8]) {
        let end = offset.checked_add(src.len()).expect("DMA buffer offset overflows");
        assert!(end <= self.len(), "write past the end of a DMA buffer");
        for (i, &b) in src.iter().enumerate() {
            // SAFETY: `offset + i < len` (checked above) and the buffer is
            // ours to write.
            unsafe { ptr::write_volatile(self.virt.as_ptr().add(offset + i), b) };
        }
    }

    /// Copy `dst.len()` bytes from the buffer at `offset` into `dst`.
    /// Panics if that runs past the end.
    pub fn read(&self, offset: usize, dst: &mut [u8]) {
        let end = offset.checked_add(dst.len()).expect("DMA buffer offset overflows");
        assert!(end <= self.len(), "read past the end of a DMA buffer");
        for (i, b) in dst.iter_mut().enumerate() {
            // SAFETY: `offset + i < len` (checked above). Volatile because
            // the device writes this memory behind the compiler's back.
            *b = unsafe { ptr::read_volatile(self.virt.as_ptr().add(offset + i)) };
        }
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        let mut frames = FRAMES.lock();
        if let Some(f) = frames.as_mut() {
            f.free(self.phys, self.order).expect("DMA buffer frames were not allocated");
        }
    }
}

crate::kernel_test! {
    fn dma_buffers_are_zeroed_aligned_and_freed() -> TestResult {
        let free = || FRAMES.lock().as_ref().map_or(0, |f| f.free_frames());
        let before = free();
        {
            let b = DmaBuffer::new(3 * PAGE_SIZE as usize + 1).ok_or("out of frames")?;
            if b.len() != 4 * PAGE_SIZE as usize || !b.phys().is_multiple_of(b.len() as u64) {
                return Err("DMA buffer not rounded up to a power of two and aligned to it");
            }
            let mut tail = [0xffu8; 8];
            b.read(b.len() - 8, &mut tail);
            if tail != [0; 8] {
                return Err("DMA buffer not zeroed");
            }
            b.write(5, b"poly");
            let mut back = [0u8; 4];
            b.read(5, &mut back);
            if &back != b"poly" {
                return Err("DMA buffer did not read back what was written");
            }
            if free() != before - 4 {
                return Err("DMA buffer took the wrong number of frames");
            }
        }
        if free() != before { Err("DMA buffer frames not freed on drop") } else { Ok(()) }
    }
}

crate::kernel_test! {
    fn bus_addresses_only_for_the_direct_map() -> TestResult {
        use alloc::boxed::Box;
        use alloc::vec;
        let page = PAGE_SIZE as usize;
        // A large heap allocation: one run of frames, page-aligned.
        let buf = vec![0u8; 3 * page];
        let base = buf.as_ptr() as u64;
        let whole = bus_address(base, buf.len()).ok_or("a three-page heap buffer was refused")?;
        // 512 bytes straddling the first page boundary.
        let start = base + page as u64 - 256;
        let bus = bus_address(start, 512).ok_or("a heap buffer crossing a page boundary was refused")?;
        if bus != whole + page as u64 - 256
            || bus_address(start + 256, 256) != Some(bus + 256)
            || arch::phys_to_virt(bus) != Some(start)
            || arch::phys_to_virt(bus + 256) != Some(start + 256)
        {
            return Err("bus addresses of a heap buffer are not its physical addresses");
        }
        // A small object lives in a slab, also direct-mapped.
        let small = Box::new([0u8; 48]);
        let at = small.as_ptr() as u64;
        if bus_address(at, 48).and_then(arch::phys_to_virt) != Some(at) {
            return Err("a small heap object has no bus address");
        }
        let d = DmaBuffer::new(2 * page).ok_or("out of frames")?;
        if bus_address(d.ptr().as_ptr() as u64, d.len()) != Some(d.phys()) {
            return Err("a DMA buffer's bus address is not its physical address");
        }
        // Refused: the kernel image (a static, and the boot stack the tests
        // run on), an MMIO mapping, the lower half, the end of the address
        // space.
        static IMAGE: [u8; 16] = [0; 16];
        let local = [0u8; 64];
        let on_stack = local.as_ptr() as u64;
        let mut refused = [IMAGE.as_ptr() as u64, arch::paging::MMIO_BASE, 0x1000, u64::MAX - 8].to_vec();
        if on_stack >= arch::KERNEL_VMA {
            refused.push(on_stack);
        }
        if refused.iter().any(|&v| bus_address(v, 16).is_some()) {
            return Err("memory outside the direct map got a bus address");
        }
        Ok(())
    }
}
