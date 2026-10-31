//! A parsed TrueType / OpenType font: the table directory and the tables
//! every font has (OpenType specification, "Font file" and the `head`,
//! `hhea`, `maxp`, `hmtx`, `cmap`, `name`, `OS/2` and `post` chapters;
//! Apple's TrueType Reference Manual describes the same tables).
//!
//! Parsing checks only what is needed up front (the directory, `head`,
//! `maxp`, `hhea`); everything else is read lazily, and malformed data
//! reads as absent rather than panicking.

use crate::path::Path;
use crate::reader::{Reader, i16_at, u16_at, u32_at};
use crate::{cff, glyf, kern};
use alloc::string::String;
use core::fmt;

/// A glyph index in a font (glyph 0 is `.notdef`, the "missing" glyph).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u16);

/// Why a font could not be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The data ends inside a structure.
    Truncated,
    /// Not a font: the first four bytes are no sfnt version or `ttcf`.
    UnknownFormat,
    /// A collection has no font with this index.
    NoSuchFont,
    /// A required table is missing (its tag).
    MissingTable([u8; 4]),
    /// A table's contents make no sense (which table).
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => f.write_str("font data is truncated"),
            Error::UnknownFormat => f.write_str("not a TrueType/OpenType font"),
            Error::NoSuchFont => f.write_str("no font with that index in the collection"),
            Error::MissingTable(t) => write!(f, "missing table '{}'", String::from_utf8_lossy(t)),
            Error::Malformed(t) => write!(f, "malformed '{t}' table"),
        }
    }
}

/// `sfntVersion` of TrueType outlines (and Apple's `true`), of CFF
/// outlines (`OTTO`), and the tag of a collection.
const SFNT_TRUETYPE: u32 = 0x0001_0000;
const SFNT_TRUE: u32 = u32::from_be_bytes(*b"true");
const SFNT_OTTO: u32 = u32::from_be_bytes(*b"OTTO");
const TTC_TAG: u32 = u32::from_be_bytes(*b"ttcf");

/// The number of fonts in `data`: 1 for a plain font file, the count of a
/// TrueType/OpenType collection (`.ttc`), 0 if it is neither.
pub fn font_count(data: &[u8]) -> u32 {
    match u32_at(data, 0) {
        Some(TTC_TAG) => u32_at(data, 8).unwrap_or(0),
        Some(SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO) => 1,
        _ => 0,
    }
}

/// Which outlines a font has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineFormat {
    /// Quadratic outlines in `glyf`/`loca`.
    TrueType,
    /// Cubic Type 2 charstrings in `CFF `.
    Cff,
    /// Neither (a bitmap-only or broken font): no outlines.
    None,
}

/// Font-wide vertical metrics, in font units (`hhea`, or `OS/2`'s typo
/// metrics when its `USE_TYPO_METRICS` bit asks for them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VMetrics {
    pub units_per_em: u16,
    /// Distance from the baseline up to the top of the line (positive).
    pub ascender: i16,
    /// Distance from the baseline down to the bottom (negative).
    pub descender: i16,
    pub line_gap: i16,
}

impl VMetrics {
    /// Baseline-to-baseline distance in font units.
    pub fn line_height(&self) -> i32 {
        i32::from(self.ascender) - i32::from(self.descender) + i32::from(self.line_gap)
    }
}

/// A glyph's horizontal metrics, in font units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HMetrics {
    pub advance: u16,
    pub left_side_bearing: i16,
}

/// What `OS/2` and `post` say about a font's style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StyleInfo {
    /// `usWeightClass` (100 thin ... 400 regular ... 900 black).
    pub weight_class: u16,
    /// `usWidthClass` (1 ultra-condensed ... 5 normal ... 9 ultra-expanded).
    pub width_class: u16,
    /// `fsSelection` bit 0 (or `head.macStyle` bit 1 without `OS/2`).
    pub italic: bool,
    /// `fsSelection` bit 9.
    pub oblique: bool,
    /// `fsSelection` bit 5 (or `head.macStyle` bit 0).
    pub bold: bool,
    /// `post.isFixedPitch` is non-zero.
    pub monospace: bool,
    /// `post.italicAngle` in degrees (negative leans right).
    pub italic_angle: f32,
    /// `OS/2.achVendID`.
    pub vendor: [u8; 4],
}

