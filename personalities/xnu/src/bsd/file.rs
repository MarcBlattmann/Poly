//! Files: Darwin's open flags, `struct stat64`, directory records and
//! `fcntl` commands (layouts and values from the spec sheets and the macOS
//! SDK headers `<sys/fcntl.h>`, `<sys/stat.h>`, `<sys/dirent.h>`).

use alloc::vec::Vec;

/// `AT_FDCWD` on Darwin (`<sys/fcntl.h>`; the fstatat64 sheet).
pub const AT_FDCWD: i32 = -2;
/// `AT_SYMLINK_NOFOLLOW` (`<sys/fcntl.h>`).
pub const AT_SYMLINK_NOFOLLOW: u32 = 0x20;

/// `<sys/fcntl.h>` open flags.
pub mod o {
    pub const ACCMODE: u32 = 3;
    pub const NONBLOCK: u32 = 0x4;
    pub const APPEND: u32 = 0x8;
    pub const SYNC: u32 = 0x80;
    pub const NOFOLLOW: u32 = 0x100;
    pub const CREAT: u32 = 0x200;
    pub const TRUNC: u32 = 0x400;
    pub const EXCL: u32 = 0x800;
    pub const NOCTTY: u32 = 0x20000;
    pub const DIRECTORY: u32 = 0x100000;
    pub const CLOEXEC: u32 = 0x100_0000;
}

/// `fcntl` commands (`<sys/fcntl.h>`).
pub mod f {
    pub const DUPFD: u32 = 0;
    pub const GETFD: u32 = 1;
    pub const SETFD: u32 = 2;
    pub const GETFL: u32 = 3;
    pub const SETFL: u32 = 4;
    pub const GETPATH: u32 = 50;
    pub const DUPFD_CLOEXEC: u32 = 67;
    /// `FD_CLOEXEC`.
    pub const FD_CLOEXEC: u64 = 1;
    /// `MAXPATHLEN`: the buffer `F_GETPATH` writes into (`<sys/param.h>`).
    pub const MAXPATHLEN: usize = 1024;
}

/// The access mode of an open file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

/// An `open` decoded from Darwin's flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenHow {
    pub access: Access,
    pub create: bool,
    pub truncate: bool,
    pub exclusive: bool,
    pub append: bool,
    pub nonblock: bool,
    pub directory: bool,
    pub nofollow: bool,
    pub cloexec: bool,
    pub noctty: bool,
    /// The mode for a new file (before the umask).
    pub mode: u16,
}

impl OpenHow {
    /// Decode `flags` and `mode`. An access mode of 3 is `EINVAL`.
    pub fn decode(flags: u32, mode: u32) -> Option<OpenHow> {
        let access = match flags & o::ACCMODE {
            0 => Access::Read,
            1 => Access::Write,
            2 => Access::ReadWrite,
            _ => return None,
        };
        Some(OpenHow {
            access,
            create: flags & o::CREAT != 0,
            truncate: flags & o::TRUNC != 0,
            exclusive: flags & o::EXCL != 0,
            append: flags & o::APPEND != 0,
            nonblock: flags & o::NONBLOCK != 0,
            directory: flags & o::DIRECTORY != 0,
            nofollow: flags & o::NOFOLLOW != 0,
            cloexec: flags & o::CLOEXEC != 0,
            noctty: flags & o::NOCTTY != 0,
            mode: (mode & 0o7777) as u16,
        })
    }
}

/// The status flags `F_GETFL` reports and `F_SETFL` sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub access: Access,
    pub append: bool,
    pub nonblock: bool,
}

impl Status {
    /// As Darwin's flags.
    pub fn bits(&self) -> u64 {
        let acc = match self.access {
            Access::Read => 0,
            Access::Write => 1,
            Access::ReadWrite => 2,
        };
        acc | if self.append { u64::from(o::APPEND) } else { 0 }
            | if self.nonblock { u64::from(o::NONBLOCK) } else { 0 }
    }
}

/// A file's attributes, as the kernel reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    /// Device of the file system, Darwin encoding (major in the top byte).
    pub dev: u32,
    pub ino: u64,
    /// Type and permission bits (`S_IF*` values are the same as Linux's).
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub size: u64,
    pub blocks: u64,
    pub blksize: u32,
    /// atime, mtime, ctime, birthtime: seconds and nanoseconds.
    pub times: [(i64, i64); 4],
}

/// `S_IFMT` and the directory type (`<sys/stat.h>`).
pub const S_IFMT: u32 = 0o170000;
pub const S_IFDIR: u32 = 0o040000;

/// The size of `struct stat64` (the fstat64 sheet).
pub const STAT64_SIZE: usize = 144;

