//! What DirectWrite knows about one font of a file: its weight, stretch and
//! style, names, metrics, character map and glyph metrics. Sources: the
//! OpenType specification (`head`, `hhea`, `hmtx`, `maxp`, `cmap`, `glyf`,
//! `loca`, `name`, `OS/2`, `post`) and Microsoft Learn (the
//! `DWRITE_FONT_METRICS`, `DWRITE_GLYPH_METRICS` and `DWRITE_FONT_WEIGHT`
//! reference pages); behaviour as recorded in the spec sheets of
//! `specs/windows/dwrite.dll`.

use alloc::vec::Vec;

use crate::names::{Localized, NameRecord, parse_names};
use crate::sfnt::{Sfnt, be16, be16s, be32};

/// `DWRITE_FONT_STYLE` (`dwrite.h`).
pub const STYLE_NORMAL: u32 = 0;
pub const STYLE_OBLIQUE: u32 = 1;
pub const STYLE_ITALIC: u32 = 2;

/// `DWRITE_FONT_SIMULATIONS` (`dwrite.h`).
pub const SIM_BOLD: u32 = 1;
pub const SIM_OBLIQUE: u32 = 2;

/// `DWRITE_FONT_METRICS` (`dwrite.h`), in design units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub design_units_per_em: u16,
    pub ascent: u16,
    pub descent: u16,
    pub line_gap: i16,
    pub cap_height: u16,
    pub x_height: u16,
    pub underline_position: i16,
    pub underline_thickness: u16,
    pub strikethrough_position: i16,
    pub strikethrough_thickness: u16,
}

/// `DWRITE_GLYPH_METRICS` (`dwrite.h`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub left_side_bearing: i32,
    pub advance_width: u32,
    pub right_side_bearing: i32,
    pub top_side_bearing: i32,
    pub advance_height: u32,
    pub bottom_side_bearing: i32,
    pub vertical_origin_y: i32,
}

/// Everything read from a font's tables once.
#[derive(Clone, Debug)]
pub struct FaceInfo {
    pub names: Vec<NameRecord>,
    pub weight: u32,
    pub stretch: u32,
    pub style: u32,
    pub is_symbol: bool,
    pub num_glyphs: u16,
    pub metrics: Metrics,
    /// The family names the font groups under: the WWS family (name ID 21),
    /// else the typographic one (16), else name ID 1.
    pub family: Localized,
    /// The face names: WWS subfamily (22), else typographic (17), else 2.
    pub face: Localized,
    pub typo_ascender: i32,
    pub typo_descender: i32,
    /// The extras of `DWRITE_FONT_METRICS1`.
    pub extra: Metrics1,
    /// `hhea` caret slope rise and run and offset.
    pub caret: [i16; 3],
}

/// The fields `DWRITE_FONT_METRICS1` adds to `DWRITE_FONT_METRICS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics1 {
    /// Left, top (the highest point), right, bottom of the font's box.
    pub glyph_box: [i16; 4],
    /// Subscript position x, y and size x, y.
    pub subscript: [i16; 4],
    pub superscript: [i16; 4],
    pub has_typographic_metrics: bool,
}