/// Name IDs of the `name` table used here.
pub mod name_id {
    pub const COPYRIGHT: u16 = 0;
    pub const FAMILY: u16 = 1;
    pub const SUBFAMILY: u16 = 2;
    pub const UNIQUE_ID: u16 = 3;
    pub const FULL_NAME: u16 = 4;
    pub const VERSION: u16 = 5;
    pub const POSTSCRIPT_NAME: u16 = 6;
    pub const TYPOGRAPHIC_FAMILY: u16 = 16;
    pub const TYPOGRAPHIC_SUBFAMILY: u16 = 17;
}

/// A font: borrowed data plus the offsets parsing found.
#[derive(Clone, Copy)]
pub struct Font<'a> {
    data: &'a [u8],
    tables: Tables<'a>,
    units_per_em: u16,
    long_loca: bool,
    num_glyphs: u16,
    num_hmetrics: u16,
    head_bbox: [i16; 4],
    mac_style: u16,
    cmap: Option<Cmap<'a>>,
}

impl fmt::Debug for Font<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("units_per_em", &self.units_per_em)
            .field("num_glyphs", &self.num_glyphs)
            .field("outlines", &self.outline_format())
            .finish_non_exhaustive()
    }
}

/// The tables this crate reads, each a slice of the font data.
#[derive(Clone, Copy, Default)]
pub(crate) struct Tables<'a> {
    pub head: &'a [u8],
    pub hhea: &'a [u8],
    pub maxp: &'a [u8],
    pub hmtx: Option<&'a [u8]>,
    pub cmap: Option<&'a [u8]>,
    pub loca: Option<&'a [u8]>,
    pub glyf: Option<&'a [u8]>,
    pub cff: Option<&'a [u8]>,
    pub name: Option<&'a [u8]>,
    pub os2: Option<&'a [u8]>,
    pub post: Option<&'a [u8]>,
    pub kern: Option<&'a [u8]>,
    pub gpos: Option<&'a [u8]>,
}

