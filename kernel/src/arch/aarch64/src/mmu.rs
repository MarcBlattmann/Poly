//! The stub's page tables: one level-1 table of 1 GiB block descriptors,
//! identity mapping the first 512 GiB of address space in the 4 KiB
//! translation granule (VMSAv8-64, Arm ARM DDI 0487 D8, "The AArch64 Virtual
//! Memory System Architecture"): everything below RAM as device memory
//! (QEMU's `virt` has its UART, GIC and virtio devices there), the gigabytes
//! of RAM as cacheable normal memory, nothing above. Real paging (4 KiB
//! pages, W^X, a higher half) comes with the kernel (ADR 0154, "What's next").

/// One gigabyte, the span of a level-1 block.
pub const GIB: u64 = 1 << 30;
/// Entries of a level-1 table.
pub const L1_ENTRIES: usize = 512;

/// `MAIR_EL1`'s attribute 0: Device-nGnRnE.
pub const ATTR_DEVICE: u64 = 0;
/// `MAIR_EL1`'s attribute 1: Normal memory, Inner and Outer Write-Back,
/// Read- and Write-Allocate, non-transient.
pub const ATTR_NORMAL: u64 = 1;
/// The `MAIR_EL1` value: attribute 0 is `0x00`, attribute 1 is `0xff`.
pub const MAIR_EL1: u64 = 0xff << 8;

/// `TCR_EL1` for the table walks of `TTBR0_EL1` only: 39-bit addresses
/// (T0SZ 25, so the walk starts at level 1), 4 KiB granule, inner
/// shareable, Write-Back Write-Allocate, `TTBR1_EL1` walks disabled
/// (EPD1), and the output size `ips` (`TCR_EL1.IPS`, bits 34:32).
pub const fn tcr_el1(ips: u64) -> u64 {
    const T0SZ: u64 = 25;
    const IRGN0_WBWA: u64 = 0b01 << 8;
    const ORGN0_WBWA: u64 = 0b01 << 10;
    const SH0_INNER: u64 = 0b11 << 12;
    const TG0_4K: u64 = 0b00 << 14;
    const EPD1: u64 = 1 << 23;
    const TG1_4K: u64 = 0b10 << 30;
    T0SZ | IRGN0_WBWA | ORGN0_WBWA | SH0_INNER | TG0_4K | EPD1 | TG1_4K | (ips << 32)
}

/// What a block maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemType {
    Device,
    Normal,
}

/// A level-1 block descriptor for the gigabyte at `pa`: valid block (bits
/// 1:0 = 0b01), `AttrIndx`, EL1 read/write and no EL0 access (AP = 0b00),
/// Access flag set, not executable at EL0 (UXN), and for device memory not
/// executable at EL1 either (PXN) and outer shareable; normal memory is
/// inner shareable.
pub const fn l1_block(pa: u64, ty: MemType) -> u64 {
    const VALID_BLOCK: u64 = 0b01;
    const AF: u64 = 1 << 10;
    const PXN: u64 = 1 << 53;
    const UXN: u64 = 1 << 54;
    const SH_OUTER: u64 = 0b10 << 8;
    const SH_INNER: u64 = 0b11 << 8;
    let (attr, sh, xn) = match ty {
        MemType::Device => (ATTR_DEVICE, SH_OUTER, PXN | UXN),
        MemType::Normal => (ATTR_NORMAL, SH_INNER, UXN),
    };
    (pa & 0x0000_ffff_c000_0000) | VALID_BLOCK | (attr << 2) | sh | AF | xn
}

/// Why [`identity_map`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// RAM is empty, or reaches past the 512 GiB the table covers.
    BadRam,
}

/// Fill `table`: the gigabytes of `[ram_base, ram_end)` normal, the ones
/// entirely below `ram_base` device, the rest invalid.
pub fn identity_map(table: &mut [u64; L1_ENTRIES], ram_base: u64, ram_end: u64) -> Result<(), MapError> {
    if ram_base >= ram_end || ram_end > L1_ENTRIES as u64 * GIB {
        return Err(MapError::BadRam);
    }
    for (i, entry) in table.iter_mut().enumerate() {
        let (start, end) = (i as u64 * GIB, (i as u64 + 1) * GIB);
        *entry = if start < ram_end && end > ram_base {
            l1_block(start, MemType::Normal)
        } else if end <= ram_base {
            l1_block(start, MemType::Device)
        } else {
            0
        };
    }
    Ok(())
}

