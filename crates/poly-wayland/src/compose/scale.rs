//! The output's scale (ADR 0179): how many physical pixels a logical one
//! is, in 120ths as `wp_fractional_scale_v1` counts them (100 % is 120, 150 %
//! is 180), and the one place logical coordinates become physical ones and
//! back.
//!
//! The rules, which every drawing, damage, hit-testing and input path of the
//! compositor, the shell and the native apps follow:
//!
//! - **Positions and edges** become physical pixels by rounding the exact
//!   product to the nearest pixel, halves up ([`Scale::px`]). A rectangle is
//!   converted by its edges ([`Scale::rect`]): `left = px(x)`,
//!   `right = px(x + width)`, so neighbours that touch in logical pixels
//!   touch in physical ones (no gaps, no overlaps), the way Chromium
//!   snaps layout to device pixels.
//! - **A length that must be the same everywhere** (a border, a radius, a
//!   gap, a font size) is converted alone ([`Scale::px`] of the length,
//!   [`Scale::line`] for a line that must not vanish): the same logical
//!   length is the same physical one wherever it is drawn.
//! - **A client's buffer** for a surface of `w` × `h` logical pixels is
//!   `px(w)` × `px(h)` pixels ([`Scale::buffer_size`]; fractional-scale-v1:
//!   the logical size times the scale, rounded).
//! - **Physical to logical** takes the floor ([`Scale::to_logical`]): a
//!   physical pixel belongs to the logical one it lies in; an output's
//!   logical size is the floor of its physical size over the scale
//!   ([`Scale::output_size`]) so no logical pixel is cut by the screen's edge.
//! - **Damage** is converted outwards ([`Scale::rect_out`], which covers the
//!   rectangle [`Scale::rect`] draws), and physical damage back to logical
//!   outwards ([`Scale::rect_to_logical_out`]).

use super::Rect;
use crate::wire::Fixed;
use alloc::format;
use alloc::string::String;
use core::ops::Mul;

/// The unit `wp_fractional_scale_v1` counts in: one 120th.
pub const UNIT: u32 = 120;

/// A scale between 100 % and 800 %, in 120ths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Scale(u32);

impl Default for Scale {
    fn default() -> Scale {
        Scale::ONE
    }
}

impl Scale {
    /// 100 %.
    pub const ONE: Scale = Scale(UNIT);
    /// The largest scale taken.
    pub const MAX: Scale = Scale(8 * UNIT);

    /// The scales Settings offers: 100, 125, 150, 175 and 200 %.
    pub const PRESETS: [Scale; 5] = [Scale(120), Scale(150), Scale(180), Scale(210), Scale(240)];

    /// `v` 120ths, kept between 100 % and 800 %.
    pub const fn from_120ths(v: u32) -> Scale {
        if v < UNIT {
            Scale(UNIT)
        } else if v > 8 * UNIT {
            Scale(8 * UNIT)
        } else {
            Scale(v)
        }
    }

    /// A whole factor (1 to 8 are kept, others clamped).
    pub const fn from_int(n: u32) -> Scale {
        Scale::from_120ths(n.saturating_mul(UNIT))
    }

    /// `p` percent, rounded to the nearest 120th.
    pub const fn from_percent(p: u32) -> Scale {
        Scale::from_120ths((p.saturating_mul(UNIT) + 50) / 100)
    }

    /// The scale in 120ths.
    pub const fn as_120ths(self) -> u32 {
        self.0
    }

    /// The scale in percent (rounded).
    pub const fn percent(self) -> u32 {
        (self.0 * 100 + UNIT / 2) / UNIT
    }

    /// Whether this is a whole number of physical pixels per logical one.
    pub const fn is_integer(self) -> bool {
        self.0.is_multiple_of(UNIT)
    }

    /// The next whole number at or above: what `wl_output.scale` and
    /// `wl_surface.preferred_buffer_scale` say to clients that do not know
    /// fractional scales (they draw at it, the compositor scales down).
    pub const fn ceil(self) -> u32 {
        self.0.div_ceil(UNIT)
    }

    /// The factor as a float (font sizes, anti-aliased shapes).
    pub fn factor(self) -> f32 {
        self.0 as f32 / UNIT as f32
    }

