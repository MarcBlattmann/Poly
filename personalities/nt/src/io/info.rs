//! The information structures the file services read and write, their x64
//! layouts (`wdm.h`, `ntifs.h`; field lists on Learn's page for each
//! structure), times, and how `FILE_ATTRIBUTE_*` maps onto the VFS.
//!
//! # Times
//!
//! NT times are 100 ns intervals since 1601-01-01 UTC (Learn: "FILETIME",
//! "FILE_BASIC_INFORMATION"); the VFS keeps seconds and nanoseconds since
//! 1970. `LastWriteTime` is `mtime`, `LastAccessTime` `atime`, `ChangeTime`
//! `ctime`. The VFS has no birth time, so `CreationTime` is stored with the
//! attributes (below) when a file is created through NT or the time is set,
//! and is `mtime` for a file that has none stored (to be confirmed by the
//! spec agent what Windows reports there — it has no such files).
//!
//! # Attributes
//!
//! - `FILE_ATTRIBUTE_READONLY` is the absence of every write permission bit
//!   (`perm & 0o222 == 0`); setting it clears them, clearing it gives the
//!   owner write permission back (and group/other where they can read).
//!   NT enforces it itself (the VFS lets root write anything): a file with
//!   it refuses write and delete access with `STATUS_ACCESS_DENIED`, and
//!   deletion with `STATUS_CANNOT_DELETE` (Learn: "File Attribute
//!   Constants"; directories ignore it, as there).
//! - `FILE_ATTRIBUTE_DIRECTORY` comes from the node's type and
//!   `FILE_ATTRIBUTE_REPARSE_POINT` marks a symbolic link.
//! - `HIDDEN`, `SYSTEM`, `ARCHIVE`, `TEMPORARY`, `OFFLINE` and
//!   `NOT_CONTENT_INDEXED` are stored in the extended attribute
//!   `user.poly.ntattr` (ADR 0060): a little-endian `u32` of those bits,
//!   then optionally the `i64` creation time. A file system without
//!   extended attributes simply does not keep them.
//! - With no other attribute a file reports `FILE_ATTRIBUTE_NORMAL`.

use super::consts::*;
use alloc::vec::Vec;
use poly_vfs::{FileType, Timespec};

/// The extended attribute that holds the stored NT attributes.
pub const NT_XATTR: &[u8] = b"user.poly.ntattr";

/// The attribute bits kept in [`NT_XATTR`].
pub const STORED_ATTRIBUTES: u32 = FILE_ATTRIBUTE_HIDDEN
    | FILE_ATTRIBUTE_SYSTEM
    | FILE_ATTRIBUTE_ARCHIVE
    | FILE_ATTRIBUTE_TEMPORARY
    | FILE_ATTRIBUTE_OFFLINE
    | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;

/// 100 ns intervals between 1601-01-01 and 1970-01-01.
pub const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;

/// A VFS time as an NT time.
pub fn nt_time(t: Timespec) -> i64 {
    t.sec.saturating_mul(10_000_000).saturating_add(i64::from(t.nsec / 100)).saturating_add(EPOCH_DIFF_100NS)
}

/// An NT time as a VFS time.
pub fn vfs_time(t: i64) -> Timespec {
    let unix = t.saturating_sub(EPOCH_DIFF_100NS);
    let sec = unix.div_euclid(10_000_000);
    let nsec = (unix.rem_euclid(10_000_000) * 100) as u32;
    Timespec::new(sec, nsec)
}

/// The stored part of a file's NT attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stored {
    pub attributes: u32,
    pub creation: Option<i64>,
}

impl Stored {
    pub fn parse(value: &[u8]) -> Stored {
        let attributes = value.get(0..4).map_or(0, |b| u32::from_le_bytes(b.try_into().expect("4 bytes")));
        let creation = value.get(4..12).map(|b| i64::from_le_bytes(b.try_into().expect("8 bytes")));
        Stored { attributes: attributes & STORED_ATTRIBUTES, creation }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = (self.attributes & STORED_ATTRIBUTES).to_le_bytes().to_vec();
        if let Some(c) = self.creation {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }
}

/// What the information classes report about a file, gathered from the VFS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub creation: i64,
    pub access: i64,
    pub write: i64,
    pub change: i64,
    pub size: u64,
    pub allocation: u64,
    pub links: u32,
    pub attributes: u32,
    pub directory: bool,
    pub delete_pending: bool,
    /// The file id (`FileInternalInformation`, `FileId`): the inode number.
    pub id: u64,
    pub reparse_tag: u32,
}

