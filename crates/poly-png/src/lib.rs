//! A PNG decoder (ADR 0146), written from the PNG specification (W3C, 3rd
//! edition) and RFC 1950 / RFC 1951 only; no libpng, zlib, miniz or
//! image-rs source was looked at.
//!
//! - [`decode`] turns a PNG file into an [`Image`]: `0xAARRGGBB` pixels
//!   (straight, not premultiplied, alpha), rows top to bottom. Every colour
//!   type (grey, RGB, palette, grey and alpha, RGBA) at every bit depth
//!   (1, 2, 4, 8, 16), palettes with `tRNS` transparency (and the single
//!   transparent colour of grey and RGB images), Adam7 interlacing and all
//!   five scanline filters. Sixteen-bit samples are reduced to eight.
//!   Colour management chunks (`gAMA`, `cHRM`, `sRGB`, `iCCP`) are ignored:
//!   samples are taken as sRGB.
//! - [`info`] reads only the header, to know a file's size and format.
//! - [`inflate`]: DEFLATE and zlib, the decoder's other half, usable on its
//!   own.
//!
//! Damaged files are errors, never panics: the signature, the chunk order,
//! every critical chunk's CRC-32 and the zlib stream's Adler-32 are
//! checked, truncation anywhere is [`Error::Truncated`], and
//! [`Limits`] bound the memory a file may make the decoder use (the pixel
//! count, which also bounds the decompressed size).
//!
//! `no_std` with `alloc`, host-tested, no unsafe code.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod inflate;

use alloc::vec::Vec;
use core::fmt;

pub use inflate::InflateError;

/// The eight bytes every PNG file starts with.
pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// Why a file was not decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a PNG: the signature is missing.
    NotPng,
    /// The data ends inside a chunk or before `IEND`.
    Truncated,
    /// A critical chunk's CRC-32 is wrong.
    BadCrc,
    /// `IHDR` is malformed: its length, a zero or oversized dimension, a
    /// colour type and bit depth that do not go together, a method other
    /// than 0, an interlace method other than 0 or 1.
    BadHeader,
    /// The chunks are in an order or number the specification forbids
    /// (`IHDR` not first, a second `IHDR` or `PLTE`, `IDAT` split by other
    /// chunks, no `IDAT`).
    BadChunkOrder,
    /// A chunk whose length is wrong for its kind (`PLTE`, `tRNS`).
    BadChunk,
    /// An unknown chunk whose name marks it critical.
    UnknownCritical,
    /// A palette image without `PLTE`, or a pixel naming a missing entry.
    BadPalette,
    /// A scanline's filter type is not 0 to 4.
    BadFilter,
    /// The image is larger than [`Limits`] allow.
    TooLarge,
    /// The compressed data is damaged, or its length is not the image's.
    Inflate(InflateError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotPng => f.write_str("not a PNG file"),
            Error::Truncated => f.write_str("the PNG file is cut short"),
            Error::BadCrc => f.write_str("a PNG chunk's CRC is wrong"),
            Error::BadHeader => f.write_str("the PNG header is invalid"),
            Error::BadChunkOrder => f.write_str("PNG chunks out of order"),
            Error::BadChunk => f.write_str("a PNG chunk has the wrong length"),
            Error::UnknownCritical => f.write_str("an unknown critical PNG chunk"),
            Error::BadPalette => f.write_str("the PNG palette is missing or too short"),
            Error::BadFilter => f.write_str("a PNG scanline has an unknown filter"),
            Error::TooLarge => f.write_str("the PNG image is too large"),
            Error::Inflate(e) => write!(f, "PNG image data: {e}"),
        }
    }
}

impl From<InflateError> for Error {
    fn from(e: InflateError) -> Error {
        match e {
            InflateError::Truncated => Error::Truncated,
            e => Error::Inflate(e),
        }
    }
}

/// How much a file may ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most pixels (width times height). Four bytes each in the result
    /// and up to eight bytes each (16-bit RGBA) while decoding.
    pub max_pixels: u64,
}

impl Default for Limits {
    /// 16 million pixels: 4096 x 4096.
    fn default() -> Limits {
        Limits { max_pixels: 1 << 24 }
    }
}

