//! World transformations (Learn: `SetWorldTransform`, `ModifyWorldTransform`,
//! `CombineTransform`, `SetGraphicsMode`, "Coordinate Spaces and
//! Transformations"): the `XFORM` matrix that maps world space to page space.
//!
//! ```text
//! x' = x * eM11 + y * eM21 + eDx
//! y' = x * eM12 + y * eM22 + eDy
//! ```
//!
//! Combining applies the first transformation and then the second (the
//! row-vector product `first * second`).

/// `XFORM` (wingdi.h): six `FLOAT`s, 24 bytes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

/// `GM_COMPATIBLE` / `GM_ADVANCED` (wingdi.h).
pub const GM_COMPATIBLE: i32 = 1;
pub const GM_ADVANCED: i32 = 2;
/// `ModifyWorldTransform` modes (wingdi.h).
pub const MWT_IDENTITY: u32 = 1;
pub const MWT_LEFTMULTIPLY: u32 = 2;
pub const MWT_RIGHTMULTIPLY: u32 = 3;

impl Xform {
    /// The default world transformation: identity, no offset.
    pub const IDENTITY: Xform = Xform { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0, dx: 0.0, dy: 0.0 };

    /// A translation only.
    pub fn translate(dx: f32, dy: f32) -> Xform {
        Xform { dx, dy, ..Xform::IDENTITY }
    }

    /// A scale about the origin.
    pub fn scale(sx: f32, sy: f32) -> Xform {
        Xform { m11: sx, m22: sy, ..Xform::IDENTITY }
    }

    /// From an `XFORM` in memory (little-endian `FLOAT`s).
    pub fn from_bytes(b: &[u8; 24]) -> Xform {
        let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Xform { m11: f(0), m12: f(4), m21: f(8), m22: f(12), dx: f(16), dy: f(20) }
    }

    /// As an `XFORM` in memory.
    pub fn to_bytes(self) -> [u8; 24] {
        let mut out = [0u8; 24];
        for (i, v) in [self.m11, self.m12, self.m21, self.m22, self.dx, self.dy].into_iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub fn is_identity(&self) -> bool {
        *self == Xform::IDENTITY
    }

    /// No rotation or shear: rectangles stay rectangles with edges along
    /// the axes (scaled, flipped or moved).
    pub fn is_axis_aligned(&self) -> bool {
        self.m12 == 0.0 && self.m21 == 0.0
    }

    /// Whether the matrix can be inverted (a transformation that maps the
    /// plane onto a line or a point cannot be one between spaces).
    pub fn invertible(&self) -> bool {
        let det = self.m11 * self.m22 - self.m12 * self.m21;
        det != 0.0 && det.is_finite() && [self.dx, self.dy].iter().all(|v| v.is_finite())
    }

    /// The transformation that undoes this one (`None`: not invertible).
    pub fn inverse(&self) -> Option<Xform> {
        if !self.invertible() {
            return None;
        }
        let det = self.m11 * self.m22 - self.m12 * self.m21;
        let (m11, m12, m21, m22) = (self.m22 / det, -self.m12 / det, -self.m21 / det, self.m11 / det);
        Some(Xform {
            m11,
            m12,
            m21,
            m22,
            dx: -(self.dx * m11 + self.dy * m21),
            dy: -(self.dx * m12 + self.dy * m22),
        })
    }

    /// `CombineTransform(out, first, second)`: `first`, then `second`.
    pub fn then(&self, second: &Xform) -> Xform {
        let (a, b) = (self, second);
        Xform {
            m11: a.m11 * b.m11 + a.m12 * b.m21,
            m12: a.m11 * b.m12 + a.m12 * b.m22,
            m21: a.m21 * b.m11 + a.m22 * b.m21,
            m22: a.m21 * b.m12 + a.m22 * b.m22,
            dx: a.dx * b.m11 + a.dy * b.m21 + b.dx,
            dy: a.dx * b.m12 + a.dy * b.m22 + b.dy,
        }
    }

    /// `ModifyWorldTransform`: the new transformation, `None` for a mode
    /// that is none of the three.
    pub fn modify(&self, by: &Xform, mode: u32) -> Option<Xform> {
        match mode {
            MWT_IDENTITY => Some(Xform::IDENTITY),
            // The given data is the left multiplicand: it applies first.
            MWT_LEFTMULTIPLY => Some(by.then(self)),
            MWT_RIGHTMULTIPLY => Some(self.then(by)),
            _ => None,
        }
    }

    /// A world point in page space (exact).
    pub fn apply_f(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.m11 + y * self.m21 + self.dx, x * self.m12 + y * self.m22 + self.dy)
    }

