//! Zstandard frames (RFC 8878): raw, RLE and compressed blocks; Huffman
//! coded literals in one or four streams; FSE coded sequences.

use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

const MAGIC: u32 = 0xFD2F_B528;
const MAX_HUF_BITS: u32 = 11;

/// Match length codes: base and extra bits.
const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027, 2051, 4099, 8195, 16387, 32771, 65539,
];
const ML_BITS: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2,
    3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];
/// Literal length codes.
const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64, 128, 256, 512, 1024,
    2048, 4096, 8192, 16384, 32768, 65536,
];
const LL_BITS: [u8; 36] =
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// The predefined distributions (RFC 8878, 3.1.1.3.2.2).
const LL_DEFAULT: [i16; 36] =
    [4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1, -1, -1, -1, -1];
const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
const OF_DEFAULT: [i16; 29] =
    [1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1];

/// A bit stream read from its end towards its start (the last byte holds a
/// 1 marker above the data); a read of `n` bits returns the next `n` bits
/// below the position, the first of them the most significant.
struct Back<'a> {
    data: &'a [u8],
    pos: usize,
    over: bool,
}

impl<'a> Back<'a> {
    fn new(data: &'a [u8]) -> Result<Back<'a>, Error> {
        let last = *data.last().ok_or(Error::Corrupt)?;
        if last == 0 {
            return Err(Error::Corrupt);
        }
        let top = 7 - last.leading_zeros() as usize;
        Ok(Back { data, pos: (data.len() - 1) * 8 + top, over: false })
    }

    fn bits(&self, start: usize, n: u32) -> u64 {
        let first = start / 8;
        let mut x = 0u64;
        for k in 0..8 {
            if let Some(&b) = self.data.get(first + k) {
                x |= u64::from(b) << (8 * k);
            }
        }
        (x >> (start % 8)) & ((1u64 << n) - 1)
    }

    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let n = n as usize;
        if self.pos >= n {
            self.bits(self.pos - n, n as u32) as u32
        } else {
            (self.bits(0, self.pos as u32) << (n - self.pos)) as u32
        }
    }

    fn read(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        if self.pos >= n as usize {
            self.pos -= n as usize;
        } else {
            self.pos = 0;
            self.over = true;
        }
        v
    }

    /// Whether more bits were asked for than the stream held.
    fn exhausted(&self) -> bool {
        self.over
    }
}

/// A forward (little-endian, low bits first) bit reader for table headers.
struct Fwd<'a> {
    data: &'a [u8],
    bit: usize,
}

impl Fwd<'_> {
    fn peek(&self, n: u32) -> u32 {
        let first = self.bit / 8;
        let mut x = 0u64;
        for k in 0..8 {
            if let Some(&b) = self.data.get(first + k) {
                x |= u64::from(b) << (8 * k);
            }
        }
        ((x >> (self.bit % 8)) & ((1u64 << n) - 1)) as u32
    }

    fn skip(&mut self, n: u32) {
        self.bit += n as usize;
    }
}

#[derive(Clone, Copy, Default)]
struct Cell {
    sym: u8,
    nbits: u8,
    base: u16,
}

#[derive(Clone, Default)]
struct Fse {
    al: u32,
    cells: Vec<Cell>,
}

