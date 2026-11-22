//! Extracting TAR (`.tar`), GZIP-compressed TAR (`.tar.gz` / `.tgz`), and
//! XZ-compressed TAR (`.tar.xz` / `.txz`) archives over [`Fs`].
//!
//! Clean-room implementation written from public format specifications:
//! - POSIX.1-1988 ustar, POSIX.1-2001 PAX (`x`/`g` extended headers), and
//!   GNU tar extensions (`L` longname, `K` longlink, base-256 binary sizes).
//! - RFC 1952 GZIP file format (header flags `FEXTRA`/`FNAME`/`FCOMMENT`/
//!   `FHCRC`, DEFLATE via `poly_png::inflate`, CRC-32 and ISIZE verification,
//!   and concatenated members).
//! - The `.xz` File Format 1.0.4 (Stream Header, Block Header, LZMA2 via
//!   `poly_compress::lzma::lzma2`, None/CRC-32/CRC-64/SHA-256 checks, Index,
//!   Stream Footer, and concatenated streams).
//!
//! Safety guarantees match [`crate::zip`]:
//! - Every member path is validated before touching the file system:
//!   backslashes become `/`, leading slashes, Windows drive prefixes, `..`
//!   segments, NUL bytes and invalid names are refused ([`TarError::UnsafePath`]).
//! - Special entries (symbolic links, hard links, character/block devices,
//!   FIFOs, volume headers) are safely skipped without touching the file system.
//! - Extraction always creates a fresh destination folder (never overwriting
//!   existing files); on any error mid-extraction, the partial folder is removed.
//! - Archive size, decompressed stream size, member count, path depth, per-file
//!   size and total extracted bytes are bounded by [`Limits`].

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use poly_png::InflateError;

use crate::fs::{Fs, FsError, Kind, create_dir_all};
use crate::ops::{delete_permanently, validate_name};
use crate::path::{file_name, join, normalize, split_ext};
use crate::zip::{ExtractReport, Limits, crc32, sanitize_path, unique_dir_name};

const BLOCK: usize = 512;
const MAX_META_PAYLOAD: usize = 1024 * 1024;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const GZIP_METHOD_DEFLATE: u8 = 8;
const GZIP_FHCRC: u8 = 0x02;
const GZIP_FEXTRA: u8 = 0x04;
const GZIP_FNAME: u8 = 0x08;
const GZIP_FCOMMENT: u8 = 0x10;
const GZIP_FRESERVED: u8 = 0xe0;

const XZ_HEADER_MAGIC: [u8; 6] = [0xfd, b'7', b'z', b'X', b'Z', 0x00];
const XZ_FOOTER_MAGIC: [u8; 2] = *b"YZ";
const XZ_FILTER_LZMA2: u64 = 0x21;

/// Why reading or extracting a TAR, GZIP, or XZ archive failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TarError {
    Fs(FsError),
    NotTar,
    NotGzip,
    NotXz,
    Truncated,
    Corrupt(&'static str),
    Unsupported(&'static str),
    BadChecksum,
    UnsafePath,
    TooLarge,
    TooManyEntries,
    Inflate(InflateError),
}

impl TarError {
    /// A short human-readable message for the UI status line.
    pub fn message(&self) -> String {
        match self {
            TarError::Fs(e) => e.message().into(),
            TarError::NotTar => "Not a valid TAR archive".into(),
            TarError::NotGzip => "Not a valid GZIP archive".into(),
            TarError::NotXz => "Not a valid XZ archive".into(),
            TarError::Truncated => "The archive is incomplete".into(),
            TarError::Corrupt(why) => format!("Corrupt archive ({why})"),
            TarError::Unsupported(why) => format!("Unsupported archive feature ({why})"),
            TarError::BadChecksum => "Archive failed checksum verification".into(),
            TarError::UnsafePath => "Archive contains an unsafe file path".into(),
            TarError::TooLarge => "Archive is too large to extract".into(),
            TarError::TooManyEntries => "Archive has too many files".into(),
            TarError::Inflate(e) => format!("GZIP decompression failed ({e})"),
        }
    }
}

impl fmt::Display for TarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl From<FsError> for TarError {
    fn from(e: FsError) -> Self {
        TarError::Fs(e)
    }
}

impl From<InflateError> for TarError {
    fn from(e: InflateError) -> Self {
        match e {
            InflateError::TooLarge => TarError::TooLarge,
            InflateError::Truncated => TarError::Truncated,
            other => TarError::Inflate(other),
        }
    }
}

/// What kind of entry a TAR header describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Special,
}

/// One entry listed in a TAR archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub size: u64,
    pub kind: EntryKind,
    /// Unix permission bits (`0o777`).
    pub mode: u16,
    data_offset: usize,
}

// ---- Checksums (CRC-64-XZ, SHA-256) ----------------------------------------

/// ECMA-182 reflected CRC-64 lookup table (used by `.xz` check type `0x04`).
const CRC64_XZ_TABLE: [u64; 256] = {
    let mut table = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u64;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xc96c_5795_d787_0f42 } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// CRC-64 (ECMA-182 reflected, as `.xz` uses).
