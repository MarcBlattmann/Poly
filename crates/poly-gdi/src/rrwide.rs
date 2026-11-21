//! `RoundRect`'s wide pens (`specs/windows/gdi32.dll/RoundRect.yaml` `outline_wide`,
//! `inside_frame`, `brush_pixels`): the outline path of `outline_1px` widened by a polygonal pen
//! and filled with the non-zero rule at the pixel centres.
//!
//! Everything is in sixteenths of a pixel, relative to the top-left pixel of the call's box: the
//! centre of pixel (x, y) is (16x, 16y), y downwards.
//!
//! - **The path**: four Bezier arcs joined by straight sides, counter-clockwise on screen from the
//!   right side, flattened as the regions are (`rgnshape::flatten`).
//! - **The pen**: a convex, centrally symmetric polygon for each width from 2 to 6 ([`pen`]). For
//!   an edge of direction `d` its left vertex `L(d)` maximises `v.x * d.y - v.y * d.x`, the right
//!   one is `-L(d)`; a direction parallel to a pen edge takes the vertex the sheet names.
//! - **The routes**: with a brush every edge of the flattened polygon selects by its own direction
//!   (StrokeAndFillPath); with the null brush the first and last chords of an arc select by the
//!   arc's start and end tangents (StrokePath).
//! - **The contours**: the outer one runs forwards with the right vertices and the fans of the
//!   left turns, the inner one backwards with the left vertices; a fan at a joint on a pixel
//!   centre has its intermediate vertices moved one sixteenth inwards.
//! - **Filling**: a pixel is lit when the winding number of both contours at its centre is not
//!   zero; an edge counts for a point in its half-open y range strictly beside it.

use crate::rgnshape::{K, Pt, ceil_f, flatten, floor_f};
use alloc::vec::Vec;

/// Pixels `(x, y)` relative to the box's top-left pixel.
pub type Pixels = Vec<(i64, i64)>;

/// The pen polygons of widths 2 to 6, clockwise on screen from the right end of the top edge
/// (`outline_wide` (3)).
pub fn pen(n: i64) -> Option<&'static [Pt]> {
    Some(match n {
        2 => &[(8, -16), (16, 0), (8, 16), (-8, 16), (-16, 0), (-8, -16)],
        3 => &[(8, -24), (24, -8), (24, 8), (8, 24), (-8, 24), (-24, 8), (-24, -8), (-8, -24)],
        4 => &[
            (8, -32),
            (24, -24),
            (32, -8),
            (32, 8),
            (24, 24),
            (8, 32),
            (-8, 32),
            (-24, 24),
            (-32, 8),
            (-32, -8),
            (-24, -24),
            (-8, -32),
        ],
        5 => &[
            (8, -40),
            (24, -32),
            (32, -24),
            (40, -8),
            (40, 8),
            (32, 24),
            (24, 32),
            (8, 40),
            (-8, 40),
            (-24, 32),
            (-32, 24),
            (-40, 8),
            (-40, -8),
            (-32, -24),
            (-24, -32),
            (-8, -40),
        ],
        6 => &[
            (8, -48),
            (24, -40),
            (40, -24),
            (48, -8),
            (48, 8),
            (40, 24),
            (24, 40),
            (8, 48),
            (-8, 48),
            (-24, 40),
            (-40, 24),
            (-48, 8),
            (-48, -8),
            (-40, -24),
            (-24, -40),
            (-8, -48),
        ],
        _ => return None,
    })
}

