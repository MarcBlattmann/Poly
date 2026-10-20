//! Rectangles in pixel coordinates.

/// A rectangle of pixels: `x` and `y` of its top-left corner (which may be
/// off-canvas, even negative), `width` and `height`. Empty when either size
/// is zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// One past the rightmost column, without overflow.
    pub const fn right(&self) -> i64 {
        self.x as i64 + self.width as i64
    }

    /// One past the bottom row.
    pub const fn bottom(&self) -> i64 {
        self.y as i64 + self.height as i64
    }

    /// The rectangle from (`left`, `top`) up to but excluding (`right`,
    /// `bottom`), clamped to what a `Rect` can hold; empty if inverted.
    pub(crate) fn from_edges(left: i64, top: i64, right: i64, bottom: i64) -> Rect {
        let clamp = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64);
        let (left, top) = (clamp(left), clamp(top));
        if right <= left || bottom <= top {
            return Rect::new(left as i32, top as i32, 0, 0);
        }
        let size = |len: i64| len.min(u32::MAX as i64) as u32;
        Rect::new(left as i32, top as i32, size(right - left), size(bottom - top))
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        let (x, y) = (x as i64, y as i64);
        x >= self.x as i64 && x < self.right() && y >= self.y as i64 && y < self.bottom()
    }

    /// The part of `self` inside `other` (empty if they do not overlap).
    pub fn intersect(&self, other: &Rect) -> Rect {
        Rect::from_edges(
            (self.x as i64).max(other.x as i64),
            (self.y as i64).max(other.y as i64),
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        )
    }

    /// The smallest rectangle holding both; an empty rectangle adds
    /// nothing.
    pub fn union(&self, other: &Rect) -> Rect {
        if other.is_empty() {
            return *self;
        }
        if self.is_empty() {
            return *other;
        }
        Rect::from_edges(
            (self.x as i64).min(other.x as i64),
            (self.y as i64).min(other.y as i64),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// Moved by (`dx`, `dy`), saturating at the coordinate limits.
    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x.saturating_add(dx), self.y.saturating_add(dy), self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_clips_and_can_be_empty() {
        let a = Rect::new(-5, -5, 10, 10);
        let screen = Rect::new(0, 0, 100, 50);
        assert_eq!(a.intersect(&screen), Rect::new(0, 0, 5, 5));
        assert!(Rect::new(100, 0, 5, 5).intersect(&screen).is_empty());
        assert!(Rect::new(0, 0, 0, 5).intersect(&screen).is_empty());
        let huge = Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX);
        assert_eq!(huge.intersect(&screen), screen);
        let far = Rect::new(i32::MAX - 1, 0, u32::MAX, 10);
        assert_eq!(far.right(), i32::MAX as i64 - 1 + u32::MAX as i64);
        assert!(far.intersect(&screen).is_empty());
    }

    #[test]
    fn union_ignores_empty_rectangles() {
        let a = Rect::new(1, 2, 3, 4);
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(Rect::new(50, 50, 0, 0).union(&a), a);
        assert_eq!(a.union(&Rect::new(10, 0, 1, 1)), Rect::new(1, 0, 10, 6));
    }

    #[test]
    fn contains_and_offset() {
        let a = Rect::new(1, 2, 3, 4);
        assert!(a.contains(1, 2) && a.contains(3, 5));
        assert!(!a.contains(4, 2) && !a.contains(1, 6) && !a.contains(0, 2));
        assert_eq!(a.offset(-1, 10), Rect::new(0, 12, 3, 4));
        assert_eq!(Rect::new(i32::MAX, 0, 1, 1).offset(5, 0).x, i32::MAX);
    }
}
