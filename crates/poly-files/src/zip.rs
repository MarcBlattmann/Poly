//! Extracting ZIP archives into a folder over [`Fs`].
//!
//! Written from PKWARE's APPNOTE.TXT (6.3.10): End of Central Directory
//! (and ZIP64 EOCD / extra fields), Central Directory file headers, Local
//! File Headers, stored (method 0) and DEFLATE (method 8, via
//! `poly_png::inflate::inflate`), IEEE 802.3 CRC-32 verification, and Unix
//! permission bits.
//!
//! Safety guarantees:
//! - Every member path is validated before touching the file system:
//!   backslashes become `/`, leading slashes, Windows drive prefixes, `..`
//!   segments, NUL bytes and invalid names are refused ([`ZipError::UnsafePath`]).
//! - Extraction always goes into a brand-new folder name (never overwriting
//!   existing files); if any member fails, the partial folder is removed.
//! - Archive size, member count, path depth, per-file size and total
//!   extracted bytes are bounded by [`Limits`].
//! - Symbolic links in an archive are skipped cleanly (the [`Fs`] trait has
//!   no symlink creation, and skipping avoids turning link targets into
//!   regular files or escaping the destination folder).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use poly_png::InflateError;

use crate::fs::{Fs, FsError, Kind, create_dir_all};
use crate::ops::{delete_permanently, validate_name};
use crate::path::{file_name, join, normalize, split_ext};

/// The MIME type of ZIP archives (as [`crate::mime`] reports it).
pub const MIME: &str = "application/zip";

const SIG_LOCAL: u32 = 0x0403_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_ZIP64_EOCD: u32 = 0x0606_4b50;
const SIG_ZIP64_LOCATOR: u32 = 0x0706_4b50;
const EXTRA_ZIP64: u16 = 0x0001;

const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// Resource limits for extracting an untrusted archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum size of the `.zip` file read into memory.
    pub max_archive_bytes: usize,
    /// Maximum uncompressed size of any single member.
    pub max_file_bytes: usize,
    /// Maximum total uncompressed bytes written across all members.
    pub max_total_bytes: u64,
    /// Maximum number of entries in the central directory.
    pub max_entries: usize,
    /// Maximum directory nesting depth inside the archive.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_archive_bytes: 256 * 1024 * 1024,
            max_file_bytes: 256 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
            max_entries: 20_000,
            max_depth: 64,
        }
    }
}

/// Why extracting or reading a ZIP archive failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZipError {
    Fs(FsError),
    NotZip,
    Truncated,
    Corrupt(&'static str),
    Encrypted,
    UnsupportedMethod(u16),
    BadChecksum,
    UnsafePath,
    TooLarge,
    TooManyEntries,
    Inflate(InflateError),
}

impl ZipError {
    /// A short human-readable message for the UI status line.
    pub fn message(&self) -> String {
        match self {
            ZipError::Fs(e) => e.message().into(),
            ZipError::NotZip => "Not a valid ZIP archive".into(),
            ZipError::Truncated => "The ZIP archive is incomplete".into(),
            ZipError::Corrupt(why) => format!("Corrupt ZIP archive ({why})"),
            ZipError::Encrypted => "Encrypted ZIP archives are not supported".into(),
            ZipError::UnsupportedMethod(m) => format!("Unsupported ZIP compression method ({m})"),
            ZipError::BadChecksum => "ZIP archive failed checksum verification".into(),
            ZipError::UnsafePath => "ZIP archive contains an unsafe file path".into(),
            ZipError::TooLarge => "ZIP archive is too large to extract".into(),
            ZipError::TooManyEntries => "ZIP archive has too many files".into(),
            ZipError::Inflate(e) => format!("ZIP decompression failed ({e})"),
        }
    }
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl From<FsError> for ZipError {
    fn from(e: FsError) -> Self {
        ZipError::Fs(e)
    }
}

impl From<InflateError> for ZipError {
    fn from(e: InflateError) -> Self {
        match e {
            InflateError::TooLarge => ZipError::TooLarge,
            other => ZipError::Inflate(other),
        }
    }
}

/// One entry listed in a ZIP archive's central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
    pub is_dir: bool,
    /// Unix permission bits (`0o777`) when recorded by a Unix archiver, else 0.
    pub mode: u16,
    pub is_symlink: bool,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    local_header: u64,
}

