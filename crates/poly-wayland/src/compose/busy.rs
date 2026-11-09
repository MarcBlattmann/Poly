//! The busy pointer's spinner (ADR 0167): the arrow with a small ring at
//! its lower right, a quarter of the ring a bright arc that fades to its
//! tail and turns clockwise, [`BUSY_PHASES`] pictures to a turn.
//!
//! Original artwork, drawn from distances (no image files): a ring of
//! [`RING_RADIUS`] and [`RING_WIDTH`] logical pixels centred at
//! ([`RING_CENTRE`]) from the hot spot, anti-aliased to one device pixel,
//! with a one-pixel halo of the opposite tone so it reads on any
//! background. In Alpine Shell's dark theme the arc is its ink (`#ebebe7`)
//! over a faint white track with a dark halo; in the light theme the arc
//! is its dark ground (`#202020`) over a faint dark track with a white
//! halo. The arrow is the usual one (white with a black outline) in both.

use super::paint::{mix, sqrt};
use super::scene::{BUSY_PHASES, BUSY_SIZE};
use super::{Framebuffer, Rect};

/// The ring's centre, logical pixels from the hot spot.
pub const RING_CENTRE: (f32, f32) = (18.0, 18.5);
/// The ring's middle radius and thickness, logical pixels.
pub const RING_RADIUS: f32 = 5.0;
pub const RING_WIDTH: f32 = 2.5;
/// The halo around the ring, logical pixels.
const HALO: f32 = 1.0;

/// The arc's half angle (67.5 degrees, a sweep of 135): its cosine and sine.
const HALF: (f32, f32) = (0.382_683_4, 0.923_879_5);
/// How much of the arc's opacity its tail keeps.
const TAIL: f32 = 0.35;

/// Cosine and sine of 45 degrees (both).
const D45: f32 = core::f32::consts::FRAC_1_SQRT_2;

/// (cos, sin) of `k * 22.5` degrees, `k` = 0..16; the angle grows clockwise
/// on the screen, whose y axis points down.
const DIRECTIONS: [(f32, f32); 16] = [
    (1.0, 0.0),
    (0.923_879_5, 0.382_683_4),
    (D45, D45),
    (0.382_683_4, 0.923_879_5),
    (0.0, 1.0),
    (-0.382_683_4, 0.923_879_5),
    (-D45, D45),
    (-0.923_879_5, 0.382_683_4),
    (-1.0, 0.0),
    (-0.923_879_5, -0.382_683_4),
    (-D45, -D45),
    (-0.382_683_4, -0.923_879_5),
    (0.0, -1.0),
    (0.382_683_4, -0.923_879_5),
    (D45, -D45),
    (0.923_879_5, -0.382_683_4),
];

/// A colour with its opacity (0-255).
type Ink = (u32, u32);

/// The spinner's colours in a dark or light desktop: (halo, track, arc).
fn inks(dark: bool) -> (Ink, Ink, u32) {
    if dark {
        ((0x00_00_00, 110), (0xff_ff_ff, 64), 0xeb_eb_e7)
    } else {
        ((0xff_ff_ff, 150), (0x00_00_00, 56), 0x20_20_20)
    }
}

/// Coverage of a band of distances `lo..hi` at distance `d`, anti-aliased
/// over `aa`.
fn band(d: f32, lo: f32, hi: f32, aa: f32) -> f32 {
    ((d - lo).min(hi - d) / aa + 0.5).clamp(0.0, 1.0)
}

fn weight(cov: f32, alpha: u32) -> u32 {
    (cov.clamp(0.0, 1.0) * alpha as f32 + 0.5) as u32
}

