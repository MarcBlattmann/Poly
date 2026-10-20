//! Drawing into a rectangle of pixels in caller memory.

use crate::color::{Color, PixelFormat};
use crate::font::{Font, Glyph};
use crate::geom::Rect;
use core::fmt;

/// Why [`Canvas::new`] refused a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// A row of `width` pixels does not fit in `stride` bytes.
    StrideTooSmall,
    /// The buffer ends before the last pixel of the last row.
    BufferTooSmall,
    /// Width or height above `i32::MAX`, or a size that overflows.
    TooLarge,
}

/// A `width` × `height` rectangle of pixels in a byte buffer: row `y` starts
/// `y * stride` bytes in, and each pixel takes
/// [`PixelFormat::bytes_per_pixel`] bytes. The buffer may be a framebuffer
/// mapping, a back buffer in RAM, or part of either ([`Canvas::region`]).
///
/// Coordinates are `i32` so that shapes may start off-canvas; everything is
/// clipped to the canvas, and nothing outside it (the bytes between the end
/// of a row and the stride included) is ever written.
pub struct Canvas<'a> {
    buf: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    format: PixelFormat,
}

impl fmt::Debug for Canvas<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// Pixel decorations for one glyph: bold (each lit pixel also lights the
/// one to its right), and full-width lines on the given glyph rows.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Decorations {
    pub bold: bool,
    pub underline: Option<u32>,
    pub strike: Option<u32>,
}

impl<'a> Canvas<'a> {
    /// Bytes a buffer needs for this layout: up to the last pixel of the
    /// last row (0 for an empty canvas). `None` on overflow.
    pub const fn required_len(width: u32, height: u32, stride: usize, format: PixelFormat) -> Option<usize> {
        if width == 0 || height == 0 {
            return Some(0);
        }
        let row = match (width as usize).checked_mul(format.bytes_per_pixel()) {
            Some(r) => r,
            None => return None,
        };
        match stride.checked_mul(height as usize - 1) {
            Some(rows) => rows.checked_add(row),
            None => None,
        }
    }

