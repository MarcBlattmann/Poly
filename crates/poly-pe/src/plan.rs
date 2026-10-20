//! Planning an image's layout in memory (spec "Section Table": sections are
//! loaded at `VirtualAddress`, aligned to `SectionAlignment`; bytes past
//! `SizeOfRawData` up to `VirtualSize` are zero-filled).

use crate::headers::{FileHeader, Pe, Protection, SectionHeader, SectionTable};
use crate::read::align_up;
use crate::view::{Layout, View};
use crate::{Error, dir};

/// Page size of the machines we load for; images with a smaller
/// `SectionAlignment` must have `FileAlignment == SectionAlignment` (spec
/// "Optional Header Windows-Specific Fields").
const PAGE_SIZE: u32 = 4096;

/// How to lay out an image in memory. Made by [`Pe::load_plan`], which
/// checks that the headers describe a consistent layout.
#[derive(Clone, Copy, Debug)]
pub struct LoadPlan<'a> {
    /// The preferred base address (`ImageBase`).
    pub image_base: u64,
    /// Bytes of address space the image needs: `SizeOfImage` rounded up to
    /// the section alignment.
    pub size_of_image: u32,
    pub section_alignment: u32,
    pub file_alignment: u32,
    /// `SizeOfHeaders`: bytes of headers copied to RVA 0.
    pub size_of_headers: u32,
    /// RVA of the entry point, zero if none.
    pub entry_point: u32,
    /// The image may be loaded elsewhere than `image_base`
    /// (`IMAGE_FILE_RELOCS_STRIPPED` is clear).
    pub relocatable: bool,
    /// The image has a base relocation directory.
    pub has_relocations: bool,
    view: View<'a>,
    sections: SectionTable<'a>,
}

/// One range of the image in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub rva: u32,
    /// Size in memory, rounded up to the section alignment.
    pub size: u32,
    /// Where the bytes to copy start in the parsed data (a file offset, or
    /// the RVA for a mapped image).
    pub data_offset: usize,
    /// How many bytes to copy; the rest of `size` is zero.
    pub data_size: u32,
    pub protection: Protection,
    /// The section's `IMAGE_SCN_*` flags (zero for the headers).
    pub characteristics: u32,
    /// Index in the section table; `None` for the headers.
    pub section: Option<usize>,
}

impl<'a> LoadPlan<'a> {
    /// Whether loading at `base` means applying relocations.
    pub fn needs_relocation(&self, base: u64) -> bool {
        base != self.image_base
    }

    /// Whether the image may be loaded at `base`.
    pub fn can_load_at(&self, base: u64) -> bool {
        base == self.image_base || self.relocatable
    }

    /// The headers, then each section in table order.
    pub fn regions(&self) -> Regions<'a> {
        Regions { plan: *self, next: 0 }
    }

    fn region(&self, index: usize) -> Option<Region> {
        if index == 0 {
            return Some(Region {
                rva: 0,
                size: align_up(self.size_of_headers, self.section_alignment).unwrap_or(u32::MAX),
                data_offset: 0,
                data_size: self.size_of_headers,
                protection: Protection::READ_ONLY,
                characteristics: 0,
                section: None,
            });
        }
        let s = self.sections.get(index - 1)?;
        Some(Region {
            rva: s.virtual_address,
            size: align_up(s.mapped_size(), self.section_alignment).unwrap_or(u32::MAX),
            data_offset: match self.view.layout() {
                Layout::File => s.pointer_to_raw_data as usize,
                Layout::Mapped => s.virtual_address as usize,
            },
            data_size: s.file_backed_size(),
            protection: s.protection(),
            characteristics: s.characteristics,
            section: Some(index - 1),
        })
    }

    /// Lays the image out in `image` (at least [`size_of_image`] bytes):
    /// everything zeroed, then the headers and each section's file-backed
    /// bytes copied to their RVAs. Protections are not this function's
    /// business; apply [`regions`](Self::regions)' after relocating and
    /// binding imports.
    ///
    /// [`size_of_image`]: Self::size_of_image
    pub fn map_into(&self, image: &mut [u8]) -> Result<(), Error> {
        let total = self.size_of_image as usize;
        let image = image.get_mut(..total).ok_or(Error::BufferTooSmall)?;
        image.fill(0);
        let data = self.view.data();
        for region in self.regions() {
            let len = region.data_size as usize;
            let src = region.data_offset.checked_add(len).and_then(|end| data.get(region.data_offset..end));
            let dst = (region.rva as usize).checked_add(len).and_then(|end| image.get_mut(region.rva as usize..end));
            match (src, dst) {
                (Some(src), Some(dst)) => dst.copy_from_slice(src),
                _ => return Err(Error::BadLayout("region outside the data or the image")),
            }
        }
        Ok(())
    }
}