/// `dst` (opaque) with the spinner's pixel at (`dx`, `dy`) logical pixels
/// from the hot spot drawn over it. `aa` is a device pixel in logical ones.
pub fn paint_pixel(dst: u32, dx: f32, dy: f32, phase: u8, dark: bool, aa: f32) -> u32 {
    let (px, py) = (dx - RING_CENTRE.0, dy - RING_CENTRE.1);
    let d = sqrt(px * px + py * py);
    let (inner, outer) = (RING_RADIUS - RING_WIDTH / 2.0, RING_RADIUS + RING_WIDTH / 2.0);
    if d > outer + HALO + aa || d < inner - HALO - aa {
        return dst;
    }
    let (halo, track, arc) = inks(dark);
    let mut out = dst;
    out = mix(out, halo.0, weight(band(d, inner - HALO, outer + HALO, aa), halo.1));
    let ring = band(d, inner, outer, aa);
    out = mix(out, track.0, weight(ring, track.1));
    if d > 0.0 {
        let (ux, uy) = (px / d, py / d);
        let (cx, cy) = DIRECTIONS[(usize::from(phase) + 12) % usize::from(BUSY_PHASES)];
        let along = ux * cx + uy * cy;
        let across = cx * uy - cy * ux;
        // Across the arc's ends: arc length from the end, in pixels.
        let ends = ((along - HALF.0) * d / HALF.1 / aa + 0.5).clamp(0.0, 1.0);
        let fade = TAIL + (1.0 - TAIL) * ((across + HALF.1) / (2.0 * HALF.1)).clamp(0.0, 1.0);
        out = mix(out, arc, weight(ring * ends * fade, 255));
    }
    out
}

