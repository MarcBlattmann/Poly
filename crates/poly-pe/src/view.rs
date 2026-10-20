//! Reading an image by RVA, whether it is a file on disk or already mapped.

use crate::Error;
use crate::headers::SectionTable;
use crate::read::{cstr, ptr_at, u16_at, u32_at};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// The file as stored: sections at their `PointerToRawData`.
    File,
    /// Laid out in memory: every RVA is an offset.
    Mapped,
}

/// Translates RVAs into the bytes of a file or a mapped image.
#[derive(Clone, Copy, Debug)]
pub(crate) struct View<'a> {
    data: &'a [u8],
    layout: Layout,
    /// End of the headers in a file: `SizeOfHeaders`, but never past the
    /// first section.
    headers_end: u32,
    sections: SectionTable<'a>,
}

impl<'a> View<'a> {
    pub(crate) fn new(data: &'a [u8], layout: Layout, size_of_headers: u32, sections: SectionTable<'a>) -> Self {
        let first = sections.iter().map(|s| s.virtual_address).min().unwrap_or(u32::MAX);
        View { data, layout, headers_end: size_of_headers.min(first), sections }
    }

    /// A mapped image with nothing known about its sections.
    pub(crate) fn mapped(data: &'a [u8]) -> Self {
        View { data, layout: Layout::Mapped, headers_end: 0, sections: SectionTable::default() }
    }

    pub(crate) fn data(&self) -> &'a [u8] {
        self.data
    }

    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }

    /// The offset of `rva` in the data and the end of the contiguous region
    /// it is in (the end of the data, the headers or the section's
    /// file-backed bytes).
    pub(crate) fn offset(&self, rva: u32) -> Result<(usize, usize), Error> {
        let len = self.data.len();
        let (off, end) = match self.layout {
            Layout::Mapped => (rva as usize, len),
            Layout::File if rva < self.headers_end => (rva as usize, (self.headers_end as usize).min(len)),
            Layout::File => {
                let s = self
                    .sections
                    .iter()
                    .find(|s| rva >= s.virtual_address && rva - s.virtual_address < s.file_backed_size())
                    .ok_or(Error::BadRva(rva))?;
                let start = s.pointer_to_raw_data as usize;
                let off = start.checked_add((rva - s.virtual_address) as usize).ok_or(Error::BadRva(rva))?;
                let end = start.saturating_add(s.file_backed_size() as usize).min(len);
                (off, end)
            }
        };
        if off >= end {
            return Err(Error::BadRva(rva));
        }
        Ok((off, end))
    }

    /// Everything from `rva` to the end of its region.
    pub(crate) fn rest(&self, rva: u32) -> Result<&'a [u8], Error> {
        let (off, end) = self.offset(rva)?;
        Ok(&self.data[off..end])
    }

    pub(crate) fn slice(&self, rva: u32, len: usize) -> Result<&'a [u8], Error> {
        self.rest(rva)?.get(..len).ok_or(Error::Truncated)
    }

    pub(crate) fn cstr(&self, rva: u32) -> Result<&'a [u8], Error> {
        cstr(self.rest(rva)?, 0)
    }

    pub(crate) fn u16(&self, rva: u32) -> Result<u16, Error> {
        u16_at(self.rest(rva)?, 0).ok_or(Error::Truncated)
    }

    pub(crate) fn u32(&self, rva: u32) -> Result<u32, Error> {
        u32_at(self.rest(rva)?, 0).ok_or(Error::Truncated)
    }

    pub(crate) fn ptr(&self, rva: u32, wide: bool) -> Result<u64, Error> {
        ptr_at(self.rest(rva)?, 0, wide).ok_or(Error::Truncated)
    }
}

/// `base + index * stride` as an RVA, or an error on overflow.
pub(crate) fn rva_add(base: u32, index: usize, stride: usize) -> Result<u32, Error> {
    index
        .checked_mul(stride)
        .and_then(|d| u32::try_from(d).ok())
        .and_then(|d| base.checked_add(d))
        .ok_or(Error::Malformed("table runs past 4 GiB"))
}
