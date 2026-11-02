//! The wallpapers, drawn (ADR 0126): Alpine Shell shows the Windows
//! wallpaper, and the two pictures it bundles carry no recorded author or
//! licence, so Poly draws its own. The default, "Valley", is a dark,
//! grainy landscape in Alpine's greys: a pale sky over three ranges of
//! dunes with a bright river winding through the nearest. The others are
//! vertical gradients with a soft light from the upper middle. Both are
//! ordered-dithered so that gentle gradients show no steps.

use crate::icons::sin_cos;
use crate::theme::wallpaper as wt;
use poly_wayland::compose::{Framebuffer, Rect};

/// A 4×4 ordered dither (thresholds 0-15 of 16).
const BAYER: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

fn ch(c: u32, sh: u32) -> i32 {
    ((c >> sh) & 0xff) as i32
}

/// A small integer hash of a pixel, for grain.
fn grain(x: i32, y: i32) -> i32 {
    let mut h = (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xff) as i32 - 128
}

/// Draw wallpaper `id` on a `w` × `h` (physical) screen, inside `clip`.
pub fn draw(fb: &mut dyn Framebuffer, clip: Rect, id: &str, w: i32, h: i32) {
    if id == wt::DEFAULT || !wt::PRESETS.iter().any(|p| p.0 == id) {
        valley(fb, clip, w, h);
    } else {
        gradient(fb, clip, wt::colours(id), w, h);
    }
}

/// A wallpaper drawn at a size, kept: drawing the valley costs a few
/// hundred operations a pixel, copying it one.
#[derive(Debug)]
pub struct Cache {
    id: alloc::string::String,
    w: i32,
    h: i32,
    pixels: alloc::vec::Vec<u32>,
}

impl Cache {
    /// Draw wallpaper `id` inside `clip` from the cache in `cell`, drawing
    /// it whole first if the cache holds another or none.
    pub fn draw(
        cell: &core::cell::RefCell<Option<Cache>>,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        id: &str,
        w: i32,
        h: i32,
    ) {
        let mut c = cell.borrow_mut();
        if !c.as_ref().is_some_and(|c| c.id == id && c.w == w && c.h == h) {
            let mut mem = poly_wayland::compose::MemoryFramebuffer::new(w.max(1) as u32, h.max(1) as u32);
            draw(&mut mem, Rect::new(0, 0, w, h), id, w, h);
            *c = Some(Cache { id: id.into(), w, h, pixels: mem.pixels().to_vec() });
        }
        let Some(c) = c.as_ref() else { return };
        let area = clip.intersect(&Rect::new(0, 0, w.min(fb.width() as i32), h.min(fb.height() as i32)));
        for y in area.y..area.bottom() {
            let src = &c.pixels[(y * w + area.x) as usize..(y * w + area.right()) as usize];
            fb.row_mut(y as u32)[area.x as usize..area.right() as usize].copy_from_slice(src);
        }
    }
}

fn gradient(fb: &mut dyn Framebuffer, clip: Rect, (top, bottom, glow): (u32, u32, u32), w: i32, h: i32) {
    for y in clip.y..clip.bottom() {
        // Channels in 1/256 levels: the vertical gradient...
        let t = (y * 256 / h.max(1)).clamp(0, 256);
        let base = [16, 8, 0].map(|sh| ch(top, sh) * (256 - t) + ch(bottom, sh) * t);
        // ...and the glow: an ellipse around a point a third down, fading
        // out half the screen's width and height away.
        let ny = (y - h / 3) * 1024 / (h / 2).max(1);
        let row = fb.row_mut(y as u32);
        for x in clip.x..clip.right() {
            let nx = (x - w / 2) * 1024 / (w / 2).max(1);
            let g = (1024 - (nx * nx + ny * ny) / 1024).max(0);
            let a = g * g / 1024 * 90 / 256;
            let dither = BAYER[((y & 3) * 4 + (x & 3)) as usize] * 16;
            let c = |i: usize, sh: u32| {
                let v = base[i] + (ch(glow, sh) * 256 - base[i]) * a / 1024;
                (((v + dither) >> 8).clamp(0, 255) as u32) << sh
            };
            row[x as usize] = 0xff00_0000 | c(0, 16) | c(1, 8) | c(2, 0);
        }
    }
}

/// The height (as a share of the screen's, 0-1) of dune range `layer` at
/// horizontal position `u` (0-1).
fn ridge(layer: usize, u: f32) -> f32 {
    const RANGES: [(f32, f32, f32, f32); 3] = [
        // (base, amplitude, frequency, phase)
        (0.46, 0.035, 5.0, 0.6),
        (0.55, 0.05, 3.2, 2.1),
        (0.66, 0.07, 2.1, 4.0),
    ];
    let (base, amp, f, p) = RANGES[layer];
    let tau = 2.0 * core::f32::consts::PI;
    let a = sin_cos(u * f * tau + p).0;
    let b = sin_cos(u * f * 2.3 * tau + p * 1.7).0;
    let c = sin_cos(u * f * 5.1 * tau + p * 0.3).0;
    base - amp * (0.65 * a + 0.25 * b + 0.1 * c)
}

