//! cpio archives in the "new ASCII" formats: `newc` (magic `070701`) and
//! `crc` (magic `070702`), the formats of a Linux initial RAM disk.
//!
//! The format, from public format documentation (the `cpio(5)` manual page
//! and the Linux initramfs buffer format description):
//!
//! - Every entry is a 110-byte header: the 6-byte magic, then 13 fields of
//!   8 hexadecimal ASCII digits: `ino`, `mode`, `uid`, `gid`, `nlink`,
//!   `mtime`, `filesize`, `devmajor`, `devminor`, `rdevmajor`, `rdevminor`,
//!   `namesize`, `check`.
//! - Then the name, `namesize` bytes including a terminating NUL, padded so
//!   that header plus name is a multiple of 4 bytes; then `filesize` bytes
//!   of data (a symbolic link's data is its target), padded to a multiple
//!   of 4.
//! - `check` is 0 in `newc`; in `crc` it is the sum of the data bytes,
//!   modulo 2^32. GNU cpio computes it for regular files only (a symbolic
//!   link's is 0), so only regular files are checked.
//! - An entry named `TRAILER!!!` ends the archive. Several archives may
//!   follow each other, with NUL padding between them (an initramfs is
//!   often built that way).
//! - Hard links are entries sharing `ino` (and device) with `nlink > 1`;
//!   writers put the data on one of them (GNU cpio: the last) and give the
//!   others size 0.
//!
//! [`entries`] parses without allocating and never panics: every length
//! and offset comes from untrusted input and is checked. [`Writer`] makes
//! archives (for tests, and for tools that build an initramfs), and
//! [`unpack`] / [`unpack_fs`] populate a file system through the VFS.

#![no_std]

extern crate alloc;

mod unpack;
mod write;

pub use unpack::{UnpackError, UnpackStats, unpack, unpack_fs};
pub use write::Writer;

use poly_vfs::FileType;

/// Magic of the `newc` format.
pub const NEWC_MAGIC: &[u8; 6] = b"070701";
/// Magic of the `crc` format.
pub const CRC_MAGIC: &[u8; 6] = b"070702";
/// Name of the entry that ends an archive.
pub const TRAILER: &[u8] = b"TRAILER!!!";
/// Size of a header.
pub const HEADER_LEN: usize = 110;

/// Which of the two formats an entry is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `070701`: no checksum.
    Newc,
    /// `070702`: `check` is the sum of the data bytes.
    Crc,
}

impl Format {
    pub const fn magic(self) -> &'static [u8; 6] {
        match self {
            Format::Newc => NEWC_MAGIC,
            Format::Crc => CRC_MAGIC,
        }
    }
}

/// The numeric fields of a header, in archive order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub ino: u32,
    /// File type (`S_IFMT` bits) and permissions.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub mtime: u32,
    pub filesize: u32,
    pub dev_major: u32,
    pub dev_minor: u32,
    pub rdev_major: u32,
    pub rdev_minor: u32,
    /// Name length including the terminating NUL.
    pub namesize: u32,
    pub check: u32,
}

/// Names of the header fields, for error messages.
const FIELDS: [&str; 13] = [
    "ino",
    "mode",
    "uid",
    "gid",
    "nlink",
    "mtime",
    "filesize",
    "devmajor",
    "devminor",
    "rdevmajor",
    "rdevminor",
    "namesize",
    "check",
];

impl Header {
    fn from_fields(f: [u32; 13]) -> Header {
        Header {
            ino: f[0],
            mode: f[1],
            uid: f[2],
            gid: f[3],
            nlink: f[4],
            mtime: f[5],
            filesize: f[6],
            dev_major: f[7],
            dev_minor: f[8],
            rdev_major: f[9],
            rdev_minor: f[10],
            namesize: f[11],
            check: f[12],
        }
    }

    pub(crate) fn fields(&self) -> [u32; 13] {
        [
            self.ino,
            self.mode,
            self.uid,
            self.gid,
            self.nlink,
            self.mtime,
            self.filesize,
            self.dev_major,
            self.dev_minor,
            self.rdev_major,
            self.rdev_minor,
            self.namesize,
            self.check,
        ]
    }
}

/// One archive member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    pub format: Format,
    pub header: Header,
    /// The name without its terminating NUL, as stored (often starting with
    /// `./` or without a leading `/`).
    pub name: &'a [u8],
    /// The data: file contents, or a symbolic link's target.
    pub data: &'a [u8],
    /// Where the header starts in the input.
    pub offset: usize,
}

impl Entry<'_> {
    /// The file type, if the mode names a known one.
    pub fn file_type(&self) -> Option<FileType> {
        FileType::from_mode(self.header.mode)
    }

    /// Permission bits including set-user-ID, set-group-ID and sticky.
    pub fn perm(&self) -> u16 {
        (self.header.mode & 0o7777) as u16
    }

    /// The device a device node stands for.
    pub fn rdev(&self) -> poly_vfs::DeviceId {
        poly_vfs::DeviceId::new(self.header.rdev_major, self.header.rdev_minor)
    }
}

