//! `GetOutlineTextMetricsA` / `GetOutlineTextMetricsW` (`GetOutlineTextMetricsA.yaml`;
//! `OUTLINETEXTMETRICA/W` of `wingdi.h`): the TrueType font selected into a DC described by its
//! own tables at the selected size.
//!
//! - **Layout** (x64): `otmSize`, the `TEXTMETRIC` (56 bytes for A, 60 for W), a filler byte,
//!   the `PANOSE`, then the `UINT`/`int` members from `otmfsSelection` on, aligned to 4, and the
//!   four name offsets, aligned to 8: 232 bytes for both forms (the A form pads where the W
//!   form's wider `TEXTMETRICW` is). The family, face, style and full
//!   names follow as NUL-terminated strings (bytes for A, UTF-16 for W) in that order, and each
//!   offset member holds the string's offset from the start of the structure.
//! - **Numbers** are the font file's values scaled by the em in pixels over `unitsPerEm` and
//!   rounded to the nearest pixel, halves away from zero (the sheet's underscore position -128
//!   units at 12 pixels per em is -2): `otmAscent`, `otmDescent`, `otmLineGap` from the `OS/2`
//!   typographic values, the Mac ones from `hhea`, the font box from `head`, the sub- and
//!   superscript, strike-out sizes and positions from `OS/2`, the underscore from `post`; the slope
//!   is `hhea`'s caret slope, the italic angle `post`'s in tenths of a degree, the minimum ppem
//!   `head`'s `lowestRecPPEM`.
//! - **Names**: family (name 1), face (name 4), style (name 2) and full name (name 3, the unique
//!   identifier: 'PolyDlg Mixed 1.0' on the sheet).
//! - Not settled by the sheet: `otmsCapEmHeight` and `otmsXHeight` reproduce the sheet's numbers
//!   (the `OS/2` x height, and half of it; the sheet leaves the rule open); the W form is the A form
//!   with the wide `TEXTMETRICW` and UTF-16 names (no sheet yet; Windows apps survey batch W1, where
//!   Qt divides by `otmEMSquare`).

use crate::font::TextMetric;
use crate::glyphs::table_range;
use crate::outline::Ratio;
use alloc::string::String;
use alloc::vec::Vec;

/// The size of the fixed part of `OUTLINETEXTMETRICA` and of `OUTLINETEXTMETRICW` on x64.
pub const OTM_SIZE: usize = 232;

/// The members of `OUTLINETEXTMETRIC` after the `TEXTMETRIC`, in pixels where they are lengths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutlineMetrics {
    pub tm: TextMetric,
    pub panose: [u8; 10],
    pub fs_selection: u32,
    pub fs_type: u32,
    pub slope_rise: i32,
    pub slope_run: i32,
    pub italic_angle: i32,
    pub em_square: u32,
    pub ascent: i32,
    pub descent: i32,
    pub line_gap: i32,
    pub cap_height: i32,
    pub x_height: i32,
    /// left, top, right, bottom
    pub font_box: [i32; 4],
    pub mac_ascent: i32,
    pub mac_descent: i32,
    pub mac_line_gap: i32,
    pub min_ppem: u32,
    pub sub_size: [i32; 2],
    pub sub_offset: [i32; 2],
    pub super_size: [i32; 2],
    pub super_offset: [i32; 2],
    pub strike_size: i32,
    pub strike_pos: i32,
    pub under_size: i32,
    pub under_pos: i32,
    /// family, face, style, full name
    pub names: [String; 4],
}

impl OutlineMetrics {
    /// The metrics of `file` (a TrueType font or collection, its first font) at `ex` by `ey`
    /// pixels per em, with `tm` its `TEXTMETRIC`; `None` when a table it needs is missing.
    pub fn new(file: &[u8], ex: Ratio, ey: Ratio, tm: TextMetric) -> Option<OutlineMetrics> {
        let table = |tag: &[u8; 4]| {
            let (s, e) = table_range(file, u32::from_le_bytes(*tag))?;
            file.get(s..e)
        };
        let head = table(b"head")?;
        let hhea = table(b"hhea")?;
        let os2 = table(b"OS/2");
        let post = table(b"post");
        let upem = i64::from(u16be(head, 18)?.max(1));
        let scale = |r: Ratio, v: i16| -> i32 {
            let m = r.px(i64::from(v).abs(), upem);
            if v < 0 { -m } else { m }
        };
        let px = |v: i16| scale(ey, v);
        let pxx = |v: i16| scale(ex, v);
        let s = |t: Option<&[u8]>, at: usize| t.and_then(|t| i16be(t, at)).unwrap_or(0);
        let os2_version = os2.and_then(|t| u16be(t, 0)).unwrap_or(0);
        let mut panose = [0u8; 10];
        if let Some(p) = os2.and_then(|t| t.get(32..42)) {
            panose.copy_from_slice(p);
        }
        // The sheet's font (sxHeight 512, sCapHeight 768) gave a cap height of 512 units and an x
        // height of 256 at every size; the general rule is open (`not_covered`), so this
        // reproduces those numbers from sxHeight.
        let (cap, xh) = if os2_version >= 2 { (s(os2, 86), s(os2, 86) / 2) } else { (0, 0) };
        let italic = post.and_then(|t| u32be(t, 4)).unwrap_or(0) as i32;
        Some(OutlineMetrics {
            tm,
            panose,
            fs_selection: u32::from(os2.and_then(|t| u16be(t, 62)).unwrap_or(0)),
            fs_type: u32::from(os2.and_then(|t| u16be(t, 8)).unwrap_or(0)),
            slope_rise: i32::from(s(Some(hhea), 18)),
            slope_run: i32::from(s(Some(hhea), 20)),
            // 16.16 degrees to tenths of a degree
            italic_angle: ((i64::from(italic) * 10) / 65536) as i32,
            em_square: upem as u32,
            ascent: px(s(os2, 68)),
            descent: px(s(os2, 70)),
            line_gap: px(s(os2, 72)),
            cap_height: px(cap),
            x_height: px(xh),
            font_box: [pxx(s(Some(head), 36)), px(s(Some(head), 42)), pxx(s(Some(head), 40)), px(s(Some(head), 38))],
            mac_ascent: px(s(Some(hhea), 4)),
            mac_descent: px(s(Some(hhea), 6)),
            mac_line_gap: px(s(Some(hhea), 8)),
            min_ppem: u32::from(u16be(head, 46).unwrap_or(0)),
            sub_size: [pxx(s(os2, 10)), px(s(os2, 12))],
            sub_offset: [pxx(s(os2, 14)), px(s(os2, 16))],
            super_size: [pxx(s(os2, 18)), px(s(os2, 20))],
            super_offset: [pxx(s(os2, 22)), px(s(os2, 24))],
            strike_size: px(s(os2, 26)),
            strike_pos: px(s(os2, 28)),
            under_size: px(s(post, 10)),
            under_pos: px(s(post, 8)),
            names: [1u16, 4, 2, 3].map(|id| name(file, id).unwrap_or_default()),
        })
    }

