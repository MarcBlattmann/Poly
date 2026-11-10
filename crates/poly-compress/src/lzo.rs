//! LZO1X streams: the instruction set of the LZO1X format (all the
//! compression levels share one decoder). Instructions, by their first
//! byte `b`, with `S` the literals (0 to 3) the previous instruction
//! carried and "run" the state after a run of four or more:
//!
//! - `0000xxxx`: with `S == 0` a literal run of `xxxx + 3` (`xxxx == 0`:
//!   18 plus extension bytes); with `S` 1 to 3 a two-byte match with a
//!   short distance; right after a long run a three-byte match with a
//!   distance of 2049 and more;
//! - `0001hlll`: a long-distance match (16 KiB and more) or, at distance
//!   16384, the end of the stream;
//! - `001lllll`: a match of up to 16 KiB;
//! - `01lddlss`/`1lldddss`: short matches (up to 2 KiB).

use crate::Error;
use alloc::vec::Vec;

struct In<'a> {
    s: &'a [u8],
    i: usize,
}

impl In<'_> {
    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.s.get(self.i).ok_or(Error::Corrupt)?;
        self.i += 1;
        Ok(b)
    }

    /// Length extension: zero bytes add 255 each, the first other byte ends.
    fn extension(&mut self) -> Result<usize, Error> {
        let mut n = 0usize;
        loop {
            let b = self.byte()?;
            if b == 0 {
                n = n.checked_add(255).ok_or(Error::Corrupt)?;
            } else {
                return n.checked_add(b as usize).ok_or(Error::Corrupt);
            }
        }
    }
}

fn copy_literals(src: &mut In<'_>, out: &mut Vec<u8>, n: usize, max_out: usize) -> Result<(), Error> {
    let end = src.i.checked_add(n).ok_or(Error::Corrupt)?;
    if end > src.s.len() {
        return Err(Error::Corrupt);
    }
    if out.len() + n > max_out {
        return Err(Error::TooLarge);
    }
    out.extend_from_slice(&src.s[src.i..end]);
    src.i = end;
    Ok(())
}

fn copy_match(out: &mut Vec<u8>, dist: usize, len: usize, max_out: usize) -> Result<(), Error> {
    if dist == 0 || dist > out.len() {
        return Err(Error::Corrupt);
    }
    if out.len() + len > max_out {
        return Err(Error::TooLarge);
    }
    let start = out.len() - dist;
    for k in 0..len {
        let b = out[start + k];
        out.push(b);
    }
    Ok(())
}

/// Decodes an LZO1X stream to at most `max_out` bytes.
pub fn decompress(data: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    let mut src = In { s: data, i: 0 };
    let mut out: Vec<u8> = Vec::with_capacity(max_out.min(1 << 20));
    // Literals carried by the previous instruction: 0..=3, or 4 for "a
    // run of four or more just ended".
    let mut state;
    let first = *data.first().ok_or(Error::Corrupt)?;
    if first > 17 {
        src.i = 1;
        let n = first as usize - 17;
        copy_literals(&mut src, &mut out, n, max_out)?;
        state = if n < 4 { n } else { 4 };
    } else {
        state = 0;
    }
    loop {
        let b = src.byte()? as usize;
        let (len, dist, trailing);
        match b {
            0..=15 => {
                if state == 0 {
                    let n = if b == 0 { 18 + src.extension()? } else { b + 3 };
                    copy_literals(&mut src, &mut out, n, max_out)?;
                    state = 4;
                    continue;
                }
                let h = src.byte()? as usize;
                if state == 4 {
                    len = 3;
                    dist = (h << 2) + ((b >> 2) & 3) + 2049;
                } else {
                    len = 2;
                    dist = (h << 2) + ((b >> 2) & 3) + 1;
                }
                trailing = b & 3;
            }
            16..=31 => {
                let l = if b & 7 == 0 { 7 + src.extension()? } else { b & 7 };
                len = 2 + l;
                let lo = src.byte()? as usize;
                let hi = src.byte()? as usize;
                let v = lo | (hi << 8);
                let d = 16384 + ((b & 8) << 11) + (v >> 2);
                if d == 16384 {
                    return Ok(out); // end of stream
                }
                dist = d;
                trailing = v & 3;
            }
            32..=63 => {
                let l = if b & 31 == 0 { 31 + src.extension()? } else { b & 31 };
                len = 2 + l;
                let lo = src.byte()? as usize;
                let hi = src.byte()? as usize;
                let v = lo | (hi << 8);
                dist = (v >> 2) + 1;
                trailing = v & 3;
            }
            64..=127 => {
                len = 3 + ((b >> 5) & 1);
                let h = src.byte()? as usize;
                dist = (h << 3) + ((b >> 2) & 7) + 1;
                trailing = b & 3;
            }
            _ => {
                len = 5 + ((b >> 5) & 3);
                let h = src.byte()? as usize;
                dist = (h << 3) + ((b >> 2) & 7) + 1;
                trailing = b & 3;
            }
        }
        copy_match(&mut out, dist, len, max_out)?;
        if trailing > 0 {
            copy_literals(&mut src, &mut out, trailing, max_out)?;
        }
        state = trailing;
    }
}
