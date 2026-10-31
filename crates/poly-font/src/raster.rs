//! The rasteriser: exact anti-aliased coverage of filled outlines by
//! signed-area accumulation.
//!
//! Every edge of the outline, flattened into straight lines, adds to two
//! numbers per pixel cell it crosses: for the part of the edge inside a
//! cell, its signed height `d` (`+` going down, `-` going up) times the
//! fraction of the cell to the right of it goes to that cell, and the rest
//! of `d` to the cell after it. Summing a row from the left then gives,
//! for each pixel, the signed area of the outline inside it: cells right
//! of an edge see its whole height, cells it passes through the exact
//! trapezoid. The absolute value, clamped to 1, is the coverage under the
//! non-zero winding rule (overlapping contours of one direction saturate),
//! which is TrueType's and CFF's fill rule.
//!
//! Coordinates are pixels, y down; anything outside the canvas is clipped
//! (an edge left of it still covers the pixels to its right).

use crate::math::{ceil, floor, sqrt};
use crate::path::{Path, Point, Segment};
use alloc::vec;
use alloc::vec::Vec;

/// How far (in pixels) a flattened curve may stray from the true curve.
const TOLERANCE: f32 = 0.03;
/// The most line segments one curve is split into.
const MAX_SPLITS: u32 = 64;

/// An accumulation buffer for one bitmap.
#[derive(Clone, Debug)]
pub struct Rasterizer {
    width: usize,
    height: usize,
    /// Rows of `width + 2` cells: an edge right of the canvas lands in the
    /// spare cells and is ignored.
    acc: Vec<f32>,
}

impl Rasterizer {
    /// A rasteriser for a `width` × `height` bitmap.
    pub fn new(width: usize, height: usize) -> Rasterizer {
        Rasterizer { width, height, acc: vec![0.0; (width + 2) * height] }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Forget everything drawn, keeping the buffer.
    pub fn clear(&mut self) {
        self.acc.fill(0.0);
    }

    /// Add a straight edge.
    pub fn line(&mut self, p0: Point, p1: Point) {
        if p0.y == p1.y || !(p0.x.is_finite() && p0.y.is_finite() && p1.x.is_finite() && p1.y.is_finite()) {
            return;
        }
        let (dir, top, bottom) = if p0.y < p1.y { (1.0f32, p0, p1) } else { (-1.0f32, p1, p0) };
        let h = self.height as f32;
        let y_start = top.y.max(0.0);
        let y_end = bottom.y.min(h);
        if y_start >= y_end {
            return;
        }
        let dxdy = (bottom.x - top.x) / (bottom.y - top.y);
        let stride = self.width + 2;
        let w = self.width as f32;
        let mut x = top.x + dxdy * (y_start - top.y);
        let first_row = floor(y_start) as usize;
        let last_row = (ceil(y_end) as usize).min(self.height);
        for row in first_row..last_row {
            let row_top = (row as f32).max(y_start);
            let row_bottom = ((row + 1) as f32).min(y_end);
            let dy = row_bottom - row_top;
            if dy <= 0.0 {
                continue;
            }
            let x_next = x + dxdy * dy;
            let d = dy * dir;
            let line = &mut self.acc[row * stride..(row + 1) * stride];
            let (xa, xb) = if x < x_next { (x, x_next) } else { (x_next, x) };
            let (xa, xb) = (xa.clamp(0.0, w), xb.clamp(0.0, w));
            let ia = floor(xa) as usize;
            let ib = floor(xb) as usize;
            if ia == ib || xb - xa < 1e-6 {
                // Within one cell: the trapezoid's width at its middle.
                let frac = (xa + xb) * 0.5 - ia as f32;
                line[ia] += d * (1.0 - frac);
                line[ia + 1] += d * frac;
            } else {
                // Across cells: each gets the share of `d` for the part of
                // the edge inside it.
                let per_x = d / (xb - xa);
                for (i, cell) in (ia..=ib).enumerate() {
                    let c = cell as f32;
                    let x0 = if i == 0 { xa } else { c };
                    let x1 = if cell == ib { xb } else { c + 1.0 };
                    if x1 <= x0 {
                        continue;
                    }
                    let dd = per_x * (x1 - x0);
                    let frac = (x0 + x1) * 0.5 - c;
                    line[cell] += dd * (1.0 - frac);
                    line[cell + 1] += dd * frac;
                }
            }
            x = x_next;
        }
    }

    /// Add a quadratic Bézier edge, flattened.
    pub fn quad(&mut self, p0: Point, c: Point, p1: Point) {
        // The curve strays at most |p0 - 2c + p1| / 4 from its chord, and a
        // split into n pieces cuts that by n².
        let dev = len(p0.x - 2.0 * c.x + p1.x, p0.y - 2.0 * c.y + p1.y) * 0.25;
        let n = splits(dev);
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let p = p0.lerp(c, t).lerp(c.lerp(p1, t), t);
            self.line(prev, p);
            prev = p;
        }
    }

