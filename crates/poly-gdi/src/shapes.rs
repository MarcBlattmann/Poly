//! The pixel rules of `FrameRgn` and `RoundRect` and the mapping modes' extents, on regions in
//! device coordinates (`specs/windows/gdi32.dll/FrameRgn.yaml`, `RoundRect.yaml`,
//! `OffsetWindowOrgEx.yaml`; Learn's "Mapping Modes and Translations" and "Hatch styles").

use crate::region::{Rect, Region};
use alloc::vec::Vec;

/// The pixels `p` of `rgn` for which every pixel `q` with `|q.x - p.x| <= w` and
/// `|q.y - p.y| <= h` is in `rgn` too (an erosion by a `(2w+1) x (2h+1)` box), with `w, h >= 0`.
/// Built by region operations, so its cost follows the region's rectangles, not its area: the
/// intersection of the region moved by `-w..=w` across and `-h..=h` down, made by doubling.
pub fn erode(rgn: &Region, w: i64, h: i64) -> Region {
    let b = rgn.bounds();
    let (width, height) = (i64::from(b.right) - i64::from(b.left), i64::from(b.bottom) - i64::from(b.top));
    if rgn.is_empty() || 2 * w + 1 > width || 2 * h + 1 > height {
        return Region::new();
    }
    let across = run_of(rgn, 2 * w + 1, true).offset_by(w as i32, 0);
    run_of(&across, 2 * h + 1, false).offset_by(0, h as i32)
}

/// The pixels `p` of `rgn` whose next `n - 1` neighbours to the right (`across`) or below are in
/// `rgn` too (`n >= 1`).
fn run_of(rgn: &Region, n: i64, across: bool) -> Region {
    let shift = |r: &Region, d: i64| if across { r.offset_by(-(d as i32), 0) } else { r.offset_by(0, -(d as i32)) };
    // `acc` holds the runs of length `len`, `pow` those of length `step` (a power of two).
    let (mut acc, mut len) = (None::<Region>, 0i64);
    let (mut pow, mut step) = (rgn.clone(), 1i64);
    let mut left = n;
    while left > 0 {
        if left & 1 == 1 {
            acc = Some(match acc {
                None => pow.clone(),
                Some(a) => a.intersect(&shift(&pow, len)),
            });
            len += step;
        }
        left >>= 1;
        if left > 0 {
            pow = pow.intersect(&shift(&pow, step));
            step *= 2;
            if pow.is_empty() {
                return Region::new();
            }
        }
    }
    acc.unwrap_or_default()
}

/// `FrameRgn`'s pixels (`FrameRgn.yaml` `painted_pixels`): those of `rgn` with a pixel outside it
/// within `w` across and `h` down (`w, h >= 1`).
pub fn frame(rgn: &Region, w: i64, h: i64) -> Region {
    rgn.subtract(&erode(rgn, w, h))
}

/// The pixels of `rgn` with one of their four neighbours outside it.
pub fn border4(rgn: &Region) -> Region {
    let inner = rgn
        .intersect(&rgn.offset_by(1, 0))
        .intersect(&rgn.offset_by(-1, 0))
        .intersect(&rgn.offset_by(0, 1))
        .intersect(&rgn.offset_by(0, -1));
    rgn.subtract(&inner)
}

/// The region of the pixels `x0..=x1`, `y0..=y1` with corners `cw` x `ch` (as
/// `CreateRoundRectRgn(x0, y0, x1 + 2, y1 + 2, cw, ch)`, whose right and bottom edges are left
/// out); empty when the box is.
fn round_box(x0: i32, y0: i32, x1: i32, y1: i32, cw: i32, ch: i32) -> Region {
    if x1 < x0 || y1 < y0 {
        return Region::new();
    }
    Region::round_rect(x0, y0, x1.saturating_add(2), y1.saturating_add(2), cw, ch).unwrap_or_default()
}

