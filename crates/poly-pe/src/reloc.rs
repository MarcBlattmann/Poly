//! Base relocations (spec "The .reloc Section (Image Only)"): blocks of
//! 16-bit entries, each a 4-bit type and a 12-bit offset into the block's
//! 4 KiB page.

use crate::headers::{Bits, ImageInfo, OptionalHeader, Pe};
use crate::read::{put_u16, put_u32, put_u64, u16_at, u32_at, u64_at};
use crate::{Error, dir, machine, reloc_kind};

/// Size of a block header: page RVA and block size.
const BLOCK_HEADER: usize = 8;

/// One relocation: what to fix and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reloc {
    /// `IMAGE_REL_BASED_*`, see [`crate::reloc_kind`].
    pub kind: u8,
    pub rva: u32,
}

/// One block: relocations for one 4 KiB page.
#[derive(Clone, Copy, Debug)]
pub struct RelocBlock<'a> {
    pub page_rva: u32,
    entries: &'a [u8],
}

impl<'a> RelocBlock<'a> {
    pub fn len(&self) -> usize {
        self.entries.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn entries(&self) -> RelocEntries<'a> {
        RelocEntries { page_rva: self.page_rva, bytes: self.entries, pos: 0 }
    }
}

/// Iterator over a block's [`Reloc`]s (including `ABSOLUTE` padding).
#[derive(Clone, Debug)]
pub struct RelocEntries<'a> {
    page_rva: u32,
    bytes: &'a [u8],
    pos: usize,
}

impl Iterator for RelocEntries<'_> {
    type Item = Reloc;

    fn next(&mut self) -> Option<Reloc> {
        let e = u16_at(self.bytes, self.pos)?;
        self.pos += 2;
        Some(decode(self.page_rva, e))
    }
}

fn decode(page_rva: u32, entry: u16) -> Reloc {
    // `page_rva` is checked to leave room for the 12-bit offset.
    Reloc { kind: (entry >> 12) as u8, rva: page_rva.wrapping_add(u32::from(entry & 0xfff)) }
}

/// A block header found at `pos` in a relocation table.
struct BlockAt {
    page_rva: u32,
    entries: usize,
    count: usize,
    next: usize,
}

/// Reads the block at `pos`. `None` at the end of the table (fewer bytes
/// left than a block header, which covers padding).
fn block_at(table: &[u8], pos: usize) -> Result<Option<BlockAt>, Error> {
    if table.len().saturating_sub(pos) < BLOCK_HEADER {
        return Ok(None);
    }
    let page_rva = u32_at(table, pos).ok_or(Error::Truncated)?;
    let size = u32_at(table, pos + 4).ok_or(Error::Truncated)? as usize;
    if size < BLOCK_HEADER {
        return Err(Error::Malformed("relocation block smaller than its header"));
    }
    if size > table.len() - pos {
        return Err(Error::Truncated);
    }
    if page_rva > u32::MAX - 0xfff {
        return Err(Error::Malformed("relocation page past 4 GiB"));
    }
    Ok(Some(BlockAt { page_rva, entries: pos + BLOCK_HEADER, count: (size - BLOCK_HEADER) / 2, next: pos + size }))
}

/// Iterator over the blocks of a relocation table. It stops after the
/// first error.
#[derive(Clone, Debug)]
pub struct RelocBlocks<'a> {
    table: &'a [u8],
    pos: usize,
}

impl<'a> RelocBlocks<'a> {
    /// Iterates over a relocation table's bytes.
    pub fn new(table: &'a [u8]) -> Self {
        RelocBlocks { table, pos: 0 }
    }
}

impl<'a> Iterator for RelocBlocks<'a> {
    type Item = Result<RelocBlock<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match block_at(self.table, self.pos) {
            Ok(Some(b)) => {
                self.pos = b.next;
                Some(Ok(RelocBlock { page_rva: b.page_rva, entries: &self.table[b.entries..b.entries + b.count * 2] }))
            }
            Ok(None) => None,
            Err(e) => {
                self.pos = self.table.len();
                Some(Err(e))
            }
        }
    }
}

impl<'a> Pe<'a> {
    /// The base relocation blocks, if the image has a relocation directory.
    pub fn relocations(&self) -> Result<Option<RelocBlocks<'a>>, Error> {
        let Some(d) = self.info().directory(dir::BASERELOC).filter(|d| d.size != 0) else { return Ok(None) };
        Ok(Some(RelocBlocks::new(self.view.slice(d.virtual_address, d.size as usize)?)))
    }
}

/// What [`relocate`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelocStats {
    /// Fix-ups written.
    pub applied: usize,
    /// `ABSOLUTE` entries (padding) skipped.
    pub skipped: usize,
}

