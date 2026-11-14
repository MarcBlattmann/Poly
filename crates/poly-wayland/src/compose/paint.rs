//! Anti-aliased shapes, soft shadows and blur on a [`Framebuffer`], for
//! the window frame and the shell (ADR 0127): the look of the owner's
//! Alpine Shell needs rounded rectangles with exact edges, CSS
//! `box-shadow`s and, where it blurs what is behind, a backdrop blur.
//!
//! - **Shapes** are signed distance functions evaluated at pixel centres;
//!   a pixel's coverage is `0.5 - distance`, clamped: exact for straight
//!   edges and within a few percent on curves, the way browsers' analytic
//!   anti-aliasing looks. [`Radii`] gives each corner its own radius;
//!   [`Chrome`] is Alpine's panel that hangs from the top bar: a body with
//!   rounded bottom corners and concave "shoulders" joining the bar.
//! - **Shadows** follow CSS Backgrounds and Borders Level 3 §7.1: the
//!   shape moved by the offset, grown by the spread, then blurred by a
//!   Gaussian whose standard deviation is half the blur radius. The
//!   Gaussian is three successive box blurs (the usual approximation, within
//!   about 3 % of the true curve), run on a coverage mask of only the
//!   damaged part plus the blur's reach, so a shadow costs a few passes over
//!   the pixels it touches.
//! - **Blur** of what the framebuffer already holds in a rectangle (a
//!   backdrop), with the same three boxes per channel.
//!
//! Coordinates are physical pixels; colours opaque `0xRRGGBB`, drawn with a
//! coverage over what is there (framebuffers hold opaque `xrgb8888`).

use super::{Framebuffer, Rect};
use alloc::vec;
use alloc::vec::Vec;

/// `|x|` (no `std` here).
pub fn fabs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

/// The square root (Newton's method from a bit-level first guess; within
/// a few ULPs for the sizes drawn here). 0 for negative input.
pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 || !x.is_finite() {
        return if x.is_finite() || x < 0.0 { 0.0 } else { x };
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1fbd_1df5);
    for _ in 0..3 {
        y = 0.5 * (y + x / y);
    }
    y
}

/// The largest integer not above `x`.
pub fn floor(x: f32) -> f32 {
    let t = x as i64 as f32;
    if t > x { t - 1.0 } else { t }
}

/// The smallest integer not below `x`.
pub fn ceil(x: f32) -> f32 {
    -floor(-x)
}

/// `src` (opaque `0xRRGGBB`) over the opaque `dst` with coverage `a`
/// (0-255), in sRGB as browsers blend.
pub fn mix(dst: u32, src: u32, a: u32) -> u32 {
    if a >= 255 {
        return src | 0xff00_0000;
    }
    if a == 0 {
        return dst | 0xff00_0000;
    }
    let ch = |s: u32| {
        let (sv, dv) = ((src >> s) & 0xff, (dst >> s) & 0xff);
        ((sv * a + dv * (255 - a) + 127) / 255) << s
    };
    0xff00_0000 | ch(16) | ch(8) | ch(0)
}

/// A coverage (0.0 to 1.0) times an alpha (0-255) as a blend weight.
fn weight(cov: f32, alpha: u8) -> u32 {
    let c = cov.clamp(0.0, 1.0);
    (c * f32::from(alpha) + 0.5) as u32
}

/// Corner radii in pixels: top left, top right, bottom right, bottom left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Radii {
    pub const fn all(r: f32) -> Radii {
        Radii { top_left: r, top_right: r, bottom_right: r, bottom_left: r }
    }

    pub const fn top(r: f32) -> Radii {
        Radii { top_left: r, top_right: r, bottom_right: 0.0, bottom_left: 0.0 }
    }

    pub const fn bottom(r: f32) -> Radii {
        Radii { top_left: 0.0, top_right: 0.0, bottom_right: r, bottom_left: r }
    }

    /// Each radius grown by `d` (shrunk if negative), never below 0.
    pub fn grow(self, d: f32) -> Radii {
        let g = |r: f32| if r > 0.0 { (r + d).max(0.0) } else { r };
        Radii {
            top_left: g(self.top_left),
            top_right: g(self.top_right),
            bottom_right: g(self.bottom_right),
            bottom_left: g(self.bottom_left),
        }
    }

    fn max(&self) -> f32 {
        self.top_left.max(self.top_right).max(self.bottom_right).max(self.bottom_left)
    }
}

/// A rectangle in fractional pixels (edges, not pixel centres).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Box2 {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Box2 {
    pub fn from_rect(r: Rect) -> Box2 {
        Box2 { x0: r.x as f32, y0: r.y as f32, x1: r.right() as f32, y1: r.bottom() as f32 }
    }

    pub fn translate(self, dx: f32, dy: f32) -> Box2 {
        Box2 { x0: self.x0 + dx, y0: self.y0 + dy, x1: self.x1 + dx, y1: self.y1 + dy }
    }

    pub fn grow(self, d: f32) -> Box2 {
        Box2 { x0: self.x0 - d, y0: self.y0 - d, x1: self.x1 + d, y1: self.y1 + d }
    }

    /// The whole pixels it touches.
    pub fn pixels(&self) -> Rect {
        let (x0, y0) = (floor(self.x0) as i32, floor(self.y0) as i32);
        Rect::new(x0, y0, ceil(self.x1) as i32 - x0, ceil(self.y1) as i32 - y0)
    }
}

