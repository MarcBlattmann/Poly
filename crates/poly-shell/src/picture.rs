//! Pictures for the wallpaper (ADR 0127): Alpine Shell's bundled
//! wallpapers, decoded from JPEG on the host when the desktop's image is
//! built (`cargo xtask`, `tools/xtask/src/wallpapers.rs`) and stored in a
//! plain format the shell reads without a decoder: `PWAL`, the width and
//! height (u32, little-endian), then rows of 8-bit RGB. Drawn to cover the
//! screen (scaled to fill it, the overflow cut evenly), sampled bilinearly.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use poly_wayland::compose::{Framebuffer, Rect};

/// The file's first four bytes.
pub const MAGIC: &[u8; 4] = b"PWAL";

/// A picture: `0xRRGGBB` pixels, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// Wallpaper pictures by preset ID.
pub type Pictures = BTreeMap<String, Picture>;

/// The file for a `width` × `height` picture of RGB bytes.
pub fn encode(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + rgb.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(rgb);
    out
}

/// Read a picture file; `None` if it is not one (or is cut short).
pub fn parse(data: &[u8]) -> Option<Picture> {
    if data.get(..4)? != MAGIC {
        return None;
    }
    let w = u32::from_le_bytes(data.get(4..8)?.try_into().ok()?);
    let h = u32::from_le_bytes(data.get(8..12)?.try_into().ok()?);
    if w == 0 || h == 0 || w > 16_384 || h > 16_384 {
        return None;
    }
    let rgb = data.get(12..12 + (w as usize) * (h as usize) * 3)?;
    let pixels =
        rgb.chunks_exact(3).map(|p| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2])).collect();
    Some(Picture { width: w, height: h, pixels })
}

impl Picture {
    /// Draw it covering a `w` × `h` screen, the part inside `clip`.
    pub fn draw_cover(&self, fb: &mut dyn Framebuffer, clip: Rect, w: i32, h: i32) {
        let (pw, ph) = (self.width as f32, self.height as f32);
        // Scale so the picture fills the screen; centre the overflow.
        let k = (w as f32 / pw).max(h as f32 / ph);
        let (ox, oy) = ((pw * k - w as f32) / 2.0, (ph * k - h as f32) / 2.0);
        let area = clip.intersect(&Rect::new(0, 0, w.min(fb.width() as i32), h.min(fb.height() as i32)));
        let px = |x: i32, y: i32| {
            let x = x.clamp(0, self.width as i32 - 1) as usize;
            let y = y.clamp(0, self.height as i32 - 1) as usize;
            self.pixels[y * self.width as usize + x]
        };
        for y in area.y..area.bottom() {
            let sy = (y as f32 + 0.5 + oy) / k - 0.5;
            let y0 = poly_wayland::compose::paint::floor(sy);
            let fy = sy - y0;
            let row = fb.row_mut(y as u32);
            for x in area.x..area.right() {
                let sx = (x as f32 + 0.5 + ox) / k - 0.5;
                let x0 = poly_wayland::compose::paint::floor(sx);
                let fx = sx - x0;
                let (xi, yi) = (x0 as i32, y0 as i32);
                let (a, b, c, d) = (px(xi, yi), px(xi + 1, yi), px(xi, yi + 1), px(xi + 1, yi + 1));
                let ch = |s: u32| {
                    let g = |p: u32| ((p >> s) & 0xff) as f32;
                    let top = g(a) + (g(b) - g(a)) * fx;
                    let bottom = g(c) + (g(d) - g(c)) * fx;
                    ((top + (bottom - top) * fy + 0.5) as u32).min(255) << s
                };
                row[x as usize] = 0xff00_0000 | ch(16) | ch(8) | ch(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::compose::MemoryFramebuffer;

    #[test]
    fn files_round_trip_and_bad_ones_are_refused() {
        let rgb = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let p = parse(&encode(2, 2, &rgb)).unwrap();
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(p.pixels, [0xff_0000, 0x00_ff00, 0x00_00ff, 0xff_ffff]);
        assert!(parse(b"PNG!xxxxxxxx").is_none());
        assert!(parse(&encode(2, 2, &rgb[..9])).is_none(), "cut short");
        assert!(parse(&encode(0, 2, &[])).is_none());
    }

    #[test]
    fn a_picture_covers_the_screen() {
        // A 4 × 2 picture, left half black, right half white, on a square
        // screen: it is scaled to the height and its middle shows.
        let px = [0, 0, 0xff_ffff, 0xff_ffff, 0, 0, 0xff_ffff, 0xff_ffff];
        let p = Picture { width: 4, height: 2, pixels: px.to_vec() };
        let mut fb = MemoryFramebuffer::new(20, 20);
        p.draw_cover(&mut fb, Rect::new(0, 0, 20, 20), 20, 20);
        assert_eq!(fb.pixel(1, 10).unwrap() & 0xff, 0, "the left is black");
        assert_eq!(fb.pixel(18, 10).unwrap() & 0xff, 255, "the right is white");
        let mid = fb.pixel(10, 10).unwrap() & 0xff;
        assert!(mid > 60 && mid < 200, "bilinear across the middle: {mid}");
        // In pieces, the same.
        let mut parts = MemoryFramebuffer::new(20, 20);
        p.draw_cover(&mut parts, Rect::new(0, 0, 20, 7), 20, 20);
        p.draw_cover(&mut parts, Rect::new(0, 7, 20, 13), 20, 20);
        assert_eq!(parts.pixels(), fb.pixels());
    }
}
