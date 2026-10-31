//! Glyph outlines as paths: closed contours of lines and quadratic
//! (TrueType) or cubic (CFF) Bézier curves.

use alloc::vec::Vec;

/// A point, in font units (y up) or pixels (y down), depending on context.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    pub(crate) fn lerp(self, other: Point, t: f32) -> Point {
        Point::new(self.x + (other.x - self.x) * t, self.y + (other.y - self.y) * t)
    }

    pub(crate) fn mid(self, other: Point) -> Point {
        self.lerp(other, 0.5)
    }
}

/// One drawing command of a [`Path`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    /// Start a new contour at the point.
    MoveTo(Point),
    /// A straight line from the current point.
    LineTo(Point),
    /// A quadratic curve: control point, end point.
    QuadTo(Point, Point),
    /// A cubic curve: two control points, end point.
    CubicTo(Point, Point, Point),
    /// Close the contour with a line back to its start.
    Close,
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

impl Bounds {
    fn include(&mut self, p: Point, first: bool) {
        if first {
            *self = Bounds { x_min: p.x, y_min: p.y, x_max: p.x, y_max: p.y };
        } else {
            self.x_min = self.x_min.min(p.x);
            self.y_min = self.y_min.min(p.y);
            self.x_max = self.x_max.max(p.x);
            self.y_max = self.y_max.max(p.y);
        }
    }
}

/// A glyph outline: contours that are filled by the non-zero winding rule.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub segments: Vec<Segment>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn move_to(&mut self, p: Point) {
        self.segments.push(Segment::MoveTo(p));
    }

    pub fn line_to(&mut self, p: Point) {
        self.segments.push(Segment::LineTo(p));
    }

    pub fn quad_to(&mut self, c: Point, p: Point) {
        self.segments.push(Segment::QuadTo(c, p));
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) {
        self.segments.push(Segment::CubicTo(c1, c2, p));
    }

    pub fn close(&mut self) {
        self.segments.push(Segment::Close);
    }

    /// The box around every point, control points included (so it holds
    /// the whole outline); `None` for an empty path.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut b = Bounds::default();
        let mut first = true;
        for s in &self.segments {
            let pts: &[Point] = match s {
                Segment::MoveTo(p) | Segment::LineTo(p) => core::slice::from_ref(p),
                Segment::QuadTo(c, p) => &[*c, *p],
                Segment::CubicTo(a, c, p) => &[*a, *c, *p],
                Segment::Close => &[],
            };
            for &p in pts {
                b.include(p, first);
                first = false;
            }
        }
        (!first).then_some(b)
    }

    /// Every point mapped through `f`.
    pub fn map(&self, f: impl Fn(Point) -> Point) -> Path {
        let segments = self
            .segments
            .iter()
            .map(|s| match *s {
                Segment::MoveTo(p) => Segment::MoveTo(f(p)),
                Segment::LineTo(p) => Segment::LineTo(f(p)),
                Segment::QuadTo(c, p) => Segment::QuadTo(f(c), f(p)),
                Segment::CubicTo(a, c, p) => Segment::CubicTo(f(a), f(c), f(p)),
                Segment::Close => Segment::Close,
            })
            .collect();
        Path { segments }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_cover_control_points() {
        let mut p = Path::new();
        assert_eq!(p.bounds(), None);
        p.move_to(Point::new(0.0, 0.0));
        p.quad_to(Point::new(5.0, 10.0), Point::new(10.0, 0.0));
        p.close();
        assert_eq!(p.bounds(), Some(Bounds { x_min: 0.0, y_min: 0.0, x_max: 10.0, y_max: 10.0 }));
        let q = p.map(|pt| Point::new(pt.x * 2.0, -pt.y));
        assert_eq!(q.bounds(), Some(Bounds { x_min: 0.0, y_min: -10.0, x_max: 20.0, y_max: 0.0 }));
    }
}
