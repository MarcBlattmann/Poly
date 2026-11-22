//! Packed DIBs (`BITMAPINFOHEADER` + colours + pixels) as the clipboard
//! holds them: the conversions between `CF_DIB`, `CF_DIBV5` and the
//! pixels a `CF_BITMAP` is made of (ADR 0172). Layouts: `wingdi.h`
//! (`BITMAPINFOHEADER`, `BITMAPV5HEADER`, `RGBQUAD`); sizes and fields as
//! `SetClipboardData.yaml` `images` observed them.

use alloc::vec::Vec;

pub const BI_RGB: u32 = 0;
pub const BI_BITFIELDS: u32 = 3;
/// `LCS_sRGB` ('sRGB', `wingdi.h`) and `LCS_GM_IMAGES`.
pub const LCS_SRGB: u32 = 0x7352_4742;
pub const LCS_GM_IMAGES: u32 = 4;

const V4: usize = 40;
const V5: usize = 124;

fn rd32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn rd16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

/// What a packed DIB says about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub header: usize,
    pub width: i32,
    /// Negative: top-down.
    pub height: i32,
    pub bpp: u16,
    pub compression: u32,
    pub size_image: u32,
    pub clr_used: u32,
    /// Offset of the pixels.
    pub bits_at: usize,
}

impl Info {
    pub fn rows(&self) -> usize {
        self.height.unsigned_abs() as usize
    }

    pub fn stride(&self) -> usize {
        (self.width.unsigned_abs() as usize * usize::from(self.bpp)).div_ceil(32) * 4
    }
}

/// Read a packed DIB's header (a 40-byte `BITMAPINFOHEADER` or a larger
/// V4/V5 header).
pub fn info(b: &[u8]) -> Option<Info> {
    let header = rd32(b, 0)? as usize;
    if header < V4 || b.len() < header {
        return None;
    }
    let width = rd32(b, 4)? as i32;
    let height = rd32(b, 8)? as i32;
    let bpp = rd16(b, 14)?;
    let compression = rd32(b, 16)?;
    let size_image = rd32(b, 20)?;
    let clr_used = rd32(b, 32)?;
    if width == 0 || height == 0 || !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return None;
    }
    // The masks of BI_BITFIELDS follow a 40-byte header (12 bytes; 16 for
    // BI_ALPHABITFIELDS) and are in the V4/V5 header's fields otherwise.
    let masks = if header == V4 && compression == BI_BITFIELDS { 12 } else { 0 };
    let colors = if bpp <= 8 {
        (if clr_used == 0 { 1usize << bpp } else { clr_used as usize }) * 4
    } else if header == V4 && compression != BI_BITFIELDS {
        (clr_used as usize) * 4
    } else {
        0
    };
    let bits_at = header + masks + colors;
    (bits_at <= b.len()).then_some(Info { header, width, height, bpp, compression, size_image, clr_used, bits_at })
}

