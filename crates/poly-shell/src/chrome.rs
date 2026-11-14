//! Surfaces that hang from the ridge, as Alpine Shell draws them
//! (`chrome/chromeShape.ts`, `ChromeCanvas.tsx`, `shared/popupPlacement.ts`;
//! ADR 0127): one at a time, in the ridge's colour, joined to it by
//! concave shoulders, its bottom corners rounded. A surface opens by
//! growing down from the ridge (460 ms, easeOutExpo) with its contents
//! sliding down inside it, closes by shrinking back (300 ms, easeInCubic),
//! and turns into another by moving and resizing (400 ms, easeOutExpo).
//!
//! [`Motion`] keeps that state on a millisecond clock; [`Geometry`] is
//! where a surface hangs and [`Geometry::place`] Alpine's placement next
//! to the item that opened it.

use crate::theme::chrome as ct;
use poly_wayland::compose::paint::{Box2, Chrome, floor};
use poly_wayland::compose::{Rect, Scale};

/// Which side a surface is put against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Free: shoulders on both sides.
    Centre,
    /// Against the screen's left edge: no left shoulder.
    Left,
    /// Against the right edge: no right shoulder.
    Right,
}

/// Where a surface hangs, logical: its body's horizontal centre, width and
/// full height (below the ridge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub centre: f32,
    pub width: f32,
    pub height: f32,
    pub anchor: Anchor,
}

impl Geometry {
    /// Centred on the screen.
    pub fn centred(screen_w: i32, width: i32, height: i32) -> Geometry {
        let left = (screen_w - width) / 2;
        Geometry {
            centre: left as f32 + width as f32 / 2.0,
            width: width as f32,
            height: height as f32,
            anchor: Anchor::Centre,
        }
    }

    /// Against the right edge.
    pub fn at_right(screen_w: i32, width: i32, height: i32) -> Geometry {
        Geometry {
            centre: screen_w as f32 - width as f32 / 2.0,
            width: width as f32,
            height: height as f32,
            anchor: Anchor::Right,
        }
    }

    /// Alpine's placement under the item centred at `trigger`: centred on
    /// it, unless a shoulder would come within 6 pixels of a screen edge;
    /// then against that edge.
    pub fn place(trigger: i32, width: i32, height: i32, screen_w: i32) -> Geometry {
        let half = width as f32 / 2.0;
        let t = trigger as f32;
        let (left, anchor) = if t - half - (ct::SHOULDER as f32) < ct::SCREEN_PAD as f32 {
            (0.0, Anchor::Left)
        } else if t + half + ct::SHOULDER as f32 > (screen_w - ct::SCREEN_PAD) as f32 {
            ((screen_w - width) as f32, Anchor::Right)
        } else {
            (floor(t - half + 0.5), Anchor::Centre)
        };
        Geometry { centre: left + half, width: width as f32, height: height as f32, anchor }
    }

    /// The body at full height below the ridge at `top`, logical.
    pub fn body(&self, top: i32) -> Rect {
        let left = floor(self.centre - self.width / 2.0 + 0.5) as i32;
        Rect::new(left, top, self.width as i32, self.height as i32)
    }

    /// The shape at output scale `s` with its body `height` logical pixels
    /// high (opening and closing).
    pub fn chrome(&self, top: i32, height: f32, s: Scale) -> Chrome {
        let f = s.factor();
        let x0 = (self.centre - self.width / 2.0) * f;
        Chrome {
            body: Box2 { x0, y0: (top * s) as f32, x1: x0 + self.width * f, y1: (top * s) as f32 + height * f },
            radius: (ct::RADIUS * s) as f32,
            shoulder: (ct::SHOULDER * s) as f32,
            left_shoulder: self.anchor != Anchor::Left,
            right_shoulder: self.anchor != Anchor::Right,
        }
    }

    /// Everything it covers at full height, shoulders included, logical.
    pub fn bounds(&self, top: i32) -> Rect {
        self.chrome(top, self.height, Scale::ONE).bounds()
    }

    fn lerp(a: Geometry, b: Geometry, t: f32) -> Geometry {
        let m = |x: f32, y: f32| x + (y - x) * t;
        Geometry {
            centre: m(a.centre, b.centre),
            width: m(a.width, b.width),
            height: m(a.height, b.height),
            anchor: b.anchor,
        }
    }
}

