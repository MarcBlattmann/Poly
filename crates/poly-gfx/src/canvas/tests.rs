extern crate std;

use super::*;
use crate::font::{SPLEEN_8X16, SPLEEN_12X24};
use std::vec;
use std::vec::Vec;

const RED: Color = Color::rgb(0xff, 0, 0);
const GREEN: Color = Color::rgb(0, 0xff, 0);
const BLUE: Color = Color::rgb(0, 0, 0xff);
/// What padding bytes hold before a test, to see that nothing wrote them.
const SENTINEL: u8 = 0xa5;

/// A buffer for a `w` × `h` canvas whose rows are padded by 3 bytes.
fn buffer(w: u32, h: u32, format: PixelFormat) -> (Vec<u8>, usize) {
    let stride = w as usize * format.bytes_per_pixel() + 3;
    (vec![SENTINEL; stride * h as usize], stride)
}

/// Every pixel, row by row.
fn pixels(c: &Canvas<'_>) -> Vec<Color> {
    (0..c.height() as i32)
        .flat_map(|y| (0..c.width() as i32).map(move |x| (x, y)))
        .map(|(x, y)| c.pixel(x, y).unwrap())
        .collect()
}

/// The padding bytes at the end of every row still hold the sentinel.
fn padding_untouched(buf: &[u8], w: u32, stride: usize, format: PixelFormat) -> bool {
    buf.chunks(stride).all(|row| row[w as usize * format.bytes_per_pixel()..].iter().all(|&b| b == SENTINEL))
}

#[test]
fn new_checks_the_layout() {
    let mut buf = vec![0u8; 100];
    let f = PixelFormat::Xrgb8888;
    assert_eq!(Canvas::new(&mut buf, 5, 5, 19, f).unwrap_err(), LayoutError::StrideTooSmall);
    assert_eq!(Canvas::new(&mut buf, 5, 6, 20, f).unwrap_err(), LayoutError::BufferTooSmall);
    assert_eq!(Canvas::new(&mut buf, u32::MAX, 1, 0, f).unwrap_err(), LayoutError::TooLarge);
    // The last row needs only its pixels, not a whole stride.
    let c = Canvas::new(&mut buf, 5, 5, 20, f).unwrap();
    assert_eq!((c.width(), c.height(), c.stride(), c.format()), (5, 5, 20, f));
    let mut buf = vec![0u8; 4 * 24 + 20];
    assert!(Canvas::new(&mut buf, 5, 5, 24, f).is_ok());
    assert_eq!(Canvas::required_len(5, 5, 24, f), Some(4 * 24 + 20));
    assert_eq!(Canvas::required_len(0, 5, 24, f), Some(0));
    assert!(Canvas::new(&mut [], 0, 0, 0, f).is_ok());
}

#[test]
fn fill_rect_clips_and_leaves_padding_alone() {
    for format in [PixelFormat::Xrgb8888, PixelFormat::Xbgr8888, PixelFormat::Rgb888, PixelFormat::Rgb565] {
        let (mut buf, stride) = buffer(10, 6, format);
        let mut c = Canvas::new(&mut buf, 10, 6, stride, format).unwrap();
        c.clear(BLUE);
        c.fill_rect(Rect::new(-3, -2, 5, 4), RED); // top-left corner: (0,0)-(1,1)
        c.fill_rect(Rect::new(8, 4, 100, 100), GREEN); // bottom-right: (8,4)-(9,5)
        c.fill_rect(Rect::new(10, 0, 5, 5), RED); // entirely outside
        c.fill_rect(Rect::new(i32::MIN, i32::MIN, u32::MAX, 1), RED); // ends above the canvas
        for y in 0..6 {
            for x in 0..10 {
                let want = match (x, y) {
                    (0..=1, 0..=1) => RED,
                    (8..=9, 4..=5) => GREEN,
                    _ => BLUE,
                };
                assert_eq!(c.pixel(x, y), Some(want), "{format:?} at ({x}, {y})");
            }
        }
        assert_eq!(c.pixel(10, 0), None);
        assert_eq!(c.pixel(-1, 0), None);
        assert!(padding_untouched(&buf, 10, stride, format), "{format:?}");
    }
}

