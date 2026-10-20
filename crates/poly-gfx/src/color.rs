//! Colours and how pixels store them.

/// A 24-bit sRGB colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color(u32);

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(0xff, 0xff, 0xff);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color((r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    /// From `0xRRGGBB`; the top byte is ignored.
    pub const fn from_rgb24(rgb: u32) -> Color {
        Color(rgb & 0x00ff_ffff)
    }

    /// As `0xRRGGBB`.
    pub const fn rgb24(self) -> u32 {
        self.0
    }

    pub const fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub const fn b(self) -> u8 {
        self.0 as u8
    }

    /// Halfway between `self` and `other`, channel by channel (for faint
    /// text).
    pub const fn mix(self, other: Color) -> Color {
        const fn avg(a: u8, b: u8) -> u8 {
            ((a as u16 + b as u16) / 2) as u8
        }
        Color::rgb(avg(self.r(), other.r()), avg(self.g(), other.g()), avg(self.b(), other.b()))
    }
}

/// How a pixel is laid out in memory. Every format is little-endian, the
/// way x86 framebuffers (and QEMU's) store them; the unused bits of the
/// 32-bit formats are written as zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 32 bits, `0x00RRGGBB`: bytes blue, green, red, unused.
    Xrgb8888,
    /// 32 bits, `0x00BBGGRR`: bytes red, green, blue, unused.
    Xbgr8888,
    /// 24 bits: bytes blue, green, red.
    Rgb888,
    /// 16 bits, `RRRRRGGG GGGBBBBB`.
    Rgb565,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Xrgb8888 | PixelFormat::Xbgr8888 => 4,
            PixelFormat::Rgb888 => 3,
            PixelFormat::Rgb565 => 2,
        }
    }

    /// The bytes of a pixel of colour `c`; only the first
    /// [`bytes_per_pixel`](Self::bytes_per_pixel) are used.
    pub const fn encode(self, c: Color) -> [u8; 4] {
        let (r, g, b) = (c.r(), c.g(), c.b());
        match self {
            PixelFormat::Xrgb8888 => [b, g, r, 0],
            PixelFormat::Xbgr8888 => [r, g, b, 0],
            PixelFormat::Rgb888 => [b, g, r, 0],
            PixelFormat::Rgb565 => {
                let v = ((r as u16) >> 3) << 11 | ((g as u16) >> 2) << 5 | (b as u16) >> 3;
                let [lo, hi] = v.to_le_bytes();
                [lo, hi, 0, 0]
            }
        }
    }

    /// The colour of the pixel stored in `px`, which holds at least
    /// [`bytes_per_pixel`](Self::bytes_per_pixel) bytes. Formats with fewer
    /// than 8 bits per channel scale each channel up to the full range.
    pub fn decode(self, px: &[u8]) -> Color {
        match (self, px) {
            (PixelFormat::Xrgb8888 | PixelFormat::Rgb888, [b, g, r, ..]) => Color::rgb(*r, *g, *b),
            (PixelFormat::Xbgr8888, [r, g, b, ..]) => Color::rgb(*r, *g, *b),
            (PixelFormat::Rgb565, [lo, hi, ..]) => {
                let v = u16::from_le_bytes([*lo, *hi]);
                let (r, g, b) = ((v >> 11) as u8 & 0x1f, (v >> 5) as u8 & 0x3f, v as u8 & 0x1f);
                Color::rgb(r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2)
            }
            _ => Color::BLACK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_round_trip() {
        let c = Color::rgb(0x12, 0x34, 0x56);
        assert_eq!((c.r(), c.g(), c.b(), c.rgb24()), (0x12, 0x34, 0x56, 0x123456));
        assert_eq!(Color::from_rgb24(0xff12_3456), c);
        assert_eq!(Color::BLACK.mix(Color::WHITE), Color::rgb(0x7f, 0x7f, 0x7f));
    }

    #[test]
    fn formats_store_the_documented_bytes() {
        let c = Color::rgb(0x12, 0x34, 0x56);
        assert_eq!(PixelFormat::Xrgb8888.encode(c), [0x56, 0x34, 0x12, 0]);
        assert_eq!(PixelFormat::Xbgr8888.encode(c), [0x12, 0x34, 0x56, 0]);
        assert_eq!(PixelFormat::Rgb888.encode(c)[..3], [0x56, 0x34, 0x12]);
        // 0x12 >> 3 = 2, 0x34 >> 2 = 13, 0x56 >> 3 = 10: 00010 001101 01010.
        assert_eq!(PixelFormat::Rgb565.encode(c)[..2], 0x11aau16.to_le_bytes());
    }

    #[test]
    fn decode_inverts_encode() {
        for f in [PixelFormat::Xrgb8888, PixelFormat::Xbgr8888, PixelFormat::Rgb888] {
            for c in [Color::BLACK, Color::WHITE, Color::rgb(1, 2, 3), Color::rgb(0xfe, 0x80, 0x01)] {
                assert_eq!(f.decode(&f.encode(c)), c, "{f:?}");
            }
        }
        // 565 keeps the top bits and stretches them back to the full range.
        let f = PixelFormat::Rgb565;
        assert_eq!(f.decode(&f.encode(Color::WHITE)), Color::WHITE);
        assert_eq!(f.decode(&f.encode(Color::BLACK)), Color::BLACK);
        assert_eq!(f.decode(&f.encode(Color::rgb(0x84, 0x82, 0x84))), Color::rgb(0x84, 0x82, 0x84));
        assert_eq!(f.decode(&[]), Color::BLACK);
    }
}