    /// Add a cubic Bézier edge, flattened.
    pub fn cubic(&mut self, p0: Point, c1: Point, c2: Point, p1: Point) {
        let d1 = len(p0.x - 2.0 * c1.x + c2.x, p0.y - 2.0 * c1.y + c2.y);
        let d2 = len(c1.x - 2.0 * c2.x + p1.x, c1.y - 2.0 * c2.y + p1.y);
        let n = splits(d1.max(d2) * 0.75);
        let mut prev = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let a = p0.lerp(c1, t);
            let b = c1.lerp(c2, t);
            let cc = c2.lerp(p1, t);
            let p = a.lerp(b, t).lerp(b.lerp(cc, t), t);
            self.line(prev, p);
            prev = p;
        }
    }

    /// Add every contour of `path`, each point mapped through `to_pixels`
    /// first. Open contours are closed.
    pub fn path(&mut self, path: &Path, to_pixels: impl Fn(Point) -> Point) {
        let mut start = Point::default();
        let mut cur = Point::default();
        let mut open = false;
        for s in &path.segments {
            match *s {
                Segment::MoveTo(p) => {
                    if open {
                        self.line(cur, start);
                    }
                    start = to_pixels(p);
                    cur = start;
                    open = true;
                }
                Segment::LineTo(p) => {
                    let p = to_pixels(p);
                    self.line(cur, p);
                    cur = p;
                }
                Segment::QuadTo(c, p) => {
                    let (c, p) = (to_pixels(c), to_pixels(p));
                    self.quad(cur, c, p);
                    cur = p;
                }
                Segment::CubicTo(c1, c2, p) => {
                    let (c1, c2, p) = (to_pixels(c1), to_pixels(c2), to_pixels(p));
                    self.cubic(cur, c1, c2, p);
                    cur = p;
                }
                Segment::Close => {
                    if open {
                        self.line(cur, start);
                        cur = start;
                        open = false;
                    }
                }
            }
        }
        if open {
            self.line(cur, start);
        }
    }

    /// The coverage of each pixel, 0.0 to 1.0, row by row.
    pub fn coverage_f32(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.width * self.height);
        let stride = self.width + 2;
        for row in 0..self.height {
            let mut sum = 0.0f32;
            for &a in &self.acc[row * stride..row * stride + self.width] {
                sum += a;
                out.push(sum.abs().min(1.0));
            }
        }
        out
    }

    /// The coverage of each pixel, 0 to 255, row by row.
    pub fn coverage(&self) -> Vec<u8> {
        self.coverage_f32().into_iter().map(|c| (c * 255.0 + 0.5) as u8).collect()
    }
}

fn len(x: f32, y: f32) -> f32 {
    sqrt(x * x + y * y)
}

