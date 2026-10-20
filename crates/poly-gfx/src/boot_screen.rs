//! The picture the kernel shows as soon as the display is up: "Poly" in
//! large letters, the kernel version and the screen size, centred on the
//! terminal background.
//!
//! It lives here rather than in the kernel so that the boot test
//! (`cargo xtask test`) can draw the same picture for the same screen size
//! and compare it, pixel for pixel, with a screenshot of the guest.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::font::{SPLEEN_8X16, SPLEEN_12X24};
use crate::geom::Rect;
use crate::grid::Palette;
use core::fmt::{self, Write};

/// The screen's colour: the terminal's background.
pub const BACKGROUND: Color = Palette::DEFAULT.background;
/// "Poly".
pub const TITLE: Color = Color::from_rgb24(0x82b6ff);
/// The line under it.
pub const TEXT: Color = Color::from_rgb24(0x8a909c);

pub const TITLE_TEXT: &str = "Poly";

/// How much the title's font is scaled up on a canvas `height` pixels tall.
pub const fn title_scale(height: u32) -> u32 {
    let s = height / 192;
    if s < 1 {
        1
    } else if s > 4 {
        4
    } else {
        s
    }
}

/// Where the title goes on a `width` × `height` canvas.
pub fn title_rect(width: u32, height: u32) -> Rect {
    let font = &SPLEEN_12X24;
    let scale = title_scale(height);
    let (w, h) = (font.text_width(TITLE_TEXT) * scale, font.height() * scale);
    let x = (width as i64 - w as i64) / 2;
    let y = height as i64 * 2 / 5 - h as i64 / 2;
    Rect::new(x as i32, y as i32, w, h)
}

/// Draw the boot screen for kernel `version` over all of `canvas`.
pub fn draw(canvas: &mut Canvas<'_>, version: &str) {
    let (width, height) = (canvas.width(), canvas.height());
    canvas.clear(BACKGROUND);
    let title = title_rect(width, height);
    let scale = title_scale(height);
    canvas.draw_str_scaled(&SPLEEN_12X24, title.x, title.y, TITLE_TEXT, TITLE, None, scale);

    // An accent line under the title, then the version and screen size.
    let line_y = title.y.saturating_add_unsigned(title.height + 4 * scale);
    canvas.hline(title.x, line_y, title.width, TITLE.mix(BACKGROUND));
    let mut text = Line::default();
    let _ = write!(text, "kernel {version} \u{b7} {width}x{height}");
    let font = &SPLEEN_8X16;
    let x = (width as i64 - font.text_width(text.as_str()) as i64) / 2;
    canvas.draw_str(font, x as i32, line_y.saturating_add_unsigned(4 * scale + 2), text.as_str(), TEXT, None);
}

/// A line of text in a fixed buffer; what does not fit is cut off at a
/// character boundary.
#[derive(Debug)]
struct Line {
    buf: [u8; 96],
    len: usize,
}

impl Default for Line {
    fn default() -> Line {
        Line { buf: [0; 96], len: 0 }
    }
}

impl Line {
    fn as_str(&self) -> &str {
        // Only whole characters are ever copied in.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            let n = c.len_utf8();
            if self.len + n > self.buf.len() {
                return Err(fmt::Error);
            }
            c.encode_utf8(&mut self.buf[self.len..]);
            self.len += n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::color::PixelFormat;
    use std::vec;

    #[test]
    fn title_and_text_are_drawn_centred() {
        let (w, h) = (640u32, 480u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let mut c = Canvas::new(&mut buf, w, h, w as usize * 4, PixelFormat::Xrgb8888).unwrap();
        draw(&mut c, "0.0.1");
        assert_eq!(c.pixel(0, 0), Some(BACKGROUND));
        assert_eq!(c.pixel(w as i32 - 1, h as i32 - 1), Some(BACKGROUND));
        let t = title_rect(w, h);
        assert_eq!(title_scale(h), 2);
        assert_eq!((t.width, t.height), (4 * 12 * 2, 24 * 2));
        assert_eq!(t.x, (640 - 96) / 2);
        let count = |color: Color, r: Rect| {
            (r.y..r.y + r.height as i32)
                .flat_map(|y| (r.x..r.x + r.width as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| c.pixel(x, y) == Some(color))
                .count()
        };
        // The title's pixels are all inside its rectangle.
        let lit = count(TITLE, t);
        assert!(lit > 500, "{lit} title pixels");
        assert_eq!(count(TITLE, c.bounds()), lit);
        // "kernel 0.0.1 · 640x480" is 22 characters, centred below.
        assert!(count(TEXT, Rect::new(0, t.y + t.height as i32, w, 40)) > 100);
        assert_eq!(count(TEXT, Rect::new(0, 0, (w - 22 * 8) / 2, h)), 0);
    }

    #[test]
    fn tiny_canvases_do_not_panic() {
        for (w, h) in [(1, 1), (10, 300), (2000, 2)] {
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let mut c = Canvas::new(&mut buf, w, h, w as usize * 4, PixelFormat::Xrgb8888).unwrap();
            draw(&mut c, "a very long version string that will not fit in the line buffer at all, not even close");
        }
    }
}
