//! Regions: sets of pixels made of rectangles, the contents of an `HRGN`
//! (`specs/windows/gdi32.dll/CombineRgn.yaml`, `CreateRectRgn.yaml`,
//! `GetRgnBox.yaml`, `PtInRegion.yaml`). Pure algebra with no handles, so
//! the window manager can use it for update and visible regions too.
//!
//! A [`Region`] is kept in one canonical form: non-overlapping rectangles
//! in y-x bands. Each band is a horizontal strip (its rectangles share top
//! and bottom); bands are sorted top to bottom, rectangles in a band left to
//! right; touching pieces in a band are merged, and vertically adjacent
//! bands with the same x spans are merged into one (the minimal band
//! decomposition). So two regions covering the same pixels compare equal,
//! and [`Region::rects`] lists the rectangles in the order `GetRegionData`
//! does (`CombineRgn.yaml`, `result_shape`).

use alloc::vec::Vec;

/// `CombineRgn` modes (`wingdi.h`).
pub const RGN_AND: i32 = 1;
pub const RGN_OR: i32 = 2;
pub const RGN_XOR: i32 = 3;
pub const RGN_DIFF: i32 = 4;
pub const RGN_COPY: i32 = 5;

/// Region kinds, as `CombineRgn`, `GetRgnBox` and the clipping calls
/// return them (`wingdi.h`).
pub const ERROR: i32 = 0;
pub const NULLREGION: i32 = 1;
pub const SIMPLEREGION: i32 = 2;
pub const COMPLEXREGION: i32 = 3;

/// A rectangle (`RECT`, `windef.h`): the pixels with `left <= x < right`
/// and `top <= y < bottom`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    /// No pixels: zero or negative width or height.
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// The overlap of two rectangles (empty, possibly inverted, when they
    /// do not overlap: check [`Rect::is_empty`]).
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::new(self.left.max(o.left), self.top.max(o.top), self.right.min(o.right), self.bottom.min(o.bottom))
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    /// The same rectangle with swapped corners put in order.
    pub fn normalized(&self) -> Rect {
        Rect::new(
            self.left.min(self.right),
            self.top.min(self.bottom),
            self.left.max(self.right),
            self.top.max(self.bottom),
        )
    }

    /// Moved by (`dx`, `dy`), wrapping on overflow.
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.left.wrapping_add(dx),
            self.top.wrapping_add(dy),
            self.right.wrapping_add(dx),
            self.bottom.wrapping_add(dy),
        )
    }
}

/// A set of pixels as non-overlapping rectangles in canonical band form
/// (see the module documentation). The default is the empty region.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Region {
    rects: Vec<Rect>,
}

/// One band while combining: top, bottom and the x spans.
type Band = (i32, i32, Vec<(i32, i32)>);

impl Region {
    /// The empty region.
    pub const fn new() -> Region {
        Region { rects: Vec::new() }
    }

    /// The pixels of one rectangle; swapped corners are put in order, an
    /// empty rectangle gives the empty region (`CreateRectRgn.yaml`).
    pub fn from_rect(r: Rect) -> Region {
        let r = r.normalized();
        if r.is_empty() { Region::new() } else { Region { rects: alloc::vec![r] } }
    }

    /// The union of rectangles (each normalised as in [`Region::from_rect`]).
    pub fn from_rects(rects: &[Rect]) -> Region {
        rects.iter().fold(Region::new(), |acc, r| acc.union(&Region::from_rect(*r)))
    }

