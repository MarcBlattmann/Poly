//! `_findfirst` / `_findnext` (Learn: "_findfirst, _findfirst32, ...",
//! "_findnext, ..."; `corecrt_io.h`'s `_finddata*_t` structures): a
//! `WIN32_FIND_DATAW` read and laid out as one of the eight `_finddata`
//! structures.
//!
//! The structures (`corecrt_io.h`, natural x64 alignment) differ in the
//! width of their three times (`__time32_t` / `__time64_t`), of `size`
//! (`_fsize_t`, an `unsigned long`, or `__int64`) and of `name`
//! (`char[260]` / `wchar_t[260]`):
//!
//! | Structure | times at | size at | name at | bytes (narrow / wide) |
//! |---|---|---|---|---|
//! | `_finddata32_t` | 4, 8, 12 | 16 (4) | 20 | 280 / 540 |
//! | `_finddata32i64_t` | 4, 8, 12 | 16 (8) | 24 | 288 / 544 |
//! | `_finddata64i32_t` | 8, 16, 24 | 32 (4) | 36 | 296 / 560 |
//! | `__finddata64_t` | 8, 16, 24 | 32 (8) | 40 | 304 / 560 |
//!
//! Poly's choices (to be confirmed by the spec agent): `attrib` keeps the
//! `_A_*` bits Learn lists (`_A_RDONLY`, `_A_HIDDEN`, `_A_SYSTEM`,
//! `_A_SUBDIR`, `_A_ARCH`; `_A_NORMAL` is 0) and drops the other
//! `FILE_ATTRIBUTE_*` bits; a time the file system does not keep (a zero
//! `FILETIME`) is −1 (Learn: "-1L for FAT file systems"), as is a time a
//! 32-bit field cannot hold; a 32-bit `size` keeps the low 32 bits; a name
//! longer than 259 units is cut.

use crate::unit::Unit;

/// `_A_RDONLY | _A_HIDDEN | _A_SYSTEM | _A_SUBDIR | _A_ARCH` (`corecrt_io.h`).
pub const ATTRIB_MASK: u32 = 0x01 | 0x02 | 0x04 | 0x10 | 0x20;
/// `_A_SUBDIR` (`corecrt_io.h`).
pub const A_SUBDIR: u32 = 0x10;
/// `MAX_PATH` (`minwindef.h`): the units of `name`, with its NUL.
pub const NAME_UNITS: usize = 260;
/// `sizeof(WIN32_FIND_DATAW)` (`minwinbase.h`).
pub const FIND_DATA_SIZE: usize = 592;

/// Which `_finddata` structure: the width of the times and of `size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `_finddata32_t`.
    T32S32,
    /// `_finddata32i64_t`.
    T32S64,
    /// `_finddata64i32_t`.
    T64S32,
    /// `__finddata64_t`.
    T64S64,
}

impl Layout {
    fn wide_time(self) -> bool {
        matches!(self, Layout::T64S32 | Layout::T64S64)
    }

    fn wide_size(self) -> bool {
        matches!(self, Layout::T32S64 | Layout::T64S64)
    }

    /// The offsets of the first time, of `size` and of `name`.
    fn offsets(self) -> (usize, usize, usize) {
        match self {
            Layout::T32S32 => (4, 16, 20),
            Layout::T32S64 => (4, 16, 24),
            Layout::T64S32 => (8, 32, 36),
            Layout::T64S64 => (8, 32, 40),
        }
    }

    /// `sizeof` the structure with `C` names (rounded to its alignment).
    pub fn size<C: Unit>(self) -> usize {
        let (_, _, name) = self.offsets();
        let end = name + NAME_UNITS * core::mem::size_of::<C>();
        let align = if self.wide_time() || self.wide_size() { 8 } else { 4 };
        end.div_ceil(align) * align
    }
}

/// One directory entry, as `_findfirst` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found<C> {
    pub attrib: u32,
    pub create: i64,
    pub access: i64,
    pub write: i64,
    pub size: u64,
    pub name: alloc::vec::Vec<C>,
}

/// A `FILETIME` as a `time_t`: −1 for zero (not kept).
pub fn time_of(ft: u64) -> i64 {
    if ft == 0 { -1 } else { crate::time::from_filetime(ft) }
}

/// A `WIN32_FIND_DATAW` (`minwinbase.h`: attributes at 0, the creation,
/// access and write `FILETIME`s at 4, 12, 20, `nFileSizeHigh` at 28,
/// `nFileSizeLow` at 32, `cFileName[260]` at 44) as an entry with wide
/// names.
pub fn from_find_data(raw: &[u8; FIND_DATA_SIZE]) -> Found<u16> {
    let u32_at = |o: usize| u32::from_le_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]);
    let u64_at = |o: usize| u64::from(u32_at(o)) | (u64::from(u32_at(o + 4)) << 32);
    let name: alloc::vec::Vec<u16> = raw[44..44 + 2 * NAME_UNITS]
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .take_while(|&u| u != 0)
        .collect();
    Found {
        attrib: u32_at(0) & ATTRIB_MASK,
        create: time_of(u64_at(4)),
        access: time_of(u64_at(12)),
        write: time_of(u64_at(20)),
        size: (u64::from(u32_at(28)) << 32) | u64::from(u32_at(32)),
        name,
    }
}

fn time32(t: i64) -> i32 {
    i32::try_from(t).unwrap_or(-1)
}