impl Fse {
    fn build(norm: &[i16], al: u32) -> Result<Fse, Error> {
        let size = 1usize << al;
        let mut cells = vec![Cell::default(); size];
        let mut next = [0u32; 256];
        let mut high = size as isize - 1;
        let mut total = 0i32;
        for (s, &c) in norm.iter().enumerate() {
            if c == -1 {
                if high < 0 {
                    return Err(Error::Corrupt);
                }
                cells[high as usize].sym = s as u8;
                high -= 1;
                next[s] = 1;
                total += 1;
            } else if c >= 0 {
                next[s] = c as u32;
                total += i32::from(c);
            } else {
                return Err(Error::Corrupt);
            }
        }
        if total as usize != size {
            return Err(Error::Corrupt);
        }
        let step = (size >> 1) + (size >> 3) + 3;
        let mask = size - 1;
        let mut pos = 0usize;
        for (s, &c) in norm.iter().enumerate() {
            for _ in 0..c.max(0) {
                cells[pos].sym = s as u8;
                pos = (pos + step) & mask;
                while pos as isize > high {
                    pos = (pos + step) & mask;
                }
            }
        }
        if pos != 0 {
            return Err(Error::Corrupt);
        }
        for cell in &mut cells {
            let s = cell.sym as usize;
            let ns = next[s];
            next[s] += 1;
            let nb = al - (31 - ns.leading_zeros());
            cell.nbits = nb as u8;
            cell.base = ((ns << nb) as usize - size) as u16;
        }
        Ok(Fse { al, cells })
    }

    /// Reads a table description at the start of `src`; returns the table
    /// and the bytes it took.
    fn read(src: &[u8], max_sym: usize, max_al: u32) -> Result<(Fse, usize), Error> {
        let mut r = Fwd { data: src, bit: 0 };
        let al = r.peek(4) + 5;
        r.skip(4);
        if al > max_al {
            return Err(Error::Corrupt);
        }
        let mut remaining = (1i32 << al) + 1;
        let mut threshold = 1i32 << al;
        let mut nb = al + 1;
        let mut norm: Vec<i16> = Vec::new();
        let mut prev0 = false;
        while remaining > 1 && norm.len() <= max_sym {
            if prev0 {
                // Runs of zero probabilities: 2-bit repeat counts.
                loop {
                    let v = r.peek(2);
                    r.skip(2);
                    norm.extend(core::iter::repeat_n(0, v as usize));
                    if v != 3 {
                        break;
                    }
                    if norm.len() > max_sym + 1 {
                        return Err(Error::Corrupt);
                    }
                }
                if norm.len() > max_sym + 1 {
                    return Err(Error::Corrupt);
                }
                prev0 = false;
                continue;
            }
            let max = (2 * threshold - 1) - remaining;
            let low = r.peek(nb - 1) as i32;
            let count;
            if low < max {
                count = low;
                r.skip(nb - 1);
            } else {
                let full = r.peek(nb) as i32;
                r.skip(nb);
                count = if full >= threshold { full - max } else { full };
            }
            let prob = count - 1;
            remaining -= prob.abs();
            norm.push(prob as i16);
            prev0 = prob == 0;
            while remaining < threshold {
                nb -= 1;
                threshold >>= 1;
            }
        }
        if remaining != 1 || norm.len() > max_sym + 1 {
            return Err(Error::Corrupt);
        }
        let used = r.bit.div_ceil(8);
        if used > src.len() {
            return Err(Error::Corrupt);
        }
        Ok((Fse::build(&norm, al)?, used))
    }

    fn rle(sym: u8) -> Fse {
        Fse { al: 0, cells: vec![Cell { sym, nbits: 0, base: 0 }] }
    }
}

/// A Huffman decoding table: `(symbol, bits)` for each of `1 << bits`
/// prefixes.
#[derive(Clone, Default)]
struct Huf {
    bits: u32,
    table: Vec<(u8, u8)>,
}

