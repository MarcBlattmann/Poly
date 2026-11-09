//! Chained fixups (`LC_DYLD_CHAINED_FIXUPS`).
//!
//! Layouts and constants are those of `<mach-o/fixup-chains.h>`. The
//! payload of the load command is, in this order of reading:
//!
//! 1. `dyld_chained_fixups_header`: version, and offsets of the segment
//!    starts, the imports table and the symbol strings.
//! 2. `dyld_chained_starts_in_image`: a segment count and one offset per
//!    segment to its `dyld_chained_starts_in_segment` (0: no fixups there).
//! 3. Per segment: page size, pointer format, where the segment starts in
//!    memory and, for each page, the offset in that page of the first
//!    fixup, or [`DYLD_CHAINED_PTR_START_NONE`].
//! 4. The imports: one record per imported symbol (library ordinal, weak
//!    flag, name offset, maybe an addend), and the names.
//!
//! The fixups themselves live in the segments' file bytes: at each page
//! start the pointer-sized slot holds a bit field that says whether it is a
//! rebase or a bind and how far the next fixup of the chain is, in units of
//! the format's stride. [`ChainedFixups::fixups`] walks every chain.
//!
//! A walk is bounded by construction: the next fixup is always at a greater
//! offset in the same page, and inside the segment's file bytes, or the
//! walk ends with an error.

use crate::bytes::{cstr, slice, u16_le, u32_le, u64_le};
use crate::consts::*;
use crate::error::{Error, Result};
use crate::image::SegmentInfo;
use crate::symtab::MAX_NAME_LEN;

/// `struct dyld_chained_fixups_header`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainedHeader {
    /// Always 0.
    pub fixups_version: u32,
    /// Offset of `dyld_chained_starts_in_image` in the payload.
    pub starts_offset: u32,
    /// Offset of the imports table in the payload.
    pub imports_offset: u32,
    /// Offset of the symbol strings in the payload.
    pub symbols_offset: u32,
    /// The number of imports.
    pub imports_count: u32,
    /// `DYLD_CHAINED_IMPORT*`.
    pub imports_format: u32,
    /// 0: strings are plain; 1: zlib compressed (not supported).
    pub symbols_format: u32,
}

/// An imported symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Import<'s> {
    /// The library: a 1-based ordinal into the dependency list, or one of
    /// `BIND_SPECIAL_DYLIB_*` (0 self, -1 main executable, -2 flat lookup,
    /// -3 weak lookup).
    pub lib_ordinal: i32,
    /// Whether the symbol may be missing.
    pub weak_import: bool,
    /// The symbol name.
    pub name: &'s [u8],
    /// A value added to the symbol's address (0 in the plain format).
    pub addend: i64,
}

/// `dyld_chained_starts_in_segment`: how the fixups of one segment start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartsInSegment<'s> {
    /// The size of the record, page starts included.
    pub size: u32,
    /// 0x1000 or 0x4000 (the only sizes the header lists).
    pub page_size: u16,
    /// `DYLD_CHAINED_PTR_*`.
    pub pointer_format: u16,
    /// Where the segment starts in memory, from the image's Mach-O header.
    pub segment_offset: u64,
    /// For 32-bit formats: values above this are not pointers.
    pub max_valid_pointer: u32,
    /// The number of pages.
    pub page_count: u16,
    page_starts: &'s [u8],
}

impl StartsInSegment<'_> {
    /// The offset in page `page` of its first fixup, or
    /// [`DYLD_CHAINED_PTR_START_NONE`].
    pub fn page_start(&self, page: u16) -> Option<u16> {
        u16_le(self.page_starts, usize::from(page) * 2)
    }

    /// The distance between two possible fixups in bytes, for the
    /// formats this crate walks.
    pub fn stride(&self) -> Result<u64> {
        stride(self.pointer_format)
    }
}

/// The distance between two possible fixups, in bytes.
fn stride(format: u16) -> Result<u64> {
    match format {
        DYLD_CHAINED_PTR_ARM64E | DYLD_CHAINED_PTR_ARM64E_USERLAND | DYLD_CHAINED_PTR_ARM64E_USERLAND24 => Ok(8),
        DYLD_CHAINED_PTR_64 | DYLD_CHAINED_PTR_64_OFFSET | DYLD_CHAINED_PTR_ARM64E_KERNEL => Ok(4),
        other => Err(Error::UnsupportedPointerFormat(other)),
    }
}