/// Lay `found` out as `layout` with `C` names into `out`
/// (`layout.size::<C>()` bytes).
pub fn store<C: Unit>(found: &Found<C>, layout: Layout, out: &mut [u8]) {
    let size = layout.size::<C>();
    let out = &mut out[..size];
    out.fill(0);
    out[0..4].copy_from_slice(&found.attrib.to_le_bytes());
    let (times, at_size, at_name) = layout.offsets();
    for (k, t) in [found.create, found.access, found.write].into_iter().enumerate() {
        if layout.wide_time() {
            out[times + 8 * k..times + 8 * k + 8].copy_from_slice(&t.to_le_bytes());
        } else {
            out[times + 4 * k..times + 4 * k + 4].copy_from_slice(&time32(t).to_le_bytes());
        }
    }
    if layout.wide_size() {
        out[at_size..at_size + 8].copy_from_slice(&found.size.to_le_bytes());
    } else {
        out[at_size..at_size + 4].copy_from_slice(&(found.size as u32).to_le_bytes());
    }
    let unit = core::mem::size_of::<C>();
    for (i, c) in found.name.iter().take(NAME_UNITS - 1).enumerate() {
        let v = c.to_u32();
        let at = at_name + i * unit;
        out[at] = v as u8;
        if unit == 2 {
            out[at + 1] = (v >> 8) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn sizes_follow_the_header() {
        let narrow: Vec<usize> =
            [Layout::T32S32, Layout::T32S64, Layout::T64S32, Layout::T64S64].map(|l| l.size::<u8>()).into();
        assert_eq!(narrow, [280, 288, 296, 304]);
        let wide: Vec<usize> =
            [Layout::T32S32, Layout::T32S64, Layout::T64S32, Layout::T64S64].map(|l| l.size::<u16>()).into();
        assert_eq!(wide, [540, 544, 560, 560]);
    }

    fn raw_entry() -> [u8; FIND_DATA_SIZE] {
        let mut raw = [0u8; FIND_DATA_SIZE];
        // Read-only, archive and FILE_ATTRIBUTE_NORMAL's 0x80 (dropped).
        raw[0..4].copy_from_slice(&(0x01u32 | 0x20 | 0x80).to_le_bytes());
        // 1970-01-01 00:00:10 as a FILETIME, creation; access 0 (not kept).
        let ft = (11_644_473_600u64 + 10) * 10_000_000;
        raw[4..12].copy_from_slice(&ft.to_le_bytes());
        raw[20..28].copy_from_slice(&(ft + 50_000_000).to_le_bytes());
        raw[28..32].copy_from_slice(&1u32.to_le_bytes());
        raw[32..36].copy_from_slice(&5u32.to_le_bytes());
        for (i, u) in "a\u{e9}.txt".encode_utf16().enumerate() {
            raw[44 + 2 * i..46 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        raw
    }

    #[test]
    fn find_data_is_read() {
        let f = from_find_data(&raw_entry());
        assert_eq!(f.attrib, 0x21);
        assert_eq!((f.create, f.access, f.write), (10, -1, 15));
        assert_eq!(f.size, (1 << 32) | 5);
        assert_eq!(f.name, "a\u{e9}.txt".encode_utf16().collect::<Vec<u16>>());
    }

    #[test]
    fn layouts_place_each_field() {
        let f = from_find_data(&raw_entry());
        let mut out = [0xAAu8; 600];
        store(&f, Layout::T64S64, &mut out);
        assert_eq!(&out[0..4], &0x21u32.to_le_bytes());
        assert_eq!(&out[8..16], &10i64.to_le_bytes());
        assert_eq!(&out[16..24], &(-1i64).to_le_bytes());
        assert_eq!(&out[24..32], &15i64.to_le_bytes());
        assert_eq!(&out[32..40], &((1u64 << 32) | 5).to_le_bytes());
        assert_eq!(&out[40..44], &[b'a', 0, 0xE9, 0]);
        assert_eq!(out[560], 0xAA, "nothing past the structure");

        store(&f, Layout::T32S32, &mut out);
        assert_eq!(&out[4..8], &10i32.to_le_bytes());
        assert_eq!(&out[12..16], &15i32.to_le_bytes());
        assert_eq!(&out[16..20], &5u32.to_le_bytes(), "the low 32 bits");
        assert_eq!(&out[20..22], &[b'a', 0]);

        let narrow = Found {
            attrib: f.attrib,
            create: f.create,
            access: f.access,
            write: f.write,
            size: f.size,
            name: b"x.c".to_vec(),
        };
        store(&narrow, Layout::T64S32, &mut out);
        assert_eq!(&out[32..36], &5u32.to_le_bytes());
        assert_eq!(&out[36..40], b"x.c\0");
        store(&narrow, Layout::T32S64, &mut out);
        assert_eq!(&out[16..24], &((1u64 << 32) | 5).to_le_bytes());
        assert_eq!(&out[24..28], b"x.c\0");
    }

    #[test]
    fn times_out_of_range_and_long_names() {
        let f = Found { attrib: 0, create: 1 << 40, access: 0, write: -5, size: 0, name: alloc::vec![b'n'; 400] };
        let mut out = [0u8; 304];
        store(&f, Layout::T32S32, &mut out);
        assert_eq!(&out[4..8], &(-1i32).to_le_bytes());
        assert_eq!(&out[12..16], &(-5i32).to_le_bytes());
        assert!(out[20..20 + 259].iter().all(|&b| b == b'n'));
        assert_eq!(out[20 + 259], 0);
    }
}