/// The `FILE_ATTRIBUTE_*` a node reports.
pub fn attributes_of(kind: FileType, perm: u16, stored: u32) -> u32 {
    let mut a = stored & STORED_ATTRIBUTES;
    match kind {
        FileType::Directory => a |= FILE_ATTRIBUTE_DIRECTORY,
        FileType::Symlink => a |= FILE_ATTRIBUTE_REPARSE_POINT,
        _ => {}
    }
    if kind != FileType::Directory && perm & 0o222 == 0 {
        a |= FILE_ATTRIBUTE_READONLY;
    }
    if a == 0 { FILE_ATTRIBUTE_NORMAL } else { a }
}

/// The permission bits after setting or clearing `FILE_ATTRIBUTE_READONLY`.
pub fn perm_for_readonly(perm: u16, readonly: bool) -> u16 {
    if readonly {
        perm & !0o222
    } else if perm & 0o222 == 0 {
        perm | 0o200 | ((perm & 0o044) >> 1)
    } else {
        perm
    }
}

/// Allocation size: whole 4 KiB clusters (our volumes report 4 KiB
/// allocation units).
pub fn allocation(size: u64, blocks: u64) -> u64 {
    (blocks * 512).max(size.div_ceil(4096) * 4096)
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn put_i64(b: &mut [u8], at: usize, v: i64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn put_name(b: &mut Vec<u8>, name: &[u16]) {
    for c in name {
        b.extend_from_slice(&c.to_le_bytes());
    }
}

/// `FILE_BASIC_INFORMATION`: `CreationTime` 0, `LastAccessTime` 8,
/// `LastWriteTime` 16, `ChangeTime` 24, `FileAttributes` 32 (40 bytes).
pub fn basic(f: &Facts) -> [u8; BASIC_INFO_SIZE] {
    let mut b = [0u8; BASIC_INFO_SIZE];
    put_i64(&mut b, 0, f.creation);
    put_i64(&mut b, 8, f.access);
    put_i64(&mut b, 16, f.write);
    put_i64(&mut b, 24, f.change);
    put_u32(&mut b, 32, f.attributes);
    b
}

/// A `FILE_BASIC_INFORMATION` a caller passed to `NtSetInformationFile`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetBasic {
    pub creation: i64,
    pub access: i64,
    pub write: i64,
    pub change: i64,
    pub attributes: u32,
}

pub fn parse_basic(b: &[u8]) -> SetBasic {
    let q = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"));
    SetBasic {
        creation: q(0),
        access: q(8),
        write: q(16),
        change: q(24),
        attributes: u32::from_le_bytes(b[32..36].try_into().expect("4 bytes")),
    }
}

/// `FILE_STANDARD_INFORMATION`: `AllocationSize` 0, `EndOfFile` 8,
/// `NumberOfLinks` 16, `DeletePending` 20, `Directory` 21 (24 bytes).
pub fn standard(f: &Facts) -> [u8; STANDARD_INFO_SIZE] {
    let mut b = [0u8; STANDARD_INFO_SIZE];
    put_u64(&mut b, 0, f.allocation);
    put_u64(&mut b, 8, f.size);
    put_u32(&mut b, 16, f.links);
    b[20] = u8::from(f.delete_pending);
    b[21] = u8::from(f.directory);
    b
}

/// `FILE_NETWORK_OPEN_INFORMATION`: the four times, `AllocationSize` 32,
/// `EndOfFile` 40, `FileAttributes` 48 (56 bytes).
pub fn network_open(f: &Facts) -> [u8; NETWORK_OPEN_INFO_SIZE] {
    let mut b = [0u8; NETWORK_OPEN_INFO_SIZE];
    put_i64(&mut b, 0, f.creation);
    put_i64(&mut b, 8, f.access);
    put_i64(&mut b, 16, f.write);
    put_i64(&mut b, 24, f.change);
    put_u64(&mut b, 32, f.allocation);
    put_u64(&mut b, 40, f.size);
    put_u32(&mut b, 48, f.attributes);
    b
}

/// `FILE_ATTRIBUTE_TAG_INFORMATION`: `FileAttributes`, `ReparseTag`.
pub fn attribute_tag(f: &Facts) -> [u8; 8] {
    let mut b = [0u8; 8];
    put_u32(&mut b, 0, f.attributes);
    put_u32(&mut b, 4, f.reparse_tag);
    b
}

/// The name of a file's default (unnamed) data stream (Learn: "File
/// Streams (Local File Systems)": `::$DATA`).
pub const DEFAULT_STREAM: &str = "::$DATA";

/// `FILE_STREAM_INFORMATION` (`ntifs.h`: `NextEntryOffset` 0,
/// `StreamNameLength` 4, `StreamSize` 8, `StreamAllocationSize` 16,
/// `StreamName` 24): Poly's volumes keep no named streams, so a file on one
/// has its default data stream only and a directory none (an empty
/// answer). A console or pipe has none either (to be confirmed).
pub fn streams(f: &Facts, on_disk: bool) -> Vec<u8> {
    if f.directory || !on_disk {
        return Vec::new();
    }
    let name: Vec<u16> = DEFAULT_STREAM.encode_utf16().collect();
    let mut b = alloc::vec![0u8; STREAM_INFO_FIXED];
    put_u32(&mut b, 4, (2 * name.len()) as u32);
    b[8..16].copy_from_slice(&f.size.to_le_bytes());
    b[16..24].copy_from_slice(&f.allocation.to_le_bytes());
    for c in name {
        b.extend_from_slice(&c.to_le_bytes());
    }
    b
}

/// A symbolic link to `target` (the VFS's bytes) as a `REPARSE_DATA_BUFFER`
/// (`ntifs.h`): `ReparseTag` 0, `ReparseDataLength` 4, `Reserved` 6, then
/// `SymbolicLinkReparseBuffer`: `SubstituteNameOffset` 8,
/// `SubstituteNameLength` 10, `PrintNameOffset` 12, `PrintNameLength` 14,
/// `Flags` 16, `PathBuffer` 20 (offsets into it, in bytes). A relative
/// target is both names with `\` for `/` and `SYMLINK_FLAG_RELATIVE`; an
/// absolute one starts at the VFS root, the volume `Z:` (ADR 0067): the
/// substitute name `\??\Z:\...`, the print name `Z:\...`.
pub fn symlink_reparse(target: &[u8]) -> Vec<u8> {
    let path: Vec<u16> = alloc::string::String::from_utf8_lossy(target)
        .encode_utf16()
        .map(|c| if c == u16::from(b'/') { u16::from(b'\\') } else { c })
        .collect();
    let (substitute, print, flags) = if target.first() == Some(&b'/') {
        let print: Vec<u16> = "Z:".encode_utf16().chain(path.iter().copied()).collect();
        let sub: Vec<u16> = "\\??\\".encode_utf16().chain(print.iter().copied()).collect();
        (sub, print, 0)
    } else {
        (path.clone(), path, SYMLINK_FLAG_RELATIVE)
    };
    let (sub_len, print_len) = (2 * substitute.len(), 2 * print.len());
    let mut b = alloc::vec![0u8; 20];
    put_u32(&mut b, 0, IO_REPARSE_TAG_SYMLINK);
    b[4..6].copy_from_slice(&((12 + sub_len + print_len) as u16).to_le_bytes());
    b[8..10].copy_from_slice(&0u16.to_le_bytes());
    b[10..12].copy_from_slice(&(sub_len as u16).to_le_bytes());
    b[12..14].copy_from_slice(&(sub_len as u16).to_le_bytes());
    b[14..16].copy_from_slice(&(print_len as u16).to_le_bytes());
    put_u32(&mut b, 16, flags);
    for c in substitute.iter().chain(&print) {
        b.extend_from_slice(&c.to_le_bytes());
    }
    b
}

/// A symbolic link's target as [`parse_symlink_reparse`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    /// The drive letter of an absolute target (`\??\X:\...`), `None` for a
    /// relative one.
    pub volume: Option<char>,
    /// The rest, as VFS bytes with `/` for `\`: below that volume's root
    /// (starting with `/`), or relative to the link's directory.
    pub path: Vec<u8>,
}