/// An arm64e pointer authentication signature of a fixup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtrAuth {
    /// The key: 0 IA, 1 IB, 2 DA, 3 DB.
    pub key: u8,
    /// The 16-bit constant mixed into the signature.
    pub diversity: u16,
    /// Whether the address of the pointer is mixed in too.
    pub addr_div: bool,
}

/// A pointer to relocate by the image's slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainedRebase {
    /// The target. A vm address if `target_is_vmaddr`, else an offset from
    /// the image's Mach-O header (the "runtime offset").
    pub target: u64,
    /// Which of the two `target` is.
    pub target_is_vmaddr: bool,
    /// The top 8 bits of the pointer, set after the slide is added.
    pub high8: u8,
    /// The signature, for an authenticated arm64e pointer.
    pub auth: Option<PtrAuth>,
}

/// A pointer to fill with an imported symbol's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainedBind {
    /// The index into the imports table
    /// ([`ChainedFixups::import`]).
    pub ordinal: u32,
    /// An addend carried in the pointer itself (arm64e formats), added to
    /// the import's own addend.
    pub addend: i64,
    /// The signature, for an authenticated arm64e pointer.
    pub auth: Option<PtrAuth>,
}

/// What a fixup does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixupKind {
    /// Add the image's slide.
    Rebase(ChainedRebase),
    /// Store the address of an import.
    Bind(ChainedBind),
}

/// One fixup found by walking a chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixup {
    /// The index of the segment (counting `LC_SEGMENT_64` commands).
    pub segment: u8,
    /// The page of the segment the fixup is in.
    pub page: u16,
    /// The file offset of the pointer slot.
    pub file_offset: u64,
    /// The memory offset of the slot from the image's Mach-O header.
    pub vm_offset: u64,
    /// The slot's bits as stored in the file.
    pub raw: u64,
    /// The `DYLD_CHAINED_PTR_*` format of the segment.
    pub pointer_format: u16,
    /// What the fixup does.
    pub kind: FixupKind,
}

/// The chained fixups of an image, borrowed from the file.
#[derive(Clone, Copy, Debug)]
pub struct ChainedFixups<'s> {
    file: &'s [u8],
    payload: &'s [u8],
    segments: &'s [SegmentInfo],
    header: ChainedHeader,
    seg_count: u32,
}

impl<'s> ChainedFixups<'s> {
    pub(crate) fn new(file: &'s [u8], payload: &'s [u8], segments: &'s [SegmentInfo]) -> Result<Self> {
        let w = |i: usize| u32_le(payload, i * 4).ok_or(Error::Truncated);
        let header = ChainedHeader {
            fixups_version: w(0)?,
            starts_offset: w(1)?,
            imports_offset: w(2)?,
            symbols_offset: w(3)?,
            imports_count: w(4)?,
            imports_format: w(5)?,
            symbols_format: w(6)?,
        };
        if header.fixups_version != 0 {
            return Err(Error::UnsupportedFixupsVersion);
        }
        if header.symbols_format != 0 {
            return Err(Error::CompressedSymbols);
        }
        let entry = import_size(header.imports_format)?;
        // The imports table and the table of segment starts must fit.
        slice(payload, u64::from(header.imports_offset), u64::from(header.imports_count) * entry as u64)
            .ok_or(Error::Truncated)?;
        let seg_count = u32_le(payload, header.starts_offset as usize).ok_or(Error::Truncated)?;
        slice(payload, u64::from(header.starts_offset) + 4, u64::from(seg_count) * 4).ok_or(Error::Truncated)?;
        if header.symbols_offset as usize > payload.len() {
            return Err(Error::Truncated);
        }
        Ok(ChainedFixups { file, payload, segments, header, seg_count })
    }

    /// The header of the payload.
    pub const fn header(&self) -> &ChainedHeader {
        &self.header
    }