/// The river's centre (share of the width) at depth `v` (0 at the
/// nearest range's top, 1 at the bottom), and its half width.
fn river(v: f32) -> (f32, f32) {
    let s = sin_cos(v * 5.5 + 0.8).0;
    (0.47 + 0.16 * s * (0.4 + 0.6 * v), 0.004 + 0.05 * v * v)
}

fn valley(fb: &mut dyn Framebuffer, clip: Rect, w: i32, h: i32) {
    let (wf, hf) = (w.max(1) as f32, h.max(1) as f32);
    // Each column's ranges, once per call.
    let cols: alloc::vec::Vec<[f32; 3]> = (clip.x..clip.right())
        .map(|x| {
            let u = x as f32 / wf;
            [ridge(0, u) * hf, ridge(1, u) * hf, ridge(2, u) * hf]
        })
        .collect();
    let sky_top = (0x6a, 0x6c, 0x6e);
    let sky_low = (0xcc, 0xcd, 0xcd);
    let layers = [(0x2c, 0x2c, 0x2b), (0x1f, 0x1f, 0x1e), (0x14, 0x14, 0x13)];
    let near_top = ridge(2, 0.5) * hf;
    for y in clip.y..clip.bottom() {
        let yf = y as f32;
        let row = fb.row_mut(y as u32);
        for (i, x) in (clip.x..clip.right()).enumerate() {
            let r = cols[i];
            // Sky: darker at the top, pale at the horizon.
            let t = (yf / (0.5 * hf)).clamp(0.0, 1.0);
            let mut c = [
                sky_top.0 as f32 + (sky_low.0 - sky_top.0) as f32 * t,
                sky_top.1 as f32 + (sky_low.1 - sky_top.1) as f32 * t,
                sky_top.2 as f32 + (sky_low.2 - sky_top.2) as f32 * t,
            ];
            for (k, &(lr, lg, lb)) in layers.iter().enumerate() {
                if yf + 1.0 > r[k] {
                    // Lit along its crest by the sky, darker below.
                    let below = ((yf - r[k]) / (0.06 * hf)).clamp(0.0, 1.0);
                    let lift = (1.0 - below) * (14.0 - 4.0 * k as f32);
                    let dune = [lr as f32 + lift, lg as f32 + lift, lb as f32 + lift];
                    // The crest's pixel is covered by its share below it.
                    let edge = (yf + 1.0 - r[k]).clamp(0.0, 1.0);
                    c = [0, 1, 2].map(|i| c[i] + (dune[i] - c[i]) * edge);
                }
            }
            // The river in the nearest range.
            if yf >= r[2] {
                let v = ((yf - near_top) / (hf - near_top).max(1.0)).clamp(0.0, 1.0);
                let (centre, half) = river(v);
                let d = ((x as f32 / wf - centre).abs() - half) * wf;
                if d < 1.0 {
                    let k = (1.0 - d).clamp(0.0, 1.0);
                    let bright = 150.0 + 70.0 * (1.0 - v);
                    c = [c[0] + (bright - c[0]) * k, c[1] + (bright - c[1]) * k, c[2] + (bright - 2.0 - c[2]) * k];
                }
            }
            let n = grain(x, y) as f32 / 128.0 * 5.0;
            let dither = BAYER[((y & 3) * 4 + (x & 3)) as usize] as f32 / 16.0;
            let q = |v: f32| ((v + n + dither).clamp(0.0, 255.0)) as u32;
            row[x as usize] = 0xff00_0000 | (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::compose::MemoryFramebuffer;

    #[test]
    fn the_valley_has_a_pale_sky_dark_dunes_and_a_river() {
        let mut fb = MemoryFramebuffer::new(320, 200);
        draw(&mut fb, Rect::new(0, 0, 320, 200), "valley", 320, 200);
        let grey = |x: u32, y: u32| fb.pixel(x, y).unwrap() & 0xff;
        assert!(grey(160, 70) > 150, "the sky near the horizon is pale");
        assert!(grey(160, 5) < grey(160, 70), "darker at the top");
        assert!(grey(20, 190) < 60, "the dunes are dark");
        let river = (0..320).map(|x| grey(x, 195)).max().unwrap();
        assert!(river > 120, "a bright river crosses the bottom");
        // Drawn in pieces, the same.
        let mut parts = MemoryFramebuffer::new(320, 200);
        draw(&mut parts, Rect::new(0, 0, 320, 100), "valley", 320, 200);
        draw(&mut parts, Rect::new(0, 100, 320, 100), "valley", 320, 200);
        assert_eq!(parts.pixels(), fb.pixels());
        // From the cache, the same.
        let cell = core::cell::RefCell::new(None);
        let mut cached = MemoryFramebuffer::new(320, 200);
        Cache::draw(&cell, &mut cached, Rect::new(0, 0, 320, 120), "valley", 320, 200);
        Cache::draw(&cell, &mut cached, Rect::new(0, 120, 320, 80), "valley", 320, 200);
        assert_eq!(cached.pixels(), fb.pixels());
    }

    #[test]
    fn the_gradients_are_smooth() {
        let mut fb = MemoryFramebuffer::new(64, 64);
        draw(&mut fb, Rect::new(0, 0, 64, 64), "forest", 64, 64);
        let (t, b) = (fb.pixel(32, 0).unwrap() & 0xff00, fb.pixel(32, 63).unwrap() & 0xff00);
        assert!(t > b, "lighter at the top");
    }
}
