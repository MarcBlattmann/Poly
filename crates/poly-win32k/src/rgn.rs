//! Update regions (ADR 0131): sets of pixels as y-x banded rectangles, the
//! form Windows reports them in (InvalidateRect.yaml: "kept as y-x banded
//! rectangles"; GetUpdateRgn.yaml's region dumps).
//!
//! A region is a list of bands, top to bottom; a band is a run of rows
//! with the same x intervals, left to right, which neither touch nor
//! overlap; two bands one above the other with the same intervals are one
//! band. So equal sets of pixels have equal lists, and the rectangles come
//! out in the order the oracle prints them.

use crate::frame::Rect;
use alloc::vec::Vec;

/// A set of pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rgn {
    rects: Vec<Rect>,
}

/// How [`Rgn::combine`] joins two regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Union,
    Intersect,
    Subtract,
    Xor,
}

impl Rgn {
    pub fn new() -> Rgn {
        Rgn::default()
    }

    /// The pixels of `r` (none for an empty or inverted rectangle).
    pub fn rect(r: Rect) -> Rgn {
        if r.right > r.left && r.bottom > r.top { Rgn { rects: alloc::vec![r] } } else { Rgn::new() }
    }

    /// The union of rectangles.
    pub fn from_rects(rects: &[Rect]) -> Rgn {
        rects.iter().fold(Rgn::new(), |acc, &r| acc.combine(&Rgn::rect(r), Op::Union))
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// NULLREGION (1), SIMPLEREGION (2) or COMPLEXREGION (3), wingdi.h.
    pub fn kind(&self) -> i32 {
        match self.rects.len() {
            0 => 1,
            1 => 2,
            _ => 3,
        }
    }

    /// The bounding box (0,0,0,0 when empty).
    pub fn bounds(&self) -> Rect {
        let mut it = self.rects.iter();
        let Some(&first) = it.next() else { return Rect::default() };
        it.fold(first, |b, r| {
            Rect::new(b.left.min(r.left), b.top.min(r.top), b.right.max(r.right), b.bottom.max(r.bottom))
        })
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rgn {
        Rgn {
            rects: self.rects.iter().map(|r| Rect::new(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy)).collect(),
        }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.rects.iter().any(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
    }

    /// Whether the region covers all of `r`.
    pub fn covers(&self, r: Rect) -> bool {
        Rgn::rect(r).combine(self, Op::Subtract).is_empty()
    }

    pub fn union(&self, other: &Rgn) -> Rgn {
        self.combine(other, Op::Union)
    }

    pub fn subtract(&self, other: &Rgn) -> Rgn {
        self.combine(other, Op::Subtract)
    }

    pub fn intersect(&self, other: &Rgn) -> Rgn {
        self.combine(other, Op::Intersect)
    }

    /// `self` op `other`, by a sweep over every band edge of both.
    pub fn combine(&self, other: &Rgn, op: Op) -> Rgn {
        let mut ys: Vec<i32> = self.rects.iter().chain(&other.rects).flat_map(|r| [r.top, r.bottom]).collect();
        ys.sort_unstable();
        ys.dedup();
        let mut out: Vec<Rect> = Vec::new();
        // The intervals of the band last emitted, and where its rects start.
        let mut last: Option<Band> = None;
        for w in ys.windows(2) {
            let (y0, y1) = (w[0], w[1]);
            let a = intervals(&self.rects, y0);
            let b = intervals(&other.rects, y0);
            let ivs = merge(&a, &b, op);
            if ivs.is_empty() {
                last = None;
                continue;
            }
            match &mut last {
                // The same intervals right below: the band grows.
                Some((bottom, prev, start)) if *bottom == y0 && *prev == ivs => {
                    for r in &mut out[*start..] {
                        r.bottom = y1;
                    }
                    *bottom = y1;
                }
                _ => {
                    let start = out.len();
                    out.extend(ivs.iter().map(|&(l, r)| Rect::new(l, y0, r, y1)));
                    last = Some((y1, ivs, start));
                }
            }
        }
        Rgn { rects: out }
    }
}

/// A band last emitted: its bottom, its x intervals, where its rects start.
type Band = (i32, Vec<(i32, i32)>, usize);

/// The x intervals of `rects` on row `y`, sorted and merged.
fn intervals(rects: &[Rect], y: i32) -> Vec<(i32, i32)> {
    let mut v: Vec<(i32, i32)> =
        rects.iter().filter(|r| r.top <= y && y < r.bottom).map(|r| (r.left, r.right)).collect();
    v.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (l, r) in v {
        match out.last_mut() {
            Some(last) if l <= last.1 => last.1 = last.1.max(r),
            _ => out.push((l, r)),
        }
    }
    out
}

/// Two interval lists joined by `op`.
fn merge(a: &[(i32, i32)], b: &[(i32, i32)], op: Op) -> Vec<(i32, i32)> {
    let mut xs: Vec<i32> = a.iter().chain(b).flat_map(|&(l, r)| [l, r]).collect();
    xs.sort_unstable();
    xs.dedup();
    let inside = |v: &[(i32, i32)], x: i32| v.iter().any(|&(l, r)| l <= x && x < r);
    let mut out: Vec<(i32, i32)> = Vec::new();
    for w in xs.windows(2) {
        let (x0, x1) = (w[0], w[1]);
        let (ia, ib) = (inside(a, x0), inside(b, x0));
        let keep = match op {
            Op::Union => ia || ib,
            Op::Intersect => ia && ib,
            Op::Subtract => ia && !ib,
            Op::Xor => ia != ib,
        };
        if keep {
            match out.last_mut() {
                Some(last) if last.1 == x0 => last.1 = x1,
                _ => out.push((x0, x1)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, rr: i32, b: i32) -> Rect {
        Rect::new(l, t, rr, b)
    }

    #[test]
    fn regions_are_banded_as_the_oracle_prints_them() {
        // InvalidateRect.yaml: 10,20,30,40 then 50,5,60,15.
        let a = Rgn::from_rects(&[r(10, 20, 30, 40), r(50, 5, 60, 15)]);
        assert_eq!(a.rects(), &[r(50, 5, 60, 15), r(10, 20, 30, 40)]);
        assert_eq!((a.kind(), a.bounds()), (3, r(10, 5, 60, 40)));
        // GetUpdateRgn.yaml gurg_kinds.
        let mut g = Rgn::rect(r(10, 10, 30, 20));
        g = g.union(&Rgn::rect(r(20, 15, 40, 25)));
        assert_eq!(g.rects(), &[r(10, 10, 30, 15), r(10, 15, 40, 20), r(20, 20, 40, 25)]);
        g = g.union(&Rgn::rect(r(40, 10, 50, 25)));
        assert_eq!(g.rects(), &[r(10, 10, 30, 15), r(40, 10, 50, 15), r(10, 15, 50, 20), r(20, 20, 50, 25)]);
        g = g.union(&Rgn::rect(r(10, 20, 30, 40)));
        assert_eq!(g.rects(), &[r(10, 10, 30, 15), r(40, 10, 50, 15), r(10, 15, 50, 25), r(10, 25, 30, 40)]);
        let s = Rgn::from_rects(&[r(0, 0, 50, 40), r(0, 40, 50, 80)]);
        assert_eq!(s.rects(), &[r(0, 0, 50, 80)]);
        // ValidateRect.yaml: a hole.
        let mut v = Rgn::rect(r(50, 40, 100, 80));
        v = v.subtract(&Rgn::rect(r(70, 50, 80, 60)));
        assert_eq!(v.rects(), &[r(50, 40, 100, 50), r(50, 50, 70, 60), r(80, 50, 100, 60), r(50, 60, 100, 80)]);
        assert!(v.contains(50, 40) && !v.contains(75, 55) && !v.contains(100, 40));
        assert!(Rgn::rect(r(5, 5, 5, 9)).is_empty());
        assert!(v.covers(r(50, 40, 60, 50)) && !v.covers(r(60, 45, 75, 55)));
    }
}
