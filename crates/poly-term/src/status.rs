//! The status bar: one row of text across the top of the screen.
//!
//! ```text
//!  Poly 0.0.1 | up 0:01:23 | 243 MiB free of 256 MiB | us | 1 of 2 CPUs        Shift+PgUp: scrollback
//! ```
//!
//! [`format`] turns a [`StatusInfo`] into that line for a screen `cols`
//! characters wide: the facts on the left, a hint (or, while the view is
//! scrolled back, where it is) on the right. What does not fit is cut from
//! the right side first, then from the end of the left. [`draw`] paints it.

use core::fmt::{self, Write};
use poly_gfx::{Canvas, Color, Font, Rect};

/// The status bar's background.
pub const BACKGROUND: Color = Color::from_rgb24(0x2a2f3a);
/// Its text.
pub const FOREGROUND: Color = Color::from_rgb24(0xd4d7dd);
/// What separates the facts on the left.
pub const SEPARATOR: &str = " | ";
/// The product name the line starts with.
pub const TITLE: &str = "Poly";

/// What the status bar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusInfo {
    /// The kernel's version.
    pub version: &'static str,
    /// Nanoseconds since boot.
    pub uptime_ns: u64,
    /// Free and total physical memory in bytes.
    pub free_bytes: u64,
    pub total_bytes: u64,
    /// The keyboard layout's short name (`us`, `de-ch`).
    pub layout: &'static str,
    /// CPUs running the kernel, and CPUs the machine has.
    pub cpus_online: usize,
    pub cpus: usize,
    /// How many lines the view is scrolled back, of how many there are.
    pub scrolled: Option<(usize, usize)>,
}

/// The longest status line, in bytes.
pub const MAX_LINE: usize = 1024;

/// A status line: text in a fixed buffer, at most [`MAX_LINE`] bytes; what
/// does not fit is dropped at a character boundary.
#[derive(Clone)]
pub struct Line {
    buf: [u8; MAX_LINE],
    len: usize,
}

impl Default for Line {
    fn default() -> Line {
        Line { buf: [0; MAX_LINE], len: 0 }
    }
}

impl fmt::Debug for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialEq for Line {
    fn eq(&self, other: &Line) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Line {}

impl Line {
    pub fn as_str(&self) -> &str {
        // Only whole characters are ever copied in.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Characters in the line.
    pub fn chars(&self) -> usize {
        self.as_str().chars().count()
    }

    /// Keep the first `n` characters.
    pub fn truncate_chars(&mut self, n: usize) {
        if let Some((i, _)) = self.as_str().char_indices().nth(n) {
            self.len = i;
        }
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            let n = c.len_utf8();
            if self.len + n > MAX_LINE {
                return Err(fmt::Error);
            }
            c.encode_utf8(&mut self.buf[self.len..]);
            self.len += n;
        }
        Ok(())
    }
}

/// Columns the start of the line takes that never changes while the
/// kernel runs: a space, [`TITLE`], a space and `version`.
pub fn title_cols(version: &str) -> usize {
    1 + TITLE.len() + 1 + version.chars().count()
}

/// An uptime as `h:mm:ss`, with days in front once there are any
/// (`2d 3:04:05`).
pub fn uptime(ns: u64) -> impl fmt::Display {
    struct Uptime(u64);
    impl fmt::Display for Uptime {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let s = self.0 / 1_000_000_000;
            let (d, h, m, s) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
            if d > 0 { write!(f, "{d}d {h}:{m:02}:{s:02}") } else { write!(f, "{h}:{m:02}:{s:02}") }
        }
    }
    Uptime(ns)
}

/// A size in bytes, in the largest binary unit that keeps it at least 1,
/// rounded down (`512 B`, `243 MiB`, `2 GiB`).
pub fn size(bytes: u64) -> impl fmt::Display {
    struct Size(u64);
    impl fmt::Display for Size {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
            let mut i = 0;
            while i + 1 < UNITS.len() && self.0 >> (10 * (i + 1)) > 0 {
                i += 1;
            }
            write!(f, "{} {}", self.0 >> (10 * i), UNITS[i])
        }
    }
    Size(bytes)
}

/// The status line for `info` on a screen `cols` characters wide: exactly
/// `cols` characters (padded with spaces), or [`MAX_LINE`] bytes' worth.
pub fn format(info: &StatusInfo, cols: usize) -> Line {
    let mut left = Line::default();
    let _ = write!(
        left,
        " {TITLE} {}{SEPARATOR}up {}{SEPARATOR}{} free of {}{SEPARATOR}{}{SEPARATOR}{} of {} CPU{}",
        info.version,
        uptime(info.uptime_ns),
        size(info.free_bytes),
        size(info.total_bytes),
        info.layout,
        info.cpus_online,
        info.cpus,
        if info.cpus == 1 { "" } else { "s" },
    );
    let mut right = Line::default();
    let _ = match info.scrolled {
        Some((view, history)) if view > 0 => write!(right, "scrollback {view} of {history} lines  Shift+PgDn "),
        _ => write!(right, "Shift+PgUp: scrollback "),
    };
    let mut line = left.clone();
    let (l, r) = (left.chars(), right.chars());
    if l + 1 + r <= cols {
        for _ in 0..cols - l - r {
            let _ = line.write_char(' ');
        }
        let _ = line.write_str(right.as_str());
    } else {
        line.truncate_chars(cols);
        for _ in line.chars()..cols {
            if line.write_char(' ').is_err() {
                break;
            }
        }
    }
    line
}