/// 2^x for x ≤ 0 (no `std` here): the integer part through the
/// exponent bits, the fraction by a polynomial.
fn exp2(x: f32) -> f32 {
    if x < -126.0 {
        return 0.0;
    }
    let i = floor(x);
    let f = x - i;
    let p = 1.0
        + f * (core::f32::consts::LN_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_3))));
    p * f32::from_bits(((i as i32 + 127) as u32) << 23)
}

/// Alpine's two curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ease {
    OutExpo,
    InCubic,
}

fn ease(e: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match e {
        Ease::OutExpo if t >= 1.0 => 1.0,
        Ease::OutExpo => 1.0 - exp2(-10.0 * t),
        Ease::InCubic => t * t * t,
    }
}

/// The surface on the screen and how it moves. `K` names the surfaces.
#[derive(Clone, Debug)]
pub struct Motion<K> {
    /// The surface open or opening.
    shown: Option<K>,
    /// The one closing (drawn until it is gone).
    closing: Option<K>,
    geom: Geometry,
    /// Turning from this geometry since then.
    morph: Option<(Geometry, u64)>,
    open: (f32, f32, u64, u64, Ease),
    animate: bool,
}

impl<K: Copy + PartialEq> Default for Motion<K> {
    fn default() -> Motion<K> {
        Motion {
            shown: None,
            closing: None,
            geom: Geometry { centre: 0.0, width: 0.0, height: 0.0, anchor: Anchor::Centre },
            morph: None,
            open: (0.0, 0.0, 0, 0, Ease::OutExpo),
            animate: true,
        }
    }
}

impl<K: Copy + PartialEq> Motion<K> {
    /// Move at once instead of animating (or animate again).
    pub fn set_animate(&mut self, on: bool) {
        self.animate = on;
    }

    /// How open the surface is at `now`, 0 to 1.
    pub fn openness(&self, now: u64) -> f32 {
        let (from, to, start, dur, e) = self.open;
        if !self.animate || dur == 0 || now >= start + dur {
            return to;
        }
        let t = now.saturating_sub(start) as f32 / dur as f32;
        from + (to - from) * ease(e, t)
    }

    /// The geometry at `now` (between two while turning).
    pub fn geometry(&self, now: u64) -> Geometry {
        match self.morph {
            Some((from, start)) if self.animate && now < start + ct::MORPH_MS => {
                let t = now.saturating_sub(start) as f32 / ct::MORPH_MS as f32;
                Geometry::lerp(from, self.geom, ease(Ease::OutExpo, t))
            }
            _ => self.geom,
        }
    }

    fn start_open(&mut self, to: f32, dur: u64, e: Ease, now: u64) {
        let from = self.openness(now);
        self.open = (from, to, now, dur, e);
    }

    /// The surface that should show now and where (`None`: none). Returns
    /// whether anything changed.
    pub fn want(&mut self, want: Option<(K, Geometry)>, now: u64) -> bool {
        match (self.shown, want) {
            (None, None) => {
                if self.closing.is_some() && self.openness(now) <= 0.0 {
                    self.closing = None;
                    return true;
                }
                false
            }
            (None, Some((k, g))) => {
                let reopen = self.closing == Some(k);
                self.shown = Some(k);
                self.closing = None;
                if !reopen {
                    self.morph = None;
                    if self.openness(now) > 0.0 {
                        // Another was still closing: turn it into this one.
                        self.morph = Some((self.geometry(now), now));
                    }
                }
                self.geom = g;
                self.start_open(1.0, ct::OPEN_MS, Ease::OutExpo, now);
                true
            }
            (Some(k), Some((w, g))) if k == w => {
                if g != self.geom {
                    // The same surface resized: no animation.
                    self.geom = g;
                    self.morph = None;
                    return true;
                }
                false
            }
            (Some(_), Some((w, g))) => {
                self.morph = Some((self.geometry(now), now));
                self.shown = Some(w);
                self.geom = g;
                self.start_open(1.0, ct::OPEN_MS, Ease::OutExpo, now);
                true
            }
            (Some(k), None) => {
                self.closing = Some(k);
                self.shown = None;
                self.morph = None;
                self.start_open(0.0, ct::CLOSE_MS, Ease::InCubic, now);
                true
            }
        }
    }