/// The target of a symbolic link's `REPARSE_DATA_BUFFER` (the layout of
/// [`symlink_reparse`]): its substitute name, relative with
/// `SYMLINK_FLAG_RELATIVE`, else `\??\X:\...`. Too short, a length past the
/// buffer or a name that is no UTF-16 is `STATUS_INVALID_PARAMETER`
/// (`STATUS_IO_REPARSE_DATA_INVALID` on Windows, to be confirmed);
/// another tag or another kind of absolute name `STATUS_NOT_SUPPORTED`.
pub fn parse_symlink_reparse(b: &[u8]) -> Result<LinkTarget, poly_ntobj::NtStatus> {
    use poly_ntobj::status::{STATUS_INVALID_PARAMETER, STATUS_NOT_SUPPORTED};
    let u16_at = |o: usize| b.get(o..o + 2).map(|x| usize::from(u16::from_le_bytes([x[0], x[1]])));
    let tag = b.get(0..4).map(|x| u32::from_le_bytes(x.try_into().expect("4 bytes")));
    let (Some(tag), Some(data_len)) = (tag, u16_at(4)) else { return Err(STATUS_INVALID_PARAMETER) };
    if tag != IO_REPARSE_TAG_SYMLINK {
        return Err(STATUS_NOT_SUPPORTED);
    }
    if b.len() < 20 || REPARSE_HEADER_SIZE + data_len > b.len() {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let (off, len) = (u16_at(8).unwrap_or(0), u16_at(10).unwrap_or(0));
    let flags = u32::from_le_bytes(b[16..20].try_into().expect("4 bytes"));
    let raw = b.get(20 + off..20 + off + len).filter(|r| r.len() % 2 == 0).ok_or(STATUS_INVALID_PARAMETER)?;
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let name = alloc::string::String::from_utf16(&units).map_err(|_| STATUS_INVALID_PARAMETER)?;
    let to_vfs = |s: &str| s.replace('\\', "/").into_bytes();
    if flags & SYMLINK_FLAG_RELATIVE != 0 {
        if name.is_empty() {
            return Err(STATUS_INVALID_PARAMETER);
        }
        return Ok(LinkTarget { volume: None, path: to_vfs(&name) });
    }
    let rest = name.strip_prefix("\\??\\").ok_or(STATUS_NOT_SUPPORTED)?;
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => {
            let tail = &rest[2..];
            let path = if tail.is_empty() { alloc::vec![b'/'] } else { to_vfs(tail) };
            if path.first() != Some(&b'/') {
                return Err(STATUS_NOT_SUPPORTED);
            }
            Ok(LinkTarget { volume: Some(letter), path })
        }
        _ => Err(STATUS_NOT_SUPPORTED),
    }
}

