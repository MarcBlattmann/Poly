//! Icon, cursor and bitmap files (ADR 0172): `.ico` and `.cur` directories
//! (`ICONDIR`, `ICONDIRENTRY`, the BMP images inside, `LoadImageW` with
//! `LR_LOADFROMFILE`) and the scaling `LoadImageW` / `CopyImage` do. Layouts
//! from the Learn pages "Icon File Format"-level documentation of
//! `LoadImage` and `wingdi.h`'s `BITMAPINFOHEADER`; hotspots of a cursor
//! file are in the directory entry's planes and bit-count fields.

use crate::dib;
use alloc::vec::Vec;

/// One image of an icon or cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub w: i32,
    pub h: i32,
    /// Top-down `B, G, R, A` rows.
    pub bgra: Vec<u8>,
    /// One byte per pixel, 1 where the AND mask is set (transparent).
    pub and: Vec<u8>,
    /// Whether the colour data carries an alpha channel.
    pub alpha: bool,
    /// The directory's planes and bit-count fields (the hotspot of a cursor).
    pub hot: (i32, i32),
}

fn rd16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn rd32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The images of an `.ico` (type 1) or `.cur` (type 2) file and its type.
pub fn parse(bytes: &[u8]) -> Option<(u16, Vec<Image>)> {
    if rd16(bytes, 0)? != 0 {
        return None;
    }
    let kind = rd16(bytes, 2)?;
    if kind != 1 && kind != 2 {
        return None;
    }
    let count = usize::from(rd16(bytes, 4)?);
    let mut out = Vec::new();
    for k in 0..count {
        let e = 6 + 16 * k;
        let hot = (i32::from(rd16(bytes, e + 4)?), i32::from(rd16(bytes, e + 6)?));
        let size = rd32(bytes, e + 8)? as usize;
        let off = rd32(bytes, e + 12)? as usize;
        let data = bytes.get(off..off.checked_add(size)?)?;
        out.push(image(data, hot)?);
    }
    (!out.is_empty()).then_some((kind, out))
}

/// One BMP image of a directory (XOR image, then the AND mask; the height
/// in the header counts both).
fn image(data: &[u8], hot: (i32, i32)) -> Option<Image> {
    let mut d = data.to_vec();
    let info = dib::info(&d)?;
    let half = (info.height / 2).abs().max(1);
    d[8..12].copy_from_slice(&half.to_le_bytes());
    let (w, h, bgra) = dib::to_bgra(&d)?;
    let info = dib::info(&d)?;
    let xor_len = info.stride() * info.rows();
    let and_at = info.bits_at + xor_len;
    let and_stride = (w as usize).div_ceil(32) * 4;
    let mut and = alloc::vec![0u8; (w * h) as usize];
    if data.len() >= and_at + and_stride * h as usize {
        for y in 0..h as usize {
            let src_row = if info.height > 0 { h as usize - 1 - y } else { y };
            let row = &data[and_at + src_row * and_stride..];
            for x in 0..w as usize {
                and[y * w as usize + x] = (row[x / 8] >> (7 - x % 8)) & 1;
            }
        }
    }
    let alpha = info.bpp == 32 && bgra.chunks_exact(4).any(|p| p[3] != 0);
    Some(Image { w, h, bgra, and, alpha, hot })
}

/// The image of the directory closest to `want` (width, height; 0 for the
/// first).
pub fn pick(images: &[Image], want: (i32, i32)) -> &Image {
    if want.0 <= 0 {
        return &images[0];
    }
    images.iter().min_by_key(|i| (i.w - want.0).abs() + (i.h - want.1).abs()).unwrap_or(&images[0])
}

/// A copy of `img` scaled (nearest neighbour) to `w` x `h`; the hotspot
/// scales with it.
pub fn scale(img: &Image, w: i32, h: i32) -> Image {
    if (w, h) == (img.w, img.h) || w <= 0 || h <= 0 {
        return img.clone();
    }
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    let mut and = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let sy = (i64::from(y) * i64::from(img.h) / i64::from(h)) as usize;
        for x in 0..w {
            let sx = (i64::from(x) * i64::from(img.w) / i64::from(w)) as usize;
            let at = sy * img.w as usize + sx;
            bgra.extend_from_slice(&img.bgra[at * 4..at * 4 + 4]);
            and.push(img.and[at]);
        }
    }
    let sc = |v: i32, from: i32, to: i32| (i64::from(v) * i64::from(to) / i64::from(from.max(1))) as i32;
    Image { w, h, bgra, and, alpha: img.alpha, hot: (sc(img.hot.0, img.w, w), sc(img.hot.1, img.h, h)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ico(kind: u16, w: u8, hot: (u16, u16)) -> Vec<u8> {
        let n = usize::from(w);
        let mut img = Vec::new();
        img.extend_from_slice(&40u32.to_le_bytes());
        img.extend_from_slice(&u32::from(w).to_le_bytes());
        img.extend_from_slice(&(2 * u32::from(w)).to_le_bytes());
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        img.extend_from_slice(&[0; 24]);
        // XOR image: the left half opaque red, the right transparent.
        for _ in 0..n {
            for x in 0..n {
                img.extend_from_slice(&if x < n / 2 { [0, 0, 255, 255] } else { [0, 0, 0, 0] });
            }
        }
        let stride = n.div_ceil(32) * 4;
        img.extend(core::iter::repeat_n(0u8, stride * n));
        let mut f = Vec::new();
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&kind.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&[w, w, 0, 0]);
        f.extend_from_slice(&hot.0.to_le_bytes());
        f.extend_from_slice(&hot.1.to_le_bytes());
        f.extend_from_slice(&(img.len() as u32).to_le_bytes());
        f.extend_from_slice(&22u32.to_le_bytes());
        f.extend_from_slice(&img);
        f
    }

    #[test]
    fn parses_and_scales_a_cursor() {
        let f = ico(2, 16, (3, 4));
        let (kind, imgs) = parse(&f).unwrap();
        assert_eq!(kind, 2);
        let i = &imgs[0];
        assert_eq!((i.w, i.h, i.hot, i.alpha), (16, 16, (3, 4), true));
        assert_eq!(i.bgra[..4], [0, 0, 255, 255]);
        let big = scale(i, 32, 32);
        assert_eq!((big.w, big.hot), (32, (6, 8)));
        assert_eq!(big.bgra.len(), 32 * 32 * 4);
        let small = scale(&parse(&ico(1, 16, (1, 32))).unwrap().1[0], 8, 8);
        assert_eq!(small.hot, (0, 16));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"BM1234").is_none());
        assert!(parse(&[0, 0, 1, 0, 5, 0]).is_none());
    }
}
