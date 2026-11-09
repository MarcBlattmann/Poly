//! Glyph geometry and rasterisation for `IDWriteGlyphRunAnalysis`,
//! `GetGlyphRunOutline`, the GDI-compatible metrics and the recommended
//! rendering mode. Rules as recorded in the spec sheets of
//! `specs/windows/dwrite.dll` (see `IDWriteFactory.CreateGlyphRunAnalysis`
//! and `IDWriteFontFace.GetGlyphRunOutline`).
//!
//! The rasteriser point-samples the outline: a ClearType texture takes six
//! horizontal samples per pixel (two per stripe) on 16 scan lines, counts
//! the samples strictly inside the outline (non-zero winding), and filters
//! the stripes with a box of three; an aliased texture takes one sample per
//! pixel at its centre. The bounds are the pixels with a non-zero result.

use alloc::vec;
use alloc::vec::Vec;

use poly_font::{Font, GlyphId, Point, Segment};

use crate::face::{FaceInfo, Metrics};

// ----- small float helpers (no `std`)

pub fn floor(v: f32) -> f32 {
    let t = v as i64 as f32;
    if t > v { t - 1.0 } else { t }
}

pub fn ceil(v: f32) -> f32 {
    let t = v as i64 as f32;
    if t < v { t + 1.0 } else { t }
}

/// Round half up.
pub fn round(v: f32) -> f32 {
    floor(v + 0.5)
}

pub fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE is part of x86-64.
        unsafe {
            use core::arch::x86_64::{_mm_cvtss_f32, _mm_set_ss, _mm_sqrt_ss};
            _mm_cvtss_f32(_mm_sqrt_ss(_mm_set_ss(v)))
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut x = v;
        for _ in 0..30 {
            x = 0.5 * (x + v / x);
        }
        x
    }
}

pub fn abs(v: f32) -> f32 {
    if v < 0.0 { -v } else { v }
}

// ----- transforms

/// `DWRITE_MATRIX`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0, dx: 0.0, dy: 0.0 };

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.m11 + y * self.m21 + self.dx, x * self.m12 + y * self.m22 + self.dy)
    }

    /// The factor by which it scales lengths: the square root of the
    /// determinant's magnitude.
    pub fn scale(&self) -> f32 {
        sqrt(abs(self.m11 * self.m22 - self.m12 * self.m21))
    }
}

// ----- glyph geometry

/// A flattened outline in pixels (y down): closed polygons.
#[derive(Clone, Debug, Default)]
pub struct Polys {
    pub contours: Vec<Vec<(f32, f32)>>,
}

impl Polys {
    pub fn is_empty(&self) -> bool {
        self.contours.is_empty()
    }

    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for &(x, y) in self.contours.iter().flatten() {
            b = Some(match b {
                None => [x, y, x, y],
                Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
            });
        }
        b
    }
}

/// A glyph of a run, placed.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub glyph: u16,
    /// The glyph origin in pixels, relative to the run origin (y down).
    pub x: f32,
    pub y: f32,
}

/// The positions of a run's glyphs. `advances` are in pixels when given;
/// otherwise the font's own. Offsets are `(advanceOffset, ascenderOffset)`.
/// Right-to-left runs extend leftwards from the origin: each glyph's origin
/// is its natural advance to the left of the pen, which moves left by the
/// given advance.
pub fn place_run(
    font: &Font<'_>,
    em_px: f32,
    glyphs: &[u16],
    advances: Option<&[f32]>,
    offsets: Option<&[(f32, f32)]>,
    rtl: bool,
) -> Vec<Placed> {
    let upem = f32::from(font.units_per_em());
    let scale = em_px / upem;
    let mut pen = 0.0f32;
    let mut out = Vec::with_capacity(glyphs.len());
    for (i, &g) in glyphs.iter().enumerate() {
        let natural = f32::from(font.advance(glyph_or_notdef(font, g))) * scale;
        let adv = advances.and_then(|a| a.get(i)).copied().unwrap_or(natural);
        let (ox, oy) = offsets.and_then(|o| o.get(i)).copied().unwrap_or((0.0, 0.0));
        let x = if rtl { pen - natural - ox } else { pen + ox };
        out.push(Placed { glyph: g, x, y: -oy });
        if rtl {
            pen -= adv;
        } else {
            pen += adv;
        }
    }
    out
}