/// The signed distance from (`px`, `py`) to the edge of `b` with corners
/// `radii`: negative inside.
pub fn rounded_distance(b: &Box2, radii: Radii, px: f32, py: f32) -> f32 {
    let (cx, cy) = ((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5);
    let (hw, hh) = (((b.x1 - b.x0) * 0.5).max(0.0), ((b.y1 - b.y0) * 0.5).max(0.0));
    let r = match (px < cx, py < cy) {
        (true, true) => radii.top_left,
        (false, true) => radii.top_right,
        (false, false) => radii.bottom_right,
        (true, false) => radii.bottom_left,
    };
    let r = r.min(hw).min(hh).max(0.0);
    let qx = fabs(px - cx) - (hw - r);
    let qy = fabs(py - cy) - (hh - r);
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    sqrt(ox * ox + oy * oy) + qx.max(qy).min(0.0) - r
}

/// How much of the pixel at (`x`, `y`) the rounded rectangle covers.
pub fn rounded_coverage(b: &Box2, radii: Radii, x: i32, y: i32) -> f32 {
    (0.5 - rounded_distance(b, radii, x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0)
}

/// Draw coverage `cov(x, y)` of `rgb` at `alpha` over the pixels of `area`
/// inside `clip`; `solid` is a part known to be fully covered (filled
/// without evaluating `cov`).
fn paint(
    fb: &mut dyn Framebuffer,
    clip: Rect,
    area: Rect,
    solid: Rect,
    rgb: u32,
    alpha: u8,
    cov: impl Fn(i32, i32) -> f32,
) {
    let area = area.intersect(&clip).intersect(&Rect::new(0, 0, fb.width() as i32, fb.height() as i32));
    if area.is_empty() || alpha == 0 {
        return;
    }
    for y in area.y..area.bottom() {
        let row = fb.row_mut(y as u32);
        let inner = y >= solid.y && y < solid.bottom();
        for x in area.x..area.right() {
            let w =
                if inner && x >= solid.x && x < solid.right() { u32::from(alpha) } else { weight(cov(x, y), alpha) };
            if w > 0 {
                let p = &mut row[x as usize];
                *p = mix(*p, rgb, w);
            }
        }
    }
}

/// Fill the rectangle `r` with corners `radii`, anti-aliased.
pub fn fill_rounded(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, radii: Radii, rgb: u32, alpha: u8) {
    let b = Box2::from_rect(r);
    let m = ceil(radii.max()) as i32;
    // Between the corners every pixel is covered: the rows across the
    // whole width, the columns between the corners in the corner rows.
    let cross_h = Rect::new(r.x + m, r.y, r.width - 2 * m, r.height);
    let cross_v = Rect::new(r.x, r.y + m, r.width, r.height - 2 * m);
    paint(fb, clip, r, cross_v, rgb, alpha, |x, y| {
        if cross_h.contains(x, y) { 1.0 } else { rounded_coverage(&b, radii, x, y) }
    });
}

/// A border `width` pixels wide along the inside of the rounded rectangle
/// `r` (CSS `border`: the inner edge's radii are the outer's less the
/// width).
pub fn stroke_rounded(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, radii: Radii, width: f32, rgb: u32, alpha: u8) {
    let b = Box2::from_rect(r);
    let inner = Box2 { x0: b.x0 + width, y0: b.y0 + width, x1: b.x1 - width, y1: b.y1 - width };
    let m = ceil(radii.max().max(width)) as i32 + 1;
    let hollow = Rect::new(r.x + m, r.y + m, r.width - 2 * m, r.height - 2 * m);
    let inner_radii = radii.grow(-width);
    paint(fb, clip, r, Rect::default(), rgb, alpha, |x, y| {
        if hollow.contains(x, y) {
            return 0.0;
        }
        let outer = rounded_coverage(&b, radii, x, y);
        let hole =
            if inner.x1 > inner.x0 && inner.y1 > inner.y0 { rounded_coverage(&inner, inner_radii, x, y) } else { 0.0 };
        (outer - hole).max(0.0)
    });
}

/// A disc (`ring` `None`) or a ring that many pixels wide, centred at
/// (`cx`, `cy`) (fractional pixel coordinates; a pixel's centre is +0.5).
#[allow(clippy::too_many_arguments)]
pub fn circle(fb: &mut dyn Framebuffer, clip: Rect, cx: f32, cy: f32, r: f32, ring: Option<f32>, rgb: u32, alpha: u8) {
    let area = Box2 { x0: cx - r - 1.0, y0: cy - r - 1.0, x1: cx + r + 1.0, y1: cy + r + 1.0 }.pixels();
    paint(fb, clip, area, Rect::default(), rgb, alpha, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let d = sqrt(dx * dx + dy * dy);
        let disc = (r + 0.5 - d).clamp(0.0, 1.0);
        match ring {
            None => disc,
            Some(w) => (disc - (r - w + 0.5 - d).clamp(0.0, 1.0)).max(0.0),
        }
    });
}

/// Alpine Shell's surface that hangs from the top bar (its
/// `chromeShape.ts`): a body from the bar's bottom edge down, its bottom
/// corners rounded, and on each side that is not against the screen's
/// edge a concave shoulder: a quarter circle that runs from the bar's
/// edge, `shoulder` pixels out, into the body's side.
///
/// ```text
///   ──────────╮                  ╭──────────  ← the bar's bottom edge
///      shoulder╲                ╱
///              │     body       │
///              ╰────────────────╯  ← radius
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chrome {
    /// The body: its top is the bar's bottom edge.
    pub body: Box2,
    pub radius: f32,
    pub shoulder: f32,
    pub left_shoulder: bool,
    pub right_shoulder: bool,
}

impl Chrome {
    /// Everything it covers, shoulders included.
    pub fn bounds(&self) -> Rect {
        let l = if self.left_shoulder { self.shoulder } else { 0.0 };
        let r = if self.right_shoulder { self.shoulder } else { 0.0 };
        Box2 { x0: self.body.x0 - l, x1: self.body.x1 + r, ..self.body }.pixels()
    }

    /// The radii actually used (Alpine limits both to what fits).
    fn sizes(&self) -> (f32, f32) {
        let h = self.body.y1 - self.body.y0;
        let r = self.radius.min((self.body.x1 - self.body.x0) * 0.5).min(h * 0.5).max(0.0);
        let s = self.shoulder.min((h - r).max(0.0));
        (r, s)
    }

    /// How much of pixel (`x`, `y`) it covers.
    pub fn coverage(&self, x: i32, y: i32) -> f32 {
        let (r, s) = self.sizes();
        let b = &self.body;
        let body = rounded_coverage(b, Radii::bottom(r), x, y);
        if body >= 1.0 || s <= 0.0 {
            return body;
        }
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        // A shoulder covers the square between the bar and the body's side
        // outside the circle of radius s around its outer lower corner.
        let shoulder = |cx: f32, x_lo: f32, x_hi: f32| {
            let (dx, dy) = (px - cx, py - (b.y0 + s));
            let outside = (sqrt(dx * dx + dy * dy) - s + 0.5).clamp(0.0, 1.0);
            let in_x = (px - x_lo + 0.5).clamp(0.0, 1.0).min((x_hi - px + 0.5).clamp(0.0, 1.0));
            let in_y = (py - b.y0 + 0.5).clamp(0.0, 1.0).min((b.y0 + s - py + 0.5).clamp(0.0, 1.0));
            outside.min(in_x).min(in_y)
        };
        let mut cov = body;
        if self.left_shoulder && px < b.x0 + 0.5 {
            cov = cov.max(shoulder(b.x0 - s, b.x0 - s, b.x0));
        }
        if self.right_shoulder && px > b.x1 - 0.5 {
            cov = cov.max(shoulder(b.x1 + s, b.x1, b.x1 + s));
        }
        cov
    }

    /// The shape grown outwards by `d` (the body wider and deeper, the
    /// corners rounder, the concave shoulders tighter; their tips stay on
    /// the bar's edge where they were).
    pub fn grown(&self, d: f32) -> Chrome {
        let (r, s) = self.sizes();
        let b = self.body;
        Chrome {
            body: Box2 { x0: b.x0 - d, x1: b.x1 + d, y0: b.y0, y1: b.y1 + d },
            radius: r + d,
            shoulder: (s - d).max(0.0),
            ..*self
        }
    }

    /// Alpine's 1-pixel line around it and along the bar's bottom edge
    /// (`barBorderPath`): `width` pixels outside the shape, then the bar's
    /// edge to the screen's sides (`screen_w` wide), drawn within `clip`.
    pub fn outline(&self, fb: &mut dyn Framebuffer, clip: Rect, screen_w: i32, width: i32, rgb: u32, alpha: u8) {
        let outer = self.grown(width as f32);
        let ob = outer.bounds();
        paint(fb, clip, ob, Rect::default(), rgb, alpha, |x, y| (outer.coverage(x, y) - self.coverage(x, y)).max(0.0));
        let top = ceil(self.body.y0) as i32;
        for band in [Rect::new(0, top, ob.x, width), Rect::new(ob.right(), top, screen_w - ob.right(), width)] {
            paint(fb, clip, band, band, rgb, alpha, |_, _| 1.0);
        }
    }

    /// Fill it with `rgb`.
    pub fn fill(&self, fb: &mut dyn Framebuffer, clip: Rect, rgb: u32, alpha: u8) {
        let (r, _) = self.sizes();
        let b = &self.body;
        // Above the rounded bottom, the body's whole pixels are covered.
        let (x0, x1) = (ceil(b.x0) as i32, floor(b.x1) as i32);
        let (y0, y1) = (ceil(b.y0) as i32, floor(b.y1 - r) as i32);
        let solid = Rect::new(x0, y0, x1 - x0, y1 - y0);
        paint(fb, clip, self.bounds(), solid, rgb, alpha, |x, y| self.coverage(x, y));
    }
}

/// The widths of three box blurs that together approximate a Gaussian of
/// standard deviation `sigma` (W. Jarosz's construction: boxes of two
/// neighbouring odd widths whose variances add up to sigma²). Returned as
/// radii: a box of radius `r` is `2r + 1` wide.
pub fn gauss_boxes(sigma: f32) -> [usize; 3] {
    if sigma < 0.5 {
        return [0, 0, 0];
    }
    let n = 3.0;
    let ideal = sqrt(12.0 * sigma * sigma / n + 1.0);
    let mut wl = floor(ideal) as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let wlf = wl as f32;
    let m = (12.0 * sigma * sigma - n * wlf * wlf - 4.0 * n * wlf - 3.0 * n) / (-4.0 * wlf - 4.0);
    let m = (m + 0.5).max(0.0) as i32;
    let mut out = [0; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let w = if (i as i32) < m { wl } else { wu };
        *o = ((w - 1) / 2) as usize;
    }
    out
}

/// One box blur of radius `r` along a line of `n` values `stride` apart,
/// in place (outside the line counts as 0), through `tmp`.
fn box_line(data: &mut [f32], start: usize, n: usize, stride: usize, r: usize, tmp: &mut [f32]) {
    if r == 0 || n == 0 {
        return;
    }
    let inv = 1.0 / (2 * r + 1) as f32;
    let at = |i: usize| start + i * stride;
    let mut sum = 0.0;
    for i in 0..r.min(n) {
        sum += data[at(i)];
    }
    for (i, t) in tmp.iter_mut().enumerate().take(n) {
        if i + r < n {
            sum += data[at(i + r)];
        }
        *t = sum * inv;
        if i >= r {
            sum -= data[at(i - r)];
        }
    }
    for (i, t) in tmp.iter().enumerate().take(n) {
        data[at(i)] = *t;
    }
}

/// Blur a `w` × `h` grid of values with the three boxes, both ways.
fn blur_grid(data: &mut [f32], w: usize, h: usize, boxes: [usize; 3]) {
    let mut tmp = vec![0.0f32; w.max(h)];
    for &r in &boxes {
        for y in 0..h {
            box_line(data, y * w, w, 1, r, &mut tmp);
        }
    }
    for &r in &boxes {
        for x in 0..w {
            box_line(data, x, h, w, r, &mut tmp);
        }
    }
}

/// A CSS box shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    /// The blur radius (the Gaussian's standard deviation is half of it).
    pub blur: f32,
    pub spread: f32,
    pub rgb: u32,
    /// Opacity, 0-255.
    pub alpha: u8,
}

