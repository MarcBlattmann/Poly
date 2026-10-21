//! A minimal PE32+ image writer, for the test programs Poly builds itself
//! (no Microsoft toolchain, no Microsoft DLLs). Formats from the PE Format
//! specification (Learn: "PE Format"): MS-DOS stub header with `e_lfanew`,
//! the `PE\0\0` signature, the COFF file header, the PE32+ optional header
//! with 16 data directories, the section table, and a `.reloc` section of
//! `IMAGE_REL_BASED_DIR64` base relocations.
//!
//! Section and file alignment are both 4 KiB, so the file is laid out as
//! the image is mapped: an RVA is a file offset, and code assembled as one
//! position-independent block keeps its RIP-relative references to data
//! placed a known number of pages after it.

use alloc::vec;
use alloc::vec::Vec;

const ALIGN: u32 = 0x1000;
const OPTIONAL_HEADER_SIZE: u16 = 240;
const PE_OFFSET: usize = 0x40;

/// `IMAGE_SCN_*` combinations (PE Format, "Section Flags").
pub const TEXT: u32 = 0x6000_0020; // CNT_CODE | MEM_EXECUTE | MEM_READ
pub const DATA: u32 = 0xC000_0040; // CNT_INITIALIZED_DATA | MEM_READ | MEM_WRITE
pub const RDATA: u32 = 0x4000_0040; // CNT_INITIALIZED_DATA | MEM_READ
const RELOC: u32 = 0x4200_0040; // CNT_INITIALIZED_DATA | MEM_DISCARDABLE | MEM_READ

/// `IMAGE_REL_BASED_DIR64`.
const DIR64: u16 = 10;

/// One section: its bytes and how much memory it takes (at least the
/// bytes).
#[derive(Clone, Debug)]
pub struct PeSection {
    pub name: [u8; 8],
    pub data: Vec<u8>,
    pub virtual_size: u32,
    pub characteristics: u32,
}

/// An executable to write. Sections go at RVA 0x1000, each starting on a
/// fresh page in the order given, then `.reloc` if there are relocations.
#[derive(Clone, Debug)]
pub struct PeImage {
    pub image_base: u64,
    /// RVA of the entry point.
    pub entry: u32,
    pub sections: Vec<PeSection>,
    /// RVAs of 64-bit absolute addresses to relocate.
    pub relocations: Vec<u32>,
    /// Set `IMAGE_FILE_RELOCS_STRIPPED`: the image cannot move.
    pub fixed: bool,
    pub stack_reserve: u64,
    pub stack_commit: u64,
}

const fn align(v: u32) -> u32 {
    v.div_ceil(ALIGN) * ALIGN
}

impl PeImage {
    /// The RVA section `index` gets.
    pub fn section_rva(&self, index: usize) -> u32 {
        let mut rva = ALIGN;
        for s in &self.sections[..index] {
            rva += align(s.virtual_size.max(s.data.len() as u32).max(1));
        }
        rva
    }

    fn reloc_section(&self) -> Option<PeSection> {
        if self.relocations.is_empty() {
            return None;
        }
        let mut rvas = self.relocations.clone();
        rvas.sort_unstable();
        let mut data = Vec::new();
        let mut i = 0;
        while i < rvas.len() {
            let page = rvas[i] & !(ALIGN - 1);
            let mut entries: Vec<u16> = Vec::new();
            while i < rvas.len() && rvas[i] & !(ALIGN - 1) == page {
                entries.push(DIR64 << 12 | (rvas[i] & (ALIGN - 1)) as u16);
                i += 1;
            }
            if !entries.len().is_multiple_of(2) {
                entries.push(0); // IMAGE_REL_BASED_ABSOLUTE: padding
            }
            data.extend_from_slice(&page.to_le_bytes());
            data.extend_from_slice(&(8 + 2 * entries.len() as u32).to_le_bytes());
            for e in entries {
                data.extend_from_slice(&e.to_le_bytes());
            }
        }
        let size = data.len() as u32;
        Some(PeSection { name: *b".reloc\0\0", data, virtual_size: size, characteristics: RELOC })
    }