/// What `RoundRect` paints in device coordinates: `(brush, pen)` for the pixel box `l..r`,
/// `t..b` (right and bottom exclusive, ordered), corners `cw` x `ch` (absolute values), and a pen
/// `width` pixels wide (`None`: the null pen) that is centred on the outline or, with
/// `inside_frame` (`PS_INSIDEFRAME`), kept inside the box (`RoundRect.yaml`). `brush` says whether
/// a brush is selected: a wide pen on a rounded box takes another route without one
/// (`outline_wide` (1), `crate::rrwide`).
#[allow(clippy::too_many_arguments)]
pub fn round_rect(
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    cw: i32,
    ch: i32,
    pen: Option<(i32, bool)>,
    brush: bool,
) -> (Region, Region) {
    if r <= l || b <= t || (r - l == 1 && b - t == 1) {
        return (Region::new(), Region::new());
    }
    let Some((width, inside)) = pen else {
        // `fill_null_pen`: exactly CreateRoundRectRgn(l, t, r, b, w, h).
        return (Region::round_rect(l, t, r, b, cw, ch).unwrap_or_default(), Region::new());
    };
    let n = width.max(1);
    let (x1, y1) = (r - 1, b - 1);
    let (w, h) = (i64::from(r) - i64::from(l), i64::from(b) - i64::from(t));
    // `outline_wide`, `inside_frame`, `brush_pixels`: a solid or inside-frame pen of width 2 to 6
    // on a rounded box (a corner of 1 x 1 included: `rect_wide`) widens the outline path with
    // its pen polygon; an inside-frame pen on a box no larger than the width is not specified.
    if (2..=6).contains(&n) && cw != 0 && ch != 0 && !(inside && (w <= i64::from(n) || h <= i64::from(n))) {
        let (cw, ch) = (i64::from(cw), i64::from(ch));
        if let Some((fill, pen)) = crate::rrwide::round_rect_wide(w, h, cw, ch, i64::from(n), inside, brush) {
            return (pixels_region(l, t, &fill), pixels_region(l, t, &pen));
        }
    }
    // `corner_size`: 0 on either axis, or 1 on both, is a plain rectangle.
    let plain = cw == 0 || ch == 0 || (cw == 1 && ch == 1);
    if n == 1 || (!inside && !plain) {
        if n == 1 && w == 2 && h == 2 {
            // `box`: a 2x2 box draws (l, t) and (l, t+1), no brush.
            return (Region::new(), rect_region(l, t, l + 1, t + 2));
        }
        let (cw, ch) = if plain { (0, 0) } else { (i64::from(cw), i64::from(ch)) };
        let path = crate::rgnshape::outline_path(w, h, cw, ch);
        let lit = if n == 1 {
            // `outline_1px`: the stroke of the path.
            crate::rgnshape::outline_1px_pixels(&path)
        } else {
            // `outline_wide` (an approximation the sheet gives for rounded boxes).
            crate::rgnshape::outline_wide_pixels(&path, i64::from(n))
        };
        let pen = pixels_region(l, t, &lit);
        // `fill_with_pen`: the brush fills the inside of the path where the pen is not.
        let runs: Vec<Rect> = crate::rgnshape::path_interior(&path, h)
            .into_iter()
            .map(|(y, a, z)| Rect::new(l + a as i32, t + y as i32, l + z as i32, t + y as i32 + 1))
            .collect();
        let brush = Region::from_rects(&runs).subtract(&pen);
        return (brush, pen);
    }
    if inside && !plain && w > i64::from(n) && h > i64::from(n) {
        // PS_INSIDEFRAME on a rounded box (no rule in the sheet): the wide stroke of the path of
        // the box made smaller so that the pen stays inside it.
        let (a, z) = (n / 2, (n - 1) / 2);
        let (sw, sh) = (w - i64::from(a + z), h - i64::from(a + z));
        let shrink = |c: i32| i64::from(c).saturating_sub(i64::from(n - 1)).max(1);
        let path = crate::rgnshape::outline_path(sw, sh, shrink(cw), shrink(ch));
        let pen = pixels_region(l + a, t + a, &crate::rgnshape::outline_wide_pixels(&path, i64::from(n)));
        let runs: Vec<Rect> = crate::rgnshape::path_interior(&path, sh)
            .into_iter()
            .map(|(y, x0, x1)| Rect::new(l + a + x0 as i32, t + a + y as i32, l + a + x1 as i32, t + a + y as i32 + 1))
            .collect();
        return (Region::from_rects(&runs).subtract(&pen), pen);
    }
    let grow = |c: i32, by: i32| if c == 0 { 0 } else { c.saturating_add(by).max(0) };
    let (outer, inner) = if inside {
        // `pen_width`: an inside-frame pen stays within the box, the brush shrinks by its width.
        (round_box(l, t, x1, y1, cw, ch), round_box(l + n, t + n, x1 - n, y1 - n, grow(cw, -2 * n), grow(ch, -2 * n)))
    } else {
        // A pen of width n centred on the outline covers c - n/2 ..= c + (n-1)/2 of each edge c.
        let (lo, hi) = (n / 2, (n - 1) / 2);
        (
            round_box(l - lo, t - lo, x1 + hi, y1 + hi, grow(cw, n - 1), grow(ch, n - 1)),
            round_box(l + hi + 1, t + hi + 1, x1 - lo - 1, y1 - lo - 1, grow(cw, -(n + 1)), grow(ch, -(n + 1))),
        )
    };
    (inner.clone(), outer.subtract(&inner))
}