fn glyph_or_notdef(font: &Font<'_>, g: u16) -> GlyphId {
    if g >= font.num_glyphs() { GlyphId(0) } else { GlyphId(g) }
}

/// One glyph's outline in pixels relative to its origin (y down), curves
/// kept (`Segment`s), sideways turned a quarter.
pub fn glyph_segments(font: &Font<'_>, info: &FaceInfo, glyph: u16, em_px: f32, sideways: bool) -> Vec<Segment> {
    let g = glyph_or_notdef(font, glyph);
    let Some(path) = font.outline(g) else { return Vec::new() };
    let upem = f32::from(font.units_per_em());
    let s = em_px / upem;
    let adv_px = f32::from(font.advance(g)) * s;
    let vo = info.typo_ascender as f32 * s;
    let map = |p: Point| -> Point {
        let (x, y) = (p.x * s, -p.y * s);
        if sideways { Point::new(y + vo, -x + adv_px / 2.0) } else { Point::new(x, y) }
    };
    path.map(map).segments
}

/// Flatten segments into closed polygons, shifted by `(ox, oy)` and
/// transformed.
pub fn flatten(segs: &[Segment], ox: f32, oy: f32, m: &Matrix, out: &mut Polys) {
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let mut last = Point::new(0.0, 0.0);
    let put = |p: Point| m.apply(p.x + ox, p.y + oy);
    let flush = |cur: &mut Vec<(f32, f32)>, out: &mut Polys| {
        if cur.len() >= 2 {
            out.contours.push(core::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    for s in segs {
        match *s {
            Segment::MoveTo(p) => {
                flush(&mut cur, out);
                cur.push(put(p));
                last = p;
            }
            Segment::LineTo(p) => {
                cur.push(put(p));
                last = p;
            }
            Segment::QuadTo(c, p) => {
                let n = 12;
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let u = 1.0 - t;
                    cur.push(put(Point::new(
                        u * u * last.x + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * last.y + 2.0 * u * t * c.y + t * t * p.y,
                    )));
                }
                last = p;
            }
            Segment::CubicTo(a, b, p) => {
                let n = 16;
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let u = 1.0 - t;
                    cur.push(put(Point::new(
                        u * u * u * last.x + 3.0 * u * u * t * a.x + 3.0 * u * t * t * b.x + t * t * t * p.x,
                        u * u * u * last.y + 3.0 * u * u * t * a.y + 3.0 * u * t * t * b.y + t * t * t * p.y,
                    )));
                }
                last = p;
            }
            Segment::Close => flush(&mut cur, out),
        }
    }
    flush(&mut cur, out);
}

/// Lean the segments to the right by `k` pixels per pixel of height (the
/// oblique simulation; y is down).
pub fn shear(segs: &mut [Segment], k: f32) {
    let f = |p: &mut Point| p.x -= k * p.y;
    for s in segs.iter_mut() {
        match s {
            Segment::MoveTo(p) | Segment::LineTo(p) => f(p),
            Segment::QuadTo(c, p) => {
                f(c);
                f(p)
            }
            Segment::CubicTo(a, b, p) => {
                f(a);
                f(b);
                f(p)
            }
            Segment::Close => {}
        }
    }
}

// ----- sampling

/// How a texture is produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One byte per pixel, 0 or 255, sampled at pixel centres.
    Aliased,
    /// One byte per pixel, anti-aliased (grayscale).
    Gray,
    /// Three bytes per pixel, ClearType, `lines` scan lines per pixel.
    ClearType { lines: u32 },
}

/// A rendered run: the pixel rectangle and the bytes of its texture.
#[derive(Clone, Debug)]
pub struct Raster {
    pub kind: Kind,
    /// `left, top, right, bottom` of the non-empty pixels.
    pub rect: [i32; 4],
    pub bytes_per_pixel: usize,
    pub data: Vec<u8>,
}

impl Raster {
    pub fn empty(kind: Kind) -> Raster {
        Raster {
            kind,
            rect: [0; 4],
            bytes_per_pixel: if matches!(kind, Kind::ClearType { .. }) { 3 } else { 1 },
            data: Vec::new(),
        }
    }

