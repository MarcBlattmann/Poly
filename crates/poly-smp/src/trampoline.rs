//! The application processors' start-up area and its parameter block.
//!
//! A start-up IPI starts an AP in real mode at `vector * 4096` (SDM Vol. 3A
//! §9.4.4.1, §11.6.1: CS = vector << 8, IP = 0), so the first code it runs
//! must lie in a 4 KiB page below 1 MiB. The kernel keeps a small area
//! there (four pages, [`AREA_SIZE`]) for the whole of bring-up:
//!
//! ```text
//! base + 0x0000  code: real mode -> protected mode -> long mode (kernel asm)
//! base + 0x0f00  parameter block (offsets in `param`)
//! base + 0x1000  PML4: entry 0 -> the PDPT below, entries 256-511 the kernel's
//! base + 0x2000  PDPT: entry 0 -> the PD below
//! base + 0x3000  PD:   entry 0 maps [0, 2 MiB) with one large page
//! ```
//!
//! The page tables identity-map the first 2 MiB, so the code keeps running
//! after paging is turned on, and share the kernel's higher half, so it can
//! jump to the kernel's entry point. CR3 is loaded in 32-bit mode, which
//! takes only 32 bits: the tables sit below 1 MiB with the code.
//!
//! The code switches modes the way SDM Vol. 3A §9.9.1 and §9.8.5 describe:
//! load a GDT, set CR0.PE and jump to a 32-bit code segment; then set
//! CR4.PAE, load CR3, set IA32_EFER.LME (and NXE, which the kernel's page
//! tables need), set CR0.PG and jump to a 64-bit code segment. Absolute
//! addresses (the GDT's base, the far jumps' targets) depend on where the
//! area is, so they are in the parameter block, which [`Params::write`]
//! fills in, rather than in the code.

/// Bytes of the start-up area: the code page and three page tables.
pub const AREA_SIZE: u64 = 4 * PAGE;

const PAGE: u64 = 4096;

/// Start-up vectors 0xa0-0xbf would point at the legacy video memory and
/// the BIOS area above it, which is not RAM; the area must end below.
pub const AREA_LIMIT: u64 = 0xa_0000;

/// The page tables' offsets in the area.
pub const PML4_OFFSET: u64 = PAGE;
pub const PDPT_OFFSET: u64 = 2 * PAGE;
pub const PD_OFFSET: u64 = 3 * PAGE;

/// Offsets of the parameter block's fields from the area's base. The
/// kernel's trampoline code uses the same constants.
pub mod param {
    /// The temporary GDT: null, 32-bit code, 32-bit data, 64-bit code.
    pub const GDT: usize = 0xf00;
    /// Its descriptor for `LGDT`: 16-bit limit, 32-bit base.
    pub const GDTR: usize = 0xf20;
    /// Far pointer (32-bit offset, 16-bit selector) to the 32-bit code.
    pub const FAR32: usize = 0xf28;
    /// Far pointer to the 64-bit code.
    pub const FAR64: usize = 0xf30;
    /// CR3 for the switch: the area's PML4 (32 bits).
    pub const CR3: usize = 0xf38;
    /// CR4 for the switch (PAE at least).
    pub const CR4: usize = 0xf3c;
    /// Bits to set in IA32_EFER (LME at least).
    pub const EFER: usize = 0xf40;
    /// The AP's stack top (a kernel virtual address).
    pub const STACK: usize = 0xf48;
    /// The kernel function the AP calls, with `ARG` as its argument.
    pub const ENTRY: usize = 0xf50;
    pub const ARG: usize = 0xf58;
    /// End of the block: the code must end before [`GDT`].
    pub const END: usize = 0xf60;
}

/// Selectors of the temporary GDT.
pub const CODE32: u16 = 0x08;
pub const DATA32: u16 = 0x10;
pub const CODE64: u16 = 0x18;

/// The temporary GDT's descriptors (SDM Vol. 3A §3.4.5): flat 4 GiB
/// segments, present, DPL 0; code execute/read, data read/write; the 32-bit
/// ones with D/B = 1 and 4 KiB granularity, the 64-bit code with L = 1.
pub const GDT: [u64; 4] = [0, 0x00cf_9a00_0000_ffff, 0x00cf_9200_0000_ffff, 0x00af_9a00_0000_ffff];

/// IA32_EFER bits (SDM Vol. 4, Table 2-2).
pub const EFER_LME: u32 = 1 << 8;
pub const EFER_NXE: u32 = 1 << 11;
/// CR4 bits (SDM Vol. 3A §2.5).
pub const CR4_PAE: u32 = 1 << 5;
pub const CR4_PGE: u32 = 1 << 7;

