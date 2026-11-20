//! A cursor theme's pictures (ADR 0252): XCursor files, the format X11 cursor themes are made in
//! (Keith Packard's `Xcursor` library documentation, "The Xcursor file format"). Poly's desktop
//! images carry the Bibata theme's files as data (`/usr/share/icons/Bibata-Modern-Ice/cursors/`);
//! the compositor reads them, and each named look is drawn from them when there is one, else from
//! Poly's own picture ([`super::NamedCursor`]).
//!
//! A file: `Xcur`, the header's length, a version and the number of entries of a table, then the
//! table (type, subtype, position). An image chunk (type `0xfffd0002`, subtype its nominal size):
//! the chunk header's length, type, subtype, version, width, height, hot spot x and y, the delay
//! before the next frame in milliseconds, then width x height pixels of premultiplied ARGB, rows
//! top to bottom, little-endian. A size with several images is an animation, frame by frame.

use super::{NamedCursor, Scale, scale::UNIT};
use alloc::vec::Vec;

const MAGIC: &[u8; 4] = b"Xcur";
const IMAGE: u32 = 0xfffd_0002;
/// The largest side accepted (the format's own limit).
const MAX_SIDE: u32 = 0x7fff;
/// A cursor's nominal size at 100 %: what desktops ask a theme for.
pub const NOMINAL: u32 = 24;

/// One picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XImage {
    /// The size it is meant for (24 for a 24-pixel cursor).
    pub nominal: u32,
    pub width: u32,
    pub height: u32,
    /// The pixel at the pointer's position.
    pub hot: (u32, u32),
    /// Milliseconds before the next frame.
    pub delay: u32,
    /// Premultiplied ARGB, `width * height`, rows top to bottom.
    pub pixels: Vec<u32>,
}

/// The pictures of one cursor: every size, every frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XCursor {
    pub images: Vec<XImage>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

impl XCursor {
    /// Read an XCursor file; `None` when it is not one or has no readable image.
    pub fn parse(b: &[u8]) -> Option<XCursor> {
        if b.get(..4)? != MAGIC {
            return None;
        }
        let header = u32_at(b, 4)? as usize;
        let count = u32_at(b, 12)? as usize;
        let mut images = Vec::new();
        for i in 0..count.min(4096) {
            let entry = header.checked_add(i * 12)?;
            let (kind, pos) = (u32_at(b, entry)?, u32_at(b, entry + 8)? as usize);
            if kind != IMAGE {
                continue;
            }
            let field = |n: usize| u32_at(b, pos.checked_add(n * 4)?);
            let (Some(len), Some(t), Some(nominal), Some(width), Some(height)) =
                (field(0), field(1), field(2), field(4), field(5))
            else {
                continue;
            };
            if t != IMAGE || width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
                continue;
            }
            let (Some(xhot), Some(yhot), Some(delay)) = (field(6), field(7), field(8)) else { continue };
            let start = pos.checked_add(len as usize)?;
            let n = (width * height) as usize;
            let Some(data) = b.get(start..start.checked_add(n * 4)?) else { continue };
            let pixels = data.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            images.push(XImage {
                nominal,
                width,
                height,
                hot: (xhot.min(width - 1), yhot.min(height - 1)),
                delay,
                pixels,
            });
        }
        (!images.is_empty()).then_some(XCursor { images })
    }

    /// The frames of the nominal size nearest to `want` (the larger of two as near).
    pub fn frames(&self, want: u32) -> Vec<&XImage> {
        let best = self.images.iter().map(|i| i.nominal).min_by_key(|&n| (n.abs_diff(want), u32::MAX - n));
        self.images.iter().filter(|i| Some(i.nominal) == best).collect()
    }
}

/// The nominal size for an output's scale: 24 at 100 %, 48 at 200 %.
pub fn nominal_for(s: Scale) -> u32 {
    (NOMINAL * s.as_120ths() + UNIT / 2) / UNIT
}

/// The pictures of the named looks that a theme has.
#[derive(Clone, Debug, Default)]
pub struct CursorTheme {
    cursors: Vec<(NamedCursor, XCursor)>,
}

impl CursorTheme {
    /// The file of a look in an X11 cursor theme's `cursors` directory (the names themes use:
    /// `left_ptr`, `hand2`, `xterm`, ...).
    pub fn file_name(n: NamedCursor) -> &'static str {
        match n {
            NamedCursor::Arrow => "left_ptr",
            NamedCursor::Progress => "left_ptr_watch",
            NamedCursor::Hand => "hand2",
            NamedCursor::Text => "xterm",
            NamedCursor::Wait => "wait",
            NamedCursor::Crosshair => "crosshair",
            NamedCursor::Move => "move",
            NamedCursor::NotAllowed => "circle",
            NamedCursor::ResizeEW => "sb_h_double_arrow",
            NamedCursor::ResizeNS => "sb_v_double_arrow",
            NamedCursor::ResizeNWSE => "bd_double_arrow",
            NamedCursor::ResizeNESW => "fd_double_arrow",
        }
    }

    /// Give look `n` the pictures of `c`.
    pub fn insert(&mut self, n: NamedCursor, c: XCursor) {
        self.cursors.retain(|(m, _)| *m != n);
        self.cursors.push((n, c));
    }

    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }

    /// How many frames look `n` has at scale `s` (0: the theme has no picture for it).
    pub fn frame_count(&self, n: NamedCursor, s: Scale) -> usize {
        self.cursors.iter().find(|(m, _)| *m == n).map_or(0, |(_, c)| c.frames(nominal_for(s)).len())
    }

    /// The picture of look `n` for scale `s`, frame `frame` of an animation (wrapping).
    pub fn image(&self, n: NamedCursor, s: Scale, frame: usize) -> Option<&XImage> {
        let (_, c) = self.cursors.iter().find(|(m, _)| *m == n)?;
        let frames = c.frames(nominal_for(s));
        frames.get(frame % frames.len().max(1)).copied()
    }
}

