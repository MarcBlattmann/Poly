//! The rows of elliptic and round-rectangle regions, as Windows makes them
//! (`specs/windows/gdi32.dll/CreateEllipticRgn.yaml` steps 1-7 and
//! `CreateRoundRectRgn.yaml`, which the elliptic region is the special case
//! of). All coordinates here are in sixteenths of a pixel, relative to the
//! region's origin, until the last step makes pixels.
//!
//! The shape is a polygon of four cubic Bezier quarter arcs (corner arcs
//! with straight edges between them for a round rectangle), each arc
//! flattened by halving with exact fractions, its points rounded to the
//! nearest sixteenth (halves up); every pixel row keeps the pixels whose
//! centres lie from the polygon's left crossing (included) to its right
//! crossing (excluded) on the line through the row's centres. The sheets
//! say the rule is exact below 192 pixels; Windows' fixed-point flattening
//! of bigger arcs can differ by a sixteenth, which is not reproduced.

use alloc::vec::Vec;

/// Fraction bits of the exact points while flattening: halving adds at most
/// three bits per level, and values stay below `2^31 * 2^60` (`i128`).
const FRAC: u32 = 60;
/// Deepest halving (3 bits per level must stay within [`FRAC`]).
const MAX_DEPTH: u32 = 20;
/// `4 * (sqrt(2) - 1) / 3` in double precision (the sheet's `k`).
const K: f64 = 0.552_284_749_830_793_6;

type Pt = (i64, i64);
/// A point with [`FRAC`] fraction bits.
type Fx = (i128, i128);

/// The largest whole number not above `x`.
fn floor_f(x: f64) -> i64 {
    let t = x as i64;
    if (t as f64) > x { t - 1 } else { t }
}

/// The smallest whole number not below `x`.
fn ceil_f(x: f64) -> i64 {
    let t = x as i64;
    if (t as f64) < x { t + 1 } else { t }
}

fn to_fx(p: Pt) -> Fx {
    (i128::from(p.0) << FRAC, i128::from(p.1) << FRAC)
}

/// Round to the nearest whole sixteenth, halves towards +infinity.
fn round_half_up(v: i128) -> i64 {
    ((v + (1i128 << (FRAC - 1))) >> FRAC) as i64
}

fn mid(a: Fx, b: Fx) -> Fx {
    ((a.0 + b.0) >> 1, (a.1 + b.1) >> 1)
}

/// `3 * max |second difference| <= 32` sixteenths, for x and y.
fn is_flat(q: &[Fx; 4]) -> bool {
    let limit = 32i128 << FRAC;
    let d = |a: i128, b: i128, c: i128| (a - 2 * b + c).abs();
    let m = d(q[0].0, q[1].0, q[2].0)
        .max(d(q[1].0, q[2].0, q[3].0))
        .max(d(q[0].1, q[1].1, q[2].1))
        .max(d(q[1].1, q[2].1, q[3].1));
    3 * m <= limit
}

/// Append the rounded end points of the flattened arc `p` (not its start).
fn flatten(p: [Pt; 4], out: &mut Vec<Pt>) {
    let mut stack: Vec<([Fx; 4], u32)> = alloc::vec![([to_fx(p[0]), to_fx(p[1]), to_fx(p[2]), to_fx(p[3])], 0)];
    while let Some((q, depth)) = stack.pop() {
        if depth >= MAX_DEPTH || is_flat(&q) {
            out.push((round_half_up(q[3].0), round_half_up(q[3].1)));
            continue;
        }
        let (m01, m12, m23) = (mid(q[0], q[1]), mid(q[1], q[2]), mid(q[2], q[3]));
        let (m012, m123) = (mid(m01, m12), mid(m12, m23));
        let m = mid(m012, m123);
        // The first half is treated first: push it last.
        stack.push(([m, m123, m23, q[3]], depth + 1));
        stack.push(([q[0], m01, m012, m], depth + 1));
    }
}