    /// `CreateRoundRectRgn` (Learn): the rectangle `l, t, r, b` with its
    /// corners rounded by an ellipse `ew` wide and `eh` high. Like GDI's
    /// regions of ellipses it is one pixel narrower and shorter than the
    /// rectangle given (0,0,80,50 with a 20-pixel corner has the box
    /// 0,0,79,49; `GetWindowRgnBox.yaml` `boxes`). A row's inset is where
    /// the corner ellipse starts covering the pixel centres of that row.
    pub fn round_rect(l: i32, t: i32, r: i32, b: i32, ew: i32, eh: i32) -> Region {
        let n = Rect::new(l, t, r, b).normalized();
        let (w, h) = (i64::from(n.right) - i64::from(n.left) - 1, i64::from(n.bottom) - i64::from(n.top) - 1);
        if w <= 0 || h <= 0 {
            return Region::new();
        }
        // The corner ellipse is at most as large as the region.
        let (cw, ch) = (i64::from(ew).abs().min(w), i64::from(eh).abs().min(h));
        let inset = |k: i64| -> i64 {
            if cw == 0 || ch == 0 || 2 * k + 1 >= ch {
                return 0;
            }
            // Pixel centre (i + 1/2, k + 1/2) inside the ellipse of
            // diameters cw, ch that touches the corner: in doubled units
            // (2i+1-cw)^2 * ch^2 + (2k+1-ch)^2 * cw^2 <= cw^2 * ch^2.
            let dy = 2 * k + 1 - ch;
            let limit = cw * cw * ch * ch - dy * dy * cw * cw;
            let mut i = 0;
            while 2 * i + 1 < cw && (2 * i + 1 - cw) * (2 * i + 1 - cw) * ch * ch > limit {
                i += 1;
            }
            i
        };
        let mut rects: Vec<Rect> = Vec::new();
        for y in 0..h {
            let i = inset(y.min(h - 1 - y));
            let (x0, x1) = (i64::from(n.left) + i, i64::from(n.left) + w - i);
            let top = (i64::from(n.top) + y) as i32;
            if x1 <= x0 {
                continue;
            }
            // Rows with the same span make one band.
            match rects.last_mut() {
                Some(last) if last.bottom == top && i64::from(last.left) == x0 && i64::from(last.right) == x1 => {
                    last.bottom = top + 1
                }
                _ => rects.push(Rect::new(x0 as i32, top, x1 as i32, top + 1)),
            }
        }
        Region { rects }
    }

    /// The rectangles in band order (`GetRegionData`'s order).
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// [`NULLREGION`], [`SIMPLEREGION`] (one rectangle) or
    /// [`COMPLEXREGION`].
    pub fn kind(&self) -> i32 {
        match self.rects.len() {
            0 => NULLREGION,
            1 => SIMPLEREGION,
            _ => COMPLEXREGION,
        }
    }

    /// The smallest rectangle around the region; 0,0,0,0 when it is empty
    /// (`GetRgnBox.yaml`).
    pub fn bounds(&self) -> Rect {
        let (Some(first), Some(last)) = (self.rects.first(), self.rects.last()) else { return Rect::default() };
        let left = self.rects.iter().map(|r| r.left).min().unwrap_or(0);
        let right = self.rects.iter().map(|r| r.right).max().unwrap_or(0);
        Rect::new(left, first.top, right, last.bottom)
    }