impl Shadow {
    /// `x y blur spread rgba(r, g, b, a)` with `a` as 0.0 to 1.0.
    pub const fn new(dx: f32, dy: f32, blur: f32, spread: f32, rgb: u32, alpha: u8) -> Shadow {
        Shadow { dx, dy, blur, spread, rgb, alpha }
    }

    /// The same at output scale `s` (lengths multiplied).
    pub fn scaled(self, s: super::Scale) -> Shadow {
        let f = s.factor();
        Shadow { dx: self.dx * f, dy: self.dy * f, blur: self.blur * f, spread: self.spread * f, ..self }
    }

    /// The pixels a shadow of the box `r` may darken.
    pub fn extent(&self, r: Rect) -> Rect {
        let reach = self.reach() as f32;
        Box2::from_rect(r).translate(self.dx, self.dy).grow(self.spread + reach).pixels()
    }

    /// How far beyond the shape the blur reaches, in pixels.
    fn reach(&self) -> i32 {
        let b = gauss_boxes(self.blur * 0.5);
        (b[0] + b[1] + b[2]) as i32 + 1
    }

    /// Draw the shadow of the box `r` with corners `radii` (inside `clip`),
    /// except over `occluder` (a part something opaque will cover anyway).
    pub fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, r: Rect, radii: Radii, occluder: Rect) {
        let screen = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
        let region = self.extent(r).intersect(&clip).intersect(&screen);
        if region.is_empty() || self.alpha == 0 || occluder_covers(occluder, region) {
            return;
        }
        let weights = self.weights(r, radii, region);
        self.blend(fb, region, region, &weights, occluder);
    }

    /// [`Shadow::draw`] with the blurred shadow of a box this size kept in
    /// `cache`: a window that moves, or is redrawn in pieces, blurs its
    /// shadow once instead of on every frame. The same picture as `draw`
    /// (a shadow does not depend on where its box is, only on its size).
    pub fn draw_cached(
        &self,
        cache: &mut ShadowCache,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        r: Rect,
        radii: Radii,
        occluder: Rect,
    ) {
        let screen = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
        let region = self.extent(r).intersect(&clip).intersect(&screen);
        if region.is_empty() || self.alpha == 0 || occluder_covers(occluder, region) {
            return;
        }
        let at_origin = Rect::new(0, 0, r.width, r.height);
        let extent = self.extent(at_origin);
        if i64::from(extent.width) * i64::from(extent.height) > ShadowCache::MAX_PIXELS {
            // Too big to keep: blur just the part wanted.
            let weights = self.weights(r, radii, region);
            self.blend(fb, region, region, &weights, occluder);
            return;
        }
        let key = ShadowKey::new(self, r, radii);
        let weights = cache.get_or_make(key, || self.weights(at_origin, radii, extent));
        // The cached weights cover `extent` around a box at the origin.
        let shifted = extent.translate(r.x, r.y);
        self.blend(fb, region, shifted, weights, occluder);
    }

    /// The weights (0-255, the shadow's opacity where it is darkest) over
    /// `region` of the shadow of the box `r`, row after row.
    fn weights(&self, r: Rect, radii: Radii, region: Rect) -> Vec<u8> {
        let shape = Box2::from_rect(r).translate(self.dx, self.dy).grow(self.spread);
        let radii = radii.grow(self.spread);
        let boxes = gauss_boxes(self.blur * 0.5);
        let reach = (boxes[0] + boxes[1] + boxes[2]) as i32 + 1;
        // The mask over the region plus the blur's reach on every side, so
        // every pixel of the region is blurred from exact values.
        let work = Rect::new(region.x - reach, region.y - reach, region.width + 2 * reach, region.height + 2 * reach);
        let (w, h) = (work.width as usize, work.height as usize);
        let mut mask = vec![0.0f32; w * h];
        let shape_px = shape.pixels();
        let inside = Rect::new(
            shape_px.x + ceil(radii.max()) as i32 + 1,
            shape_px.y + ceil(radii.max()) as i32 + 1,
            shape_px.width - 2 * (ceil(radii.max()) as i32 + 1),
            shape_px.height - 2 * (ceil(radii.max()) as i32 + 1),
        );
        let lit = shape_px.grow(1).intersect(&work);
        for y in lit.y..lit.bottom() {
            let row = (y - work.y) as usize * w;
            for x in lit.x..lit.right() {
                mask[row + (x - work.x) as usize] =
                    if inside.contains(x, y) { 1.0 } else { rounded_coverage(&shape, radii, x, y) };
            }
        }
        blur_grid(&mut mask, w, h, boxes);
        let mut out = Vec::with_capacity(region.width as usize * region.height as usize);
        for y in region.y..region.bottom() {
            let mrow = (y - work.y) as usize * w + (region.x - work.x) as usize;
            out.extend(mask[mrow..mrow + region.width as usize].iter().map(|&m| weight(m, self.alpha) as u8));
        }
        out
    }

    /// Blend the shadow into the `region` of `fb` from `weights`, which
    /// cover `covered` (rows of its width), except over `occluder`.
    fn blend(&self, fb: &mut dyn Framebuffer, region: Rect, covered: Rect, weights: &[u8], occluder: Rect) {
        for y in region.y..region.bottom() {
            let row = fb.row_mut(y as u32);
            let wrow = (y - covered.y) as usize * covered.width as usize;
            for x in region.x..region.right() {
                if occluder.contains(x, y) {
                    continue;
                }
                let a = u32::from(weights[wrow + (x - covered.x) as usize]);
                if a > 0 {
                    let p = &mut row[x as usize];
                    *p = mix(*p, self.rgb, a);
                }
            }
        }
    }
}

