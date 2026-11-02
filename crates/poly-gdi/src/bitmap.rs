//! Bitmaps and their pixel formats (`CreateDIBSection.yaml`, `GetPixel.yaml`,
//! `SetPixel.yaml`, `BitBlt.yaml`): 1, 4, 8, 16 (5-5-5), 24 and 32 bits per
//! pixel; device-independent bitmaps top-down or bottom-up with their
//! colour table; monochrome device-dependent bitmaps (only white is 1).
//!
//! A bitmap's pixels are memory the caller can address (a DIB section's
//! bits) or memory the bitmap owns; either way [`Bitmap`] works through a
//! raw pointer it was given.

use alloc::vec::Vec;

/// An RGB colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb { r: 255, g: 255, b: 255 };

    /// `COLORREF` `0x00BBGGRR` (a value with high byte 1 is an index into
    /// the default palette).
    pub fn from_colorref(c: u32) -> Rgb {
        if c >> 24 == 1 {
            let (r, g, b) = DEFAULT_PALETTE.get((c & 0xFFFF) as usize).copied().unwrap_or((0, 0, 0));
            return Rgb { r, g, b };
        }
        Rgb { r: c as u8, g: (c >> 8) as u8, b: (c >> 16) as u8 }
    }

    pub fn colorref(self) -> u32 {
        u32::from(self.r) | (u32::from(self.g) << 8) | (u32::from(self.b) << 16)
    }

    /// An `RGBQUAD` / 32-bit DIB value `0x00RRGGBB`.
    pub fn from_quad(q: u32) -> Rgb {
        Rgb { r: (q >> 16) as u8, g: (q >> 8) as u8, b: q as u8 }
    }

    pub fn quad(self) -> u32 {
        (u32::from(self.r) << 16) | (u32::from(self.g) << 8) | u32::from(self.b)
    }

    fn distance(self, o: Rgb) -> u32 {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).unsigned_abs();
        d(self.r, o.r).pow(2) + d(self.g, o.g).pow(2) + d(self.b, o.b).pow(2)
    }
}

/// The default palette's 20 colours (`SetPixel.yaml`: entry 2 is
/// 0x008000).
pub const DEFAULT_PALETTE: [(u8, u8, u8); 20] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (192, 220, 192),
    (166, 202, 240),
    (255, 251, 240),
    (160, 160, 164),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// Where a bitmap's pixels live.
#[derive(Debug)]
pub enum Storage {
    /// Owned by the bitmap.
    Owned(Vec<u8>),
    /// Memory the caller sees (a DIB section): address and length.
    External(u64, usize),
}

/// A bitmap.
#[derive(Debug)]
pub struct Bitmap {
    pub width: i32,
    pub height: i32,
    pub bpp: u16,
    /// Bytes per row in memory.
    pub stride: usize,
    /// Logical row 0 first in memory.
    pub top_down: bool,
    /// A device-independent bitmap (with `colors`) rather than a
    /// device-dependent one.
    pub dib: bool,
    /// The colour table (`0x00RRGGBB`).
    pub colors: Vec<u32>,
    pub storage: Storage,
}

impl Bitmap {
    /// A device-dependent bitmap of `bpp` 1 (monochrome) or 32, zeroed.
    pub fn ddb(width: i32, height: i32, bpp: u16) -> Bitmap {
        let bpp = if bpp == 1 { 1 } else { 32 };
        let stride = Self::dib_stride(width, bpp);
        Bitmap {
            width,
            height,
            bpp,
            stride,
            top_down: true,
            dib: false,
            colors: Vec::new(),
            storage: Storage::Owned(alloc::vec![0; stride * height.max(0) as usize]),
        }
    }

    /// Bytes per row of a DIB: rounded up to 4 bytes.
    pub fn dib_stride(width: i32, bpp: u16) -> usize {
        ((width.max(0) as usize * usize::from(bpp)).div_ceil(32)) * 4
    }

