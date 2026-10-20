//! Sequence numbers: 32-bit values compared modulo 2^32 (RFC 9293 3.4).
//! `a < b` means `b` is less than 2^31 ahead of `a`.

use core::cmp::Ordering;
use core::fmt;
use core::ops::{Add, Sub};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Seq(pub u32);

impl Seq {
    /// Bytes from `earlier` to `self`, which must not be behind `earlier`
    /// (zero if it is).
    pub fn since(self, earlier: Seq) -> usize {
        let d = self.0.wrapping_sub(earlier.0) as i32;
        if d < 0 { 0 } else { d as usize }
    }

    /// The later of two sequence numbers.
    pub fn max(self, other: Seq) -> Seq {
        if self > other { self } else { other }
    }

    /// `lo <= self < hi` in sequence space.
    pub fn in_range(self, lo: Seq, hi: Seq) -> bool {
        lo <= self && self < hi
    }
}

impl PartialOrd for Seq {
    fn partial_cmp(&self, other: &Seq) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Not a total order over all values (it is circular), but consistent for
// any set of values within 2^31 of each other, which is all TCP compares.
impl Ord for Seq {
    fn cmp(&self, other: &Seq) -> Ordering {
        (self.0.wrapping_sub(other.0) as i32).cmp(&0)
    }
}

impl Add<usize> for Seq {
    type Output = Seq;
    fn add(self, n: usize) -> Seq {
        Seq(self.0.wrapping_add(n as u32))
    }
}

impl Sub<usize> for Seq {
    type Output = Seq;
    fn sub(self, n: usize) -> Seq {
        Seq(self.0.wrapping_sub(n as u32))
    }
}

impl fmt::Debug for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraparound_comparisons() {
        let a = Seq(u32::MAX - 10);
        let b = a + 20;
        assert_eq!(b.0, 9);
        assert!(a < b);
        assert!(b > a);
        assert_eq!(b.since(a), 20);
        assert_eq!(a.since(b), 0);
        assert_eq!(a.max(b), b);
        assert!((a + 5).in_range(a, b));
        assert!(!b.in_range(a, b));
        assert_eq!(b - 20, a);
        // Half the space away flips the order.
        assert!(Seq(0) < Seq(0x7fff_ffff));
        assert!(Seq(0) > Seq(0x8000_0001));
    }
}