impl Stat {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }

    /// `struct stat64`: st_dev i32 at 0, st_mode u16 at 4, st_nlink u16 at
    /// 6, st_ino at 8, st_uid 16, st_gid 20, st_rdev 24, four timespecs
    /// 32..96, st_size 96, st_blocks 104, st_blksize 112, st_flags 116,
    /// st_gen 120 (the fstat64 sheet).
    pub fn encode(&self) -> [u8; STAT64_SIZE] {
        let mut b = [0u8; STAT64_SIZE];
        b[0..4].copy_from_slice(&self.dev.to_le_bytes());
        b[4..6].copy_from_slice(&(self.mode as u16).to_le_bytes());
        b[6..8].copy_from_slice(&(self.nlink.min(0xffff) as u16).to_le_bytes());
        b[8..16].copy_from_slice(&self.ino.to_le_bytes());
        b[16..20].copy_from_slice(&self.uid.to_le_bytes());
        b[20..24].copy_from_slice(&self.gid.to_le_bytes());
        b[24..28].copy_from_slice(&self.rdev.to_le_bytes());
        for (i, (s, ns)) in self.times.iter().enumerate() {
            let at = 32 + i * 16;
            b[at..at + 8].copy_from_slice(&s.to_le_bytes());
            b[at + 8..at + 16].copy_from_slice(&ns.to_le_bytes());
        }
        b[96..104].copy_from_slice(&self.size.to_le_bytes());
        b[104..112].copy_from_slice(&self.blocks.to_le_bytes());
        b[112..116].copy_from_slice(&self.blksize.to_le_bytes());
        b
    }
}

/// `d_type` values (`<sys/dirent.h>`).
pub mod dt {
    pub const UNKNOWN: u8 = 0;
    pub const FIFO: u8 = 1;
    pub const CHR: u8 = 2;
    pub const DIR: u8 = 4;
    pub const BLK: u8 = 6;
    pub const REG: u8 = 8;
    pub const LNK: u8 = 10;
    pub const SOCK: u8 = 12;
}

/// One directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry<'a> {
    pub ino: u64,
    pub kind: u8,
    pub name: &'a [u8],
}

/// The smallest buffer getdirentries64 takes (its sheet: 32 is `EINVAL`).
pub const MIN_DIRBUF: u64 = 64;
/// The base getdirentries64 reports once everything was read.
pub const DIR_END_BASE: u64 = 0x7fff_ffff;

/// The length of an entry's record: 21 bytes of header, the name and a
/// NUL, rounded up to 8, at least 32 (the sheet: 32 for names up to 10
/// bytes, 280 for 255).
pub const fn record_len(name_len: usize) -> usize {
    let n = (21 + name_len + 1).next_multiple_of(8);
    if n < 32 { 32 } else { n }
}

/// Append `e`'s record (`struct dirent` with 64-bit inodes: d_ino,
/// d_seekoff, d_reclen, d_namlen, d_type, the name, a NUL) to `out` if it
/// fits in `room` more bytes.
pub fn push_record(out: &mut Vec<u8>, room: usize, e: &DirEntry<'_>) -> bool {
    let len = record_len(e.name.len());
    if len > room || e.name.len() > 1023 {
        return false;
    }
    let start = out.len();
    out.extend_from_slice(&e.ino.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
    out.push(e.kind);
    out.extend_from_slice(e.name);
    out.resize(start + len, 0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_flags() {
        let h = OpenHow::decode(o::CREAT | o::EXCL | 2 | o::CLOEXEC, 0o100644).unwrap();
        assert_eq!(h.access, Access::ReadWrite);
        assert!(h.create && h.exclusive && h.cloexec && !h.truncate);
        assert_eq!(h.mode, 0o644);
        assert!(OpenHow::decode(3, 0).is_none());
    }

    #[test]
    fn stat64_layout() {
        let s = Stat {
            dev: 0x0100_0004,
            ino: 0x1234,
            mode: 0o100644,
            nlink: 1,
            uid: 501,
            gid: 20,
            size: 5,
            blocks: 8,
            blksize: 4096,
            times: [(1, 2), (3, 4), (5, 6), (7, 8)],
            ..Stat::default()
        };
        let b = s.encode();
        assert_eq!(u16::from_le_bytes([b[4], b[5]]), 0o100644);
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 1);
        assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), 0x1234);
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 501);
        assert_eq!(i64::from_le_bytes(b[48..56].try_into().unwrap()), 3); // mtime
        assert_eq!(u64::from_le_bytes(b[96..104].try_into().unwrap()), 5);
        assert_eq!(u32::from_le_bytes(b[112..116].try_into().unwrap()), 4096);
    }

    #[test]
    fn dirent_records() {
        assert_eq!(record_len(1), 32);
        assert_eq!(record_len(10), 32);
        assert_eq!(record_len(11), 40);
        assert_eq!(record_len(255), 280);
        let mut out = Vec::new();
        assert!(push_record(&mut out, 64, &DirEntry { ino: 7, kind: dt::DIR, name: b"." }));
        assert!(push_record(&mut out, 32, &DirEntry { ino: 2, kind: dt::DIR, name: b".." }));
        assert!(!push_record(&mut out, 0, &DirEntry { ino: 3, kind: dt::REG, name: b"x" }));
        assert_eq!(out.len(), 64);
        assert_eq!(u16::from_le_bytes([out[16], out[17]]), 32);
        assert_eq!(u16::from_le_bytes([out[18], out[19]]), 1);
        assert_eq!(out[20], dt::DIR);
        assert_eq!(&out[21..23], b".\0");
        assert_eq!(&out[32 + 21..32 + 24], b"..\0");
    }

    #[test]
    fn status_bits() {
        let s = Status { access: Access::ReadWrite, append: true, nonblock: false };
        assert_eq!(s.bits(), 2 | 8);
    }
}
