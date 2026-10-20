//! Making archives.

use crate::{Format, Header, TRAILER, checksum};
use alloc::vec::Vec;
use poly_vfs::{DeviceId, FileType};

/// Builds an archive in memory.
///
/// The convenience methods number inodes from 1 and give every entry one
/// link; [`Writer::push`] writes any header (and [`Writer::hard_link`]
/// writes a group of links the way GNU cpio does).
#[derive(Clone, Debug)]
pub struct Writer {
    format: Format,
    out: Vec<u8>,
    next_ino: u32,
    /// Modification time given to entries made by the convenience methods.
    pub mtime: u32,
}

impl Writer {
    pub fn new(format: Format) -> Writer {
        Writer { format, out: Vec::new(), next_ino: 1, mtime: 0 }
    }

    fn ino(&mut self) -> u32 {
        let ino = self.next_ino;
        self.next_ino += 1;
        ino
    }

    /// Appends one entry. `namesize`, `filesize` and (for `crc`) `check`
    /// are filled in from `name` and `data`; like GNU cpio, `check` is
    /// only computed for regular files.
    ///
    /// # Panics
    ///
    /// If `name` or `data` is 4 GiB or longer, which the format cannot hold.
    pub fn push(&mut self, header: Header, name: &[u8], data: &[u8]) {
        let mut h = header;
        h.namesize = u32::try_from(name.len() + 1).expect("cpio name too long");
        h.filesize = u32::try_from(data.len()).expect("cpio member too large");
        let regular = FileType::from_mode(h.mode) == Some(FileType::Regular);
        h.check = if self.format == Format::Crc && regular { checksum(data) } else { 0 };
        self.out.extend_from_slice(self.format.magic());
        for field in h.fields() {
            self.out.extend_from_slice(alloc::format!("{field:08x}").as_bytes());
        }
        self.out.extend_from_slice(name);
        self.out.push(0);
        self.pad();
        self.out.extend_from_slice(data);
        self.pad();
    }

    fn pad(&mut self) {
        while !self.out.len().is_multiple_of(4) {
            self.out.push(0);
        }
    }

    fn simple(&mut self, kind: FileType, perm: u16, name: &[u8], data: &[u8], rdev: DeviceId) {
        let ino = self.ino();
        let nlink = if kind == FileType::Directory { 2 } else { 1 };
        let header = Header {
            ino,
            mode: kind.mode_bits() | (perm & 0o7777) as u32,
            nlink,
            mtime: self.mtime,
            rdev_major: rdev.major,
            rdev_minor: rdev.minor,
            ..Header::default()
        };
        self.push(header, name, data);
    }

    pub fn dir(&mut self, name: &[u8], perm: u16) {
        self.simple(FileType::Directory, perm, name, &[], DeviceId::default());
    }

    pub fn file(&mut self, name: &[u8], perm: u16, data: &[u8]) {
        self.simple(FileType::Regular, perm, name, data, DeviceId::default());
    }

    pub fn symlink(&mut self, name: &[u8], target: &[u8]) {
        self.simple(FileType::Symlink, 0o777, name, target, DeviceId::default());
    }

    /// A device node, FIFO or socket.
    pub fn node(&mut self, name: &[u8], kind: FileType, perm: u16, rdev: DeviceId) {
        self.simple(kind, perm, name, &[], rdev);
    }

    /// A regular file with several names, one shared inode number, and the
    /// data on the last entry.
    pub fn hard_link(&mut self, names: &[&[u8]], perm: u16, data: &[u8]) {
        let ino = self.ino();
        let header = Header {
            ino,
            mode: FileType::Regular.mode_bits() | (perm & 0o7777) as u32,
            nlink: names.len() as u32,
            mtime: self.mtime,
            ..Header::default()
        };
        for (i, name) in names.iter().enumerate() {
            self.push(header, name, if i + 1 == names.len() { data } else { &[] });
        }
    }

    /// Appends the trailer and returns the archive.
    pub fn finish(mut self) -> Vec<u8> {
        self.push(Header { nlink: 1, ..Header::default() }, TRAILER, &[]);
        self.out
    }
}