/// The region of `pixels` (offsets from `l`, `t`): their runs on each row.
fn pixels_region(l: i32, t: i32, pixels: &[(i64, i64)]) -> Region {
    let mut v: Vec<(i64, i64)> = pixels.iter().map(|&(x, y)| (y, x)).collect();
    v.sort_unstable();
    let mut runs: Vec<Rect> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let (y, x0) = v[i];
        let mut x1 = x0 + 1;
        i += 1;
        while i < v.len() && v[i] == (y, x1) {
            x1 += 1;
            i += 1;
        }
        let (y, x0, x1) = (t + y as i32, l + x0 as i32, l + x1 as i32);
        runs.push(Rect::new(x0, y, x1, y + 1));
    }
    Region::from_rects(&runs)
}

/// Whether hatch `style` (`HS_HORIZONTAL` 0 .. `HS_DIAGCROSS` 5, `wingdi.h`) has a pixel at
/// (`x`, `y`) of its 8x8 cell, relative to the brush origin: the lines lie on row and column 3,
/// the diagonals through (3, 3) and (4, 4) (as recorded: `FrameRgn.yaml` `brush`, `RoundRect.yaml`
/// `hatch_brush`).
pub fn hatch_bit(style: u32, x: i32, y: i32) -> bool {
    let (x, y) = (x.rem_euclid(8), y.rem_euclid(8));
    let horizontal = y == 3;
    let vertical = x == 3;
    let down = x == y;
    let up = x + y == 7;
    match style {
        0 => horizontal,
        1 => vertical,
        2 => down,
        3 => up,
        4 => horizontal || vertical,
        5 => down || up,
        _ => false,
    }
}

/// The ternary raster operation (pattern and destination only) of binary raster operation `rop2`
/// (`R2_BLACK` 1 .. `R2_WHITE` 16, `wingdi.h`): `rop2 - 1` is the truth table over (pen, dest)
/// with bit `2 * pen + dest` (`R2_COPYPEN` 13: pen; `R2_NOT` 6: not dest). Out of range:
/// `R2_COPYPEN`.
pub fn rop2_code(rop2: i32) -> u8 {
    let t = if (1..=16).contains(&rop2) { (rop2 - 1) as u32 } else { 12 };
    let mut code = 0u8;
    for k in 0..8u32 {
        let (p, d) = ((k >> 2) & 1, k & 1);
        if t >> (2 * p + d) & 1 == 1 {
            code |= 1 << k;
        }
    }
    code
}

/// Mapping modes (`wingdi.h`).
pub const MM_TEXT: i32 = 1;
pub const MM_ISOTROPIC: i32 = 7;
pub const MM_ANISOTROPIC: i32 = 8;

/// The window and viewport extents a fixed mapping mode sets, for a device of 96 pixels per
/// logical inch (y runs up in all of them): `MM_LOMETRIC` 0.1 mm, `MM_HIMETRIC` 0.01 mm,
/// `MM_LOENGLISH` 0.01 inch, `MM_HIENGLISH` 0.001 inch, `MM_TWIPS` 1/1440 inch; `MM_ISOTROPIC`
/// starts as `MM_LOMETRIC` (`OffsetWindowOrgEx.yaml` `mapping_text`). `None` for `MM_TEXT` and
/// `MM_ANISOTROPIC` (they keep or reset extents themselves) and for other values.
pub fn fixed_extents(mode: i32) -> Option<((i32, i32), (i32, i32))> {
    let units_per_inch = match mode {
        2 | MM_ISOTROPIC => 254,
        3 => 2540,
        4 => 100,
        5 => 1000,
        6 => 1440,
        _ => return None,
    };
    Some(((units_per_inch, units_per_inch), (96, -96)))
}