    /// Whether pixel (`x`, `y`) is in the region (`PtInRegion`).
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        self.rects.iter().any(|r| r.contains_point(x, y))
    }

    /// Whether any part of `r` (normalised) lies in the region
    /// (`RectInRegion`): an edge shared from outside does not count; an
    /// empty or zero-height rectangle strictly inside does (observed,
    /// `PtInRegion.yaml`).
    pub fn intersects_rect(&self, r: Rect) -> bool {
        let r = r.normalized();
        self.rects.iter().any(|q| q.left < r.right && q.right > r.left && q.top < r.bottom && q.bottom > r.top)
    }

    pub fn union(&self, o: &Region) -> Region {
        self.op(o, |a, b| a || b)
    }

    pub fn intersect(&self, o: &Region) -> Region {
        self.op(o, |a, b| a && b)
    }

    /// The pixels of `self` that are not in `o` (`RGN_DIFF`).
    pub fn subtract(&self, o: &Region) -> Region {
        self.op(o, |a, b| a && !b)
    }

    pub fn xor(&self, o: &Region) -> Region {
        self.op(o, |a, b| a != b)
    }

    /// `self` combined with `o` by a `CombineRgn` mode ([`RGN_AND`] ...
    /// [`RGN_COPY`], which gives `self`); `None` for any other mode.
    pub fn combine(&self, o: &Region, mode: i32) -> Option<Region> {
        Some(match mode {
            RGN_AND => self.intersect(o),
            RGN_OR => self.union(o),
            RGN_XOR => self.xor(o),
            RGN_DIFF => self.subtract(o),
            RGN_COPY => self.clone(),
            _ => return None,
        })
    }

    /// The region within rectangle `r`.
    pub fn intersect_rect(&self, r: Rect) -> Region {
        self.intersect(&Region::from_rect(r))
    }

    /// Move the region by (`dx`, `dy`); false, leaving it as it was, when
    /// a coordinate would overflow.
    pub fn offset(&mut self, dx: i32, dy: i32) -> bool {
        let fits = self.rects.iter().all(|r| {
            r.left.checked_add(dx).is_some()
                && r.right.checked_add(dx).is_some()
                && r.top.checked_add(dy).is_some()
                && r.bottom.checked_add(dy).is_some()
        });
        if fits {
            for r in &mut self.rects {
                *r = r.offset(dx, dy);
            }
        }
        fits
    }

    /// A copy moved by (`dx`, `dy`), wrapping on overflow.
    pub fn offset_by(&self, dx: i32, dy: i32) -> Region {
        Region { rects: self.rects.iter().map(|r| r.offset(dx, dy)).collect() }
    }

    /// The x spans of the band covering rows `y0..y1` (canonical regions:
    /// a band either covers the whole strip or misses it).
    fn spans(&self, y0: i32, y1: i32) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.rects.iter().filter(move |r| r.top <= y0 && r.bottom >= y1).map(|r| (r.left, r.right))
    }

    /// The pixels where `keep(in self, in o)` holds, in canonical form: a
    /// sweep over the strips between all band edges, then over the x edges
    /// within each strip; equal neighbouring strips are merged.
    fn op(&self, o: &Region, keep: impl Fn(bool, bool) -> bool) -> Region {
        let mut ys: Vec<i32> = self.rects.iter().chain(&o.rects).flat_map(|r| [r.top, r.bottom]).collect();
        ys.sort_unstable();
        ys.dedup();
        let mut bands: Vec<Band> = Vec::new();
        for w in ys.windows(2) {
            let (y0, y1) = (w[0], w[1]);
            let sa: Vec<(i32, i32)> = self.spans(y0, y1).collect();
            let sb: Vec<(i32, i32)> = o.spans(y0, y1).collect();
            let mut xs: Vec<i32> = sa.iter().chain(&sb).flat_map(|s| [s.0, s.1]).collect();
            xs.sort_unstable();
            xs.dedup();
            let covers = |s: &[(i32, i32)], x: i32| s.iter().any(|&(l, r)| l <= x && x < r);
            let mut spans: Vec<(i32, i32)> = Vec::new();
            for v in xs.windows(2) {
                let (x0, x1) = (v[0], v[1]);
                if !keep(covers(&sa, x0), covers(&sb, x0)) {
                    continue;
                }
                match spans.last_mut() {
                    Some(last) if last.1 == x0 => last.1 = x1,
                    _ => spans.push((x0, x1)),
                }
            }
            if spans.is_empty() {
                continue;
            }
            match bands.last_mut() {
                Some(last) if last.1 == y0 && last.2 == spans => last.1 = y1,
                _ => bands.push((y0, y1, spans)),
            }
        }
        let rects = bands
            .into_iter()
            .flat_map(|(t, b, spans)| spans.into_iter().map(move |(l, r)| Rect::new(l, t, r, b)))
            .collect();
        Region { rects }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// GetWindowRgnBox.yaml `boxes`: a round rectangle 0,0,80,50 with a
    /// 20-pixel corner is complex and has the box 0,0,79,49.
    #[test]
    fn round_rectangles_are_one_pixel_smaller_with_cut_corners() {
        let g = Region::round_rect(0, 0, 80, 50, 20, 20);
        assert_eq!(g.kind(), COMPLEXREGION);
        assert_eq!(g.bounds(), Rect::new(0, 0, 79, 49));
        // The corners are cut, the middle of every side is in.
        for (x, y) in [(0, 0), (78, 0), (0, 48), (78, 48), (79, 24), (40, 49)] {
            assert!(!g.contains_point(x, y), "{x},{y}");
        }
        for (x, y) in [(40, 0), (0, 24), (78, 24), (40, 48), (10, 10), (68, 38)] {
            assert!(g.contains_point(x, y), "{x},{y}");
        }
        // Symmetric, and in canonical band form (equal to the union of its rows).
        for y in 0..49 {
            for x in 0..79 {
                assert_eq!(g.contains_point(x, y), g.contains_point(78 - x, y));
                assert_eq!(g.contains_point(x, y), g.contains_point(x, 48 - y));
            }
        }
        assert_eq!(Region::from_rects(g.rects()), g);
        // No corner ellipse: the plain (smaller) rectangle; nothing for an empty one.
        assert_eq!(Region::round_rect(5, 5, 20, 15, 0, 0), Region::from_rect(Rect::new(5, 5, 19, 14)));
        assert!(Region::round_rect(5, 5, 6, 30, 4, 4).is_empty());
        // Swapped corners are put in order; a huge corner ellipse is the
        // region's own ellipse.
        assert_eq!(Region::round_rect(80, 50, 0, 0, 20, 20), g);
        let e = Region::round_rect(0, 0, 60, 40, 500, 500);
        assert_eq!(e.bounds(), Rect::new(0, 0, 59, 39));
        assert!(e.contains_point(30, 20) && !e.contains_point(0, 0));
    }

    fn r(l: i32, t: i32, rr: i32, b: i32) -> Rect {
        Rect::new(l, t, rr, b)
    }

    fn rg(l: i32, t: i32, rr: i32, b: i32) -> Region {
        Region::from_rect(r(l, t, rr, b))
    }

    #[test]
    fn rect_regions_are_normalised_and_empty_ones_are_empty() {
        assert_eq!(rg(110, 20, 10, 70).rects(), &[r(10, 20, 110, 70)]);
        assert_eq!(rg(110, 70, 10, 20), rg(10, 20, 110, 70));
        assert!(rg(10, 20, 10, 70).is_empty());
        assert_eq!(rg(5, 5, 5, 5), rg(7, 7, 7, 9));
        assert_eq!(rg(5, 5, 5, 5).bounds(), Rect::default());
        assert_eq!(rg(5, 5, 5, 5).kind(), NULLREGION);
        assert_eq!(rg(i32::MIN, i32::MIN, i32::MAX, i32::MAX).kind(), SIMPLEREGION);
    }

    #[test]
    fn overlapping_rectangles_combine_into_bands() {
        let (a, b) = (rg(10, 10, 50, 40), rg(30, 20, 70, 60));
        assert_eq!(a.intersect(&b).rects(), &[r(30, 20, 50, 40)]);
        assert_eq!(a.union(&b).rects(), &[r(10, 10, 50, 20), r(10, 20, 70, 40), r(30, 40, 70, 60)]);
        assert_eq!(a.xor(&b).rects(), &[r(10, 10, 50, 20), r(10, 20, 30, 40), r(50, 20, 70, 40), r(30, 40, 70, 60)]);
        assert_eq!(a.subtract(&b).rects(), &[r(10, 10, 50, 20), r(10, 20, 30, 40)]);
        assert_eq!(b.subtract(&a).rects(), &[r(50, 20, 70, 40), r(30, 40, 70, 60)]);
        assert_eq!(a.combine(&b, RGN_COPY), Some(a.clone()));
        assert_eq!(a.combine(&b, 0), None);
        assert_eq!(a.combine(&b, 6), None);
        assert_eq!(a.union(&b).kind(), COMPLEXREGION);
        assert_eq!(a.union(&b).bounds(), r(10, 10, 70, 60));
    }

    #[test]
    fn touching_pieces_merge() {
        assert_eq!(rg(0, 0, 10, 10).union(&rg(10, 0, 20, 10)).rects(), &[r(0, 0, 20, 10)]);
        assert_eq!(rg(0, 0, 10, 10).xor(&rg(10, 0, 20, 10)).rects(), &[r(0, 0, 20, 10)]);
        assert_eq!(rg(0, 0, 10, 10).union(&rg(0, 10, 10, 20)).rects(), &[r(0, 0, 10, 20)]);
        let tiles = Region::from_rects(&[r(0, 0, 10, 10), r(10, 0, 20, 10), r(0, 10, 10, 20), r(10, 10, 20, 20)]);
        assert_eq!(tiles.rects(), &[r(0, 0, 20, 20)]);
        assert_eq!(
            rg(0, 0, 10, 10).union(&rg(10, 5, 20, 15)).rects(),
            &[r(0, 0, 10, 5), r(0, 5, 20, 10), r(10, 10, 20, 15)]
        );
        assert_eq!(rg(0, 0, 10, 10).union(&rg(20, 0, 30, 10)).rects(), &[r(0, 0, 10, 10), r(20, 0, 30, 10)]);
    }

    #[test]
    fn nested_identical_and_empty() {
        let (o, i) = (rg(0, 0, 30, 30), rg(10, 10, 20, 20));
        let frame = [r(0, 0, 30, 10), r(0, 10, 10, 20), r(20, 10, 30, 20), r(0, 20, 30, 30)];
        assert_eq!(o.union(&i), o);
        assert_eq!(o.intersect(&i), i);
        assert_eq!(o.subtract(&i).rects(), &frame);
        assert_eq!(o.xor(&i).rects(), &frame);
        assert!(o.subtract(&o).is_empty());
        assert!(o.xor(&o).is_empty());
        let e = Region::new();
        assert_eq!(e.union(&i), i);
        assert_eq!(i.xor(&e), i);
        assert!(e.subtract(&i).is_empty());
        assert!(e.intersect(&i).is_empty());
        // Rectangles touching at a corner have nothing in common.
        assert!(rg(0, 0, 10, 10).intersect(&rg(10, 10, 20, 20)).is_empty());
    }

    #[test]
    fn many_rectangle_unions() {
        let stairs = Region::from_rects(&[r(0, 0, 10, 10), r(10, 10, 20, 20), r(20, 20, 30, 30)]);
        assert_eq!(stairs.rects().len(), 3);
        let frame = Region::from_rects(&[r(0, 0, 30, 5), r(0, 25, 30, 30), r(0, 0, 5, 30), r(25, 0, 30, 30)]);
        assert_eq!(frame.rects(), &[r(0, 0, 30, 5), r(0, 5, 5, 25), r(25, 5, 30, 25), r(0, 25, 30, 30)]);
        let cross = Region::from_rects(&[r(10, 0, 20, 30), r(0, 10, 30, 20)]);
        assert_eq!(cross.rects(), &[r(10, 0, 20, 10), r(0, 10, 30, 20), r(10, 20, 20, 30)]);
        let comb = Region::from_rects(&[r(0, 0, 2, 10), r(4, 0, 6, 10), r(8, 0, 10, 10), r(0, 10, 10, 12)]);
        assert_eq!(comb.rects(), &[r(0, 0, 2, 10), r(4, 0, 6, 10), r(8, 0, 10, 10), r(0, 10, 10, 12)]);
        assert_eq!(comb.bounds(), r(0, 0, 10, 12));
        // Equal pixels, equal regions, whatever the order of the pieces.
        assert_eq!(Region::from_rects(&[r(0, 10, 30, 20), r(10, 0, 20, 30)]), cross);
        assert_ne!(stairs, rg(0, 0, 30, 30));
    }

    #[test]
    fn points_and_rectangles() {
        let x = rg(10, 10, 20, 20);
        for (px, py) in [(10, 10), (19, 19), (19, 10), (10, 19), (15, 15)] {
            assert!(x.contains_point(px, py));
        }
        for (px, py) in [(20, 20), (20, 10), (10, 20), (9, 15), (15, 9), (19, 20), (20, 19)] {
            assert!(!x.contains_point(px, py));
        }
        assert!(!x.contains_point(i32::MIN, i32::MIN) && !x.contains_point(i32::MAX, i32::MAX));
        assert!(!Region::new().contains_point(0, 0));
        let l = rg(0, 0, 10, 30).union(&rg(0, 20, 30, 30));
        assert!(l.contains_point(9, 19) && l.contains_point(29, 29) && !l.contains_point(10, 19));
        assert!(x.intersects_rect(r(15, 15, 25, 25)));
        assert!(!x.intersects_rect(r(20, 10, 30, 20)));
        assert!(!x.intersects_rect(r(0, 10, 10, 20)));
        assert!(x.intersects_rect(r(25, 15, 15, 18)));
        assert!(x.intersects_rect(r(15, 15, 15, 15)));
        assert!(x.intersects_rect(r(12, 15, 18, 15)));
        assert!(!l.intersects_rect(r(15, 5, 25, 15)));
        assert!(l.intersects_rect(r(0, 10, 10, 20)));
    }

    #[test]
    fn offset_moves_or_refuses() {
        let mut a = rg(0, 0, 10, 10).union(&rg(20, 0, 30, 10));
        assert!(a.offset(5, -5));
        assert_eq!(a.rects(), &[r(5, -5, 15, 5), r(25, -5, 35, 5)]);
        let before = a.clone();
        assert!(!a.offset(i32::MAX, 0));
        assert_eq!(a, before);
        let mut e = Region::new();
        assert!(e.offset(3, 3));
        assert_eq!(e.bounds(), Rect::default());
        assert_eq!(rg(1, 1, 2, 2).offset_by(1, 2).rects(), &[r(2, 3, 3, 4)]);
        assert_eq!(rg(0, 0, 20, 20).intersect_rect(r(10, 10, 30, 30)).rects(), vec![r(10, 10, 20, 20)].as_slice());
    }
}