/// The `CF_DIBV5` of a packed DIB: the same header fields in a
/// `BITMAPV5HEADER` (masks of a BI_BITFIELDS image in the header and kept
/// after it, as observed), then the colours and pixels.
pub fn to_v5(dib: &[u8]) -> Option<Vec<u8>> {
    let i = info(dib)?;
    if i.header >= V5 {
        return Some(dib.to_vec());
    }
    let mut out = Vec::with_capacity(dib.len() + V5 - i.header);
    let mut h = dib[..V4].to_vec();
    h[0..4].copy_from_slice(&(V5 as u32).to_le_bytes());
    out.extend_from_slice(&h);
    // Masks: a BI_BITFIELDS image has them after the 40-byte header.
    let masks = if i.compression == BI_BITFIELDS { dib.get(V4..V4 + 12) } else { None };
    for k in 0..4 {
        let m = match (masks, k) {
            (Some(m), 0..=2) => u32::from_le_bytes([m[k * 4], m[k * 4 + 1], m[k * 4 + 2], m[k * 4 + 3]]),
            _ => 0,
        };
        out.extend_from_slice(&m.to_le_bytes());
    }
    out.extend_from_slice(&LCS_SRGB.to_le_bytes());
    out.extend_from_slice(&[0u8; 36 + 12]);
    out.extend_from_slice(&LCS_GM_IMAGES.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    debug_assert_eq!(out.len(), V5);
    out.extend_from_slice(&dib[i.header..]);
    Some(out)
}

/// The `CF_DIB` of a `CF_DIBV5`: a 40-byte header, the masks after it for
/// a BI_BITFIELDS image.
pub fn from_v5(v5: &[u8]) -> Option<Vec<u8>> {
    let i = info(v5)?;
    if i.header < V5 {
        return Some(v5.to_vec());
    }
    let mut out = v5[..V4].to_vec();
    out[0..4].copy_from_slice(&(V4 as u32).to_le_bytes());
    if i.compression == BI_BITFIELDS && v5.len() >= i.header + 12 && rd32(v5, i.header) == rd32(v5, 40) {
        // The masks were kept after the header: they are already in place.
        out.extend_from_slice(&v5[i.header..]);
    } else {
        if i.compression == BI_BITFIELDS {
            out.extend_from_slice(&v5[40..52]);
        }
        out.extend_from_slice(&v5[i.header..]);
    }
    Some(out)
}

/// A bottom-up 32-bit `BI_BITFIELDS` DIB (header, red/green/blue masks of
/// B, G, R order, pixels) from top-down `B, G, R, A` rows.
pub fn make32(width: i32, height: i32, bgra_top_down: &[u8]) -> Vec<u8> {
    let (w, h) = (width.max(0) as usize, height.max(0) as usize);
    let mut out = Vec::with_capacity(V4 + 12 + w * h * 4);
    out.extend_from_slice(&(V4 as u32).to_le_bytes());
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(height as u32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&BI_BITFIELDS.to_le_bytes());
    out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for m in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF] {
        out.extend_from_slice(&m.to_le_bytes());
    }
    for row in (0..h).rev() {
        out.extend_from_slice(bgra_top_down.get(row * w * 4..(row + 1) * w * 4).unwrap_or(&[]));
    }
    out
}