    /// A world point in page space, rounded to the nearest pixel.
    pub fn apply(&self, x: i32, y: i32) -> (i32, i32) {
        let (fx, fy) = self.apply_f(x as f32, y as f32);
        (round(fx), round(fy))
    }

    /// The page-space rectangle a world rectangle (`l`, `t`, `r`, `b`,
    /// right and bottom exclusive) covers: its corners transformed, and the
    /// box around them when a rotation or shear turns it.
    pub fn apply_rect(&self, l: i32, t: i32, r: i32, b: i32) -> (i32, i32, i32, i32) {
        let pts = [self.apply(l, t), self.apply(r, t), self.apply(l, b), self.apply(r, b)];
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        (x0, y0, x1, y1)
    }
}

/// Round half away from zero, saturating (no `f32::round` without `std`).
pub fn round(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    let r = if v >= 0.0 { v + 0.5 } else { v - 0.5 };
    // `as` saturates and truncates toward zero.
    r as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_applies_first_then_second() {
        let a = Xform::translate(10.0, 5.0);
        let b = Xform::scale(2.0, 3.0);
        let c = a.then(&b);
        // (1, 1) + (10, 5) = (11, 6); scaled: (22, 18).
        assert_eq!(c.apply(1, 1), (22, 18));
        let d = b.then(&a);
        // scaled: (2, 3); moved: (12, 8).
        assert_eq!(d.apply(1, 1), (12, 8));
    }

    #[test]
    fn modify_left_and_right() {
        let cur = Xform::translate(100.0, 0.0);
        let s = Xform::scale(2.0, 2.0);
        // Left multiply: the given data applies first (scale, then move).
        assert_eq!(cur.modify(&s, MWT_LEFTMULTIPLY).unwrap().apply(1, 1), (102, 2));
        // Right multiply: the current one first (move, then scale).
        assert_eq!(cur.modify(&s, MWT_RIGHTMULTIPLY).unwrap().apply(1, 1), (202, 2));
        assert_eq!(cur.modify(&s, MWT_IDENTITY), Some(Xform::IDENTITY));
        assert_eq!(cur.modify(&s, 4), None);
    }

    #[test]
    fn rotation_by_ninety_degrees() {
        // Learn's rotation: eM11 = cos, eM12 = sin, eM21 = -sin, eM22 = cos.
        let r = Xform { m11: 0.0, m12: 1.0, m21: -1.0, m22: 0.0, dx: 0.0, dy: 0.0 };
        assert_eq!(r.apply(10, 0), (0, 10));
        assert_eq!(r.apply(0, 10), (-10, 0));
        assert!(!r.is_axis_aligned());
        assert_eq!(r.apply_rect(0, 0, 10, 20), (-20, 0, 0, 10));
    }

    #[test]
    fn bytes_round_trip_and_invertibility() {
        let x = Xform { m11: 1.5, m12: 0.25, m21: -0.5, m22: 2.0, dx: -7.0, dy: 3.5 };
        assert_eq!(Xform::from_bytes(&x.to_bytes()), x);
        assert!(x.invertible());
        assert!(!Xform::scale(0.0, 1.0).invertible());
        assert!(Xform::IDENTITY.is_identity());
        let back = x.then(&x.inverse().unwrap());
        assert_eq!(back.apply(37, -12), (37, -12));
        assert_eq!(Xform::scale(0.0, 1.0).inverse(), None);
    }

    #[test]
    fn rounding() {
        assert_eq!(round(1.5), 2);
        assert_eq!(round(-1.5), -2);
        assert_eq!(round(1.49), 1);
        assert_eq!(round(f32::NAN), 0);
        assert_eq!(round(1e20), i32::MAX);
    }
}