/// Iterator over a plan's [`Region`]s.
#[derive(Clone, Debug)]
pub struct Regions<'a> {
    plan: LoadPlan<'a>,
    next: usize,
}

impl Iterator for Regions<'_> {
    type Item = Region;

    fn next(&mut self) -> Option<Region> {
        let r = self.plan.region(self.next)?;
        self.next += 1;
        Some(r)
    }
}

impl<'a> Pe<'a> {
    /// Checks the layout the headers describe and returns the plan for
    /// loading it: alignments are powers of two with `FileAlignment <=
    /// SectionAlignment`; the headers hold the section table; sections are
    /// aligned, in ascending order, do not overlap and end within
    /// `SizeOfImage`; their file-backed bytes are inside the data.
    pub fn load_plan(&self) -> Result<LoadPlan<'a>, Error> {
        let info = self.info();
        let opt = &info.optional;
        let (sa, fa) = (opt.section_alignment, opt.file_alignment);
        if !sa.is_power_of_two() || !fa.is_power_of_two() {
            return Err(Error::BadLayout("alignment is not a power of two"));
        }
        if fa > sa {
            return Err(Error::BadLayout("FileAlignment is larger than SectionAlignment"));
        }
        if sa < PAGE_SIZE && fa != sa {
            return Err(Error::BadLayout("SectionAlignment below the page size differs from FileAlignment"));
        }
        let size_of_image = align_up(opt.size_of_image, sa).ok_or(Error::BadLayout("SizeOfImage overflows"))?;
        if size_of_image == 0 {
            return Err(Error::BadLayout("SizeOfImage is zero"));
        }

        let sections = self.sections();
        let table_end = info.nt_offset as usize
            + 4
            + FileHeader::SIZE
            + usize::from(info.file_header.size_of_optional_header)
            + sections.len() * SectionHeader::SIZE;
        let soh = opt.size_of_headers;
        if (soh as usize) < table_end {
            return Err(Error::BadLayout("SizeOfHeaders does not cover the section table"));
        }
        let data = self.data();
        if soh as usize > data.len() {
            return Err(Error::Truncated);
        }
        let mut prev_end = align_up(soh, sa).ok_or(Error::BadLayout("SizeOfHeaders overflows"))?;
        if prev_end > size_of_image {
            return Err(Error::BadLayout("headers end beyond SizeOfImage"));
        }

        for s in sections.iter() {
            let va = s.virtual_address;
            if va % sa != 0 {
                return Err(Error::BadLayout("section address is not aligned"));
            }
            if va < prev_end {
                return Err(Error::BadLayout("sections overlap or are out of order"));
            }
            let extent = align_up(s.mapped_size(), sa).ok_or(Error::BadLayout("section size overflows"))?;
            let end = va.checked_add(extent).ok_or(Error::BadLayout("section ends past 4 GiB"))?;
            if end > size_of_image {
                return Err(Error::BadLayout("section ends beyond SizeOfImage"));
            }
            let copy = s.file_backed_size() as usize;
            let src = match self.view.layout() {
                Layout::File => s.pointer_to_raw_data as usize,
                Layout::Mapped => va as usize,
            };
            if copy > 0 && src.checked_add(copy).is_none_or(|e| e > data.len()) {
                return Err(Error::Truncated);
            }
            prev_end = end;
        }

        if opt.address_of_entry_point >= size_of_image {
            return Err(Error::BadLayout("entry point outside the image"));
        }

        Ok(LoadPlan {
            image_base: opt.image_base,
            size_of_image,
            section_alignment: sa,
            file_alignment: fa,
            size_of_headers: soh,
            entry_point: opt.address_of_entry_point,
            relocatable: info.relocatable(),
            has_relocations: info.directory(dir::BASERELOC).is_some_and(|d| d.size != 0),
            view: self.view,
            sections,
        })
    }
}