pub fn crc64_xz(data: &[u8]) -> u64 {
    let mut c = !0u64;
    for &b in data {
        c = CRC64_XZ_TABLE[((c ^ u64::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// SHA-256 (FIPS 180-4, used by `.xz` check type `0x0A`).
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];
    let compress = |h: &mut [u32; 8], blk: &[u8; 64]| {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([blk[4 * i], blk[4 * i + 1], blk[4 * i + 2], blk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, delta) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(delta);
        }
    };

    let mut chunks = data.chunks_exact(64);
    for blk in &mut chunks {
        let arr: &[u8; 64] = blk.try_into().expect("64-byte chunk");
        compress(&mut h, arr);
    }
    let rem = chunks.remainder();
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let total_blocks = if rem.len() < 56 { 1 } else { 2 };
    let bit_len = (data.len() as u64).wrapping_mul(8);
    tail[total_blocks * 64 - 8..total_blocks * 64].copy_from_slice(&bit_len.to_be_bytes());
    for idx in 0..total_blocks {
        let arr: &[u8; 64] = tail[idx * 64..(idx + 1) * 64].try_into().expect("64-byte tail");
        compress(&mut h, arr);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn u16_le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn u64_le(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

// ---- GZIP (RFC 1952) -------------------------------------------------------

/// Decode a GZIP stream (RFC 1952), including concatenated members, verifying
/// header flags, optional FHCRC, member CRC-32, and ISIZE.
///
/// Output is bounded by `max_out` bytes across all concatenated members.
pub fn gunzip(data: &[u8], max_out: usize) -> Result<Vec<u8>, TarError> {
    if data.len() < 2 || data[..2] != GZIP_MAGIC {
        return Err(TarError::NotGzip);
    }
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < data.len() {
        let member_start = pos;
        let hdr = data.get(pos..pos + 10).ok_or(TarError::Truncated)?;
        if hdr[..2] != GZIP_MAGIC {
            return Err(TarError::Corrupt("bad GZIP member header"));
        }
        if hdr[2] != GZIP_METHOD_DEFLATE {
            return Err(TarError::Unsupported("GZIP compression method"));
        }
        let flg = hdr[3];
        if flg & GZIP_FRESERVED != 0 {
            return Err(TarError::Corrupt("reserved GZIP flags set"));
        }
        pos += 10;

        if flg & GZIP_FEXTRA != 0 {
            let xlen = usize::from(u16_le(data, pos).ok_or(TarError::Truncated)?);
            pos = pos.checked_add(2).and_then(|v| v.checked_add(xlen)).ok_or(TarError::Truncated)?;
            if pos > data.len() {
                return Err(TarError::Truncated);
            }
        }
        if flg & GZIP_FNAME != 0 {
            let rest = data.get(pos..).ok_or(TarError::Truncated)?;
            let nul = rest.iter().position(|&b| b == 0).ok_or(TarError::Truncated)?;
            pos += nul + 1;
        }
        if flg & GZIP_FCOMMENT != 0 {
            let rest = data.get(pos..).ok_or(TarError::Truncated)?;
            let nul = rest.iter().position(|&b| b == 0).ok_or(TarError::Truncated)?;
            pos += nul + 1;
        }
        if flg & GZIP_FHCRC != 0 {
            let stored = u16_le(data, pos).ok_or(TarError::Truncated)?;
            let computed = (crc32(&data[member_start..pos]) & 0xffff) as u16;
            if stored != computed {
                return Err(TarError::BadChecksum);
            }
            pos += 2;
        }

        let stream = data.get(pos..).ok_or(TarError::Truncated)?;
        let remaining = max_out.saturating_sub(out.len());
        let (part, used) = poly_png::inflate::inflate(stream, remaining)?;
        pos = pos.checked_add(used).ok_or(TarError::Truncated)?;

        let want_crc = u32_le(data, pos).ok_or(TarError::Truncated)?;
        let want_isize = u32_le(data, pos + 4).ok_or(TarError::Truncated)?;
        pos += 8;

        if crc32(&part) != want_crc || (part.len() as u32) != want_isize {
            return Err(TarError::BadChecksum);
        }
        if out.is_empty() {
            out = part;
        } else {
            out.extend_from_slice(&part);
        }
    }
    Ok(out)
}

// ---- XZ (`.xz` File Format 1.0.4) ------------------------------------------

fn xz_varint(src: &[u8], pos: &mut usize) -> Result<u64, TarError> {
    let mut v = 0u64;
    for shift in 0..9 {
        let b = *src.get(*pos).ok_or(TarError::Truncated)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << (7 * shift);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(TarError::Corrupt("XZ integer overflow"))
}

fn xz_check_len(check_type: u8) -> Result<usize, TarError> {
    match check_type {
        0x00 => Ok(0),
        0x01 => Ok(4),
        0x04 => Ok(8),
        0x0a => Ok(32),
        _ => Err(TarError::Unsupported("XZ check type")),
    }
}

fn verify_xz_check(check_type: u8, check_bytes: &[u8], data: &[u8]) -> Result<(), TarError> {
    match check_type {
        0x00 => Ok(()),
        0x01 => {
            let stored = u32_le(check_bytes, 0).ok_or(TarError::Truncated)?;
            if crc32(data) == stored { Ok(()) } else { Err(TarError::BadChecksum) }
        }
        0x04 => {
            let stored = u64_le(check_bytes, 0).ok_or(TarError::Truncated)?;
            if crc64_xz(data) == stored { Ok(()) } else { Err(TarError::BadChecksum) }
        }
        0x0a => {
            if check_bytes == sha256(data) {
                Ok(())
            } else {
                Err(TarError::BadChecksum)
            }
        }
        _ => Err(TarError::Unsupported("XZ check type")),
    }
}

/// Decode an XZ stream (`.xz` File Format 1.0.4), including multi-block and
/// concatenated streams, verifying Stream Header CRC-32, Block Header CRC-32,
/// block integrity checks (None/CRC-32/CRC-64/SHA-256), Index records + CRC-32,
/// and Stream Footer.
///
/// Output is bounded by `max_out` bytes across all streams.
pub fn unxz(data: &[u8], max_out: usize) -> Result<Vec<u8>, TarError> {
    if data.len() < 6 || data[..6] != XZ_HEADER_MAGIC {
        return Err(TarError::NotXz);
    }
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < data.len() {
        let hdr = data.get(pos..pos + 12).ok_or(TarError::Truncated)?;
        if hdr[..6] != XZ_HEADER_MAGIC {
            return Err(TarError::Corrupt("bad XZ stream header"));
        }
        let stream_flags = [hdr[6], hdr[7]];
        if stream_flags[0] != 0 || stream_flags[1] > 0x0f {
            return Err(TarError::Corrupt("invalid XZ stream flags"));
        }
        let stored_hdr_crc = u32_le(hdr, 8).ok_or(TarError::Truncated)?;
        if crc32(&stream_flags) != stored_hdr_crc {
            return Err(TarError::BadChecksum);
        }
        let check_type = stream_flags[1] & 0x0f;
        let check_len = xz_check_len(check_type)?;
        pos += 12;

        let mut records: Vec<(u64, u64)> = Vec::new();
        loop {
            let size_byte = *data.get(pos).ok_or(TarError::Truncated)?;
            if size_byte == 0 {
                break;
            }
            let header_len = (usize::from(size_byte) + 1) * 4;
            let bhdr =
                data.get(pos..pos.checked_add(header_len).ok_or(TarError::Truncated)?).ok_or(TarError::Truncated)?;
            let stored_bhdr_crc = u32_le(bhdr, header_len - 4).ok_or(TarError::Truncated)?;
            if crc32(&bhdr[..header_len - 4]) != stored_bhdr_crc {
                return Err(TarError::BadChecksum);
            }

            let bflags = bhdr[1];
            if bflags & 0x3c != 0 {
                return Err(TarError::Corrupt("reserved XZ block flags set"));
            }
            let num_filters = usize::from(bflags & 0x03) + 1;
            let mut h = 2usize;
            let header_body_end = header_len - 4;
            let header_body = &bhdr[..header_body_end];
            let want_compressed = if bflags & 0x40 != 0 { Some(xz_varint(header_body, &mut h)?) } else { None };
            let want_uncompressed = if bflags & 0x80 != 0 { Some(xz_varint(header_body, &mut h)?) } else { None };

            if num_filters != 1 {
                return Err(TarError::Unsupported("XZ filter chain"));
            }
            let filter_id = xz_varint(header_body, &mut h)?;
            let props_len = xz_varint(header_body, &mut h)?;
            if filter_id != XZ_FILTER_LZMA2 {
                return Err(TarError::Unsupported("XZ filter"));
            }
            if props_len != 1 {
                return Err(TarError::Corrupt("invalid LZMA2 properties size"));
            }
            let dict_prop = *header_body.get(h).ok_or(TarError::Corrupt("missing LZMA2 properties"))?;
            if dict_prop > 40 {
                return Err(TarError::Corrupt("invalid LZMA2 dictionary size"));
            }
            h += 1;
            if header_body[h..].iter().any(|&b| b != 0) {
                return Err(TarError::Corrupt("non-zero XZ block header padding"));
            }
            pos += header_len;

            let remaining = max_out.saturating_sub(out.len());
            let (part, used) = poly_compress::lzma::lzma2(&data[pos..], remaining).map_err(|e| match e {
                poly_compress::Error::TooLarge => TarError::TooLarge,
                poly_compress::Error::Unsupported => TarError::Unsupported("LZMA2 feature"),
                poly_compress::Error::Corrupt => TarError::Corrupt("LZMA2 decompression failed"),
            })?;
            if want_compressed.is_some_and(|cs| cs != used as u64)
                || want_uncompressed.is_some_and(|us| us != part.len() as u64)
            {
                return Err(TarError::Corrupt("XZ block size mismatch"));
            }
            pos = pos.checked_add(used).ok_or(TarError::Truncated)?;

            let pad = (4 - (used % 4)) % 4;
            let pad_bytes =
                data.get(pos..pos.checked_add(pad).ok_or(TarError::Truncated)?).ok_or(TarError::Truncated)?;
            if pad_bytes.iter().any(|&b| b != 0) {
                return Err(TarError::Corrupt("non-zero XZ block padding"));
            }
            pos += pad;

            let check_bytes =
                data.get(pos..pos.checked_add(check_len).ok_or(TarError::Truncated)?).ok_or(TarError::Truncated)?;
            verify_xz_check(check_type, check_bytes, &part)?;
            pos += check_len;

            let unpadded = (header_len as u64)
                .checked_add(used as u64)
                .and_then(|v| v.checked_add(check_len as u64))
                .ok_or(TarError::Corrupt("XZ record size overflow"))?;
            records.push((unpadded, part.len() as u64));

            if out.is_empty() {
                out = part;
            } else {
                out.extend_from_slice(&part);
            }
        }

        // Index
        let index_start = pos;
        if data.get(pos) != Some(&0x00) {
            return Err(TarError::Corrupt("missing XZ index indicator"));
        }
        pos += 1;
        let rec_count = xz_varint(data, &mut pos)?;
        if rec_count != records.len() as u64 {
            return Err(TarError::Corrupt("XZ index record count mismatch"));
        }
        for (want_unpadded, want_uncompressed) in &records {
            let unpadded = xz_varint(data, &mut pos)?;
            let uncompressed = xz_varint(data, &mut pos)?;
            if unpadded != *want_unpadded || uncompressed != *want_uncompressed {
                return Err(TarError::Corrupt("XZ index record mismatch"));
            }
        }
        while !(pos - index_start).is_multiple_of(4) {
            if *data.get(pos).ok_or(TarError::Truncated)? != 0 {
                return Err(TarError::Corrupt("non-zero XZ index padding"));
            }
            pos += 1;
        }
        let stored_index_crc = u32_le(data, pos).ok_or(TarError::Truncated)?;
        if crc32(&data[index_start..pos]) != stored_index_crc {
            return Err(TarError::BadChecksum);
        }
        pos += 4;
        let index_len = (pos - index_start) as u64;

        // Stream Footer (12 bytes)
        let footer = data.get(pos..pos + 12).ok_or(TarError::Truncated)?;
        if footer[10..12] != XZ_FOOTER_MAGIC {
            return Err(TarError::Corrupt("bad XZ stream footer magic"));
        }
        let stored_footer_crc = u32_le(footer, 0).ok_or(TarError::Truncated)?;
        if crc32(&footer[4..10]) != stored_footer_crc {
            return Err(TarError::BadChecksum);
        }
        let backward_size = (u64::from(u32_le(footer, 4).ok_or(TarError::Truncated)?) + 1) * 4;
        if backward_size != index_len {
            return Err(TarError::Corrupt("XZ backward size mismatch"));
        }
        if footer[8..10] != stream_flags {
            return Err(TarError::Corrupt("XZ stream flags mismatch"));
        }
        pos += 12;

        // Optional 4-byte NUL stream padding between or after streams.
        while data.get(pos..pos + 4) == Some(&[0, 0, 0, 0]) {
            pos += 4;
        }
    }
    Ok(out)
}

// ---- TAR (POSIX ustar + GNU longname + PAX) --------------------------------

fn cstr(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Parse an octal or GNU base-256 numeric TAR header field.
fn parse_tar_num(field: &[u8]) -> Option<u64> {
    if field.is_empty() {
        return Some(0);
    }
    // GNU base-256 binary encoding: MSB set.
    if field[0] & 0x80 != 0 {
        // Bit 6 is sign; reject negative values.
        if field[0] & 0x40 != 0 {
            return None;
        }
        let mut acc = u64::from(field[0] & 0x3f);
        for &b in &field[1..] {
            if acc > (u64::MAX >> 8) {
                return None;
            }
            acc = (acc << 8) | u64::from(b);
        }
        return Some(acc);
    }
    let nul = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let text = core::str::from_utf8(&field[..nul]).ok()?;
    let digits = text.trim_matches(' ');
    if digits.is_empty() { Some(0) } else { u64::from_str_radix(digits, 8).ok() }
}

/// Verify the 512-byte TAR header checksum (accepting both unsigned POSIX sum
/// and signed legacy sum).
fn verify_header_checksum(h: &[u8]) -> bool {
    let Some(stored) = parse_tar_num(&h[148..156]) else {
        return false;
    };
    let mut unsigned_sum = 0u64;
    let mut signed_sum = 0i64;
    for (i, &b) in h.iter().enumerate() {
        let byte = if (148..156).contains(&i) { b' ' } else { b };
        unsigned_sum += u64::from(byte);
        signed_sum += i64::from(byte as i8);
    }
    unsigned_sum == stored || (signed_sum >= 0 && signed_sum as u64 == stored)
}

#[derive(Default)]
struct PaxAttrs {
    path: Option<String>,
    size: Option<u64>,
    gnu_sparse: bool,
}

/// Parse POSIX.1-2001 PAX extended header records (`<len> <key>=<value>\n`).
fn parse_pax(data: &[u8], attrs: &mut PaxAttrs, is_global: bool) -> Result<(), TarError> {
    let mut pos = 0usize;
    while pos < data.len() {
        let rest = &data[pos..];
        let sp = rest.iter().position(|&b| b == b' ').ok_or(TarError::Corrupt("bad PAX record"))?;
        let len_str = core::str::from_utf8(&rest[..sp]).map_err(|_| TarError::Corrupt("bad PAX length"))?;
        let rec_len: usize = len_str.parse().map_err(|_| TarError::Corrupt("bad PAX length"))?;
        if rec_len <= sp + 2 {
            return Err(TarError::Corrupt("invalid PAX record length"));
        }
        let end = pos.checked_add(rec_len).ok_or(TarError::Corrupt("PAX length overflow"))?;
        let rec = data.get(pos..end).ok_or(TarError::Corrupt("truncated PAX record"))?;
        if *rec.last().unwrap_or(&0) != b'\n' {
            return Err(TarError::Corrupt("PAX record missing newline"));
        }
        let kv = &rec[sp + 1..rec.len() - 1];
        let eq = kv.iter().position(|&b| b == b'=').ok_or(TarError::Corrupt("PAX record missing '='"))?;
        let key = &kv[..eq];
        let val = &kv[eq + 1..];
        match key {
            b"path" => {
                if val.is_empty() && is_global {
                    attrs.path = None;
                } else {
                    attrs.path = Some(String::from_utf8_lossy(val).into_owned());
                }
            }
            b"size" => {
                if val.is_empty() && is_global {
                    attrs.size = None;
                } else {
                    let s = core::str::from_utf8(val).map_err(|_| TarError::Corrupt("bad PAX size"))?;
                    attrs.size = Some(s.parse::<u64>().map_err(|_| TarError::Corrupt("bad PAX size"))?);
                }
            }
            b"GNU.sparse.major" | b"GNU.sparse.size" => {
                attrs.gnu_sparse = true;
            }
            _ => {}
        }
        pos = end;
    }
    Ok(())
}

/// List every entry in an uncompressed TAR archive `tar`, bounded by
/// `max_entries`.
pub fn members(tar: &[u8], max_entries: usize) -> Result<Vec<Member>, TarError> {
    if tar.is_empty() {
        return Ok(Vec::new());
    }
    if tar.len() < BLOCK {
        return if tar.iter().all(|&b| b == 0) { Err(TarError::Truncated) } else { Err(TarError::NotTar) };
    }

    let mut out = Vec::new();
    let mut at = 0usize;
    let mut headers_seen = 0usize;
    let max_headers = max_entries.saturating_mul(4).saturating_add(64);
    let mut global_pax = PaxAttrs::default();
    let mut next_pax: Option<PaxAttrs> = None;
    let mut next_gnu_name: Option<String> = None;

    while at < tar.len() {
        let Some(h) = tar.get(at..at + BLOCK) else {
            return Err(TarError::Truncated);
        };
        if h.iter().all(|&b| b == 0) {
            // End-of-archive zero block(s).
            break;
        }
        if !verify_header_checksum(h) {
            return if headers_seen == 0 {
                Err(TarError::NotTar)
            } else {
                Err(TarError::Corrupt("bad TAR header checksum"))
            };
        }
        headers_seen += 1;
        if headers_seen > max_headers {
            return Err(TarError::TooManyEntries);
        }

        let header_size = parse_tar_num(&h[124..136]).ok_or(TarError::Corrupt("bad TAR entry size"))?;
        let typeflag = h[156];

        // Metadata headers apply to subsequent entries and use their own header size.
        if matches!(typeflag, b'L' | b'K' | b'x' | b'X' | b'g') {
            if header_size > MAX_META_PAYLOAD as u64 {
                return Err(TarError::TooLarge);
            }
            let payload_len = usize::try_from(header_size).map_err(|_| TarError::TooLarge)?;
            let data_start = at + BLOCK;
            let data_end = data_start.checked_add(payload_len).ok_or(TarError::Truncated)?;
            let payload = tar.get(data_start..data_end).ok_or(TarError::Truncated)?;
            let padded = payload_len.checked_next_multiple_of(BLOCK).ok_or(TarError::TooLarge)?;
            let next_at = data_start.checked_add(padded).ok_or(TarError::Truncated)?;
            if next_at > tar.len() {
                return Err(TarError::Truncated);
            }
            match typeflag {
                b'L' => {
                    let end = payload.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
                    next_gnu_name = Some(String::from_utf8_lossy(&payload[..end]).into_owned());
                }
                b'K' => {
                    // Long link target for symlinks/hardlinks: safely ignored.
                }
                b'x' | b'X' => {
                    let pax = next_pax.get_or_insert_with(PaxAttrs::default);
                    parse_pax(payload, pax, false)?;
                }
                b'g' => {
                    parse_pax(payload, &mut global_pax, true)?;
                }
                _ => {}
            }
            at = next_at;
            continue;
        }

        let pax = next_pax.take();
        let gnu_name = next_gnu_name.take();
        let gnu_sparse = pax.as_ref().is_some_and(|p| p.gnu_sparse) || global_pax.gnu_sparse;

        let size = pax.as_ref().and_then(|p| p.size).or(global_pax.size).unwrap_or(header_size);
        let size_usize = usize::try_from(size).map_err(|_| TarError::TooLarge)?;
        let data_start = at + BLOCK;
        let data_end = data_start.checked_add(size_usize).ok_or(TarError::Truncated)?;
        if data_end > tar.len() {
            return Err(TarError::Truncated);
        }
        let padded = size_usize.checked_next_multiple_of(BLOCK).ok_or(TarError::TooLarge)?;
        let next_at = data_start.checked_add(padded).ok_or(TarError::Truncated)?;
        if next_at > tar.len() {
            return Err(TarError::Truncated);
        }

        let name = if let Some(p) = pax.and_then(|p| p.path) {
            p
        } else if let Some(g) = gnu_name {
            g
        } else if let Some(gp) = global_pax.path.clone() {
            gp
        } else {
            let base = cstr(&h[..100]);
            if &h[257..263] == b"ustar\0" {
                let prefix = cstr(&h[345..500]);
                if !prefix.is_empty() { format!("{prefix}/{base}") } else { base }
            } else {
                base
            }
        };

        let mode = parse_tar_num(&h[100..108]).map_or(0, |m| (m & 0o777) as u16);

        let kind = if gnu_sparse {
            EntryKind::Special
        } else {
            match typeflag {
                b'0' | b'7' => EntryKind::File,
                0 => {
                    if name.ends_with('/') {
                        EntryKind::Dir
                    } else {
                        EntryKind::File
                    }
                }
                b'5' => EntryKind::Dir,
                b'2' => EntryKind::Symlink,
                // Volume headers (V), hard links (1), devices (3/4), FIFOs (6), GNU dump/sparse (D/M/N/S)
                _ => EntryKind::Special,
            }
        };

        // GNU volume headers carry a label, not a file system entry.
        if typeflag != b'V' {
            if out.len() >= max_entries {
                return Err(TarError::TooManyEntries);
            }
            out.push(Member { name, size, kind, mode, data_offset: data_start });
        }
        at = next_at;
    }

    Ok(out)
}

/// Read the raw payload bytes of a regular-file `Member` from `tar`.
pub fn read_member<'a>(tar: &'a [u8], m: &Member, max_file_bytes: usize) -> Result<&'a [u8], TarError> {
    let len = usize::try_from(m.size).map_err(|_| TarError::TooLarge)?;
    if len > max_file_bytes {
        return Err(TarError::TooLarge);
    }
    let end = m.data_offset.checked_add(len).ok_or(TarError::Truncated)?;
    tar.get(m.data_offset..end).ok_or(TarError::Truncated)
}

// ---- Extraction ------------------------------------------------------------

/// Default folder stem for `archive_path` when the archive does not contain a
/// single top-level directory (strips `.tar.gz`, `.tar.xz`, `.tgz`, `.txz`,
/// `.tar`, etc.).
fn archive_stem(archive_path: &str) -> &str {
    let name = file_name(archive_path);
    let (stem, _) = split_ext(name);
    if validate_name(stem).is_ok() { stem } else { "Extracted" }
}

/// Maximum decompressed TAR byte budget derived from `limits`.
fn max_tar_bytes(limits: &Limits) -> usize {
    let overhead = (limits.max_entries as u64).saturating_mul(1024);
    usize::try_from(limits.max_total_bytes.saturating_add(overhead)).unwrap_or(usize::MAX)
}

fn read_archive_file(fs: &dyn Fs, archive_path: &str, dest_parent: &str, limits: &Limits) -> Result<Vec<u8>, TarError> {
    if fs.stat(dest_parent)?.kind != Kind::Dir {
        return Err(TarError::Fs(FsError::NotADirectory));
    }
    let meta = fs.stat(archive_path)?;
    if meta.kind == Kind::Dir {
        return Err(TarError::Fs(FsError::IsADirectory));
    }
    if meta.size > limits.max_archive_bytes as u64 {
        return Err(TarError::TooLarge);
    }
    let data = fs.read_head(archive_path, limits.max_archive_bytes.saturating_add(1))?;
    if data.len() > limits.max_archive_bytes {
        return Err(TarError::TooLarge);
    }
    Ok(data)
}

/// Extract an uncompressed `.tar` archive at `archive_path` into a new folder
/// inside `dest_parent` with [`Limits::default`].
pub fn extract_tar(fs: &dyn Fs, archive_path: &str, dest_parent: &str) -> Result<ExtractReport, TarError> {
    extract_tar_with_limits(fs, archive_path, dest_parent, &Limits::default())
}

/// Extract an uncompressed `.tar` archive enforcing `limits`.
pub fn extract_tar_with_limits(
    fs: &dyn Fs,
    archive_path: &str,
    dest_parent: &str,
    limits: &Limits,
) -> Result<ExtractReport, TarError> {
    let archive_path = normalize(archive_path);
    let dest_parent = normalize(dest_parent);
    let data = read_archive_file(fs, &archive_path, &dest_parent, limits)?;
    extract_tar_bytes(fs, &data, &archive_path, &dest_parent, limits)
}

/// Extract a `.tar.gz` / `.tgz` archive at `archive_path` into a new folder
/// inside `dest_parent` with [`Limits::default`].
pub fn extract_tar_gz(fs: &dyn Fs, archive_path: &str, dest_parent: &str) -> Result<ExtractReport, TarError> {
    extract_tar_gz_with_limits(fs, archive_path, dest_parent, &Limits::default())
}

/// Extract a `.tar.gz` / `.tgz` archive enforcing `limits`.
pub fn extract_tar_gz_with_limits(
    fs: &dyn Fs,
    archive_path: &str,
    dest_parent: &str,
    limits: &Limits,
) -> Result<ExtractReport, TarError> {
    let archive_path = normalize(archive_path);
    let dest_parent = normalize(dest_parent);
    let data = read_archive_file(fs, &archive_path, &dest_parent, limits)?;
    let tar = gunzip(&data, max_tar_bytes(limits))?;
    extract_tar_bytes(fs, &tar, &archive_path, &dest_parent, limits)
}

/// Extract a `.tar.xz` / `.txz` archive at `archive_path` into a new folder
/// inside `dest_parent` with [`Limits::default`].
pub fn extract_tar_xz(fs: &dyn Fs, archive_path: &str, dest_parent: &str) -> Result<ExtractReport, TarError> {
    extract_tar_xz_with_limits(fs, archive_path, dest_parent, &Limits::default())
}

/// Extract a `.tar.xz` / `.txz` archive enforcing `limits`.
pub fn extract_tar_xz_with_limits(
    fs: &dyn Fs,
    archive_path: &str,
    dest_parent: &str,
    limits: &Limits,
) -> Result<ExtractReport, TarError> {
    let archive_path = normalize(archive_path);
    let dest_parent = normalize(dest_parent);
    let data = read_archive_file(fs, &archive_path, &dest_parent, limits)?;
    let tar = unxz(&data, max_tar_bytes(limits))?;
    extract_tar_bytes(fs, &tar, &archive_path, &dest_parent, limits)
}

/// Extract already-decompressed TAR bytes into a fresh folder inside
/// `dest_parent`, using `archive_path`'s stem when the archive has no single
/// top-level directory.
pub fn extract_tar_bytes(
    fs: &dyn Fs,
    tar: &[u8],
    archive_path: &str,
    dest_parent: &str,
    limits: &Limits,
) -> Result<ExtractReport, TarError> {
    let dest_parent = normalize(dest_parent);
    if fs.stat(&dest_parent)?.kind != Kind::Dir {
        return Err(TarError::Fs(FsError::NotADirectory));
    }

    let entries = members(tar, limits.max_entries)?;

    // Validate every path and file size before touching the file system.
    let mut parsed: Vec<(Member, Vec<String>)> = Vec::with_capacity(entries.len());
    let mut planned_bytes = 0u64;
    for m in entries {
        let segs = sanitize_path(&m.name, limits.max_depth).map_err(|_| TarError::UnsafePath)?;
        if segs.is_empty() {
            continue;
        }
        if m.kind == EntryKind::File {
            if m.size > limits.max_file_bytes as u64 {
                return Err(TarError::TooLarge);
            }
            planned_bytes = planned_bytes.checked_add(m.size).ok_or(TarError::TooLarge)?;
            if planned_bytes > limits.max_total_bytes {
                return Err(TarError::TooLarge);
            }
        }
        parsed.push((m, segs));
    }

    // Deduplicate a single common top-level directory (avoiding `foo/foo/...`).
    let single_root: Option<String> = {
        let mut root: Option<&str> = None;
        let mut all_under_dir = !parsed.is_empty();
        for (m, segs) in &parsed {
            if segs.len() == 1 && m.kind != EntryKind::Dir {
                all_under_dir = false;
                break;
            }
            match root {
                None => root = Some(segs[0].as_str()),
                Some(r) if r == segs[0].as_str() => {}
                Some(_) => {
                    all_under_dir = false;
                    break;
                }
            }
        }
        if all_under_dir { root.map(String::from) } else { None }
    };

    let base_name = single_root.as_deref().unwrap_or_else(|| archive_stem(archive_path));
    let existing: Vec<String> = fs.read_dir(&dest_parent)?.into_iter().map(|e| e.name).collect();
    let folder_name = unique_dir_name(existing.iter().map(String::as_str), base_name);
    let dest = join(&dest_parent, &folder_name);
    fs.create_dir(&dest)?;

    let strip_root = single_root.is_some();
    match extract_entries(fs, tar, &dest, &parsed, strip_root, limits) {
        Ok((files, dirs, bytes, skipped_symlinks)) => {
            Ok(ExtractReport { dest, name: folder_name, files, dirs, bytes, skipped_symlinks })
        }
        Err(e) => {
            let _ = delete_permanently(fs, &dest);
            Err(e)
        }
    }
}

fn extract_entries(
    fs: &dyn Fs,
    tar: &[u8],
    dest: &str,
    parsed: &[(Member, Vec<String>)],
    strip_root: bool,
    limits: &Limits,
) -> Result<(usize, usize, u64, usize), TarError> {
    let (mut files, mut dirs, mut total_bytes, mut skipped_symlinks) = (0usize, 0usize, 0u64, 0usize);

    for (m, segs) in parsed {
        let rel = if strip_root { &segs[1..] } else { segs.as_slice() };
        if rel.is_empty() {
            continue;
        }
        match m.kind {
            EntryKind::Symlink => {
                skipped_symlinks += 1;
                continue;
            }
            EntryKind::Special => {
                continue;
            }
            EntryKind::Dir => {
                let mut target = String::from(dest);
                for seg in rel {
                    target = join(&target, seg);
                }
                if !fs.exists(&target) {
                    create_dir_all(fs, &target)?;
                    dirs += 1;
                }
                continue;
            }
            EntryKind::File => {}
        }

        let mut target = String::from(dest);
        for seg in rel {
            target = join(&target, seg);
        }

        // Ensure implicit parent directories exist.
        if rel.len() > 1 {
            let mut parent_dir = String::from(dest);
            for seg in &rel[..rel.len() - 1] {
                parent_dir = join(&parent_dir, seg);
                if !fs.exists(&parent_dir) {
                    fs.create_dir(&parent_dir)?;
                    dirs += 1;
                }
            }
        }

        let remaining = limits.max_total_bytes.saturating_sub(total_bytes);
        let max_this = limits.max_file_bytes.min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let bytes = read_member(tar, m, max_this)?;
        total_bytes = total_bytes.checked_add(bytes.len() as u64).ok_or(TarError::TooLarge)?;
        if total_bytes > limits.max_total_bytes {
            return Err(TarError::TooLarge);
        }

        fs.write(&target, bytes)?;
        if m.mode != 0 {
            let mode = (m.mode & 0o777) | 0o600;
            let _ = fs.set_mode(&target, mode);
        }
        files += 1;
    }

    Ok((files, dirs, total_bytes, skipped_symlinks))
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;

    pub(crate) fn write_tar_header(out: &mut Vec<u8>, name: &str, prefix: &str, typeflag: u8, mode: u16, body: &[u8]) {
        let mut h = [0u8; BLOCK];
        let nbytes = name.as_bytes();
        h[..nbytes.len().min(100)].copy_from_slice(&nbytes[..nbytes.len().min(100)]);
        h[100..108].copy_from_slice(format!("{mode:07o}\0").as_bytes());
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", body.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].copy_from_slice(b"        ");
        h[156] = typeflag;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let pbytes = prefix.as_bytes();
        h[345..345 + pbytes.len().min(155)].copy_from_slice(&pbytes[..pbytes.len().min(155)]);
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(body);
        out.resize(out.len().next_multiple_of(BLOCK), 0);
    }

    pub(crate) fn build_tar(entries: &[(&str, u8, u16, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(name, kind, mode, body) in entries {
            write_tar_header(&mut out, name, "", kind, mode, body);
        }
        out.resize(out.len() + 2 * BLOCK, 0);
        out
    }

    /// Encode raw bytes as a valid RFC 1952 GZIP member using stored DEFLATE blocks.
    pub(crate) fn build_gzip(data: &[u8]) -> Vec<u8> {
        build_gzip_with_options(data, None, None, None, false)
    }

    pub(crate) fn build_gzip_with_options(
        data: &[u8],
        extra: Option<&[u8]>,
        fname: Option<&[u8]>,
        fcomment: Option<&[u8]>,
        fhcrc: bool,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut flg = 0u8;
        if extra.is_some() {
            flg |= GZIP_FEXTRA;
        }
        if fname.is_some() {
            flg |= GZIP_FNAME;
        }
        if fcomment.is_some() {
            flg |= GZIP_FCOMMENT;
        }
        if fhcrc {
            flg |= GZIP_FHCRC;
        }
        out.extend_from_slice(&[0x1f, 0x8b, 8, flg, 0, 0, 0, 0, 0, 3]);
        if let Some(ex) = extra {
            out.extend_from_slice(&(ex.len() as u16).to_le_bytes());
            out.extend_from_slice(ex);
        }
        if let Some(name) = fname {
            out.extend_from_slice(name);
            out.push(0);
        }
        if let Some(cmt) = fcomment {
            out.extend_from_slice(cmt);
            out.push(0);
        }
        if fhcrc {
            let hc = (crc32(&out) & 0xffff) as u16;
            out.extend_from_slice(&hc.to_le_bytes());
        }
        // Stored DEFLATE blocks (max 65535 bytes each).
        if data.is_empty() {
            out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
        } else {
            let mut pos = 0;
            while pos < data.len() {
                let end = (pos + 65535).min(data.len());
                let chunk = &data[pos..end];
                let last = u8::from(end == data.len());
                let len = chunk.len() as u16;
                out.push(last);
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(&(!len).to_le_bytes());
                out.extend_from_slice(chunk);
                pos = end;
            }
        }
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out
    }

    fn push_varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push(((v as u8) & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    /// Encode raw bytes as a valid `.xz` stream with LZMA2 uncompressed chunks
    /// and the specified check type (`0x00` None, `0x01` CRC-32, `0x04` CRC-64,
    /// `0x0a` SHA-256).
    pub(crate) fn build_xz(data: &[u8], check_type: u8) -> Vec<u8> {
        build_xz_blocks(&[data], check_type)
    }

    pub(crate) fn build_xz_blocks(blocks: &[&[u8]], check_type: u8) -> Vec<u8> {
        let mut out = Vec::new();
        // Stream Header
        let stream_flags = [0u8, check_type];
        out.extend_from_slice(&XZ_HEADER_MAGIC);
        out.extend_from_slice(&stream_flags);
        out.extend_from_slice(&crc32(&stream_flags).to_le_bytes());

        let mut records: Vec<(u64, u64)> = Vec::new();
        for &blk in blocks {
            // LZMA2 uncompressed chunk(s) + end marker
            let mut lzma2_payload = Vec::new();
            let mut first = true;
            for chunk in blk.chunks(65536) {
                lzma2_payload.push(if first { 1 } else { 2 });
                first = false;
                let n = (chunk.len() - 1) as u16;
                lzma2_payload.extend_from_slice(&n.to_be_bytes());
                lzma2_payload.extend_from_slice(chunk);
            }
            lzma2_payload.push(0); // LZMA2 end marker

            // Block Header: 1 filter (LZMA2), dict prop = 0
            let mut bhdr = alloc::vec![0u8, 0x00, 0x21, 0x01, 0x00];
            while !(bhdr.len() + 4).is_multiple_of(4) {
                bhdr.push(0);
            }
            let header_len = bhdr.len() + 4;
            bhdr[0] = ((header_len / 4) - 1) as u8;
            let bcrc = crc32(&bhdr);
            bhdr.extend_from_slice(&bcrc.to_le_bytes());
            out.extend_from_slice(&bhdr);

            out.extend_from_slice(&lzma2_payload);
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            let check_len = match check_type {
                0x00 => 0,
                0x01 => {
                    out.extend_from_slice(&crc32(blk).to_le_bytes());
                    4
                }
                0x04 => {
                    out.extend_from_slice(&crc64_xz(blk).to_le_bytes());
                    8
                }
                0x0a => {
                    out.extend_from_slice(&sha256(blk));
                    32
                }
                _ => panic!("unsupported check type in test helper"),
            };
            records.push(((header_len + lzma2_payload.len() + check_len) as u64, blk.len() as u64));
        }

        // Index
        let index_start = out.len();
        out.push(0x00);
        push_varint(&mut out, records.len() as u64);
        for (unpadded, uncompressed) in records {
            push_varint(&mut out, unpadded);
            push_varint(&mut out, uncompressed);
        }
        while !(out.len() - index_start).is_multiple_of(4) {
            out.push(0);
        }
        let icrc = crc32(&out[index_start..]);
        out.extend_from_slice(&icrc.to_le_bytes());
        let index_len = out.len() - index_start;

        // Stream Footer
        let backward_size = ((index_len / 4) - 1) as u32;
        let mut fbody = [0u8; 6];
        fbody[..4].copy_from_slice(&backward_size.to_le_bytes());
        fbody[4..6].copy_from_slice(&stream_flags);
        out.extend_from_slice(&crc32(&fbody).to_le_bytes());
        out.extend_from_slice(&fbody);
        out.extend_from_slice(&XZ_FOOTER_MAGIC);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use crate::testfs::{MemFs, StdFs};

    fn pax_record(key: &str, val: &str) -> Vec<u8> {
        let mut len = key.len() + val.len() + 3; // space + '=' + '\n'
        loop {
            let candidate = format!("{len} {key}={val}\n");
            if candidate.len() == len {
                return candidate.into_bytes();
            }
            len = candidate.len();
        }
    }

    #[test]
    fn checksum_standards_match_test_vectors() {
        assert_eq!(crc64_xz(b"123456789"), 0x995d_c9bb_df19_39fa);
        assert_eq!(crc64_xz(b""), 0);
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23, 0xb0,
                0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn extracts_tar_files_dirs_permissions_and_avoids_collisions() {
        let tar = build_tar(&[
            ("readme.txt", b'0', 0o644, b"hello tar\n"),
            ("scripts/", b'5', 0o755, b""),
            ("scripts/run.sh", b'0', 0o755, b"#!/bin/sh\necho hi\n"),
            ("nested/implicit/file.dat", b'0', 0o600, b"data"),
        ]);
        let fs = MemFs::new();
        fs.create_dir("/work").unwrap();
        fs.write("/work/bundle.tar", &tar).unwrap();

        let report = extract_tar(&fs, "/work/bundle.tar", "/work").unwrap();
        assert_eq!(report.dest, "/work/bundle");
        assert_eq!(report.name, "bundle");
        assert_eq!(report.files, 3);
        assert_eq!(report.dirs, 3);
        assert_eq!(report.bytes, 10 + 18 + 4);
        assert_eq!(fs.read_to_string("/work/bundle/readme.txt").unwrap(), "hello tar\n");
        assert_eq!(fs.read_to_string("/work/bundle/scripts/run.sh").unwrap(), "#!/bin/sh\necho hi\n");
        assert_eq!(fs.stat("/work/bundle/scripts/run.sh").unwrap().mode, 0o755);
        assert_eq!(fs.read_to_string("/work/bundle/nested/implicit/file.dat").unwrap(), "data");

        // Second extraction creates "bundle 2", third creates "bundle 3".
        let r2 = extract_tar(&fs, "/work/bundle.tar", "/work").unwrap();
        assert_eq!(r2.dest, "/work/bundle 2");
        let r3 = extract_tar(&fs, "/work/bundle.tar", "/work").unwrap();
        assert_eq!(r3.dest, "/work/bundle 3");
    }

    #[test]
    fn deduplicates_single_top_level_directory_including_dot_slash() {
        let tar = build_tar(&[
            ("./", b'5', 0o755, b""),
            ("./myproj/", b'5', 0o755, b""),
            ("./myproj/Cargo.toml", b'0', 0o644, b"[package]\n"),
            ("./myproj/src/lib.rs", b'0', 0o644, b"// ok\n"),
        ]);
        let fs = StdFs::new();
        fs.create_dir("/dl").unwrap();
        fs.write("/dl/release.tar", &tar).unwrap();

        let report = extract_tar(&fs, "/dl/release.tar", "/dl").unwrap();
        assert_eq!(report.dest, "/dl/myproj");
        assert_eq!(report.name, "myproj");
        assert_eq!(fs.read_to_string("/dl/myproj/Cargo.toml").unwrap(), "[package]\n");
        assert_eq!(fs.read_to_string("/dl/myproj/src/lib.rs").unwrap(), "// ok\n");
        assert!(!fs.exists("/dl/myproj/myproj"));
    }

    #[test]
    fn supports_ustar_prefix_gnu_longname_pax_and_base256_size() {
        let mut tar = Vec::new();
        // 1. POSIX ustar prefix
        write_tar_header(&mut tar, "file.txt", "deep/prefix/path", b'0', 0o644, b"ustar");

        // 2. GNU longname ('L') > 100 chars
        let long_name = format!("gnu/{}/target.txt", "segment_".repeat(14));
        let mut long_payload = long_name.as_bytes().to_vec();
        long_payload.push(0);
        write_tar_header(&mut tar, "././@LongLink", "", b'L', 0, &long_payload);
        write_tar_header(&mut tar, "truncated_placeholder", "", b'0', 0o644, b"gnu-long");

        // 3. GNU longlink ('K') + symlink ('2') should be skipped cleanly
        write_tar_header(&mut tar, "././@LongLink", "", b'K', 0, b"/etc/passwd\0");
        write_tar_header(&mut tar, "evil_link", "", b'2', 0o777, b"");

        // 4. PAX global ('g') + per-file ('x') override
        let g_pax = pax_record("comment", "global-meta");
        write_tar_header(&mut tar, "pax_global_header", "", b'g', 0, &g_pax);

        let pax_path = format!("pax/{}/hello.txt", "dir_".repeat(30));
        let mut x_pax = pax_record("path", &pax_path);
        x_pax.extend_from_slice(&pax_record("size", "9"));
        write_tar_header(&mut tar, "PaxHeaders.0/hello", "", b'x', 0, &x_pax);
        // Header says size 0, PAX overrides to 9 bytes.
        let mut h = [0u8; BLOCK];
        h[..5].copy_from_slice(b"short");
        h[100..108].copy_from_slice(b"0000644\0");
        h[124..136].copy_from_slice(b"00000000000\0");
        h[148..156].copy_from_slice(b"        ");
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&h);
        tar.extend_from_slice(b"pax-bytes");
        tar.resize(tar.len().next_multiple_of(BLOCK), 0);

        // 5. GNU base-256 binary size encoding
        let mut b256 = [0u8; BLOCK];
        b256[..8].copy_from_slice(b"b256.bin");
        b256[100..108].copy_from_slice(b"0000644\0");
        b256[124] = 0x80; // base-256 positive marker
        b256[135] = 4; // size = 4
        b256[148..156].copy_from_slice(b"        ");
        b256[156] = b'0';
        b256[257..263].copy_from_slice(b"ustar ");
        let sum: u32 = b256.iter().map(|&b| u32::from(b)).sum();
        b256[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&b256);
        tar.extend_from_slice(b"1234");
        tar.resize(tar.len().next_multiple_of(BLOCK), 0);

        tar.resize(tar.len() + 2 * BLOCK, 0);

        let fs = MemFs::new();
        fs.create_dir("/out").unwrap();
        fs.write("/out/rich.tar", &tar).unwrap();

        let report = extract_tar(&fs, "/out/rich.tar", "/out").unwrap();
        assert_eq!(report.dest, "/out/rich");
        assert_eq!(report.files, 4);
        assert_eq!(report.skipped_symlinks, 1);
        assert_eq!(fs.read_to_string("/out/rich/deep/prefix/path/file.txt").unwrap(), "ustar");
        assert_eq!(fs.read_to_string(&format!("/out/rich/{long_name}")).unwrap(), "gnu-long");
        assert_eq!(fs.read_to_string(&format!("/out/rich/{pax_path}")).unwrap(), "pax-bytes");
        assert_eq!(fs.read_to_string("/out/rich/b256.bin").unwrap(), "1234");
        assert!(!fs.exists("/out/rich/evil_link"));
    }

    #[test]
    fn skips_special_entries_and_handles_empty_tar() {
        let tar = build_tar(&[
            ("keep.txt", b'0', 0o644, b"ok"),
            ("sym", b'2', 0o777, b""),
            ("hard", b'1', 0o644, b""),
            ("chrdev", b'3', 0o600, b""),
            ("blkdev", b'4', 0o600, b""),
            ("pipe", b'6', 0o644, b""),
            ("VOL_LABEL", b'V', 0, b""),
        ]);
        let fs = MemFs::new();
        fs.create_dir("/d").unwrap();
        fs.write("/d/spec.tar", &tar).unwrap();

        let report = extract_tar(&fs, "/d/spec.tar", "/d").unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(report.skipped_symlinks, 1);
        assert_eq!(fs.read_to_string("/d/spec/keep.txt").unwrap(), "ok");
        for skipped in ["sym", "hard", "chrdev", "blkdev", "pipe", "VOL_LABEL"] {
            assert!(!fs.exists(&format!("/d/spec/{skipped}")));
        }

        // Empty tar (two zero blocks) creates an empty folder.
        let empty = alloc::vec![0u8; 2 * BLOCK];
        fs.write("/d/empty.tar", &empty).unwrap();
        let report = extract_tar(&fs, "/d/empty.tar", "/d").unwrap();
        assert_eq!(report.dest, "/d/empty");
        assert_eq!(report.files, 0);
        assert_eq!(fs.stat("/d/empty").unwrap().kind, Kind::Dir);
    }

    #[test]
    fn rejects_path_traversal_in_ustar_gnu_and_pax_without_creating_dest() {
        for bad in ["../escape.txt", "/etc/passwd", "a/../../b", "C:\\evil.txt", "./../x"] {
            let tar = build_tar(&[(bad, b'0', 0o644, b"x")]);
            let fs = MemFs::new();
            fs.create_dir("/d").unwrap();
            fs.write("/d/bad.tar", &tar).unwrap();
            assert_eq!(extract_tar(&fs, "/d/bad.tar", "/d"), Err(TarError::UnsafePath), "path={bad:?}");
            assert!(!fs.exists("/d/bad"));
        }

        // NUL byte injected through PAX path header
        let mut tar = Vec::new();
        let x_pax = pax_record("path", "safe/\0evil.txt");
        write_tar_header(&mut tar, "PaxHeaders", "", b'x', 0, &x_pax);
        write_tar_header(&mut tar, "benign.txt", "", b'0', 0o644, b"x");
        tar.resize(tar.len() + 2 * BLOCK, 0);
        let fs = MemFs::new();
        fs.create_dir("/d").unwrap();
        fs.write("/d/paxbad.tar", &tar).unwrap();
        assert_eq!(extract_tar(&fs, "/d/paxbad.tar", "/d"), Err(TarError::UnsafePath));
        assert!(!fs.exists("/d/paxbad"));
    }

    #[test]
    fn rejects_corrupt_truncated_and_oversized_tar() {
        let fs = MemFs::new();
        fs.create_dir("/d").unwrap();

        fs.write("/d/not.tar", b"plain text").unwrap();
        assert_eq!(extract_tar(&fs, "/d/not.tar", "/d"), Err(TarError::NotTar));
        assert!(!fs.exists("/d/not"));

        let good = build_tar(&[("a.txt", b'0', 0o644, &[b'a'; 700])]);
        // Truncated inside file payload
        fs.write("/d/trunc.tar", &good[..BLOCK + 300]).unwrap();
        assert_eq!(extract_tar(&fs, "/d/trunc.tar", "/d"), Err(TarError::Truncated));

        // Corrupt checksum on first header -> NotTar
        let mut bad_first = good.clone();
        bad_first[0] ^= 1;
        fs.write("/d/bad1.tar", &bad_first).unwrap();
        assert_eq!(extract_tar(&fs, "/d/bad1.tar", "/d"), Err(TarError::NotTar));

        // Corrupt checksum on second header -> Corrupt
        let two = build_tar(&[("a.txt", b'0', 0o644, b"ok"), ("b.txt", b'0', 0o644, b"no")]);
        let mut bad_second = two.clone();
        bad_second[2 * BLOCK] ^= 1;
        fs.write("/d/bad2.tar", &bad_second).unwrap();
        assert!(matches!(extract_tar(&fs, "/d/bad2.tar", "/d"), Err(TarError::Corrupt(_))));
        assert!(!fs.exists("/d/bad2"));

        // Size limit exceeded
        let limits = Limits { max_total_bytes: 500, ..Limits::default() };
        fs.write("/d/big.tar", &good).unwrap();
        assert_eq!(extract_tar_with_limits(&fs, "/d/big.tar", "/d", &limits), Err(TarError::TooLarge));
        assert!(!fs.exists("/d/big"));
    }

    #[test]
    fn gunzip_handles_all_header_flags_concatenated_members_and_errors() {
        let part1 = b"Hello from member one! ";
        let part2 = b"And member two.";
        let gz1 = build_gzip_with_options(part1, Some(b"\x01\x02extra"), Some(b"orig.tar"), Some(b"a comment"), true);
        let gz2 = build_gzip(part2);

        assert_eq!(gunzip(&gz1, 1024).unwrap(), part1);

        let mut concat = gz1.clone();
        concat.extend_from_slice(&gz2);
        let mut want = part1.to_vec();
        want.extend_from_slice(part2);
        assert_eq!(gunzip(&concat, 1024).unwrap(), want);

        // Reserved flags rejected
        let mut bad_flg = gz2.clone();
        bad_flg[3] |= 0x20;
        assert!(matches!(gunzip(&bad_flg, 1024), Err(TarError::Corrupt(_))));

        // Bad FHCRC rejected
        let mut bad_hcrc = gz1.clone();
        // FHCRC sits right before the DEFLATE payload: 10 + (2+7) + 9 + 10 = 38
        bad_hcrc[38] ^= 1;
        assert_eq!(gunzip(&bad_hcrc, 1024), Err(TarError::BadChecksum));

        // Bad trailer CRC-32 rejected
        let mut bad_crc = gz2.clone();
        let n = bad_crc.len();
        bad_crc[n - 8] ^= 1;
        assert_eq!(gunzip(&bad_crc, 1024), Err(TarError::BadChecksum));

        // Bad ISIZE rejected
        let mut bad_isize = gz2.clone();
        bad_isize[n - 1] ^= 1;
        assert_eq!(gunzip(&bad_isize, 1024), Err(TarError::BadChecksum));

        // Truncated at any prefix fails cleanly
        for cut in 0..gz1.len() {
            assert!(gunzip(&gz1[..cut], 1024).is_err(), "cut={cut}");
        }

        // TooLarge enforced
        assert_eq!(gunzip(&concat, 10), Err(TarError::TooLarge));
        assert_eq!(gunzip(b"not gzip", 1024), Err(TarError::NotGzip));
    }

    #[test]
    fn unxz_verifies_all_checks_multiblock_concatenated_and_truncation() {
        let payload = b"The quick brown fox jumps over the lazy dog. ".repeat(20);
        for check in [0x00u8, 0x01, 0x04, 0x0a] {
            let xz = build_xz(&payload, check);
            assert_eq!(unxz(&xz, 4096).unwrap(), payload, "check={check:#x}");
            // Corrupting a check byte (when check is present) fails with BadChecksum.
            if check != 0x00 {
                let mut bad = xz.clone();
                // Flip a byte right inside the payload data so check mismatches.
                let off = 12 + 8 + 3; // stream hdr (12) + block hdr (8) + lzma2 hdr (3)
                bad[off] ^= 0xff;
                assert_eq!(unxz(&bad, 4096), Err(TarError::BadChecksum), "check={check:#x}");
            }
        }

        // Multi-block + concatenated streams with 4-byte NUL stream padding.
        let s1 = build_xz_blocks(&[b"block-one|", b"block-two|"], 0x04);
        let s2 = build_xz(b"stream-two", 0x01);
        let mut combined = s1;
        combined.extend_from_slice(&[0, 0, 0, 0]);
        combined.extend_from_slice(&s2);
        assert_eq!(unxz(&combined, 4096).unwrap(), b"block-one|block-two|stream-two");

        // Truncating at any prefix fails cleanly without panicking.
        let single = build_xz(b"hello xz", 0x04);
        for cut in 0..single.len() {
            assert!(unxz(&single[..cut], 4096).is_err(), "cut={cut}");
        }

        // TooLarge & NotXz
        assert_eq!(unxz(&single, 4), Err(TarError::TooLarge));
        assert_eq!(unxz(b"not xz", 4096), Err(TarError::NotXz));
    }

    #[test]
    fn extracts_tar_gz_tgz_tar_xz_and_txz_with_proper_stems() {
        let tar = build_tar(&[("a.txt", b'0', 0o644, b"alpha"), ("sub/b.txt", b'0', 0o755, b"beta")]);
        let gz = build_gzip(&tar);
        let xz = build_xz(&tar, 0x04);

        let fs = MemFs::new();
        fs.create_dir("/work").unwrap();
        fs.write("/work/pack.tar.gz", &gz).unwrap();
        fs.write("/work/short.tgz", &gz).unwrap();
        fs.write("/work/pkg.tar.xz", &xz).unwrap();
        fs.write("/work/tiny.txz", &xz).unwrap();

        let r1 = extract_tar_gz(&fs, "/work/pack.tar.gz", "/work").unwrap();
        assert_eq!(r1.dest, "/work/pack");
        assert_eq!(fs.read_to_string("/work/pack/a.txt").unwrap(), "alpha");
        assert_eq!(fs.read_to_string("/work/pack/sub/b.txt").unwrap(), "beta");

        let r2 = extract_tar_gz(&fs, "/work/short.tgz", "/work").unwrap();
        assert_eq!(r2.dest, "/work/short");
        assert_eq!(fs.read_to_string("/work/short/a.txt").unwrap(), "alpha");

        let r3 = extract_tar_xz(&fs, "/work/pkg.tar.xz", "/work").unwrap();
        assert_eq!(r3.dest, "/work/pkg");
        assert_eq!(fs.read_to_string("/work/pkg/a.txt").unwrap(), "alpha");
        assert_eq!(fs.stat("/work/pkg/sub/b.txt").unwrap().mode, 0o755);

        let r4 = extract_tar_xz(&fs, "/work/tiny.txz", "/work").unwrap();
        assert_eq!(r4.dest, "/work/tiny");
        assert_eq!(fs.read_to_string("/work/tiny/sub/b.txt").unwrap(), "beta");
    }

    #[test]
    fn extracts_real_gzip_and_xz_compressed_tar_fixtures() {
        // Real gzip(1) and xz(1) multi-block fixtures from poly-pkg.
        const REAL_GZ: &[u8] = include_bytes!("../../poly-pkg/testdata/control.tar.gz");
        const REAL_XZ: &[u8] = include_bytes!("../../poly-pkg/testdata/control.tar.xz");

        let fs = MemFs::new();
        fs.create_dir("/f").unwrap();
        fs.write("/f/ctrl.tar.gz", REAL_GZ).unwrap();
        fs.write("/f/ctrl.tar.xz", REAL_XZ).unwrap();

        let rgz = extract_tar_gz(&fs, "/f/ctrl.tar.gz", "/f").unwrap();
        assert_eq!(rgz.dest, "/f/ctrl");
        assert!(fs.read_to_string("/f/ctrl/control").unwrap().contains("Package: tiny"));

        let rxz = extract_tar_xz(&fs, "/f/ctrl.tar.xz", "/f").unwrap();
        assert_eq!(rxz.dest, "/f/ctrl 2");
        assert!(fs.read_to_string("/f/ctrl 2/control").unwrap().contains("Package: tiny"));
    }
}