/// Summary of a completed extraction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractReport {
    /// Full path of the created destination folder.
    pub dest: String,
    /// Folder name created inside the parent directory.
    pub name: String,
    /// Regular files written.
    pub files: usize,
    /// Directories created inside `dest` (excluding `dest` itself).
    pub dirs: usize,
    /// Total file bytes written.
    pub bytes: u64,
    /// Symlink members skipped.
    pub skipped_symlinks: usize,
}

/// IEEE 802.3 reflected CRC-32 lookup table.
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// CRC-32 (IEEE 802.3, reflected, as ZIP uses).
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c = CRC32_TABLE[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// Find the End of Central Directory record offset, verifying its comment
/// reaches the end of `zip` (falling back to the last EOCD signature if
/// trailing garbage is present).
fn find_eocd(zip: &[u8]) -> Option<usize> {
    if zip.len() < 22 {
        return None;
    }
    let lowest = zip.len().saturating_sub(22 + 0xffff);
    let last_possible = zip.len() - 22;
    let mut fallback = None;
    for i in (lowest..=last_possible).rev() {
        if u32_at(zip, i) == Some(SIG_EOCD) {
            let comment_len = usize::from(u16_at(zip, i + 20)?);
            if i + 22 + comment_len == zip.len() {
                return Some(i);
            }
            if fallback.is_none() {
                fallback = Some(i);
            }
        }
    }
    fallback
}

/// Read the central directory's entry count and start offset, resolving
/// ZIP64 EOCD records when the 32-bit EOCD uses sentinel values.
fn central_dir_location(zip: &[u8], eocd: usize) -> Result<(u64, u64), ZipError> {
    let count16 = u16_at(zip, eocd + 10).ok_or(ZipError::Truncated)?;
    let offset32 = u32_at(zip, eocd + 16).ok_or(ZipError::Truncated)?;
    let mut count = u64::from(count16);
    let mut offset = u64::from(offset32);

    if (count16 == u16::MAX || offset32 == u32::MAX) && eocd >= 20 && u32_at(zip, eocd - 20) == Some(SIG_ZIP64_LOCATOR)
    {
        let z64_off =
            usize::try_from(u64_at(zip, eocd - 12).ok_or(ZipError::Truncated)?).map_err(|_| ZipError::TooLarge)?;
        if u32_at(zip, z64_off) != Some(SIG_ZIP64_EOCD) {
            return Err(ZipError::Corrupt("bad ZIP64 end of central directory"));
        }
        if count16 == u16::MAX {
            count = u64_at(zip, z64_off + 32).ok_or(ZipError::Truncated)?;
        }
        if offset32 == u32::MAX {
            offset = u64_at(zip, z64_off + 48).ok_or(ZipError::Truncated)?;
        }
    }
    Ok((count, offset))
}

/// Resolve ZIP64 extra field values for a central directory entry whose
/// 32-bit fields are `0xffff_ffff` (APPNOTE 4.5.3: uncompressed size,
/// compressed size, local header offset, in that order).
fn apply_zip64_extra(
    extra: &[u8],
    size: &mut u64,
    compressed: &mut u64,
    local_header: &mut u64,
) -> Result<(), ZipError> {
    let mut p = 0;
    while p + 4 <= extra.len() {
        let id = u16_at(extra, p).ok_or(ZipError::Truncated)?;
        let len = usize::from(u16_at(extra, p + 2).ok_or(ZipError::Truncated)?);
        let end = p.checked_add(4).and_then(|v| v.checked_add(len)).ok_or(ZipError::Truncated)?;
        let body = extra.get(p + 4..end).ok_or(ZipError::Truncated)?;
        if id == EXTRA_ZIP64 {
            let mut off = 0;
            if *size == u64::from(u32::MAX) {
                *size = u64_at(body, off).ok_or(ZipError::Truncated)?;
                off += 8;
            }
            if *compressed == u64::from(u32::MAX) {
                *compressed = u64_at(body, off).ok_or(ZipError::Truncated)?;
                off += 8;
            }
            if *local_header == u64::from(u32::MAX) {
                *local_header = u64_at(body, off).ok_or(ZipError::Truncated)?;
            }
            return Ok(());
        }
        p = end;
    }
    Ok(())
}

/// List every member from `zip`'s central directory, bounded by `max_entries`.
pub fn members(zip: &[u8], max_entries: usize) -> Result<Vec<Member>, ZipError> {
    let eocd = find_eocd(zip).ok_or(ZipError::NotZip)?;
    let (count, cd_offset) = central_dir_location(zip, eocd)?;
    let count = usize::try_from(count).map_err(|_| ZipError::TooManyEntries)?;
    if count > max_entries {
        return Err(ZipError::TooManyEntries);
    }
    let mut at = usize::try_from(cd_offset).map_err(|_| ZipError::TooLarge)?;
    if at > zip.len() {
        return Err(ZipError::Truncated);
    }
    let mut out = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        if u32_at(zip, at) != Some(SIG_CENTRAL) {
            return Err(ZipError::Corrupt("bad central directory header"));
        }
        let field = |o: usize| u16_at(zip, at + o).ok_or(ZipError::Truncated);
        let made_by = field(4)?;
        let flags = field(8)?;
        let method = field(10)?;
        let crc = u32_at(zip, at + 16).ok_or(ZipError::Truncated)?;
        let mut compressed = u64::from(u32_at(zip, at + 20).ok_or(ZipError::Truncated)?);
        let mut size = u64::from(u32_at(zip, at + 24).ok_or(ZipError::Truncated)?);
        let name_len = usize::from(field(28)?);
        let extra_len = usize::from(field(30)?);
        let comment_len = usize::from(field(32)?);
        let external = u32_at(zip, at + 38).ok_or(ZipError::Truncated)?;
        let mut local_header = u64::from(u32_at(zip, at + 42).ok_or(ZipError::Truncated)?);

        let name_start = at.checked_add(46).ok_or(ZipError::Truncated)?;
        let name_end = name_start.checked_add(name_len).ok_or(ZipError::Truncated)?;
        let extra_end = name_end.checked_add(extra_len).ok_or(ZipError::Truncated)?;
        let next = extra_end.checked_add(comment_len).ok_or(ZipError::Truncated)?;
        let name_bytes = zip.get(name_start..name_end).ok_or(ZipError::Truncated)?;
        let extra_bytes = zip.get(name_end..extra_end).ok_or(ZipError::Truncated)?;
        if next > zip.len() {
            return Err(ZipError::Truncated);
        }

        if size == u64::from(u32::MAX) || compressed == u64::from(u32::MAX) || local_header == u64::from(u32::MAX) {
            apply_zip64_extra(extra_bytes, &mut size, &mut compressed, &mut local_header)?;
        }

        let name = String::from_utf8_lossy(name_bytes).into_owned();
        let unix = (made_by >> 8) == 3;
        let unix_mode = if unix { (external >> 16) as u16 } else { 0 };
        let file_type = unix_mode & 0o170000;
        let is_symlink = unix && file_type == 0o120000;
        let is_dir = name.ends_with('/') || name.ends_with('\\') || (unix && file_type == 0o040000);

        out.push(Member {
            name,
            size,
            is_dir,
            mode: if unix { unix_mode & 0o777 } else { 0 },
            is_symlink,
            method,
            flags,
            crc,
            compressed,
            local_header,
        });
        at = next;
    }
    Ok(out)
}