    /// The texture bytes of `rect` (zero where the raster has none).
    pub fn texture(&self, rect: [i32; 4]) -> Vec<u8> {
        let (w, h) = ((rect[2] - rect[0]) as usize, (rect[3] - rect[1]) as usize);
        let bpp = self.bytes_per_pixel;
        let mut out = vec![0u8; w * h * bpp];
        let rw = (self.rect[2] - self.rect[0]) as usize;
        for y in 0..h as i32 {
            let sy = rect[1] + y - self.rect[1];
            if sy < 0 || sy >= self.rect[3] - self.rect[1] {
                continue;
            }
            for x in 0..w as i32 {
                let sx = rect[0] + x - self.rect[0];
                if sx < 0 || sx >= rw as i32 {
                    continue;
                }
                let s = (sy as usize * rw + sx as usize) * bpp;
                let d = (y as usize * w + x as usize) * bpp;
                out[d..d + bpp].copy_from_slice(&self.data[s..s + bpp]);
            }
        }
        out
    }
}

/// The biggest raster side or area attempted.
const MAX_SIDE: f32 = 8192.0;
const MAX_AREA: f32 = 64.0 * 1024.0 * 1024.0;

struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dir: i32,
}

fn edges(polys: &Polys) -> Vec<Edge> {
    let mut out = Vec::new();
    for c in &polys.contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if a.1 == b.1 {
                continue;
            }
            let (x0, y0, x1, y1, dir) = if a.1 < b.1 { (a.0, a.1, b.0, b.1, 1) } else { (b.0, b.1, a.0, a.1, -1) };
            out.push(Edge { x0, y0, x1, y1, dir });
        }
    }
    out
}

/// The spans of non-zero winding on the line `y`.
fn spans(edges: &[Edge], y: f32, scratch: &mut Vec<(f32, i32)>, out: &mut Vec<(f32, f32)>) {
    scratch.clear();
    out.clear();
    for e in edges {
        if e.y0 <= y && y < e.y1 {
            let x = e.x0 + (y - e.y0) * (e.x1 - e.x0) / (e.y1 - e.y0);
            scratch.push((x, e.dir));
        }
    }
    scratch.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    let mut wind = 0;
    let mut start = 0.0;
    for &(x, d) in scratch.iter() {
        let before = wind;
        wind += d;
        if before == 0 && wind != 0 {
            start = x;
        } else if before != 0 && wind == 0 {
            out.push((start, x));
        }
    }
}

/// Rasterise `polys`.
pub fn rasterize(polys: &Polys, kind: Kind) -> Raster {
    let Some(b) = polys.bounds() else { return Raster::empty(kind) };
    let (w, h) = (b[2] - b[0] + 4.0, b[3] - b[1] + 4.0);
    if !(w > 0.0 && h > 0.0) || w > MAX_SIDE || h > MAX_SIDE || w * h > MAX_AREA {
        return Raster::empty(kind);
    }
    let left = floor(b[0]) as i32 - 2;
    let top = floor(b[1]) as i32 - 2;
    let (pw, ph) = (w as usize + 2, h as usize + 2);
    let edges = edges(polys);
    let mut scratch = Vec::new();
    let mut sp = Vec::new();
    match kind {
        Kind::Aliased => {
            let mut px = vec![0u8; pw * ph];
            for row in 0..ph {
                let y = (top + row as i32) as f32 + 0.5;
                spans(&edges, y, &mut scratch, &mut sp);
                for &(xa, xb) in &sp {
                    // pixel centres strictly inside (xa, xb)
                    let k0 = floor(xa - 0.5) as i32 + 1;
                    let k1 = ceil(xb - 0.5) as i32 - 1;
                    for k in k0..=k1 {
                        let c = k - left;
                        if c >= 0 && (c as usize) < pw {
                            px[row * pw + c as usize] = 255;
                        }
                    }
                }
            }
            crop(px, pw, ph, left, top, 1, kind)
        }
        Kind::Gray | Kind::ClearType { .. } => {
            let lines = match kind {
                Kind::ClearType { lines } => lines.max(1),
                _ => 16,
            } as usize;
            let subs = pw * 3;
            let mut out = vec![0u8; if kind == Kind::Gray { pw * ph } else { subs * ph }];
            let mut count = vec![0u16; subs];
            for row in 0..ph {
                for c in count.iter_mut() {
                    *c = 0;
                }
                for j in 0..lines {
                    let y = (top + row as i32) as f32 + (j as f32 + 0.5) / lines as f32;
                    spans(&edges, y, &mut scratch, &mut sp);
                    for &(xa, xb) in &sp {
                        let k0 = floor(xa * 6.0 - 0.5) as i32 + 1 - left * 6;
                        let k1 = ceil(xb * 6.0 - 0.5) as i32 - 1 - left * 6;
                        for k in k0.max(0)..=k1.min(subs as i32 * 2 - 1) {
                            count[(k / 2) as usize] += 1;
                        }
                    }
                }
                let per = (2 * lines) as f32;
                if kind == Kind::Gray {
                    for x in 0..pw {
                        let n: u32 = count[3 * x..3 * x + 3].iter().map(|&c| u32::from(c)).sum();
                        out[row * pw + x] = (n as f32 / (3.0 * per) * 255.0 + 0.5) as u8;
                    }
                } else {
                    for s in 0..subs {
                        let at = |i: i32| -> f32 { if i < 0 || i >= subs as i32 { 0.0 } else { f32::from(count[i as usize]) / per } };
                        let f = (at(s as i32 - 1) + at(s as i32) + at(s as i32 + 1)) / 3.0;
                        out[row * subs + s] = (f * 255.0 + 0.5) as u8;
                    }
                }
            }
            let bpp = if kind == Kind::Gray { 1 } else { 3 };
            crop(out, pw, ph, left, top, bpp, kind)
        }
    }
}

