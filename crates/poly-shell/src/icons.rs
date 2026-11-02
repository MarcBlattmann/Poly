//! Icons: Alpine Shell draws its glyphs with the Lucide icon set
//! (lucide-react 0.575.0, ISC licence, `third_party/lucide/LICENSE`), so
//! the shell carries the path data of the icons it uses and strokes them
//! the way an SVG renderer does: 24 × 24 units, 2-unit round-capped,
//! round-joined lines, scaled to the icon's pixel size (ADR 0127).
//!
//! [`Icon::draw`] parses the SVG path data (`M L H V C S Q T A Z`, absolute
//! and relative, SVG 1.1 §8.3), flattens curves and arcs into line
//! segments (arcs converted from endpoint to centre form, SVG 1.1 F.6.5)
//! and covers each pixel by its distance to the nearest segment.

use alloc::vec::Vec;
use poly_wayland::compose::paint::{fabs, floor, mix, sqrt};
use poly_wayland::compose::{Framebuffer, Rect};

/// An icon: Lucide's SVG path data, one string per element (circles,
/// rectangles and lines written as paths).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon(pub &'static [&'static str]);

/// π.
const PI: f32 = core::f32::consts::PI;

/// sin and cos of `a` (radians): reduced to [-π, π], then their Taylor
/// series to the 11th power (error below 1e-5 there).
pub fn sin_cos(a: f32) -> (f32, f32) {
    let tau = 2.0 * PI;
    let mut x = a - tau * floor(a / tau + 0.5);
    if x > PI {
        x -= tau;
    }
    let x2 = x * x;
    let sin = x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0 * (1.0 - x2 / 110.0)))));
    let cos = 1.0
        - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0 * (1.0 - x2 / 90.0 * (1.0 - x2 / 132.0)))));
    (sin, cos)
}

/// atan2(y, x) by a minimax polynomial for atan on [0, 1] (error about
/// 1e-5 radians).
pub fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (fabs(x), fabs(y));
    let z = if ax >= ay { ay / ax } else { ax / ay };
    let z2 = z * z;
    let mut a = z * (0.999_866 + z2 * (-0.330_299_5 + z2 * (0.180_141 + z2 * (-0.085_133 + z2 * 0.020_835_1))));
    if ay > ax {
        a = PI / 2.0 - a;
    }
    if x < 0.0 {
        a = PI - a;
    }
    if y < 0.0 { -a } else { a }
}