impl<'a> Font<'a> {
    /// The (first) font in `data`.
    pub fn parse(data: &'a [u8]) -> Result<Font<'a>, Error> {
        Font::parse_index(data, 0)
    }

    /// Font `index` of a collection (index 0 of a plain font file).
    pub fn parse_index(data: &'a [u8], index: u32) -> Result<Font<'a>, Error> {
        let tag = u32_at(data, 0).ok_or(Error::Truncated)?;
        let offset = match tag {
            TTC_TAG => {
                let count = u32_at(data, 8).ok_or(Error::Truncated)?;
                if index >= count {
                    return Err(Error::NoSuchFont);
                }
                u32_at(data, 12 + 4 * index as usize).ok_or(Error::Truncated)? as usize
            }
            SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO if index == 0 => 0,
            SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO => return Err(Error::NoSuchFont),
            _ => return Err(Error::UnknownFormat),
        };
        match u32_at(data, offset) {
            Some(SFNT_TRUETYPE | SFNT_TRUE | SFNT_OTTO) => {}
            Some(_) => return Err(Error::UnknownFormat),
            None => return Err(Error::Truncated),
        }
        let num_tables = u16_at(data, offset + 4).ok_or(Error::Truncated)?;
        let mut r = Reader::at(data, offset + 12).ok_or(Error::Truncated)?;
        let mut head = None;
        let mut hhea = None;
        let mut maxp = None;
        let mut t = Tables::default();
        for _ in 0..num_tables {
            let tag = r.bytes(4).ok_or(Error::Truncated)?;
            let _checksum = r.u32().ok_or(Error::Truncated)?;
            let off = r.u32().ok_or(Error::Truncated)? as usize;
            let len = r.u32().ok_or(Error::Truncated)? as usize;
            // A table running past the end of the file is treated as absent.
            let Some(slice) = off.checked_add(len).and_then(|end| data.get(off..end)) else { continue };
            match tag {
                b"head" => head = Some(slice),
                b"hhea" => hhea = Some(slice),
                b"maxp" => maxp = Some(slice),
                b"hmtx" => t.hmtx = Some(slice),
                b"cmap" => t.cmap = Some(slice),
                b"loca" => t.loca = Some(slice),
                b"glyf" => t.glyf = Some(slice),
                b"CFF " => t.cff = Some(slice),
                b"name" => t.name = Some(slice),
                b"OS/2" => t.os2 = Some(slice),
                b"post" => t.post = Some(slice),
                b"kern" => t.kern = Some(slice),
                b"GPOS" => t.gpos = Some(slice),
                _ => {}
            }
        }
        t.head = head.ok_or(Error::MissingTable(*b"head"))?;
        t.hhea = hhea.ok_or(Error::MissingTable(*b"hhea"))?;
        t.maxp = maxp.ok_or(Error::MissingTable(*b"maxp"))?;
        // head: magicNumber at 12, unitsPerEm at 18, the box at 36..44,
        // macStyle at 44, indexToLocFormat at 50.
        if t.head.len() < 54 || u32_at(t.head, 12) != Some(0x5F0F_3CF5) {
            return Err(Error::Malformed("head"));
        }
        let units_per_em = u16_at(t.head, 18).unwrap_or(0);
        if !(16..=16384).contains(&units_per_em) {
            return Err(Error::Malformed("head"));
        }
        let head_bbox = [
            i16_at(t.head, 36).unwrap_or(0),
            i16_at(t.head, 38).unwrap_or(0),
            i16_at(t.head, 40).unwrap_or(0),
            i16_at(t.head, 42).unwrap_or(0),
        ];
        let mac_style = u16_at(t.head, 44).unwrap_or(0);
        let long_loca = i16_at(t.head, 50) == Some(1);
        let num_glyphs = u16_at(t.maxp, 4).ok_or(Error::Malformed("maxp"))?;
        let num_hmetrics = u16_at(t.hhea, 34).ok_or(Error::Malformed("hhea"))?;
        let cmap = t.cmap.and_then(Cmap::pick);
        Ok(Font { data, tables: t, units_per_em, long_loca, num_glyphs, num_hmetrics, head_bbox, mac_style, cmap })
    }

    /// The whole font file this font was parsed from.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub(crate) fn tables(&self) -> &Tables<'a> {
        &self.tables
    }

    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    pub(crate) fn long_loca(&self) -> bool {
        self.long_loca
    }

    /// The font-wide bounding box from `head`: x_min, y_min, x_max, y_max.
    pub fn bounding_box(&self) -> [i16; 4] {
        self.head_bbox
    }

    pub fn outline_format(&self) -> OutlineFormat {
        if self.tables.glyf.is_some() && self.tables.loca.is_some() {
            OutlineFormat::TrueType
        } else if self.tables.cff.is_some() {
            OutlineFormat::Cff
        } else {
            OutlineFormat::None
        }
    }

    /// Font-wide vertical metrics.
    pub fn vmetrics(&self) -> VMetrics {
        let h = self.tables.hhea;
        let mut m = VMetrics {
            units_per_em: self.units_per_em,
            ascender: i16_at(h, 4).unwrap_or(0),
            descender: i16_at(h, 6).unwrap_or(0),
            line_gap: i16_at(h, 8).unwrap_or(0),
        };
        // OS/2 fsSelection bit 7, USE_TYPO_METRICS (sTypo* at 68, 70, 72).
        if let Some(os2) = self.tables.os2
            && u16_at(os2, 62).is_some_and(|s| s & (1 << 7) != 0)
            && let (Some(a), Some(d), Some(g)) = (i16_at(os2, 68), i16_at(os2, 70), i16_at(os2, 72))
        {
            m = VMetrics { units_per_em: self.units_per_em, ascender: a, descender: d, line_gap: g };
        }
        if m.ascender == 0 && m.descender == 0 {
            m.ascender = self.head_bbox[3];
            m.descender = self.head_bbox[1];
        }
        m
    }

    /// The glyph for a character, or `None` if the font has none.
    pub fn glyph_index(&self, c: char) -> Option<GlyphId> {
        let g = self.cmap.as_ref()?.lookup(u32::from(c))?;
        (g != 0 && g < self.num_glyphs).then_some(GlyphId(g))
    }

    /// Whether the font maps `c` to a glyph.
    pub fn has_char(&self, c: char) -> bool {
        self.glyph_index(c).is_some()
    }

    /// A glyph's advance and left side bearing (`hmtx`: the last advance
    /// repeats for glyphs past `numberOfHMetrics`).
    pub fn hmetrics(&self, glyph: GlyphId) -> HMetrics {
        let Some(hmtx) = self.tables.hmtx else { return HMetrics::default() };
        let n = usize::from(self.num_hmetrics);
        let g = usize::from(glyph.0);
        if n == 0 {
            return HMetrics::default();
        }
        if g < n {
            HMetrics {
                advance: u16_at(hmtx, 4 * g).unwrap_or(0),
                left_side_bearing: i16_at(hmtx, 4 * g + 2).unwrap_or(0),
            }
        } else {
            HMetrics {
                advance: u16_at(hmtx, 4 * (n - 1)).unwrap_or(0),
                left_side_bearing: i16_at(hmtx, 4 * n + 2 * (g - n)).unwrap_or(0),
            }
        }
    }

    /// A glyph's advance width in font units.
    pub fn advance(&self, glyph: GlyphId) -> u16 {
        self.hmetrics(glyph).advance
    }

    /// A glyph's outline in font units (y up), or `None` for a glyph with
    /// none (a space) or one that cannot be read.
    pub fn outline(&self, glyph: GlyphId) -> Option<Path> {
        if glyph.0 >= self.num_glyphs {
            return None;
        }
        let path = match self.outline_format() {
            OutlineFormat::TrueType => glyf::outline(self, glyph)?,
            OutlineFormat::Cff => cff::outline(self.tables.cff?, glyph)?,
            OutlineFormat::None => return None,
        };
        (!path.is_empty()).then_some(path)
    }

    /// The horizontal kerning between two glyphs in font units (`GPOS`
    /// pair adjustment of the `kern` feature, else the `kern` table).
    pub fn kerning(&self, left: GlyphId, right: GlyphId) -> i16 {
        if let Some(gpos) = self.tables.gpos
            && let Some(k) = kern::gpos_pair(gpos, left, right)
        {
            return k;
        }
        self.tables.kern.and_then(|k| kern::kern_pair(k, left, right)).unwrap_or(0)
    }

    /// A string of the `name` table: US English Windows names first, then
    /// any Windows or Unicode name, then a Macintosh Roman one.
    pub fn name(&self, id: u16) -> Option<String> {
        let data = self.tables.name?;
        let count = u16_at(data, 2)?;
        let storage = usize::from(u16_at(data, 4)?);
        let mut best: Option<(u8, u16, u16, usize, usize)> = None;
        for i in 0..usize::from(count) {
            let mut r = Reader::at(data, 6 + 12 * i)?;
            let (platform, encoding, language, name) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
            let (len, off) = (usize::from(r.u16()?), usize::from(r.u16()?));
            if name != id {
                continue;
            }
            let rank = match (platform, encoding, language) {
                (3, 1 | 10, 0x0409) => 0,
                (3, 1 | 10, _) | (0, _, _) => 1,
                (1, 0, 0) => 2,
                _ => continue,
            };
            if best.is_none_or(|b| rank < b.0) {
                best = Some((rank, platform, encoding, storage + off, len));
            }
        }
        let (_, platform, _, start, len) = best?;
        let bytes = data.get(start..start.checked_add(len)?)?;
        Some(if platform == 1 { decode_mac_roman(bytes) } else { decode_utf16be(bytes) })
    }

    /// The family name (typographic family, name ID 16, if present, else 1).
    pub fn family_name(&self) -> Option<String> {
        self.name(name_id::TYPOGRAPHIC_FAMILY).or_else(|| self.name(name_id::FAMILY))
    }

    /// The style name (name ID 17 if present, else 2), e.g. "Bold".
    pub fn style_name(&self) -> Option<String> {
        self.name(name_id::TYPOGRAPHIC_SUBFAMILY).or_else(|| self.name(name_id::SUBFAMILY))
    }

    /// Weight, width, slant and pitch from `OS/2`, `post` and `head`.
    pub fn style(&self) -> StyleInfo {
        let mut s = StyleInfo {
            weight_class: if self.mac_style & 1 != 0 { 700 } else { 400 },
            width_class: 5,
            bold: self.mac_style & 1 != 0,
            italic: self.mac_style & 2 != 0,
            ..StyleInfo::default()
        };
        if let Some(os2) = self.tables.os2 {
            if let Some(w) = u16_at(os2, 4).filter(|w| (1..=1000).contains(w)) {
                s.weight_class = w;
            }
            if let Some(w) = u16_at(os2, 6).filter(|w| (1..=9).contains(w)) {
                s.width_class = w;
            }
            if let Some(sel) = u16_at(os2, 62) {
                s.italic = sel & 1 != 0;
                s.bold = sel & (1 << 5) != 0;
                s.oblique = sel & (1 << 9) != 0;
            }
            if let Some(v) = os2.get(58..62) {
                s.vendor.copy_from_slice(v);
            }
        }
        if let Some(post) = self.tables.post {
            s.italic_angle = Reader::at(post, 4).and_then(|mut r| r.fixed()).unwrap_or(0.0);
            s.monospace = u32_at(post, 12).is_some_and(|v| v != 0);
        }
        s
    }
}

fn decode_utf16be(bytes: &[u8]) -> String {
    let units = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]));
    char::decode_utf16(units).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