fn splits(deviation: f32) -> u32 {
    if !deviation.is_finite() {
        return 1;
    }
    (ceil(sqrt(deviation / TOLERANCE)) as u32).clamp(1, MAX_SPLITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(r: &mut Rasterizer, x0: f32, y0: f32, x1: f32, y1: f32) {
        let mut p = Path::new();
        p.move_to(Point::new(x0, y0));
        p.line_to(Point::new(x1, y0));
        p.line_to(Point::new(x1, y1));
        p.line_to(Point::new(x0, y1));
        p.close();
        r.path(&p, |p| p);
    }

    #[test]
    fn pixel_aligned_rectangle_is_solid() {
        let mut r = Rasterizer::new(6, 4);
        rect(&mut r, 1.0, 1.0, 4.0, 3.0);
        let c = r.coverage();
        #[rustfmt::skip]
        let want = [
            0, 0, 0, 0, 0, 0,
            0, 255, 255, 255, 0, 0,
            0, 255, 255, 255, 0, 0,
            0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(c, want);
    }

    #[test]
    fn fractional_edges_give_partial_coverage() {
        let mut r = Rasterizer::new(4, 2);
        rect(&mut r, 0.25, 0.5, 2.5, 2.0);
        let c = r.coverage_f32();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(c[0], 0.375), "{}", c[0]); // 0.75 wide × 0.5 high
        assert!(close(c[1], 0.5));
        assert!(close(c[2], 0.25));
        assert!(close(c[3], 0.0));
        assert!(close(c[4], 0.75));
        assert!(close(c[5], 1.0));
        assert!(close(c[6], 0.5));
    }

    #[test]
    fn a_diagonal_halves_its_pixels() {
        // A right triangle covering exactly the lower-left half of a 4×4
        // square: diagonal pixels half covered, the area sums to 8.
        let mut r = Rasterizer::new(4, 4);
        let mut p = Path::new();
        p.move_to(Point::new(0.0, 0.0));
        p.line_to(Point::new(4.0, 4.0));
        p.line_to(Point::new(0.0, 4.0));
        p.close();
        r.path(&p, |p| p);
        let c = r.coverage_f32();
        for i in 0..4 {
            assert!((c[i * 4 + i] - 0.5).abs() < 1e-5);
        }
        let total: f32 = c.iter().sum();
        assert!((total - 8.0).abs() < 1e-4, "{total}");
    }

    #[test]
    fn winding_direction_and_holes() {
        // Both directions fill; an inner contour of the other direction is
        // a hole; one of the same direction saturates.
        for (inner_reversed, want) in [(true, 0.0), (false, 1.0)] {
            let mut r = Rasterizer::new(6, 6);
            rect(&mut r, 0.0, 0.0, 6.0, 6.0);
            if inner_reversed {
                rect(&mut r, 4.0, 2.0, 2.0, 4.0);
            } else {
                rect(&mut r, 2.0, 2.0, 4.0, 4.0);
            }
            let c = r.coverage_f32();
            assert_eq!(c[0], 1.0);
            assert_eq!(c[2 * 6 + 2], want);
        }
    }

    #[test]
    fn clipped_shapes_and_bad_input() {
        let mut r = Rasterizer::new(4, 4);
        // Starts left of and above the canvas, ends right of and below it.
        rect(&mut r, -10.0, -10.0, 2.0, 20.0);
        rect(&mut r, 3.5, 1.0, 50.0, 2.0);
        let c = r.coverage_f32();
        assert_eq!(&c[0..4], &[1.0, 1.0, 0.0, 0.0]);
        assert_eq!(&c[4..8], &[1.0, 1.0, 0.0, 0.5]);
        r.line(Point::new(f32::NAN, 0.0), Point::new(1.0, 3.0));
        r.line(Point::new(0.0, f32::INFINITY), Point::new(1.0, 3.0));
        assert_eq!(r.coverage_f32(), c);
        let mut empty = Rasterizer::new(0, 0);
        rect(&mut empty, 0.0, 0.0, 1.0, 1.0);
        assert!(empty.coverage().is_empty());
    }

    #[test]
    fn curves_approach_a_circle() {
        // A circle of radius 8 from four quadratic arcs (each control point
        // at a corner of the bounding square is a poor circle) and from
        // cubic arcs with the usual 0.5523 handle: areas near pi*r^2.
        let (cx, cy, rad) = (10.0f32, 10.0f32, 8.0f32);
        let k = 0.552_284_8 * rad;
        let mut p = Path::new();
        p.move_to(Point::new(cx + rad, cy));
        p.cubic_to(Point::new(cx + rad, cy + k), Point::new(cx + k, cy + rad), Point::new(cx, cy + rad));
        p.cubic_to(Point::new(cx - k, cy + rad), Point::new(cx - rad, cy + k), Point::new(cx - rad, cy));
        p.cubic_to(Point::new(cx - rad, cy - k), Point::new(cx - k, cy - rad), Point::new(cx, cy - rad));
        p.cubic_to(Point::new(cx + k, cy - rad), Point::new(cx + rad, cy - k), Point::new(cx + rad, cy));
        p.close();
        let mut r = Rasterizer::new(20, 20);
        r.path(&p, |p| p);
        let area: f32 = r.coverage_f32().iter().sum();
        // The flattened curve is a polygon inside the circle (here of 40
        // sides, 0.4% smaller).
        let want = core::f32::consts::PI * rad * rad;
        assert!(area < want && want - area < 1.0, "{area} vs {want}");
        // Quadratic arcs with their control points on the corners of the
        // square around the circle: parabolic segments, whose area is known.
        let mut q = Path::new();
        q.move_to(Point::new(cx + rad, cy));
        q.quad_to(Point::new(cx + rad, cy + rad), Point::new(cx, cy + rad));
        q.quad_to(Point::new(cx - rad, cy + rad), Point::new(cx - rad, cy));
        q.quad_to(Point::new(cx - rad, cy - rad), Point::new(cx, cy - rad));
        q.quad_to(Point::new(cx + rad, cy - rad), Point::new(cx + rad, cy));
        let mut r = Rasterizer::new(20, 20);
        r.path(&q, |p| p);
        let area: f32 = r.coverage_f32().iter().sum();
        // Diamond 2r² plus four parabolic caps, each 2/3 of its triangle
        // (r²/2): 2r² + 4·(2/3)(r²/2) = (10/3) r².
        let want = 10.0 / 3.0 * rad * rad;
        assert!(area < want && want - area < 1.0, "{area} vs {want}");
    }
}
