//! What a page flip has to copy to the device: the damage of a
//! framebuffer, from what a client says (`DRM_IOCTL_MODE_DIRTYFB`).
//!
//! A flip used to copy the whole framebuffer to the device, however little
//! had been drawn. Now a client that tells the device which parts of a
//! buffer it changed (the clips of a `DIRTYFB` on a buffer that is not on
//! the screen) gets only those parts copied when it flips to that buffer.
//! The contract is the one `DIRTYFB` has always had: the clips cover
//! everything changed in the buffer since it was last shown. A client that
//! tells nothing, and a framebuffer never shown before, get all of the
//! framebuffer copied.

use crate::Rect;
use alloc::vec::Vec;

/// The most rectangles kept in one list before they become their bounding
/// box.
const MAX_RECTS: usize = 16;

/// A list of rectangles in framebuffer pixels, kept short: a rectangle
/// inside another is dropped, ones it covers are replaced, and past
/// [`MAX_RECTS`] the list is its bounding box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rects(Vec<Rect>);

fn end(a: u32, b: u32) -> u64 {
    u64::from(a) + u64::from(b)
}

/// Whether `outer` covers all of `inner` (an empty `inner` is covered).
fn covers(outer: &Rect, inner: &Rect) -> bool {
    inner.is_empty()
        || (outer.x <= inner.x
            && outer.y <= inner.y
            && end(outer.x, outer.width) >= end(inner.x, inner.width)
            && end(outer.y, outer.height) >= end(inner.y, inner.height))
}

/// The smallest rectangle covering both.
fn bounding(a: &Rect, b: &Rect) -> Rect {
    if a.is_empty() {
        return *b;
    }
    if b.is_empty() {
        return *a;
    }
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let right = end(a.x, a.width).max(end(b.x, b.width));
    let bottom = end(a.y, a.height).max(end(b.y, b.height));
    Rect::new(x, y, (right - u64::from(x)) as u32, (bottom - u64::from(y)) as u32)
}

/// The part both cover (empty if none).
fn overlap(a: &Rect, b: &Rect) -> Rect {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let right = end(a.x, a.width).min(end(b.x, b.width));
    let bottom = end(a.y, a.height).min(end(b.y, b.height));
    if right <= u64::from(x) || bottom <= u64::from(y) {
        return Rect::default();
    }
    Rect::new(x, y, (right - u64::from(x)) as u32, (bottom - u64::from(y)) as u32)
}

impl Rects {
    /// Just the rectangle `r` (nothing if it is empty).
    pub fn of(r: Rect) -> Rects {
        let mut l = Rects::default();
        l.add(r);
        l
    }

    pub fn as_slice(&self) -> &[Rect] {
        &self.0
    }

    /// Add `r`.
    pub fn add(&mut self, r: Rect) {
        if r.is_empty() || self.0.iter().any(|d| covers(d, &r)) {
            return;
        }
        self.0.retain(|d| !covers(&r, d));
        self.0.push(r);
        if self.0.len() > MAX_RECTS {
            let all = self.0.iter().fold(Rect::default(), |a, d| bounding(&a, d));
            self.0 = Vec::from([all]);
        }
    }

    /// Add every rectangle of `other`.
    pub fn extend(&mut self, other: &Rects) {
        for r in &other.0 {
            self.add(*r);
        }
    }

    /// The parts inside `area`, not empty.
    pub fn within(&self, area: &Rect) -> Rects {
        let mut out = Rects::default();
        for r in &self.0 {
            out.add(overlap(r, area));
        }
        out
    }
}

/// What the device has to be told about one framebuffer.
#[derive(Clone, Debug)]
pub(crate) struct FbDamage {
    /// What the client said it changed since the buffer was last shown
    /// (`DIRTYFB` while it was not on the screen); `None`: it said nothing,
    /// so any of it may have changed.
    told: Option<Rects>,
    /// Never copied to the device yet: all of it is wanted.
    fresh: bool,
}

impl Default for FbDamage {
    fn default() -> FbDamage {
        FbDamage { told: None, fresh: true }
    }
}

impl FbDamage {
    /// The client reports `clips` changed.
    pub fn tell(&mut self, clips: &Rects) {
        self.told.get_or_insert_with(Rects::default).extend(clips);
    }

    /// What to copy to show this framebuffer's `area`; this consumes what
    /// was told. Everything if the client told nothing, or this is the
    /// first copy.
    pub fn take_for(&mut self, area: &Rect) -> Rects {
        let told = self.told.take();
        match told {
            Some(told) if !core::mem::take(&mut self.fresh) => told.within(area),
            _ => {
                self.fresh = false;
                Rects::of(*area)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_inside_others_are_dropped_and_covered_ones_replaced() {
        let mut l = Rects::default();
        l.add(Rect::new(0, 0, 10, 10));
        l.add(Rect::new(2, 2, 3, 3));
        assert_eq!(l.as_slice(), [Rect::new(0, 0, 10, 10)]);
        l.add(Rect::new(20, 0, 5, 5));
        l.add(Rect::new(0, 0, 30, 30));
        assert_eq!(l.as_slice(), [Rect::new(0, 0, 30, 30)]);
        l.add(Rect::new(5, 5, 0, 4));
        assert_eq!(l.as_slice().len(), 1, "an empty rectangle is nothing");
    }

    #[test]
    fn a_long_list_becomes_its_bounding_box() {
        let mut l = Rects::default();
        for i in 0..=MAX_RECTS as u32 {
            l.add(Rect::new(i * 10, i * 3, 2, 2));
        }
        assert_eq!(l.as_slice(), [Rect::new(0, 0, MAX_RECTS as u32 * 10 + 2, MAX_RECTS as u32 * 3 + 2)]);
    }

    #[test]
    fn within_cuts_to_the_area() {
        let mut l = Rects::of(Rect::new(0, 0, 100, 100));
        l.add(Rect::new(300, 0, 5, 5));
        let w = l.within(&Rect::new(50, 50, 100, 100));
        assert_eq!(w.as_slice(), [Rect::new(50, 50, 50, 50)]);
    }

    #[test]
    fn a_new_framebuffer_and_a_silent_client_copy_everything() {
        let screen = Rect::new(0, 0, 64, 48);
        let mut d = FbDamage::default();
        // Told something, a new framebuffer still has to be copied whole.
        d.tell(&Rects::of(Rect::new(1, 1, 2, 2)));
        assert_eq!(d.take_for(&screen).as_slice(), [screen]);
        // Then what was told is what is copied (cut to the area).
        d.tell(&Rects::of(Rect::new(1, 1, 2, 2)));
        d.tell(&Rects::of(Rect::new(60, 40, 20, 20)));
        let got = d.take_for(&screen);
        assert_eq!(got.as_slice(), [Rect::new(1, 1, 2, 2), Rect::new(60, 40, 4, 8)]);
        // Taking consumed it: the next flip without word copies all.
        assert_eq!(d.take_for(&screen).as_slice(), [screen]);
        // Told that nothing changed: nothing is copied.
        d.tell(&Rects::default());
        assert!(d.take_for(&screen).as_slice().is_empty());
    }
}