    /// The number of entries of the segment table (normally the number of
    /// segments of the image).
    pub const fn segment_count(&self) -> u32 {
        self.seg_count
    }

    /// The number of imports.
    pub const fn import_count(&self) -> u32 {
        self.header.imports_count
    }

    /// How the fixups of segment `index` start, or `None` if it has none.
    pub fn starts_in_segment(&self, index: u32) -> Result<Option<StartsInSegment<'s>>> {
        if index >= self.seg_count {
            return Err(Error::BadSegmentIndex);
        }
        let entry = u64::from(self.header.starts_offset) + 4 + u64::from(index) * 4;
        let rel = u32_le(self.payload, entry as usize).ok_or(Error::Truncated)?;
        if rel == 0 {
            return Ok(None);
        }
        let at = u64::from(self.header.starts_offset) + u64::from(rel);
        let fixed = slice(self.payload, at, CHAINED_STARTS_IN_SEGMENT_SIZE as u64).ok_or(Error::BadStartsInSegment)?;
        let size = u32_le(fixed, 0).ok_or(Error::BadStartsInSegment)?;
        let page_size = u16_le(fixed, 4).ok_or(Error::BadStartsInSegment)?;
        let pointer_format = u16_le(fixed, 6).ok_or(Error::BadStartsInSegment)?;
        let segment_offset = u64_le(fixed, 8).ok_or(Error::BadStartsInSegment)?;
        let max_valid_pointer = u32_le(fixed, 16).ok_or(Error::BadStartsInSegment)?;
        let page_count = u16_le(fixed, 20).ok_or(Error::BadStartsInSegment)?;
        if !matches!(page_size, 0x1000 | 0x4000) {
            return Err(Error::BadPageSize);
        }
        let starts_len = u64::from(page_count) * 2;
        if u64::from(size) < CHAINED_STARTS_IN_SEGMENT_SIZE as u64 + starts_len {
            return Err(Error::BadStartsInSegment);
        }
        let page_starts = slice(self.payload, at + CHAINED_STARTS_IN_SEGMENT_SIZE as u64, starts_len)
            .ok_or(Error::BadStartsInSegment)?;
        Ok(Some(StartsInSegment {
            size,
            page_size,
            pointer_format,
            segment_offset,
            max_valid_pointer,
            page_count,
            page_starts,
        }))
    }

    /// The import at `index`.
    pub fn import(&self, index: u32) -> Result<Import<'s>> {
        if index >= self.header.imports_count {
            return Err(Error::ImportOutOfRange);
        }
        let size = import_size(self.header.imports_format)?;
        let at = u64::from(self.header.imports_offset) + u64::from(index) * size as u64;
        let rec = slice(self.payload, at, size as u64).ok_or(Error::Truncated)?;
        let (lib_ordinal, weak_import, name_offset, addend) = match self.header.imports_format {
            DYLD_CHAINED_IMPORT | DYLD_CHAINED_IMPORT_ADDEND => {
                let v = u32_le(rec, 0).ok_or(Error::BadImport)?;
                let raw = v & 0xff;
                let ordinal = if raw > 0xf0 { i32::from(raw as u8 as i8) } else { raw as i32 };
                let addend = if size == 8 { i64::from(u32_le(rec, 4).ok_or(Error::BadImport)? as i32) } else { 0 };
                (ordinal, (v >> 8) & 1 != 0, u64::from(v >> 9), addend)
            }
            _ => {
                let v = u64_le(rec, 0).ok_or(Error::BadImport)?;
                let raw = (v & 0xffff) as u16;
                let ordinal = if raw > 0xfff0 { i32::from(raw as i16) } else { i32::from(raw) };
                let addend = u64_le(rec, 8).ok_or(Error::BadImport)? as i64;
                (ordinal, (v >> 16) & 1 != 0, v >> 32, addend)
            }
        };
        let name_at = u64::from(self.header.symbols_offset).checked_add(name_offset).ok_or(Error::BadImport)?;
        let name_at = usize::try_from(name_at).map_err(|_| Error::BadImport)?;
        let name = cstr(self.payload, name_at, MAX_NAME_LEN).ok_or(Error::BadImport)?;
        Ok(Import { lib_ordinal, weak_import, name, addend })
    }

    /// All imports, in table order.
    pub fn imports(&self) -> Imports<'s> {
        Imports { fixups: *self, next: 0 }
    }

    /// Walks every chain of every segment and yields the fixups in segment,
    /// page and chain order.
    pub fn fixups(&self) -> Fixups<'s> {
        Fixups { fixups: *self, next_segment: 0, walk: None, done: false }
    }
}