/// What the blurred shadow of a box depends on: the shadow's own numbers,
/// the box's size and its corners (not where it is, not the colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShadowKey {
    shadow: [u32; 4],
    alpha: u8,
    size: (i32, i32),
    radii: [u32; 4],
}

impl ShadowKey {
    fn new(s: &Shadow, r: Rect, radii: Radii) -> ShadowKey {
        ShadowKey {
            shadow: [s.dx.to_bits(), s.dy.to_bits(), s.blur.to_bits(), s.spread.to_bits()],
            alpha: s.alpha,
            size: (r.width, r.height),
            radii: [
                radii.top_left.to_bits(),
                radii.top_right.to_bits(),
                radii.bottom_right.to_bits(),
                radii.bottom_left.to_bits(),
            ],
        }
    }
}

/// Blurred shadows kept between frames (see [`Shadow::draw_cached`]): the
/// few most recently used sizes, so a window that is dragged, and the
/// others on the screen, find theirs.
#[derive(Debug, Default)]
pub struct ShadowCache {
    /// Least recently used first.
    entries: Vec<(ShadowKey, Vec<u8>)>,
    /// Shadows blurred (misses), for tests and statistics.
    pub made: u64,
}

impl ShadowCache {
    /// The most shadows kept.
    const ENTRIES: usize = 6;
    /// The largest shadow kept, in pixels (4096 × 4096).
    const MAX_PIXELS: i64 = 1 << 24;

