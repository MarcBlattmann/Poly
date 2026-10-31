//! A minimal PNG writer (PNG specification, W3C/ISO 15948): 8-bit grey or
//! RGB, no filtering, zlib data in uncompressed ("stored") deflate
//! blocks. Enough to look at rendered text and golden bitmaps.

use crate::render::{Image, RenderMode};
use alloc::vec::Vec;

/// CRC-32 (ISO 3309, as PNG chunks use it).
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for part in parts {
        for &b in *part {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// A PNG of `width` × `height` pixels of `channels` (1: grey, 3: RGB)
/// bytes each, rows top to bottom.
pub fn encode(width: u32, height: u32, channels: u8, pixels: &[u8]) -> Vec<u8> {
    let row = width as usize * usize::from(channels);
    // Each row: filter type 0, then its bytes.
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        let start = y * row;
        let src = pixels.get(start..start + row).unwrap_or(&[]);
        raw.extend_from_slice(src);
        raw.resize(raw.len() + (row - src.len()), 0);
    }
    let mut z = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 16);
    z.extend_from_slice(&[0x78, 0x01]);
    let mut blocks = raw.chunks(65535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        let len = b.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 3 { 2 } else { 0 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A coverage image as a PNG, dark text on white (coverage inverted).
pub fn encode_image(img: &Image) -> Vec<u8> {
    let inverted: Vec<u8> = img.data.iter().map(|&v| 255 - v).collect();
    let channels = if img.mode == RenderMode::Lcd { 3 } else { 1 };
    encode(img.width, img.height, channels, &inverted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(crc32(&[b"IEND"]), 0xae42_6082);
        assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn a_tiny_png() {
        let png = encode(2, 1, 1, &[0, 255]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        // IDAT: zlib header, one final stored block of 3 bytes (filter + 2).
        let idat = 8 + 25;
        assert_eq!(&png[idat + 4..idat + 8], b"IDAT");
        assert_eq!(&png[idat + 8..idat + 18], &[0x78, 0x01, 1, 3, 0, 0xfc, 0xff, 0, 0, 255]);
    }
}