#[test]
fn lines_and_outlines() {
    let f = PixelFormat::Xrgb8888;
    let (mut buf, stride) = buffer(8, 8, f);
    let mut c = Canvas::new(&mut buf, 8, 8, stride, f).unwrap();
    c.clear(Color::BLACK);
    c.stroke_rect(Rect::new(1, 1, 4, 3), RED);
    c.hline(-2, 6, 4, GREEN); // (0,6), (1,6)
    c.vline(7, 5, 10, BLUE); // (7,5)-(7,7)
    c.set_pixel(6, 0, GREEN);
    c.set_pixel(8, 0, GREEN); // outside
    let art: Vec<&str> = [
        "......G.", //
        ".RRRR...", ".R..R...", ".RRRR...", "........", ".......B", "GG.....B", ".......B",
    ]
    .to_vec();
    for (y, line) in art.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            let want = match ch {
                'R' => RED,
                'G' => GREEN,
                'B' => BLUE,
                _ => Color::BLACK,
            };
            assert_eq!(c.pixel(x as i32, y as i32), Some(want), "({x}, {y})");
        }
    }
    // A 1x1 outline is one pixel; an empty one draws nothing.
    c.clear(Color::BLACK);
    c.stroke_rect(Rect::new(3, 3, 1, 1), RED);
    c.stroke_rect(Rect::new(0, 0, 0, 5), RED);
    assert_eq!(pixels(&c).iter().filter(|&&p| p == RED).count(), 1);
}

/// A canvas of `w` × `h` in `format` with a distinct colour per pixel.
fn numbered(buf: &mut Vec<u8>, w: u32, h: u32, format: PixelFormat) -> Canvas<'_> {
    let stride = w as usize * format.bytes_per_pixel();
    buf.resize(stride * h as usize, 0);
    let mut c = Canvas::new(buf, w, h, stride, format).unwrap();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            c.set_pixel(x, y, Color::rgb(x as u8 * 8, y as u8 * 8, 0x80));
        }
    }
    c
}

#[test]
fn blit_copies_clips_and_converts() {
    let mut sbuf = Vec::new();
    let src = numbered(&mut sbuf, 6, 4, PixelFormat::Xrgb8888);
    for format in [PixelFormat::Xrgb8888, PixelFormat::Xbgr8888, PixelFormat::Rgb888] {
        let (mut buf, stride) = buffer(5, 5, format);
        let mut c = Canvas::new(&mut buf, 5, 5, stride, format).unwrap();
        c.clear(Color::BLACK);
        // Source rectangle sticks out on the left and top of the source;
        // destination sticks out on the right of the canvas.
        c.blit(2, 1, &src, Rect::new(-1, -1, 5, 3));
        for y in 0..5 {
            for x in 0..5 {
                // Destination (2,1) shows source (-1,-1), which does not
                // exist: only source pixels from (0,0) on are copied.
                let (sx, sy) = (x - 2 - 1, y - 1 - 1);
                let want = if (3..5).contains(&x) && (2..4).contains(&y) && sx < 4 && sy < 2 {
                    src.pixel(sx, sy).unwrap()
                } else {
                    Color::BLACK
                };
                assert_eq!(c.pixel(x, y), Some(want), "{format:?} ({x}, {y})");
            }
        }
        assert!(padding_untouched(&buf, 5, stride, format));
    }
}

/// `copy_within` against a straightforward model: read every source pixel
/// first, then write them.
#[test]
fn copy_within_matches_a_copy_through_a_temporary() {
    let mut rng = Rng(0x0dd_ba11);
    for _ in 0..500 {
        let (w, h) = (1 + rng.below(12), 1 + rng.below(12));
        let mut buf = Vec::new();
        let mut c = numbered(&mut buf, w, h, PixelFormat::Xrgb8888);
        let before = pixels(&c);
        let src = Rect::new(rng.range(-4, 14), rng.range(-4, 14), rng.below(16), rng.below(16));
        let (x, y) = (rng.range(-6, 14), rng.range(-6, 14));
        c.copy_within(src, x, y);
        let mut want = before.clone();
        for sy in src.y..src.y + src.height as i32 {
            for sx in src.x..src.x + src.width as i32 {
                let (dx, dy) = (x + sx - src.x, y + sy - src.y);
                let inside = |px: i32, py: i32| (0..w as i32).contains(&px) && (0..h as i32).contains(&py);
                if inside(sx, sy) && inside(dx, dy) {
                    want[(dy * w as i32 + dx) as usize] = before[(sy * w as i32 + sx) as usize];
                }
            }
        }
        assert_eq!(pixels(&c), want, "{w}x{h}: copy {src:?} to ({x}, {y})");
    }
}

