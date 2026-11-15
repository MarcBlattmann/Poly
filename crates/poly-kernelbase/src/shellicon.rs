//! The icons of the shell's system image list (`SHGetFileInfoW` with `SHGFI_SYSICONINDEX` /
//! `SHGFI_ICON`): a generic file, a folder, an application, a library, a drive, a text document
//! and an archive, drawn as flat shapes in the colours of Poly's desktop at the two sizes the
//! shell asks for (16 and 32 pixels). The indices are those of [`crate::fileinfo::icon`].
//!
//! Poly's own artwork (no Windows icon is reproduced): a rectangle language, drawn on a 32 x 32
//! grid and scaled to the size asked for.

use alloc::vec::Vec;

/// A pixel in `0xAARRGGBB` form (the layout of a 32-bit bitmap's bytes read as a little-endian
/// word); alpha 0 is transparent.
pub type Pixel = u32;

const CLEAR: Pixel = 0;
const OUTLINE: Pixel = 0xFF6B7280;
const PAPER: Pixel = 0xFFFDFDFE;
const PAPER_SHADE: Pixel = 0xFFE5E7EB;
const INK: Pixel = 0xFF9CA3AF;
const BLUE: Pixel = 0xFF3B82F6;
const BLUE_DARK: Pixel = 0xFF1D4ED8;
const YELLOW: Pixel = 0xFFFBBF24;
const YELLOW_DARK: Pixel = 0xFFD99A0B;
const YELLOW_LIGHT: Pixel = 0xFFFCD97A;
const GREY: Pixel = 0xFFD1D5DB;
const GREY_DARK: Pixel = 0xFF4B5563;
const GREEN: Pixel = 0xFF22C55E;

struct Canvas {
    size: i32,
    px: Vec<Pixel>,
}

impl Canvas {
    fn new(size: i32) -> Canvas {
        Canvas { size, px: alloc::vec![CLEAR; (size * size) as usize] }
    }

    /// A rectangle of the 32-grid (right and bottom exclusive), at least one pixel in each direction.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Pixel) {
        let s = self.size;
        let a = x0 * s / 32;
        let b = (x1 * s / 32).max(a + 1);
        let u = y0 * s / 32;
        let d = (y1 * s / 32).max(u + 1);
        for y in u.max(0)..d.min(s) {
            for x in a.max(0)..b.min(s) {
                self.px[(y * s + x) as usize] = c;
            }
        }
    }

    /// A rectangle with an outline of one pixel (of the real size) in `edge`.
    fn boxed(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, fill: Pixel, edge: Pixel) {
        self.rect(x0, y0, x1, y1, edge);
        let s = self.size;
        let a = x0 * s / 32 + 1;
        let b = (x1 * s / 32).max(a + 1) - 1;
        let u = y0 * s / 32 + 1;
        let d = (y1 * s / 32).max(u + 1) - 1;
        for y in u.max(0)..d.min(s) {
            for x in a.max(0)..b.min(s) {
                self.px[(y * s + x) as usize] = fill;
            }
        }
    }
}

fn page(c: &mut Canvas) {
    c.boxed(6, 2, 26, 30, PAPER, OUTLINE);
    // The folded corner.
    c.rect(20, 2, 26, 8, PAPER_SHADE);
    c.rect(20, 7, 26, 8, OUTLINE);
    c.rect(20, 2, 21, 8, OUTLINE);
}

fn lines(c: &mut Canvas, color: Pixel, from: i32, count: i32) {
    for i in 0..count {
        c.rect(10, from + i * 4, 22, from + i * 4 + 2, color);
    }
}

/// The pixels of icon `kind` (an index of [`crate::fileinfo::icon`]) at `size` pixels square:
/// `size * size` words, top row first.
pub fn render(kind: i32, size: i32) -> Vec<Pixel> {
    use crate::fileinfo::icon;
    let mut c = Canvas::new(size);
    match kind {
        icon::FOLDER => {
            c.boxed(2, 7, 14, 12, YELLOW_DARK, YELLOW_DARK);
            c.boxed(2, 10, 30, 27, YELLOW, YELLOW_DARK);
            c.rect(3, 11, 29, 14, YELLOW_LIGHT);
        }
        icon::APPLICATION => {
            c.boxed(3, 5, 29, 27, PAPER, OUTLINE);
            c.rect(4, 6, 28, 11, BLUE);
            c.rect(6, 7, 9, 10, PAPER);
            c.rect(5, 14, 27, 25, PAPER_SHADE);
            c.rect(7, 16, 20, 18, INK);
            c.rect(7, 20, 15, 22, INK);
        }
        icon::LIBRARY => {
            page(&mut c);
            c.boxed(10, 12, 22, 24, BLUE, BLUE_DARK);
            c.rect(14, 15, 18, 21, PAPER);
        }
        icon::DRIVE => {
            c.boxed(2, 11, 30, 25, GREY, GREY_DARK);
            c.rect(4, 13, 28, 15, PAPER_SHADE);
            c.rect(6, 20, 20, 22, GREY_DARK);
            c.rect(23, 19, 26, 22, GREEN);
        }
        icon::TEXT => {
            page(&mut c);
            lines(&mut c, GREY_DARK, 12, 4);
        }
        icon::ARCHIVE => {
            c.boxed(5, 4, 27, 29, YELLOW, YELLOW_DARK);
            c.rect(14, 4, 18, 29, PAPER);
            for y in (5..28).step_by(4) {
                c.rect(14, y, 18, y + 2, GREY_DARK);
            }
        }
        _ => {
            page(&mut c);
            lines(&mut c, INK, 12, 4);
        }
    }
    c.px
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fileinfo::icon;

    #[test]
    fn every_icon_is_drawn_at_both_sizes_with_clear_corners() {
        for kind in 0..icon::COUNT {
            for size in [16, 32] {
                let px = render(kind, size);
                assert_eq!(px.len(), (size * size) as usize);
                assert_eq!(px[0] >> 24, 0, "kind {kind} size {size}: the corner is clear");
                assert!(px.iter().any(|&p| p >> 24 == 0xFF), "kind {kind} size {size}: something is drawn");
            }
        }
    }

    #[test]
    fn the_kinds_look_different() {
        for a in 0..icon::COUNT {
            for b in a + 1..icon::COUNT {
                assert_ne!(render(a, 32), render(b, 32), "{a} and {b}");
            }
        }
    }
}