/// An address no entry of [`identity_map`] covers: the first gigabyte after
/// RAM, if the table reaches that far.
pub const fn first_unmapped_address(ram_end: u64) -> Option<u64> {
    let addr = ram_end.div_ceil(GIB) * GIB;
    if addr < L1_ENTRIES as u64 * GIB { Some(addr) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_descriptor_bits() {
        let normal = l1_block(0x4000_0000, MemType::Normal);
        assert_eq!(normal & 0b11, 0b01, "a valid block, not a table");
        assert_eq!((normal >> 2) & 0b111, ATTR_NORMAL);
        assert_eq!((normal >> 8) & 0b11, 0b11, "inner shareable");
        assert_ne!(normal & (1 << 10), 0, "access flag");
        assert_eq!(normal & (1 << 53), 0, "RAM is executable at EL1");
        assert_ne!(normal & (1 << 54), 0, "never at EL0");
        assert_eq!(normal & 0x0000_ffff_c000_0000, 0x4000_0000);

        let device = l1_block(0, MemType::Device);
        assert_eq!((device >> 2) & 0b111, ATTR_DEVICE);
        assert_eq!((device >> 8) & 0b11, 0b10, "outer shareable");
        assert_ne!(device & (1 << 53), 0, "devices are not executable");
        // Only the block's address bits come from `pa`.
        assert_eq!(l1_block(0x4000_1234, MemType::Normal), normal);
    }

    #[test]
    fn identity_map_of_qemu_virt() {
        let mut t = [u64::MAX; L1_ENTRIES];
        identity_map(&mut t, 0x4000_0000, 0x5000_0000).unwrap();
        assert_eq!(t[0], l1_block(0, MemType::Device), "UART, GIC, virtio");
        assert_eq!(t[1], l1_block(GIB, MemType::Normal), "RAM");
        assert!(t[2..].iter().all(|&e| e == 0), "nothing above RAM");
    }

    #[test]
    fn ram_spanning_gigabytes_and_unaligned_ram() {
        let mut t = [0; L1_ENTRIES];
        identity_map(&mut t, 0x4000_0000, 0x1_0000_0000).unwrap();
        assert_eq!([t[1] & 3, t[2] & 3, t[3] & 3], [1, 1, 1]);
        assert_eq!(t[3], l1_block(3 * GIB, MemType::Normal));
        assert_eq!(t[4], 0);
        // RAM that starts inside a gigabyte makes that whole gigabyte normal.
        identity_map(&mut t, 0x4800_0000, 0x5000_0000).unwrap();
        assert_eq!(t[1], l1_block(GIB, MemType::Normal));
        assert_eq!(t[0], l1_block(0, MemType::Device));
    }

    #[test]
    fn refuses_impossible_ram() {
        let mut t = [0; L1_ENTRIES];
        assert_eq!(identity_map(&mut t, 0x4000_0000, 0x4000_0000), Err(MapError::BadRam));
        assert_eq!(identity_map(&mut t, 0x4000_0000, 513 * GIB), Err(MapError::BadRam));
    }

    #[test]
    fn unmapped_address_is_past_ram() {
        assert_eq!(first_unmapped_address(0x5000_0000), Some(2 * GIB));
        assert_eq!(first_unmapped_address(2 * GIB), Some(2 * GIB));
        assert_eq!(first_unmapped_address(512 * GIB), None);
        let mut t = [0; L1_ENTRIES];
        identity_map(&mut t, 0x4000_0000, 0x5000_0000).unwrap();
        let hole = first_unmapped_address(0x5000_0000).unwrap();
        assert_eq!(t[(hole / GIB) as usize], 0);
    }

    #[test]
    fn tcr_fields() {
        let tcr = tcr_el1(0b101);
        assert_eq!(tcr & 0x3f, 25, "T0SZ");
        assert_eq!((tcr >> 14) & 0b11, 0, "4 KiB granule for TTBR0");
        assert_ne!(tcr & (1 << 23), 0, "TTBR1 walks disabled");
        assert_eq!((tcr >> 32) & 0b111, 0b101, "IPS");
    }
}