/// Top-down `B, G, R, A` rows of a packed DIB (1, 4, 8, 16, 24 and 32
/// bits; BI_RGB and the usual BI_BITFIELDS layouts), with its size.
pub fn to_bgra(b: &[u8]) -> Option<(i32, i32, Vec<u8>)> {
    let i = info(b)?;
    let (w, h) = (i.width.unsigned_abs() as usize, i.rows());
    if w == 0 || h == 0 || w > 1 << 15 || h > 1 << 15 {
        return None;
    }
    let stride = i.stride();
    let bits = b.get(i.bits_at..)?;
    let palette: Vec<[u8; 4]> = if i.bpp <= 8 {
        let n = if i.clr_used == 0 { 1usize << i.bpp } else { i.clr_used as usize };
        let p = i.header;
        (0..n).filter_map(|k| b.get(p + k * 4..p + k * 4 + 4).map(|c| [c[0], c[1], c[2], 255])).collect()
    } else {
        Vec::new()
    };
    let masks: [u32; 3] = if i.compression == BI_BITFIELDS {
        let at = if i.header >= V5 { 40 } else { V4 };
        [rd32(b, at)?, rd32(b, at + 4)?, rd32(b, at + 8)?]
    } else if i.bpp == 16 {
        [0x7C00, 0x03E0, 0x001F]
    } else {
        [0x00FF_0000, 0x0000_FF00, 0x0000_00FF]
    };
    let scale = |v: u32, m: u32| -> u8 {
        if m == 0 {
            return 0;
        }
        let sh = m.trailing_zeros();
        let max = m >> sh;
        let x = (v & m) >> sh;
        ((x * 255 + max / 2) / max) as u8
    };
    let mut out = alloc::vec![0u8; w * h * 4];
    for y in 0..h {
        // Bottom-up unless the height is negative.
        let src_row = if i.height > 0 { h - 1 - y } else { y };
        let row = bits.get(src_row * stride..)?;
        for x in 0..w {
            let px: [u8; 4] = match i.bpp {
                32 => {
                    let v = rd32(row, x * 4)?;
                    // A plain 32-bit image carries its alpha in the fourth byte.
                    let a = if i.compression == BI_RGB { (v >> 24) as u8 } else { 255 };
                    [scale(v, masks[2]), scale(v, masks[1]), scale(v, masks[0]), a]
                }
                24 => {
                    let p = row.get(x * 3..x * 3 + 3)?;
                    [p[0], p[1], p[2], 255]
                }
                16 => {
                    let v = u32::from(rd16(row, x * 2)?);
                    [scale(v, masks[2]), scale(v, masks[1]), scale(v, masks[0]), 255]
                }
                8 => *palette.get(usize::from(*row.get(x)?))?,
                4 => {
                    let v = *row.get(x / 2)?;
                    *palette.get(usize::from(if x % 2 == 0 { v >> 4 } else { v & 15 }))?
                }
                _ => {
                    let v = *row.get(x / 8)?;
                    *palette.get(usize::from((v >> (7 - x % 8)) & 1))?
                }
            };
            out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    Some((w as i32, h as i32, out))
}

/// The 14-byte `BITMAPFILEHEADER` size (`wingdi.h`).
const BMP_FILE_HEADER: usize = 14;

/// Upper bound on pixels accepted when decoding a PNG for clipboard bridging
/// (4M pixels = 16 MiB at 32 bpp).
const MAX_CLIPBOARD_PIXELS: u64 = 4_000_000;

/// Wrap a packed DIB (`CF_DIB` / `CF_DIBV5`) in a 14-byte `BITMAPFILEHEADER`
/// to produce a complete BMP file (`image/bmp`).
pub fn to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let i = info(dib)?;
    let total = u32::try_from(BMP_FILE_HEADER.checked_add(dib.len())?).ok()?;
    let off = u32::try_from(BMP_FILE_HEADER.checked_add(i.bits_at)?).ok()?;
    let mut out = Vec::with_capacity(BMP_FILE_HEADER + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&off.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// Extract a packed `CF_DIB` (40-byte `BITMAPINFOHEADER` + masks/palette +
/// pixels) from a BMP file (`image/bmp`).
pub fn from_bmp(bmp: &[u8]) -> Option<Vec<u8>> {
    if bmp.len() < BMP_FILE_HEADER + V4 || &bmp[0..2] != b"BM" {
        return None;
    }
    let bf_off = rd32(bmp, 10)? as usize;
    let dib = &bmp[BMP_FILE_HEADER..];
    let i = info(dib)?;
    let packed = if bf_off > BMP_FILE_HEADER + i.bits_at && bf_off <= bmp.len() {
        let mut p = Vec::with_capacity(i.bits_at + (bmp.len() - bf_off));
        p.extend_from_slice(&dib[..i.bits_at]);
        p.extend_from_slice(&bmp[bf_off..]);
        p
    } else {
        dib.to_vec()
    };
    if i.header >= V5 { from_v5(&packed) } else { Some(packed) }
}

/// Encode a packed DIB (`CF_DIB` / `CF_DIBV5`) as an 8-bit RGBA PNG file
/// (`image/png`), using stored DEFLATE blocks (W3C PNG 3rd ed., RFC 1950/1951).
pub fn to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let i = info(dib)?;
    let (w, h, mut bgra) = to_bgra(dib)?;
    if w <= 0 || h <= 0 {
        return None;
    }
    // Many Windows apps place 32-bpp BI_RGB DIBs with zeroed alpha bytes
    // (XRGB layout); treat an all-zero alpha channel as opaque so Linux apps
    // do not display a completely transparent image.
    if i.bpp == 32 && i.compression == BI_RGB && bgra.chunks_exact(4).all(|p| p[3] == 0) {
        for p in bgra.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    let (w, h) = (w as u32, h as u32);
    let row_bytes = (w as usize).checked_mul(4)?;
    let raw_len = (h as usize).checked_mul(row_bytes.checked_add(1)?)?;
    let mut raw = Vec::with_capacity(raw_len);
    for row in bgra.chunks_exact(row_bytes) {
        raw.push(0); // filter type 0 (None)
        for px in row.chunks_exact(4) {
            raw.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    let mut z = Vec::with_capacity(raw.len() + (raw.len() / 0xFFFF + 1) * 5 + 6);
    // Zlib header: CM=8 (DEFLATE), CINFO=7 (32K window), FCHECK makes header % 31 == 0.
    z.extend_from_slice(&[0x78, 0x01]);
    let blocks: Vec<&[u8]> = raw.chunks(0xFFFF).collect();
    for (idx, block) in blocks.iter().enumerate() {
        z.push(u8::from(idx + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    if blocks.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    z.extend_from_slice(&poly_png::inflate::adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(poly_png::SIGNATURE.len() + 25 + z.len() + 24);
    out.extend_from_slice(&poly_png::SIGNATURE);
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&poly_png::crc32(&[kind, data]).to_be_bytes());
    };
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8..13].copy_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    Some(out)
}

/// Decode a PNG file (`image/png`) into a bottom-up 32-bit `BI_RGB` packed
/// `CF_DIB` (40-byte `BITMAPINFOHEADER` + `B, G, R, A` rows), preserving alpha.
pub fn from_png(png: &[u8]) -> Option<Vec<u8>> {
    let img = poly_png::decode_with(png, &poly_png::Limits { max_pixels: MAX_CLIPBOARD_PIXELS }).ok()?;
    let (w, h) = (i32::try_from(img.width).ok()?, i32::try_from(img.height).ok()?);
    if w <= 0 || h <= 0 {
        return None;
    }
    let pixel_bytes = (img.width as usize).checked_mul(img.height as usize)?.checked_mul(4)?;
    let size_image = u32::try_from(pixel_bytes).ok()?;
    let mut out = Vec::with_capacity(V4 + pixel_bytes);
    out.extend_from_slice(&(V4 as u32).to_le_bytes());
    out.extend_from_slice(&(w as u32).to_le_bytes());
    out.extend_from_slice(&(h as u32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&BI_RGB.to_le_bytes());
    out.extend_from_slice(&size_image.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    let wu = img.width as usize;
    for y in (0..img.height as usize).rev() {
        for &argb in &img.pixels[y * wu..(y + 1) * wu] {
            out.extend_from_slice(&[argb as u8, (argb >> 8) as u8, (argb >> 16) as u8, (argb >> 24) as u8]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn sizes_match_the_observed_clipboard() {
        // A 4x3 32-bit bitmap: CF_DIB 100 bytes, CF_DIBV5 184 (SetClipboardData.yaml images).
        let dib = make32(4, 3, &[0; 48]);
        assert_eq!(dib.len(), 100);
        let v5 = to_v5(&dib).unwrap();
        assert_eq!(v5.len(), 184);
        assert_eq!(rd32(&v5, 0), Some(124));
        assert_eq!(from_v5(&v5).unwrap(), dib);
        // A 24-bit 2x2 DIB: 56 bytes, V5 140.
        let mut d24 = vec![0u8; 56];
        d24[0..4].copy_from_slice(&40u32.to_le_bytes());
        d24[4..8].copy_from_slice(&2u32.to_le_bytes());
        d24[8..12].copy_from_slice(&2u32.to_le_bytes());
        d24[12..14].copy_from_slice(&1u16.to_le_bytes());
        d24[14..16].copy_from_slice(&24u16.to_le_bytes());
        d24[20..24].copy_from_slice(&16u32.to_le_bytes());
        assert_eq!(to_v5(&d24).unwrap().len(), 140);
        let (w, h, px) = to_bgra(&d24).unwrap();
        assert_eq!((w, h, px.len()), (2, 2, 16));
    }

    #[test]
    fn pixels_round_trip_bottom_up() {
        let px: Vec<u8> = (0..16).collect();
        let dib = make32(2, 2, &px);
        let (_, _, back) = to_bgra(&dib).unwrap();
        assert_eq!(&back[..4], &[0, 1, 2, 255]);
        assert_eq!(&back[12..], &[12, 13, 14, 255]);
    }

    #[test]
    fn bmp_and_dib_round_trip() {
        let px = [0x10u8, 0x20, 0x30, 0xFF, 0x40, 0x50, 0x60, 0xFF, 0x70, 0x80, 0x90, 0xFF, 0xA0, 0xB0, 0xC0, 0xFF];
        let dib = make32(2, 2, &px);
        let bmp = to_bmp(&dib).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(rd32(&bmp, 2), Some((14 + dib.len()) as u32));
        assert_eq!(rd32(&bmp, 10), Some((14 + 40 + 12) as u32));
        assert_eq!(from_bmp(&bmp).unwrap(), dib);

        // V5-header BMP normalizes to 40-byte CF_DIB.
        let v5 = to_v5(&dib).unwrap();
        let v5_bmp = to_bmp(&v5).unwrap();
        assert_eq!(from_bmp(&v5_bmp).unwrap(), dib);

        // BMP with extra padding before pixel bits is compacted into packed DIB.
        let mut padded = bmp[..14 + 52].to_vec();
        padded.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        padded.extend_from_slice(&bmp[14 + 52..]);
        let new_len = padded.len() as u32;
        padded[2..6].copy_from_slice(&new_len.to_le_bytes());
        padded[10..14].copy_from_slice(&((14 + 52 + 4) as u32).to_le_bytes());
        assert_eq!(from_bmp(&padded).unwrap(), dib);

        // Invalid inputs rejected cleanly.
        assert!(from_bmp(b"NO").is_none());
        assert!(from_bmp(b"BMshort").is_none());
        assert!(to_bmp(&[0u8; 10]).is_none());
    }

    #[test]
    fn png_and_dib_round_trip() {
        // Opaque 2x2 BI_BITFIELDS DIB -> PNG -> DIB -> BGRA.
        let px = [0x11u8, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF, 0x77, 0x88, 0x99, 0xFF, 0xAA, 0xBB, 0xCC, 0xFF];
        let dib = make32(2, 2, &px);
        let png = to_png(&dib).unwrap();
        assert!(png.starts_with(&poly_png::SIGNATURE));
        let back_dib = from_png(&png).unwrap();
        let (w, h, back_px) = to_bgra(&back_dib).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(back_px, px);

        // Semi-transparent 32-bpp BI_RGB DIB preserves alpha through PNG round-trip.
        let mut argb_dib = vec![0u8; 40 + 8];
        argb_dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        argb_dib[4..8].copy_from_slice(&2u32.to_le_bytes());
        argb_dib[8..12].copy_from_slice(&1u32.to_le_bytes());
        argb_dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        argb_dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        argb_dib[40..44].copy_from_slice(&[0x10, 0x20, 0x30, 0x80]);
        argb_dib[44..48].copy_from_slice(&[0x40, 0x50, 0x60, 0xFF]);
        let png = to_png(&argb_dib).unwrap();
        let decoded = poly_png::decode(&png).unwrap();
        assert_eq!(decoded.pixels, [0x8030_2010, 0xFF60_5040]);
        let (_, _, back) = to_bgra(&from_png(&png).unwrap()).unwrap();
        assert_eq!(back, [0x10, 0x20, 0x30, 0x80, 0x40, 0x50, 0x60, 0xFF]);

        // 32-bpp BI_RGB with all-zero alpha (XRGB) becomes opaque in PNG.
        let mut xrgb_dib = argb_dib.clone();
        xrgb_dib[43] = 0;
        xrgb_dib[47] = 0;
        let decoded = poly_png::decode(&to_png(&xrgb_dib).unwrap()).unwrap();
        assert_eq!(decoded.pixels, [0xFF30_2010, 0xFF60_5040]);

        // Multi-DEFLATE-block PNG (> 65535 raw bytes): 200x100 = 80100 raw bytes.
        let big = make32(200, 100, &vec![0x5Au8; 200 * 100 * 4]);
        let big_png = to_png(&big).unwrap();
        let big_back = from_png(&big_png).unwrap();
        let (bw, bh, bpx) = to_bgra(&big_back).unwrap();
        assert_eq!((bw, bh, bpx.len()), (200, 100, 200 * 100 * 4));
        assert_eq!(&bpx[..4], &[0x5A, 0x5A, 0x5A, 0xFF]);

        // Invalid PNG rejected cleanly.
        assert!(from_png(b"not a png").is_none());
    }
}