impl FaceInfo {
    /// Read font `index` of `data`; `None` when it lacks `head` or `maxp`
    /// or they are unusable.
    pub fn parse(data: &[u8], index: u32) -> Option<FaceInfo> {
        let sf = Sfnt::parse(data, index)?;
        let head = sf.table(b"head")?;
        let maxp = sf.table(b"maxp")?;
        if head.len() < 54 || be32(head, 12)? != 0x5F0F_3CF5 {
            return None;
        }
        let upem = be16(head, 18)?;
        if upem == 0 {
            return None;
        }
        let num_glyphs = be16(maxp, 4)?;
        let mac_style = be16(head, 44).unwrap_or(0);
        let names = sf.table(b"name").map(parse_names).unwrap_or_default();
        let hhea = sf.table(b"hhea");
        let os2 = sf.table(b"OS/2");
        let post = sf.table(b"post");

        let fs_selection = os2.and_then(|t| be16(t, 62)).unwrap_or(0);
        let (italic_bit, bold_bit) = match os2 {
            Some(_) => (fs_selection & 1 != 0, fs_selection & 0x20 != 0),
            None => (mac_style & 2 != 0, mac_style & 1 != 0),
        };
        let class = os2.and_then(|t| be16(t, 4)).map(u32::from).unwrap_or(if bold_bit { 700 } else { 400 });
        let mut weight = match class {
            0 => 1,
            1..=9 => class * 100,
            1000.. => 999,
            c => c,
        };
        if bold_bit && weight < 700 {
            weight = 700;
        }
        let stretch = match os2.and_then(|t| be16(t, 6)) {
            None | Some(0) => 5,
            Some(w @ 1..=9) => u32::from(w),
            Some(_) => 9,
        };

        let family = pick(&names, &[21, 16, 1]);
        let face = pick(&names, &[22, 17, 2]);
        let oblique_name =
            [2u16, 17, 22].iter().any(|&id| names.iter().any(|r| r.name_id == id && contains_ci(&r.text, "oblique")));
        let style = if italic_bit {
            STYLE_ITALIC
        } else if oblique_name || fs_selection & 0x200 != 0 {
            STYLE_OBLIQUE
        } else {
            STYLE_NORMAL
        };

        // Metrics.
        let hhea_asc = hhea.and_then(|t| be16s(t, 4)).map(i32::from).unwrap_or(0);
        let hhea_desc = hhea.and_then(|t| be16s(t, 6)).map(i32::from).unwrap_or(0);
        let hhea_gap = hhea.and_then(|t| be16s(t, 8)).map(i32::from).unwrap_or(0);
        let (typo_asc, typo_desc, typo_gap) = match os2 {
            Some(t) => (
                be16s(t, 68).map(i32::from).unwrap_or(hhea_asc),
                be16s(t, 70).map(i32::from).unwrap_or(hhea_desc),
                be16s(t, 72).map(i32::from).unwrap_or(0),
            ),
            None => (hhea_asc, hhea_desc, hhea_gap),
        };
        let (ascent, descent, line_gap) = match os2 {
            Some(_) if fs_selection & 0x80 != 0 => (typo_asc, -typo_desc, typo_gap.max(0)),
            Some(t) => {
                let wa = be16(t, 74).map(i32::from).unwrap_or(hhea_asc);
                let wd = be16(t, 76).map(i32::from).unwrap_or(-hhea_desc);
                (wa, wd, (hhea_asc - hhea_desc + hhea_gap - (wa + wd)).max(0))
            }
            None => (hhea_asc, -hhea_desc, hhea_gap.max(0)),
        };
        let ul_pos = post.and_then(|t| be16s(t, 8)).unwrap_or(0);
        let ul_thick = post.and_then(|t| be16s(t, 10)).unwrap_or(0).max(0) as u16;
        let (st_pos, mut st_thick) = match os2 {
            Some(t) => (be16s(t, 28).unwrap_or(0), be16s(t, 26).unwrap_or(0).max(0) as u16),
            None => (0, 0),
        };
        if st_thick == 0 {
            st_thick = ul_thick;
        }
        let os2_version = os2.and_then(|t| be16(t, 0)).unwrap_or(0);
        let (mut x_height, mut cap_height) = match os2 {
            Some(t) if os2_version >= 2 => {
                (be16s(t, 86).unwrap_or(0).max(0) as u16, be16s(t, 88).unwrap_or(0).max(0) as u16)
            }
            _ => (0, 0),
        };
        if cap_height == 0 {
            cap_height = (u32::from(upem) * 7 / 10) as u16;
        }
        if x_height == 0 {
            x_height = (u32::from(upem) / 2) as u16;
        }
        let metrics = Metrics {
            design_units_per_em: upem,
            ascent: ascent.clamp(0, 0xFFFF) as u16,
            descent: descent.clamp(0, 0xFFFF) as u16,
            line_gap: line_gap.clamp(-0x8000, 0x7FFF) as i16,
            cap_height,
            x_height,
            underline_position: ul_pos,
            underline_thickness: ul_thick,
            strikethrough_position: st_pos,
            strikethrough_thickness: st_thick,
        };
        let cmap = Cmap::of(&sf);
        Some(FaceInfo {
            names,
            weight,
            stretch,
            style,
            is_symbol: cmap.is_symbol,
            num_glyphs,
            metrics,
            family,
            face,
            typo_ascender: typo_asc,
            typo_descender: typo_desc,
            extra: Metrics1 {
                glyph_box: [
                    be16s(head, 36).unwrap_or(0),
                    be16s(head, 42).unwrap_or(0),
                    be16s(head, 40).unwrap_or(0),
                    be16s(head, 38).unwrap_or(0),
                ],
                // OS/2: subscript size x,y then offset x,y; superscript likewise.
                subscript: os2.map_or([0; 4], |t| {
                    [be16s(t, 14), be16s(t, 16), be16s(t, 10), be16s(t, 12)].map(|v| v.unwrap_or(0))
                }),
                superscript: os2.map_or([0; 4], |t| {
                    [be16s(t, 22), be16s(t, 24), be16s(t, 18), be16s(t, 20)].map(|v| v.unwrap_or(0))
                }),
                has_typographic_metrics: os2.is_some() && fs_selection & 0x80 != 0,
            },
            caret: hhea.map_or([0, 1, 0], |t| {
                [be16s(t, 18).unwrap_or(1), be16s(t, 20).unwrap_or(0), be16s(t, 22).unwrap_or(0)]
            }),
        })
    }