/// Why an archive could not be read. Offsets are into the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ends inside a header, name or data.
    Truncated { offset: usize },
    /// No `070701` / `070702` where a header should start. The old binary
    /// and `odc` (`070707`) formats are reported this way too.
    BadMagic { offset: usize },
    /// A header field is not 8 hexadecimal digits.
    BadField { offset: usize, field: &'static str },
    /// A name that is empty, lacks its NUL or contains one.
    BadName { offset: usize },
    /// A `crc` entry whose data does not add up to its `check` field.
    Checksum { offset: usize, expected: u32, actual: u32 },
    /// The input ends after a complete entry, without a trailer.
    MissingTrailer { offset: usize },
}

/// The entries of `data`: one archive or several in a row, ending with
/// trailers. Trailers themselves are not yielded. After an error the
/// iterator ends. Empty input (or only NUL padding) holds no entries.
pub fn entries(data: &[u8]) -> Entries<'_> {
    Entries { data, pos: 0, state: State::BetweenArchives }
}

/// Iterator returned by [`entries`].
#[derive(Clone, Debug)]
pub struct Entries<'a> {
    data: &'a [u8],
    pos: usize,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Before an archive: NUL padding may come, or the end.
    BetweenArchives,
    /// Inside an archive: the next thing must be a header.
    InArchive,
    Done,
}

fn align4(n: usize) -> Option<usize> {
    n.checked_add(3).map(|n| n & !3)
}

fn hex8(field: &[u8]) -> Option<u32> {
    if field.len() != 8 {
        return None;
    }
    field.iter().try_fold(0u32, |acc, &c| {
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        Some(acc << 4 | digit as u32)
    })
}

/// Sum of the bytes, modulo 2^32: the `crc` format's checksum.
pub fn checksum(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |sum, &b| sum.wrapping_add(b as u32))
}

impl<'a> Entries<'a> {
    fn parse_at(&self, offset: usize) -> Result<(Entry<'a>, usize), Error> {
        let data = self.data;
        let header = data.get(offset..offset.checked_add(HEADER_LEN).ok_or(Error::Truncated { offset })?);
        let Some(header) = header else {
            // Not even a magic's worth: call it truncated if what is there
            // could start a header, a bad magic otherwise.
            let rest = &data[offset.min(data.len())..];
            let n = rest.len().min(6);
            return if rest[..n] == NEWC_MAGIC[..n] || rest[..n] == CRC_MAGIC[..n] {
                Err(Error::Truncated { offset })
            } else {
                Err(Error::BadMagic { offset })
            };
        };
        let format = match &header[..6] {
            m if m == NEWC_MAGIC => Format::Newc,
            m if m == CRC_MAGIC => Format::Crc,
            _ => return Err(Error::BadMagic { offset }),
        };
        let mut fields = [0u32; 13];
        for (i, f) in fields.iter_mut().enumerate() {
            let start = 6 + 8 * i;
            *f = hex8(&header[start..start + 8]).ok_or(Error::BadField { offset, field: FIELDS[i] })?;
        }
        let h = Header::from_fields(fields);
        let name_start = offset + HEADER_LEN;
        let namesize = h.namesize as usize;
        if namesize == 0 {
            return Err(Error::BadName { offset });
        }
        let name_end = name_start.checked_add(namesize).ok_or(Error::Truncated { offset })?;
        let name = data.get(name_start..name_end).ok_or(Error::Truncated { offset })?;
        let (name, nul) = name.split_at(namesize - 1);
        if nul != [0] || name.is_empty() || name.contains(&0) {
            return Err(Error::BadName { offset });
        }
        let data_start = align4(name_end).ok_or(Error::Truncated { offset })?;
        let data_end = data_start.checked_add(h.filesize as usize).ok_or(Error::Truncated { offset })?;
        // Without data, the name's padding may be what the input lacks.
        let body = if h.filesize == 0 {
            &[][..]
        } else {
            data.get(data_start..data_end).ok_or(Error::Truncated { offset })?
        };
        if format == Format::Crc && FileType::from_mode(h.mode) == Some(FileType::Regular) {
            let actual = checksum(body);
            if actual != h.check {
                return Err(Error::Checksum { offset, expected: h.check, actual });
            }
        }
        // The last padding may be missing at the very end of the input.
        let next = align4(data_end).ok_or(Error::Truncated { offset })?.min(data.len());
        Ok((Entry { format, header: h, name, data: body, offset }, next))
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = Result<Entry<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.state {
                State::Done => return None,
                State::BetweenArchives => {
                    while self.data.get(self.pos) == Some(&0) {
                        self.pos += 1;
                    }
                    if self.pos >= self.data.len() {
                        self.state = State::Done;
                        return None;
                    }
                    self.state = State::InArchive;
                }
                State::InArchive => {
                    if self.pos >= self.data.len() {
                        self.state = State::Done;
                        return Some(Err(Error::MissingTrailer { offset: self.pos }));
                    }
                    match self.parse_at(self.pos) {
                        Err(e) => {
                            self.state = State::Done;
                            return Some(Err(e));
                        }
                        Ok((entry, next)) => {
                            self.pos = next;
                            if entry.name == TRAILER {
                                self.state = State::BetweenArchives;
                                continue;
                            }
                            return Some(Ok(entry));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