/// Page-table entry bits (SDM Vol. 3A §4.5): present, writable, and the
/// large-page bit of a PD entry.
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const LARGE: u64 = 1 << 7;

/// The lowest suitable base for the start-up area among `usable` RAM
/// ranges `(start, end)`: page-aligned, at 4 KiB or above (page 0 holds the
/// real-mode interrupt vectors and BIOS data), wholly inside one range and
/// ending at or below [`AREA_LIMIT`]. `None` if there is no such place.
pub fn find_base(usable: impl IntoIterator<Item = (u64, u64)>) -> Option<u64> {
    usable
        .into_iter()
        .filter_map(|(start, end)| {
            let base = start.max(PAGE).next_multiple_of(PAGE);
            let top = base.checked_add(AREA_SIZE)?;
            (top <= end && top <= AREA_LIMIT).then_some(base)
        })
        .min()
}

/// The start-up IPI's vector for an area at `base`: its page number.
/// `None` unless `base` is a page-aligned address the area may use.
pub fn startup_vector(base: u64) -> Option<u8> {
    let ok = base.is_multiple_of(PAGE) && base >= PAGE && base + AREA_SIZE <= AREA_LIMIT;
    ok.then_some((base >> 12) as u8)
}

/// The identity-map entries of the area's own page tables at `base`:
/// PML4[0], PDPT[0] and PD[0] (2 MiB at physical 0, which holds the area).
pub fn identity_entries(base: u64) -> [u64; 3] {
    [base + PDPT_OFFSET | PRESENT | WRITABLE, base + PD_OFFSET | PRESENT | WRITABLE, PRESENT | WRITABLE | LARGE]
}

/// Why [`Params::write`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamError {
    /// The area's base is not a valid start-up page.
    Base,
    /// The page buffer is shorter than one page.
    Short,
    /// A code offset lies in the parameter block or beyond.
    Offset,
}

/// What one AP's start needs, written into the area's first page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// Physical address of the area.
    pub base: u64,
    /// Offsets of the 32-bit and 64-bit code from the base.
    pub entry32: u32,
    pub entry64: u32,
    /// CR4 and EFER bits for the switch.
    pub cr4: u32,
    pub efer: u32,
    /// The AP's kernel stack top, entry point and its argument.
    pub stack: u64,
    pub entry: u64,
    pub arg: u64,
}