/// Decompress and verify member `m` from `zip`.
pub fn read_member(zip: &[u8], m: &Member, max_file_bytes: usize) -> Result<Vec<u8>, ZipError> {
    if m.flags & 1 != 0 {
        return Err(ZipError::Encrypted);
    }
    let expected = usize::try_from(m.size).map_err(|_| ZipError::TooLarge)?;
    if expected > max_file_bytes {
        return Err(ZipError::TooLarge);
    }
    let compressed = usize::try_from(m.compressed).map_err(|_| ZipError::TooLarge)?;
    let h = usize::try_from(m.local_header).map_err(|_| ZipError::TooLarge)?;
    if u32_at(zip, h) != Some(SIG_LOCAL) {
        return Err(ZipError::Corrupt("bad local file header"));
    }
    let name_len = usize::from(u16_at(zip, h + 26).ok_or(ZipError::Truncated)?);
    let extra_len = usize::from(u16_at(zip, h + 28).ok_or(ZipError::Truncated)?);
    let start = h
        .checked_add(30)
        .and_then(|v| v.checked_add(name_len))
        .and_then(|v| v.checked_add(extra_len))
        .ok_or(ZipError::Truncated)?;
    let end = start.checked_add(compressed).ok_or(ZipError::Truncated)?;
    let data = zip.get(start..end).ok_or(ZipError::Truncated)?;

    let out = match m.method {
        METHOD_STORED => {
            if data.len() != expected {
                return Err(ZipError::BadChecksum);
            }
            data.to_vec()
        }
        METHOD_DEFLATE => {
            let (buf, _) = poly_png::inflate::inflate(data, expected)?;
            buf
        }
        other => return Err(ZipError::UnsupportedMethod(other)),
    };
    if out.len() != expected || crc32(&out) != m.crc {
        return Err(ZipError::BadChecksum);
    }
    Ok(out)
}

