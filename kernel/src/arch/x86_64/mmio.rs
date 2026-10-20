//! Device registers through an uncached mapping: the kernel's one
//! implementation of [`Mmio`] (the trait `poly-pci` defines and `poly-irq`
//! reuses), shared by the APICs, the HPET, PCI and virtio.
//!
//! [`MmioRegion::map`] maps the range with `mm::map_mmio` (uncached,
//! no-execute, never RAM). Every access is a single volatile load or store of
//! its natural width, which device registers require; an access that does
//! not fit inside the region (or is misaligned) is dropped, reading all ones,
//! as the trait asks.

use crate::mm;
use poly_irq::Mmio;

/// A mapped block of device registers.
#[derive(Debug)]
pub struct MmioRegion {
    virt: u64,
    len: usize,
}

impl MmioRegion {
    /// Map `len` bytes of device memory at physical address `phys`. `None`
    /// if `mm::map_mmio` refuses (RAM, too large, out of window or frames).
    pub fn map(phys: u64, len: usize) -> Option<MmioRegion> {
        let virt = mm::map_mmio(phys, len as u64)?;
        Some(MmioRegion { virt, len })
    }

    /// The address of a `size`-byte access at `offset`, if it lies inside
    /// the region and is naturally aligned.
    fn at(&self, offset: usize, size: usize) -> Option<u64> {
        let end = offset.checked_add(size)?;
        (end <= self.len && offset.is_multiple_of(size)).then(|| self.virt + offset as u64)
    }
}

macro_rules! access {
    ($read:ident, $write:ident, $t:ty) => {
        fn $read(&self, offset: usize) -> $t {
            match self.at(offset, size_of::<$t>()) {
                // SAFETY: `at` checked that the access lies inside the region,
                // which `map_mmio` mapped (uncached) for the kernel's lifetime,
                // and is naturally aligned. Device registers are not Rust
                // objects, so a volatile access does not alias anything.
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
