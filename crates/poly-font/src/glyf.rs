//! TrueType outlines: the `loca` and `glyf` tables (OpenType
//! specification, "glyf — Glyph Data"; TrueType Reference Manual, "The
//! 'glyf' table").
//!
//! A glyph is decoded into its points (on- and off-curve, with the
//! contour ends), composites by assembling their components' points with
//! each component's transform; only then are the points turned into a
//! path. Keeping points until the end is what lets a component be placed
//! by matching points instead of by an offset.

use crate::font::{Font, GlyphId};
use crate::path::{Path, Point};
use crate::reader::{Reader, u16_at, u32_at};
use alloc::vec::Vec;

/// Simple glyph point flags.
const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME_OR_POSITIVE: u8 = 0x10;
const Y_SAME_OR_POSITIVE: u8 = 0x20;

/// Composite glyph component flags.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

/// How deep composites may nest, and how many points a glyph may have,
/// so a malicious font cannot recurse or allocate without bound.
const MAX_DEPTH: u32 = 8;
const MAX_POINTS: usize = 1 << 16;

/// A glyph's points: `(x, y, on_curve)` and the index of each contour's
/// last point.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Points {
    pub pts: Vec<(f32, f32, bool)>,
    pub ends: Vec<usize>,
}

/// The bytes of a glyph in `glyf` (empty for a glyph with no outline).
fn glyph_data<'a>(font: &Font<'a>, glyph: GlyphId) -> Option<&'a [u8]> {
    let t = font.tables();
    let (loca, glyf) = (t.loca?, t.glyf?);
    let g = usize::from(glyph.0);
    let (start, end) = if font.long_loca() {
        (u32_at(loca, 4 * g)? as usize, u32_at(loca, 4 * g + 4)? as usize)
    } else {
        (usize::from(u16_at(loca, 2 * g)?) * 2, usize::from(u16_at(loca, 2 * g + 2)?) * 2)
    };
    if start >= end {
        return Some(&[]);
    }
    glyf.get(start..end)
}

/// A glyph's outline in font units.
pub(crate) fn outline(font: &Font<'_>, glyph: GlyphId) -> Option<Path> {
    let mut pts = Points::default();
    decode(font, glyph, 0, &mut pts)?;
    Some(to_path(&pts))
}

/// Append the points of `glyph` to `out`.
fn decode(font: &Font<'_>, glyph: GlyphId, depth: u32, out: &mut Points) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let data = glyph_data(font, glyph)?;
    if data.is_empty() {
        return Some(());
    }
    let mut r = Reader::new(data);
    let contours = r.i16()?;
    r.skip(8)?; // the glyph's box
    if contours >= 0 { simple(r, contours as usize, out) } else { composite(font, r, depth, out) }
}

fn simple(mut r: Reader<'_>, contours: usize, out: &mut Points) -> Option<()> {
    let base = out.pts.len();
    let mut ends = Vec::with_capacity(contours);
    let mut last: Option<usize> = None;
    for _ in 0..contours {
        let e = usize::from(r.u16()?);
        // Contour ends must increase.
        if last.is_some_and(|l| e <= l) {
            return None;
        }
        last = Some(e);
        ends.push(e);
    }
    let n = last.map_or(0, |l| l + 1);
    if base + n > MAX_POINTS {
        return None;
    }
    let instructions = usize::from(r.u16()?);
    r.skip(instructions)?;
    let mut flags = Vec::with_capacity(n);
    while flags.len() < n {
        let f = r.u8()?;
        flags.push(f);
        if f & REPEAT != 0 {
            let count = r.u8()?;
            for _ in 0..count {
                if flags.len() == n {
                    break;
                }
                flags.push(f);
            }
        }
    }
    let mut xs = Vec::with_capacity(n);
    let mut x = 0i32;
    for &f in &flags {
        x += coordinate(&mut r, f, X_SHORT, X_SAME_OR_POSITIVE)?;
        xs.push(x);
    }
    let mut y = 0i32;
    for (i, &f) in flags.iter().enumerate() {
        y += coordinate(&mut r, f, Y_SHORT, Y_SAME_OR_POSITIVE)?;
        out.pts.push((xs[i] as f32, y as f32, f & ON_CURVE != 0));
    }
    out.ends.extend(ends.iter().map(|e| base + e));
    Some(())
}

