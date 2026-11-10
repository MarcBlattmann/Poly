//! The on-disk structures of squashfs 4 (the format's public description):
//! all integers are little endian.

use poly_vfs::{DeviceId, FileType};

pub const MAGIC: u32 = 0x7371_7368;
pub const SUPER_SIZE: usize = 96;
/// Metadata blocks hold at most this much once decompressed.
pub const METADATA_SIZE: usize = 8192;

/// Compression ids.
pub const COMP_GZIP: u16 = 1;
pub const COMP_LZO: u16 = 3;
pub const COMP_XZ: u16 = 4;
pub const COMP_LZ4: u16 = 5;
pub const COMP_ZSTD: u16 = 6;

pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
pub fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// The superblock.
#[derive(Clone, Debug)]
pub struct Super {
    pub inodes: u32,
    pub block_size: u32,
    pub fragments: u32,
    pub compression: u16,
    pub block_log: u16,
    pub ids: u16,
    pub major: u16,
    pub root: u64,
    pub bytes_used: u64,
    pub id_table: u64,
    pub xattr_table: u64,
    pub inode_table: u64,
    pub dir_table: u64,
    pub frag_table: u64,
}

impl Super {
    pub fn parse(b: &[u8]) -> Super {
        Super {
            inodes: u32_at(b, 4),
            block_size: u32_at(b, 12),
            fragments: u32_at(b, 16),
            compression: u16_at(b, 20),
            block_log: u16_at(b, 22),
            ids: u16_at(b, 26),
            major: u16_at(b, 28),
            root: u64_at(b, 32),
            bytes_used: u64_at(b, 40),
            id_table: u64_at(b, 48),
            xattr_table: u64_at(b, 56),
            inode_table: u64_at(b, 64),
            dir_table: u64_at(b, 72),
            frag_table: u64_at(b, 80),
        }
    }
}

/// An inode's kind from its type number (1 to 14, basic then extended).
pub fn kind_of(t: u16) -> Option<FileType> {
    Some(match t {
        1 | 8 => FileType::Directory,
        2 | 9 => FileType::Regular,
        3 | 10 => FileType::Symlink,
        4 | 11 => FileType::BlockDevice,
        5 | 12 => FileType::CharDevice,
        6 | 13 => FileType::Fifo,
        7 | 14 => FileType::Socket,
        _ => return None,
    })
}

/// A device number in squashfs's encoding (major in bits 8 to 19, the
/// minor split around it).
pub fn decode_rdev(r: u32) -> DeviceId {
    DeviceId::new((r >> 8) & 0xfff, (r & 0xff) | ((r >> 12) & 0xfff00))
}