/// `FILE_NAME_INFORMATION`: `FileNameLength` (bytes) then the name.
pub fn name(name: &[u16]) -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + 2 * name.len());
    b.extend_from_slice(&((2 * name.len()) as u32).to_le_bytes());
    put_name(&mut b, name);
    b
}

/// What `FileAllInformation` needs besides the facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandleFacts {
    pub access: u32,
    pub position: u64,
    /// `FILE_MODE_INFORMATION.Mode`: the handle's `FILE_SYNCHRONOUS_IO_*`,
    /// `FILE_DELETE_ON_CLOSE`, ... options.
    pub mode: u32,
}

/// `FILE_ALL_INFORMATION`: Basic (0), Standard (40), Internal (64), Ea
/// (72), Access (76), Position (80), Mode (88), Alignment (92), Name (96).
pub fn all(f: &Facts, h: &HandleFacts, file_name: &[u16]) -> Vec<u8> {
    let mut b = alloc::vec![0u8; ALL_INFO_FIXED];
    b[0..40].copy_from_slice(&basic(f));
    b[40..64].copy_from_slice(&standard(f));
    put_u64(&mut b, 64, f.id);
    put_u32(&mut b, 72, 0); // EaSize: no extended attributes in NT's sense
    put_u32(&mut b, 76, h.access);
    put_u64(&mut b, 80, h.position);
    put_u32(&mut b, 88, h.mode);
    put_u32(&mut b, 92, 0); // FILE_BYTE_ALIGNMENT
    put_u32(&mut b, 96, (2 * file_name.len()) as u32);
    put_name(&mut b, file_name);
    b
}