    /// A name table entry by DirectWrite's informational string id
    /// (`DWRITE_INFORMATIONAL_STRING_ID`): the strings, or `None`.
    pub fn informational(&self, id: u32) -> Option<Localized> {
        let name_id = match id {
            1 => 0,
            2 => 5,
            3 => 7,
            4 => 8,
            5 => 9,
            6 => 12,
            7 => 10,
            8 => 11,
            9 => 13,
            10 => 14,
            11 => 1,
            12 => 2,
            13 => 16,
            14 => 17,
            15 => 19,
            16 => 4,
            17 => 6,
            18 => 20,
            19 => 21,
            _ => return None,
        };
        let l = Localized::from_records(&self.names, name_id);
        (!l.is_empty()).then_some(l)
    }
}

/// The first of `ids` the names have, as localized strings.
fn pick(names: &[NameRecord], ids: &[u16]) -> Localized {
    for &id in ids {
        let l = Localized::from_records(names, id);
        if !l.is_empty() {
            return l;
        }
    }
    Localized::default()
}

fn contains_ci(text: &[u16], needle: &str) -> bool {
    let n = needle.as_bytes();
    if n.len() > text.len() {
        return false;
    }
    (0..=text.len() - n.len())
        .any(|s| n.iter().enumerate().all(|(i, &c)| crate::names::lower(text[s + i]) == u16::from(c)))
}

/// A `cmap` table: the subtable DirectWrite would use.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cmap<'a> {
    sub: &'a [u8],
    /// Whether the (3, 0) symbol subtable is the one used.
    pub is_symbol: bool,
}