    /// The image file.
    pub fn write(&self) -> Vec<u8> {
        let mut sections = self.sections.clone();
        let reloc = self.reloc_section();
        let reloc_index = reloc.as_ref().map(|_| sections.len());
        sections.extend(reloc);
        let mut rvas = Vec::new();
        let mut rva = ALIGN;
        for s in &sections {
            rvas.push(rva);
            rva += align(s.virtual_size.max(s.data.len() as u32).max(1));
        }
        let size_of_image = rva;
        let mut out = vec![0u8; size_of_image as usize];
        let put16 = |out: &mut Vec<u8>, at: usize, v: u16| out[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let put32 = |out: &mut Vec<u8>, at: usize, v: u32| out[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let put64 = |out: &mut Vec<u8>, at: usize, v: u64| out[at..at + 8].copy_from_slice(&v.to_le_bytes());

        out[0..2].copy_from_slice(b"MZ");
        put32(&mut out, 0x3C, PE_OFFSET as u32);
        out[PE_OFFSET..PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
        let coff = PE_OFFSET + 4;
        put16(&mut out, coff, poly_pe::machine::AMD64);
        put16(&mut out, coff + 2, sections.len() as u16);
        put16(&mut out, coff + 16, OPTIONAL_HEADER_SIZE);
        let mut characteristics = poly_pe::file_flags::EXECUTABLE_IMAGE | poly_pe::file_flags::LARGE_ADDRESS_AWARE;
        if self.fixed {
            characteristics |= poly_pe::file_flags::RELOCS_STRIPPED;
        }
        put16(&mut out, coff + 18, characteristics);

        let opt = coff + 20;
        let sum = |flag: u32| -> u32 {
            sections.iter().filter(|s| s.characteristics & flag != 0).map(|s| align(s.data.len() as u32)).sum()
        };
        put16(&mut out, opt, 0x20B);
        put32(&mut out, opt + 4, sum(0x20));
        put32(&mut out, opt + 8, sum(0x40));
        put32(&mut out, opt + 16, self.entry);
        put32(&mut out, opt + 20, ALIGN);
        put64(&mut out, opt + 24, self.image_base);
        put32(&mut out, opt + 32, ALIGN);
        put32(&mut out, opt + 36, ALIGN);
        put16(&mut out, opt + 40, 6);
        put16(&mut out, opt + 48, 6);
        put32(&mut out, opt + 56, size_of_image);
        put32(&mut out, opt + 60, ALIGN);
        put16(&mut out, opt + 68, poly_pe::subsystem::WINDOWS_CUI);
        let mut dll = poly_pe::dll_flags::NX_COMPAT;
        if !self.fixed {
            dll |= poly_pe::dll_flags::DYNAMIC_BASE | poly_pe::dll_flags::HIGH_ENTROPY_VA;
        }
        put16(&mut out, opt + 70, dll);
        put64(&mut out, opt + 72, self.stack_reserve);
        put64(&mut out, opt + 80, self.stack_commit);
        put64(&mut out, opt + 88, 0x10_0000);
        put64(&mut out, opt + 96, 0x1000);
        put32(&mut out, opt + 108, 16);
        if let Some(i) = reloc_index {
            let dir = opt + 112 + poly_pe::dir::BASERELOC * 8;
            put32(&mut out, dir, rvas[i]);
            put32(&mut out, dir + 4, sections[i].data.len() as u32);
        }

        let table = opt + OPTIONAL_HEADER_SIZE as usize;
        for (i, s) in sections.iter().enumerate() {
            let h = table + i * 40;
            out[h..h + 8].copy_from_slice(&s.name);
            put32(&mut out, h + 8, s.virtual_size.max(s.data.len() as u32));
            put32(&mut out, h + 12, rvas[i]);
            put32(&mut out, h + 16, align(s.data.len() as u32));
            put32(&mut out, h + 20, if s.data.is_empty() { 0 } else { rvas[i] });
            put32(&mut out, h + 36, s.characteristics);
            let at = rvas[i] as usize;
            out[at..at + s.data.len()].copy_from_slice(&s.data);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::ImageSection;

    fn sample(fixed: bool) -> PeImage {
        let mut data = vec![0u8; 16];
        data[8..16].copy_from_slice(&(0x1_4000_1000u64).to_le_bytes());
        PeImage {
            image_base: 0x1_4000_0000,
            entry: 0x1000,
            sections: vec![
                PeSection { name: *b".text\0\0\0", data: vec![0xC3], virtual_size: 1, characteristics: TEXT },
                PeSection { name: *b".data\0\0\0", data, virtual_size: 0x1800, characteristics: DATA },
            ],
            relocations: if fixed { vec![] } else { vec![0x2008] },
            fixed,
            stack_reserve: 0x10_0000,
            stack_commit: 0x1000,
        }
    }

    #[test]
    fn writes_an_image_poly_pe_loads() {
        let file = sample(false).write();
        let pe = poly_pe::Pe::parse(&file).unwrap();
        assert_eq!(pe.info().image_base(), 0x1_4000_0000);
        assert_eq!(pe.info().entry_point(), 0x1000);
        assert!(pe.info().relocatable());
        let plan = pe.load_plan().unwrap();
        assert_eq!(plan.size_of_image, 0x5000);
        let img = ImageSection::from_file(&file).unwrap();
        use crate::consts::*;
        let prot: Vec<u32> = img.pages.iter().map(|p| p.value()).collect();
        assert_eq!(prot, [PAGE_READONLY, PAGE_EXECUTE_READ, PAGE_WRITECOPY, PAGE_WRITECOPY, PAGE_READONLY]);
        let moved = img.relocated(0x2_0000_0000).unwrap();
        let slot = u64::from_le_bytes(moved[0x2008..0x2010].try_into().unwrap());
        assert_eq!(slot, 0x2_0000_1000);
        assert_eq!(&img.image[0x2008..0x2010], &0x1_4000_1000u64.to_le_bytes());
    }

    #[test]
    fn fixed_images_cannot_move() {
        let img = ImageSection::from_file(&sample(true).write()).unwrap();
        assert!(!img.relocatable());
        assert!(img.relocated(0x2_0000_0000).is_err());
    }
}