/// The `FILE_MODE_INFORMATION` bits of a handle's create options.
pub fn mode_bits(options: u32) -> u32 {
    options
        & (FILE_WRITE_THROUGH
            | FILE_SEQUENTIAL_ONLY
            | FILE_NO_INTERMEDIATE_BUFFERING
            | FILE_SYNCHRONOUS_IO_ALERT
            | FILE_SYNCHRONOUS_IO_NONALERT
            | FILE_DELETE_ON_CLOSE)
}

/// One directory entry for `NtQueryDirectoryFile`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: Vec<u16>,
    pub facts: Facts,
}

/// The fixed part of each directory information class, up to `FileName`;
/// `None` for a class we do not answer.
///
/// - `FILE_DIRECTORY_INFORMATION`: `NextEntryOffset` 0, `FileIndex` 4, the
///   four times 8..40, `EndOfFile` 40, `AllocationSize` 48,
///   `FileAttributes` 56, `FileNameLength` 60, `FileName` 64.
/// - `FILE_FULL_DIR_INFORMATION`: then `EaSize` 64, `FileName` 68.
/// - `FILE_ID_FULL_DIR_INFORMATION`: `EaSize` 64, `FileId` 72, `FileName` 80.
/// - `FILE_BOTH_DIR_INFORMATION`: `EaSize` 64, `ShortNameLength` 68,
///   `ShortName[12]` 70, `FileName` 94.
/// - `FILE_ID_BOTH_DIR_INFORMATION`: as Both, then `FileId` 96, `FileName`
///   104.
/// - `FILE_NAMES_INFORMATION`: `NextEntryOffset`, `FileIndex`,
///   `FileNameLength` 8, `FileName` 12.
pub fn dir_fixed(class: u32) -> Option<usize> {
    Some(match class {
        FILE_DIRECTORY_INFORMATION => 64,
        FILE_FULL_DIRECTORY_INFORMATION => 68,
        FILE_ID_FULL_DIRECTORY_INFORMATION => 80,
        FILE_BOTH_DIRECTORY_INFORMATION => 94,
        FILE_ID_BOTH_DIRECTORY_INFORMATION => 104,
        FILE_NAMES_INFORMATION => 12,
        _ => return None,
    })
}

