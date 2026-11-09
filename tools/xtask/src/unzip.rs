//! Reading members of a ZIP archive, in pure Rust so that `cargo xtask
//! fonts fetch` unpacks Geist's release archive the same way on Windows
//! and Linux hosts (no `unzip` there by default; ADR 0124's rule).
//!
//! Written from the public specifications: PKWARE's APPNOTE.TXT (6.3.10)
//! for the archive — the end of central directory record, the central
//! directory's file headers, local file headers, stored (0) and deflated
//! (8) members, no encryption, no ZIP64 — and RFC 1951 for DEFLATE: stored,
//! fixed Huffman and dynamic Huffman blocks. Members are checked against
//! the CRC-32 the archive gives.

/// One member of an archive, as its central directory lists it: its name,
/// size and Unix mode, and where its data starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// The size of the unpacked data.
    pub size: usize,
    /// The Unix mode (`st_mode`) from the high 16 bits of the external
    /// file attributes, 0 when the archive was not made on Unix.
    pub mode: u32,
    method: u16,
    crc: u32,
    compressed: usize,
    local_header: usize,
}

impl Member {
    /// Whether the member is a symbolic link (its data is the target).
    pub fn is_symlink(&self) -> bool {
        self.mode & 0o170000 == 0o120000
    }
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// The central directory's entries.
pub fn members(zip: &[u8]) -> Result<Vec<Member>, String> {
    // The end of central directory record: signature 0x06054b50, within
    // the last 22 + 65535 bytes (its comment may be that long).
    let lowest = zip.len().saturating_sub(22 + 0xffff);
    let eocd = (lowest..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(zip, i) == Some(0x0605_4b50))
        .ok_or("not a ZIP archive (no end of central directory record)")?;
    let count = u16_at(zip, eocd + 10).ok_or("truncated")? as usize;
    let mut at = u32_at(zip, eocd + 16).ok_or("truncated")? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(zip, at) != Some(0x0201_4b50) {
            return Err("a bad central directory file header".into());
        }
        let field = |o: usize| u16_at(zip, at + o).ok_or("truncated");
        let method = field(10)?;
        let crc = u32_at(zip, at + 16).ok_or("truncated")?;
        let compressed = u32_at(zip, at + 20).ok_or("truncated")? as usize;
        let size = u32_at(zip, at + 24).ok_or("truncated")? as usize;
        let (name_len, extra_len, comment_len) = (field(28)? as usize, field(30)? as usize, field(32)? as usize);
        let external = u32_at(zip, at + 38).ok_or("truncated")?;
        let local_header = u32_at(zip, at + 42).ok_or("truncated")? as usize;
        let name = zip.get(at + 46..at + 46 + name_len).ok_or("truncated")?;
        out.push(Member {
            name: String::from_utf8_lossy(name).into_owned(),
            size,
            mode: external >> 16,
            method,
            crc,
            compressed,
            local_header,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// The data of member `name`.
pub fn extract(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let m = members(zip)?.into_iter().find(|e| e.name == name).ok_or_else(|| format!("{name}: not in the archive"))?;
    read(zip, &m)
}

/// Every file member (not directories) with its data, handed to `each` one
/// at a time so a large archive is never all in memory at once.
pub fn for_each(zip: &[u8], mut each: impl FnMut(&str, Vec<u8>) -> Result<(), String>) -> Result<(), String> {
    for m in members(zip)? {
        if m.name.ends_with('/') {
            continue;
        }
        let data = read(zip, &m)?;
        each(&m.name, data)?;
    }
    Ok(())
}

/// The data of member `m` (from [`members`] of the same archive).
pub fn read(zip: &[u8], m: &Member) -> Result<Vec<u8>, String> {
    let name = &m.name;
    let h = m.local_header;
    if u32_at(zip, h) != Some(0x0403_4b50) {
        return Err(format!("{name}: a bad local file header"));
    }
    let start =
        h + 30 + u16_at(zip, h + 26).ok_or("truncated")? as usize + u16_at(zip, h + 28).ok_or("truncated")? as usize;
    let data = zip.get(start..start + m.compressed).ok_or_else(|| format!("{name}: truncated"))?;
    let out = match m.method {
        0 => data.to_vec(),
        8 => inflate(data, m.size)?,
        x => return Err(format!("{name}: compression method {x} is not supported")),
    };
    if out.len() != m.size || crc32(&out) != m.crc {
        return Err(format!("{name}: the CRC-32 or size does not match"));
    }
    Ok(out)
}

/// CRC-32 (IEEE 802.3, reflected, as ZIP uses).
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// Reads bits least significant first (RFC 1951 §3.1.1).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, String> {
        let byte = *self.data.get(self.pos).ok_or("DEFLATE data ends early")?;
        let b = (u32::from(byte) >> self.bit) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Ok(b)
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

/// A canonical Huffman code (RFC 1951 §3.2.2): how many codes of each
/// length and the symbols in code order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        for i in 1..16 {
            offsets[i] = offsets[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = s as u16;
                offsets[l as usize] += 1;
            }
        }
        Huffman { counts, symbols }
    }

    /// The next symbol: codes are read most significant bit first.
    fn decode(&self, b: &mut Bits<'_>) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bit()? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("a bad Huffman code".into())
    }
}

const LENGTH_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Decompress raw DEFLATE data (`size` is the expected length, a hint).
pub fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(size);
    let mut b = Bits { data, pos: 0, bit: 0 };
    loop {
        let last = b.bit()?;
        match b.bits(2)? {
            0 => {
                b.align();
                let len = u16_at(data, b.pos).ok_or("truncated")?;
                let nlen = u16_at(data, b.pos + 2).ok_or("truncated")?;
                if len != !nlen {
                    return Err("a stored block's length is corrupt".into());
                }
                let start = b.pos + 4;
                out.extend_from_slice(data.get(start..start + len as usize).ok_or("truncated")?);
                b.pos = start + len as usize;
            }
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                block(&mut b, &mut out, &Huffman::new(&lengths), &Huffman::new(&[5; 30]))?;
            }
            2 => {
                let hlit = b.bits(5)? as usize + 257;
                let hdist = b.bits(5)? as usize + 1;
                let hclen = b.bits(4)? as usize + 4;
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut cl = [0u8; 19];
                for &i in ORDER.iter().take(hclen) {
                    cl[i] = b.bits(3)? as u8;
                }
                let clh = Huffman::new(&cl);
                let mut lengths = Vec::with_capacity(hlit + hdist);
                while lengths.len() < hlit + hdist {
                    let sym = clh.decode(&mut b)?;
                    match sym {
                        0..=15 => lengths.push(sym as u8),
                        16 => {
                            let prev = *lengths.last().ok_or("a repeat with nothing before it")?;
                            for _ in 0..3 + b.bits(2)? {
                                lengths.push(prev);
                            }
                        }
                        17 => lengths.extend(std::iter::repeat_n(0, 3 + b.bits(3)? as usize)),
                        _ => lengths.extend(std::iter::repeat_n(0, 11 + b.bits(7)? as usize)),
                    }
                }
                if lengths.len() != hlit + hdist {
                    return Err("code lengths overrun".into());
                }
                block(&mut b, &mut out, &Huffman::new(&lengths[..hlit]), &Huffman::new(&lengths[hlit..]))?;
            }
            _ => return Err("a reserved block type".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

/// One compressed block's symbols.
fn block(b: &mut Bits<'_>, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman) -> Result<(), String> {
    loop {
        let sym = lit.decode(b)? as usize;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            257..=285 => {
                let i = sym - 257;
                let len = LENGTH_BASE[i] as usize + b.bits(u32::from(LENGTH_EXTRA[i]))? as usize;
                let d = dist.decode(b)? as usize;
                if d >= 30 {
                    return Err("a bad distance code".into());
                }
                let back = DIST_BASE[d] as usize + b.bits(u32::from(DIST_EXTRA[d]))? as usize;
                if back > out.len() {
                    return Err("a distance before the start".into());
                }
                let from = out.len() - back;
                for k in 0..len {
                    out.push(out[from + k]);
                }
            }
            _ => return Err("a bad length code".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn stored_and_fixed_blocks() {
        // A stored block: BFINAL 1, BTYPE 00, aligned, LEN 3, NLEN, "abc".
        let stored = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&stored, 3).unwrap(), b"abc");
        // Fixed codes with back references (raw DEFLATE as zlib emits it).
        assert_eq!(inflate(&[0x4b, 0x4c, 0x4a, 0x4e, 0x44, 0x45, 0x00], 18).unwrap(), b"abcabcabcabcabcabc");
        let hello = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01];
        assert_eq!(inflate(&hello, 23).unwrap(), b"hello hello hello hello");
    }

    #[test]
    fn a_dynamic_block() {
        let hex = "b5ca550282501000c0abacdddddddd8addd2f91409c138bb5ec2ef198c21e1a6b2380f1719e91250e8019c2a5eef80345206e5c7c2d93480407410b0bf65b0586d7687d3e5f6787dfe4030148e4463f1443295ce6473f942b154ae546bf546b3d5ee747bfdc170349e4c67d87cb15cad37dbddfe703c9d2f38415234c372bc204ae87a93ef8aaae90fc37cbede9f2f";
        let data: Vec<u8> =
            (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect();
        let mut want = b"The quick brown fox jumps over the lazy dog. ".repeat(3);
        want.extend(32u8..127);
        assert_eq!(inflate(&data, want.len()).unwrap(), want);
    }

    #[test]
    fn a_small_archive() {
        // A ZIP with one stored member "hi.txt" = "hello\n", built here from
        // the APPNOTE's records.
        let data = b"hello\n";
        let crc = crc32(data);
        let name = b"hi.txt";
        let mut zip = Vec::new();
        let local = |zip: &mut Vec<u8>| {
            zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            zip.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            zip.extend_from_slice(&crc.to_le_bytes());
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(name);
            zip.extend_from_slice(data);
        };
        local(&mut zip);
        let cd = zip.len();
        zip.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&[0; 12]);
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(name);
        let cd_len = zip.len() - cd;
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend_from_slice(&(cd_len as u32).to_le_bytes());
        zip.extend_from_slice(&(cd as u32).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(extract(&zip, "hi.txt").unwrap(), data);
        assert!(extract(&zip, "nope").is_err());
        // A flipped byte fails the CRC.
        let mut bad = zip.clone();
        bad[30 + name.len()] ^= 1;
        assert!(extract(&bad, "hi.txt").is_err());
    }
}