/// Bytes a relocation of `kind` changes, if this machine uses it.
fn width(machine: u16, kind: u8) -> Option<usize> {
    match (machine, kind) {
        (_, reloc_kind::ABSOLUTE) => Some(0),
        (machine::AMD64 | machine::ARM64 | machine::I386, reloc_kind::HIGHLOW) => Some(4),
        (machine::AMD64 | machine::ARM64, reloc_kind::DIR64) => Some(8),
        (machine::I386, reloc_kind::HIGH | reloc_kind::LOW) => Some(2),
        _ => None,
    }
}

/// Moves a mapped image from its header's `ImageBase` to `new_base`: adds
/// the difference to every location the relocation table names and stores
/// `new_base` in the header's `ImageBase`, so a later
/// [`Pe::parse_mapped`] of the image sees the new base.
///
/// `image` is laid out in memory (see [`crate::LoadPlan::map_into`]); the
/// relocation table is read from it. Supported types: `ABSOLUTE` (skipped),
/// `HIGHLOW` and `DIR64` for x64 and ARM64, and `ABSOLUTE`, `HIGH`, `LOW`,
/// `HIGHLOW` for x86. Any other type fails the call with
/// [`Error::UnsupportedRelocation`] before anything is written: the table
/// is checked in full first. Additions wrap, as on the hardware.
pub fn relocate(image: &mut [u8], info: &ImageInfo, new_base: u64) -> Result<RelocStats, Error> {
    let delta = new_base.wrapping_sub(info.image_base());
    if delta == 0 {
        return Ok(RelocStats::default());
    }
    if info.relocs_stripped() {
        return Err(Error::RelocsStripped);
    }
    let machine = info.machine();
    if !matches!(machine, machine::AMD64 | machine::ARM64 | machine::I386) {
        return Err(Error::UnsupportedMachine(machine));
    }
    if info.bits() == Bits::Pe32 && new_base > u64::from(u32::MAX) {
        return Err(Error::BadVa(new_base));
    }
    let base_field = (info.nt_offset as usize)
        .checked_add(4 + crate::headers::FileHeader::SIZE + OptionalHeader::image_base_offset(info.bits()))
        .ok_or(Error::Truncated)?;
    let base_width = info.bits().pointer_size();
    if base_field.checked_add(base_width).is_none_or(|e| e > image.len()) {
        return Err(Error::Truncated);
    }

    let mut stats = RelocStats::default();
    if let Some(d) = info.directory(dir::BASERELOC).filter(|d| d.size != 0) {
        let start = d.virtual_address as usize;
        let end = start.checked_add(d.size as usize).ok_or(Error::BadRva(d.virtual_address))?;
        if end > image.len() {
            return Err(Error::BadRva(d.virtual_address));
        }
        // Pass 1: every entry is a type we apply, inside the image.
        for block in RelocBlocks::new(&image[start..end]) {
            for r in block?.entries() {
                check(image.len(), machine, r)?;
            }
        }
        // Pass 2: apply. The table is read back from `image` as we go (an
        // entry could patch the table itself), so everything is checked
        // again; that can only fail on such a self-modifying table.
        let mut pos = start;
        while let Some(b) = block_at(&image[..end], pos)? {
            for i in 0..b.count {
                let entry = u16_at(image, b.entries + 2 * i).ok_or(Error::Truncated)?;
                let r = decode(b.page_rva, entry);
                let at = r.rva as usize;
                let ok = match (check(image.len(), machine, r)?, r.kind) {
                    (0, _) => {
                        stats.skipped += 1;
                        continue;
                    }
                    (_, reloc_kind::DIR64) => u64_at(image, at).and_then(|v| put_u64(image, at, v.wrapping_add(delta))),
                    (_, reloc_kind::HIGHLOW) => {
                        u32_at(image, at).and_then(|v| put_u32(image, at, v.wrapping_add(delta as u32)))
                    }
                    (_, reloc_kind::HIGH) => {
                        u16_at(image, at).and_then(|v| put_u16(image, at, v.wrapping_add((delta >> 16) as u16)))
                    }
                    _ => u16_at(image, at).and_then(|v| put_u16(image, at, v.wrapping_add(delta as u16))),
                };
                ok.ok_or(Error::BadRva(r.rva))?;
                stats.applied += 1;
            }
            pos = b.next;
        }
    }

    let stored = match info.bits() {
        Bits::Pe32Plus => put_u64(image, base_field, new_base),
        Bits::Pe32 => put_u32(image, base_field, new_base as u32),
    };
    stored.ok_or(Error::Truncated)?;
    Ok(stats)
}

/// Checks one relocation and returns how many bytes it changes.
fn check(image_len: usize, machine: u16, r: Reloc) -> Result<usize, Error> {
    let w = width(machine, r.kind).ok_or(Error::UnsupportedRelocation { kind: r.kind, rva: r.rva })?;
    if w > 0 && (r.rva as usize).checked_add(w).is_none_or(|e| e > image_len) {
        return Err(Error::BadRva(r.rva));
    }
    Ok(w)
}