/// `MM_ISOTROPIC`: the viewport extent made to give both axes the same scale, the smaller one
/// (Learn, `SetWindowExtEx` / `SetViewportExtEx`: "the viewport extent is adjusted so that the
/// logical units are the same size on both axes"); signs are kept.
pub fn isotropic(window: (i32, i32), viewport: (i32, i32)) -> (i32, i32) {
    let (wx, wy) = (i64::from(window.0).abs(), i64::from(window.1).abs());
    let (vx, vy) = (i64::from(viewport.0).abs(), i64::from(viewport.1).abs());
    if wx == 0 || wy == 0 {
        return viewport;
    }
    let sign = |v: i32| if v < 0 { -1 } else { 1 };
    // Compare vx / wx with vy / wy.
    if vx * wy > vy * wx {
        let nx = (vy * wx / wy).clamp(1, i64::from(i32::MAX)) as i32;
        (sign(viewport.0) * nx, viewport.1)
    } else {
        let ny = (vx * wy / wx).clamp(1, i64::from(i32::MAX)) as i32;
        (viewport.0, sign(viewport.1) * ny)
    }
}

/// `v * num / den` rounded to the nearest integer, halves away from zero (`den != 0`).
pub fn scale_round(v: i64, num: i64, den: i64) -> i64 {
    let (n, d) = (i128::from(v) * i128::from(num), i128::from(den));
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    let q = if n >= 0 { (2 * n + d) / (2 * d) } else { -((-2 * n + d) / (2 * d)) };
    q.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// A rectangle's region (helper for callers that build regions of device boxes).
pub fn rect_region(l: i32, t: i32, r: i32, b: i32) -> Region {
    Region::from_rect(Rect::new(l, t, r, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    /// The pixels of a region as (x, y) pairs, sorted.
    fn pixels(r: &Region) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for rc in r.rects() {
            for y in rc.top..rc.bottom {
                for x in rc.left..rc.right {
                    v.push((x, y));
                }
            }
        }
        v.sort();
        v
    }

    /// The frame by the sheet's definition, pixel by pixel.
    fn frame_by_definition(r: &Region, w: i32, h: i32) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for (x, y) in pixels(r) {
            let mut outside = false;
            for dy in -h..=h {
                for dx in -w..=w {
                    outside |= !r.contains_point(x + dx, y + dy);
                }
            }
            if outside {
                out.push((x, y));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn frame_of_a_rectangle_matches_the_sheet_example() {
        let r = rect_region(10, 6, 30, 20);
        // w = h = 1: rows 6 and 19 whole, x 10 and 29 between.
        assert_eq!(pixels(&frame(&r, 1, 1)).len(), 20 + 20 + 2 * 12);
        assert_eq!(pixels(&frame(&r, 2, 2)).len(), 2 * 20 * 2 + 2 * 2 * 10);
        assert_eq!(pixels(&frame(&r, 3, 1)).len(), 2 * 20 + 2 * 3 * 12);
        // Wider than half the region: everything.
        assert_eq!(pixels(&frame(&r, 30, 2)).len(), 20 * 14);
        let one = rect_region(5, 5, 6, 6);
        assert_eq!(pixels(&frame(&one, 3, 3)), [(5, 5)]);
    }

    #[test]
    fn frame_follows_the_box_rule_on_complex_regions() {
        let outer = rect_region(4, 4, 35, 24);
        let hole = rect_region(14, 10, 24, 18);
        let ring = outer.subtract(&hole);
        let two = rect_region(0, 0, 10, 10).union(&rect_region(12, 0, 20, 10));
        let ell = Region::ellipse(0, 0, 23, 17).unwrap();
        let l = rect_region(0, 0, 20, 5).union(&rect_region(0, 5, 6, 20));
        for r in [&ring, &two, &ell, &l] {
            for (w, h) in [(1, 1), (2, 2), (3, 1), (1, 4), (5, 5), (4, 3)] {
                assert_eq!(pixels(&frame(r, w.into(), h.into())), frame_by_definition(r, w, h), "{w}x{h}");
            }
        }
    }

    #[test]
    fn border4_of_a_box_is_its_outline() {
        let r = rect_region(0, 0, 5, 4);
        assert_eq!(pixels(&border4(&r)).len(), 5 * 4 - 3 * 2);
    }

    #[test]
    fn round_rect_plain_boxes_and_pens() {
        // Corner 0, 1-pixel pen: the outline of the box and the inside (RoundRect.yaml `box`).
        let (brush, pen) = round_rect(10, 8, 40, 30, 0, 0, Some((1, false)), true);
        assert_eq!((pixels(&pen).len(), pixels(&brush).len()), (100, 560));
        // Wider centred pens grow outward (pen_width).
        for (n, pen_n, brush_n) in [(2, 200, 513), (3, 300, 468), (5, 500, 384)] {
            let (brush, pen) = round_rect(10, 8, 40, 30, 0, 0, Some((n, false)), true);
            assert_eq!((pixels(&pen).len(), pixels(&brush).len()), (pen_n, brush_n), "width {n}");
        }
        for (n, pen_n, brush_n) in [(1, 100, 560), (3, 276, 384), (5, 420, 240)] {
            let (brush, pen) = round_rect(10, 8, 40, 30, 0, 0, Some((n, true)), true);
            assert_eq!((pixels(&pen).len(), pixels(&brush).len()), (pen_n, brush_n), "inside {n}");
        }
        // The null pen: CreateRoundRectRgn's region; a 1x1 box: nothing.
        let (brush, pen) = round_rect(0, 0, 10, 8, 0, 0, None, true);
        assert_eq!((pixels(&brush).len(), pixels(&pen).len()), (9 * 7, 0));
        assert!(round_rect(5, 5, 6, 6, 0, 0, Some((1, false)), true).1.is_empty());
    }

    #[test]
    fn one_pixel_outline_of_a_circle_and_small_boxes() {
        // RoundRect.yaml `outline_1px`, the 21x21 circle of rr_pen1 at (2,2): its top run is
        // x 10..=14 on row 2, its widest rows x 2 and 22 on rows 10..=14.
        let (brush, pen) = round_rect(2, 2, 23, 23, 21, 21, Some((1, false)), true);
        let row = |y: i32| (0..30).filter(|&x| pen.contains_point(x, y)).collect::<Vec<_>>();
        assert_eq!(row(2), [10, 11, 12, 13, 14]);
        for y in 10..=14 {
            assert_eq!(row(y), [2, 22], "row {y}");
        }
        // The brush never overlaps the pen.
        assert!(brush.intersect(&pen).is_empty());
        // `box`: 2x2 draws (l, t) and (l, t+1); 3x3 with corners four pen pixels and one brush pixel.
        assert_eq!(pixels(&round_rect(5, 5, 7, 7, 4, 4, Some((1, false)), true).1), [(5, 5), (5, 6)]);
        let (b3, p3) = round_rect(5, 5, 8, 8, 3, 3, Some((1, false)), true);
        assert_eq!((pixels(&p3).len(), pixels(&b3)), (4, std::vec![(6, 6)]));
    }

    #[test]
    fn hatches_and_raster_operations() {
        // HS_DIAGCROSS through (3,3) and (4,4); HS_HORIZONTAL on row 3.
        assert!(hatch_bit(5, 0, 0) && hatch_bit(5, 7, 0) && hatch_bit(5, 3, 4) && !hatch_bit(5, 1, 0));
        assert!(hatch_bit(0, 5, 3) && hatch_bit(0, 5, 11) && !hatch_bit(0, 5, 4));
        assert!(hatch_bit(4, 3, 0) && hatch_bit(4, 0, 3) && !hatch_bit(4, 0, 0));
        assert_eq!(rop2_code(13), 0xF0); // R2_COPYPEN = PATCOPY
        assert_eq!(rop2_code(6), 0x55); // R2_NOT = DSTINVERT
        assert_eq!(rop2_code(1), 0x00); // R2_BLACK = BLACKNESS
        assert_eq!(rop2_code(7), 0x5A); // R2_XORPEN = PATINVERT
        assert_eq!(rop2_code(11), 0xAA); // R2_NOP
    }

    #[test]
    fn mapping_extents_and_rounding() {
        let ((wx, _), (vx, vy)) = fixed_extents(2).unwrap();
        // MM_LOMETRIC: 10 logical units are 4 pixels; y runs up.
        assert_eq!(scale_round(10, i64::from(vx), i64::from(wx)), 4);
        assert_eq!(scale_round(-20, i64::from(vy), i64::from(wx)), 8);
        assert_eq!(scale_round(90, 96, 254), 34);
        assert_eq!(scale_round(-80, 96, 1440), -5);
        assert_eq!(scale_round(-5, 1, 2), -3);
        assert_eq!(isotropic((100, 100), (200, -300)), (200, -200));
        assert_eq!(isotropic((100, 50), (300, 50)), (100, 50));
    }
}