/// Where a polygon's edges meet the line `y = c`: the smallest and the
/// largest crossing as exact fractions `(numerator, denominator > 0)`.
fn crossings(poly: &[Pt], c: i64) -> Option<((i128, i128), (i128, i128))> {
    let less = |a: (i128, i128), b: (i128, i128)| a.0 * b.1 < b.0 * a.1;
    let mut lo: Option<(i128, i128)> = None;
    let mut hi: Option<(i128, i128)> = None;
    let mut take = |v: (i128, i128)| {
        if lo.is_none_or(|l| less(v, l)) {
            lo = Some(v);
        }
        if hi.is_none_or(|h| less(h, v)) {
            hi = Some(v);
        }
    };
    let n = poly.len();
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let (ylo, yhi) = (p.1.min(q.1), p.1.max(q.1));
        if c < ylo || c > yhi {
            continue;
        }
        if p.1 == q.1 {
            // A horizontal edge on the line counts with both its ends.
            take((i128::from(p.0), 1));
            take((i128::from(q.0), 1));
        } else {
            let dy = i128::from(q.1) - i128::from(p.1);
            let num = i128::from(p.0) * dy + i128::from(c - p.1) * (i128::from(q.0) - i128::from(p.0));
            if dy > 0 { take((num, dy)) } else { take((-num, -dy)) }
        }
    }
    lo.zip(hi)
}

/// `ceil(a / b)` for `b > 0`.
fn ceil_div(a: i128, b: i128) -> i128 {
    -((-a).div_euclid(b))
}

/// The closed polygon of a `w` x `h` pixel shape with corner radii `ra`,
/// `rb` (sixteenths; `a`, `b` for the ellipse).
fn polygon(w: i64, h: i64, ra: i64, rb: i64) -> Vec<Pt> {
    let (x0, y0) = (4, 4);
    let (x1, y1) = (16 * (w - 1) - 4, 16 * (h - 1) - 4);
    let ka = ceil_f(K * ra as f64);
    let kb = floor_f(K * rb as f64);
    let mut v: Vec<Pt> = Vec::new();
    v.push((x1, y0 + rb));
    flatten([(x1, y0 + rb), (x1, y0 + rb - kb), (x1 - ra + ka, y0), (x1 - ra, y0)], &mut v);
    v.push((x0 + ra, y0)); // the straight top edge
    flatten([(x0 + ra, y0), (x0 + ra - ka, y0), (x0, y0 + rb - kb), (x0, y0 + rb)], &mut v);
    v.push((x0, y1 - rb)); // left edge
    flatten([(x0, y1 - rb), (x0, y1 - rb + kb), (x0 + ra - ka, y1), (x0 + ra, y1)], &mut v);
    v.push((x1 - ra, y1)); // bottom edge
    flatten([(x1 - ra, y1), (x1 - ra + ka, y1), (x1, y1 - rb + kb), (x1, y1 - rb)], &mut v);
    // The right edge closes the polygon; equal neighbours merge.
    v.dedup();
    if v.len() > 1 && v.first() == v.last() {
        v.pop();
    }
    v
}

/// The runs of a `w` x `h` shape with corner radii `ra`, `rb` in
/// sixteenths: `(row, left, right)` with the right end excluded, rows
/// `-1 ..= h-1` that have pixels, in order, relative to the origin.
fn rows(w: i64, h: i64, ra: i64, rb: i64) -> Vec<(i64, i64, i64)> {
    let poly = polygon(w, h, ra, rb);
    let mut out = Vec::new();
    for y in -1..h {
        let Some(((ln, ld), (rn, rd))) = crossings(&poly, 16 * y + 8) else { continue };
        // Smallest x with 16x + 8 >= XL, and the first x with 16x + 8 >= XR.
        let x0 = ceil_div(ln - 8 * ld, 16 * ld);
        let x1 = ceil_div(rn - 8 * rd, 16 * rd);
        if x1 > x0 {
            out.push((y, x0 as i64, x1 as i64));
        }
    }
    out
}

/// `A = 8 * (W - 1) - 4`: the ellipse's radius across a side of `n` pixels.
fn radius(n: i64) -> i64 {
    8 * (n - 1) - 4
}

/// The corner radius for a corner size `c` on a side of `n` pixels:
/// `min(|c|, n) * radius / n` rounded to the nearest, halves up; 0 when
/// that is negative or `n` is 0.
fn corner(c: i64, n: i64) -> i64 {
    if n == 0 {
        return 0;
    }
    let e = c.abs().min(n);
    (2 * e * radius(n) + n).div_euclid(2 * n).max(0)
}

/// The rows of the round rectangle of `w` x `h` pixels with corner sizes
/// `cw`, `ch` (`CreateRoundRectRgn`): `(row, left, right)`, right excluded.
pub fn round_rect_rows(w: i64, h: i64, cw: i64, ch: i64) -> Vec<(i64, i64, i64)> {
    rows(w, h, corner(cw, w), corner(ch, h))
}

/// The rows of the ellipse of `w` x `h` pixels (`CreateEllipticRgn`).
pub fn ellipse_rows(w: i64, h: i64) -> Vec<(i64, i64, i64)> {
    rows(w, h, radius(w), radius(h))
}

