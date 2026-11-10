//! The xz container (the public ".xz file format" description) around
//! LZMA2, as far as one squashfs block needs: a stream header, one block
//! whose only filter is LZMA2 and the block's data. The check value and
//! the index are not read (a squashfs block carries its own size).
//! [`decompress_stream`] reads a whole stream, every block of it, as a file
//! made by xz(1) is (a `.deb`'s `control.tar.xz`, ADR 0173).

use crate::Error;
use crate::lzma;
use alloc::vec::Vec;

const MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0];

/// A variable-length integer (7 bits per byte, low first, at most 9 bytes).
fn varint(src: &[u8], i: &mut usize) -> Result<u64, Error> {
    let mut v = 0u64;
    for shift in 0..9 {
        let b = *src.get(*i).ok_or(Error::Corrupt)?;
        *i += 1;
        v |= u64::from(b & 0x7f) << (7 * shift);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(Error::Corrupt)
}

/// Decodes the first block of an xz stream to at most `max_out` bytes.
pub fn decompress(src: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    let (mut i, _) = stream_header(src)?;
    match block(src, &mut i, max_out)? {
        Some(out) => Ok(out),
        // The index: a stream without blocks.
        None => Ok(Vec::new()),
    }
}

/// Decodes every block of the first xz stream in `src` (what xz(1) writes
/// for one file, with or without `--block-size`) to at most `max_out`
/// bytes together. The check values and the index are not read.
pub fn decompress_stream(src: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    let (mut i, check_len) = stream_header(src)?;
    let mut out = Vec::new();
    while let Some(part) = block(src, &mut i, max_out - out.len())? {
        if out.is_empty() {
            out = part;
        } else {
            out.extend_from_slice(&part);
        }
        // Block padding to a multiple of four bytes, then the check.
        i = i.next_multiple_of(4) + check_len;
    }
    Ok(out)
}

/// Checks the stream header; returns where the first block (or the index)
/// starts and how long each block's check value is.
fn stream_header(src: &[u8]) -> Result<(usize, usize), Error> {
    if src.len() < 12 + 12 || src[..6] != MAGIC {
        return Err(Error::Corrupt);
    }
    // Stream flags: the second byte's low nibble is the check type.
    if src[6] != 0 || src[7] > 0x0f {
        return Err(Error::Corrupt);
    }
    // The size of the check by its type (".xz file format", 2.1.1.2).
    let check_len = match src[7] {
        0 => 0,
        1..=3 => 4,
        4..=6 => 8,
        7..=9 => 16,
        10..=12 => 32,
        _ => 64,
    };
    Ok((12, check_len))
}

/// Decodes the block at `*i` (its header and LZMA2 data) and moves `*i`
/// past the data; `None` where the index starts instead of a block.
fn block(src: &[u8], i: &mut usize, max_out: usize) -> Result<Option<Vec<u8>>, Error> {
    let size_byte = *src.get(*i).ok_or(Error::Corrupt)?;
    if size_byte == 0 {
        return Ok(None);
    }
    let header_len = (usize::from(size_byte) + 1) * 4;
    let header = src.get(*i..*i + header_len).ok_or(Error::Corrupt)?;
    let flags = header[1];
    let filters = usize::from(flags & 3) + 1;
    let mut h = 2usize;
    if flags & 0x40 != 0 {
        varint(header, &mut h)?; // compressed size
    }
    if flags & 0x80 != 0 {
        varint(header, &mut h)?; // uncompressed size
    }
    if filters != 1 {
        return Err(Error::Unsupported);
    }
    let id = varint(header, &mut h)?;
    let props = varint(header, &mut h)? as usize;
    if id != 0x21 {
        return Err(Error::Unsupported);
    }
    if props != 1 || header.get(h).is_none_or(|&d| d > 40) {
        return Err(Error::Corrupt);
    }
    *i += header_len;
    let (out, used) = lzma::lzma2(&src[*i..], max_out)?;
    *i += used;
    Ok(Some(out))
}