/// Paint the status bar: `rect` in [`BACKGROUND`], `text` in
/// [`FOREGROUND`] from its left edge, one `font` cell per character
/// (clipped to `rect`). Returns `rect` (clipped to the canvas).
pub fn draw(canvas: &mut Canvas<'_>, rect: Rect, font: &Font, text: &str) -> Rect {
    {
        let mut area = canvas.region(rect);
        area.clear(BACKGROUND);
        area.draw_str(font, 0, 0, text, FOREGROUND, Some(BACKGROUND));
    }
    rect.intersect(&canvas.bounds())
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_gfx::{PixelFormat, SPLEEN_8X16};
    use std::string::ToString;
    use std::vec;

    fn info() -> StatusInfo {
        StatusInfo {
            version: "0.0.1",
            uptime_ns: 83 * 1_000_000_000 + 999_999_999,
            free_bytes: 243 << 20,
            total_bytes: (256 << 20) - 4096,
            layout: "us",
            cpus_online: 1,
            cpus: 2,
            scrolled: None,
        }
    }

    #[test]
    fn numbers_read_well() {
        assert_eq!(uptime(0).to_string(), "0:00:00");
        assert_eq!(uptime(3_723_000_000_000).to_string(), "1:02:03");
        assert_eq!(uptime((2 * 86_400 + 3 * 3600 + 4 * 60 + 5) * 1_000_000_000).to_string(), "2d 3:04:05");
        assert_eq!(size(0).to_string(), "0 B");
        assert_eq!(size(1023).to_string(), "1023 B");
        assert_eq!(size(1024).to_string(), "1 KiB");
        assert_eq!(size((256 << 20) - 1).to_string(), "255 MiB");
        assert_eq!(size(2 << 30).to_string(), "2 GiB");
        assert_eq!(size(u64::MAX).to_string(), "16777215 TiB");
    }

    #[test]
    fn a_wide_screen_gets_facts_left_and_the_hint_right() {
        let line = format(&info(), 100);
        assert_eq!(line.chars(), 100);
        assert!(
            line.as_str().starts_with(" Poly 0.0.1 | up 0:01:23 | 243 MiB free of 255 MiB | us | 1 of 2 CPUs  "),
            "{line:?}"
        );
        assert!(line.as_str().ends_with(" Shift+PgUp: scrollback "), "{line:?}");
        assert_eq!(title_cols("0.0.1"), " Poly 0.0.1".len());
        let one = format(&StatusInfo { cpus: 1, ..info() }, 100);
        assert!(one.as_str().contains("| 1 of 1 CPU "), "{one:?}");
    }

    #[test]
    fn scrolling_back_shows_where_the_view_is() {
        let line = format(&StatusInfo { scrolled: Some((48, 300)), ..info() }, 120);
        assert!(line.as_str().ends_with(" scrollback 48 of 300 lines  Shift+PgDn "), "{line:?}");
        let bottom = format(&StatusInfo { scrolled: Some((0, 300)), ..info() }, 120);
        assert!(bottom.as_str().ends_with("Shift+PgUp: scrollback "));
    }

    #[test]
    fn narrow_screens_cut_the_right_side_then_the_left() {
        let line = format(&info(), 40);
        assert_eq!(line.as_str(), " Poly 0.0.1 | up 0:01:23 | 243 MiB free ");
        assert_eq!(format(&info(), 0).as_str(), "");
        assert_eq!(format(&info(), 3).as_str(), " Po");
        // Wider than the buffer: as much as fits.
        let wide = format(&info(), 5000);
        assert_eq!(wide.as_str().len(), MAX_LINE);
        let mut l = Line::default();
        assert!(l.write_str(&"é".repeat(600)).is_err());
        assert_eq!(l.as_str().len(), 1024);
    }

    #[test]
    fn draws_the_text_on_the_bar_colour() {
        let (w, h) = (64u32, 20u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let mut c = Canvas::new(&mut buf, w, h, w as usize * 4, PixelFormat::Xrgb8888).unwrap();
        let r = draw(&mut c, Rect::new(0, 0, 100, 16), &SPLEEN_8X16, " Poly");
        assert_eq!(r, Rect::new(0, 0, 64, 16));
        let count = |color: Color| {
            (0..16).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| c.pixel(x, y) == Some(color)).count()
        };
        let (bg, fg) = (count(BACKGROUND), count(FOREGROUND));
        assert_eq!(bg + fg, 64 * 16);
        assert!(fg > 30, "{fg} text pixels");
        // Nothing outside the bar, and no text in the first (blank) cell.
        assert!((16..20).all(|y| (0..64).all(|x| c.pixel(x, y) == Some(Color::BLACK))));
        assert!((0..16).all(|y| (0..8).all(|x| c.pixel(x, y) == Some(BACKGROUND))));
    }
}