    pub fn new(
        buf: &'a mut [u8],
        width: u32,
        height: u32,
        stride: usize,
        format: PixelFormat,
    ) -> Result<Canvas<'a>, LayoutError> {
        if width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(LayoutError::TooLarge);
        }
        let row = (width as usize).checked_mul(format.bytes_per_pixel()).ok_or(LayoutError::TooLarge)?;
        if height > 1 && stride < row {
            return Err(LayoutError::StrideTooSmall);
        }
        let need = Self::required_len(width, height, stride, format).ok_or(LayoutError::TooLarge)?;
        if buf.len() < need {
            return Err(LayoutError::BufferTooSmall);
        }
        Ok(Canvas { buf, width, height, stride, format })
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Bytes from the start of one row to the start of the next.
    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// The pixel bytes, row by row with the stride's padding.
    pub fn as_bytes(&self) -> &[u8] {
        self.buf
    }

    /// The same pixels, borrowed for a shorter time: hand a canvas to a
    /// function that takes `&mut Canvas<'_>` and keep using it afterwards.
    pub fn reborrow(&mut self) -> Canvas<'_> {
        Canvas { buf: self.buf, width: self.width, height: self.height, stride: self.stride, format: self.format }
    }

    /// A canvas over the part of this one inside `r` (clipped; possibly
    /// empty), whose (0, 0) is `r`'s top-left corner.
    pub fn region(&mut self, r: Rect) -> Canvas<'_> {
        let r = r.intersect(&self.bounds());
        let (format, stride) = (self.format, self.stride);
        if r.is_empty() {
            return Canvas { buf: &mut [], width: 0, height: 0, stride, format };
        }
        let start = self.offset(r.x as u32, r.y as u32);
        // Inside the buffer: `r` lies within the canvas, which `new` checked.
        let len = Self::required_len(r.width, r.height, stride, format).unwrap_or(0);
        Canvas { buf: &mut self.buf[start..start + len], width: r.width, height: r.height, stride, format }
    }

    const fn bpp(&self) -> usize {
        self.format.bytes_per_pixel()
    }

    /// Byte offset of pixel (`x`, `y`), which must be inside the canvas.
    const fn offset(&self, x: u32, y: u32) -> usize {
        y as usize * self.stride + x as usize * self.bpp()
    }

    /// `r` clipped to the canvas, as unsigned coordinates; `None` if
    /// nothing is left.
    fn clip(&self, r: Rect) -> Option<(u32, u32, u32, u32)> {
        let r = r.intersect(&self.bounds());
        (!r.is_empty()).then_some((r.x as u32, r.y as u32, r.width, r.height))
    }

    /// The colour at (`x`, `y`); `None` outside the canvas.
    pub fn pixel(&self, x: i32, y: i32) -> Option<Color> {
        if !self.bounds().contains(x, y) {
            return None;
        }
        let at = self.offset(x as u32, y as u32);
        Some(self.format.decode(&self.buf[at..at + self.bpp()]))
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, c: Color) {
        self.fill_rect(Rect::new(x, y, 1, 1), c);
    }

    pub fn clear(&mut self, c: Color) {
        self.fill_rect(self.bounds(), c);
    }

    /// Fill the part of `r` inside the canvas.
    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        let Some((x, y, w, h)) = self.clip(r) else { return };
        let px = self.format.encode(c);
        let bpp = self.bpp();
        for row in y..y + h {
            let start = self.offset(x, row);
            let span = &mut self.buf[start..start + w as usize * bpp];
            for p in span.chunks_exact_mut(bpp) {
                p.copy_from_slice(&px[..bpp]);
            }
        }
    }

    /// `len` pixels to the right from (`x`, `y`).
    pub fn hline(&mut self, x: i32, y: i32, len: u32, c: Color) {
        self.fill_rect(Rect::new(x, y, len, 1), c);
    }

    /// `len` pixels down from (`x`, `y`).
    pub fn vline(&mut self, x: i32, y: i32, len: u32, c: Color) {
        self.fill_rect(Rect::new(x, y, 1, len), c);
    }

    /// The one-pixel outline of `r`.
    pub fn stroke_rect(&mut self, r: Rect, c: Color) {
        if r.is_empty() {
            return;
        }
        let (right, bottom) = (r.right() - 1, r.bottom() - 1);
        self.hline(r.x, r.y, r.width, c);
        self.vline(r.x, r.y, r.height, c);
        if let Ok(bottom) = i32::try_from(bottom) {
            self.hline(r.x, bottom, r.width, c);
        }
        if let Ok(right) = i32::try_from(right) {
            self.vline(right, r.y, r.height, c);
        }
    }

    /// Where pixels of `src_rect` (in a canvas of `src_bounds`) land when
    /// its top-left corner goes to (`x`, `y`) here, both sides clipped:
    /// (source x, source y, destination x, destination y, width, height).
    fn clip_copy(&self, src_bounds: Rect, src_rect: Rect, x: i32, y: i32) -> Option<[u32; 6]> {
        let s = src_rect.intersect(&src_bounds);
        if s.is_empty() {
            return None;
        }
        // Where the clipped source's corner lands, then the destination
        // clipped to this canvas, then the source shifted to match.
        let dx = x as i64 + (s.x as i64 - src_rect.x as i64);
        let dy = y as i64 + (s.y as i64 - src_rect.y as i64);
        let d = Rect::from_edges(dx, dy, dx + s.width as i64, dy + s.height as i64).intersect(&self.bounds());
        if d.is_empty() {
            return None;
        }
        let sx = s.x as i64 + (d.x as i64 - dx);
        let sy = s.y as i64 + (d.y as i64 - dy);
        Some([sx as u32, sy as u32, d.x as u32, d.y as u32, d.width, d.height])
    }

    /// Copy the pixels of `src_rect` in `src` here, with its top-left corner
    /// at (`x`, `y`). Both sides are clipped; formats are converted.
    pub fn blit(&mut self, x: i32, y: i32, src: &Canvas<'_>, src_rect: Rect) {
        let Some([sx, sy, dx, dy, w, h]) = self.clip_copy(src.bounds(), src_rect, x, y) else { return };
        let (sbpp, dbpp) = (src.bpp(), self.bpp());
        for row in 0..h {
            let s = src.offset(sx, sy + row);
            let d = self.offset(dx, dy + row);
            let (s, d) = (&src.buf[s..s + w as usize * sbpp], &mut self.buf[d..d + w as usize * dbpp]);
            if src.format == self.format {
                d.copy_from_slice(s);
            } else {
                for (dp, sp) in d.chunks_exact_mut(dbpp).zip(s.chunks_exact(sbpp)) {
                    dp.copy_from_slice(&self.format.encode(src.format.decode(sp))[..dbpp]);
                }
            }
        }
    }

    /// Copy the pixels of `src` to (`x`, `y`) in this canvas, as if through
    /// a temporary copy (the two may overlap): how a text screen scrolls.
    pub fn copy_within(&mut self, src: Rect, x: i32, y: i32) {
        let Some([sx, sy, dx, dy, w, h]) = self.clip_copy(self.bounds(), src, x, y) else { return };
        let len = w as usize * self.bpp();
        // Moving down, copy the bottom row first so no source row is
        // overwritten before it is read; within a row, `copy_within` is a
        // memmove.
        let mut copy_row = |row: u32| {
            let s = self.offset(sx, sy + row);
            self.buf.copy_within(s..s + len, self.offset(dx, dy + row));
        };
        if dy > sy {
            (0..h).rev().for_each(&mut copy_row);
        } else {
            (0..h).for_each(&mut copy_row);
        }
    }

    /// Draw `glyph` with its top-left corner at (`x`, `y`), each glyph pixel
    /// `scale` × `scale` pixels big (a scale of 0 draws nothing). Lit
    /// pixels get `fg`, the others `bg`, or stay as they are without one.
    pub fn draw_glyph(&mut self, x: i32, y: i32, glyph: Glyph<'_>, fg: Color, bg: Option<Color>, scale: u32) {
        self.draw_decorated(x, y, glyph, fg, bg, scale, Decorations::default());
    }

    #[allow(clippy::too_many_arguments, reason = "internal; the public wrappers take fewer")]
    pub(crate) fn draw_decorated(
        &mut self,
        x: i32,
        y: i32,
        glyph: Glyph<'_>,
        fg: Color,
        bg: Option<Color>,
        scale: u32,
        deco: Decorations,
    ) {
        let (gw, gh) = (glyph.width(), glyph.height());
        let cell = Rect::new(x, y, gw.saturating_mul(scale), gh.saturating_mul(scale));
        let Some((cx, cy, w, h)) = self.clip(cell) else { return };
        let (fg, bg) = (self.format.encode(fg), bg.map(|c| self.format.encode(c)));
        let bpp = self.bpp();
        // The glyph is at most 32 pixels wide (see `Glyph::row`).
        let mask = u32::MAX.checked_shl(32 - gw.min(32)).unwrap_or(0);
        // Offsets of the visible part within the scaled glyph.
        let (ox, oy) = ((cx as i64 - x as i64) as u32, (cy as i64 - y as i64) as u32);
        for row in 0..h {
            let gy = (oy + row) / scale;
            let mut bits = glyph.row(gy);
            if deco.bold {
                bits |= bits >> 1;
            }
            if deco.underline == Some(gy) || deco.strike == Some(gy) {
                bits = u32::MAX;
            }
            bits &= mask;
            let start = self.offset(cx, cy + row);
            let span = &mut self.buf[start..start + w as usize * bpp];
            // Each glyph column covers `scale` pixels: shift to the next bit
            // after that many (no division per pixel, which matters for
            // full-screen text).
            let mut bit = 0x8000_0000u32.checked_shr(ox / scale).unwrap_or(0);
            let mut left = scale - ox % scale;
            for p in span.chunks_exact_mut(bpp) {
                if bits & bit != 0 {
                    p.copy_from_slice(&fg[..bpp]);
                } else if let Some(bg) = &bg {
                    p.copy_from_slice(&bg[..bpp]);
                }
                left -= 1;
                if left == 0 {
                    (bit, left) = (bit >> 1, scale);
                }
            }
        }
    }

    /// Draw `c` in `font` at (`x`, `y`), the top-left corner of its cell;
    /// returns the `x` of the next cell.
    pub fn draw_char(&mut self, font: &Font, x: i32, y: i32, c: char, fg: Color, bg: Option<Color>) -> i32 {
        self.draw_glyph(x, y, font.glyph(c), fg, bg, 1);
        x.saturating_add(font.width() as i32)
    }

    /// Draw `s` on one line, one cell per character (control characters
    /// included, as the fallback glyph); returns the `x` after the last.
    pub fn draw_str(&mut self, font: &Font, x: i32, y: i32, s: &str, fg: Color, bg: Option<Color>) -> i32 {
        self.draw_str_scaled(font, x, y, s, fg, bg, 1)
    }

    /// [`draw_str`](Self::draw_str) with every glyph pixel `scale` ×
    /// `scale` pixels big.
    #[allow(clippy::too_many_arguments, reason = "draw_str plus a scale")]
    pub fn draw_str_scaled(
        &mut self,
        font: &Font,
        mut x: i32,
        y: i32,
        s: &str,
        fg: Color,
        bg: Option<Color>,
        scale: u32,
    ) -> i32 {
        let advance = i32::try_from(font.width().saturating_mul(scale)).unwrap_or(i32::MAX);
        for c in s.chars() {
            self.draw_glyph(x, y, font.glyph(c), fg, bg, scale);
            x = x.saturating_add(advance);
        }
        x
    }
}

#[cfg(test)]
pub(crate) mod tests;
