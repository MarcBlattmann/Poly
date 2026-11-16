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

#[cfg(test)]
mod tests;