/// The colour types of `IHDR` (section 11.2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorType {
    Grey = 0,
    Rgb = 2,
    Palette = 3,
    GreyAlpha = 4,
    Rgba = 6,
}

impl ColorType {
    /// Samples per pixel (a palette image has one: the index).
    pub fn channels(self) -> usize {
        match self {
            ColorType::Grey | ColorType::Palette => 1,
            ColorType::GreyAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        }
    }
}

/// What `IHDR` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: ColorType,
    /// Adam7 interlacing.
    pub interlaced: bool,
}

impl Header {
    fn bits_per_pixel(&self) -> usize {
        self.color_type.channels() * usize::from(self.bit_depth)
    }
}

/// A decoded picture: `0xAARRGGBB` pixels with straight (not
/// premultiplied) alpha, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// CRC-32 (ISO 3309 / ITU-T V.42, section 5.5 of the specification) over
/// the parts, in order.
pub fn crc32(parts: &[&[u8]]) -> u32 {
    // The table for the reflected polynomial 0xedb88320, made at compile
    // time.
    const TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                k += 1;
            }
            t[n] = c;
            n += 1;
        }
        t
    };
    let mut crc = 0xffff_ffffu32;
    for part in parts {
        for &b in *part {
            crc = TABLE[((crc ^ u32::from(b)) & 0xff) as usize] ^ (crc >> 8);
        }
    }
    !crc
}

/// One chunk: its name, data and whether its CRC matched.
struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    crc_ok: bool,
}

impl Chunk<'_> {
    /// Ancillary chunks (lower-case first letter) may be ignored.
    fn critical(&self) -> bool {
        self.kind[0] & 0x20 == 0
    }
}

/// The chunks of a file, after the signature.
struct Chunks<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let head = match self.rest.get(..8) {
            Some(h) => h,
            None => return Some(Err(self.fail())),
        };
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = [head[4], head[5], head[6], head[7]];
        // A length of 2^31 or more is not allowed; anything past the end
        // of the data is a cut-short file either way.
        let end = 8usize.saturating_add(len);
        let (Some(data), Some(crc)) = (self.rest.get(8..end), self.rest.get(end..end.saturating_add(4))) else {
            return Some(Err(self.fail()));
        };
        let stored = u32::from_be_bytes([crc[0], crc[1], crc[2], crc[3]]);
        self.rest = &self.rest[end + 4..];
        Some(Ok(Chunk { kind, data, crc_ok: crc32(&[&kind, data]) == stored }))
    }
}

impl Chunks<'_> {
    fn fail(&mut self) -> Error {
        self.rest = &[];
        Error::Truncated
    }
}

fn parse_header(chunk: &Chunk<'_>) -> Result<Header, Error> {
    if &chunk.kind != b"IHDR" || chunk.data.len() != 13 {
        return Err(Error::BadHeader);
    }
    if !chunk.crc_ok {
        return Err(Error::BadCrc);
    }
    let d = chunk.data;
    let width = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
    let height = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
    let bit_depth = d[8];
    let color_type = match d[9] {
        0 => ColorType::Grey,
        2 => ColorType::Rgb,
        3 => ColorType::Palette,
        4 => ColorType::GreyAlpha,
        6 => ColorType::Rgba,
        _ => return Err(Error::BadHeader),
    };
    // Section 11.2.2, table 7: the allowed depths per colour type.
    let depth_ok = match color_type {
        ColorType::Grey => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        ColorType::Palette => matches!(bit_depth, 1 | 2 | 4 | 8),
        ColorType::Rgb | ColorType::GreyAlpha | ColorType::Rgba => matches!(bit_depth, 8 | 16),
    };
    // Compression method 0, filter method 0, interlace 0 (none) or 1 (Adam7).
    let (compression, filter, interlace) = (d[10], d[11], d[12]);
    if width == 0 || height == 0 || width > 0x7fff_ffff || height > 0x7fff_ffff || !depth_ok {
        return Err(Error::BadHeader);
    }
    if compression != 0 || filter != 0 || interlace > 1 {
        return Err(Error::BadHeader);
    }
    Ok(Header { width, height, bit_depth, color_type, interlaced: interlace == 1 })
}