/// Validate a member's relative path inside an archive, rejecting traversal,
/// absolute paths, Windows drive prefixes and invalid path segments.
pub fn sanitize_path(raw: &str, max_depth: usize) -> Result<Vec<String>, ZipError> {
    if raw.contains('\0') {
        return Err(ZipError::UnsafePath);
    }
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(ZipError::UnsafePath);
    }
    // Reject Windows drive prefixes like `C:...`.
    if normalized.as_bytes().get(1) == Some(&b':') && normalized.as_bytes()[0].is_ascii_alphabetic() {
        return Err(ZipError::UnsafePath);
    }
    let mut segs = Vec::new();
    for seg in normalized.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || validate_name(seg).is_err() {
            return Err(ZipError::UnsafePath);
        }
        segs.push(seg.to_string());
        if segs.len() > max_depth {
            return Err(ZipError::UnsafePath);
        }
    }
    Ok(segs)
}

/// Pick `base` if free in `existing`, else `"{base} 2"`, `"{base} 3"`, ...
pub fn unique_dir_name<'a>(existing: impl IntoIterator<Item = &'a str>, base: &str) -> String {
    let taken: Vec<&str> = existing.into_iter().collect();
    if !taken.contains(&base) {
        return base.into();
    }
    (2u32..).map(|n| format!("{base} {n}")).find(|c| !taken.contains(&c.as_str())).unwrap_or_default()
}

/// Default folder stem for `archive_path` when the archive does not contain a
/// single top-level directory.
fn archive_stem(archive_path: &str) -> &str {
    let name = file_name(archive_path);
    let (stem, _) = split_ext(name);
    if validate_name(stem).is_ok() { stem } else { "Extracted" }
}

/// Extract the ZIP archive at `archive_path` into a new folder inside
/// `dest_parent` with [`Limits::default`].
pub fn extract(fs: &dyn Fs, archive_path: &str, dest_parent: &str) -> Result<ExtractReport, ZipError> {
    extract_with_limits(fs, archive_path, dest_parent, &Limits::default())
}

