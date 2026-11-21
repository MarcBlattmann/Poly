//! Clean-room Compound File Binary Format (`[MS-CFB]`) reader for Windows
//! Installer (`.msi`) packages.
//!
//! Written strictly from Microsoft's public `[MS-CFB]` specification (v3 with
//! 512-byte sectors and v4 with 4096-byte sectors) and the public MSI stream
//! name encoding (6-bit alphabet packed into `U+3800..=U+4840`).
//!
//! All tables and sector chains are bounded and cycle-checked so malformed or
//! truncated inputs return `Err` without panicking or allocating unboundedly.

use crate::Error;
use crate::windows::program::{CFB_SIGNATURE, ReadAt};
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

/// Regular sectors are strictly below this value (`[MS-CFB]` 2.1).
pub const MAXREGSECT: u32 = 0xFFFF_FFFA;
/// End of a FAT / Mini FAT / DIFAT sector chain.
pub const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
/// Unallocated sector / DIFAT slot.
pub const FREESECT: u32 = 0xFFFF_FFFF;
/// Empty directory tree link (`NOSTREAM`, `[MS-CFB]` 2.6.1).
pub const NOSTREAM: u32 = 0xFFFF_FFFF;

/// Directory entry object types (`[MS-CFB]` 2.6.1).
pub const OBJ_UNKNOWN: u8 = 0;
pub const OBJ_STORAGE: u8 = 1;
pub const OBJ_STREAM: u8 = 2;
pub const OBJ_ROOT: u8 = 5;

/// Safety limits so crafted files cannot exhaust memory or spin.
const MAX_DIFAT_SECTORS: u32 = 1024;
const MAX_FAT_SECTORS: usize = 65_536;
const MAX_MINI_FAT_SECTORS: usize = 4096;
const MAX_DIR_SECTORS: usize = 4096;
const MAX_MINI_STREAM_BYTES: usize = 32 << 20;

/// 64-character alphabet used by MSI compressed stream names.
const MSI_ALPHABET: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz._";

/// Decode a UTF-16LE MSI compound-file stream name.
///
/// Returns `(is_table, decoded_name)`:
/// - Leading `U+4840` marks a database table stream (`is_table == true`).
/// - `U+3800..=U+47FF` encodes two 6-bit characters from [`MSI_ALPHABET`]:
///   low 6 bits first, high 6 bits second.
/// - `U+4800..=U+483F` encodes one 6-bit character from [`MSI_ALPHABET`].
/// - All other code units are decoded as literal UTF-16 characters.
pub fn decode_msi_stream_name(units: &[u16]) -> (bool, String) {
    let mut is_table = false;
    let mut i = 0;
    if units.first() == Some(&0x4840) {
        is_table = true;
        i = 1;
    }
    let mut out = String::new();
    let mut literal: Vec<u16> = Vec::new();
    let flush = |lit: &mut Vec<u16>, dst: &mut String| {
        if !lit.is_empty() {
            dst.push_str(&String::from_utf16_lossy(lit));
            lit.clear();
        }
    };
    while i < units.len() {
        let u = units[i];
        i += 1;
        if (0x3800..=0x47FF).contains(&u) {
            flush(&mut literal, &mut out);
            let v = usize::from(u - 0x3800);
            out.push(char::from(MSI_ALPHABET[v & 0x3F]));
            out.push(char::from(MSI_ALPHABET[(v >> 6) & 0x3F]));
        } else if (0x4800..=0x483F).contains(&u) {
            flush(&mut literal, &mut out);
            out.push(char::from(MSI_ALPHABET[usize::from(u - 0x4800)]));
        } else if u == 0x4840 {
            flush(&mut literal, &mut out);
            is_table = true;
        } else {
            literal.push(u);
        }
    }
    flush(&mut literal, &mut out);
    (is_table, out)
}

