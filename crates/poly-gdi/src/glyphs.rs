//! Glyph data for programs that lay out text themselves (Windows apps survey, batch W1): `GetFontData`,
//! `GetGlyphOutlineW` and `GetCharABCWidthsFloatW` of the font a DC has selected. Qt's GDI font
//! engine reads the font's tables and each glyph's metrics through them; without them it drew
//! no text (VLC's dialogs, the Windows apps survey).
//!
//! What is here follows the Microsoft Learn reference of the three functions and of
//! `GLYPHMETRICS`, `MAT2`, `TTPOLYGONHEADER`, `TTPOLYCURVE`, `POINTFX`, `FIXED` and `ABCFLOAT`
//! (`wingdi.h` for the constants and layouts). No spec sheet records them yet; where Learn leaves
//! a value open the choice is written down beside it, to be sharpened by one.
//!
//! - **Font data** is the selected outline font's file: table 0 is the whole file, any other
//!   value a table of its directory by tag (the tag's four characters in memory order).
//! - **Glyph outlines** are in the device's pixels, y up, relative to the glyph's origin on the
//!   baseline, through the program's `MAT2`. The bitmaps come from the same rasteriser that draws
//!   text ([`crate::outline::render_xf`]), so a glyph measured and a glyph drawn agree.
//! - **ABC widths** are the left side bearing, the black box and the rest of the advance, in
//!   fractional pixels.

use crate::outline::{SLANT, Typeface, render_xf};
use alloc::vec;
use alloc::vec::Vec;
use poly_font::{GlyphId, Segment};

/// `GGO_*` (`wingdi.h`): what `GetGlyphOutlineW` returns.
pub const GGO_METRICS: u32 = 0;
pub const GGO_BITMAP: u32 = 1;
pub const GGO_NATIVE: u32 = 2;
pub const GGO_BEZIER: u32 = 3;
pub const GGO_GRAY2_BITMAP: u32 = 4;
pub const GGO_GRAY4_BITMAP: u32 = 5;
pub const GGO_GRAY8_BITMAP: u32 = 6;
pub const GGO_GLYPH_INDEX: u32 = 0x0080;
pub const GGO_UNHINTED: u32 = 0x0100;
/// `TT_POLYGON_TYPE`, `TT_PRIM_LINE`, `TT_PRIM_QSPLINE`, `TT_PRIM_CSPLINE` (`wingdi.h`).
pub const TT_POLYGON_TYPE: u32 = 24;
pub const TT_PRIM_LINE: u16 = 1;
pub const TT_PRIM_QSPLINE: u16 = 2;
pub const TT_PRIM_CSPLINE: u16 = 3;

/// `MAT2` as numbers (its four `FIXED` values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat2 {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
}

impl Mat2 {
    pub const IDENTITY: Mat2 = Mat2 { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0 };

    /// From the 16 bytes of a `MAT2`: four `FIXED` (`WORD fract; short value`), each the
    /// little-endian 16.16 number.
    pub fn from_bytes(b: &[u8; 16]) -> Mat2 {
        let f = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as f32 / 65536.0;
        Mat2 { m11: f(0), m12: f(4), m21: f(8), m22: f(12) }
    }
}

/// `GLYPHMETRICS` (20 bytes: two `UINT`, a `POINT`, two `short`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub black_box_x: u32,
    pub black_box_y: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub cell_inc_x: i16,
    pub cell_inc_y: i16,
}

impl GlyphMetrics {
    pub fn to_bytes(&self) -> [u8; 20] {
        let mut b = [0u8; 20];
        b[0..4].copy_from_slice(&self.black_box_x.to_le_bytes());
        b[4..8].copy_from_slice(&self.black_box_y.to_le_bytes());
        b[8..12].copy_from_slice(&self.origin_x.to_le_bytes());
        b[12..16].copy_from_slice(&self.origin_y.to_le_bytes());
        b[16..18].copy_from_slice(&self.cell_inc_x.to_le_bytes());
        b[18..20].copy_from_slice(&self.cell_inc_y.to_le_bytes());
        b
    }
}

/// Why a glyph request fails (`GDI_ERROR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphError {
    /// The selected font is a bitmap face: it has no outlines.
    NotOutline,
    /// A format `GetGlyphOutlineW` does not know.
    BadFormat,
}