impl Huf {
    fn from_weights(weights: &[u8]) -> Result<Huf, Error> {
        // Weights of all symbols but the last, which completes the sum.
        let mut sum = 0u32;
        for &w in weights {
            if w as u32 > MAX_HUF_BITS {
                return Err(Error::Corrupt);
            }
            if w > 0 {
                sum += 1 << (w - 1);
            }
        }
        if sum == 0 {
            return Err(Error::Corrupt);
        }
        let bits = 32 - sum.leading_zeros();
        if bits > MAX_HUF_BITS {
            return Err(Error::Corrupt);
        }
        let left = (1u32 << bits) - sum;
        if left == 0 || !left.is_power_of_two() {
            return Err(Error::Corrupt);
        }
        let last = (left.trailing_zeros() + 1) as u8;
        let mut all: Vec<u8> = weights.to_vec();
        all.push(last);
        if all.len() > 256 {
            return Err(Error::Corrupt);
        }
        let mut table = vec![(0u8, 0u8); 1usize << bits];
        let mut at = 0usize;
        for w in 1..=bits {
            for (s, &sw) in all.iter().enumerate() {
                if u32::from(sw) == w {
                    let span = 1usize << (w - 1);
                    for e in &mut table[at..at + span] {
                        *e = (s as u8, (bits + 1 - w) as u8);
                    }
                    at += span;
                }
            }
        }
        if at != table.len() {
            return Err(Error::Corrupt);
        }
        Ok(Huf { bits, table })
    }

    /// Reads a tree description at `src`; returns it and the bytes used.
    fn read(src: &[u8]) -> Result<(Huf, usize), Error> {
        let hb = *src.first().ok_or(Error::Corrupt)? as usize;
        let mut weights: Vec<u8> = Vec::new();
        let used;
        if hb >= 128 {
            let n = hb - 127;
            let bytes = n.div_ceil(2);
            let body = src.get(1..1 + bytes).ok_or(Error::Corrupt)?;
            for i in 0..n {
                let b = body[i / 2];
                weights.push(if i % 2 == 0 { b >> 4 } else { b & 15 });
            }
            used = 1 + bytes;
        } else {
            let body = src.get(1..1 + hb).ok_or(Error::Corrupt)?;
            let (fse, taken) = Fse::read(body, 255, 6)?;
            let mut bs = Back::new(&body[taken..])?;
            let mut s1 = bs.read(fse.al) as usize;
            let mut s2 = bs.read(fse.al) as usize;
            loop {
                if weights.len() > 255 {
                    return Err(Error::Corrupt);
                }
                let c1 = fse.cells[s1];
                weights.push(c1.sym);
                s1 = c1.base as usize + bs.read(u32::from(c1.nbits)) as usize;
                if bs.exhausted() {
                    weights.push(fse.cells[s2].sym);
                    break;
                }
                let c2 = fse.cells[s2];
                weights.push(c2.sym);
                s2 = c2.base as usize + bs.read(u32::from(c2.nbits)) as usize;
                if bs.exhausted() {
                    weights.push(fse.cells[s1].sym);
                    break;
                }
            }
            used = 1 + hb;
        }
        Ok((Huf::from_weights(&weights)?, used))
    }