/// The index of the left vertex `L(d)` of `pen` for direction `d` (`outline_wide` (3)): the
/// vertex maximising `v.x * d.y - v.y * d.x`; a direction parallel to the pen edge from `v_i` to
/// `v_(i+1)` (`k` vertices) takes `v_(i+1)` when `4i < k` or `4i >= 3k`, else `v_i`.
pub fn left_vertex(pen: &[Pt], d: Pt) -> usize {
    let k = pen.len();
    let score = |v: Pt| i128::from(v.0) * i128::from(d.1) - i128::from(v.1) * i128::from(d.0);
    let best = pen.iter().map(|&v| score(v)).max().unwrap_or(0);
    let tied: Vec<usize> = (0..k).filter(|&i| score(pen[i]) == best).collect();
    if tied.len() >= 2 {
        // Two neighbours (an edge parallel to d): i and i + 1 (mod k).
        let i = if tied.len() == 2 && tied[1] == tied[0] + 1 {
            tied[0]
        } else if tied.contains(&0) && tied.contains(&(k - 1)) {
            k - 1
        } else {
            tied[0]
        };
        return if 4 * i < k || 4 * i >= 3 * k { (i + 1) % k } else { i };
    }
    tied[0]
}

/// The index of `-pen[i]` (the polygon is centrally symmetric).
fn opposite(pen: &[Pt], i: usize) -> usize {
    (i + pen.len() / 2) % pen.len()
}

/// One edge of the flattened path: start, end, and the direction that selects its pen vertices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub a: Pt,
    pub b: Pt,
    pub dir: Pt,
}

/// The box and radii of a round-rect path in sixteenths (`outline_1px` (1), `inside_frame`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathBox {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
    pub ra: i64,
    pub rb: i64,
}

impl PathBox {
    /// `outline_1px` (1) for a box of `w` x `h` pixels and corners `cw` x `ch`.
    pub fn outline(w: i64, h: i64, cw: i64, ch: i64) -> PathBox {
        Self::with(0, 0, 16 * (w - 1), 16 * (h - 1), w, h, cw, ch, 1)
    }

    /// `inside_frame` for a pen of width `n`: the path box shrunk by `n / 2` pixel on every side.
    pub fn inside_frame(w: i64, h: i64, cw: i64, ch: i64, n: i64) -> PathBox {
        Self::with(8 * n, 8 * n, 16 * w - 8 * n, 16 * h - 8 * n, w, h, cw, ch, n)
    }

    /// The radii `floor((16 c (side - m) + side) / (2 side))`, each at most half the path's side.
    #[allow(clippy::too_many_arguments)]
    fn with(x0: i64, y0: i64, x1: i64, y1: i64, w: i64, h: i64, cw: i64, ch: i64, m: i64) -> PathBox {
        let r = |c: i64, side: i64, half: i64| {
            let c = c.unsigned_abs().min(1 << 40) as i64;
            ((16 * c * (side - m) + side).div_euclid(2 * side)).min(half).max(0)
        };
        let ra = r(cw, w, (x1 - x0) / 2);
        let rb = r(ch, h, (y1 - y0) / 2);
        PathBox { x0, y0, x1, y1, ra, rb }
    }

    /// The four arcs' Bezier points, in path order (`outline_1px` (1)).
    fn arcs(&self) -> [[Pt; 4]; 4] {
        let PathBox { x0, y0, x1, y1, ra, rb } = *self;
        let ka = ceil_f(K * ra as f64);
        let kb = floor_f(K * rb as f64);
        [
            [(x1, y0 + rb), (x1, y0 + rb - kb), (x1 - ra + ka, y0), (x1 - ra, y0)],
            [(x0 + ra, y0), (x0 + ra - ka, y0), (x0, y0 + rb - kb), (x0, y0 + rb)],
            [(x0, y1 - rb), (x0, y1 - rb + kb), (x0 + ra - ka, y1), (x0 + ra, y1)],
            [(x1 - ra, y1), (x1 - ra + ka, y1), (x1, y1 - rb + kb), (x1, y1 - rb)],
        ]
    }

    /// The flattened closed polygon (`outline_1px` (2)), repeated points dropped.
    pub fn polygon(&self) -> Vec<Pt> {
        self.edges(true).iter().map(|e| e.a).collect()
    }

