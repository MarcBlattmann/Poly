//! Reading zip archives, from PKWARE's public `.ZIP` File Format Specification (APPNOTE.TXT):
//! the end of central directory record (4.3.16) and its ZIP64 forms (4.3.14, 4.3.15), the central
//! directory's file headers (4.3.12) with their ZIP64 extra field (4.5.3), the local file headers
//! (4.3.7) and the two methods archives use, stored (0) and deflated (8; RFC 1951, decoded by
//! `poly_png::inflate`), each entry's CRC-32 checked (4.4.7).
//!
//! The archive is a byte slice; a large one can be opened from its end only:
//! [`central_directory`] finds where the directory is from the last bytes, [`Archive::from_directory`]
//! reads the entries from the directory's bytes, and an entry's data is read at its offset.
//!
//! [`part_name`] decodes the percent escapes of an Open Packaging Conventions part name (ECMA-376
//! Part 2, the format of NuGet packages), where a file `a b.txt` is stored as `a%20b.txt`.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Why an archive or an entry could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No end of central directory record in the last bytes.
    NotZip,
    /// A record lies outside the bytes, or its signature is wrong.
    Corrupt,
    /// A method other than stored or deflated, or an encrypted entry.
    Unsupported,
    /// The data's CRC-32 or size is not the header's.
    BadChecksum,
    /// The entry is larger than the caller allows.
    TooLarge,
}

const EOCD: u32 = 0x0605_4b50;
const EOCD64: u32 = 0x0606_4b50;
const EOCD64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Where the central directory is: its offset in the archive, its size and how many entries it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Directory {
    pub offset: u64,
    pub size: u64,
    pub entries: u64,
}

/// How many bytes from the end of an archive [`central_directory`] needs at most: the end record
/// with the longest comment, the ZIP64 locator and the ZIP64 end record.
pub const TAIL: usize = 22 + 0xFFFF + 20 + 56;

/// The central directory of an archive of `len` bytes whose last bytes are `tail` (at most
/// [`TAIL`] of them: the whole archive if it is smaller).
pub fn central_directory(tail: &[u8], len: u64) -> Result<Directory, Error> {
    let tail_start = len.checked_sub(tail.len() as u64).ok_or(Error::Corrupt)?;
    // The end record is the last one with its signature whose comment reaches the end.
    let mut at = tail.len().checked_sub(22).ok_or(Error::NotZip)?;
    loop {
        if u32_at(tail, at) == Some(EOCD) {
            let comment = usize::from(u16_at(tail, at + 20).ok_or(Error::Corrupt)?);
            if at + 22 + comment == tail.len() {
                break;
            }
        }
        if at == 0 {
            return Err(Error::NotZip);
        }
        at -= 1;
    }
    let entries = u64::from(u16_at(tail, at + 10).ok_or(Error::Corrupt)?);
    let size = u64::from(u32_at(tail, at + 12).ok_or(Error::Corrupt)?);
    let offset = u64::from(u32_at(tail, at + 16).ok_or(Error::Corrupt)?);
    let dir = Directory { offset, size, entries };
    // ZIP64: a locator just before the end record points to the ZIP64 end record.
    if at >= 20 && u32_at(tail, at - 20) == Some(EOCD64_LOCATOR) {
        let record = u64_at(tail, at - 20 + 8).ok_or(Error::Corrupt)?;
        let o = record.checked_sub(tail_start).ok_or(Error::Corrupt)? as usize;
        if u32_at(tail, o) != Some(EOCD64) {
            return Err(Error::Corrupt);
        }
        return Ok(Directory {
            entries: u64_at(tail, o + 32).ok_or(Error::Corrupt)?,
            size: u64_at(tail, o + 40).ok_or(Error::Corrupt)?,
            offset: u64_at(tail, o + 48).ok_or(Error::Corrupt)?,
        });
    }
    Ok(dir)
}

/// One file (or folder) of the archive, from the central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name as stored, `/` between folders (UTF-8, or code page 437 read as Latin-1).
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    /// Where its local file header is.
    pub local_offset: u64,
}

impl Entry {
    /// Whether it is a folder (its name ends with `/`).
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

/// The entries of an archive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Archive {
    pub entries: Vec<Entry>,
}