/// Reads SVG path data.
struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn at_number(&mut self) -> bool {
        self.skip();
        self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    fn sign(&mut self) -> bool {
        match self.s.get(self.i) {
            Some(b'-') => {
                self.i += 1;
                true
            }
            Some(b'+') => {
                self.i += 1;
                false
            }
            _ => false,
        }
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let neg = self.sign();
        let mut v = 0.0f32;
        let mut digits = 0;
        while let Some(&c) = self.s.get(self.i).filter(|c| c.is_ascii_digit()) {
            v = v * 10.0 + f32::from(c - b'0');
            self.i += 1;
            digits += 1;
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            let mut f = 0.1f32;
            while let Some(&c) = self.s.get(self.i).filter(|c| c.is_ascii_digit()) {
                v += f * f32::from(c - b'0');
                f *= 0.1;
                self.i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            self.i = start;
            return None;
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            let mark = self.i;
            self.i += 1;
            let eneg = self.sign();
            let mut e = 0i32;
            let first = self.i;
            while let Some(&c) = self.s.get(self.i).filter(|c| c.is_ascii_digit()) {
                e = e * 10 + i32::from(c - b'0');
                self.i += 1;
            }
            if self.i == first {
                self.i = mark;
            } else {
                for _ in 0..e.min(38) {
                    v = if eneg { v / 10.0 } else { v * 10.0 };
                }
            }
        }
        Some(if neg { -v } else { v })
    }

    /// An arc's flag: one character, `0` or `1`.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }

    fn pair(&mut self) -> Option<(f32, f32)> {
        Some((self.number()?, self.number()?))
    }
}

/// A polyline in icon units.
type Line = Vec<(f32, f32)>;

/// Flatten path data `d` into polylines, curves in `steps` segments.
fn flatten(d: &str, steps: usize) -> Vec<Line> {
    let mut lx = Lexer { s: d.as_bytes(), i: 0 };
    let mut out: Vec<Line> = Vec::new();
    let mut cur: Line = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    // The last control point, for S and T.
    let mut ctrl: Option<(f32, f32)> = None;
    let mut cmd = b'M';
    loop {
        if let Some(c) = lx.command() {
            cmd = c;
        } else if !lx.at_number() {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        let mut next_ctrl = None;
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let Some((a, b)) = lx.pair() else { break };
                if cur.len() > 1 {
                    out.push(core::mem::take(&mut cur));
                }
                cur.clear();
                (x, y) = (ox + a, oy + b);
                (sx, sy) = (x, y);
                cur.push((x, y));
                // Further pairs are lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let Some((a, b)) = lx.pair() else { break };
                (x, y) = (ox + a, oy + b);
                cur.push((x, y));
            }
            b'H' => {
                let Some(a) = lx.number() else { break };
                x = ox + a;
                cur.push((x, y));
            }
            b'V' => {
                let Some(b) = lx.number() else { break };
                y = oy + b;
                cur.push((x, y));
            }
            b'C' | b'S' => {
                let (x1, y1) = if cmd.eq_ignore_ascii_case(&b'C') {
                    let Some((a, b)) = lx.pair() else { break };
                    (ox + a, oy + b)
                } else {
                    ctrl.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy))
                };
                let (Some((a, b)), Some((c, e))) = (lx.pair(), lx.pair()) else { break };
                let (x2, y2, ex, ey) = (ox + a, oy + b, ox + c, oy + e);
                for k in 1..=steps {
                    let t = k as f32 / steps as f32;
                    let u = 1.0 - t;
                    let (p, q, r, s) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push((p * x + q * x1 + r * x2 + s * ex, p * y + q * y1 + r * y2 + s * ey));
                }
                next_ctrl = Some((x2, y2));
                (x, y) = (ex, ey);
            }
            b'Q' | b'T' => {
                let (x1, y1) = if cmd.eq_ignore_ascii_case(&b'Q') {
                    let Some((a, b)) = lx.pair() else { break };
                    (ox + a, oy + b)
                } else {
                    ctrl.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy))
                };
                let Some((a, b)) = lx.pair() else { break };
                let (ex, ey) = (ox + a, oy + b);
                for k in 1..=steps {
                    let t = k as f32 / steps as f32;
                    let u = 1.0 - t;
                    cur.push((u * u * x + 2.0 * u * t * x1 + t * t * ex, u * u * y + 2.0 * u * t * y1 + t * t * ey));
                }
                next_ctrl = Some((x1, y1));
                (x, y) = (ex, ey);
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(_rotation)) = (lx.number(), lx.number(), lx.number()) else { break };
                let (Some(large), Some(sweep)) = (lx.flag(), lx.flag()) else { break };
                let Some((a, b)) = lx.pair() else { break };
                let (ex, ey) = (ox + a, oy + b);
                arc(&mut cur, (x, y), (ex, ey), (fabs(rx), fabs(ry)), large, sweep, steps);
                (x, y) = (ex, ey);
            }
            b'Z' => {
                cur.push((sx, sy));
                (x, y) = (sx, sy);
                if cur.len() > 1 {
                    out.push(core::mem::take(&mut cur));
                }
                cur.push((x, y));
            }
            _ => break,
        }
        ctrl = next_ctrl;
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// An elliptical arc (axes along x and y, as all of Lucide's are) from
/// `p` to `e` with radii `r`, appended as points.
fn arc(out: &mut Line, p: (f32, f32), e: (f32, f32), r: (f32, f32), large: bool, sweep: bool, steps: usize) {
    let (mut rx, mut ry) = r;
    if rx == 0.0 || ry == 0.0 || (p.0 == e.0 && p.1 == e.1) {
        out.push(e);
        return;
    }
    let (x1p, y1p) = ((p.0 - e.0) / 2.0, (p.1 - e.1) / 2.0);
    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        let l = sqrt(lambda);
        rx *= l;
        ry *= l;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 { sqrt((num / den).max(0.0)) } else { 0.0 };
    if large == sweep {
        coef = -coef;
    }
    let (cxp, cyp) = (coef * rx * y1p / ry, -coef * ry * x1p / rx);
    let (cx, cy) = (cxp + (p.0 + e.0) / 2.0, cyp + (p.1 + e.1) / 2.0);
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let t1 = angle(1.0, 0.0, ux, uy);
    let mut dt = angle(ux, uy, vx, vy);
    if !sweep && dt > 0.0 {
        dt -= 2.0 * PI;
    } else if sweep && dt < 0.0 {
        dt += 2.0 * PI;
    }
    let n = ((steps as f32) * fabs(dt) / (PI / 2.0)).max(2.0) as usize;
    for k in 1..=n {
        let (s, c) = sin_cos(t1 + dt * k as f32 / n as f32);
        out.push((cx + rx * c, cy + ry * s));
    }
    // The exact end, so joins meet.
    if let Some(last) = out.last_mut() {
        *last = e;
    }
}