/// Cut a pixel grid to the pixels with a non-zero byte.
fn crop(px: Vec<u8>, pw: usize, ph: usize, left: i32, top: i32, bpp: usize, kind: Kind) -> Raster {
    let (mut x0, mut y0, mut x1, mut y1) = (pw, ph, 0usize, 0usize);
    for y in 0..ph {
        for x in 0..pw {
            if px[(y * pw + x) * bpp..(y * pw + x + 1) * bpp].iter().any(|&v| v != 0) {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Raster::empty(kind);
    }
    let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0) * bpp);
    for y in y0..y1 {
        data.extend_from_slice(&px[(y * pw + x0) * bpp..(y * pw + x1) * bpp]);
    }
    Raster {
        kind,
        rect: [left + x0 as i32, top + y0 as i32, left + x1 as i32, top + y1 as i32],
        bytes_per_pixel: bpp,
        data,
    }
}

// ----- the recommended rendering mode

pub mod mode {
    pub const DEFAULT: u32 = 0;
    pub const ALIASED: u32 = 1;
    pub const GDI_CLASSIC: u32 = 2;
    pub const GDI_NATURAL: u32 = 3;
    pub const NATURAL: u32 = 4;
    pub const NATURAL_SYMMETRIC: u32 = 5;
    pub const OUTLINE: u32 = 6;
}

/// `GetRecommendedRenderingMode`: `None` for an invalid measuring mode.
pub fn recommended_mode(em_size: f32, pixels_per_dip: f32, measuring: u32, params_mode: u32) -> Option<u32> {
    if measuring > 2 {
        return None;
    }
    if !(em_size > 0.0) || !(pixels_per_dip > 0.0) {
        return Some(mode::NATURAL);
    }
    if params_mode != mode::DEFAULT {
        return Some(params_mode);
    }
    if em_size * pixels_per_dip >= 100.0 {
        return Some(mode::OUTLINE);
    }
    Some(match measuring {
        0 => mode::NATURAL_SYMMETRIC,
        1 => mode::GDI_CLASSIC,
        _ => mode::GDI_NATURAL,
    })
}

// ----- GDI-compatible metrics

/// A value in design units as GDI sees it at `ppem`: scaled to pixels,
/// rounded (half down), back to design units (half up).
fn gdi_round(v: f32, upem: f32, ppem: f32) -> f32 {
    let px = ceil(v * ppem / upem - 0.5);
    floor(px * upem / ppem + 0.5)
}

