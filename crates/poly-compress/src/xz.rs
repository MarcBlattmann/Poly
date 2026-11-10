//! The xz container (the public ".xz file format" description) around
//! LZMA2, as far as one squashfs block needs: a stream header, one block
//! whose only filter is LZMA2 and the block's data. The check value and
//! the index are not read (a squashfs block carries its own size).

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
    if src.len() < 12 + 12 || src[..6] != MAGIC {
        return Err(Error::Corrupt);
    }
    // Stream flags: the second byte's low nibble is the check type.
    if src[6] != 0 || src[7] > 0x0f {
        return Err(Error::Corrupt);
    }
    let mut i = 12usize;
    let size_byte = src[i];
    if size_byte == 0 {
        // The index: a stream without blocks.
        return Ok(Vec::new());
    }
    let header_len = (usize::from(size_byte) + 1) * 4;
    let header = src.get(i..i + header_len).ok_or(Error::Corrupt)?;
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
    i += header_len;
    let (out, _) = lzma::lzma2(&src[i..], max_out)?;
    Ok(out)
}