/// Filling and clipping against a model of the canvas as a grid of colours.
#[test]
fn random_fills_match_a_model() {
    let mut rng = Rng(0x5eed);
    let (w, h) = (17, 9);
    let f = PixelFormat::Xrgb8888;
    let (mut buf, stride) = buffer(w, h, f);
    let mut c = Canvas::new(&mut buf, w, h, stride, f).unwrap();
    c.clear(Color::BLACK);
    let mut model = vec![Color::BLACK; (w * h) as usize];
    for i in 0..2000u32 {
        let r = Rect::new(rng.range(-20, 30), rng.range(-20, 30), rng.below(30), rng.below(30));
        let color = Color::from_rgb24(i.wrapping_mul(0x9e3779b9));
        c.fill_rect(r, color);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if r.contains(x, y) {
                    model[(y * w as i32 + x) as usize] = color;
                }
            }
        }
    }
    assert_eq!(pixels(&c), model);
    assert!(padding_untouched(&buf, w, stride, f));
}

#[test]
fn region_is_a_window_onto_the_canvas() {
    let f = PixelFormat::Xrgb8888;
    let (mut buf, stride) = buffer(10, 10, f);
    let mut c = Canvas::new(&mut buf, 10, 10, stride, f).unwrap();
    c.clear(Color::BLACK);
    {
        let mut r = c.region(Rect::new(6, 7, 10, 10)); // clipped to 4x3
        assert_eq!((r.width(), r.height()), (4, 3));
        r.clear(RED);
        r.set_pixel(0, 0, GREEN);
        assert!(c_empty(&mut r.region(Rect::new(4, 0, 1, 1))));
    }
    assert_eq!(c.pixel(6, 7), Some(GREEN));
    assert_eq!(c.pixel(9, 9), Some(RED));
    assert_eq!(c.pixel(5, 7), Some(Color::BLACK));
    assert_eq!(c.pixel(6, 6), Some(Color::BLACK));
    assert!(c_empty(&mut c.region(Rect::new(-5, 0, 5, 5))));
    assert_eq!(c.reborrow().pixel(9, 9), Some(RED));
}

fn c_empty(c: &mut Canvas<'_>) -> bool {
    c.clear(RED); // must not panic
    c.width() == 0 && c.height() == 0
}

/// The canvas as text: `#` for `fg`, `.` for anything else.
fn art(c: &Canvas<'_>, r: Rect, fg: Color) -> Vec<std::string::String> {
    (r.y..r.y + r.height as i32)
        .map(|y| (r.x..r.x + r.width as i32).map(|x| if c.pixel(x, y) == Some(fg) { '#' } else { '.' }).collect())
        .collect()
}

#[test]
fn draws_glyphs_pixel_exact() {
    let f = PixelFormat::Xrgb8888;
    let (mut buf, stride) = buffer(40, 20, f);
    let mut c = Canvas::new(&mut buf, 40, 20, stride, f).unwrap();
    c.clear(BLUE);
    let next = c.draw_char(&SPLEEN_8X16, 2, 1, 'A', Color::WHITE, Some(Color::BLACK));
    assert_eq!(next, 10);
    // Spleen 8x16 "A" (spleen-8x16.bdf, ENCODING 65).
    let a = [
        "........", "........", ".#####..", "##...##.", "##...##.", "##...##.", "#######.", "##...##.", "##...##.",
        "##...##.", "##...##.", "##...##.", "........", "........", "........", "........",
    ];
    assert_eq!(art(&c, Rect::new(2, 1, 8, 16), Color::WHITE), a);
    // The background was drawn too, and nothing outside the cell.
    assert_eq!(c.pixel(2, 1), Some(Color::BLACK));
    assert_eq!(c.pixel(1, 1), Some(BLUE));
    assert_eq!(c.pixel(10, 5), Some(BLUE));
    assert_eq!(c.pixel(2, 17), Some(BLUE));

    // Without a background only the lit pixels change.
    c.clear(BLUE);
    c.draw_char(&SPLEEN_8X16, 0, 0, 'ä', RED, None);
    let umlaut = art(&c, Rect::new(0, 0, 8, 12), RED);
    assert_eq!(umlaut[2], ".##.##..");
    assert_eq!(umlaut[4], "........");
    assert_eq!(umlaut[5], ".#####..");
    assert_eq!(c.pixel(0, 0), Some(BLUE));
}