// ----- RoundRect's outline (`specs/windows/gdi32.dll/RoundRect.yaml` `outline_1px`, `outline_wide`)
//
// Here the centre of pixel (x, y) is (16x, 16y), relative to the box's top-left pixel.

/// The closed path `RoundRect` strokes for a box of `w` x `h` pixels (its right and bottom edges
/// one pixel inside the call's box) with corner sizes `cw`, `ch` (absolute values), flattened as
/// the regions are: four Bezier arcs joined by straight edges, counter-clockwise from the right
/// edge, in sixteenths.
pub fn outline_path(w: i64, h: i64, cw: i64, ch: i64) -> Vec<Pt> {
    let (x0, y0, x1, y1) = (0, 0, 16 * (w - 1), 16 * (h - 1));
    // 8 * c * (n - 1) / n rounded, halves up; at most half the side.
    let r = |c: i64, n: i64| {
        ((16 * c.unsigned_abs().min(1 << 40) as i64 * (n - 1) + n).div_euclid(2 * n)).min(8 * (n - 1)).max(0)
    };
    let (ra, rb) = (r(cw, w), r(ch, h));
    let ka = ceil_f(K * ra as f64);
    let kb = floor_f(K * rb as f64);
    let mut v: Vec<Pt> = Vec::new();
    v.push((x1, y0 + rb));
    flatten([(x1, y0 + rb), (x1, y0 + rb - kb), (x1 - ra + ka, y0), (x1 - ra, y0)], &mut v);
    v.push((x0 + ra, y0));
    flatten([(x0 + ra, y0), (x0 + ra - ka, y0), (x0, y0 + rb - kb), (x0, y0 + rb)], &mut v);
    v.push((x0, y1 - rb));
    flatten([(x0, y1 - rb), (x0, y1 - rb + kb), (x0 + ra - ka, y1), (x0 + ra, y1)], &mut v);
    v.push((x1 - ra, y1));
    flatten([(x1 - ra, y1), (x1 - ra + ka, y1), (x1, y1 - rb + kb), (x1, y1 - rb)], &mut v);
    v.dedup();
    if v.len() > 1 && v.first() == v.last() {
        v.pop();
    }
    v
}

/// Whether point `p` lies in the diamond of pixel (`x`, `y`): `|du| + |dv| < 8`, or its right
/// or bottom vertex.
fn in_diamond(p: Pt, x: i64, y: i64) -> bool {
    let (du, dv) = (p.0 - 16 * x, p.1 - 16 * y);
    du.abs() + dv.abs() < 8 || (du, dv) == (8, 0) || (du, dv) == (0, 8)
}

/// Whether the closed segment `p`..`q` meets the diamond of pixel (`x`, `y`).
fn meets_diamond(p: Pt, q: Pt, x: i64, y: i64) -> bool {
    let (cx, cy) = (16 * x, 16 * y);
    if in_diamond(p, x, y) || in_diamond(q, x, y) {
        return true;
    }
    let (du, dv) = (q.0 - p.0, q.1 - p.1);
    // The L1 distance along the segment is smallest at an end or where u or v passes the centre:
    // check those parameters t = n / d exactly.
    let below = |n: i64, d: i64| -> bool {
        if d <= 0 || n < 0 || n > d {
            return false;
        }
        let a = (p.0 - cx) * d + n * du;
        let b = (p.1 - cy) * d + n * dv;
        a.abs() + b.abs() < 8 * d
    };
    let at = |num: i64, den: i64| if den < 0 { below(-num, -den) } else { below(num, den) };
    if du != 0 && at(cx - p.0, du) {
        return true;
    }
    if dv != 0 && at(cy - p.1, dv) {
        return true;
    }
    // The two vertices that belong to the diamond.
    let on = |v: Pt| {
        let cross = (v.0 - p.0) * dv - (v.1 - p.1) * du;
        cross == 0 && (v.0 - p.0) * du + (v.1 - p.1) * dv >= 0 && (v.0 - q.0) * du + (v.1 - q.1) * dv <= 0
    };
    on((cx + 8, cy)) || on((cx, cy + 8))
}