/// The logical rectangle that holds `img` drawn with its hot spot at logical (`x`, `y`) at
/// scale `s`: a little larger than the picture, so that rounding never leaves a pixel outside.
pub fn logical_rect(img: &XImage, x: i32, y: i32, s: Scale) -> super::Rect {
    let unit = s.as_120ths() as i64;
    let to_logical_up = |p: u32| ((p as i64 * UNIT as i64 + unit - 1) / unit) as i32;
    let (hx, hy) = (to_logical_up(img.hot.0) + 1, to_logical_up(img.hot.1) + 1);
    super::Rect::new(x - hx, y - hy, to_logical_up(img.width) + 2, to_logical_up(img.height) + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// An XCursor file with the images `(nominal, w, h, hx, hy, colour)`.
    pub(crate) fn file(images: &[(u32, u32, u32, u32, u32, u32)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        for v in [16u32, 0x1_0000, images.len() as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let mut pos = 16 + 12 * images.len() as u32;
        for &(nominal, w, h, ..) in images {
            for v in [IMAGE, nominal, pos] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            pos += 36 + 4 * w * h;
        }
        for &(nominal, w, h, hx, hy, colour) in images {
            for v in [36, IMAGE, nominal, 1, w, h, hx, hy, 40] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            for _ in 0..w * h {
                b.extend_from_slice(&colour.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn reads_images_and_picks_the_size_for_the_scale() {
        let f = file(&[
            (24, 24, 24, 5, 1, 0xff00_0000),
            (32, 32, 32, 6, 2, 1),
            (48, 48, 48, 10, 3, 2),
            (48, 48, 48, 10, 3, 3),
        ]);
        let c = XCursor::parse(&f).unwrap();
        assert_eq!(c.images.len(), 4);
        assert_eq!(c.images[0].hot, (5, 1));
        assert_eq!(c.images[0].pixels.len(), 24 * 24);
        assert_eq!(c.frames(24).len(), 1);
        assert_eq!(c.frames(48).len(), 2);
        assert_eq!(c.frames(36)[0].nominal, 32);
        assert_eq!(c.frames(40)[0].nominal, 48);
        assert_eq!(nominal_for(Scale::ONE), 24);
        assert_eq!(nominal_for(Scale::from_120ths(240)), 48);
        assert_eq!(nominal_for(Scale::from_120ths(180)), 36);
        let mut t = CursorTheme::default();
        t.insert(NamedCursor::Hand, c);
        let two = Scale::from_120ths(240);
        assert_eq!(t.image(NamedCursor::Hand, two, 0).unwrap().pixels[0], 2);
        assert_eq!(t.image(NamedCursor::Hand, two, 3).unwrap().pixels[0], 3);
        assert_eq!(t.image(NamedCursor::Text, two, 0), None);
    }

    #[test]
    fn junk_is_refused() {
        assert_eq!(XCursor::parse(b""), None);
        assert_eq!(XCursor::parse(b"Xcur"), None);
        let mut f = file(&[(24, 4, 4, 1, 1, 0)]);
        f.truncate(f.len() - 3);
        assert_eq!(XCursor::parse(&f), None);
        assert_eq!(XCursor::parse(&file(&[(24, 0, 4, 0, 0, 0)])), None);
        // A hot spot outside the picture is kept inside it.
        let c = XCursor::parse(&file(&[(24, 4, 4, 9, 9, 0)])).unwrap();
        assert_eq!(c.images[0].hot, (3, 3));
    }

    #[test]
    fn the_rectangle_holds_the_picture() {
        let img = XImage { nominal: 48, width: 48, height: 48, hot: (21, 3), delay: 0, pixels: vec![0; 48 * 48] };
        for s in [Scale::ONE, Scale::from_120ths(150), Scale::from_120ths(180), Scale::from_120ths(240)] {
            let r = s.rect(logical_rect(&img, 100, 50, s));
            let (ox, oy) = (s.px(100) - 21, s.px(50) - 3);
            assert!(r.x <= ox && r.y <= oy && r.right() >= ox + 48 && r.bottom() >= oy + 48, "{s:?} {r:?}");
        }
    }

    #[test]
    fn every_look_has_a_file() {
        let looks = [
            NamedCursor::Arrow,
            NamedCursor::Progress,
            NamedCursor::Hand,
            NamedCursor::Text,
            NamedCursor::Wait,
            NamedCursor::Crosshair,
            NamedCursor::Move,
            NamedCursor::NotAllowed,
            NamedCursor::ResizeEW,
            NamedCursor::ResizeNS,
            NamedCursor::ResizeNWSE,
            NamedCursor::ResizeNESW,
        ];
        let mut names: Vec<&str> = looks.iter().map(|&n| CursorTheme::file_name(n)).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), looks.len());
    }
}