    /// Read a scale: `1.5`, `150%`, `2` (a whole factor from 1 to 8), or
    /// `150` (a percentage from 100 up).
    pub fn parse(text: &str) -> Option<Scale> {
        let t = text.trim();
        let (digits, percent) = match t.strip_suffix('%') {
            Some(d) => (d.trim(), true),
            None => (t, false),
        };
        let valid = |p: u32| (100..=800).contains(&p).then(|| Scale::from_percent(p));
        if let Some((whole, frac)) = digits.split_once('.') {
            if percent || frac.is_empty() || frac.len() > 3 {
                return None;
            }
            let whole: u32 = whole.parse().ok()?;
            let frac_digits: u32 = frac.parse().ok()?;
            // Thousandths, then percent.
            let thousandths = whole.checked_mul(1000)? + frac_digits * 10u32.pow(3 - frac.len() as u32);
            return valid((thousandths + 5) / 10);
        }
        let n: u32 = digits.parse().ok()?;
        if percent || n >= 100 { valid(n) } else { (1..=8).contains(&n).then(|| Scale::from_int(n)) }
    }

    /// The text the settings file keeps: the percentage (`150`).
    pub fn to_text(self) -> String {
        format!("{}", self.percent())
    }

    // Logical to physical.

    /// The physical pixel a logical position or length of `v` lies at:
    /// `v` times the scale, rounded to the nearest, halves up.
    pub const fn px(self, v: i32) -> i32 {
        let n = 2 * v as i64 * self.0 as i64 + UNIT as i64;
        let d = 2 * UNIT as i64;
        let r = n.div_euclid(d);
        if r > i32::MAX as i64 {
            i32::MAX
        } else if r < i32::MIN as i64 {
            i32::MIN
        } else {
            r as i32
        }
    }

    /// A line of `v` logical pixels (a border, a stroke): [`Scale::px`], but
    /// at least one pixel for any positive `v`.
    pub const fn line(self, v: i32) -> i32 {
        let p = self.px(v);
        if v > 0 && p < 1 { 1 } else { p }
    }

    /// A rectangle by its edges: `px(left)`, `px(top)`, `px(right)`,
    /// `px(bottom)`.
    pub fn rect(self, r: Rect) -> Rect {
        let (x0, y0) = (self.px(r.x), self.px(r.y));
        Rect::from_edges(x0, y0, self.px(r.right()).max(x0), self.px(r.bottom()).max(y0))
    }

    /// The smallest physical rectangle that holds the exact scaled `r`:
    /// edges rounded outwards. It covers [`Scale::rect`] of the same
    /// rectangle, so it is what damage uses.
    pub fn rect_out(self, r: Rect) -> Rect {
        let (x0, y0) = (self.floor_px(r.x), self.floor_px(r.y));
        Rect::from_edges(x0, y0, self.ceil_px(r.right()).max(x0), self.ceil_px(r.bottom()).max(y0))
    }

    const fn floor_px(self, v: i32) -> i32 {
        (v as i64 * self.0 as i64).div_euclid(UNIT as i64) as i32
    }

    const fn ceil_px(self, v: i32) -> i32 {
        let n = v as i64 * self.0 as i64 + UNIT as i64 - 1;
        n.div_euclid(UNIT as i64) as i32
    }

    /// The pixels of the buffer a client draws for a surface of `w` × `h`
    /// logical pixels (`wp_fractional_scale_v1`: size times scale, rounded).
    pub const fn buffer_size(self, w: i32, h: i32) -> (i32, i32) {
        (self.px(w), self.px(h))
    }

    // Physical to logical.

    /// The logical pixel physical pixel `p` lies in (floor).
    pub const fn to_logical(self, p: i32) -> i32 {
        (p as i64 * UNIT as i64).div_euclid(self.0 as i64) as i32
    }

    /// Physical position `p` as a logical position in 1/256 pixels
    /// (`wl_fixed`), exact to that precision, for pointer and touch input.
    pub fn to_logical_fixed(self, p: i32) -> Fixed {
        self.physical_to_logical(Fixed::from_int(p))
    }

    /// A physical position or distance `p` (itself in 1/256 pixels) as a
    /// logical one: what a pointer device's motion, a tablet's or a touch
    /// screen's position becomes before it reaches the seat.
    pub fn physical_to_logical(self, p: Fixed) -> Fixed {
        Fixed((i64::from(p.0) * i64::from(UNIT) / i64::from(self.0)) as i32)
    }

    /// A physical rectangle as logical pixels, rounded outwards.
    pub fn rect_to_logical_out(self, r: Rect) -> Rect {
        let to_ceil =
            |p: i32| (i64::from(p) * i64::from(UNIT) + i64::from(self.0) - 1).div_euclid(i64::from(self.0)) as i32;
        let (x0, y0) = (self.to_logical(r.x), self.to_logical(r.y));
        Rect::from_edges(x0, y0, to_ceil(r.right()).max(x0), to_ceil(r.bottom()).max(y0))
    }