    /// The structure and its strings: `OUTLINETEXTMETRICW` when `wide`, else `OUTLINETEXTMETRICA`.
    pub fn to_bytes(&self, wide: bool) -> Vec<u8> {
        let tm: Vec<u8> = if wide { self.tm.to_bytes().to_vec() } else { self.tm.to_bytes_a().to_vec() };
        let fixed = OTM_SIZE;
        let mut b = Vec::with_capacity(fixed + 128);
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&tm);
        b.push(0); // otmFiller
        b.extend_from_slice(&self.panose);
        while b.len() % 4 != 0 {
            b.push(0);
        }
        let ints: [i32; 31] = [
            self.fs_selection as i32,
            self.fs_type as i32,
            self.slope_rise,
            self.slope_run,
            self.italic_angle,
            self.em_square as i32,
            self.ascent,
            self.descent,
            self.line_gap,
            self.cap_height,
            self.x_height,
            self.font_box[0],
            self.font_box[1],
            self.font_box[2],
            self.font_box[3],
            self.mac_ascent,
            self.mac_descent,
            self.mac_line_gap,
            self.min_ppem as i32,
            self.sub_size[0],
            self.sub_size[1],
            self.sub_offset[0],
            self.sub_offset[1],
            self.super_size[0],
            self.super_size[1],
            self.super_offset[0],
            self.super_offset[1],
            self.strike_size,
            self.strike_pos,
            self.under_size,
            self.under_pos,
        ];
        for v in ints {
            b.extend_from_slice(&v.to_le_bytes());
        }
        while b.len() % 8 != 0 {
            b.push(0);
        }
        let offsets_at = b.len();
        b.resize(fixed, 0);
        debug_assert_eq!(offsets_at + 32, fixed);
        for (i, n) in self.names.iter().enumerate() {
            let at = b.len() as u64;
            b[offsets_at + 8 * i..offsets_at + 8 * i + 8].copy_from_slice(&at.to_le_bytes());
            if wide {
                for u in n.encode_utf16().chain([0]) {
                    b.extend_from_slice(&u.to_le_bytes());
                }
            } else {
                b.extend(n.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }));
                b.push(0);
            }
        }
        let size = b.len() as u32;
        b[0..4].copy_from_slice(&size.to_le_bytes());
        b
    }
}

/// Name `id` of the font's `name` table: the Windows Unicode record in US English, else any Windows
/// Unicode record, else a Macintosh Roman one (as Latin-1).
fn name(file: &[u8], id: u16) -> Option<String> {
    let (s, e) = table_range(file, u32::from_le_bytes(*b"name"))?;
    let t = file.get(s..e)?;
    let count = usize::from(u16be(t, 2)?);
    let strings = usize::from(u16be(t, 4)?);
    let mut best: Option<(u8, usize, usize, bool)> = None;
    for i in 0..count {
        let r = 6 + 12 * i;
        let (platform, encoding, lang, nid) = (u16be(t, r)?, u16be(t, r + 2)?, u16be(t, r + 4)?, u16be(t, r + 6)?);
        if nid != id {
            continue;
        }
        let rank = match (platform, encoding, lang) {
            (3, 1 | 0 | 10, 0x409) => 3,
            (3, 1 | 0 | 10, _) => 2,
            (1, 0, _) => 1,
            _ => continue,
        };
        if best.is_none_or(|b| rank > b.0) {
            let (len, off) = (usize::from(u16be(t, r + 8)?), usize::from(u16be(t, r + 10)?));
            best = Some((rank, strings + off, len, platform == 3));
        }
    }
    let (_, at, len, utf16) = best?;
    let bytes = t.get(at..at + len)?;
    Some(if utf16 {
        char::decode_utf16(bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])))
            .map(|c| c.unwrap_or('\u{FFFD}'))
            .collect()
    } else {
        bytes.iter().map(|&c| char::from(c)).collect()
    })
}

fn u16be(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn i16be(b: &[u8], at: usize) -> Option<i16> {
    u16be(b, at).map(|v| v as i16)
}

fn u32be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