fn import_size(format: u32) -> Result<usize> {
    match format {
        DYLD_CHAINED_IMPORT => Ok(4),
        DYLD_CHAINED_IMPORT_ADDEND => Ok(8),
        DYLD_CHAINED_IMPORT_ADDEND64 => Ok(16),
        _ => Err(Error::UnsupportedImportsFormat),
    }
}

/// Iterator over the imports of a [`ChainedFixups`].
#[derive(Clone, Debug)]
pub struct Imports<'s> {
    fixups: ChainedFixups<'s>,
    next: u32,
}

impl<'s> Iterator for Imports<'s> {
    type Item = Result<Import<'s>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.fixups.header.imports_count {
            return None;
        }
        let r = self.fixups.import(self.next);
        // After an error the rest of the table is not trusted.
        self.next = if r.is_err() { self.fixups.header.imports_count } else { self.next + 1 };
        Some(r)
    }
}

/// Where a walk is inside one segment.
#[derive(Clone, Copy, Debug)]
struct SegWalk<'s> {
    index: u8,
    seg: SegmentInfo,
    starts: StartsInSegment<'s>,
    stride: u64,
    /// The next page to look at.
    next_page: u32,
    /// The page and the offset in it of the next fixup of the current chain.
    chain: Option<(u16, u64)>,
}

/// Iterator over the fixups of all chains; see [`ChainedFixups::fixups`].
#[derive(Clone, Debug)]
pub struct Fixups<'s> {
    fixups: ChainedFixups<'s>,
    next_segment: u32,
    walk: Option<SegWalk<'s>>,
    done: bool,
}

fn bits(v: u64, lo: u32, width: u32) -> u64 {
    (v >> lo) & ((1u64 << width) - 1)
}

fn sign_extend(v: u64, width: u32) -> i64 {
    let shift = 64 - width;
    ((v << shift) as i64) >> shift
}

/// The pointer authentication fields shared by the arm64e bit layouts.
fn ptr_auth(raw: u64) -> PtrAuth {
    PtrAuth { key: bits(raw, 49, 2) as u8, diversity: bits(raw, 32, 16) as u16, addr_div: bits(raw, 48, 1) != 0 }
}

/// Splits a slot into the number of strides to the next fixup and what the
/// fixup does.
fn decode(format: u16, raw: u64) -> Result<(u64, FixupKind)> {
    match format {
        DYLD_CHAINED_PTR_ARM64E
        | DYLD_CHAINED_PTR_ARM64E_KERNEL
        | DYLD_CHAINED_PTR_ARM64E_USERLAND
        | DYLD_CHAINED_PTR_ARM64E_USERLAND24 => {
            let next = bits(raw, 51, 11);
            let is_bind = bits(raw, 62, 1) != 0;
            let is_auth = bits(raw, 63, 1) != 0;
            let auth = is_auth.then(|| ptr_auth(raw));
            let kind = if is_bind {
                let ordinal =
                    if format == DYLD_CHAINED_PTR_ARM64E_USERLAND24 { bits(raw, 0, 24) } else { bits(raw, 0, 16) }
                        as u32;
                // Authenticated binds carry no addend: its bits are the
                // signature's diversity.
                let addend = if is_auth { 0 } else { sign_extend(bits(raw, 32, 19), 19) };
                FixupKind::Bind(ChainedBind { ordinal, addend, auth })
            } else if is_auth {
                // The target is a 32-bit offset from the image's header.
                FixupKind::Rebase(ChainedRebase { target: bits(raw, 0, 32), target_is_vmaddr: false, high8: 0, auth })
            } else {
                FixupKind::Rebase(ChainedRebase {
                    target: bits(raw, 0, 43),
                    target_is_vmaddr: format == DYLD_CHAINED_PTR_ARM64E,
                    high8: bits(raw, 43, 8) as u8,
                    auth: None,
                })
            };
            Ok((next, kind))
        }
        DYLD_CHAINED_PTR_64 | DYLD_CHAINED_PTR_64_OFFSET => {
            let next = bits(raw, 51, 12);
            let kind = if bits(raw, 63, 1) != 0 {
                FixupKind::Bind(ChainedBind {
                    ordinal: bits(raw, 0, 24) as u32,
                    addend: bits(raw, 24, 8) as i64,
                    auth: None,
                })
            } else {
                FixupKind::Rebase(ChainedRebase {
                    target: bits(raw, 0, 36),
                    target_is_vmaddr: format == DYLD_CHAINED_PTR_64,
                    high8: bits(raw, 36, 8) as u8,
                    auth: None,
                })
            };
            Ok((next, kind))
        }
        other => Err(Error::UnsupportedPointerFormat(other)),
    }
}