    /// The edges of the flattened polygon, edges of length 0 dropped, with their selection
    /// directions: each its own with a brush (`brush`), the arcs' end tangents for their first and
    /// last chords with the null brush (`outline_wide` (4)).
    pub fn edges(&self, brush: bool) -> Vec<Edge> {
        let mut out: Vec<Edge> = Vec::new();
        let arcs = self.arcs();
        for (i, arc) in arcs.iter().enumerate() {
            let mut pts = alloc::vec![arc[0]];
            flatten(*arc, &mut pts);
            pts.dedup();
            let chords: Vec<(Pt, Pt)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
            let m = chords.len();
            for (j, &(a, b)) in chords.iter().enumerate() {
                let own = (b.0 - a.0, b.1 - a.1);
                let dir = if brush || m < 2 {
                    own
                } else if j == 0 {
                    first_nonzero(&[sub(arc[1], arc[0]), sub(arc[2], arc[0]), sub(arc[3], arc[0])], own)
                } else if j == m - 1 {
                    first_nonzero(&[sub(arc[3], arc[2]), sub(arc[3], arc[1]), sub(arc[3], arc[0])], own)
                } else {
                    own
                };
                out.push(Edge { a, b, dir });
            }
            // The straight side to the next arc.
            let (a, b) = (arc[3], arcs[(i + 1) % 4][0]);
            if a != b {
                out.push(Edge { a, b, dir: sub(b, a) });
            }
        }
        out
    }
}

fn sub(a: Pt, b: Pt) -> Pt {
    (a.0 - b.0, a.1 - b.1)
}

fn first_nonzero(c: &[Pt], fallback: Pt) -> Pt {
    c.iter().copied().find(|&d| d != (0, 0)).unwrap_or(fallback)
}

fn cross(a: Pt, b: Pt) -> i128 {
    i128::from(a.0) * i128::from(b.1) - i128::from(a.1) * i128::from(b.0)
}

/// The vertices strictly between `from` and `to`, going down the list (wrapping).
fn between_down(pen: &[Pt], from: usize, to: usize) -> Vec<Pt> {
    let k = pen.len();
    let mut out = Vec::new();
    let mut i = (from + k - 1) % k;
    while i != to {
        out.push(pen[i]);
        i = (i + k - 1) % k;
    }
    out
}

/// A fan vertex at joint `f`: moved one sixteenth towards the pen's centre when the joint lies on
/// a pixel centre.
fn fan_point(f: Pt, v: Pt) -> Pt {
    let v =
        if f.0.rem_euclid(16) == 0 && f.1.rem_euclid(16) == 0 { (v.0 - v.0.signum(), v.1 - v.1.signum()) } else { v };
    (f.0 + v.0, f.1 + v.1)
}

/// The outer and the inner contour of the widened closed polyline `edges` (`outline_wide` (4)).
pub fn contours(edges: &[Edge], pen: &[Pt]) -> (Vec<Pt>, Vec<Pt>) {
    let m = edges.len();
    let left: Vec<usize> = edges.iter().map(|e| left_vertex(pen, e.dir)).collect();
    let right: Vec<usize> = left.iter().map(|&l| opposite(pen, l)).collect();
    let add = |p: Pt, v: Pt| (p.0 + v.0, p.1 + v.1);
    let mut outer = Vec::new();
    for i in 0..m {
        let j = (i + 1) % m;
        let f = edges[i].b;
        outer.push(add(f, pen[right[i]]));
        if right[j] != right[i] {
            if cross(edges[i].dir, edges[j].dir) < 0 {
                for v in between_down(pen, right[i], right[j]) {
                    outer.push(fan_point(f, v));
                }
            } else {
                outer.push(f);
            }
            outer.push(add(f, pen[right[j]]));
        }
    }
    let mut inner = Vec::new();
    for i in (0..m).rev() {
        let p = (i + m - 1) % m;
        let a = edges[i].a;
        inner.push(add(a, pen[left[i]]));
        if left[p] != left[i] {
            if cross(edges[p].dir, edges[i].dir) < 0 {
                inner.push(a);
            } else {
                for v in between_down(pen, left[i], left[p]) {
                    inner.push(fan_point(a, v));
                }
            }
            inner.push(add(a, pen[left[p]]));
        }
    }
    (outer, inner)
}