/// The distance from (`px`, `py`) to the segment `a`–`b`.
fn segment_distance(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
    sqrt(qx * qx + qy * qy)
}

/// A segment from one point to another, in pixels.
type Segment = ((f32, f32), (f32, f32));

impl Icon {
    /// The icon's segments at `size` pixels, placed with its top left
    /// corner at (`x`, `y`) (physical pixels).
    fn segments(&self, x: f32, y: f32, size: f32) -> Vec<Segment> {
        let k = size / 24.0;
        let steps = if size <= 20.0 { 6 } else { 10 };
        let mut segs = Vec::new();
        for d in self.0 {
            for line in flatten(d, steps) {
                let pts: Vec<(f32, f32)> = line.iter().map(|&(u, v)| (x + u * k, y + v * k)).collect();
                if pts.len() == 1 {
                    segs.push((pts[0], pts[0]));
                }
                for w in pts.windows(2) {
                    segs.push((w[0], w[1]));
                }
            }
        }
        segs
    }

    /// Draw the icon `size` pixels square with its top left corner at
    /// (`x`, `y`), Lucide's 2-unit stroke, in `rgb` at `alpha`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, x: f32, y: f32, size: f32, rgb: u32, alpha: u8) {
        self.draw_stroke(fb, clip, (x, y), size, 2.0, rgb, alpha);
    }

    /// [`Icon::draw`] with another stroke width (in icon units).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_stroke(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        at: (f32, f32),
        size: f32,
        stroke: f32,
        rgb: u32,
        alpha: u8,
    ) {
        let (x, y) = at;
        let segs = self.segments(x, y, size);
        let w = stroke * size / 24.0;
        let half = w / 2.0;
        let x0 = floor(x - half - 1.0) as i32;
        let y0 = floor(y - half - 1.0) as i32;
        let side = (size + w + 3.0) as i32;
        let screen = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
        let area = Rect::new(x0, y0, side, side).intersect(&clip).intersect(&screen);
        // A stroke thinner than a pixel covers at most its width of one.
        let peak = w.min(1.0);
        for py in area.y..area.bottom() {
            let row = fb.row_mut(py as u32);
            for px in area.x..area.right() {
                let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
                let d = segs.iter().fold(f32::MAX, |m, &(a, b)| m.min(segment_distance(cx, cy, a, b)));
                let cov = (half + 0.5 - d).clamp(0.0, peak);
                if cov > 0.0 {
                    let a = (cov * f32::from(alpha) + 0.5) as u32;
                    row[px as usize] = mix(row[px as usize], rgb, a);
                }
            }
        }
    }
}

/// The icons the shell uses (Lucide's names).
pub mod lucide {
    use super::Icon;

