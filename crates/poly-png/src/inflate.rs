//! DEFLATE (RFC 1951) and the zlib wrapper around it (RFC 1950), decoded.
//!
//! Written from the two RFCs. The output is kept whole in memory (the
//! history window is the output itself) and is bounded by the caller's
//! `max_out`, so a hostile stream cannot grow without limit.

use alloc::vec::Vec;
use core::fmt;

/// Why a stream was not decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InflateError {
    /// The input ended inside the stream.
    Truncated,
    /// The zlib header is wrong (method, window, check bits, or a preset
    /// dictionary, which PNG does not allow).
    BadHeader,
    /// A block of the reserved type 3.
    BadBlockType,
    /// A stored block whose length and its complement disagree.
    BadStoredLength,
    /// A Huffman code that is over-subscribed, a code-length alphabet
    /// used wrongly, or a missing end-of-block code.
    BadHuffman,
    /// A bit pattern that is no code of the table, or a length or distance
    /// symbol outside its alphabet (286, 287, 30, 31).
    BadSymbol,
    /// A match reaching back before the start of the output.
    BadDistance,
    /// The output would exceed the limit.
    TooLarge,
    /// The Adler-32 of the output differs from the stream's.
    BadChecksum,
}

impl fmt::Display for InflateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InflateError::Truncated => "compressed data ends early",
            InflateError::BadHeader => "bad zlib header",
            InflateError::BadBlockType => "reserved deflate block type",
            InflateError::BadStoredLength => "stored block length mismatch",
            InflateError::BadHuffman => "invalid Huffman code",
            InflateError::BadSymbol => "invalid symbol in compressed data",
            InflateError::BadDistance => "match reaches before the start of the data",
            InflateError::TooLarge => "decompressed data is larger than allowed",
            InflateError::BadChecksum => "zlib checksum mismatch",
        })
    }
}

/// Adler-32 (RFC 1950 section 8.2) of `data`.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 bytes keep `b` below 2^32 before the reduction.
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// Reads bits least-significant first (RFC 1951 section 3.1.1).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, pos: 0, buf: 0, count: 0 }
    }

    /// `n` bits (at most 16) as a number, the first bit read the lowest.
    fn take(&mut self, n: u32) -> Result<u32, InflateError> {
        while self.count < n {
            let byte = *self.data.get(self.pos).ok_or(InflateError::Truncated)?;
            self.pos += 1;
            self.buf |= u32::from(byte) << self.count;
            self.count += 8;
        }
        let v = self.buf & ((1u32 << n) - 1);
        self.buf >>= n;
        self.count -= n;
        Ok(v)
    }

    fn bit(&mut self) -> Result<u32, InflateError> {
        self.take(1)
    }

    /// Drop the bits left in the current byte.
    fn align(&mut self) {
        self.buf = 0;
        self.count = 0;
    }

    /// The bytes not read yet (after `align`).
    fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}

const MAX_BITS: usize = 15;

/// A canonical Huffman code (RFC 1951 section 3.2.2): how many codes there
/// are of each length, and the symbols in code order (by length, then by
/// symbol value).
struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl Huffman {
    /// The code for symbols `0..lengths.len()` with these bit lengths (0:
    /// the symbol is not used). Over-subscribed sets are refused;
    /// incomplete ones are accepted (a bit pattern with no symbol is then
    /// an error when it is read).
    fn new(lengths: &[u8]) -> Result<Huffman, InflateError> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            count[usize::from(l)] += 1;
        }
        // The codes of length `len` are `first[len]..first[len] + count[len]`
        // where first[len] = (first[len - 1] + count[len - 1]) << 1; no code
        // may run past the 2^len codes that exist.
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left = (left << 1) - i32::from(c);
            if left < 0 {
                return Err(InflateError::BadHuffman);
            }
        }
        let mut offsets = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            offsets[len + 1] = offsets[len] + count[len];
        }
        let mut symbols = alloc::vec![0u16; usize::from(offsets[MAX_BITS + 1])];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offsets[usize::from(l)])] = sym as u16;
                offsets[usize::from(l)] += 1;
            }
        }
        Ok(Huffman { count, symbols })
    }

    /// The next symbol: the code's bits come most-significant first.
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, InflateError> {
        let mut code: i32 = 0; // the bits read so far
        let mut first: i32 = 0; // the first code of this length
        let mut index: i32 = 0; // the index of that code's symbol
        for len in 1..=MAX_BITS {
            code |= bits.bit()? as i32;
            let count = i32::from(self.count[len]);
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(InflateError::BadSymbol)
    }
}