/// The winding number of the closed `poly` at `p` (`outline_wide` (2)): an edge counts when the
/// point's y lies in its half-open range and the point lies strictly beside it, on the side the
/// edge's direction says; a point on an edge line does not count.
fn winding(poly: &[Pt], p: Pt) -> i32 {
    let n = poly.len();
    let mut w = 0;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let (up, lo, hi) = if a.1 < b.1 { (false, a, b) } else { (true, b, a) };
        if lo.1 == hi.1 || p.1 < lo.1 || p.1 >= hi.1 {
            continue;
        }
        // The edge's x at p.y against p.x: (p.x - x) * (hi.y - lo.y) = p.x*(dy) - lo.x*dy - (p.y-lo.y)*dx.
        let dy = i128::from(hi.1 - lo.1);
        let dx = i128::from(hi.0 - lo.0);
        let s = (i128::from(p.0) - i128::from(lo.0)) * dy - (i128::from(p.1) - i128::from(lo.1)) * dx;
        if s < 0 {
            // The point lies to the left of the edge (smaller x).
            w += if up { 1 } else { -1 };
        }
    }
    w
}

/// The pixels whose centres have a non-zero winding number over all of `polys`.
pub fn fill(polys: &[&[Pt]]) -> Vec<(i64, i64)> {
    let mut lo = (i64::MAX, i64::MAX);
    let mut hi = (i64::MIN, i64::MIN);
    for p in polys.iter().flat_map(|p| p.iter()) {
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0), hi.1.max(p.1));
    }
    let mut out = Vec::new();
    if lo.0 > hi.0 {
        return out;
    }
    for y in lo.1.div_euclid(16)..=hi.1.div_euclid(16) + 1 {
        for x in lo.0.div_euclid(16)..=hi.0.div_euclid(16) + 1 {
            let c = (16 * x, 16 * y);
            if polys.iter().map(|p| winding(p, c)).sum::<i32>() != 0 {
                out.push((x, y));
            }
        }
    }
    out
}