/// The range of the font file `table` is: the whole file for 0, else the table with that tag in
/// the directory of the file's first font (`None`: no such table).
pub fn table_range(file: &[u8], table: u32) -> Option<(usize, usize)> {
    if table == 0 {
        return Some((0, file.len()));
    }
    let tag = table.to_le_bytes();
    if &tag == b"ttcf" && file.starts_with(b"ttcf") {
        return Some((0, file.len()));
    }
    // A collection: the directory of its first font.
    let dir = if file.starts_with(b"ttcf") { be32(file, 12)? as usize } else { 0 };
    let count = usize::from(be16(file, dir + 4)?);
    (0..count).find_map(|i| {
        let rec = dir + 12 + 16 * i;
        (file.get(rec..rec + 4)? == tag).then_some(())?;
        let (off, len) = (be32(file, rec + 8)? as usize, be32(file, rec + 12)? as usize);
        (off.checked_add(len)? <= file.len()).then_some((off, off + len))
    })
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The map from the font's units (x right, y up) to the device's pixels (y up), `MAT2` applied
/// after the font's size and slant: `x' = a x + c y`, `y' = b x + d y`.
fn units_to_pixels(tf: &Typeface<'_>, mat: Mat2) -> Option<[f32; 4]> {
    let (o, _) = tf.parts()?;
    let upem = o.upem as f32;
    let (sx, sy) = (o.ex.em() / upem, o.ey.em() / upem);
    let k = if o.synth_italic { SLANT } else { 0.0 };
    // Before MAT2: x = sx X + k sy Y, y = sy Y.
    let (a0, c0, d0) = (sx, k * sy, sy);
    Some([a0 * mat.m11, a0 * mat.m12, c0 * mat.m11 + d0 * mat.m21, c0 * mat.m12 + d0 * mat.m22])
}

/// `GetGlyphOutlineW`'s answer for character (or, with `GGO_GLYPH_INDEX`, glyph) `c`: the
/// metrics and the data of `format` (empty for `GGO_METRICS`).
pub fn glyph_outline(tf: &Typeface<'_>, c: u16, format: u32, mat: Mat2) -> Result<(GlyphMetrics, Vec<u8>), GlyphError> {
    let (_, font) = tf.parts().ok_or(GlyphError::NotOutline)?;
    let kind = format & !(GGO_GLYPH_INDEX | GGO_UNHINTED);
    if kind > GGO_GRAY8_BITMAP {
        return Err(GlyphError::BadFormat);
    }
    let gid = tf.glyph_of(c, format & GGO_GLYPH_INDEX != 0).unwrap_or(GlyphId(0));
    let [a, b, c_, d] = units_to_pixels(tf, mat).ok_or(GlyphError::NotOutline)?;
    // The rasteriser's map is y down.
    let bmp = render_xf(font, gid, [a, -b, c_, -d], false);
    let adv = tf.glyph_advance(gid).unwrap_or(0) as f32;
    let round = |v: f32| if v < 0.0 { -((-v + 0.5) as i32) } else { (v + 0.5) as i32 };
    let mut gm = GlyphMetrics {
        cell_inc_x: round(adv * mat.m11) as i16,
        cell_inc_y: round(adv * mat.m12) as i16,
        ..GlyphMetrics::default()
    };
    if bmp.width == 0 || bmp.height == 0 {
        // An empty glyph (a space): a black box of one pixel at the origin (Learn gives no size
        // for it; the choice of the common implementations of this call).
        gm.black_box_x = 1;
        gm.black_box_y = 1;
    } else {
        gm.black_box_x = bmp.width;
        gm.black_box_y = bmp.height;
        gm.origin_x = bmp.left;
        gm.origin_y = -bmp.top;
    }
    let data = match kind {
        GGO_METRICS => Vec::new(),
        GGO_BITMAP => mono(&bmp),
        GGO_GRAY2_BITMAP => gray(&bmp, 4),
        GGO_GRAY4_BITMAP => gray(&bmp, 16),
        GGO_GRAY8_BITMAP => gray(&bmp, 64),
        _ => outline_records(font, gid, [a, b, c_, d], kind == GGO_BEZIER),
    };
    Ok((gm, data))
}

/// `GGO_BITMAP`: one bit a pixel, the first pixel in the high bit, rows a multiple of 32 bits.
fn mono(b: &poly_font::Bitmap) -> Vec<u8> {
    let (w, h) = (b.width as usize, b.height as usize);
    let stride = w.div_ceil(32) * 4;
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            if b.data[y * w + x] >= 128 {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

/// `GGO_GRAY*_BITMAP`: a byte a pixel from 0 to `levels`, rows a multiple of 32 bits.
fn gray(b: &poly_font::Bitmap, levels: u32) -> Vec<u8> {
    let (w, h) = (b.width as usize, b.height as usize);
    let stride = w.div_ceil(4) * 4;
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            out[y * stride + x] = ((u32::from(b.data[y * w + x]) * levels + 127) / 255) as u8;
        }
    }
    out
}

/// A `FIXED` (16.16) as the 4 bytes of `POINTFX`'s halves.
fn fixed(v: f32) -> [u8; 4] {
    let x = v * 65536.0;
    // To the nearest (no_std: no `f32::round`); `as` saturates.
    let r = if x < 0.0 { -((-x + 0.5) as i32) } else { (x + 0.5) as i32 };
    r.to_le_bytes()
}

/// `GGO_NATIVE` (lines and quadratic splines) or `GGO_BEZIER` (lines and cubic splines): one
/// `TTPOLYGONHEADER` per contour (`cb` the contour's bytes, `TT_POLYGON_TYPE`, its first point),
/// then `TTPOLYCURVE` records (`wType`, `cpfx`, the points). A contour closes by itself back to
/// its first point.
fn outline_records(font: &poly_font::Font<'_>, gid: GlyphId, m: [f32; 4], bezier: bool) -> Vec<u8> {
    let Some(path) = font.outline(gid) else { return Vec::new() };
    let map = |p: poly_font::Point| (m[0] * p.x + m[2] * p.y, m[1] * p.x + m[3] * p.y);
    let mut out = Vec::new();
    let mut contour: Vec<u8> = Vec::new();
    let mut start = (0.0f32, 0.0f32);
    let mut cur = (0.0f32, 0.0f32);
    let mut open = false;
    // The line record being collected: its points.
    let mut lines: Vec<(f32, f32)> = Vec::new();
    let point = |buf: &mut Vec<u8>, p: (f32, f32)| {
        buf.extend_from_slice(&fixed(p.0));
        buf.extend_from_slice(&fixed(p.1));
    };
    let record = |buf: &mut Vec<u8>, kind: u16, pts: &[(f32, f32)]| {
        buf.extend_from_slice(&kind.to_le_bytes());
        buf.extend_from_slice(&(pts.len() as u16).to_le_bytes());
        for &p in pts {
            point(buf, p);
        }
    };
    let finish = |out: &mut Vec<u8>, contour: &mut Vec<u8>, lines: &mut Vec<(f32, f32)>, start: (f32, f32)| {
        if !lines.is_empty() {
            record(contour, TT_PRIM_LINE, lines);
            lines.clear();
        }
        let cb = 16 + contour.len() as u32;
        out.extend_from_slice(&cb.to_le_bytes());
        out.extend_from_slice(&TT_POLYGON_TYPE.to_le_bytes());
        point(out, start);
        out.append(contour);
    };
    for seg in &path.segments {
        match *seg {
            Segment::MoveTo(p) => {
                if open {
                    finish(&mut out, &mut contour, &mut lines, start);
                }
                start = map(p);
                cur = start;
                open = true;
            }
            Segment::LineTo(p) => {
                cur = map(p);
                lines.push(cur);
            }
            Segment::QuadTo(c, p) => {
                if !lines.is_empty() {
                    record(&mut contour, TT_PRIM_LINE, &lines);
                    lines.clear();
                }
                let (c, p) = (map(c), map(p));
                if bezier {
                    let c1 = (cur.0 + 2.0 / 3.0 * (c.0 - cur.0), cur.1 + 2.0 / 3.0 * (c.1 - cur.1));
                    let c2 = (p.0 + 2.0 / 3.0 * (c.0 - p.0), p.1 + 2.0 / 3.0 * (c.1 - p.1));
                    record(&mut contour, TT_PRIM_CSPLINE, &[c1, c2, p]);
                } else {
                    record(&mut contour, TT_PRIM_QSPLINE, &[c, p]);
                }
                cur = p;
            }
            Segment::CubicTo(c1, c2, p) => {
                if !lines.is_empty() {
                    record(&mut contour, TT_PRIM_LINE, &lines);
                    lines.clear();
                }
                let (c1, c2, p) = (map(c1), map(c2), map(p));
                record(&mut contour, TT_PRIM_CSPLINE, &[c1, c2, p]);
                cur = p;
            }
            Segment::Close => {
                if open {
                    finish(&mut out, &mut contour, &mut lines, start);
                    open = false;
                }
            }
        }
    }
    if open {
        finish(&mut out, &mut contour, &mut lines, start);
    }
    out
}

/// `ABCFLOAT` of character `c`: the left side bearing, the black box's width and the rest of the
/// advance, in pixels. A bitmap face has no bearings: its width is the black box.
pub fn abc_float(tf: &Typeface<'_>, c: u16) -> [f32; 3] {
    let Some((o, font)) = tf.parts() else { return [0.0, tf.width(c) as f32, 0.0] };
    let gid = tf.glyph_of(c, false).unwrap_or(GlyphId(0));
    let sx = o.ex.em() / o.upem as f32;
    let adv = f32::from(font.advance(gid)) * sx + if o.synth_bold { 1.0 } else { 0.0 };
    match font.outline(gid).and_then(|p| p.bounds()) {
        Some(b) => {
            let a = b.x_min * sx;
            let bw = (b.x_max - b.x_min) * sx;
            [a, bw, adv - a - bw]
        }
        None => [0.0, 0.0, adv],
    }
}

/// `GetCharABCWidthsW` / `GetCharABCWidthsI` (Learn): the whole-pixel `ABC` of character (or glyph
/// index) `c`: A the black box's left edge (`gmptGlyphOrigin.x` of [`glyph_outline`]), B its width,
/// C the rest of the advance, so that the three add up to the advance text is drawn with. `None` for
/// a bitmap face (Learn: these functions work with TrueType fonts only).
pub fn abc_int(tf: &Typeface<'_>, c: u16, glyph_index: bool) -> Option<(i32, u32, i32)> {
    let format = if glyph_index { GGO_METRICS | GGO_GLYPH_INDEX } else { GGO_METRICS };
    let (gm, _) = glyph_outline(tf, c, format, Mat2::IDENTITY).ok()?;
    let adv = tf.glyph_advance(tf.glyph_of(c, glyph_index)?)?;
    Some((gm.origin_x, gm.black_box_x, adv - gm.origin_x - gm.black_box_x as i32))
}