    /// The surface open (not one closing).
    pub fn shown(&self) -> Option<K> {
        self.shown
    }

    /// What to draw at `now`: the surface (open or closing), its geometry,
    /// and how high its body is.
    pub fn drawn(&self, now: u64) -> Option<(K, Geometry, f32)> {
        let k = self.shown.or(self.closing)?;
        let g = self.geometry(now);
        let h = g.height * self.openness(now);
        (h > 0.01).then_some((k, g, h))
    }

    /// Whether anything moves at `now`.
    pub fn animating(&self, now: u64) -> bool {
        if !self.animate {
            return false;
        }
        let (_, _, start, dur, _) = self.open;
        let opening = now < start + dur;
        let morphing = self.morph.is_some_and(|(_, s)| now < s + ct::MORPH_MS);
        opening || morphing || (self.closing.is_some() && self.shown.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_follows_alpine() {
        // Under an item in the middle: centred on it.
        let g = Geometry::place(640, 260, 100, 1280);
        assert_eq!((g.anchor, g.body(24)), (Anchor::Centre, Rect::new(510, 24, 260, 100)));
        // Near the right edge: against it, no right shoulder.
        let g = Geometry::place(1200, 260, 100, 1280);
        assert_eq!((g.anchor, g.body(24).right()), (Anchor::Right, 1280));
        assert_eq!(g.bounds(24), Rect::new(1020 - 14, 24, 274, 100));
        // Near the left: against it.
        let g = Geometry::place(15, 150, 64, 1280);
        assert_eq!((g.anchor, g.body(24).x), (Anchor::Left, 0));
    }

    #[test]
    fn curves() {
        assert!((ease(Ease::OutExpo, 0.0)).abs() < 1e-6);
        assert_eq!(ease(Ease::OutExpo, 1.0), 1.0);
        assert!((ease(Ease::OutExpo, 0.1) - 0.5).abs() < 0.01, "half way after a tenth");
        assert!((ease(Ease::InCubic, 0.5) - 0.125).abs() < 1e-6);
        assert!((exp2(-1.0) - 0.5).abs() < 1e-5);
        assert!((exp2(-3.5) - 0.088_388).abs() < 1e-4);
    }

    #[test]
    fn a_surface_opens_turns_and_closes() {
        let a = Geometry::centred(1280, 720, 48);
        let b = Geometry::place(1200, 260, 190, 1280);
        let mut m: Motion<u8> = Motion::default();
        assert!(m.drawn(0).is_none());
        assert!(m.want(Some((1, a)), 1000));
        // Opening: growing, easing out; done after 460 ms.
        let (_, _, h) = m.drawn(1046).unwrap();
        assert!(h > 20.0 && h < 48.0, "{h}");
        assert!(m.animating(1100));
        assert_eq!(m.drawn(1460).unwrap().2, 48.0);
        assert!(!m.animating(1460));
        assert!(!m.want(Some((1, a)), 1500), "nothing changed");
        // Another surface: it turns from the first's geometry.
        assert!(m.want(Some((2, b)), 2000));
        let g = m.geometry(2050);
        assert!(g.width < 720.0 && g.width > 260.0);
        assert_eq!(m.geometry(2400), b);
        assert_eq!(m.shown(), Some(2));
        // Closing: shrinks for 300 ms, drawn until gone.
        assert!(m.want(None, 3000));
        assert_eq!(m.shown(), None);
        assert_eq!(m.drawn(3100).map(|d| d.0), Some(2));
        assert!(m.drawn(3300).is_none());
        assert!(m.want(None, 3300), "the closed surface is forgotten");
        assert!(!m.animating(3300));
        // Without animations everything is at once.
        let mut m: Motion<u8> = Motion::default();
        m.set_animate(false);
        m.want(Some((1, a)), 0);
        assert_eq!(m.drawn(0).unwrap().2, 48.0);
        m.want(None, 0);
        assert!(m.drawn(0).is_none());
    }
}