/// Encode one entry of `class` (`NextEntryOffset` 0; the caller links
/// entries). 8.3 short names are not generated (`ShortNameLength` 0, as on
/// a volume with short names disabled).
pub fn dir_entry(class: u32, e: &DirEntryInfo) -> Vec<u8> {
    let fixed = dir_fixed(class).expect("a directory class");
    let mut b = alloc::vec![0u8; fixed];
    let name_len = (2 * e.name.len()) as u32;
    let f = &e.facts;
    if class == FILE_NAMES_INFORMATION {
        put_u32(&mut b, 8, name_len);
    } else {
        put_i64(&mut b, 8, f.creation);
        put_i64(&mut b, 16, f.access);
        put_i64(&mut b, 24, f.write);
        put_i64(&mut b, 32, f.change);
        put_u64(&mut b, 40, f.size);
        put_u64(&mut b, 48, f.allocation);
        put_u32(&mut b, 56, f.attributes);
        put_u32(&mut b, 60, name_len);
        // EaSize (64) holds the reparse tag for a reparse point (Learn:
        // "FILE_FULL_DIR_INFORMATION").
        if fixed > 64 {
            put_u32(&mut b, 64, f.reparse_tag);
        }
        match class {
            FILE_ID_FULL_DIRECTORY_INFORMATION => put_u64(&mut b, 72, f.id),
            FILE_ID_BOTH_DIRECTORY_INFORMATION => put_u64(&mut b, 96, f.id),
            _ => {}
        }
    }
    put_name(&mut b, &e.name);
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_ntobj::name::wide;

    #[test]
    fn times_round_trip_from_1601() {
        // 1970-01-01 is 11644473600 s after 1601-01-01.
        assert_eq!(nt_time(Timespec::new(0, 0)), EPOCH_DIFF_100NS);
        assert_eq!(nt_time(Timespec::new(1, 500)), EPOCH_DIFF_100NS + 10_000_005);
        assert_eq!(vfs_time(EPOCH_DIFF_100NS + 10_000_005), Timespec::new(1, 500));
        // Before 1970.
        assert_eq!(vfs_time(EPOCH_DIFF_100NS - 1), Timespec::new(-1, 999_999_900));
        assert_eq!(vfs_time(0), Timespec::new(-11_644_473_600, 0));
    }

    #[test]
    fn attribute_mapping() {
        assert_eq!(attributes_of(FileType::Regular, 0o644, 0), FILE_ATTRIBUTE_NORMAL);
        assert_eq!(attributes_of(FileType::Regular, 0o444, 0), FILE_ATTRIBUTE_READONLY);
        assert_eq!(
            attributes_of(FileType::Regular, 0o644, FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE | 0x8000_0000),
            FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE
        );
        assert_eq!(attributes_of(FileType::Directory, 0o555, 0), FILE_ATTRIBUTE_DIRECTORY);
        assert_eq!(attributes_of(FileType::Symlink, 0o777, 0), FILE_ATTRIBUTE_REPARSE_POINT);
        assert_eq!(perm_for_readonly(0o644, true), 0o444);
        assert_eq!(perm_for_readonly(0o444, false), 0o666);
        assert_eq!(perm_for_readonly(0o400, false), 0o600);
        assert_eq!(perm_for_readonly(0o640, false), 0o640);
        let s = Stored { attributes: FILE_ATTRIBUTE_SYSTEM, creation: Some(42) };
        assert_eq!(Stored::parse(&s.encode()), s);
        assert_eq!(Stored::parse(&[]), Stored::default());
        assert_eq!(Stored::parse(&4u32.to_le_bytes()), Stored { attributes: 4, creation: None });
    }

    #[test]
    fn layouts() {
        let f = Facts {
            creation: 1,
            access: 2,
            write: 3,
            change: 4,
            size: 100,
            allocation: 4096,
            links: 1,
            attributes: FILE_ATTRIBUTE_ARCHIVE,
            directory: false,
            delete_pending: true,
            id: 77,
            reparse_tag: 0,
        };
        let b = basic(&f);
        assert_eq!(i64::from_le_bytes(b[16..24].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), FILE_ATTRIBUTE_ARCHIVE);
        let s = standard(&f);
        assert_eq!(u64::from_le_bytes(s[8..16].try_into().unwrap()), 100);
        assert_eq!((s[20], s[21]), (1, 0));
        let n = network_open(&f);
        assert_eq!(u64::from_le_bytes(n[40..48].try_into().unwrap()), 100);
        let name_w = wide("\\a.txt");
        let a = all(&f, &HandleFacts { access: 0x1F01FF, position: 9, mode: 0x20 }, &name_w);
        assert_eq!(a.len(), ALL_INFO_FIXED + 12);
        assert_eq!(u64::from_le_bytes(a[64..72].try_into().unwrap()), 77);
        assert_eq!(u64::from_le_bytes(a[80..88].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(a[96..100].try_into().unwrap()), 12);
        let e = DirEntryInfo { name: wide("x.y"), facts: f };
        let d = dir_entry(FILE_ID_BOTH_DIRECTORY_INFORMATION, &e);
        assert_eq!(d.len(), 104 + 6);
        assert_eq!(u32::from_le_bytes(d[60..64].try_into().unwrap()), 6);
        assert_eq!(u64::from_le_bytes(d[96..104].try_into().unwrap()), 77);
        assert_eq!(&d[104..106], &[b'x', 0]);
        let d = dir_entry(FILE_NAMES_INFORMATION, &e);
        assert_eq!(d.len(), 12 + 6);
        assert_eq!(u32::from_le_bytes(d[8..12].try_into().unwrap()), 6);
        assert_eq!(dir_fixed(FILE_BASIC_INFORMATION), None);
    }
}