/// Macintosh Roman: ASCII as is; the upper half (accented letters and
/// symbols) is not needed for the names fonts use, so it reads as U+FFFD.
fn decode_mac_roman(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b < 0x80 { char::from(b) } else { char::REPLACEMENT_CHARACTER }).collect()
}

/// The `cmap` subtable used for lookups.
#[derive(Clone, Copy)]
pub(crate) struct Cmap<'a> {
    format: u16,
    data: &'a [u8],
}

impl<'a> Cmap<'a> {
    /// The best Unicode subtable: a full-repertoire one (Windows UCS-4,
    /// Unicode full) before a BMP one (Windows Unicode BMP, Unicode BMP),
    /// before the Macintosh Roman one; only formats 0, 4, 6 and 12.
    fn pick(cmap: &'a [u8]) -> Option<Cmap<'a>> {
        let count = u16_at(cmap, 2)?;
        let mut best: Option<(u8, Cmap<'a>)> = None;
        for i in 0..usize::from(count) {
            let mut r = Reader::at(cmap, 4 + 8 * i)?;
            let (platform, encoding, offset) = (r.u16()?, r.u16()?, r.u32()? as usize);
            let rank = match (platform, encoding) {
                (3, 10) | (0, 4) | (0, 6) => 0,
                (3, 1) | (0, 0..=3) => 1,
                (1, 0) => 2,
                _ => continue,
            };
            let Some(data) = cmap.get(offset..) else { continue };
            let Some(format) = u16_at(data, 0) else { continue };
            if !matches!(format, 0 | 4 | 6 | 12) {
                continue;
            }
            if best.as_ref().is_none_or(|b| rank < b.0) {
                best = Some((rank, Cmap { format, data }));
            }
        }
        best.map(|b| b.1)
    }

    fn lookup(&self, c: u32) -> Option<u16> {
        let d = self.data;
        match self.format {
            0 => d.get(6 + usize::try_from(c).ok().filter(|&c| c < 256)?).map(|&g| u16::from(g)),
            4 => {
                let c = u16::try_from(c).ok()?;
                let seg_x2 = usize::from(u16_at(d, 6)?);
                let seg_count = seg_x2 / 2;
                let ends = 14;
                let starts = ends + seg_x2 + 2;
                let deltas = starts + seg_x2;
                let ranges = deltas + seg_x2;
                // The first segment whose end is >= c (ends are sorted).
                let (mut lo, mut hi) = (0usize, seg_count);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if u16_at(d, ends + 2 * mid)? < c { lo = mid + 1 } else { hi = mid }
                }
                if lo >= seg_count {
                    return None;
                }
                let start = u16_at(d, starts + 2 * lo)?;
                if c < start {
                    return None;
                }
                let delta = u16_at(d, deltas + 2 * lo)?;
                let range_at = ranges + 2 * lo;
                let range = usize::from(u16_at(d, range_at)?);
                if range == 0 {
                    Some(c.wrapping_add(delta))
                } else {
                    let g = u16_at(d, range_at + range + 2 * usize::from(c - start))?;
                    (g != 0).then(|| g.wrapping_add(delta))
                }
            }
            6 => {
                let first = u32::from(u16_at(d, 6)?);
                let count = u32::from(u16_at(d, 8)?);
                let i = c.checked_sub(first).filter(|&i| i < count)?;
                u16_at(d, 10 + 2 * i as usize)
            }
            12 => {
                let groups = u32_at(d, 12)? as usize;
                let (mut lo, mut hi) = (0usize, groups);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let at = 16 + 12 * mid;
                    let (start, end) = (u32_at(d, at)?, u32_at(d, at + 4)?);
                    if c < start {
                        hi = mid;
                    } else if c > end {
                        lo = mid + 1;
                    } else {
                        let g = u32_at(d, at + 8)?.checked_add(c - start)?;
                        return u16::try_from(g).ok();
                    }
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfont;

    #[test]
    fn corrupted_bytes_never_panic() {
        // Every byte of the test font flipped (and set to 0xff, and to a
        // small value) in turn: parsing, every lookup, outline, kerning and
        // a small rendering must not panic.
        let data = testfont::build();
        for i in 0..data.len() {
            for v in [data[i] ^ 0xff, 0xff, 0x02] {
                let mut bad = data.clone();
                bad[i] = v;
                let Ok(f) = Font::parse(&bad) else { continue };
                let _ = (f.vmetrics(), f.style(), f.family_name(), f.style_name());
                for c in ['A', 'O', ' ', 'Á', '😀', 'z'] {
                    let _ = f.glyph_index(c);
                }
                for g in 0..8 {
                    let g = GlyphId(g);
                    let _ = (f.outline(g), f.hmetrics(g), f.kerning(g, GlyphId(2)));
                    let _ = crate::render::render_glyph(&f, g, 12.0, 0.5, crate::render::RenderMode::Gray);
                }
            }
        }
    }

    #[test]
    fn rejects_what_is_not_a_font() {
        assert_eq!(Font::parse(&[]).unwrap_err(), Error::Truncated);
        assert_eq!(Font::parse(b"hello world, not a font").unwrap_err(), Error::UnknownFormat);
        assert_eq!(font_count(b"nope"), 0);
        let data = testfont::build();
        assert_eq!(font_count(&data), 1);
        assert_eq!(Font::parse_index(&data, 1).unwrap_err(), Error::NoSuchFont);
        // Every truncation of a valid font fails cleanly or parses.
        for len in 0..data.len() {
            if let Ok(f) = Font::parse(&data[..len]) {
                for c in ['A', 'B', 'O', ' ', 'Á'] {
                    if let Some(g) = f.glyph_index(c) {
                        let _ = f.outline(g);
                        let _ = f.hmetrics(g);
                    }
                }
                let _ = f.name(name_id::FAMILY);
            }
        }
    }

    #[test]
    fn reads_the_test_font() {
        let data = testfont::build();
        let f = Font::parse(&data).unwrap();
        assert_eq!(f.units_per_em(), 1000);
        assert_eq!(f.outline_format(), OutlineFormat::TrueType);
        assert_eq!(f.vmetrics(), VMetrics { units_per_em: 1000, ascender: 800, descender: -200, line_gap: 100 });
        assert_eq!(f.vmetrics().line_height(), 1100);
        assert_eq!(f.glyph_index('A'), Some(GlyphId(1)));
        assert_eq!(f.glyph_index('O'), Some(GlyphId(2)));
        assert_eq!(f.glyph_index('Á'), Some(GlyphId(3)));
        assert_eq!(f.glyph_index(' '), Some(GlyphId(4)));
        assert_eq!(f.glyph_index('😀'), Some(GlyphId(2)), "format 12 maps beyond the BMP");
        assert_eq!(f.glyph_index('z'), None);
        assert_eq!(f.hmetrics(GlyphId(1)), HMetrics { advance: 600, left_side_bearing: 50 });
        assert_eq!(f.hmetrics(GlyphId(4)), HMetrics { advance: 250, left_side_bearing: 0 });
        assert_eq!(f.advance(GlyphId(5)), 250, "glyphs past numberOfHMetrics reuse the last advance");
        assert_eq!(f.family_name().as_deref(), Some("Poly Test"));
        assert_eq!(f.style_name().as_deref(), Some("Regular"));
        assert_eq!(f.name(name_id::FULL_NAME).as_deref(), Some("Poly Test Regular"));
        let s = f.style();
        assert_eq!((s.weight_class, s.width_class, s.italic, s.monospace), (400, 5, false, false));
        assert_eq!(f.kerning(GlyphId(1), GlyphId(2)), -40);
        assert_eq!(f.kerning(GlyphId(2), GlyphId(1)), 0);
    }

    #[test]
    fn cmap_format_4_segments() {
        let data = testfont::cmap_format4();
        let cmap = Cmap { format: 4, data: &data };
        // Mapped by idDelta (even segments) and through the glyph array (odd).
        assert_eq!(cmap.lookup(0x20), Some(4));
        assert_eq!(cmap.lookup(0x41), Some(1));
        assert_eq!(cmap.lookup(0x4f), Some(2));
        assert_eq!(cmap.lookup(0xc1), Some(3));
        assert_eq!(cmap.lookup(0x42), None);
        assert_eq!(cmap.lookup(0xffff), Some(0));
        assert_eq!(cmap.lookup(0x1f600), None);
        for len in 0..data.len() {
            let _ = Cmap { format: 4, data: &data[..len] }.lookup(0xc1);
        }
    }

    #[test]
    fn collections_and_styles() {
        let bold = testfont::build_styled(&testfont::Spec {
            style: "Bold",
            weight: 700,
            fs_selection: 0x20,
            ..testfont::Spec::default()
        });
        let ttc = testfont::collection(&[testfont::build(), bold]);
        assert_eq!(font_count(&ttc), 2);
        let f = Font::parse_index(&ttc, 1).unwrap();
        assert_eq!(f.style_name().as_deref(), Some("Bold"));
        let s = f.style();
        assert!(s.bold && !s.italic);
        assert_eq!(s.weight_class, 700);
        assert_eq!(&s.vendor, b"POLY");
        assert!(f.outline(GlyphId(1)).is_some());
        assert_eq!(Font::parse_index(&ttc, 0).unwrap().style_name().as_deref(), Some("Regular"));
        assert_eq!(Font::parse_index(&ttc, 2).unwrap_err(), Error::NoSuchFont);
    }
}