/// Encode an MSI stream name into UTF-16 code units using the 6-bit MSI
/// compression scheme (used for building test fixtures and matching).
pub fn encode_msi_stream_name(name: &str, is_table: bool) -> Vec<u16> {
    let idx = |b: u8| -> Option<u16> {
        match b {
            b'0'..=b'9' => Some(u16::from(b - b'0')),
            b'A'..=b'Z' => Some(10 + u16::from(b - b'A')),
            b'a'..=b'z' => Some(36 + u16::from(b - b'a')),
            b'.' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::new();
    if is_table {
        out.push(0x4840);
    }
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(a) = idx(bytes[i]) {
            if let Some(&next) = bytes.get(i + 1)
                && let Some(b) = idx(next)
            {
                out.push(0x3800 + a + (b << 6));
                i += 2;
            } else {
                out.push(0x4800 + a);
                i += 1;
            }
        } else {
            // Non-alphabet UTF-8 character: emit its UTF-16 code units directly.
            let rest = &name[i..];
            let Some(ch) = rest.chars().next() else { break };
            let mut buf = [0u16; 2];
            out.extend_from_slice(ch.encode_utf16(&mut buf));
            i += ch.len_utf8();
        }
    }
    out
}

/// One parsed `[MS-CFB]` directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// Raw UTF-16LE name (without trailing NUL).
    pub raw_name: Vec<u16>,
    /// Literal UTF-16 decoded name.
    pub literal_name: String,
    /// MSI-decoded stream name.
    pub decoded_name: String,
    /// Whether the MSI table-stream prefix `U+4840` was present.
    pub is_table: bool,
    /// Entry type (`OBJ_ROOT`, `OBJ_STORAGE`, `OBJ_STREAM`, `OBJ_UNKNOWN`).
    pub object_type: u8,
    pub left_sibling: u32,
    pub right_sibling: u32,
    pub child: u32,
    pub clsid: [u8; 16],
    pub start_sector: u32,
    pub size: u64,
}