    /// The logical size of an output of `width` × `height` physical pixels.
    pub const fn output_size(self, width: u32, height: u32) -> (i32, i32) {
        (self.to_logical(width as i32), self.to_logical(height as i32))
    }
}

impl From<u32> for Scale {
    /// A whole factor.
    fn from(n: u32) -> Scale {
        Scale::from_int(n)
    }
}

impl core::fmt::Display for Scale {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}%", self.percent())
    }
}

/// `length * scale`: [`Scale::px`], so shell code reads `RADIUS * s`.
impl Mul<Scale> for i32 {
    type Output = i32;
    fn mul(self, s: Scale) -> i32 {
        s.px(self)
    }
}

impl Mul<i32> for Scale {
    type Output = i32;
    fn mul(self, v: i32) -> i32 {
        self.px(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S125: Scale = Scale::from_percent(125);
    const S150: Scale = Scale::from_percent(150);
    const S175: Scale = Scale::from_percent(175);

    #[test]
    fn presets_are_exact_120ths() {
        let p: [u32; 5] = Scale::PRESETS.map(|s| s.as_120ths());
        assert_eq!(p, [120, 150, 180, 210, 240]);
        assert_eq!(Scale::PRESETS.map(|s| s.percent()), [100, 125, 150, 175, 200]);
        assert_eq!(Scale::PRESETS.map(|s| s.ceil()), [1, 2, 2, 2, 2]);
        assert_eq!(Scale::PRESETS.map(|s| s.is_integer()), [true, false, false, false, true]);
    }

    #[test]
    fn positions_round_to_the_nearest_pixel_halves_up() {
        assert_eq!((0..6).map(|v| S150.px(v)).collect::<alloc::vec::Vec<_>>(), [0, 2, 3, 5, 6, 8]);
        assert_eq!((0..5).map(|v| S125.px(v)).collect::<alloc::vec::Vec<_>>(), [0, 1, 3, 4, 5]);
        assert_eq!((0..5).map(|v| S175.px(v)).collect::<alloc::vec::Vec<_>>(), [0, 2, 4, 5, 7]);
        // Negative positions follow (halves up, so -1 at 150 % is -1.5 -> -1).
        assert_eq!(S150.px(-1), -1);
        assert_eq!(S150.px(-2), -3);
        // Whole scales are exact.
        assert_eq!(Scale::from_int(2).px(7), 14);
        assert_eq!(Scale::ONE.px(7), 7);
        // No overflow.
        assert_eq!(S150.px(i32::MAX), i32::MAX);
        assert_eq!(S150.px(i32::MIN), i32::MIN);
    }

    #[test]
    fn lines_never_vanish() {
        assert_eq!(Scale::ONE.line(1), 1);
        assert_eq!(S125.line(1), 1);
        assert_eq!(S150.line(1), 2);
        assert_eq!(S175.line(1), 2);
        assert_eq!(S150.line(0), 0);
        assert_eq!(S150.line(-1), -1);
    }

    #[test]
    fn touching_rectangles_still_touch() {
        // Rows of 33 logical pixels: each row's bottom is the next row's top,
        // at every scale, and the total is the total's own conversion.
        for s in Scale::PRESETS {
            let mut prev_bottom = None;
            for i in 0..40 {
                let r = s.rect(Rect::new(3, i * 33, 100, 33));
                if let Some(b) = prev_bottom {
                    assert_eq!(r.y, b, "{s}: row {i} starts where the last ended");
                }
                prev_bottom = Some(r.bottom());
            }
            assert_eq!(prev_bottom, Some(s.px(40 * 33)));
        }
        // A 1.25 row can be one pixel taller than another: the edges, not
        // the heights, are what is rounded.
        let heights: alloc::vec::Vec<i32> = (0..4).map(|i| S125.rect(Rect::new(0, i * 33, 1, 33)).height).collect();
        assert_eq!(heights, [41, 42, 41, 41]);
    }

    #[test]
    fn rects_and_damage() {
        let r = Rect::new(3, 5, 10, 7);
        assert_eq!(S150.rect(r), Rect::new(5, 8, 15, 10));
        // Outwards: 4.5..19.5 -> 4..20, 7.5..18 -> 7..18.
        assert_eq!(S150.rect_out(r), Rect::new(4, 7, 16, 11));
        for s in Scale::PRESETS {
            for x in 0..9 {
                for w in 0..9 {
                    let r = Rect::new(x, x * 2, w, w + 1);
                    let (a, b) = (s.rect(r), s.rect_out(r));
                    if !a.is_empty() {
                        assert_eq!(a.union(&b), b, "{s}: the damage of {r:?} covers what is drawn");
                    }
                }
            }
        }
        assert!(S150.rect(Rect::new(1, 1, 0, 5)).is_empty());
    }

    #[test]
    fn physical_pixels_belong_to_one_logical_pixel() {
        assert_eq!((0..8).map(|p| S150.to_logical(p)).collect::<alloc::vec::Vec<_>>(), [0, 0, 1, 2, 2, 3, 4, 4]);
        assert_eq!((0..6).map(|p| S125.to_logical(p)).collect::<alloc::vec::Vec<_>>(), [0, 0, 1, 2, 3, 4]);
        for s in Scale::PRESETS {
            for l in 0..50 {
                // The first pixel of logical pixel `l` converts back to `l`.
                let p = s.rect(Rect::new(l, 0, 1, 1)).x;
                assert!(s.to_logical(p) <= l && s.to_logical(p + 1) >= l, "{s}: {l} -> {p}");
                // A pixel inside the converted rectangle maps back inside the
                // logical one (within the rounding of the edge).
                let r = s.rect(Rect::new(l, 0, 1, 1));
                if !r.is_empty() {
                    let back = s.to_logical(r.x + r.width / 2);
                    assert_eq!(back, l, "{s}: the middle of {l} is in {l}");
                }
            }
        }
        assert_eq!(S150.to_logical_fixed(3), Fixed(512));
        assert_eq!(S150.physical_to_logical(Fixed(3 * 256 + 128)), Fixed(512 + 85));
        assert_eq!(Scale::ONE.physical_to_logical(Fixed(12345)), Fixed(12345));
        assert_eq!(S150.to_logical_fixed(1).0, 170);
        assert_eq!(Scale::from_int(2).to_logical_fixed(5), Fixed(640));
        assert_eq!(S150.rect_to_logical_out(Rect::new(5, 8, 15, 10)), Rect::new(3, 5, 11, 7));
        assert_eq!(S150.rect_to_logical_out(Rect::new(1, 1, 1, 1)), Rect::new(0, 0, 2, 2));
    }

    #[test]
    fn output_sizes_leave_no_cut_pixel() {
        assert_eq!(S150.output_size(1280, 800), (853, 533));
        assert_eq!(S125.output_size(1280, 800), (1024, 640));
        assert_eq!(S175.output_size(1280, 800), (731, 457));
        assert_eq!(Scale::from_int(2).output_size(2000, 1280), (1000, 640));
        for s in Scale::PRESETS {
            for phys in [640u32, 1000, 1280, 1313, 1920, 2000, 2880] {
                let (l, _) = s.output_size(phys, phys);
                assert!(s.px(l) <= phys as i32, "{s}: {l} logical pixels fit in {phys}");
                assert!(s.px(l + 1) > phys as i32 - 1, "{s}: and one more would not (nearly)");
            }
        }
    }

    #[test]
    fn buffers_of_clients() {
        assert_eq!(S150.buffer_size(853, 533), (1280, 800));
        assert_eq!(S125.buffer_size(1001, 3), (1251, 4));
        assert_eq!(Scale::from_int(2).buffer_size(10, 20), (20, 40));
    }

    #[test]
    fn operators_are_px() {
        assert_eq!(7 * S150, S150.px(7));
        assert_eq!(S150 * 7, S150.px(7));
    }

    #[test]
    fn reading_and_writing() {
        for (text, want) in [
            ("1", 100),
            ("2", 200),
            ("1.5", 150),
            ("1.25", 125),
            ("1.75", 175),
            ("150", 150),
            ("150%", 150),
            (" 125 % ", 125),
            ("100", 100),
            ("2.0", 200),
            ("8", 800),
        ] {
            assert_eq!(Scale::parse(text).map(Scale::percent), Some(want), "{text}");
        }
        for bad in ["", "0", "9", "99", "801", "1.", ".5", "1.5%", "abc", "-2", "1.2345", "3.5.1"] {
            assert_eq!(Scale::parse(bad), None, "{bad:?}");
        }
        assert_eq!(S150.to_text(), "150");
        assert_eq!(Scale::parse(&S175.to_text()), Some(S175));
        assert_eq!(alloc::format!("{S125}"), "125%");
        assert_eq!(Scale::from_120ths(5), Scale::ONE);
        assert_eq!(Scale::from_120ths(99_999), Scale::MAX);
    }
}