    /// lucide `app-window`.
    pub const APP_WINDOW: Icon = Icon(&[
        "M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z",
        "M10 4v4",
        "M2 8h20",
        "M6 4v4",
    ]);
    /// lucide `bell`.
    pub const BELL: Icon = Icon(&[
        "M10.268 21a2 2 0 0 0 3.464 0",
        "M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326",
    ]);
    /// lucide `calendar-clock`.
    pub const CALENDAR_CLOCK: Icon = Icon(&[
        "M16 14v2.2l1.6 1",
        "M16 2v4",
        "M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h3.5",
        "M3 10h5",
        "M8 2v4",
        "M10 16a6 6 0 1 0 12 0a6 6 0 1 0 -12 0",
    ]);
    /// lucide `check`.
    pub const CHECK: Icon = Icon(&["M20 6 9 17l-5-5"]);
    /// lucide `chevron-left`.
    pub const CHEVRON_LEFT: Icon = Icon(&["m15 18-6-6 6-6"]);
    /// lucide `chevron-right`.
    pub const CHEVRON_RIGHT: Icon = Icon(&["m9 18 6-6-6-6"]);
    /// lucide `chevron-down`.
    pub const CHEVRON_DOWN: Icon = Icon(&["m6 9 6 6 6-6"]);
    /// lucide `circle-dashed`.
    pub const CIRCLE_DASHED: Icon = Icon(&[
        "M10.1 2.182a10 10 0 0 1 3.8 0",
        "M13.9 21.818a10 10 0 0 1-3.8 0",
        "M17.609 3.721a10 10 0 0 1 2.69 2.7",
        "M2.182 13.9a10 10 0 0 1 0-3.8",
        "M20.279 17.609a10 10 0 0 1-2.7 2.69",
        "M21.818 10.1a10 10 0 0 1 0 3.8",
        "M3.721 6.391a10 10 0 0 1 2.7-2.69",
        "M6.391 20.279a10 10 0 0 1-2.69-2.7",
    ]);
    /// lucide `ethernet-port`.
    pub const ETHERNET_PORT: Icon = Icon(&[
        "m15 20 3-3h2a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h2l3 3z",
        "M6 8v1",
        "M10 8v1",
        "M14 8v1",
        "M18 8v1",
    ]);
    /// lucide `house`.
    pub const HOUSE: Icon = Icon(&[
        "M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8",
        "M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
    ]);
    /// lucide `info`.
    pub const INFO: Icon = Icon(&["M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0", "M12 16v-4", "M12 8h.01"]);
    /// lucide `keyboard`.
    pub const KEYBOARD: Icon = Icon(&[
        "M10 8h.01",
        "M12 12h.01",
        "M14 8h.01",
        "M16 12h.01",
        "M18 8h.01",
        "M6 8h.01",
        "M7 16h10",
        "M8 12h.01",
        "M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z",
    ]);
    /// lucide `log-out`.
    pub const LOG_OUT: Icon = Icon(&["m16 17 5-5-5-5", "M21 12H9", "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"]);
    /// lucide `monitor`.
    pub const MONITOR: Icon = Icon(&[
        "M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2z",
        "M8 21L16 21",
        "M12 17L12 21",
    ]);
    /// lucide `power`.
    pub const POWER: Icon = Icon(&["M12 2v10", "M18.4 6.6a9 9 0 1 1-12.77.04"]);
    /// lucide `rotate-ccw`.
    pub const ROTATE_CCW: Icon = Icon(&["M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8", "M3 3v5h5"]);
    /// lucide `search`.
    pub const SEARCH: Icon = Icon(&["m21 21-4.34-4.34", "M3 11a8 8 0 1 0 16 0a8 8 0 1 0 -16 0"]);
    /// lucide `settings`.
    pub const SETTINGS: Icon = Icon(&[
        "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915",
        "M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0",
    ]);
    /// lucide `speaker`.
    pub const SPEAKER: Icon = Icon(&[
        "M6 2h12a2 2 0 0 1 2 2v16a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-16a2 2 0 0 1 2 -2z",
        "M12 6h.01",
        "M8 14a4 4 0 1 0 8 0a4 4 0 1 0 -8 0",
        "M12 14h.01",
    ]);
    /// lucide `volume`.
    pub const VOLUME: Icon = Icon(&[
        "M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z",
    ]);
    /// lucide `volume-1`.
    pub const VOLUME_1: Icon = Icon(&[
        "M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z",
        "M16 9a5 5 0 0 1 0 6",
    ]);
    /// lucide `volume-2`.
    pub const VOLUME_2: Icon = Icon(&[
        "M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z",
        "M16 9a5 5 0 0 1 0 6",
        "M19.364 18.364a9 9 0 0 0 0-12.728",
    ]);
    /// lucide `volume-x`.
    pub const VOLUME_X: Icon = Icon(&[
        "M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z",
        "M22 9L16 15",
        "M16 9L22 15",
    ]);
    /// lucide `wallpaper`.
    pub const WALLPAPER: Icon = Icon(&[
        "M12 17v4",
        "M8 21h8",
        "m9 17 6.1-6.1a2 2 0 0 1 2.81.01L22 15",
        "M6 9a2 2 0 1 0 4 0a2 2 0 1 0 -4 0",
        "M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2z",
    ]);
    /// lucide `wifi`.
    pub const WIFI: Icon =
        Icon(&["M12 20h.01", "M2 8.82a15 15 0 0 1 20 0", "M5 12.859a10 10 0 0 1 14 0", "M8.5 16.429a5 5 0 0 1 7 0"]);
    /// lucide `wifi-off`.
    pub const WIFI_OFF: Icon = Icon(&[
        "M12 20h.01",
        "M8.5 16.429a5 5 0 0 1 7 0",
        "M5 12.859a10 10 0 0 1 5.17-2.69",
        "M19 12.859a10 10 0 0 0-2.007-1.523",
        "M2 8.82a15 15 0 0 1 4.177-2.643",
        "M22 8.82a15 15 0 0 0-11.288-3.764",
        "m2 2 20 20",
    ]);
    /// lucide `x`.
    pub const X: Icon = Icon(&["M18 6 6 18", "m6 6 12 12"]);
    /// lucide `minus`.
    pub const MINUS: Icon = Icon(&["M5 12h14"]);
    /// lucide `square`.
    pub const SQUARE: Icon = Icon(&["M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2z"]);
    /// lucide `copy`.
    pub const COPY: Icon = Icon(&[
        "M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2z",
        "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2",
    ]);
    /// lucide `arrow-left`.
    pub const ARROW_LEFT: Icon = Icon(&["m12 19-7-7 7-7", "M19 12H5"]);
    /// lucide `arrow-right`.
    pub const ARROW_RIGHT: Icon = Icon(&["M5 12h14", "m12 5 7 7-7 7"]);
    /// lucide `sun`.
    pub const SUN: Icon = Icon(&[
        "M8 12a4 4 0 1 0 8 0a4 4 0 1 0 -8 0",
        "M12 2v2",
        "M12 20v2",
        "m4.93 4.93 1.41 1.41",
        "m17.66 17.66 1.41 1.41",
        "M2 12h2",
        "M20 12h2",
        "m6.34 17.66-1.41 1.41",
        "m19.07 4.93-1.41 1.41",
    ]);
    /// lucide `bluetooth-off`.
    pub const BLUETOOTH_OFF: Icon = Icon(&["m17 17-5 5V12l-5 5", "m2 2 20 20", "M14.5 9.5 17 7l-5-5v4.5"]);
    /// lucide `bluetooth`.
    pub const BLUETOOTH: Icon = Icon(&["m7 7 10 10-5 5V2l5 5L7 17"]);