/// A parsed `[MS-CFB]` compound file header, FAT, Mini FAT and directory tree.
#[derive(Clone, Debug)]
pub struct Cfb {
    pub major_version: u16,
    pub sector_shift: u16,
    pub mini_sector_shift: u16,
    pub mini_stream_cutoff: u32,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    root_clsid: [u8; 16],
    mini_stream_start: u32,
    mini_stream_size: u64,
    /// Direct stream children of the Root Storage.
    root_streams: Vec<DirEntry>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn read_exact_at(read: ReadAt<'_>, at: u64, len: usize) -> Result<Vec<u8>, Error> {
    let mut buf = alloc::vec![0u8; len];
    let mut got = 0;
    while got < len {
        let n = read(at.checked_add(got as u64).ok_or(Error::Corrupt)?, &mut buf[got..]);
        if n == 0 {
            return Err(Error::Truncated);
        }
        got += n;
    }
    Ok(buf)
}

fn read_sector(read: ReadAt<'_>, sector: u32, shift: u16) -> Result<Vec<u8>, Error> {
    if sector >= MAXREGSECT {
        return Err(Error::Corrupt);
    }
    let size = 1usize << shift;
    let offset = (u64::from(sector) + 1).checked_shl(u32::from(shift)).ok_or(Error::Corrupt)?;
    read_exact_at(read, offset, size)
}

fn parse_dir_entry(raw: &[u8], major_version: u16) -> Result<DirEntry, Error> {
    if raw.len() != 128 {
        return Err(Error::Corrupt);
    }
    let object_type = raw[0x42];
    let name_bytes = usize::from(u16_at(raw, 0x40).ok_or(Error::Corrupt)?).min(64);
    let unit_count = name_bytes / 2;
    let mut units = Vec::with_capacity(unit_count);
    for i in 0..unit_count {
        let u = u16_at(raw, i * 2).ok_or(Error::Corrupt)?;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    let literal_name = String::from_utf16_lossy(&units);
    let (is_table, decoded_name) = decode_msi_stream_name(&units);
    let left_sibling = u32_at(raw, 0x44).ok_or(Error::Corrupt)?;
    let right_sibling = u32_at(raw, 0x48).ok_or(Error::Corrupt)?;
    let child = u32_at(raw, 0x4C).ok_or(Error::Corrupt)?;
    let mut clsid = [0u8; 16];
    clsid.copy_from_slice(&raw[0x50..0x60]);
    let start_sector = u32_at(raw, 0x74).ok_or(Error::Corrupt)?;
    // [MS-CFB] 2.6.1: version 3 uses only the low 32 bits of Stream Size.
    let size = if major_version == 3 {
        u64::from(u32_at(raw, 0x78).ok_or(Error::Corrupt)?)
    } else {
        u64_at(raw, 0x78).ok_or(Error::Corrupt)?
    };
    Ok(DirEntry {
        raw_name: units,
        literal_name,
        decoded_name,
        is_table,
        object_type,
        left_sibling,
        right_sibling,
        child,
        clsid,
        start_sector,
        size,
    })
}

impl Cfb {
    /// Parse a compound file's header, FAT, Mini FAT and root directory tree.
    pub fn open(read: ReadAt<'_>) -> Result<Cfb, Error> {
        let head = read_exact_at(read, 0, 512)?;
        if head[..8] != CFB_SIGNATURE {
            return Err(Error::NotThisFormat);
        }
        let major_version = u16_at(&head, 0x1A).ok_or(Error::Corrupt)?;
        let byte_order = u16_at(&head, 0x1C).ok_or(Error::Corrupt)?;
        let sector_shift = u16_at(&head, 0x1E).ok_or(Error::Corrupt)?;
        let mini_sector_shift = u16_at(&head, 0x20).ok_or(Error::Corrupt)?;
        if byte_order != 0xFFFE {
            return Err(Error::Corrupt);
        }
        match (major_version, sector_shift) {
            (3, 9) | (4, 12) => {}
            _ => return Err(Error::Corrupt),
        }
        if mini_sector_shift != 6 {
            return Err(Error::Corrupt);
        }
        let num_fat_sectors = u32_at(&head, 0x2C).ok_or(Error::Corrupt)? as usize;
        let first_dir_sector = u32_at(&head, 0x30).ok_or(Error::Corrupt)?;
        let mini_stream_cutoff = u32_at(&head, 0x38).ok_or(Error::Corrupt)?;
        let first_mini_fat_sector = u32_at(&head, 0x3C).ok_or(Error::Corrupt)?;
        let num_mini_fat_sectors = u32_at(&head, 0x40).ok_or(Error::Corrupt)? as usize;
        let first_difat_sector = u32_at(&head, 0x44).ok_or(Error::Corrupt)?;
        let num_difat_sectors = u32_at(&head, 0x48).ok_or(Error::Corrupt)?;

        if num_fat_sectors == 0 || num_fat_sectors > MAX_FAT_SECTORS || num_difat_sectors > MAX_DIFAT_SECTORS {
            return Err(Error::Corrupt);
        }
        if num_mini_fat_sectors > MAX_MINI_FAT_SECTORS {
            return Err(Error::Corrupt);
        }

        // Collect FAT sector locations from header DIFAT (109 entries) + DIFAT chain.
        let mut fat_sector_ids = Vec::with_capacity(num_fat_sectors.min(1024));
        for i in 0..109 {
            if fat_sector_ids.len() >= num_fat_sectors {
                break;
            }
            let sid = u32_at(&head, 0x4C + i * 4).ok_or(Error::Corrupt)?;
            if sid == FREESECT || sid == ENDOFCHAIN {
                break;
            }
            if sid >= MAXREGSECT {
                return Err(Error::Corrupt);
            }
            fat_sector_ids.push(sid);
        }

        let entries_per_sector = (1usize << sector_shift) / 4;
        let mut difat_cur = first_difat_sector;
        let mut difat_seen = BTreeSet::new();
        for _ in 0..num_difat_sectors {
            if difat_cur == ENDOFCHAIN || difat_cur == FREESECT || fat_sector_ids.len() >= num_fat_sectors {
                break;
            }
            if !difat_seen.insert(difat_cur) {
                return Err(Error::Corrupt);
            }
            let sec = read_sector(read, difat_cur, sector_shift)?;
            for i in 0..entries_per_sector - 1 {
                if fat_sector_ids.len() >= num_fat_sectors {
                    break;
                }
                let sid = u32_at(&sec, i * 4).ok_or(Error::Corrupt)?;
                if sid == FREESECT || sid == ENDOFCHAIN {
                    break;
                }
                if sid >= MAXREGSECT {
                    return Err(Error::Corrupt);
                }
                fat_sector_ids.push(sid);
            }
            difat_cur = u32_at(&sec, (entries_per_sector - 1) * 4).ok_or(Error::Corrupt)?;
        }

        // Build flat FAT.
        let mut fat = Vec::with_capacity(fat_sector_ids.len().saturating_mul(entries_per_sector));
        for sid in fat_sector_ids {
            let sec = read_sector(read, sid, sector_shift)?;
            for chunk in sec.chunks_exact(4) {
                fat.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
        }

        // Build flat Mini FAT.
        let mut mini_fat = Vec::new();
        if first_mini_fat_sector != ENDOFCHAIN && first_mini_fat_sector != FREESECT && num_mini_fat_sectors > 0 {
            let mut cur = first_mini_fat_sector;
            let mut seen = BTreeSet::new();
            for _ in 0..num_mini_fat_sectors {
                if cur == ENDOFCHAIN || cur == FREESECT {
                    break;
                }
                if !seen.insert(cur) {
                    return Err(Error::Corrupt);
                }
                let sec = read_sector(read, cur, sector_shift)?;
                for chunk in sec.chunks_exact(4) {
                    mini_fat.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
                cur = *fat.get(cur as usize).ok_or(Error::Corrupt)?;
            }
        }

        // Read directory chain.
        let mut all_entries = Vec::new();
        let mut dir_cur = first_dir_sector;
        let mut dir_seen = BTreeSet::new();
        while dir_cur != ENDOFCHAIN && dir_cur != FREESECT {
            if dir_seen.len() >= MAX_DIR_SECTORS || !dir_seen.insert(dir_cur) {
                return Err(Error::Corrupt);
            }
            let sec = read_sector(read, dir_cur, sector_shift)?;
            for chunk in sec.chunks_exact(128) {
                all_entries.push(parse_dir_entry(chunk, major_version)?);
            }
            dir_cur = *fat.get(dir_cur as usize).ok_or(Error::Corrupt)?;
        }

        let root = all_entries.first().ok_or(Error::Corrupt)?;
        if root.object_type != OBJ_ROOT {
            return Err(Error::Corrupt);
        }
        let root_clsid = root.clsid;
        let mini_stream_start = root.start_sector;
        let mini_stream_size = root.size;

        // Walk the root storage's child red-black tree to collect direct child streams.
        let mut root_streams = Vec::new();
        if root.child != NOSTREAM {
            let mut stack = alloc::vec![root.child];
            let mut visited = BTreeSet::new();
            while let Some(id) = stack.pop() {
                if id == NOSTREAM || !visited.insert(id) {
                    continue;
                }
                let Some(entry) = all_entries.get(id as usize) else {
                    return Err(Error::Corrupt);
                };
                if entry.left_sibling != NOSTREAM {
                    stack.push(entry.left_sibling);
                }
                if entry.right_sibling != NOSTREAM {
                    stack.push(entry.right_sibling);
                }
                if entry.object_type == OBJ_STREAM {
                    root_streams.push(entry.clone());
                }
            }
        }

        Ok(Cfb {
            major_version,
            sector_shift,
            mini_sector_shift,
            mini_stream_cutoff,
            fat,
            mini_fat,
            root_clsid,
            mini_stream_start,
            mini_stream_size,
            root_streams,
        })
    }

    /// The Root Storage CLSID (16 bytes on disk).
    pub fn root_clsid(&self) -> [u8; 16] {
        self.root_clsid
    }

    /// Direct stream children of the Root Storage.
    pub fn streams(&self) -> &[DirEntry] {
        &self.root_streams
    }

    /// Find a direct table stream (with `U+4840` prefix) by its decoded table name.
    pub fn find_table(&self, table: &str) -> Option<&DirEntry> {
        self.root_streams.iter().find(|e| e.is_table && e.decoded_name == table)
    }

    /// Find a non-table direct stream by its decoded or literal name (case-insensitive).
    pub fn find_stream(&self, name: &str) -> Option<&DirEntry> {
        self.root_streams.iter().find(|e| {
            !e.is_table && (e.decoded_name.eq_ignore_ascii_case(name) || e.literal_name.eq_ignore_ascii_case(name))
        })
    }

    /// Read a stream chain from the regular FAT, bounded by `size` and `max_size`.
    fn read_fat_chain(&self, start: u32, size: u64, read: ReadAt<'_>, max_size: usize) -> Result<Vec<u8>, Error> {
        let len = usize::try_from(size).map_err(|_| Error::TooLarge)?;
        if len > max_size {
            return Err(Error::TooLarge);
        }
        if len == 0 {
            return Ok(Vec::new());
        }
        let sec_size = 1usize << self.sector_shift;
        let needed_sectors = len.div_ceil(sec_size);
        let mut out = Vec::with_capacity(len);
        let mut cur = start;
        let mut seen = BTreeSet::new();
        for _ in 0..needed_sectors {
            if cur == ENDOFCHAIN || cur == FREESECT || cur >= MAXREGSECT {
                return Err(Error::Truncated);
            }
            if !seen.insert(cur) {
                return Err(Error::Corrupt);
            }
            let sec = read_sector(read, cur, self.sector_shift)?;
            let take = (len - out.len()).min(sec_size);
            out.extend_from_slice(&sec[..take]);
            if out.len() >= len {
                break;
            }
            cur = *self.fat.get(cur as usize).ok_or(Error::Corrupt)?;
        }
        if out.len() != len {
            return Err(Error::Truncated);
        }
        Ok(out)
    }

    /// Read a direct stream entry's bytes, up to `max_size` bytes.
    pub fn read_stream(&self, entry: &DirEntry, read: ReadAt<'_>, max_size: usize) -> Result<Vec<u8>, Error> {
        let len = usize::try_from(entry.size).map_err(|_| Error::TooLarge)?;
        if len > max_size {
            return Err(Error::TooLarge);
        }
        if len == 0 {
            return Ok(Vec::new());
        }
        if entry.size < u64::from(self.mini_stream_cutoff) {
            let mini_bytes =
                self.read_fat_chain(self.mini_stream_start, self.mini_stream_size, read, MAX_MINI_STREAM_BYTES)?;
            let mini_size = 1usize << self.mini_sector_shift;
            let needed = len.div_ceil(mini_size);
            let mut out = Vec::with_capacity(len);
            let mut cur = entry.start_sector;
            let mut seen = BTreeSet::new();
            for _ in 0..needed {
                if cur == ENDOFCHAIN || cur == FREESECT || cur >= MAXREGSECT {
                    return Err(Error::Truncated);
                }
                if !seen.insert(cur) {
                    return Err(Error::Corrupt);
                }
                let off = (cur as usize).checked_mul(mini_size).ok_or(Error::Corrupt)?;
                let take = (len - out.len()).min(mini_size);
                let slice =
                    mini_bytes.get(off..off.checked_add(take).ok_or(Error::Corrupt)?).ok_or(Error::Truncated)?;
                out.extend_from_slice(slice);
                if out.len() >= len {
                    break;
                }
                cur = *self.mini_fat.get(cur as usize).ok_or(Error::Corrupt)?;
            }
            if out.len() != len {
                return Err(Error::Truncated);
            }
            Ok(out)
        } else {
            self.read_fat_chain(entry.start_sector, entry.size, read, max_size)
        }
    }
}