/// `GetGdiCompatibleMetrics` for a size in pixels per em.
pub fn gdi_metrics(m: &Metrics, ppem: f32) -> Metrics {
    let u = f32::from(m.design_units_per_em);
    let r = |v: i32| gdi_round(v as f32, u, ppem) as i32;
    Metrics {
        design_units_per_em: m.design_units_per_em,
        ascent: r(i32::from(m.ascent)).max(0) as u16,
        descent: r(i32::from(m.descent)).max(0) as u16,
        line_gap: r(i32::from(m.line_gap)) as i16,
        cap_height: r(i32::from(m.cap_height)).max(0) as u16,
        x_height: r(i32::from(m.x_height)).max(0) as u16,
        underline_position: r(i32::from(m.underline_position)) as i16,
        underline_thickness: r(i32::from(m.underline_thickness)).max(0) as u16,
        strikethrough_position: r(i32::from(m.strikethrough_position)) as i16,
        strikethrough_thickness: r(i32::from(m.strikethrough_thickness)).max(0) as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Polys {
        Polys { contours: alloc::vec![alloc::vec![(x0, y0), (x0, y1), (x1, y1), (x1, y0)]] }
    }

    #[test]
    fn aliased_square_bounds() {
        let r = rasterize(&square(1.0, -8.0, 9.0, 0.0), Kind::Aliased);
        assert_eq!(r.rect, [1, -8, 9, 0]);
        assert!(r.data.iter().all(|&v| v == 255));
        let r = rasterize(&square(3.3, -26.4, 29.7, 0.0), Kind::Aliased);
        assert_eq!(r.rect, [3, -26, 30, 0]);
    }

    #[test]
    fn cleartype_square_bounds_and_edge() {
        let r = rasterize(&square(1.0, -8.0, 9.0, 0.0), Kind::ClearType { lines: 16 });
        assert_eq!(r.rect, [0, -8, 10, 0]);
        // pixel 0: 00 00 55, pixel 1: aa ff ff
        assert_eq!(&r.data[..6], &[0, 0, 0x55, 0xaa, 0xff, 0xff]);
        let r = rasterize(&square(3.3, -26.4, 29.7, 0.0), Kind::ClearType { lines: 16 });
        assert_eq!(r.rect, [3, -27, 30, 0]);
        let r = rasterize(&square(3.3, -26.4, 29.7, 0.0), Kind::ClearType { lines: 1 });
        assert_eq!(r.rect, [3, -26, 30, 0]);
        let r = rasterize(&square(2.25, -16.0, 18.25, 0.0), Kind::ClearType { lines: 16 });
        assert_eq!(r.rect[0], 2);
    }

    #[test]
    fn gdi_rounding() {
        let m = Metrics {
            design_units_per_em: 1000,
            ascent: 900,
            descent: 300,
            line_gap: 0,
            cap_height: 700,
            x_height: 500,
            underline_position: -100,
            underline_thickness: 50,
            strikethrough_position: 250,
            strikethrough_thickness: 40,
        };
        let g = gdi_metrics(&m, 8.0);
        assert_eq!((g.ascent, g.descent, g.cap_height, g.x_height), (875, 250, 750, 500));
        assert_eq!((g.underline_position, g.underline_thickness), (-125, 0));
        let g = gdi_metrics(&m, 13.0);
        assert_eq!((g.ascent, g.descent, g.cap_height, g.x_height), (923, 308, 692, 462));
        assert_eq!((g.underline_position, g.underline_thickness, g.strikethrough_position, g.strikethrough_thickness), (-77, 77, 231, 77));
        let g = gdi_metrics(&m, 20.5);
        assert_eq!((g.ascent, g.descent, g.cap_height, g.x_height), (878, 293, 683, 488));
        assert_eq!(gdi_metrics(&m, 100.0), m);
    }

    #[test]
    fn recommended() {
        assert_eq!(recommended_mode(12.0, 1.0, 0, 0), Some(5));
        assert_eq!(recommended_mode(12.0, 1.0, 1, 0), Some(2));
        assert_eq!(recommended_mode(12.0, 1.0, 2, 0), Some(3));
        assert_eq!(recommended_mode(100.0, 1.0, 1, 0), Some(6));
        assert_eq!(recommended_mode(12.0, 1.0, 0, 1), Some(1));
        assert_eq!(recommended_mode(0.0, 1.0, 0, 0), Some(4));
        assert_eq!(recommended_mode(12.0, 1.0, 7, 0), None);
    }
}