/// One coordinate delta: a byte whose sign is the "same or positive" bit,
/// nothing (the same as before), or a signed word.
fn coordinate(r: &mut Reader<'_>, flag: u8, short: u8, same_or_positive: u8) -> Option<i32> {
    Some(if flag & short != 0 {
        let d = i32::from(r.u8()?);
        if flag & same_or_positive != 0 { d } else { -d }
    } else if flag & same_or_positive != 0 {
        0
    } else {
        i32::from(r.i16()?)
    })
}

fn composite(font: &Font<'_>, mut r: Reader<'_>, depth: u32, out: &mut Points) -> Option<()> {
    loop {
        let flags = r.u16()?;
        let glyph = GlyphId(r.u16()?);
        let (arg1, arg2) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            if flags & ARGS_ARE_XY_VALUES != 0 {
                (i32::from(r.i16()?), i32::from(r.i16()?))
            } else {
                (i32::from(r.u16()?), i32::from(r.u16()?))
            }
        } else if flags & ARGS_ARE_XY_VALUES != 0 {
            (i32::from(r.i8()?), i32::from(r.i8()?))
        } else {
            (i32::from(r.u8()?), i32::from(r.u8()?))
        };
        // The transform x' = a*x + c*y + e, y' = b*x + d*y + f.
        let (mut a, mut b, mut c, mut d) = (1.0f32, 0.0f32, 0.0f32, 1.0f32);
        if flags & WE_HAVE_A_SCALE != 0 {
            a = r.f2dot14()?;
            d = a;
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            a = r.f2dot14()?;
            d = r.f2dot14()?;
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            a = r.f2dot14()?;
            b = r.f2dot14()?;
            c = r.f2dot14()?;
            d = r.f2dot14()?;
        }
        let mut part = Points::default();
        decode(font, glyph, depth + 1, &mut part)?;
        if out.pts.len() + part.pts.len() > MAX_POINTS {
            return None;
        }
        for p in &mut part.pts {
            let (x, y) = (p.0, p.1);
            p.0 = a * x + c * y;
            p.1 = b * x + d * y;
        }
        let (e, f) = if flags & ARGS_ARE_XY_VALUES != 0 {
            let (mut e, mut f) = (arg1 as f32, arg2 as f32);
            // Offsets are in the parent's units unless the font asks for
            // them to be transformed too (the OpenType default).
            if flags & SCALED_COMPONENT_OFFSET != 0 {
                (e, f) = (a * e + c * f, b * e + d * f);
            }
            (e, f)
        } else {
            // Point matching: move the component so that its point arg2
            // lands on point arg1 of what is assembled so far.
            let parent = out.pts.get(usize::try_from(arg1).ok()?)?;
            let child = part.pts.get(usize::try_from(arg2).ok()?)?;
            (parent.0 - child.0, parent.1 - child.1)
        };
        let base = out.pts.len();
        out.pts.extend(part.pts.iter().map(|p| (p.0 + e, p.1 + f, p.2)));
        out.ends.extend(part.ends.iter().map(|i| base + i));
        if flags & MORE_COMPONENTS == 0 {
            return Some(());
        }
    }
}