    /// Decodes `count` symbols from one backward stream.
    fn decode(&self, stream: &[u8], count: usize, out: &mut Vec<u8>) -> Result<(), Error> {
        let mut bs = Back::new(stream)?;
        for _ in 0..count {
            let idx = bs.peek(self.bits) as usize;
            let (sym, nb) = self.table[idx];
            out.push(sym);
            bs.read(u32::from(nb));
            if bs.exhausted() {
                return Err(Error::Corrupt);
            }
        }
        if bs.pos != 0 {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}

/// What persists between the blocks of a frame.
#[derive(Default)]
struct Ctx {
    huf: Option<Huf>,
    ll: Option<Fse>,
    of: Option<Fse>,
    ml: Option<Fse>,
    reps: [u32; 3],
}

fn parse_literals(src: &[u8], ctx: &mut Ctx, block_max: usize) -> Result<(Vec<u8>, usize), Error> {
    let b0 = *src.first().ok_or(Error::Corrupt)?;
    let kind = b0 & 3;
    let fmt = (b0 >> 2) & 3;
    match kind {
        0 | 1 => {
            let (size, hdr) = match fmt {
                0 | 2 => (usize::from(b0 >> 3), 1),
                1 => {
                    let b = src.get(0..2).ok_or(Error::Corrupt)?;
                    ((usize::from(b[0]) | usize::from(b[1]) << 8) >> 4, 2)
                }
                _ => {
                    let b = src.get(0..3).ok_or(Error::Corrupt)?;
                    ((usize::from(b[0]) | usize::from(b[1]) << 8 | usize::from(b[2]) << 16) >> 4, 3)
                }
            };
            if size > block_max {
                return Err(Error::Corrupt);
            }
            if kind == 0 {
                let d = src.get(hdr..hdr + size).ok_or(Error::Corrupt)?;
                Ok((d.to_vec(), hdr + size))
            } else {
                let b = *src.get(hdr).ok_or(Error::Corrupt)?;
                Ok((vec![b; size], hdr + 1))
            }
        }
        _ => {
            let (regen, comp, hdr, streams) = match fmt {
                0 | 1 => {
                    let b = src.get(0..3).ok_or(Error::Corrupt)?;
                    let v = usize::from(b[0]) | usize::from(b[1]) << 8 | usize::from(b[2]) << 16;
                    ((v >> 4) & 0x3ff, (v >> 14) & 0x3ff, 3, if fmt == 0 { 1 } else { 4 })
                }
                2 => {
                    let b = src.get(0..4).ok_or(Error::Corrupt)?;
                    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
                    ((v >> 4) & 0x3fff, (v >> 18) & 0x3fff, 4, 4)
                }
                _ => {
                    let b = src.get(0..5).ok_or(Error::Corrupt)?;
                    let v = u64::from(b[0])
                        | u64::from(b[1]) << 8
                        | u64::from(b[2]) << 16
                        | u64::from(b[3]) << 24
                        | u64::from(b[4]) << 32;
                    (((v >> 4) & 0x3ffff) as usize, ((v >> 22) & 0x3ffff) as usize, 5, 4)
                }
            };
            if regen > block_max {
                return Err(Error::Corrupt);
            }
            let body = src.get(hdr..hdr + comp).ok_or(Error::Corrupt)?;
            let mut at = 0usize;
            if kind == 2 {
                let (h, used) = Huf::read(body)?;
                ctx.huf = Some(h);
                at = used;
            }
            let huf = ctx.huf.as_ref().ok_or(Error::Corrupt)?;
            let mut out: Vec<u8> = Vec::with_capacity(regen);
            let data = &body[at..];
            if streams == 1 {
                huf.decode(data, regen, &mut out)?;
            } else {
                let jt = data.get(0..6).ok_or(Error::Corrupt)?;
                let s1 = usize::from(jt[0]) | usize::from(jt[1]) << 8;
                let s2 = usize::from(jt[2]) | usize::from(jt[3]) << 8;
                let s3 = usize::from(jt[4]) | usize::from(jt[5]) << 8;
                let rest = &data[6..];
                if s1 + s2 + s3 > rest.len() {
                    return Err(Error::Corrupt);
                }
                let s4 = rest.len() - s1 - s2 - s3;
                let each = regen.div_ceil(4);
                if each * 3 > regen {
                    return Err(Error::Corrupt);
                }
                let parts = [
                    (&rest[..s1], each),
                    (&rest[s1..s1 + s2], each),
                    (&rest[s1 + s2..s1 + s2 + s3], each),
                    (&rest[s1 + s2 + s3..s1 + s2 + s3 + s4], regen - 3 * each),
                ];
                for (stream, n) in parts {
                    huf.decode(stream, n, &mut out)?;
                }
            }
            Ok((out, hdr + comp))
        }
    }
}

/// One sequence-table mode: reads the table, or reuses the last one.
fn sequence_table(
    mode: u8,
    src: &[u8],
    at: &mut usize,
    (max_sym, max_al, default, default_al): (usize, u32, &[i16], u32),
    prev: &mut Option<Fse>,
) -> Result<Fse, Error> {
    let fse = match mode {
        0 => Fse::build(default, default_al)?,
        1 => {
            let s = *src.get(*at).ok_or(Error::Corrupt)?;
            *at += 1;
            if s as usize > max_sym {
                return Err(Error::Corrupt);
            }
            Fse::rle(s)
        }
        2 => {
            let (f, used) = Fse::read(src.get(*at..).ok_or(Error::Corrupt)?, max_sym, max_al)?;
            *at += used;
            f
        }
        _ => return prev.clone().ok_or(Error::Corrupt),
    };
    *prev = Some(fse.clone());
    Ok(fse)
}

fn compressed_block(
    src: &[u8],
    ctx: &mut Ctx,
    out: &mut Vec<u8>,
    frame_start: usize,
    max_out: usize,
) -> Result<(), Error> {
    let block_max = 128 * 1024;
    let (literals, mut at) = parse_literals(src, ctx, block_max)?;
    let b0 = *src.get(at).ok_or(Error::Corrupt)?;
    let n = if b0 < 128 {
        at += 1;
        usize::from(b0)
    } else if b0 < 255 {
        let b1 = *src.get(at + 1).ok_or(Error::Corrupt)?;
        at += 2;
        ((usize::from(b0) - 128) << 8) + usize::from(b1)
    } else {
        let b = src.get(at + 1..at + 3).ok_or(Error::Corrupt)?;
        at += 3;
        usize::from(b[0]) + (usize::from(b[1]) << 8) + 0x7f00
    };
    if n == 0 {
        if out.len() + literals.len() > max_out {
            return Err(Error::TooLarge);
        }
        out.extend_from_slice(&literals);
        return Ok(());
    }
    let modes = *src.get(at).ok_or(Error::Corrupt)?;
    at += 1;
    if modes & 3 != 0 {
        return Err(Error::Corrupt);
    }
    let ll = sequence_table(modes >> 6, src, &mut at, (35, 9, &LL_DEFAULT, 6), &mut ctx.ll)?;
    let of = sequence_table((modes >> 4) & 3, src, &mut at, (31, 8, &OF_DEFAULT, 5), &mut ctx.of)?;
    let ml = sequence_table((modes >> 2) & 3, src, &mut at, (52, 9, &ML_DEFAULT, 6), &mut ctx.ml)?;
    let mut bs = Back::new(src.get(at..).ok_or(Error::Corrupt)?)?;
    let mut sl = bs.read(ll.al) as usize;
    let mut so = bs.read(of.al) as usize;
    let mut sm = bs.read(ml.al) as usize;
    let mut lit_at = 0usize;
    for i in 0..n {
        let (cl, co, cm) = (ll.cells[sl], of.cells[so], ml.cells[sm]);
        let (llc, ofc, mlc) = (cl.sym as usize, co.sym as u32, cm.sym as usize);
        if llc >= LL_BASE.len() || mlc >= ML_BASE.len() || ofc > 31 {
            return Err(Error::Corrupt);
        }
        let ofv = (1u32 << ofc) + bs.read(ofc);
        let mlen = ML_BASE[mlc] as usize + bs.read(u32::from(ML_BITS[mlc])) as usize;
        let llen = LL_BASE[llc] as usize + bs.read(u32::from(LL_BITS[llc])) as usize;
        if i + 1 < n {
            sl = cl.base as usize + bs.read(u32::from(cl.nbits)) as usize;
            sm = cm.base as usize + bs.read(u32::from(cm.nbits)) as usize;
            so = co.base as usize + bs.read(u32::from(co.nbits)) as usize;
        }
        if bs.exhausted() {
            return Err(Error::Corrupt);
        }
        // The offset: a new one, or one of the three last used.
        let offset;
        if ofv > 3 {
            offset = ofv - 3;
            ctx.reps = [offset, ctx.reps[0], ctx.reps[1]];
        } else {
            let idx = (ofv - 1) as usize + usize::from(llen == 0);
            if idx == 0 {
                offset = ctx.reps[0];
            } else if idx == 1 {
                offset = ctx.reps[1];
                ctx.reps = [offset, ctx.reps[0], ctx.reps[2]];
            } else if idx == 2 {
                offset = ctx.reps[2];
                ctx.reps = [offset, ctx.reps[0], ctx.reps[1]];
            } else {
                offset = ctx.reps[0].checked_sub(1).ok_or(Error::Corrupt)?;
                ctx.reps = [offset, ctx.reps[0], ctx.reps[1]];
            }
        }
        if offset == 0 {
            return Err(Error::Corrupt);
        }
        let lit = literals.get(lit_at..lit_at + llen).ok_or(Error::Corrupt)?;
        if out.len() + llen + mlen > max_out {
            return Err(Error::TooLarge);
        }
        out.extend_from_slice(lit);
        lit_at += llen;
        let off = offset as usize;
        if off > out.len() - frame_start {
            return Err(Error::Corrupt);
        }
        let from = out.len() - off;
        for k in 0..mlen {
            let b = out[from + k];
            out.push(b);
        }
    }
    if bs.pos != 0 {
        return Err(Error::Corrupt);
    }
    let rest = &literals[lit_at..];
    if out.len() + rest.len() > max_out {
        return Err(Error::TooLarge);
    }
    out.extend_from_slice(rest);
    Ok(())
}

/// Decodes the zstd frames of `src` to at most `max_out` bytes.
pub fn decompress(src: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    let mut out: Vec<u8> = Vec::with_capacity(max_out.min(1 << 20));
    let mut i = 0usize;
    while i < src.len() {
        let m = src.get(i..i + 4).ok_or(Error::Corrupt)?;
        let magic = u32::from_le_bytes([m[0], m[1], m[2], m[3]]);
        i += 4;
        if magic & 0xFFFF_FFF0 == 0x184D_2A50 {
            let s = src.get(i..i + 4).ok_or(Error::Corrupt)?;
            i += 4 + u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize;
            continue;
        }
        if magic != MAGIC {
            return Err(Error::Corrupt);
        }
        let fd = *src.get(i).ok_or(Error::Corrupt)?;
        i += 1;
        let single = fd & 0x20 != 0;
        let checksum = fd & 0x04 != 0;
        if fd & 0x08 != 0 {
            return Err(Error::Corrupt);
        }
        if !single {
            i += 1; // window descriptor
        }
        i += match fd & 3 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        i += match fd >> 6 {
            0 => usize::from(single),
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let frame_start = out.len();
        let mut ctx = Ctx { reps: [1, 4, 8], ..Ctx::default() };
        loop {
            let h = src.get(i..i + 3).ok_or(Error::Corrupt)?;
            let v = usize::from(h[0]) | usize::from(h[1]) << 8 | usize::from(h[2]) << 16;
            i += 3;
            let last = v & 1 != 0;
            let size = v >> 3;
            match (v >> 1) & 3 {
                0 => {
                    let d = src.get(i..i + size).ok_or(Error::Corrupt)?;
                    if out.len() + size > max_out {
                        return Err(Error::TooLarge);
                    }
                    out.extend_from_slice(d);
                    i += size;
                }
                1 => {
                    let b = *src.get(i).ok_or(Error::Corrupt)?;
                    if out.len() + size > max_out {
                        return Err(Error::TooLarge);
                    }
                    out.resize(out.len() + size, b);
                    i += 1;
                }
                2 => {
                    let d = src.get(i..i + size).ok_or(Error::Corrupt)?;
                    compressed_block(d, &mut ctx, &mut out, frame_start, max_out)?;
                    i += size;
                }
                _ => return Err(Error::Corrupt),
            }
            if last {
                break;
            }
        }
        if checksum {
            i += 4;
        }
    }
    Ok(out)
}