/// Length codes 257..=285: the shortest match and the extra bits.
const LENGTH_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
/// Distance codes 0..=29: the shortest distance and the extra bits.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order the code-length code's lengths come in (section 3.2.7).
const CLEN_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// The fixed codes of block type 1 (section 3.2.6).
fn fixed_codes() -> (Huffman, Huffman) {
    let mut lengths = [0u8; 288];
    for (sym, l) in lengths.iter_mut().enumerate() {
        *l = match sym {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    // Both tables are complete or valid by construction.
    let lit = Huffman::new(&lengths).expect("the fixed literal code is valid");
    let dist = Huffman::new(&[5u8; 32]).expect("the fixed distance code is valid");
    (lit, dist)
}

/// The two codes of a dynamic block (section 3.2.7).
fn dynamic_codes(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), InflateError> {
    let hlit = bits.take(5)? as usize + 257;
    let hdist = bits.take(5)? as usize + 1;
    let hclen = bits.take(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(InflateError::BadHuffman);
    }
    let mut clen = [0u8; 19];
    for &slot in CLEN_ORDER.iter().take(hclen) {
        clen[slot] = bits.take(3)? as u8;
    }
    let clen_code = Huffman::new(&clen)?;
    let mut lengths = [0u8; 286 + 30];
    let total = hlit + hdist;
    let mut i = 0;
    while i < total {
        let sym = clen_code.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                // Copy the previous length 3 to 6 times.
                let prev = if i == 0 { return Err(InflateError::BadHuffman) } else { lengths[i - 1] };
                (prev, 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            _ => (0, 11 + bits.take(7)? as usize),
        };
        if i + repeat > total {
            return Err(InflateError::BadHuffman);
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    // The end-of-block code must exist.
    if lengths[256] == 0 {
        return Err(InflateError::BadHuffman);
    }
    Ok((Huffman::new(&lengths[..hlit])?, Huffman::new(&lengths[hlit..total])?))
}

/// One compressed block's symbols, until its end-of-block code.
fn inflate_block(
    bits: &mut Bits<'_>,
    lit: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
    max_out: usize,
) -> Result<(), InflateError> {
    loop {
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => {
                if out.len() >= max_out {
                    return Err(InflateError::TooLarge);
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let i = usize::from(sym - 257);
                let len = usize::from(LENGTH_BASE[i]) + bits.take(u32::from(LENGTH_EXTRA[i]))? as usize;
                let d = dist.decode(bits)?;
                if d >= 30 {
                    return Err(InflateError::BadSymbol);
                }
                let d = usize::from(d);
                let distance = usize::from(DIST_BASE[d]) + bits.take(u32::from(DIST_EXTRA[d]))? as usize;
                if distance > out.len() {
                    return Err(InflateError::BadDistance);
                }
                if out.len() + len > max_out {
                    return Err(InflateError::TooLarge);
                }
                // A match may overlap its own output (distance < length).
                let start = out.len() - distance;
                for k in 0..len {
                    out.push(out[start + k]);
                }
            }
            _ => return Err(InflateError::BadSymbol),
        }
    }
}

/// Decode a raw DEFLATE stream. Returns the output and how many bytes of
/// `data` the stream used.
pub fn inflate(data: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), InflateError> {
    let mut bits = Bits::new(data);
    let mut out = Vec::new();
    let mut fixed: Option<(Huffman, Huffman)> = None;
    loop {
        let last = bits.bit()? == 1;
        match bits.take(2)? {
            0 => {
                bits.align();
                let rest = bits.rest();
                let header = rest.get(..4).ok_or(InflateError::Truncated)?;
                let len = usize::from(u16::from_le_bytes([header[0], header[1]]));
                let nlen = u16::from_le_bytes([header[2], header[3]]);
                if nlen != !(len as u16) {
                    return Err(InflateError::BadStoredLength);
                }
                let body = rest.get(4..4 + len).ok_or(InflateError::Truncated)?;
                if out.len() + len > max_out {
                    return Err(InflateError::TooLarge);
                }
                out.extend_from_slice(body);
                bits.pos += 4 + len;
            }
            1 => {
                let (lit, dist) = fixed.get_or_insert_with(fixed_codes);
                inflate_block(&mut bits, lit, dist, &mut out, max_out)?;
            }
            2 => {
                let (lit, dist) = dynamic_codes(&mut bits)?;
                inflate_block(&mut bits, &lit, &dist, &mut out, max_out)?;
            }
            _ => return Err(InflateError::BadBlockType),
        }
        if last {
            break;
        }
    }
    // Bits left in the last byte are padding; whole bytes buffered ahead
    // were never taken (`take` reads a byte only when it needs it).
    Ok((out, bits.pos))
}

/// Decode a zlib stream (RFC 1950): the two header bytes, DEFLATE data,
/// and the big-endian Adler-32 of the output, which is checked.
pub fn zlib_decompress(data: &[u8], max_out: usize) -> Result<Vec<u8>, InflateError> {
    let &[cmf, flg, ..] = data else { return Err(InflateError::Truncated) };
    // Method 8 (deflate), a window of at most 32 KiB, the header's two
    // bytes a multiple of 31, no preset dictionary.
    if cmf & 0x0f != 8 || cmf >> 4 > 7 || (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 || flg & 0x20 != 0 {
        return Err(InflateError::BadHeader);
    }
    let (out, used) = inflate(&data[2..], max_out)?;
    let tail = data.get(2 + used..2 + used + 4).ok_or(InflateError::Truncated)?;
    if u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) != adler32(&out) {
        return Err(InflateError::BadChecksum);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// 493 bytes: a sentence three times, then every byte value.
    fn text() -> Vec<u8> {
        let mut t = Vec::new();
        for _ in 0..3 {
            t.extend_from_slice(b"It was the best of times, it was the worst of times, it was the age of wisdom, ");
        }
        t.extend(0..=255u8);
        t
    }

    // Streams made by zlib for the text above (fixed codes) and for
    // `a` x 1000 (a match of distance 1, longer than its distance).
    const FIXED: &str = "7801f32c51284f2c5628c94855484a2d2e51c84f5328c9cc4d2dd651c844c894e717e1924a4c4f0549946716a7e4e7ea28780e6ae3181899985958d9d83938b9b87978f9f8050485844544c5c42524a5a46564e5e4151495945554d5d43534b5b47574f5f40d0c8d8c4d4ccdcc2d2cadac6d6cedec1d1c9d9c5d5cdddc3d3cbdbc7d7cfdfc030283824342c3c22322a3a26362e3e2131293925352d3d23332b3b27372f3f20b0a812e2b2d2bafa8acaaaea9adab6f686c6a6e696d6befe8eceaeee9edeb9f3071d2e42953a74d9f3173d6ec3973e7cd5fb070d1e2254b972d5fb172d5ea356bd7addfb071d3e62d5bb76ddfb173d7ee3d7bf7ed3f70f0d0e123478f1d3f71f2d4e93367cf9dbf70f1d2e52b57af5dbf71f3d6ed3b77efdd7ff0f0d1e3274f9f3d7ff1f2d5eb376fdfbdfff0f1d3e72f5fbf7dfff1f3d7ef3f7ffffd0700cbfcd16e";
    const RLE: &str = "78014b1c05a360140c7b0000f9d87af8";
    // The same text with the Huffman-only strategy: a dynamic block.
    const DYNAMIC: &str = "780105c103a054411806d06cdb7db95737dbc6665bb3edccdea96e53fbff35d9b66dd7ebbd6cdbb66ddb38c7c7b082c0ae845f12c328b0f62439d00c2b08ec4a581322865160ed4972a0195610d895104109a36035058ce7c0c7b082c0ae845f12c328b0f62439d00c2b08ec4a581322865160ed4972a0195610d895104109a36035058ce7c0c7b082c0ae845f12c328b0f62439d00c2b08ec4a581322865160ed4972a0195610d895104109a36035058ce7204ad468d163c48c153b4edc78f113244c943849d264c953a44c953a4dda74e93364cc941959b266cb9e2367aedc6179f23af9f2172858a87091a2c58a972859aa7499b2e5ca57a858a97295aad5aad7a8e9ab55bb4edd7af51b346cd4b849d366cd5bb46cd5ba4ddb76ed3b087fc78054415777eadcc5eb6aba750f11f7e8697bf5eed3b75fff0103070d1e3274d8f01123478d1e3376dcf80913274d9e3275daf4193367cd9e3377defc050b172d5eb274d9f2152b57ad5e13be362272ddfa0d1b376ddeb275dbf61d3b77eddeb377dffe03070f1d3e72f4d8f113274f9d3e73f6dcf90b172f5dbe72f5daf51b376fddbe73f7defd070f1f3d7ef2f4d9f3172f5fbd7ef3f6ddfb0f1f3f7dfef2f5dbf71f3f7ffdfef3f7df7fcbfcd16e";

    #[test]
    fn adler_of_known_strings() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        // Long enough to need the reduction more than once.
        assert_eq!(adler32(&vec![0xff; 100_000]), {
            let (mut a, mut b) = (1u64, 0u64);
            for _ in 0..100_000 {
                a = (a + 255) % 65521;
                b = (b + a) % 65521;
            }
            ((b << 16) | a) as u32
        });
    }

    #[test]
    fn fixed_dynamic_and_overlapping_matches() {
        assert_eq!(zlib_decompress(&unhex(FIXED), 1 << 20).unwrap(), text());
        assert_eq!(zlib_decompress(&unhex(DYNAMIC), 1 << 20).unwrap(), text());
        assert_eq!(zlib_decompress(&unhex(RLE), 1 << 20).unwrap(), vec![b'a'; 1000]);
    }

    #[test]
    fn stored_blocks() {
        // Two stored blocks, "ab" then "c", the second the last; no zlib.
        let raw = [0, 2, 0, 0xfd, 0xff, b'a', b'b', 1, 1, 0, 0xfe, 0xff, b'c'];
        let (out, used) = inflate(&raw, 100).unwrap();
        assert_eq!(out, b"abc");
        assert_eq!(used, raw.len());
        // A wrong complement.
        let bad = [1, 2, 0, 0xfd, 0xfe, b'a', b'b'];
        assert_eq!(inflate(&bad, 100), Err(InflateError::BadStoredLength));
        // Cut short, in the header and in the data.
        assert_eq!(inflate(&raw[..3], 100), Err(InflateError::Truncated));
        assert_eq!(inflate(&raw[..6], 100), Err(InflateError::Truncated));
        // An empty last stored block.
        assert_eq!(inflate(&[1, 0, 0, 0xff, 0xff], 0).unwrap().0, b"");
    }

    #[test]
    fn the_output_limit_holds() {
        assert_eq!(zlib_decompress(&unhex(FIXED), 492), Err(InflateError::TooLarge));
        assert!(zlib_decompress(&unhex(FIXED), 493).is_ok());
        assert_eq!(zlib_decompress(&unhex(RLE), 999), Err(InflateError::TooLarge));
        assert_eq!(inflate(&[0, 2, 0, 0xfd, 0xff, 1, 2], 1), Err(InflateError::TooLarge));
    }

    #[test]
    fn damaged_streams_fail_cleanly() {
        let good = unhex(FIXED);
        // Every prefix is an error, never a panic.
        for n in 0..good.len() {
            assert!(zlib_decompress(&good[..n], 1 << 20).is_err(), "prefix {n}");
        }
        // The checksum.
        let mut bad = good.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(zlib_decompress(&bad, 1 << 20), Err(InflateError::BadChecksum));
        // The header: wrong method, a preset dictionary, bad check bits.
        for h in [[0x79, 0x01], [0x78, 0x20 | 0x1c], [0x78, 0x02]] {
            let mut z = good.clone();
            z[..2].copy_from_slice(&h);
            assert_eq!(zlib_decompress(&z, 1 << 20), Err(InflateError::BadHeader), "{h:?}");
        }
        // Block type 3.
        assert_eq!(inflate(&[0b0000_0111], 10), Err(InflateError::BadBlockType));
        // Flipping any single bit of the dynamic stream gives an error or
        // other bytes, but never a panic.
        let dynamic = unhex(DYNAMIC);
        for i in 0..dynamic.len() * 8 {
            let mut z = dynamic.clone();
            z[i / 8] ^= 1 << (i % 8);
            let _ = zlib_decompress(&z, 1 << 16);
        }
    }

    #[test]
    fn a_distance_before_the_start_is_refused() {
        // Fixed block: the literal 'a' (code 0x30 + 0x61 = 0x91, 8 bits),
        // then a length-3 match (code 257 = 0000001, 7 bits) at distance
        // code 1 (= 2, 5 bits): the output has only one byte.
        let mut w = BitWriter::default();
        w.bits(1, 1);
        w.bits(1, 2);
        w.code(0x30 + b'a' as u32, 8);
        w.code(1, 7);
        w.code(1, 5);
        w.code(0, 7);
        assert_eq!(inflate(&w.finish(), 100), Err(InflateError::BadDistance));
        // The same with distance code 0 (= 1) is fine: "aaaa".
        let mut w = BitWriter::default();
        w.bits(1, 1);
        w.bits(1, 2);
        w.code(0x30 + b'a' as u32, 8);
        w.code(1, 7);
        w.code(0, 5);
        w.code(0, 7);
        assert_eq!(inflate(&w.finish(), 100).unwrap().0, b"aaaa");
        // Distance symbols 30 and 31 do not exist.
        let mut w = BitWriter::default();
        w.bits(1, 1);
        w.bits(1, 2);
        w.code(0x30 + b'a' as u32, 8);
        w.code(1, 7);
        w.code(30, 5);
        assert_eq!(inflate(&w.finish(), 100), Err(InflateError::BadSymbol));
        // Length symbol 286 does not exist (code 0xc0 + 6 = 8 bits).
        let mut w = BitWriter::default();
        w.bits(1, 1);
        w.bits(1, 2);
        w.code(0xc0 + 6, 8);
        assert_eq!(inflate(&w.finish(), 100), Err(InflateError::BadSymbol));
    }

    #[test]
    fn over_subscribed_codes_are_refused() {
        // Three codes of length 1.
        assert!(matches!(Huffman::new(&[1, 1, 1]), Err(InflateError::BadHuffman)));
        // Two of length 1 is complete; a lone code of length 1 is accepted
        // (incomplete).
        assert!(Huffman::new(&[1, 1]).is_ok());
        assert!(Huffman::new(&[0, 1]).is_ok());
    }

    #[test]
    fn canonical_codes_follow_the_rfc_example() {
        // RFC 1951 section 3.2.2: A..H with lengths 3,3,3,3,3,2,4,4 are
        // F=00, A=010, B=011, C=100, D=101, E=110, G=1110, H=1111.
        let h = Huffman::new(&[3, 3, 3, 3, 3, 2, 4, 4]).unwrap();
        let mut w = BitWriter::default();
        for (code, len) in
            [(0b00, 2), (0b010, 3), (0b011, 3), (0b100, 3), (0b101, 3), (0b110, 3), (0b1110, 4), (0b1111, 4)]
        {
            w.code(code, len);
        }
        let bytes = w.finish();
        let mut bits = Bits::new(&bytes);
        let got: Vec<u16> = (0..8).map(|_| h.decode(&mut bits).unwrap()).collect();
        assert_eq!(got, [5, 0, 1, 2, 3, 4, 6, 7]);
    }

    #[test]
    fn a_long_stream_of_every_kind_of_block() {
        // Level-0 (stored) and level-6 streams of 200 000 bytes made by
        // zlib are in the integration tests (the icons); here: many small
        // stored blocks of 3 bytes.
        let mut z = vec![0x78, 0x01];
        let mut all = Vec::new();
        for i in 0..1000u32 {
            let b = [i as u8, (i >> 3) as u8, 7];
            z.push(u8::from(i == 999));
            z.extend_from_slice(&3u16.to_le_bytes());
            z.extend_from_slice(&(!3u16).to_le_bytes());
            z.extend_from_slice(&b);
            all.extend_from_slice(&b);
        }
        z.extend_from_slice(&adler32(&all).to_be_bytes());
        assert_eq!(zlib_decompress(&z, 1 << 20).unwrap(), all);
    }

    /// Writes bits least-significant first; Huffman codes most-significant
    /// first (RFC 1951 section 3.1.1).
    #[derive(Default)]
    struct BitWriter {
        out: Vec<u8>,
        n: u32,
    }

    impl BitWriter {
        fn bit(&mut self, b: u32) {
            if self.n.is_multiple_of(8) {
                self.out.push(0);
            }
            *self.out.last_mut().unwrap() |= (b as u8 & 1) << (self.n % 8);
            self.n += 1;
        }
        fn bits(&mut self, v: u32, n: u32) {
            for i in 0..n {
                self.bit(v >> i);
            }
        }
        fn code(&mut self, v: u32, n: u32) {
            for i in (0..n).rev() {
                self.bit(v >> i);
            }
        }
        fn finish(self) -> Vec<u8> {
            self.out
        }
    }
}