impl<'a> Cmap<'a> {
    /// Pick a subtable: a full Unicode one (3,10 / 0,4 / 0,6), else the BMP
    /// one (3,1 / 0,3 / 0,2 / 0,1 / 0,0), else the symbol one (3,0).
    pub fn of(sf: &Sfnt<'a>) -> Cmap<'a> {
        let Some(t) = sf.table(b"cmap") else { return Cmap::default() };
        let n = usize::from(be16(t, 2).unwrap_or(0));
        let mut best: Option<(u8, &'a [u8], bool)> = None;
        for i in 0..n {
            let (Some(p), Some(e), Some(off)) = (be16(t, 4 + 8 * i), be16(t, 6 + 8 * i), be32(t, 8 + 8 * i)) else {
                break;
            };
            let Some(sub) = t.get(off as usize..) else { continue };
            let rank = match (p, e) {
                (3, 10) => 6,
                (0, 4) | (0, 6) => 5,
                (3, 1) => 4,
                (0, 3) | (0, 2) | (0, 1) | (0, 0) => 3,
                (3, 0) => 2,
                _ => continue,
            };
            if best.is_none_or(|b| rank > b.0) {
                best = Some((rank, sub, p == 3 && e == 0));
            }
        }
        match best {
            Some((_, sub, sym)) => Cmap { sub, is_symbol: sym },
            None => Cmap::default(),
        }
    }

    /// The glyph of a code point; 0 when the font has none.
    pub fn glyph(&self, cp: u32) -> u16 {
        if cp > 0x10FFFF || (0xD800..=0xDFFF).contains(&cp) {
            return 0;
        }
        let g = self.lookup(cp);
        if g != 0 || !self.is_symbol {
            return g;
        }
        if (0xF000..=0xF0FF).contains(&cp) {
            self.lookup(cp - 0xF000)
        } else if cp <= 0xFF {
            self.lookup(cp + 0xF000)
        } else {
            0
        }
    }

    /// The code point ranges the character map covers, merged and sorted
    /// (the segments of a format 4 table, the groups of a format 12 one).
    pub fn ranges(&self) -> Vec<(u32, u32)> {
        let s = self.sub;
        let mut out: Vec<(u32, u32)> = Vec::new();
        match be16(s, 0) {
            Some(4) => {
                let segx2 = usize::from(be16(s, 6).unwrap_or(0));
                for i in 0..segx2 / 2 {
                    let end = be16(s, 14 + 2 * i).unwrap_or(0);
                    let start = be16(s, 16 + segx2 + 2 * i).unwrap_or(1);
                    if start <= end && !(start == 0xFFFF && end == 0xFFFF) {
                        out.push((u32::from(start), u32::from(end)));
                    }
                }
            }
            Some(12) => {
                for i in 0..be32(s, 12).unwrap_or(0) as usize {
                    if let (Some(a), Some(b)) = (be32(s, 16 + 12 * i), be32(s, 20 + 12 * i))
                        && a <= b
                    {
                        out.push((a, b));
                    }
                }
            }
            _ => {}
        }
        out.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (a, b) in out {
            match merged.last_mut() {
                Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        merged
    }

    fn lookup(&self, cp: u32) -> u16 {
        let s = self.sub;
        match be16(s, 0) {
            Some(0) => {
                if cp < 256 {
                    s.get(6 + cp as usize).copied().map(u16::from).unwrap_or(0)
                } else {
                    0
                }
            }
            Some(4) => {
                if cp > 0xFFFF {
                    return 0;
                }
                let cp = cp as u16;
                let segx2 = usize::from(be16(s, 6).unwrap_or(0));
                let (ends, starts, deltas, ranges) = (14, 16 + segx2, 16 + 2 * segx2, 16 + 3 * segx2);
                let n = segx2 / 2;
                // Binary search on endCode.
                let (mut lo, mut hi) = (0usize, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if be16(s, ends + 2 * mid).unwrap_or(0xFFFF) < cp {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo >= n {
                    return 0;
                }
                let start = be16(s, starts + 2 * lo).unwrap_or(0xFFFF);
                if start > cp {
                    return 0;
                }
                let delta = be16(s, deltas + 2 * lo).unwrap_or(0);
                let ro = be16(s, ranges + 2 * lo).unwrap_or(0);
                if ro == 0 {
                    return cp.wrapping_add(delta);
                }
                let at = ranges + 2 * lo + usize::from(ro) + 2 * usize::from(cp - start);
                match be16(s, at) {
                    Some(0) | None => 0,
                    Some(g) => g.wrapping_add(delta),
                }
            }
            Some(6) => {
                let first = u32::from(be16(s, 6).unwrap_or(0));
                let count = u32::from(be16(s, 8).unwrap_or(0));
                if cp >= first && cp < first + count { be16(s, 10 + 2 * (cp - first) as usize).unwrap_or(0) } else { 0 }
            }
            Some(12) => {
                let n = be32(s, 12).unwrap_or(0) as usize;
                let (mut lo, mut hi) = (0usize, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let end = be32(s, 16 + 12 * mid + 4).unwrap_or(u32::MAX);
                    if end < cp {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                if lo >= n {
                    return 0;
                }
                let start = be32(s, 16 + 12 * lo).unwrap_or(u32::MAX);
                if start > cp {
                    return 0;
                }
                let g = be32(s, 16 + 12 * lo + 8).unwrap_or(0).wrapping_add(cp - start);
                if g > 0xFFFF { 0 } else { g as u16 }
            }
            _ => 0,
        }
    }
}

/// Advance width of `glyph` from `hmtx`.
pub fn advance(sf: &Sfnt<'_>, glyph: u16) -> u16 {
    let (Some(hhea), Some(hmtx)) = (sf.table(b"hhea"), sf.table(b"hmtx")) else { return 0 };
    let n = be16(hhea, 34).unwrap_or(0);
    if n == 0 {
        return 0;
    }
    let g = glyph.min(n - 1);
    be16(hmtx, 4 * usize::from(g)).unwrap_or(0)
}

/// The bounding box of a TrueType glyph from its `glyf` header (xMin,
/// yMin, xMax, yMax); `None` for a glyph without outline.
pub fn glyph_box(sf: &Sfnt<'_>, glyph: u16) -> Option<[i32; 4]> {
    let (head, loca, glyf) = (sf.table(b"head")?, sf.table(b"loca")?, sf.table(b"glyf")?);
    let long = be16s(head, 50)? == 1;
    let g = usize::from(glyph);
    let (a, b) = if long {
        (be32(loca, 4 * g)? as usize, be32(loca, 4 * g + 4)? as usize)
    } else {
        (usize::from(be16(loca, 2 * g)?) * 2, usize::from(be16(loca, 2 * g + 2)?) * 2)
    };
    if b <= a || b - a < 10 {
        return None;
    }
    let h = glyf.get(a..a + 10)?;
    let n = i16::from_be_bytes([h[0], h[1]]);
    if n == 0 {
        return None;
    }
    let v = |i: usize| i32::from(i16::from_be_bytes([h[i], h[i + 1]]));
    Some([v(2), v(4), v(6), v(8)])
}

/// `DWRITE_GLYPH_METRICS` of a glyph in design units. A glyph beyond the
/// font gets the metrics of glyph 0.
pub fn design_glyph_metrics(sf: &Sfnt<'_>, info: &FaceInfo, glyph: u16) -> GlyphMetrics {
    let g = if glyph >= info.num_glyphs { 0 } else { glyph };
    let adv = i32::from(advance(sf, g));
    let (lsb, rsb, y_min, y_max) = match glyph_box_any(sf, g) {
        Some([x0, y0, x1, y1]) => (x0, adv - x1, y0, y1),
        None => (0, adv, 0, 0),
    };
    let vo = info.typo_ascender;
    let advh = info.typo_ascender - info.typo_descender;
    let tsb = vo - y_max;
    let bsb = advh - tsb - (y_max - y_min);
    GlyphMetrics {
        left_side_bearing: lsb,
        advance_width: adv as u32,
        right_side_bearing: rsb,
        top_side_bearing: tsb,
        advance_height: advh.max(0) as u32,
        bottom_side_bearing: bsb,
        vertical_origin_y: vo,
    }
}

/// The glyph box from `glyf`, or from the outline (`CFF`).
pub fn glyph_box_any(sf: &Sfnt<'_>, glyph: u16) -> Option<[i32; 4]> {
    if sf.table(b"glyf").is_some() {
        return glyph_box(sf, glyph);
    }
    let font = poly_font::Font::parse(sf.data()).ok()?;
    let b = font.outline(poly_font::GlyphId(glyph))?.bounds()?;
    Some([b.x_min as i32, b.y_min as i32, b.x_max as i32, b.y_max as i32])
}
