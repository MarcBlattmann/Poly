//! ext4's metadata checksums ("ext4 Data Structures and Algorithms",
//! "Checksums"): CRC-32C chained from a per-volume seed (the UUID's
//! CRC-32C, or `s_checksum_seed`), and for objects that belong to an inode
//! from a per-inode seed (the volume seed, then the inode number and its
//! generation). Each function returns the value stored on disk.

use crate::crc::{crc16, crc32c};
use crate::layout::{DIR_TAIL, Inode, SUPERBLOCK_SIZE, ino, le16};
use crate::{Csum, Geometry};

/// The superblock's `s_checksum`: CRC-32C of everything before it.
pub fn sb(raw: &[u8]) -> u32 {
    crc32c(!0, &raw[..SUPERBLOCK_SIZE - 4])
}

/// A group descriptor's `bg_checksum` (the 16 bits at 0x1E), from its raw
/// bytes (the stored checksum is skipped).
pub fn group_desc(geo: &Geometry, g: u32, raw: &[u8]) -> u16 {
    match geo.csum {
        Csum::None => 0,
        Csum::Meta(seed) => {
            let c = crc32c(seed, &g.to_le_bytes());
            let c = crc32c(c, &raw[..0x1E]);
            let c = crc32c(c, &[0, 0]);
            (crc32c(c, &raw[0x20..geo.desc_size]) & 0xFFFF) as u16
        }
        Csum::Gdt => {
            let c = crc16(!0, &geo.uuid);
            let c = crc16(c, &g.to_le_bytes());
            let c = crc16(c, &raw[..0x1E]);
            if geo.desc_size > 0x20 { crc16(c, &raw[0x20..geo.desc_size]) } else { c }
        }
    }
}

/// A bitmap's checksum: over the bytes that hold the group's bits.
pub fn bitmap(seed: u32, data: &[u8], bits: u64) -> u32 {
    crc32c(seed, &data[..(bits / 8) as usize])
}

/// The per-inode seed: volume seed, inode number, generation.
pub fn inode_seed(seed: u32, ino: u32, generation: u32) -> u32 {
    crc32c(crc32c(seed, &ino.to_le_bytes()), &generation.to_le_bytes())
}

/// Whether the inode record has room for the checksum's high half.
fn has_hi(raw: &[u8]) -> bool {
    raw.len() > 128 && Inode::extra(raw) >= 4
}

/// An inode record's checksum over the whole record, both checksum fields
/// counted as zeros.
pub fn inode(seed: u32, ino_num: u32, raw: &[u8]) -> u32 {
    let generation = u32::from_le_bytes(raw[ino::GENERATION..ino::GENERATION + 4].try_into().unwrap_or([0; 4]));
    let s = inode_seed(seed, ino_num, generation);
    let c = crc32c(s, &raw[..ino::CHECKSUM_LO]);
    let c = crc32c(c, &[0, 0]);
    if raw.len() <= 128 {
        return crc32c(c, &raw[ino::CHECKSUM_LO + 2..]);
    }
    let c = crc32c(c, &raw[ino::CHECKSUM_LO + 2..ino::CHECKSUM_HI]);
    if has_hi(raw) {
        let c = crc32c(c, &[0, 0]);
        crc32c(c, &raw[ino::CHECKSUM_HI + 2..])
    } else {
        crc32c(c, &raw[ino::CHECKSUM_HI..])
    }
}

/// Stores the record's checksum (the low half, and the high half if the
/// record has room for it).
pub fn set_inode(seed: u32, ino_num: u32, raw: &mut [u8]) {
    let c = inode(seed, ino_num, raw);
    raw[ino::CHECKSUM_LO..ino::CHECKSUM_LO + 2].copy_from_slice(&(c as u16).to_le_bytes());
    if has_hi(raw) {
        raw[ino::CHECKSUM_HI..ino::CHECKSUM_HI + 2].copy_from_slice(&((c >> 16) as u16).to_le_bytes());
    }
}

/// Whether the stored checksum matches (only the low half if the record
/// has no room for the high one).
pub fn inode_ok(seed: u32, ino_num: u32, raw: &[u8]) -> bool {
    let c = inode(seed, ino_num, raw);
    let lo = le16(raw, ino::CHECKSUM_LO) as u32;
    if has_hi(raw) { lo | (le16(raw, ino::CHECKSUM_HI) as u32) << 16 == c } else { lo == c & 0xFFFF }
}

/// A directory leaf block's tail checksum: over everything before the tail.
pub fn dir_block(iseed: u32, block: &[u8]) -> u32 {
    crc32c(iseed, &block[..block.len() - DIR_TAIL])
}

/// A hash index block's checksum: over the header and the `count` entries
/// in use (from `count_off`, where the limit and count are), then the
/// 8-byte tail with its checksum as zeros.
pub fn dx_block(iseed: u32, block: &[u8], count_off: usize, count: usize) -> u32 {
    let c = crc32c(iseed, &block[..count_off + count * 8]);
    crc32c(c, &[0; 8])
}

/// An extent tree block's checksum: over the header and `max` entries.
pub fn extent_block(iseed: u32, block: &[u8], tail: usize) -> u32 {
    crc32c(iseed, &block[..tail])
}

/// An extended attribute block's checksum: the block number (64 bits),
/// then the block with the checksum field (at 0x10) as zeros.
pub fn xattr_block(seed: u32, block_nr: u64, block: &[u8]) -> u32 {
    let c = crc32c(seed, &block_nr.to_le_bytes());
    let c = crc32c(c, &block[..0x10]);
    let c = crc32c(c, &[0; 4]);
    crc32c(c, &block[0x14..])
}
