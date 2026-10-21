//! Poly's hive file format, version 1.0 (ADR 0066).
//!
//! Microsoft's on-disk hive format is not publicly documented, so Poly does
//! not read or write it. Applications never see a hive file: they only use
//! the documented registry services. A Poly hive file is a snapshot of one
//! hive's non-volatile keys, written whole and replaced atomically by the
//! kernel (write `<name>.hive.new`, then rename it over `<name>.hive`).
//!
//! All integers are little-endian; strings are UTF-16LE code units with a
//! count, no terminator.
//!
//! ```text
//! header (32 bytes)
//!   0  [u8; 8]  magic "POLYHIVE"
//!   8  u16      major version (1)
//!  10  u16      minor version (0)
//!  12  u32      header size (32; a reader skips to here)
//!  16  u64      sequence: incremented by every save
//!  24  u32      payload size in bytes
//!  28  u32      CRC-32 (IEEE 802.3, reflected, as zlib's crc32) of the payload
//! payload: the hive's root key record
//!   key record
//!     u16       name length (code units), then the name
//!     u16       class length (code units), then the class
//!     u64       last-write time (FILETIME: 100 ns since 1601-01-01 UTC)
//!     u32       flags: bit 0 symbolic link; other bits must be 0
//!     u32       value count, then each value record
//!     u32       subkey count, then each subkey's key record
//!   value record
//!     u16       name length (code units), then the name
//!     u32       type (REG_*)
//!     u32       data length in bytes, then the data
//! ```
//!
//! Compatibility: a reader rejects another major version. A later minor
//! version may only append data after the root key record inside the
//! payload; a 1.0 reader ignores what follows the root record. The root
//! record's name is informational (the mount point names the hive). Every
//! subkey name is 1–255 code units without `\`, unique ignoring case among
//! its siblings; value names are unique ignoring case within a key; the
//! tree is at most [`MAX_DEPTH`] deep. Anything else is
//! `STATUS_REGISTRY_CORRUPT`.

use crate::consts::*;
use crate::tree::{Value, fold};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::SEPARATOR;

pub const MAGIC: [u8; 8] = *b"POLYHIVE";
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 0;
pub const HEADER_SIZE: usize = 32;
/// Record flag: the key is a symbolic link.
pub const FLAG_LINK: u32 = 1;

/// One key of a hive file and everything below it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyImage {
    pub name: Vec<u16>,
    pub class: Vec<u16>,
    pub last_write: u64,
    pub link: bool,
    pub values: Vec<Value>,
    pub subkeys: Vec<KeyImage>,
}

/// A whole hive file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiveFile {
    pub sequence: u64,
    pub root: KeyImage,
}

/// CRC-32 (IEEE 802.3 polynomial 0x04C11DB7, reflected; initial value and
/// final XOR 0xFFFFFFFF).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn put_str(out: &mut Vec<u8>, s: &[u16]) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    for c in s {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

fn put_key(out: &mut Vec<u8>, k: &KeyImage) {
    put_str(out, &k.name);
    put_str(out, &k.class);
    out.extend_from_slice(&k.last_write.to_le_bytes());
    out.extend_from_slice(&(if k.link { FLAG_LINK } else { 0 }).to_le_bytes());
    out.extend_from_slice(&(k.values.len() as u32).to_le_bytes());
    for v in &k.values {
        put_str(out, &v.name);
        out.extend_from_slice(&v.kind.to_le_bytes());
        out.extend_from_slice(&(v.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&v.data);
    }
    out.extend_from_slice(&(k.subkeys.len() as u32).to_le_bytes());
    for s in &k.subkeys {
        put_key(out, s);
    }
}

/// Write a hive file.
pub fn encode(file: &HiveFile) -> Vec<u8> {
    let mut payload = Vec::new();
    put_key(&mut payload, &file.root);
    let mut out = Vec::with_capacity(HEADER_SIZE + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION_MAJOR.to_le_bytes());
    out.extend_from_slice(&VERSION_MINOR.to_le_bytes());
    out.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&file.sequence.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

const CORRUPT: NtStatus = STATUS_REGISTRY_CORRUPT;

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], NtStatus> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(CORRUPT)?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, NtStatus> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2 bytes")))
    }

    fn u32(&mut self) -> Result<u32, NtStatus> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64, NtStatus> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    fn string(&mut self) -> Result<Vec<u16>, NtStatus> {
        let n = self.u16()? as usize;
        Ok(self.take(n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }

    fn key(&mut self, depth: usize) -> Result<KeyImage, NtStatus> {
        if depth > MAX_DEPTH {
            return Err(CORRUPT);
        }
        let name = self.string()?;
        if depth > 0 && (name.is_empty() || name.len() > MAX_KEY_NAME_CHARS || name.contains(&SEPARATOR)) {
            return Err(CORRUPT);
        }
        let class = self.string()?;
        let last_write = self.u64()?;
        let flags = self.u32()?;
        if flags & !FLAG_LINK != 0 {
            return Err(CORRUPT);
        }
        let count = self.u32()? as usize;
        let mut values = Vec::new();
        let mut seen = BTreeSet::new();
        for _ in 0..count {
            let name = self.string()?;
            if name.len() > MAX_VALUE_NAME_CHARS || !seen.insert(fold(&name)) {
                return Err(CORRUPT);
            }
            let kind = self.u32()?;
            let len = self.u32()? as usize;
            let data = self.take(len)?.to_vec();
            values.push(Value { name, kind, data });
        }
        let count = self.u32()? as usize;
        let mut subkeys = Vec::new();
        let mut seen = BTreeSet::new();
        for _ in 0..count {
            let s = self.key(depth + 1)?;
            if !seen.insert(fold(&s.name)) {
                return Err(CORRUPT);
            }
            subkeys.push(s);
        }
        if flags & FLAG_LINK != 0 && !subkeys.is_empty() {
            return Err(CORRUPT);
        }
        Ok(KeyImage { name, class, last_write, link: flags & FLAG_LINK != 0, values, subkeys })
    }
}

/// Read a hive file: `STATUS_REGISTRY_CORRUPT` for anything that is not a
/// well-formed version 1 file.
pub fn decode(bytes: &[u8]) -> Result<HiveFile, NtStatus> {
    if bytes.len() < HEADER_SIZE || bytes[..8] != MAGIC {
        return Err(CORRUPT);
    }
    let mut r = Reader { data: bytes, pos: 8 };
    if r.u16()? != VERSION_MAJOR {
        return Err(CORRUPT);
    }
    let _minor = r.u16()?;
    let header = r.u32()? as usize;
    let sequence = r.u64()?;
    let size = r.u32()? as usize;
    let crc = r.u32()?;
    if header < HEADER_SIZE {
        return Err(CORRUPT);
    }
    let payload = header.checked_add(size).and_then(|end| bytes.get(header..end)).ok_or(CORRUPT)?;
    if crc32(payload) != crc {
        return Err(CORRUPT);
    }
    let root = Reader { data: payload, pos: 0 }.key(0)?;
    Ok(HiveFile { sequence, root })
}