/// Turn contours of points into a path. Two off-curve points in a row
/// imply an on-curve point halfway between them; a contour starts at an
/// on-curve point (its first, else its last, else the implied point
/// between the last and the first).
pub(crate) fn to_path(p: &Points) -> Path {
    let mut path = Path::new();
    let mut start = 0;
    for &end in &p.ends {
        if end < start || end >= p.pts.len() {
            break;
        }
        let contour = &p.pts[start..=end];
        start = end + 1;
        if contour.len() < 2 {
            continue;
        }
        let pt = |i: usize| Point::new(contour[i].0, contour[i].1);
        let on = |i: usize| contour[i].2;
        let n = contour.len();
        let (first, skip) = if on(0) {
            (pt(0), 1)
        } else if on(n - 1) {
            (pt(n - 1), 0)
        } else {
            (pt(n - 1).mid(pt(0)), 0)
        };
        // Points left to walk: indices skip..n, minus the last when it was
        // used as the start.
        let last = if !on(0) && on(n - 1) { n - 1 } else { n };
        path.move_to(first);
        let mut control: Option<Point> = None;
        for i in skip..last {
            let q = pt(i);
            if on(i) {
                match control.take() {
                    Some(c) => path.quad_to(c, q),
                    None => path.line_to(q),
                }
            } else {
                if let Some(c) = control {
                    path.quad_to(c, c.mid(q));
                }
                control = Some(q);
            }
        }
        match control {
            Some(c) => path.quad_to(c, first),
            None => path.line_to(first),
        }
        path.close();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Segment;
    use crate::testfont;

    fn pts(list: &[(f32, f32, bool)]) -> Points {
        Points { pts: list.to_vec(), ends: alloc::vec![list.len() - 1] }
    }

    #[test]
    fn implied_points_between_off_curve_points() {
        // A square of four off-curve points: a closed curve through the
        // edge midpoints, starting between the last and first points.
        let p = to_path(&pts(&[(0.0, 0.0, false), (0.0, 10.0, false), (10.0, 10.0, false), (10.0, 0.0, false)]));
        assert_eq!(
            p.segments,
            alloc::vec![
                Segment::MoveTo(Point::new(5.0, 0.0)),
                Segment::QuadTo(Point::new(0.0, 0.0), Point::new(0.0, 5.0)),
                Segment::QuadTo(Point::new(0.0, 10.0), Point::new(5.0, 10.0)),
                Segment::QuadTo(Point::new(10.0, 10.0), Point::new(10.0, 5.0)),
                Segment::QuadTo(Point::new(10.0, 0.0), Point::new(5.0, 0.0)),
                Segment::Close,
            ]
        );
        // Starting off-curve with an on-curve last point: start there.
        let p = to_path(&pts(&[(5.0, 10.0, false), (10.0, 0.0, true), (0.0, 0.0, true)]));
        assert_eq!(
            p.segments,
            alloc::vec![
                Segment::MoveTo(Point::new(0.0, 0.0)),
                Segment::QuadTo(Point::new(5.0, 10.0), Point::new(10.0, 0.0)),
                Segment::LineTo(Point::new(0.0, 0.0)),
                Segment::Close,
            ]
        );
    }

    #[test]
    fn simple_and_composite_glyphs_of_the_test_font() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        // 'A': a triangle with a triangular hole, all on-curve.
        let a = font.outline(GlyphId(1)).unwrap();
        let moves = a.segments.iter().filter(|s| matches!(s, Segment::MoveTo(_))).count();
        assert_eq!(moves, 2);
        assert_eq!(a.segments[0], Segment::MoveTo(Point::new(50.0, 0.0)));
        assert_eq!(a.segments[1], Segment::LineTo(Point::new(300.0, 700.0)));
        let b = a.bounds().unwrap();
        assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (50.0, 0.0, 550.0, 700.0));
        // 'Á': 'A' plus the accent scaled to half height and moved.
        let aa = font.outline(GlyphId(3)).unwrap();
        let b = aa.bounds().unwrap();
        assert_eq!((b.x_min, b.y_min, b.x_max, b.y_max), (50.0, 0.0, 550.0, 800.0));
        assert_eq!(&aa.segments[..a.segments.len()], &a.segments[..]);
        assert_eq!(aa.segments[a.segments.len()], Segment::MoveTo(Point::new(200.0, 750.0)));
        // The space has no outline.
        assert_eq!(font.outline(GlyphId(4)), None);
        assert_eq!(font.outline(GlyphId(999)), None);
    }

    #[test]
    fn composites_that_recurse_forever_are_refused() {
        let data = testfont::build_with_self_reference();
        let font = Font::parse(&data).unwrap();
        assert_eq!(font.outline(GlyphId(3)), None);
        assert!(font.outline(GlyphId(1)).is_some());
    }
}