impl Archive {
    /// The entries of the central directory `dir` (its bytes), which has `count` of them. The
    /// offsets in them are the archive's.
    pub fn from_directory(dir: &[u8], count: u64) -> Result<Archive, Error> {
        let mut entries = Vec::new();
        let mut at = 0usize;
        for _ in 0..count {
            if u32_at(dir, at) != Some(CENTRAL) {
                return Err(Error::Corrupt);
            }
            let field = |o: usize| u32_at(dir, at + o).ok_or(Error::Corrupt);
            let half = |o: usize| u16_at(dir, at + o).map(usize::from).ok_or(Error::Corrupt);
            let flags = half(8)? as u16;
            let method = half(10)? as u16;
            let crc32 = field(16)?;
            let mut compressed_size = u64::from(field(20)?);
            let mut size = u64::from(field(24)?);
            let (name_len, extra_len, comment_len) = (half(28)?, half(30)?, half(32)?);
            let mut local_offset = u64::from(field(42)?);
            let name_bytes = dir.get(at + 46..at + 46 + name_len).ok_or(Error::Corrupt)?;
            let name = if flags & 0x800 != 0 {
                String::from_utf8_lossy(name_bytes).into_owned()
            } else {
                name_bytes.iter().map(|&b| char::from(b)).collect()
            };
            // The ZIP64 extra field holds, in this order, the values that are 0xFFFFFFFF above.
            let extra = dir.get(at + 46 + name_len..at + 46 + name_len + extra_len).ok_or(Error::Corrupt)?;
            let mut e = 0;
            while e + 4 <= extra.len() {
                let (id, len) = (u16_at(extra, e).unwrap_or(0), usize::from(u16_at(extra, e + 2).unwrap_or(0)));
                if id == 0x0001 {
                    let mut v = e + 4;
                    for slot in [&mut size, &mut compressed_size, &mut local_offset] {
                        if *slot == u64::from(u32::MAX) {
                            *slot = u64_at(extra, v).ok_or(Error::Corrupt)?;
                            v += 8;
                        }
                    }
                }
                e += 4 + len;
            }
            entries.push(Entry { name, method, flags, crc32, compressed_size, size, local_offset });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Archive { entries })
    }

    /// The entries of a whole archive in memory.
    pub fn parse(data: &[u8]) -> Result<Archive, Error> {
        let tail = &data[data.len().saturating_sub(TAIL)..];
        let dir = central_directory(tail, data.len() as u64)?;
        let start = usize::try_from(dir.offset).map_err(|_| Error::Corrupt)?;
        let end = start.checked_add(usize::try_from(dir.size).map_err(|_| Error::Corrupt)?).ok_or(Error::Corrupt)?;
        Archive::from_directory(data.get(start..end).ok_or(Error::Corrupt)?, dir.entries)
    }

    /// The entry named `name` (as stored), without regard to ASCII case.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

/// How many bytes of the archive, from the entry's local header on, [`data_start`] needs to see.
pub const LOCAL_HEADER: usize = 30;

/// Where the entry's data starts relative to its local header, from the header's first bytes
/// (at least [`LOCAL_HEADER`]).
pub fn data_start(local: &[u8]) -> Result<u64, Error> {
    if u32_at(local, 0) != Some(LOCAL) {
        return Err(Error::Corrupt);
    }
    let name = u64::from(u16_at(local, 26).ok_or(Error::Corrupt)?);
    let extra = u64::from(u16_at(local, 28).ok_or(Error::Corrupt)?);
    Ok(30 + name + extra)
}

/// The entry's contents from its compressed bytes `raw` (exactly `compressed_size` of them), at
/// most `max` bytes, the CRC-32 checked.
pub fn decode(entry: &Entry, raw: &[u8], max: usize) -> Result<Vec<u8>, Error> {
    if entry.flags & 1 != 0 {
        return Err(Error::Unsupported);
    }
    if entry.size > max as u64 {
        return Err(Error::TooLarge);
    }
    let out = match entry.method {
        0 => raw.to_vec(),
        8 => poly_png::inflate::inflate(raw, entry.size as usize).map_err(|_| Error::Corrupt)?.0,
        _ => return Err(Error::Unsupported),
    };
    if out.len() as u64 != entry.size || poly_png::crc32(&[&out]) != entry.crc32 {
        return Err(Error::BadChecksum);
    }
    Ok(out)
}

/// The contents of `entry` of the archive `data` (whole, in memory), at most `max` bytes.
pub fn read(data: &[u8], entry: &Entry, max: usize) -> Result<Vec<u8>, Error> {
    let raw = raw(data, entry)?;
    decode(entry, raw, max)
}

/// The compressed bytes of `entry` of the archive `data`.
pub fn raw<'a>(data: &'a [u8], entry: &Entry) -> Result<&'a [u8], Error> {
    let local = usize::try_from(entry.local_offset).map_err(|_| Error::Corrupt)?;
    let start = local + data_start(data.get(local..).ok_or(Error::Corrupt)?)? as usize;
    let end =
        start.checked_add(usize::try_from(entry.compressed_size).map_err(|_| Error::Corrupt)?).ok_or(Error::Corrupt)?;
    data.get(start..end).ok_or(Error::Corrupt)
}

/// An Open Packaging Conventions part name (`lib/net45/a%20b.txt`) as a file name: `%XX` escapes
/// decoded (as UTF-8), the rest as it is.
pub fn part_name(name: &str) -> String {
    let b = name.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests;