/// `RoundRect` with a solid (or inside-frame) pen of width `n` (2 to 6) on a rounded box of `w` x
/// `h` pixels with corners `cw` x `ch`: `(brush pixels, pen pixels)` relative to the box's top-left
/// pixel; `brush` says whether a brush is selected (the route of `outline_wide` (1)). `None` for a
/// width without a pen polygon.
pub fn round_rect_wide(
    w: i64,
    h: i64,
    cw: i64,
    ch: i64,
    n: i64,
    inside: bool,
    brush: bool,
) -> Option<(Pixels, Pixels)> {
    let pen = pen(n)?;
    let pb = if inside { PathBox::inside_frame(w, h, cw, ch, n) } else { PathBox::outline(w, h, cw, ch) };
    let edges = pb.edges(brush);
    if edges.is_empty() {
        return Some((Vec::new(), Vec::new()));
    }
    let (outer, inner) = contours(&edges, pen);
    let mut lit = fill(&[&outer, &inner]);
    lit.sort_unstable();
    let fill_px = if brush {
        let poly = pb.polygon();
        fill(&[&poly]).into_iter().filter(|p| lit.binary_search(p).is_err()).collect()
    } else {
        Vec::new()
    };
    Some((fill_px, lit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(px: &[(i64, i64)]) -> (i64, i64, i64, i64, usize) {
        let x0 = px.iter().map(|p| p.0).min().unwrap();
        let x1 = px.iter().map(|p| p.0).max().unwrap();
        let y0 = px.iter().map(|p| p.1).min().unwrap();
        let y1 = px.iter().map(|p| p.1).max().unwrap();
        (x0, y0, x1, y1, px.len())
    }

    /// `rr_pen_wide`'s rounded boxes (`RoundRect.tests/windows.json`): the bounding boxes and
    /// counts of pen and brush pixels, offsets from the box's top-left pixel (10, 8).
    #[test]
    fn wide_pens_draw_the_recorded_pixels() {
        // (W, H, corner, n, inside, pen bbox+count, brush bbox+count) in absolute pixels.
        type Case = (i64, i64, i64, i64, bool, (i64, i64, i64, i64, usize), (i64, i64, i64, i64, usize));
        let cases: &[Case] = &[
            (30, 22, 8, 2, false, (9, 7, 39, 29, 186), (11, 9, 37, 27, 502)),
            (30, 22, 8, 3, false, (9, 7, 40, 30, 280), (12, 10, 37, 27, 456)),
            (30, 22, 8, 5, false, (8, 6, 41, 31, 464), (13, 11, 36, 26, 380)),
            (30, 22, 8, 3, true, (10, 8, 39, 29, 256), (13, 11, 36, 26, 375)),
            (30, 22, 8, 5, true, (10, 8, 39, 29, 392), (15, 13, 34, 24, 235)),
            (26, 14, 30, 2, false, (10, 7, 35, 21, 127), (12, 10, 33, 19, 184)),
            (26, 14, 30, 3, false, (9, 7, 36, 22, 186), (12, 10, 33, 19, 164)),
            (26, 14, 30, 5, false, (8, 6, 37, 23, 304), (13, 11, 32, 18, 120)),
            (26, 14, 30, 3, true, (10, 8, 35, 21, 166), (14, 12, 32, 18, 113)),
            (26, 14, 30, 5, true, (10, 8, 35, 21, 242), (16, 14, 30, 16, 37)),
        ];
        for &(w, h, c, n, inside, pen_want, brush_want) in cases {
            let (brush, pen) = round_rect_wide(w, h, c, c, n, inside, true).unwrap();
            let shift = |b: (i64, i64, i64, i64, usize)| (b.0 + 10, b.1 + 8, b.2 + 10, b.3 + 8, b.4);
            assert_eq!(shift(bbox(&pen)), pen_want, "pen {w}x{h} corner {c} n {n} inside {inside}");
            assert_eq!(shift(bbox(&brush)), brush_want, "brush {w}x{h} corner {c} n {n} inside {inside}");
        }
    }

    /// `outline_wide` (3): the vertices a direction parallel to a pen edge takes.
    #[test]
    fn parallel_directions_take_the_named_vertices() {
        let p3 = pen(3).unwrap();
        let at = |d: Pt| p3[left_vertex(p3, d)];
        // 0 degrees (right), 45 (down-right on screen is +y), 90 (down) ... as the sheet lists them,
        // with the angle measured from +x towards +y.
        assert_eq!(at((1, 0)), (8, -24));
        assert_eq!(at((1, 1)), (24, -8));
        assert_eq!(at((0, 1)), (24, 8));
        assert_eq!(at((-1, 1)), (24, 8));
        assert_eq!(at((-1, 0)), (8, 24));
        assert_eq!(at((-1, -1)), (-8, 24));
        assert_eq!(at((0, -1)), (-24, 8));
        assert_eq!(at((1, -1)), (-8, -24));
    }

    /// `inside_frame`'s example: box (10,8,40,30), corner 8x8, n = 3, the first arc.
    #[test]
    fn the_inside_frame_path_is_the_sheets() {
        let pb = PathBox::inside_frame(30, 22, 8, 8, 3);
        let arc = pb.arcs()[0];
        // Absolute sixteenths: add 16 * (10, 8).
        let abs = |p: Pt| (p.0 + 160, p.1 + 128);
        assert_eq!(arc.map(abs), [(616, 207), (616, 177), (591, 152), (558, 152)]);
    }
}