/// `outline_1px`: the pixels a 1-pixel pen lights along the closed `path`: each edge lights
/// the pixels whose diamond it meets, less the one its end point lies in.
pub fn outline_1px_pixels(path: &[Pt]) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::new();
    let n = path.len();
    if n == 0 {
        return out;
    }
    for i in 0..n {
        let (p, q) = (path[i], path[(i + 1) % n]);
        if n == 1 {
            // A single point: the pixel it lies in.
            out.extend(
                (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                    .map(|(dx, dy)| (p.0.div_euclid(16) + dx, p.1.div_euclid(16) + dy))
                    .filter(|&(x, y)| in_diamond(p, x, y)),
            );
            break;
        }
        let x_lo = p.0.min(q.0).div_euclid(16) - 1;
        let x_hi = p.0.max(q.0).div_euclid(16) + 1;
        let y_lo = p.1.min(q.1).div_euclid(16) - 1;
        let y_hi = p.1.max(q.1).div_euclid(16) + 1;
        for y in y_lo..=y_hi {
            for x in x_lo..=x_hi {
                if meets_diamond(p, q, x, y) && !in_diamond(q, x, y) {
                    out.push((x, y));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The pixels whose centres lie inside the closed `path` (on each row from the left crossing,
/// included, to the right one, excluded): the inside a brush fills.
pub fn path_interior(path: &[Pt], h: i64) -> Vec<(i64, i64, i64)> {
    let mut out = Vec::new();
    for y in 0..h {
        let Some(((ln, ld), (rn, rd))) = crossings(path, 16 * y) else { continue };
        let x0 = ceil_div(ln, 16 * ld);
        let x1 = ceil_div(rn, 16 * rd);
        if x1 > x0 {
            out.push((y, x0 as i64, x1 as i64));
        }
    }
    out
}

/// `outline_wide`: the pixels whose centres lie closer than `n / 2` pixels to the closed `path`
/// (a disc swept along it); at exactly `n / 2` from a vertical (horizontal) edge a pixel to its
/// left (above it) counts. The sheet calls this an approximation of Windows' wide pens.
pub fn outline_wide_pixels(path: &[Pt], n: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    if path.is_empty() {
        return out;
    }
    let reach = 8 * n;
    let r2 = i128::from(reach) * i128::from(reach);
    let (mut x_lo, mut x_hi, mut y_lo, mut y_hi) = (i64::MAX, i64::MIN, i64::MAX, i64::MIN);
    for p in path {
        x_lo = x_lo.min(p.0);
        x_hi = x_hi.max(p.0);
        y_lo = y_lo.min(p.1);
        y_hi = y_hi.max(p.1);
    }
    let m = path.len();
    for y in (y_lo - reach).div_euclid(16) - 1..=(y_hi + reach).div_euclid(16) + 1 {
        for x in (x_lo - reach).div_euclid(16) - 1..=(x_hi + reach).div_euclid(16) + 1 {
            let c = (16 * x, 16 * y);
            let mut lit = false;
            for i in 0..m {
                let (p, q) = (path[i], path[(i + 1) % m]);
                let order = dist2_cmp(c, p, q, r2);
                if order == core::cmp::Ordering::Less {
                    lit = true;
                    break;
                }
                if order == core::cmp::Ordering::Equal {
                    // The tie rule of axis-parallel edges.
                    if p.0 == q.0 && c.0 < p.0 && c.1 >= p.1.min(q.1) && c.1 <= p.1.max(q.1) {
                        lit = true;
                        break;
                    }
                    if p.1 == q.1 && c.1 < p.1 && c.0 >= p.0.min(q.0) && c.0 <= p.0.max(q.0) {
                        lit = true;
                        break;
                    }
                }
            }
            if lit {
                out.push((x, y));
            }
        }
    }
    out
}

/// How the squared distance from `c` to the closed segment `p`..`q` compares with `r2`, exactly.
fn dist2_cmp(c: Pt, p: Pt, q: Pt, r2: i128) -> core::cmp::Ordering {
    let (du, dv) = (i128::from(q.0 - p.0), i128::from(q.1 - p.1));
    let (wu, wv) = (i128::from(c.0 - p.0), i128::from(c.1 - p.1));
    let len2 = du * du + dv * dv;
    let t = wu * du + wv * dv;
    if len2 == 0 || t <= 0 {
        return (wu * wu + wv * wv).cmp(&r2);
    }
    if t >= len2 {
        let (eu, ev) = (i128::from(c.0 - q.0), i128::from(c.1 - q.1));
        return (eu * eu + ev * ev).cmp(&r2);
    }
    // |w|^2 - t^2 / len2 against r2, multiplied through by len2.
    ((wu * wu + wv * wv) * len2 - t * t).cmp(&(r2 * len2))
}

#[cfg(test)]
mod tests;