/// Extract the ZIP archive at `archive_path` into a new folder inside
/// `dest_parent`, enforcing `limits`.
///
/// If every member sits inside one common top-level directory (e.g.
/// `project/README.md`, `project/src/...`), that folder name is used directly
/// without creating a redundant outer wrapper (`project/project/...`).
/// Otherwise a folder named after the archive's stem is created. Existing
/// names are never overwritten (`"name 2"`, `"name 3"`, ...).
pub fn extract_with_limits(
    fs: &dyn Fs,
    archive_path: &str,
    dest_parent: &str,
    limits: &Limits,
) -> Result<ExtractReport, ZipError> {
    let archive_path = normalize(archive_path);
    let dest_parent = normalize(dest_parent);
    if fs.stat(&dest_parent)?.kind != Kind::Dir {
        return Err(ZipError::Fs(FsError::NotADirectory));
    }
    let meta = fs.stat(&archive_path)?;
    if meta.kind == Kind::Dir {
        return Err(ZipError::Fs(FsError::IsADirectory));
    }
    if meta.size > limits.max_archive_bytes as u64 {
        return Err(ZipError::TooLarge);
    }
    let data = fs.read_head(&archive_path, limits.max_archive_bytes.saturating_add(1))?;
    if data.len() > limits.max_archive_bytes {
        return Err(ZipError::TooLarge);
    }

    let entries = members(&data, limits.max_entries)?;
    // Validate every path before creating anything on disk.
    let mut parsed: Vec<(Member, Vec<String>)> = Vec::with_capacity(entries.len());
    for m in entries {
        let segs = sanitize_path(&m.name, limits.max_depth)?;
        if segs.is_empty() {
            continue;
        }
        parsed.push((m, segs));
    }

    // Detect whether all entries live inside a single top-level directory so
    // we avoid double-nesting `foo/foo/...`.
    let single_root: Option<String> = {
        let mut root: Option<&str> = None;
        let mut all_under_dir = !parsed.is_empty();
        for (m, segs) in &parsed {
            if segs.len() == 1 && !m.is_dir {
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

    let base_name = single_root.as_deref().unwrap_or_else(|| archive_stem(&archive_path));
    let existing: Vec<String> = fs.read_dir(&dest_parent)?.into_iter().map(|e| e.name).collect();
    let folder_name = unique_dir_name(existing.iter().map(String::as_str), base_name);
    let dest = join(&dest_parent, &folder_name);
    fs.create_dir(&dest)?;

    let strip_root = single_root.is_some();
    let result = extract_entries(fs, &data, &dest, &parsed, strip_root, limits);
    match result {
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
    zip: &[u8],
    dest: &str,
    parsed: &[(Member, Vec<String>)],
    strip_root: bool,
    limits: &Limits,
) -> Result<(usize, usize, u64, usize), ZipError> {
    let (mut files, mut dirs, mut total_bytes, mut skipped_symlinks) = (0usize, 0usize, 0u64, 0usize);

    for (m, segs) in parsed {
        let rel = if strip_root { &segs[1..] } else { segs.as_slice() };
        if rel.is_empty() {
            continue;
        }
        if m.is_symlink {
            skipped_symlinks += 1;
            continue;
        }

        let mut target = String::from(dest);
        for seg in rel {
            target = join(&target, seg);
        }

        if m.is_dir {
            if !fs.exists(&target) {
                create_dir_all(fs, &target)?;
                dirs += 1;
            }
            continue;
        }

        // Ensure parent directories exist for files whose archive omitted
        // explicit directory entries.
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
        if m.size > remaining {
            return Err(ZipError::TooLarge);
        }
        let bytes = read_member(zip, m, max_this)?;
        total_bytes = total_bytes.checked_add(bytes.len() as u64).ok_or(ZipError::TooLarge)?;
        if total_bytes > limits.max_total_bytes {
            return Err(ZipError::TooLarge);
        }

        fs.write(&target, &bytes)?;
        if m.mode != 0 {
            // Keep owner read/write bits so extracted files stay accessible.
            let mode = (m.mode & 0o777) | 0o600;
            let _ = fs.set_mode(&target, mode);
        }
        files += 1;
    }

    Ok((files, dirs, total_bytes, skipped_symlinks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::{MemFs, StdFs};

    /// Build a minimal valid ZIP archive from `(name, method, flags, unix_mode, raw_or_deflated, uncompressed_len, crc)`.
    struct EntrySpec<'a> {
        name: &'a str,
        method: u16,
        flags: u16,
        unix_mode: Option<u16>,
        payload: &'a [u8],
        uncompressed_size: u32,
        crc: u32,
    }

    fn stored<'a>(name: &'a str, data: &'a [u8]) -> EntrySpec<'a> {
        EntrySpec {
            name,
            method: METHOD_STORED,
            flags: 0,
            unix_mode: None,
            payload: data,
            uncompressed_size: data.len() as u32,
            crc: crc32(data),
        }
    }

    fn build_zip(entries: &[EntrySpec<'_>]) -> Vec<u8> {
        let mut zip = Vec::new();
        let mut offsets = Vec::with_capacity(entries.len());
        for e in entries {
            offsets.push(zip.len() as u32);
            zip.extend_from_slice(&SIG_LOCAL.to_le_bytes());
            zip.extend_from_slice(&20u16.to_le_bytes());
            zip.extend_from_slice(&e.flags.to_le_bytes());
            zip.extend_from_slice(&e.method.to_le_bytes());
            zip.extend_from_slice(&0u32.to_le_bytes()); // mtime/mdate
            zip.extend_from_slice(&e.crc.to_le_bytes());
            zip.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
            zip.extend_from_slice(&e.uncompressed_size.to_le_bytes());
            zip.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(e.name.as_bytes());
            zip.extend_from_slice(e.payload);
        }
        let cd_start = zip.len() as u32;
        for (e, off) in entries.iter().zip(offsets) {
            zip.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
            let made_by: u16 = if e.unix_mode.is_some() { (3 << 8) | 20 } else { 20 };
            zip.extend_from_slice(&made_by.to_le_bytes());
            zip.extend_from_slice(&20u16.to_le_bytes());
            zip.extend_from_slice(&e.flags.to_le_bytes());
            zip.extend_from_slice(&e.method.to_le_bytes());
            zip.extend_from_slice(&0u32.to_le_bytes());
            zip.extend_from_slice(&e.crc.to_le_bytes());
            zip.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
            zip.extend_from_slice(&e.uncompressed_size.to_le_bytes());
            zip.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&0u16.to_le_bytes()); // extra
            zip.extend_from_slice(&0u16.to_le_bytes()); // comment
            zip.extend_from_slice(&0u16.to_le_bytes()); // disk
            zip.extend_from_slice(&0u16.to_le_bytes()); // internal
            let ext = e.unix_mode.map_or(0u32, |m| u32::from(m) << 16);
            zip.extend_from_slice(&ext.to_le_bytes());
            zip.extend_from_slice(&off.to_le_bytes());
            zip.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = zip.len() as u32 - cd_start;
        zip.extend_from_slice(&SIG_EOCD.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&cd_size.to_le_bytes());
        zip.extend_from_slice(&cd_start.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    #[test]
    fn crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn extracts_stored_and_deflated_files_into_stem_folder() {
        // Raw DEFLATE stream for "hello hello hello hello" (23 bytes).
        let deflated = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01];
        let want_deflated = b"hello hello hello hello";
        let zip = build_zip(&[
            stored("readme.txt", b"hi\n"),
            EntrySpec {
                name: "sub/greeting.txt",
                method: METHOD_DEFLATE,
                flags: 0,
                unix_mode: Some(0o100755),
                payload: &deflated,
                uncompressed_size: want_deflated.len() as u32,
                crc: crc32(want_deflated),
            },
        ]);

        let fs = MemFs::new();
        fs.create_dir("/work").unwrap();
        fs.write("/work/bundle.zip", &zip).unwrap();

        let report = extract(&fs, "/work/bundle.zip", "/work").unwrap();
        assert_eq!(report.dest, "/work/bundle");
        assert_eq!(report.name, "bundle");
        assert_eq!(report.files, 2);
        assert_eq!(report.dirs, 1);
        assert_eq!(report.bytes, 3 + 23);
        assert_eq!(fs.read_to_string("/work/bundle/readme.txt").unwrap(), "hi\n");
        assert_eq!(fs.read_to_string("/work/bundle/sub/greeting.txt").unwrap(), "hello hello hello hello");
        assert_eq!(fs.stat("/work/bundle/sub/greeting.txt").unwrap().mode, 0o755);

        // Extracting a second time picks a fresh name without overwriting.
        let second = extract(&fs, "/work/bundle.zip", "/work").unwrap();
        assert_eq!(second.dest, "/work/bundle 2");
        assert_eq!(fs.read_to_string("/work/bundle 2/readme.txt").unwrap(), "hi\n");
    }

    #[test]
    fn deduplicates_single_top_level_directory() {
        let zip = build_zip(&[
            stored("myproj/", b""),
            stored("myproj/Cargo.toml", b"[package]\n"),
            stored("myproj/src/lib.rs", b"// hi\n"),
        ]);
        let fs = StdFs::new();
        fs.create_dir("/dl").unwrap();
        fs.write("/dl/download.zip", &zip).unwrap();

        let report = extract(&fs, "/dl/download.zip", "/dl").unwrap();
        assert_eq!(report.dest, "/dl/myproj");
        assert_eq!(report.name, "myproj");
        assert_eq!(fs.read_to_string("/dl/myproj/Cargo.toml").unwrap(), "[package]\n");
        assert_eq!(fs.read_to_string("/dl/myproj/src/lib.rs").unwrap(), "// hi\n");
        assert!(!fs.exists("/dl/myproj/myproj"));
    }

    #[test]
    fn handles_windows_backslashes_empty_archives_and_symlinks() {
        let zip = build_zip(&[
            stored("win\\nested\\a.txt", b"ok"),
            EntrySpec {
                name: "win\\link",
                method: METHOD_STORED,
                flags: 0,
                unix_mode: Some(0o120777),
                payload: b"/etc/passwd",
                uncompressed_size: 11,
                crc: crc32(b"/etc/passwd"),
            },
        ]);
        let fs = MemFs::new();
        fs.create_dir("/d").unwrap();
        fs.write("/d/w.zip", &zip).unwrap();

        let report = extract(&fs, "/d/w.zip", "/d").unwrap();
        assert_eq!(report.dest, "/d/win");
        assert_eq!(report.files, 1);
        assert_eq!(report.skipped_symlinks, 1);
        assert_eq!(fs.read_to_string("/d/win/nested/a.txt").unwrap(), "ok");
        assert!(!fs.exists("/d/win/link"));

        // Empty archive creates an empty folder named after the stem.
        let empty = build_zip(&[]);
        fs.write("/d/empty.zip", &empty).unwrap();
        let report = extract(&fs, "/d/empty.zip", "/d").unwrap();
        assert_eq!(report.dest, "/d/empty");
        assert_eq!(report.files, 0);
        assert_eq!(fs.stat("/d/empty").unwrap().kind, Kind::Dir);
    }

    #[test]
    fn refuses_zip_slip_paths_without_creating_destination() {
        for bad in ["../escape.txt", "/etc/passwd", "a/../../b", "C:\\evil.txt", "a/\0b"] {
            let zip = build_zip(&[stored(bad, b"x")]);
            let fs = MemFs::new();
            fs.create_dir("/d").unwrap();
            fs.write("/d/bad.zip", &zip).unwrap();
            assert_eq!(extract(&fs, "/d/bad.zip", "/d"), Err(ZipError::UnsafePath), "path={bad:?}");
            assert!(!fs.exists("/d/bad"));
        }
    }

    #[test]
    fn cleans_up_partial_extraction_on_checksum_encryption_or_limit_error() {
        let mut bad_crc = stored("two.txt", b"hello");
        bad_crc.crc ^= 1;
        let zip = build_zip(&[stored("one.txt", b"ok"), bad_crc]);
        let fs = MemFs::new();
        fs.create_dir("/d").unwrap();
        fs.write("/d/broken.zip", &zip).unwrap();
        assert_eq!(extract(&fs, "/d/broken.zip", "/d"), Err(ZipError::BadChecksum));
        assert!(!fs.exists("/d/broken"), "partial folder should be cleaned up");

        let enc = build_zip(&[EntrySpec {
            name: "secret.txt",
            method: METHOD_STORED,
            flags: 1,
            unix_mode: None,
            payload: b"abc",
            uncompressed_size: 3,
            crc: crc32(b"abc"),
        }]);
        fs.write("/d/enc.zip", &enc).unwrap();
        assert_eq!(extract(&fs, "/d/enc.zip", "/d"), Err(ZipError::Encrypted));
        assert!(!fs.exists("/d/enc"));

        let big = build_zip(&[stored("a.bin", b"123456"), stored("b.bin", b"789012")]);
        fs.write("/d/big.zip", &big).unwrap();
        let limits = Limits { max_total_bytes: 10, ..Limits::default() };
        assert_eq!(extract_with_limits(&fs, "/d/big.zip", "/d", &limits), Err(ZipError::TooLarge));
        assert!(!fs.exists("/d/big"));

        fs.write("/d/notzip.zip", b"plain text").unwrap();
        assert_eq!(extract(&fs, "/d/notzip.zip", "/d"), Err(ZipError::NotZip));
    }

    #[test]
    fn parses_zip64_eocd_and_extra_fields() {
        // Build a ZIP whose EOCD and central directory use ZIP64 sentinel values.
        let payload = b"zip64 works!";
        let crc = crc32(payload);
        let name = b"hello.txt";
        let mut zip = Vec::new();
        // Local header at offset 0.
        zip.extend_from_slice(&SIG_LOCAL.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&METHOD_STORED.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(payload);

        let cd_start = zip.len() as u64;
        // ZIP64 extra field with uncompressed size, compressed size, local header offset.
        let mut extra = Vec::new();
        extra.extend_from_slice(&EXTRA_ZIP64.to_le_bytes());
        extra.extend_from_slice(&24u16.to_le_bytes());
        extra.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        extra.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        extra.extend_from_slice(&0u64.to_le_bytes());

        zip.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&METHOD_STORED.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&u32::MAX.to_le_bytes()); // compressed -> ZIP64
        zip.extend_from_slice(&u32::MAX.to_le_bytes()); // size -> ZIP64
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&u32::MAX.to_le_bytes()); // local_header -> ZIP64
        zip.extend_from_slice(name);
        zip.extend_from_slice(&extra);
        let cd_size = zip.len() as u64 - cd_start;

        // ZIP64 EOCD (56 bytes).
        let z64_eocd = zip.len() as u64;
        zip.extend_from_slice(&SIG_ZIP64_EOCD.to_le_bytes());
        zip.extend_from_slice(&44u64.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&1u64.to_le_bytes());
        zip.extend_from_slice(&1u64.to_le_bytes());
        zip.extend_from_slice(&cd_size.to_le_bytes());
        zip.extend_from_slice(&cd_start.to_le_bytes());

        // ZIP64 EOCD locator (20 bytes).
        zip.extend_from_slice(&SIG_ZIP64_LOCATOR.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&z64_eocd.to_le_bytes());
        zip.extend_from_slice(&1u32.to_le_bytes());

        // Classic EOCD with sentinels.
        zip.extend_from_slice(&SIG_EOCD.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&u16::MAX.to_le_bytes());
        zip.extend_from_slice(&u16::MAX.to_le_bytes());
        zip.extend_from_slice(&u32::MAX.to_le_bytes());
        zip.extend_from_slice(&u32::MAX.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());

        let fs = MemFs::new();
        fs.create_dir("/d").unwrap();
        fs.write("/d/z64.zip", &zip).unwrap();
        let report = extract(&fs, "/d/z64.zip", "/d").unwrap();
        assert_eq!(report.dest, "/d/z64");
        assert_eq!(fs.read_to_string("/d/z64/hello.txt").unwrap(), "zip64 works!");
    }
}