#[test]
fn glyphs_clip_at_every_edge() {
    let f = PixelFormat::Xrgb8888;
    let (mut buf, stride) = buffer(12, 10, f);
    let mut c = Canvas::new(&mut buf, 12, 10, stride, f).unwrap();
    // A full block (U+2588) at each corner, half off the canvas.
    for (x, y) in [(-4, -8), (8, -8), (-4, 2), (8, 2)] {
        c.clear(Color::BLACK);
        c.draw_char(&SPLEEN_8X16, x, y, '█', Color::WHITE, None);
        let lit = pixels(&c).iter().filter(|&&p| p == Color::WHITE).count();
        assert_eq!(lit, 4 * 8, "block at ({x}, {y})");
    }
    c.draw_char(&SPLEEN_8X16, i32::MAX, i32::MAX, 'x', RED, Some(RED));
    c.draw_char(&SPLEEN_8X16, i32::MIN, i32::MIN, 'x', RED, Some(RED));
    assert!(!pixels(&c).contains(&RED));
    assert!(padding_untouched(&buf, 12, stride, f));
}

#[test]
fn scaled_text_blows_up_every_pixel() {
    let f = PixelFormat::Xrgb8888;
    let (mut buf1, s1) = buffer(24, 24, f);
    let (mut buf3, s3) = buffer(72, 72, f);
    let mut one = Canvas::new(&mut buf1, 24, 24, s1, f).unwrap();
    let mut three = Canvas::new(&mut buf3, 72, 72, s3, f).unwrap();
    let end = one.draw_str(&SPLEEN_12X24, 0, 0, "Pö", Color::WHITE, Some(Color::BLACK));
    let end3 = three.draw_str_scaled(&SPLEEN_12X24, 0, 0, "Pö", Color::WHITE, Some(Color::BLACK), 3);
    assert_eq!((end, end3), (24, 72));
    for y in 0..72 {
        for x in 0..72 {
            assert_eq!(three.pixel(x, y), one.pixel(x / 3, y / 3), "({x}, {y})");
        }
    }
    // Clipped on the left (and top) part-way into a scaled pixel: what is
    // left is the same as the unclipped drawing, shifted.
    for (dx, dy) in [(5, 0), (1, 2), (3, 7), (35, 4)] {
        let (mut buf, s) = buffer(40, 40, f);
        let mut clipped = Canvas::new(&mut buf, 40, 40, s, f).unwrap();
        clipped.clear(Color::BLACK);
        clipped.draw_str_scaled(&SPLEEN_12X24, -dx, -dy, "Pö", Color::WHITE, None, 3);
        for y in 0..40 {
            for x in 0..40 {
                let want = three.pixel(x + dx, y + dy).unwrap_or(Color::BLACK);
                assert_eq!(clipped.pixel(x, y), Some(want), "shift ({dx}, {dy}) at ({x}, {y})");
            }
        }
    }
    // Scale 0 draws nothing.
    three.clear(BLUE);
    three.draw_str_scaled(&SPLEEN_12X24, 0, 0, "P", RED, Some(RED), 0);
    assert!(!pixels(&three).contains(&RED));
}

#[test]
fn decorations() {
    let f = PixelFormat::Xrgb8888;
    let (mut buf, stride) = buffer(8, 16, f);
    let mut c = Canvas::new(&mut buf, 8, 16, stride, f).unwrap();
    let deco = Decorations { bold: true, underline: Some(13), strike: Some(7) };
    c.draw_decorated(0, 0, SPLEEN_8X16.glyph('l'), Color::WHITE, Some(Color::BLACK), 1, deco);
    let rows = art(&c, Rect::new(0, 0, 8, 16), Color::WHITE);
    let plain = SPLEEN_8X16.glyph('l');
    for (y, row) in rows.iter().enumerate() {
        if y == 7 || y == 13 {
            assert_eq!(row, "########");
            continue;
        }
        // Bold: every lit pixel and its right neighbour.
        for x in 0..8u32 {
            let lit = plain.is_set(x, y as u32) || (x > 0 && plain.is_set(x - 1, y as u32));
            assert_eq!(row.as_bytes()[x as usize] == b'#', lit, "row {y} col {x}");
        }
    }
}

#[test]
fn text_in_other_formats() {
    for format in [PixelFormat::Rgb565, PixelFormat::Rgb888, PixelFormat::Xbgr8888] {
        let (mut buf, stride) = buffer(16, 16, format);
        let mut c = Canvas::new(&mut buf, 16, 16, stride, format).unwrap();
        c.clear(Color::BLACK);
        c.draw_str(&SPLEEN_8X16, 0, 0, "A", Color::WHITE, None);
        assert_eq!(art(&c, Rect::new(0, 2, 8, 1), Color::WHITE), [".#####.."]);
        assert!(padding_untouched(&buf, 16, stride, format));
    }
}

/// xorshift64*: deterministic and dependency-free.
pub(crate) struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }

    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo) as u32) as i32
    }
}