/// Draw the spinner for a pointer with its hot spot at logical (`x`, `y`)
/// on the output of scale `s`, inside `clip` (the arrow is drawn by the
/// caller).
pub fn draw(fb: &mut dyn Framebuffer, clip: Rect, x: i32, y: i32, s: i32, phase: u8, dark: bool) {
    let area = Rect::new(x, y, BUSY_SIZE.0, BUSY_SIZE.1).scale(s).intersect(&clip);
    let (scale, aa) = (s as f32, 1.0 / s as f32);
    for py in area.y..area.bottom() {
        let row = fb.row_mut(py as u32);
        let dy = (py as f32 + 0.5) / scale - y as f32;
        for px in area.x..area.right() {
            let dx = (px as f32 + 0.5) / scale - x as f32;
            let at = &mut row[px as usize];
            *at = paint_pixel(*at, dx, dy, phase, dark, aa);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREY: u32 = 0xff80_8080;

    /// A point on the ring in direction `k` (of 16, from the right).
    fn on_ring(k: usize) -> (f32, f32) {
        let (c, s) = DIRECTIONS[k % 16];
        (RING_CENTRE.0 + c * RING_RADIUS, RING_CENTRE.1 + s * RING_RADIUS)
    }

    fn lum(p: u32) -> u32 {
        ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff)
    }

    #[test]
    fn nothing_is_drawn_away_from_the_ring() {
        for dark in [true, false] {
            // The ring's middle, a corner of the cursor, the hot spot.
            assert_eq!(paint_pixel(GREY, RING_CENTRE.0, RING_CENTRE.1, 0, dark, 0.5), GREY);
            assert_eq!(paint_pixel(GREY, 0.0, 0.0, 0, dark, 0.5), GREY);
            assert_eq!(paint_pixel(GREY, 23.5, 24.5, 3, dark, 0.5), GREY);
        }
    }

    #[test]
    fn the_arc_is_where_the_phase_says_and_the_far_side_is_track() {
        // Phase 0: the arc is centred at the top (direction 12 = up).
        for dark in [true, false] {
            let (x, y) = on_ring(12);
            let head = paint_pixel(GREY, x, y, 0, dark, 0.5);
            let (x, y) = on_ring(4);
            let far = paint_pixel(GREY, x, y, 0, dark, 0.5);
            assert_ne!(far, GREY, "the track shows");
            // Dark: a light arc over grey; light: a dark arc.
            if dark {
                assert!(lum(head) > lum(far) + 100, "{head:#x} {far:#x}");
            } else {
                assert!(lum(head) + 100 < lum(far), "{head:#x} {far:#x}");
            }
        }
    }

    #[test]
    fn the_arc_turns_clockwise_with_the_phase() {
        let up = on_ring(12);
        let right = on_ring(0);
        let at = |p: (f32, f32), phase| paint_pixel(GREY, p.0, p.1, phase, true, 0.5);
        // At phase 0 the arc's centre is up; four phases (a quarter turn)
        // later it is to the right, and up is no longer its middle.
        assert!(lum(at(up, 0)) > lum(at(right, 0)));
        assert!(lum(at(right, 4)) > lum(at(up, 4)));
        assert!(lum(at(up, 4)) < lum(at(up, 0)));
        // A whole turn is the same picture.
        assert_eq!(at(up, 0), paint_pixel(GREY, up.0, up.1, BUSY_PHASES, true, 0.5));
    }

    #[test]
    fn the_arc_fades_toward_its_tail() {
        // Phase 0: centred up, turning clockwise: the head is up-right, the
        // tail up-left.
        let head = on_ring(13);
        let tail = on_ring(11);
        let a = paint_pixel(GREY, head.0, head.1, 0, true, 0.5);
        let b = paint_pixel(GREY, tail.0, tail.1, 0, true, 0.5);
        assert!(lum(a) > lum(b), "{a:#x} {b:#x}");
    }

    #[test]
    fn light_and_dark_differ_and_the_halo_helps_on_any_ground() {
        let (x, y) = on_ring(12);
        for ground in [0xff10_1010, 0xfff0_f0f0] {
            let dark = paint_pixel(ground, x, y, 0, true, 0.5);
            let light = paint_pixel(ground, x, y, 0, false, 0.5);
            assert_ne!(dark, light);
        }
        // The halo sits outside the ring: darkens a light ground in the dark
        // theme, lightens a dark one in the light theme.
        let outside = (RING_CENTRE.0 + RING_RADIUS + RING_WIDTH / 2.0 + 0.5, RING_CENTRE.1);
        let d = paint_pixel(0xfff0_f0f0, outside.0, outside.1, 0, true, 0.5);
        assert!(lum(d) < lum(0xfff0_f0f0));
        let l = paint_pixel(0xff10_1010, outside.0, outside.1, 0, false, 0.5);
        assert!(lum(l) > lum(0xff10_1010));
    }

    #[test]
    fn the_whole_spinner_fits_in_the_cursor_rectangle() {
        let reach = RING_RADIUS + RING_WIDTH / 2.0 + HALO + 0.5;
        assert!(RING_CENTRE.0 + reach <= BUSY_SIZE.0 as f32);
        assert!(RING_CENTRE.1 + reach <= BUSY_SIZE.1 as f32);
        assert!(RING_CENTRE.0 - reach >= 0.0);
    }

    /// With `POLY_BUSY_DUMP=<dir>`: pictures of the pointer in both themes,
    /// four phases each, at scale 4, as PPM files (for looking at).
    #[test]
    fn dump_pictures_when_asked() {
        let Some(dir) = std::env::var_os("POLY_BUSY_DUMP") else { return };
        let s = 4;
        for (name, dark, ground) in [("dark", true, 0xff30_3030u32), ("light", false, 0xffe8_e8e8)] {
            let (w, h) = ((BUSY_SIZE.0 + 2) * s * 4, BUSY_SIZE.1 * s);
            let mut fb = crate::compose::MemoryFramebuffer::new(w as u32, h as u32);
            for y in 0..h as u32 {
                fb.row_mut(y).fill(ground);
            }
            for (i, phase) in [0u8, 4, 8, 12].into_iter().enumerate() {
                let x = i as i32 * (BUSY_SIZE.0 + 2);
                let clip = Rect::new(0, 0, w, h);
                crate::compose::render::draw_cursor(&mut fb, clip, x, 0, s);
                draw(&mut fb, clip, x, 0, s, phase, dark);
            }
            let mut out = std::format!(
                "P6
{w} {h}
255
"
            )
            .into_bytes();
            for y in 0..h as u32 {
                for &p in fb.row_mut(y).iter() {
                    out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
                }
            }
            std::fs::write(std::path::Path::new(&dir).join(std::format!("busy-{name}.ppm")), out).unwrap();
        }
    }
}
