//! Device registers through an uncached mapping: the kernel's one
//! implementation of [`Mmio`] (the trait `poly-pci` defines and `poly-irq`
//! reuses), shared by the APICs, the HPET, PCI BARs and virtio.
//!
//! [`MmioRegion::map`] maps a range with `mm::map_mmio` (uncached,
//! no-execute, never RAM); `pci::Device::map_bar` hands out regions over a
//! BAR it mapped once. Every access is a single volatile load or store of its
//! natural width, which device registers require; an access that does not
//! fit inside the region (or is misaligned) is dropped, reading all ones, as
//! the trait asks. Registers are little-endian, like the CPU.

use crate::mm;
use poly_irq::Mmio;

/// A mapped block of device registers, or part of one. Copies share the
/// mapping, which lives for the kernel's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MmioRegion {
    virt: u64,
    len: usize,
}

impl MmioRegion {
    /// Map `len` bytes of device memory at physical address `phys`. `None`
    /// if `mm::map_mmio` refuses (RAM, too large, out of window or frames).
    pub fn map(phys: u64, len: usize) -> Option<MmioRegion> {
        let virt = mm::map_mmio(phys, len as u64)?;
        // SAFETY: `map_mmio` just mapped `len` bytes at `virt`, uncached and
        // never unmapped.
        Some(unsafe { MmioRegion::from_mapping(virt, len) })
    }

    /// A region over `len` bytes at `virt`.
    ///
    /// # Safety
    /// `virt..virt + len` is device memory mapped by `mm::map_mmio` (or an
    /// equivalent uncached mapping) for the kernel's lifetime.
    pub unsafe fn from_mapping(virt: u64, len: usize) -> MmioRegion {
        MmioRegion { virt, len }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// `len` bytes starting `offset` bytes in, if they lie inside.
    pub fn subregion(&self, offset: usize, len: usize) -> Option<MmioRegion> {
        (offset.checked_add(len)? <= self.len).then_some(MmioRegion { virt: self.virt + offset as u64, len })
    }

    /// The address of a `size`-byte access at `offset`, if it lies inside
    /// the region and is naturally aligned.
    fn at(&self, offset: usize, size: usize) -> Option<u64> {
        let end = offset.checked_add(size)?;
        let addr = self.virt + offset as u64;
        (end <= self.len && addr.is_multiple_of(size as u64)).then_some(addr)
    }
}

macro_rules! access {
    ($read:ident, $write:ident, $t:ty) => {
        fn $read(&self, offset: usize) -> $t {
            match self.at(offset, size_of::<$t>()) {
                // SAFETY: `at` checked that the access lies inside the region,
                // which is mapped uncached for the kernel's lifetime (see
                // `from_mapping`), and is naturally aligned. Device registers
                // are not Rust objects, so a volatile access aliases nothing.
                Some(addr) => unsafe { (addr as *const $t).read_volatile() },
                None => <$t>::MAX,
            }
        }

        fn $write(&self, offset: usize, value: $t) {
            if let Some(addr) = self.at(offset, size_of::<$t>()) {
                // SAFETY: as for the read: inside the mapped region, aligned.
                unsafe { (addr as *mut $t).write_volatile(value) }
            }
        }
    };
}

impl Mmio for MmioRegion {
    access!(read8, write8, u8);
    access!(read16, write16, u16);
    access!(read32, write32, u32);
}