    /// Every icon above (for tests).
    pub const ALL: &[Icon] = &[
        APP_WINDOW,
        BELL,
        CALENDAR_CLOCK,
        CHECK,
        CHEVRON_LEFT,
        CHEVRON_RIGHT,
        CHEVRON_DOWN,
        CIRCLE_DASHED,
        ETHERNET_PORT,
        HOUSE,
        INFO,
        KEYBOARD,
        LOG_OUT,
        MONITOR,
        POWER,
        ROTATE_CCW,
        SEARCH,
        SETTINGS,
        SPEAKER,
        VOLUME,
        VOLUME_1,
        VOLUME_2,
        VOLUME_X,
        WALLPAPER,
        WIFI,
        WIFI_OFF,
        X,
        MINUS,
        SQUARE,
        COPY,
        ARROW_LEFT,
        ARROW_RIGHT,
        SUN,
        BLUETOOTH_OFF,
        BLUETOOTH,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::compose::MemoryFramebuffer;

    #[test]
    fn trigonometry() {
        for a in [0.0f32, 0.5, 1.0, 2.0, 3.0, -1.0, 4.0, 7.0] {
            let (s, c) = sin_cos(a);
            assert!(fabs(s * s + c * c - 1.0) < 1e-3, "{a}");
            let wrapped = a - 2.0 * PI * floor(a / (2.0 * PI) + 0.5);
            assert!(fabs(atan2(s, c) - wrapped) < 1e-3, "{a}");
        }
        assert!(fabs(atan2(1.0, 0.0) - PI / 2.0) < 1e-4);
        assert!(fabs(atan2(0.0, -1.0) - PI) < 1e-4);
    }

    #[test]
    fn path_data_is_parsed_with_implicit_commands_and_packed_numbers() {
        // `M20 6 9 17l-5-5`: a move, then implicit lines.
        let l = flatten("M20 6 9 17l-5-5", 4);
        assert_eq!(l, [alloc::vec![(20.0, 6.0), (9.0, 17.0), (4.0, 12.0)]]);
        // Numbers run together: `h.5-1.25`.
        let l = flatten("M1 1h.5-1.25", 4);
        assert_eq!(l[0], [(1.0, 1.0), (1.5, 1.0), (0.25, 1.0)]);
        // A closed rectangle returns to its start.
        let l = flatten("M2 2h4v4h-4z", 4);
        assert_eq!(l[0].first(), l[0].last());
        // A full circle as two half arcs stays on the circle.
        let l = flatten("M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0", 8);
        for &(x, y) in &l[0] {
            let r = sqrt((x - 12.0) * (x - 12.0) + (y - 12.0) * (y - 12.0));
            assert!(fabs(r - 10.0) < 0.01, "{x},{y}: {r}");
        }
        // Arc flags packed without separators (`0 011`).
        assert_eq!(flatten("M0 0a5 5 0 011 1", 4), flatten("M0 0a5 5 0 0 1 1 1", 4));
    }

    #[test]
    fn icons_are_stroked_where_their_paths_run() {
        // The check mark at 24 px: its stroke passes (9, 17), not (20, 20).
        let mut fb = MemoryFramebuffer::new(24, 24);
        lucide::CHECK.draw(&mut fb, Rect::new(0, 0, 24, 24), 0.0, 0.0, 24.0, 0xff_ffff, 255);
        assert!(fb.pixel(9, 16).unwrap() & 0xff > 200);
        assert_eq!(fb.pixel(20, 20).unwrap() & 0xff, 0);
        // At 12 px everything shrinks: the X's centre is lit, the corners
        // beyond the stroke's reach stay dark.
        let mut fb = MemoryFramebuffer::new(12, 12);
        lucide::X.draw(&mut fb, Rect::new(0, 0, 12, 12), 0.0, 0.0, 12.0, 0xff_ffff, 255);
        assert!(fb.pixel(5, 5).unwrap() & 0xff > 150);
        assert_eq!(fb.pixel(0, 0).unwrap() & 0xff, 0);
        assert_eq!(fb.pixel(11, 0).unwrap() & 0xff, 0);
        // Clipped.
        let mut fb = MemoryFramebuffer::new(24, 24);
        lucide::SETTINGS.draw(&mut fb, Rect::new(0, 0, 12, 24), 0.0, 0.0, 24.0, 0xff_ffff, 255);
        assert!(fb.pixels().iter().enumerate().all(|(i, &p)| i % 24 < 12 || p & 0xff == 0));
        assert!(fb.pixels().iter().any(|&p| p & 0xff > 0));
    }

    #[test]
    fn every_icon_parses() {
        for icon in lucide::ALL {
            assert!(icon.0.iter().all(|d| !flatten(d, 4).is_empty()), "{icon:?}");
        }
    }
}