    pub fn new() -> ShadowCache {
        ShadowCache::default()
    }

    /// How many shadows are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The weights for `key`, made by `make` if they are not kept, and now
    /// the most recently used.
    fn get_or_make(&mut self, key: ShadowKey, make: impl FnOnce() -> Vec<u8>) -> &[u8] {
        let entry = match self.entries.iter().position(|(k, _)| *k == key) {
            Some(i) => self.entries.remove(i),
            None => {
                if self.entries.len() >= Self::ENTRIES {
                    self.entries.remove(0);
                }
                self.made += 1;
                (key, make())
            }
        };
        self.entries.push(entry);
        &self.entries[self.entries.len() - 1].1
    }
}

fn occluder_covers(o: Rect, r: Rect) -> bool {
    !o.is_empty() && o.intersect(&r) == r
}

impl Rect {
    /// Grown by `d` on every side.
    pub fn grow(&self, d: i32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.width + 2 * d, self.height + 2 * d)
    }
}

/// Blur what the framebuffer holds in `r` (the part inside `clip`) with a
/// Gaussian of standard deviation `sigma`, reading up to the blur's reach
/// around it (CSS `backdrop-filter: blur(radius)` uses the radius as the
/// standard deviation).
pub fn blur_backdrop(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, sigma: f32) {
    let screen = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
    let region = r.intersect(&clip).intersect(&screen);
    let boxes = gauss_boxes(sigma);
    if region.is_empty() || boxes == [0, 0, 0] {
        return;
    }
    let reach = (boxes[0] + boxes[1] + boxes[2]) as i32 + 1;
    let work = region.grow(reach).intersect(&screen);
    let (w, h) = (work.width as usize, work.height as usize);
    let mut planes = [vec![0.0f32; w * h], vec![0.0f32; w * h], vec![0.0f32; w * h]];
    for y in 0..h {
        let row = fb.row_mut((work.y + y as i32) as u32);
        for x in 0..w {
            let p = row[work.x as usize + x];
            for (c, plane) in planes.iter_mut().enumerate() {
                plane[y * w + x] = ((p >> (16 - 8 * c)) & 0xff) as f32;
            }
        }
    }
    // Edges: the box sums treat outside as 0, so blur a plane of ones too
    // and divide by it (the mean of what is there).
    let mut ones = vec![1.0f32; w * h];
    blur_grid(&mut ones, w, h, boxes);
    for plane in &mut planes {
        blur_grid(plane, w, h, boxes);
    }
    for y in region.y..region.bottom() {
        let row = fb.row_mut(y as u32);
        let my = (y - work.y) as usize * w;
        for x in region.x..region.right() {
            let i = my + (x - work.x) as usize;
            let n = ones[i].max(1e-6);
            let ch = |c: usize| ((planes[c][i] / n + 0.5) as u32).min(255);
            row[x as usize] = 0xff00_0000 | (ch(0) << 16) | (ch(1) << 8) | ch(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::MemoryFramebuffer;

    fn grey(p: Option<u32>) -> u32 {
        p.map_or(0, |p| p & 0xff)
    }

    fn solid(fb: &mut MemoryFramebuffer, r: Rect, rgb: u32) {
        for y in r.y..r.bottom() {
            fb.row_mut(y as u32)[r.x as usize..r.right() as usize].fill(rgb | 0xff00_0000);
        }
    }

    #[test]
    fn maths() {
        for v in [0.0f32, 1.0, 2.0, 9.0, 0.25, 1234.5] {
            let s = sqrt(v);
            assert!(fabs(s * s - v) <= v * 1e-5 + 1e-6, "{v}: {s}");
        }
        assert_eq!(floor(-0.5), -1.0);
        assert_eq!(floor(2.7), 2.0);
        assert_eq!(ceil(2.1), 3.0);
        assert_eq!(fabs(-3.0), 3.0);
    }

    #[test]
    fn rounded_rectangles_are_exact_on_edges_and_smooth_on_corners() {
        let mut fb = MemoryFramebuffer::new(40, 40);
        let all = Rect::new(0, 0, 40, 40);
        fill_rounded(&mut fb, all, Rect::new(4, 4, 32, 32), Radii::all(8.0), 0xff_ffff, 255);
        assert_eq!(grey(fb.pixel(20, 4)), 255, "an edge's middle row is covered");
        assert_eq!(grey(fb.pixel(20, 3)), 0, "nothing above it");
        assert_eq!(grey(fb.pixel(4, 4)), 0, "the corner is cut");
        let partial = (4..12).flat_map(|y| (4..12).map(move |x| (x, y))).filter(|&(x, y)| {
            let g = grey(fb.pixel(x, y));
            g > 0 && g < 255
        });
        assert!(partial.count() >= 6, "the arc is anti-aliased");
        assert_eq!(fb.pixel(5, 9), fb.pixel(34, 30), "symmetric");
        // Only the bottom corners rounded.
        let mut fb = MemoryFramebuffer::new(40, 40);
        fill_rounded(&mut fb, all, Rect::new(0, 0, 40, 40), Radii::bottom(10.0), 0xff_ffff, 255);
        assert_eq!(grey(fb.pixel(0, 0)), 255);
        assert_eq!(grey(fb.pixel(0, 39)), 0);
        // Half a pixel's coverage on a fractional edge.
        let b = Box2 { x0: 0.0, y0: 0.0, x1: 10.5, y1: 10.0 };
        let c = rounded_coverage(&b, Radii::default(), 10, 5);
        assert!((c - 0.5).abs() < 0.01, "{c}");
    }

    #[test]
    fn borders_follow_the_corners() {
        let mut fb = MemoryFramebuffer::new(40, 40);
        let all = Rect::new(0, 0, 40, 40);
        stroke_rounded(&mut fb, all, Rect::new(0, 0, 40, 40), Radii::all(6.0), 1.0, 0xff_ffff, 255);
        assert_eq!(grey(fb.pixel(20, 0)), 255);
        assert_eq!(grey(fb.pixel(20, 1)), 0, "one pixel wide");
        assert_eq!(grey(fb.pixel(20, 20)), 0, "hollow");
        assert!(grey(fb.pixel(1, 1)) > 0 && grey(fb.pixel(1, 1)) < 255, "the corner's arc is soft");
    }

    #[test]
    fn circles_and_rings() {
        let mut fb = MemoryFramebuffer::new(40, 40);
        let all = Rect::new(0, 0, 40, 40);
        circle(&mut fb, all, 20.0, 20.0, 8.0, Some(2.0), 0xff_ffff, 255);
        assert_eq!(grey(fb.pixel(20, 20)), 0, "a ring is empty inside");
        assert!(grey(fb.pixel(20, 12)) > 240);
        assert_eq!(grey(fb.pixel(20, 30)), 0);
    }

    #[test]
    fn chrome_hangs_from_the_bar_with_shoulders() {
        let mut fb = MemoryFramebuffer::new(100, 60);
        let c = Chrome {
            body: Box2 { x0: 30.0, y0: 10.0, x1: 70.0, y1: 50.0 },
            radius: 14.0,
            shoulder: 14.0,
            left_shoulder: true,
            right_shoulder: true,
        };
        assert_eq!(c.bounds(), Rect::new(16, 10, 68, 40));
        c.fill(&mut fb, Rect::new(0, 0, 100, 60), 0xff_ffff, 255);
        // The body's middle and its top edge are covered.
        assert_eq!(grey(fb.pixel(50, 30)), 255);
        assert_eq!(grey(fb.pixel(30, 10)), 255);
        // The shoulder: just outside the body at the bar's edge, covered;
        // further down and out, not (the concave curve).
        assert_eq!(grey(fb.pixel(29, 10)), 255);
        assert_eq!(grey(fb.pixel(17, 22)), 0);
        assert!(grey(fb.pixel(17, 10)) > 0, "the shoulder starts at the bar's edge");
        // The bottom corners are rounded.
        assert_eq!(grey(fb.pixel(30, 49)), 0);
        // Symmetric.
        assert_eq!(fb.pixel(25, 12), fb.pixel(74, 12));
        // Against the left edge: no shoulder on that side.
        let c = Chrome { left_shoulder: false, ..c };
        let mut fb = MemoryFramebuffer::new(100, 60);
        c.fill(&mut fb, Rect::new(0, 0, 100, 60), 0xff_ffff, 255);
        assert_eq!(grey(fb.pixel(29, 10)), 0);
        assert_eq!(c.bounds().x, 30);
    }

    #[test]
    fn three_boxes_approximate_the_gaussian() {
        for sigma in [1.0f32, 2.5, 5.0, 12.0] {
            let b = gauss_boxes(sigma);
            // The variance of a box of width w is (w² - 1) / 12.
            let var: f32 = b.iter().map(|&r| (((2 * r + 1) * (2 * r + 1)) as f32 - 1.0) / 12.0).sum();
            assert!(fabs(sqrt(var) - sigma) < 0.8, "sigma {sigma}: boxes {b:?} give {}", sqrt(var));
        }
        assert_eq!(gauss_boxes(0.2), [0, 0, 0]);
    }

    #[test]
    fn a_shadow_is_soft_offset_and_centred_like_css() {
        // `0 10px 15px -3px rgba(0, 0, 0, 0.5)` (Alpine's floating shadow)
        // under a 100 × 60 box on white.
        let mut fb = MemoryFramebuffer::new(200, 160);
        let all = Rect::new(0, 0, 200, 160);
        solid(&mut fb, all, 0xff_ffff);
        let r = Rect::new(50, 40, 100, 60);
        let s = Shadow::new(0.0, 10.0, 15.0, -3.0, 0, 128);
        s.draw(&mut fb, all, r, Radii::all(8.0), Rect::default());
        // Darkest under the box, about half way at the shifted, shrunk edge,
        // white far away.
        let under = grey(fb.pixel(100, 80));
        assert!(under < 140 && under > 120, "{under}");
        let edge = grey(fb.pixel(100, 40 + 10 + 60 - 3));
        assert!(edge > 160 && edge < 220, "{edge}");
        assert_eq!(grey(fb.pixel(100, 150)), 255);
        assert_eq!(grey(fb.pixel(5, 5)), 255);
        // Left and right are mirror images (the blur is centred).
        assert_eq!(fb.pixel(45, 80), fb.pixel(154, 80));
        // The extent covers everything it darkened.
        let e = s.extent(r);
        for y in 0..160 {
            for x in 0..200 {
                if !e.contains(x, y) {
                    assert_eq!(grey(fb.pixel(x as u32, y as u32)), 255, "({x}, {y}) outside the extent");
                }
            }
        }
    }

    #[test]
    fn a_shadow_drawn_in_pieces_equals_one_drawn_whole() {
        let all = Rect::new(0, 0, 120, 100);
        let r = Rect::new(30, 20, 60, 40);
        let s = Shadow::new(0.0, 8.0, 24.0, 0.0, 0, 90);
        let mut whole = MemoryFramebuffer::new(120, 100);
        solid(&mut whole, all, 0x80_80_80);
        s.draw(&mut whole, all, r, Radii::all(8.0), Rect::default());
        let mut pieces = MemoryFramebuffer::new(120, 100);
        solid(&mut pieces, all, 0x80_80_80);
        for clip in [Rect::new(0, 0, 60, 50), Rect::new(60, 0, 60, 50), Rect::new(0, 50, 120, 50)] {
            s.draw(&mut pieces, clip, r, Radii::all(8.0), Rect::default());
        }
        assert_eq!(whole.pixels(), pieces.pixels());
        // An occluder leaves its pixels alone.
        let mut o = MemoryFramebuffer::new(120, 100);
        solid(&mut o, all, 0x80_80_80);
        s.draw(&mut o, all, r, Radii::all(8.0), r);
        assert_eq!(o.pixel(60, 40), Some(0x80_80_80));
        assert_ne!(o.pixel(60, 70), Some(0x80_80_80));
    }

    /// The largest difference between two framebuffers' channels.
    fn max_difference(a: &MemoryFramebuffer, b: &MemoryFramebuffer) -> u32 {
        a.pixels()
            .iter()
            .zip(b.pixels())
            .flat_map(|(&p, &q)| (0..3).map(move |c| ((p >> (8 * c)) & 0xff).abs_diff((q >> (8 * c)) & 0xff)))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn a_cached_shadow_is_the_same_picture_wherever_its_box_is() {
        let all = Rect::new(0, 0, 220, 160);
        let s = Shadow::new(0.0, 8.0, 24.0, 0.0, 0, 90);
        let radii = Radii::top(8.0);
        let mut cache = ShadowCache::new();
        // The same box size at several places, whole and in pieces (and
        // cut off by the screen's edge).
        for (x, y) in [(30, 20), (31, 22), (-10, 15), (150, 100), (30, 20)] {
            let r = Rect::new(x, y, 60, 40);
            let mut plain = MemoryFramebuffer::new(220, 160);
            solid(&mut plain, all, 0x80_80_80);
            s.draw(&mut plain, all, r, radii, Rect::new(x, y + 8, 60, 32));
            let mut cached = MemoryFramebuffer::new(220, 160);
            solid(&mut cached, all, 0x80_80_80);
            for clip in [Rect::new(0, 0, 110, 80), Rect::new(110, 0, 110, 80), Rect::new(0, 80, 220, 80)] {
                s.draw_cached(&mut cache, &mut cached, clip, r, radii, Rect::new(x, y + 8, 60, 32));
            }
            assert!(max_difference(&plain, &cached) <= 1, "box at ({x}, {y})");
        }
        assert_eq!(cache.made, 1, "one size, blurred once");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_shadow_cache_keeps_the_most_recently_used_sizes() {
        let all = Rect::new(0, 0, 400, 400);
        let s = Shadow::new(0.0, 4.0, 8.0, 0.0, 0, 90);
        let mut fb = MemoryFramebuffer::new(400, 400);
        let mut cache = ShadowCache::new();
        let mut draw = |cache: &mut ShadowCache, w: i32| {
            s.draw_cached(cache, &mut fb, all, Rect::new(50, 50, w, 30), Radii::all(4.0), Rect::default());
        };
        for w in 40..46 {
            draw(&mut cache, w);
        }
        assert_eq!((cache.made, cache.len()), (6, 6));
        draw(&mut cache, 40); // a hit: now the most recently used
        assert_eq!(cache.made, 6);
        draw(&mut cache, 99); // a new size: the least recently used (41) goes
        assert_eq!((cache.made, cache.len()), (7, 6));
        draw(&mut cache, 40);
        assert_eq!(cache.made, 7, "40 was kept");
        draw(&mut cache, 41);
        assert_eq!(cache.made, 8, "41 was dropped");
        // Another opacity or corner is another shadow.
        let lighter = Shadow::new(0.0, 4.0, 8.0, 0.0, 0, 40);
        lighter.draw_cached(&mut cache, &mut fb, all, Rect::new(50, 50, 40, 30), Radii::all(4.0), Rect::default());
        assert_eq!(cache.made, 9);
    }

    #[test]
    fn backdrop_blur_softens_an_edge_inside_its_rectangle_only() {
        let mut fb = MemoryFramebuffer::new(60, 20);
        let all = Rect::new(0, 0, 60, 20);
        solid(&mut fb, Rect::new(0, 0, 30, 20), 0);
        solid(&mut fb, Rect::new(30, 0, 30, 20), 0xff_ffff);
        blur_backdrop(&mut fb, all, Rect::new(10, 0, 40, 20), 4.0);
        let mid = grey(fb.pixel(30, 10));
        assert!(mid > 60 && mid < 200, "{mid}");
        assert_eq!(grey(fb.pixel(5, 10)), 0, "outside the rectangle unchanged");
        assert_eq!(grey(fb.pixel(55, 10)), 255);
        // A flat area stays flat (the edges divide by what was there).
        let mut flat = MemoryFramebuffer::new(30, 30);
        solid(&mut flat, Rect::new(0, 0, 30, 30), 0x40_80_c0);
        blur_backdrop(&mut flat, Rect::new(0, 0, 30, 30), Rect::new(0, 0, 30, 30), 3.0);
        assert!(flat.pixels().iter().all(|&p| p & 0xff_ffff == 0x40_80_c0));
    }
}