impl Params {
    /// Write the parameter block (GDT, GDTR, far pointers, CR3 of the
    /// area's PML4, CR4, EFER, stack, entry, argument) into `page`, the
    /// area's first page; the code bytes before [`param::GDT`] are left
    /// alone.
    pub fn write(&self, page: &mut [u8]) -> Result<(), ParamError> {
        startup_vector(self.base).ok_or(ParamError::Base)?;
        if page.len() < PAGE as usize {
            return Err(ParamError::Short);
        }
        if self.entry32 as usize >= param::GDT || self.entry64 as usize >= param::GDT {
            return Err(ParamError::Offset);
        }
        let base = self.base as u32;
        let mut put = |at: usize, bytes: &[u8]| page[at..at + bytes.len()].copy_from_slice(bytes);
        for (i, d) in GDT.iter().enumerate() {
            put(param::GDT + 8 * i, &d.to_le_bytes());
        }
        put(param::GDTR, &((GDT.len() * 8 - 1) as u16).to_le_bytes());
        put(param::GDTR + 2, &(base + param::GDT as u32).to_le_bytes());
        put(param::FAR32, &(base + self.entry32).to_le_bytes());
        put(param::FAR32 + 4, &CODE32.to_le_bytes());
        put(param::FAR64, &(base + self.entry64).to_le_bytes());
        put(param::FAR64 + 4, &CODE64.to_le_bytes());
        put(param::CR3, &(base + PML4_OFFSET as u32).to_le_bytes());
        put(param::CR4, &(self.cr4 | CR4_PAE).to_le_bytes());
        put(param::EFER, &(self.efer | EFER_LME).to_le_bytes());
        put(param::STACK, &self.stack.to_le_bytes());
        put(param::ENTRY, &self.entry.to_le_bytes());
        put(param::ARG, &self.arg.to_le_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(p: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(p[at..at + 2].try_into().unwrap())
    }
    fn u32_at(p: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(p[at..at + 4].try_into().unwrap())
    }
    fn u64_at(p: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(p[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn the_area_goes_into_low_usable_ram() {
        // QEMU's PVH map: RAM up to 0x9fc00, then the EBDA.
        assert_eq!(find_base([(0, 0x9fc00)]), Some(0x1000));
        assert_eq!(find_base([(0x500, 0x9fc00)]), Some(0x1000));
        assert_eq!(find_base([(0x8800, 0x9fc00), (0x2000, 0x6000)]), Some(0x2000));
        assert_eq!(find_base([(0x2000, 0x5000)]), None, "too small");
        assert_eq!(find_base([(0x9d000, 0x10_0000)]), None, "would reach the video memory");
        assert_eq!(find_base([(0x9c000, 0xa0000)]), Some(0x9c000));
        assert_eq!(find_base([(0x10_0000, 0x800_0000)]), None, "above 1 MiB");
        assert_eq!(find_base([]), None);
    }

    #[test]
    fn startup_vectors_are_page_numbers() {
        assert_eq!(startup_vector(0x8000), Some(0x08));
        assert_eq!(startup_vector(0x9c000), Some(0x9c));
        assert_eq!(startup_vector(0x9d000), None, "the area would cross 0xa0000");
        assert_eq!(startup_vector(0), None);
        assert_eq!(startup_vector(0x8001), None);
    }

    #[test]
    fn identity_tables_map_the_first_two_megabytes() {
        let [pml4, pdpt, pd] = identity_entries(0x8000);
        assert_eq!(pml4, 0xa003);
        assert_eq!(pdpt, 0xb003);
        assert_eq!(pd, 0x83, "present, writable, 2 MiB page at 0");
    }

    #[test]
    fn the_parameter_block_has_absolute_addresses() {
        let mut page = [0xccu8; 4096];
        let p = Params {
            base: 0x8000,
            entry32: 0x40,
            entry64: 0x80,
            cr4: CR4_PGE,
            efer: EFER_NXE,
            stack: 0xffff_8000_1234_0000,
            entry: 0xffff_ffff_8010_0000,
            arg: 3,
        };
        p.write(&mut page).unwrap();
        assert!(page[..param::GDT].iter().all(|&b| b == 0xcc), "code bytes untouched");
        for (i, d) in GDT.iter().enumerate() {
            assert_eq!(u64_at(&page, param::GDT + 8 * i), *d);
        }
        assert_eq!(u16_at(&page, param::GDTR), 31);
        assert_eq!(u32_at(&page, param::GDTR + 2), 0x8f00);
        assert_eq!((u32_at(&page, param::FAR32), u16_at(&page, param::FAR32 + 4)), (0x8040, CODE32));
        assert_eq!((u32_at(&page, param::FAR64), u16_at(&page, param::FAR64 + 4)), (0x8080, CODE64));
        assert_eq!(u32_at(&page, param::CR3), 0x9000);
        assert_eq!(u32_at(&page, param::CR4), CR4_PGE | CR4_PAE);
        assert_eq!(u32_at(&page, param::EFER), EFER_NXE | EFER_LME);
        assert_eq!(u64_at(&page, param::STACK), 0xffff_8000_1234_0000);
        assert_eq!(u64_at(&page, param::ENTRY), 0xffff_ffff_8010_0000);
        assert_eq!(u64_at(&page, param::ARG), 3);
        assert!(param::END <= 4096);
        assert_eq!(page[param::END..], [0xcc; 4096 - param::END]);
    }

    #[test]
    fn bad_parameters_are_refused() {
        let ok = Params { base: 0x8000, entry32: 0x40, entry64: 0x80, cr4: 0, efer: 0, stack: 0, entry: 0, arg: 0 };
        let mut page = [0u8; 4096];
        assert_eq!(Params { base: 0x8800, ..ok }.write(&mut page), Err(ParamError::Base));
        assert_eq!(ok.write(&mut [0u8; 100]), Err(ParamError::Short));
        assert_eq!(Params { entry64: param::GDT as u32, ..ok }.write(&mut page), Err(ParamError::Offset));
    }

    #[test]
    fn descriptors_have_the_documented_bits() {
        // Access byte (bits 47:40): present, DPL 0, code/data, type.
        let access = |d: u64| (d >> 40) & 0xff;
        let flags = |d: u64| (d >> 52) & 0xf; // G, D/B, L, AVL
        assert_eq!((access(GDT[1]), flags(GDT[1])), (0x9a, 0b1100));
        assert_eq!((access(GDT[2]), flags(GDT[2])), (0x92, 0b1100));
        assert_eq!((access(GDT[3]), flags(GDT[3])), (0x9a, 0b1010), "L set, D clear");
        assert_eq!(usize::from(CODE64) / 8, 3);
    }
}