/// The header of a PNG file, without decoding it.
pub fn info(data: &[u8]) -> Result<Header, Error> {
    let rest = data.strip_prefix(&SIGNATURE).ok_or(Error::NotPng)?;
    let first = Chunks { rest }.next().ok_or(Error::Truncated)??;
    parse_header(&first)
}

/// Decode a PNG file with the default [`Limits`].
pub fn decode(data: &[u8]) -> Result<Image, Error> {
    decode_with(data, &Limits::default())
}

/// Decode a PNG file.
pub fn decode_with(data: &[u8], limits: &Limits) -> Result<Image, Error> {
    let rest = data.strip_prefix(&SIGNATURE).ok_or(Error::NotPng)?;
    let mut chunks = Chunks { rest };
    let header = parse_header(&chunks.next().ok_or(Error::Truncated)??)?;
    if u64::from(header.width) * u64::from(header.height) > limits.max_pixels {
        return Err(Error::TooLarge);
    }
    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut trns: Option<&[u8]> = None;
    let mut idat: Vec<u8> = Vec::new();
    let (mut seen_idat, mut idat_done, mut seen_iend) = (false, false, false);
    for chunk in chunks {
        let chunk = chunk?;
        // Other chunks between `IDAT`s end the image data.
        if seen_idat && &chunk.kind != b"IDAT" {
            idat_done = true;
        }
        if !chunk.crc_ok {
            if chunk.critical() {
                return Err(Error::BadCrc);
            }
            // A damaged ancillary chunk is ignored.
            continue;
        }
        match &chunk.kind {
            b"IHDR" => return Err(Error::BadChunkOrder),
            b"PLTE" => {
                if palette.is_some() || seen_idat {
                    return Err(Error::BadChunkOrder);
                }
                let n = chunk.data.len() / 3;
                if chunk.data.len() % 3 != 0 || n == 0 || n > 256 {
                    return Err(Error::BadChunk);
                }
                // Only palette images use it; for others it is a hint.
                if header.color_type == ColorType::Palette {
                    if n > 1usize << header.bit_depth {
                        return Err(Error::BadChunk);
                    }
                    palette = Some(chunk.data.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect());
                } else {
                    palette = Some(Vec::new());
                }
            }
            b"tRNS" => {
                if seen_idat {
                    continue;
                }
                let want = match header.color_type {
                    ColorType::Grey => Some(2),
                    ColorType::Rgb => Some(6),
                    ColorType::Palette => None,
                    // Not allowed with an alpha channel: ignored.
                    ColorType::GreyAlpha | ColorType::Rgba => continue,
                };
                if want.is_some_and(|w| chunk.data.len() != w) {
                    return Err(Error::BadChunk);
                }
                trns = Some(chunk.data);
            }
            b"IDAT" => {
                if idat_done {
                    return Err(Error::BadChunkOrder);
                }
                seen_idat = true;
                idat.extend_from_slice(chunk.data);
            }
            b"IEND" => {
                seen_iend = true;
                break;
            }
            _ if chunk.critical() => return Err(Error::UnknownCritical),
            _ => {}
        }
    }
    if !seen_iend {
        return Err(Error::Truncated);
    }
    if !seen_idat {
        return Err(Error::BadChunkOrder);
    }
    let palette = match header.color_type {
        ColorType::Palette => Some(palette.filter(|p| !p.is_empty()).ok_or(Error::BadPalette)?),
        _ => None,
    };
    let raw_len = raw_size(&header)?;
    let mut raw = inflate::zlib_decompress(&idat, raw_len)?;
    drop(idat);
    if raw.len() != raw_len {
        // Fewer bytes than the image needs.
        return Err(Error::Inflate(InflateError::Truncated));
    }
    let mut pixels = alloc::vec![0u32; header.width as usize * header.height as usize];
    let mut offset = 0;
    for pass in passes(&header) {
        let rb = row_bytes(&header, pass.width);
        unfilter(&mut raw[offset..offset + pass.height * (rb + 1)], rb, pass.height, header.bits_per_pixel())?;
        for py in 0..pass.height {
            let row = &raw[offset + py * (rb + 1) + 1..offset + (py + 1) * (rb + 1)];
            let y = pass.y0 + py * pass.dy;
            let base = y * header.width as usize;
            let mut put = |px: usize, argb: u32| pixels[base + pass.x0 + px * pass.dx] = argb;
            convert_row(&header, row, pass.width, palette.as_deref(), trns, &mut put)?;
        }
        offset += pass.height * (rb + 1);
    }
    Ok(Image { width: header.width, height: header.height, pixels })
}