    /// `BITMAP.bmWidthBytes`: DIBs round to 4 bytes, device-dependent
    /// bitmaps to 2 (`CreateCompatibleDC.yaml`).
    pub fn width_bytes(&self) -> usize {
        if self.dib { self.stride } else { (self.width.max(0) as usize * usize::from(self.bpp)).div_ceil(16) * 2 }
    }

    /// The address of the pixels (`bmBits` of a DIB section).
    pub fn bits_address(&self) -> u64 {
        match &self.storage {
            Storage::Owned(v) => v.as_ptr() as u64,
            Storage::External(a, _) => *a,
        }
    }

    fn base(&self) -> *mut u8 {
        match &self.storage {
            Storage::Owned(v) => v.as_ptr() as *mut u8,
            Storage::External(a, _) => *a as *mut u8,
        }
    }

    fn row_offset(&self, y: i32) -> usize {
        let row = if self.top_down { y } else { self.height - 1 - y };
        row as usize * self.stride
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    /// The raw value of a pixel (inside the bitmap).
    pub fn raw(&self, x: i32, y: i32) -> u32 {
        let off = self.row_offset(y);
        let p = self.base();
        let x = x as usize;
        // SAFETY: (x, y) is inside the bitmap, whose memory holds
        // `stride * height` bytes.
        unsafe {
            match self.bpp {
                32 => (p.add(off + x * 4) as *const u32).read_unaligned(),
                24 => {
                    let q = p.add(off + x * 3);
                    u32::from(*q) | (u32::from(*q.add(1)) << 8) | (u32::from(*q.add(2)) << 16)
                }
                16 => u32::from((p.add(off + x * 2) as *const u16).read_unaligned()),
                8 => u32::from(*p.add(off + x)),
                4 => u32::from(*p.add(off + x / 2) >> (if x.is_multiple_of(2) { 4 } else { 0 })) & 0xF,
                _ => u32::from(*p.add(off + x / 8) >> (7 - x % 8)) & 1,
            }
        }
    }

    /// Store a raw value (inside the bitmap).
    pub fn set_raw(&mut self, x: i32, y: i32, v: u32) {
        let off = self.row_offset(y);
        let p = self.base();
        let x = x as usize;
        // SAFETY: as in `raw`.
        unsafe {
            match self.bpp {
                32 => (p.add(off + x * 4) as *mut u32).write_unaligned(v),
                24 => {
                    let q = p.add(off + x * 3);
                    *q = v as u8;
                    *q.add(1) = (v >> 8) as u8;
                    *q.add(2) = (v >> 16) as u8;
                }
                16 => (p.add(off + x * 2) as *mut u16).write_unaligned(v as u16),
                8 => *p.add(off + x) = v as u8,
                4 => {
                    let q = p.add(off + x / 2);
                    let shift = if x.is_multiple_of(2) { 4 } else { 0 };
                    *q = (*q & !(0xF << shift)) | (((v & 0xF) as u8) << shift);
                }
                _ => {
                    let q = p.add(off + x / 8);
                    let bit = 1u8 << (7 - x % 8);
                    if v & 1 != 0 {
                        *q |= bit;
                    } else {
                        *q &= !bit;
                    }
                }
            }
        }
    }

    /// The colour a raw value shows.
    pub fn to_rgb(&self, v: u32) -> Rgb {
        match self.bpp {
            32 | 24 => Rgb::from_quad(v),
            16 => {
                let ex = |c: u32| ((c << 3) | (c >> 2)) as u8;
                Rgb { r: ex((v >> 10) & 31), g: ex((v >> 5) & 31), b: ex(v & 31) }
            }
            _ if !self.dib => {
                if v & 1 != 0 {
                    Rgb::WHITE
                } else {
                    Rgb::BLACK
                }
            }
            _ => self.colors.get(v as usize).map_or(Rgb::BLACK, |&q| Rgb::from_quad(q)),
        }
    }

    /// The raw value nearest to a colour.
    pub fn from_rgb(&self, c: Rgb) -> u32 {
        match self.bpp {
            32 | 24 => c.quad(),
            16 => (u32::from(c.r >> 3) << 10) | (u32::from(c.g >> 3) << 5) | u32::from(c.b >> 3),
            _ if !self.dib => u32::from(c == Rgb::WHITE),
            _ => self
                .colors
                .iter()
                .enumerate()
                .min_by_key(|(_, q)| c.distance(Rgb::from_quad(**q)))
                .map_or(0, |(i, _)| i as u32),
        }
    }

    /// All bits of a raw value.
    pub fn raw_mask(&self) -> u32 {
        if self.bpp >= 32 { u32::MAX } else { (1u32 << self.bpp) - 1 }
    }
}

/// A raster operation's result for pattern `p`, source `s` and destination
/// `d` (the ternary operation's code is the ROP's third byte).
pub fn rop3(code: u8, p: u32, s: u32, d: u32) -> u32 {
    let mut out = 0;
    for k in 0..8u32 {
        if code & (1 << k) == 0 {
            continue;
        }
        let pk = if k & 4 != 0 { p } else { !p };
        let sk = if k & 2 != 0 { s } else { !s };
        let dk = if k & 1 != 0 { d } else { !d };
        out |= pk & sk & dk;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dib(bpp: u16, w: i32, h: i32, colors: &[u32]) -> (Bitmap, Vec<u8>) {
        let stride = Bitmap::dib_stride(w, bpp);
        let mem = alloc::vec![0u8; stride * h as usize];
        let b = Bitmap {
            width: w,
            height: h,
            bpp,
            stride,
            top_down: true,
            dib: true,
            colors: colors.to_vec(),
            storage: Storage::External(mem.as_ptr() as u64, mem.len()),
        };
        (b, mem)
    }

    #[test]
    fn formats_store_and_read_back_like_the_sheets() {
        let (mut b, _m) = dib(16, 2, 1, &[]);
        let c = Rgb::from_colorref(0x0056_3412);
        let v = b.from_rgb(c);
        assert_eq!(v, 0x08ca);
        b.set_raw(0, 0, v);
        assert_eq!(b.to_rgb(b.raw(0, 0)).colorref(), 0x0052_3110);
        assert_eq!(b.to_rgb(0x7fff).colorref(), 0x00ff_ffff);
        assert_eq!(b.to_rgb((1 << 10) | (2 << 5) | 3).colorref(), 0x0018_1008);
        assert_eq!(b.from_rgb(Rgb::from_colorref(0x0008_07ff)), 0x7c01);
        let (b8, _m) = dib(8, 4, 1, &[0, 0xFF0000, 0x00FF00, 0x123456]);
        assert_eq!(b8.to_rgb(3).colorref(), 0x0056_3412);
        assert_eq!(b8.to_rgb(9).colorref(), 0);
        assert_eq!(Bitmap::dib_stride(33, 1), 8);
        assert_eq!(Bitmap::dib_stride(3, 24), 12);
        assert_eq!(Bitmap::ddb(10, 5, 1).width_bytes(), 2);
        assert_eq!(Bitmap::ddb(10, 5, 32).width_bytes(), 40);
        assert_eq!(Rgb::from_colorref(0x0100_0002).quad(), 0x0000_8000);
    }

    #[test]
    fn raster_operations() {
        let (p, s, d) = (0x000F_F055, 0x00F0_F00F, 0x000F_0FF0);
        assert_eq!(rop3(0xCC, p, s, d), s);
        assert_eq!(rop3(0xEE, p, s, d), s | d);
        assert_eq!(rop3(0x88, p, s, d), s & d);
        assert_eq!(rop3(0x66, p, s, d), s ^ d);
        assert_eq!(rop3(0x33, p, s, d), !s);
        assert_eq!(rop3(0x55, p, s, d), !d);
        assert_eq!(rop3(0x00, p, s, d), 0);
        assert_eq!(rop3(0xFF, p, s, d), u32::MAX);
        assert_eq!(rop3(0xF0, p, s, d), p);
    }
}
