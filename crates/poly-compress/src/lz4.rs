//! LZ4 blocks (the block format of the LZ4 project's published
//! description): sequences of a token, literals and a match.

use crate::Error;
use alloc::vec::Vec;

/// Adds the length bytes that extend a 15 nibble: bytes of 255 continue.
fn extend(src: &[u8], i: &mut usize, mut n: usize) -> Result<usize, Error> {
    loop {
        let b = *src.get(*i).ok_or(Error::Corrupt)?;
        *i += 1;
        n = n.checked_add(b as usize).ok_or(Error::Corrupt)?;
        if b != 255 {
            return Ok(n);
        }
    }
}

/// Decodes one LZ4 block (no frame header) to at most `max_out` bytes.
pub fn decompress(src: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    let mut out: Vec<u8> = Vec::with_capacity(max_out.min(1 << 20));
    let mut i = 0;
    while i < src.len() {
        let token = src[i];
        i += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            lit = extend(src, &mut i, lit)?;
        }
        let end = i.checked_add(lit).ok_or(Error::Corrupt)?;
        if end > src.len() {
            return Err(Error::Corrupt);
        }
        if out.len() + lit > max_out {
            return Err(Error::TooLarge);
        }
        out.extend_from_slice(&src[i..end]);
        i = end;
        if i == src.len() {
            break; // the last sequence has literals only
        }
        let lo = *src.get(i).ok_or(Error::Corrupt)?;
        let hi = *src.get(i + 1).ok_or(Error::Corrupt)?;
        let off = u16::from_le_bytes([lo, hi]) as usize;
        i += 2;
        if off == 0 || off > out.len() {
            return Err(Error::Corrupt);
        }
        let mut len = (token & 15) as usize;
        if len == 15 {
            len = extend(src, &mut i, len)?;
        }
        len += 4;
        if out.len() + len > max_out {
            return Err(Error::TooLarge);
        }
        let start = out.len() - off;
        for k in 0..len {
            let b = out[start + k];
            out.push(b);
        }
    }
    Ok(out)
}