/// The bytes of a scanline of `width` pixels (without its filter byte).
fn row_bytes(h: &Header, width: usize) -> usize {
    (width * h.bits_per_pixel()).div_ceil(8)
}

/// One pass over (part of) the image: the pixels at `x0 + i * dx`,
/// `y0 + j * dy` for `i < width`, `j < height`.
#[derive(Clone, Copy)]
struct Pass {
    x0: usize,
    y0: usize,
    dx: usize,
    dy: usize,
    width: usize,
    height: usize,
}

/// The non-empty passes: one for a plain image, up to seven for Adam7
/// (section 7.3: each pass's starting pixel and step).
fn passes(h: &Header) -> impl Iterator<Item = Pass> + use<> {
    const ADAM7: [(usize, usize, usize, usize); 7] =
        [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
    let (w, hh) = (h.width as usize, h.height as usize);
    let list: &[(usize, usize, usize, usize)] = if h.interlaced { &ADAM7 } else { &[(0, 0, 1, 1)] };
    list.iter().filter_map(move |&(x0, y0, dx, dy)| {
        // How many of 0..w are x0 + i * dx.
        let width = if w > x0 { (w - x0).div_ceil(dx) } else { 0 };
        let height = if hh > y0 { (hh - y0).div_ceil(dy) } else { 0 };
        (width > 0 && height > 0).then_some(Pass { x0, y0, dx, dy, width, height })
    })
}

/// The size of the decompressed data: every pass's rows, each a filter
/// byte and the row.
fn raw_size(h: &Header) -> Result<usize, Error> {
    let mut total: u64 = 0;
    for p in passes(h) {
        total += (p.height as u64) * (row_bytes(h, p.width) as u64 + 1);
    }
    usize::try_from(total).map_err(|_| Error::TooLarge)
}

/// Undo the filters of `rows` scanlines of `rb` bytes (each behind its
/// filter byte), in place (section 9).
fn unfilter(data: &mut [u8], rb: usize, rows: usize, bits_per_pixel: usize) -> Result<(), Error> {
    // The distance to the byte of the pixel to the left.
    let bpp = bits_per_pixel.div_ceil(8).max(1);
    for r in 0..rows {
        let start = r * (rb + 1);
        // The previous row's bytes are right before this row's filter byte.
        let (before, here) = data.split_at_mut(start + 1);
        let filter = before[start];
        let cur = &mut here[..rb];
        let prev: Option<&[u8]> = (r > 0).then(|| &before[start - rb..start]);
        match filter {
            0 => {}
            1 => {
                for i in bpp..rb {
                    cur[i] = cur[i].wrapping_add(cur[i - bpp]);
                }
            }
            2 => {
                if let Some(prev) = prev {
                    for i in 0..rb {
                        cur[i] = cur[i].wrapping_add(prev[i]);
                    }
                }
            }
            3 => {
                for i in 0..rb {
                    let a = if i >= bpp { u16::from(cur[i - bpp]) } else { 0 };
                    let b = prev.map_or(0, |p| u16::from(p[i]));
                    cur[i] = cur[i].wrapping_add(((a + b) / 2) as u8);
                }
            }
            4 => {
                for i in 0..rb {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    let b = prev.map_or(0, |p| p[i]);
                    let c = if i >= bpp { prev.map_or(0, |p| p[i - bpp]) } else { 0 };
                    cur[i] = cur[i].wrapping_add(paeth(a, b, c));
                }
            }
            _ => return Err(Error::BadFilter),
        }
    }
    Ok(())
}

/// The Paeth predictor (section 9.4): whichever of left `a`, above `b` and
/// upper-left `c` is nearest to `a + b - c`, ties to `a`, then `b`.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Scale a `depth`-bit sample (at most 8 bits) to 0 to 255.
fn scale_to_8(v: u32, depth: u8) -> u8 {
    match depth {
        1 => (v * 255) as u8,
        2 => (v * 85) as u8,
        4 => (v * 17) as u8,
        _ => v as u8,
    }
}

/// A 16-bit sample as 8 bits: `v * 255 / 65535`, rounded.
fn from_16(v: u32) -> u8 {
    ((v * 255 + 32_767) / 65_535) as u8
}

fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

/// Turn a decoded scanline of `width` pixels into colours, `put(i, argb)`
/// for pixel `i`.
fn convert_row(
    h: &Header,
    row: &[u8],
    width: usize,
    palette: Option<&[[u8; 3]]>,
    trns: Option<&[u8]>,
    put: &mut dyn FnMut(usize, u32),
) -> Result<(), Error> {
    let depth = h.bit_depth;
    // Sample `i` of the row (not for 16-bit depth).
    let small = |i: usize| -> u32 {
        match depth {
            8 => u32::from(row[i]),
            d => {
                let bit = i * usize::from(d);
                let shift = 8 - usize::from(d) - (bit % 8);
                (u32::from(row[bit / 8]) >> shift) & ((1 << d) - 1)
            }
        }
    };
    let big = |i: usize| -> u32 { u32::from(u16::from_be_bytes([row[2 * i], row[2 * i + 1]])) };
    let word = |t: &[u8], i: usize| -> u32 { u32::from(u16::from_be_bytes([t[2 * i], t[2 * i + 1]])) };
    match (h.color_type, depth) {
        (ColorType::Grey, 16) => {
            let key = trns.map(|t| word(t, 0));
            for x in 0..width {
                let v = big(x);
                let g = from_16(v);
                put(x, argb(if key == Some(v) { 0 } else { 255 }, g, g, g));
            }
        }
        (ColorType::Grey, d) => {
            let key = trns.map(|t| word(t, 0));
            for x in 0..width {
                let v = small(x);
                let g = scale_to_8(v, d);
                put(x, argb(if key == Some(v) { 0 } else { 255 }, g, g, g));
            }
        }
        (ColorType::Rgb, 16) => {
            let key = trns.map(|t| [word(t, 0), word(t, 1), word(t, 2)]);
            for x in 0..width {
                let s = [big(3 * x), big(3 * x + 1), big(3 * x + 2)];
                let a = if key == Some(s) { 0 } else { 255 };
                put(x, argb(a, from_16(s[0]), from_16(s[1]), from_16(s[2])));
            }
        }
        (ColorType::Rgb, _) => {
            let key = trns.map(|t| [word(t, 0), word(t, 1), word(t, 2)]);
            for x in 0..width {
                let s = [small(3 * x), small(3 * x + 1), small(3 * x + 2)];
                let a = if key == Some(s) { 0 } else { 255 };
                put(x, argb(a, s[0] as u8, s[1] as u8, s[2] as u8));
            }
        }
        (ColorType::Palette, _) => {
            let palette = palette.ok_or(Error::BadPalette)?;
            for x in 0..width {
                let i = small(x) as usize;
                let [r, g, b] = *palette.get(i).ok_or(Error::BadPalette)?;
                // Entries past the end of tRNS are opaque.
                let a = trns.and_then(|t| t.get(i)).copied().unwrap_or(255);
                put(x, argb(a, r, g, b));
            }
        }
        (ColorType::GreyAlpha, 16) => {
            for x in 0..width {
                let (g, a) = (from_16(big(2 * x)), from_16(big(2 * x + 1)));
                put(x, argb(a, g, g, g));
            }
        }
        (ColorType::GreyAlpha, _) => {
            for x in 0..width {
                let (g, a) = (row[2 * x], row[2 * x + 1]);
                put(x, argb(a, g, g, g));
            }
        }
        (ColorType::Rgba, 16) => {
            for x in 0..width {
                let c: [u8; 4] = core::array::from_fn(|k| from_16(big(4 * x + k)));
                put(x, argb(c[3], c[0], c[1], c[2]));
            }
        }
        (ColorType::Rgba, _) => {
            for x in 0..width {
                let p = &row[4 * x..4 * x + 4];
                put(x, argb(p[3], p[0], p[1], p[2]));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