impl<'s> Fixups<'s> {
    /// Opens the walk of the next segment that has fixups.
    fn open_next_segment(&mut self) -> Result<bool> {
        while self.next_segment < self.fixups.seg_count {
            let index = self.next_segment;
            self.next_segment += 1;
            let Some(starts) = self.fixups.starts_in_segment(index)? else {
                continue;
            };
            let stride = starts.stride()?;
            let seg = *self.fixups.segments.get(index as usize).ok_or(Error::BadSegmentIndex)?;
            self.walk = Some(SegWalk {
                index: u8::try_from(index).map_err(|_| Error::BadSegmentIndex)?,
                seg,
                starts,
                stride,
                next_page: 0,
                chain: None,
            });
            return Ok(true);
        }
        Ok(false)
    }

    fn step(&mut self) -> Result<Option<Fixup>> {
        loop {
            let Some(w) = self.walk.as_mut() else {
                if self.open_next_segment()? {
                    continue;
                }
                return Ok(None);
            };
            let page_size = u64::from(w.starts.page_size);
            if let Some((page, pos)) = w.chain {
                let in_segment = u64::from(page) * page_size + pos;
                let file_offset = w.seg.fileoff.checked_add(in_segment).ok_or(Error::ChainEscapesPage)?;
                let raw = file_offset
                    .checked_add(8)
                    .filter(|&end| end <= w.seg.fileoff + w.seg.filesize)
                    .and_then(|_| usize::try_from(file_offset).ok())
                    .and_then(|o| u64_le(self.fixups.file, o))
                    .ok_or(Error::ChainEscapesPage)?;
                let (next, kind) = decode(w.starts.pointer_format, raw)?;
                w.chain = if next == 0 {
                    None
                } else {
                    // `next` is at most 12 bits: no overflow.
                    let np = pos + next * w.stride;
                    if np + 8 > page_size {
                        return Err(Error::ChainEscapesPage);
                    }
                    Some((page, np))
                };
                return Ok(Some(Fixup {
                    segment: w.index,
                    page,
                    file_offset,
                    vm_offset: w.starts.segment_offset.checked_add(in_segment).ok_or(Error::ChainEscapesPage)?,
                    raw,
                    pointer_format: w.starts.pointer_format,
                    kind,
                }));
            }
            // Find the next page that has a chain.
            let mut found = None;
            while w.next_page < u32::from(w.starts.page_count) {
                let page = w.next_page as u16;
                w.next_page += 1;
                let start = w.starts.page_start(page).ok_or(Error::BadPageStart)?;
                if start == DYLD_CHAINED_PTR_START_NONE {
                    continue;
                }
                // The multiple-starts flag exists for 32-bit formats only.
                if start & DYLD_CHAINED_PTR_START_MULTI != 0 {
                    return Err(Error::BadPageStart);
                }
                let start = u64::from(start);
                if start + 8 > page_size {
                    return Err(Error::BadPageStart);
                }
                found = Some((page, start));
                break;
            }
            match found {
                Some(c) => w.chain = Some(c),
                None => self.walk = None,
            }
        }
    }
}

impl Iterator for Fixups<'_> {
    type Item = Result<Fixup>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(f)) => Some(Ok(f)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}
