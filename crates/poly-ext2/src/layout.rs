//! The ext2/ext3/ext4 on-disk format: superblock, group descriptors, inodes
//! and directory entries, decoded from and encoded into little-endian bytes.
//!
//! Field offsets, sizes and constant values follow the public format
//! documentation: "The Second Extended File System: Internal Layout" by
//! Dave Poirier (nongnu.org/ext2-doc), and for ext3/ext4 the kernel.org
//! document "ext4 Data Structures and Algorithms" (superblock, 64-bit group
//! descriptors, the large inode, checksums, feature flags). Extents, the
//! journal and inline data have modules of their own.
//!
//! Decoding never panics: every reader takes the bytes it needs from a slice
//! of the right length and the callers check structure bounds first.
//! Encoding patches known fields into the existing bytes, so fields this
//! driver does not know survive a rewrite.

use poly_vfs::{FileType, Timespec};

/// `s_magic`.
pub const MAGIC: u16 = 0xEF53;
/// The superblock is 1024 bytes at byte offset 1024 of the volume.
pub const SUPERBLOCK_OFFSET: u64 = 1024;
pub const SUPERBLOCK_SIZE: usize = 1024;
/// Revision 0 (fixed inode size 128, first inode 11) and 1 ("dynamic").
pub const GOOD_OLD_REV: u32 = 0;
pub const DYNAMIC_REV: u32 = 1;
pub const GOOD_OLD_INODE_SIZE: u32 = 128;
pub const GOOD_OLD_FIRST_INO: u32 = 11;
/// Reserved inode numbers.
pub const ROOT_INO: u32 = 2;
pub const JOURNAL_INO: u32 = 8;
/// `s_state` bits.
pub const STATE_VALID: u16 = 1;
pub const STATE_ERROR: u16 = 2;
/// `s_creator_os` values whose inode layout carries uid/gid high halves.
pub const OS_LINUX: u32 = 0;
pub const OS_HURD: u32 = 1;

/// `s_feature_compat`: features a driver may ignore entirely.
pub const COMPAT_DIR_PREALLOC: u32 = 0x0001;
pub const COMPAT_HAS_JOURNAL: u32 = 0x0004;
pub const COMPAT_EXT_ATTR: u32 = 0x0008;
pub const COMPAT_RESIZE_INODE: u32 = 0x0010;
pub const COMPAT_DIR_INDEX: u32 = 0x0020;
/// Superblock backups only in the (up to) two groups `s_backup_bgs` names.
pub const COMPAT_SPARSE_SUPER2: u32 = 0x0200;
pub const COMPAT_FAST_COMMIT: u32 = 0x0400;
/// `s_feature_incompat`: features a driver must understand to mount at all.
pub const INCOMPAT_FILETYPE: u32 = 0x0002;
/// The journal must be replayed before the volume is consistent.
pub const INCOMPAT_RECOVER: u32 = 0x0004;
pub const INCOMPAT_JOURNAL_DEV: u32 = 0x0008;
pub const INCOMPAT_META_BG: u32 = 0x0010;
pub const INCOMPAT_EXTENTS: u32 = 0x0040;
pub const INCOMPAT_64BIT: u32 = 0x0080;
pub const INCOMPAT_MMP: u32 = 0x0100;
pub const INCOMPAT_FLEX_BG: u32 = 0x0200;
/// The checksum seed is `s_checksum_seed`, not derived from the UUID.
pub const INCOMPAT_CSUM_SEED: u32 = 0x2000;
pub const INCOMPAT_INLINE_DATA: u32 = 0x8000;
pub const INCOMPAT_SUPPORTED: u32 = INCOMPAT_FILETYPE
    | INCOMPAT_RECOVER
    | INCOMPAT_EXTENTS
    | INCOMPAT_64BIT
    | INCOMPAT_FLEX_BG
    | INCOMPAT_CSUM_SEED
    | INCOMPAT_INLINE_DATA;
/// Understood for reading only: the volume mounts read-only.
pub const INCOMPAT_READ_ONLY: u32 = INCOMPAT_INLINE_DATA;
/// `s_feature_ro_compat`: features a driver must understand to write.
pub const RO_COMPAT_SPARSE_SUPER: u32 = 0x0001;
pub const RO_COMPAT_LARGE_FILE: u32 = 0x0002;
pub const RO_COMPAT_HUGE_FILE: u32 = 0x0008;
/// Group descriptors carry a CRC-16 and the "uninitialised" flags.
pub const RO_COMPAT_GDT_CSUM: u32 = 0x0010;
pub const RO_COMPAT_DIR_NLINK: u32 = 0x0020;
pub const RO_COMPAT_EXTRA_ISIZE: u32 = 0x0040;
/// CRC-32C on every metadata structure.
pub const RO_COMPAT_METADATA_CSUM: u32 = 0x0400;
pub const RO_COMPAT_SUPPORTED: u32 = RO_COMPAT_SPARSE_SUPER
    | RO_COMPAT_LARGE_FILE
    | RO_COMPAT_HUGE_FILE
    | RO_COMPAT_GDT_CSUM
    | RO_COMPAT_DIR_NLINK
    | RO_COMPAT_EXTRA_ISIZE
    | RO_COMPAT_METADATA_CSUM;

/// `i_flags` bits this driver acts on.
pub const FL_IMMUTABLE: u32 = 0x0000_0010;
pub const FL_APPEND: u32 = 0x0000_0020;
/// A hash-indexed directory (`EXT2_INDEX_FL`); cleared when a driver that
/// does not maintain the index changes the directory.
pub const FL_INDEX: u32 = 0x0000_1000;
/// `i_blocks` counts file system blocks, not 512-byte units.
pub const FL_HUGE_FILE: u32 = 0x0004_0000;
/// `i_block` holds an extent tree.
pub const FL_EXTENTS: u32 = 0x0008_0000;
/// The inode holds extended attribute data (`ea_inode`).
pub const FL_EA_INODE: u32 = 0x0020_0000;
/// The file's data is in the inode (`i_block` and the `system.data`
/// attribute).
pub const FL_INLINE_DATA: u32 = 0x1000_0000;

/// Group descriptor `bg_flags`.
pub const BG_INODE_UNINIT: u16 = 0x1;
pub const BG_BLOCK_UNINIT: u16 = 0x2;
pub const BG_INODE_ZEROED: u16 = 0x4;

/// Direct block pointers in `i_block`, then single, double and triple
/// indirect.
pub const NDIR_BLOCKS: usize = 12;
pub const IND_BLOCK: usize = 12;
pub const DIND_BLOCK: usize = 13;
pub const TIND_BLOCK: usize = 14;
pub const N_BLOCKS: usize = 15;
/// A symbolic link target shorter than this lives in `i_block` itself.
pub const FAST_SYMLINK_MAX: usize = 60;
/// Most links an inode may have: `EXT2_LINK_MAX`, and ext4's.
pub const LINK_MAX: u16 = 32000;
pub const EXT4_LINK_MAX: u16 = 65000;

/// Group descriptor size without `64bit`.
pub const DESC_SIZE: usize = 32;
/// With `64bit`, at least this (`s_desc_size`).
pub const DESC_SIZE_64: usize = 64;
/// Extended attribute block header magic.
pub const XATTR_MAGIC: u32 = 0xEA02_0000;

/// Directory entry header: inode (4), rec_len (2), name_len (1 or 2), type (1).
pub const DIRENT_HEADER: usize = 8;
/// With `metadata_csum`, a directory leaf block ends with this fake entry:
/// inode 0, rec_len 12, name_len 0, type 0xDE, then the CRC-32C.
pub const DIR_TAIL: usize = 12;
pub const DIR_TAIL_TYPE: u8 = 0xDE;

pub fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

pub fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn le64(b: &[u8], at: usize) -> u64 {
    le32(b, at) as u64 | (le32(b, at + 4) as u64) << 32
}

pub fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// The superblock, kept as its 1024 raw bytes (on the heap: kernel stacks
/// are small); accessors decode fields.
#[derive(Clone)]
pub struct Superblock {
    pub raw: alloc::boxed::Box<[u8; SUPERBLOCK_SIZE]>,
}

macro_rules! field {
    ($get:ident, $set:ident, u32, $off:expr) => {
        pub fn $get(&self) -> u32 {
            le32(&self.raw[..], $off)
        }
        #[allow(dead_code)]
        pub fn $set(&mut self, v: u32) {
            put32(&mut self.raw[..], $off, v)
        }
    };
    ($get:ident, $set:ident, u16, $off:expr) => {
        pub fn $get(&self) -> u16 {
            le16(&self.raw[..], $off)
        }
        #[allow(dead_code)]
        pub fn $set(&mut self, v: u16) {
            put16(&mut self.raw[..], $off, v)
        }
    };
    ($get:ident, $set:ident, u8, $off:expr) => {
        pub fn $get(&self) -> u8 {
            self.raw[$off]
        }
        #[allow(dead_code)]
        pub fn $set(&mut self, v: u8) {
            self.raw[$off] = v
        }
    };
}

impl core::fmt::Debug for Superblock {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Superblock")
            .field("blocks", &self.blocks_count())
            .field("inodes", &self.inodes_count())
            .field("rev", &self.rev_level())
            .finish_non_exhaustive()
    }
}

impl Superblock {
    field!(inodes_count, set_inodes_count, u32, 0);
    field!(blocks_count_lo, set_blocks_count_lo, u32, 4);
    field!(r_blocks_count_lo, set_r_blocks_count_lo, u32, 8);
    field!(free_blocks_count_lo, set_free_blocks_count_lo, u32, 12);
    field!(free_inodes_count, set_free_inodes_count, u32, 16);
    field!(first_data_block, set_first_data_block, u32, 20);
    field!(log_block_size, set_log_block_size, u32, 24);
    field!(log_frag_size, set_log_frag_size, u32, 28);
    field!(blocks_per_group, set_blocks_per_group, u32, 32);
    field!(frags_per_group, set_frags_per_group, u32, 36);
    field!(inodes_per_group, set_inodes_per_group, u32, 40);
    field!(mtime, set_mtime, u32, 44);
    field!(wtime, set_wtime, u32, 48);
    field!(mnt_count, set_mnt_count, u16, 52);
    field!(max_mnt_count, set_max_mnt_count, u16, 54);
    field!(magic, set_magic, u16, 56);
    field!(state, set_state, u16, 58);
    field!(errors, set_errors, u16, 60);
    field!(lastcheck, set_lastcheck, u32, 64);
    field!(creator_os, set_creator_os, u32, 72);
    field!(rev_level, set_rev_level, u32, 76);
    field!(first_ino_raw, set_first_ino, u32, 84);
    field!(inode_size_raw, set_inode_size, u16, 88);
    field!(block_group_nr, set_block_group_nr, u16, 90);
    field!(feature_compat_raw, set_feature_compat, u32, 92);
    field!(feature_incompat_raw, set_feature_incompat, u32, 96);
    field!(feature_ro_compat_raw, set_feature_ro_compat, u32, 100);
    field!(reserved_gdt_blocks, set_reserved_gdt_blocks, u16, 206);
    field!(journal_inum, set_journal_inum, u32, 224);
    field!(journal_dev, set_journal_dev, u32, 228);
    field!(last_orphan, set_last_orphan, u32, 232);
    field!(def_hash_version, set_def_hash_version, u8, 252);
    field!(jnl_backup_type, set_jnl_backup_type, u8, 253);
    field!(desc_size_raw, set_desc_size, u16, 254);
    field!(mkfs_time, set_mkfs_time, u32, 264);
    field!(blocks_count_hi, set_blocks_count_hi, u32, 336);
    field!(r_blocks_count_hi, set_r_blocks_count_hi, u32, 340);
    field!(free_blocks_count_hi, set_free_blocks_count_hi, u32, 344);
    field!(min_extra_isize, set_min_extra_isize, u16, 348);
    field!(want_extra_isize, set_want_extra_isize, u16, 350);
    field!(flags, set_flags, u32, 352);
    field!(log_groups_per_flex, set_log_groups_per_flex, u8, 372);
    field!(checksum_type, set_checksum_type, u8, 373);
    field!(checksum_seed, set_checksum_seed, u32, 624);
    field!(checksum, set_checksum, u32, 1020);

    pub fn new(raw: [u8; SUPERBLOCK_SIZE]) -> Superblock {
        Superblock::from_bytes(&raw)
    }

    /// From the first 1024 bytes of `raw` (zeros past its end).
    pub fn from_bytes(raw: &[u8]) -> Superblock {
        let mut v = alloc::vec![0u8; SUPERBLOCK_SIZE];
        let n = raw.len().min(SUPERBLOCK_SIZE);
        v[..n].copy_from_slice(&raw[..n]);
        let raw: alloc::boxed::Box<[u8; SUPERBLOCK_SIZE]> = v.into_boxed_slice().try_into().expect("1024 bytes");
        Superblock { raw }
    }

    /// Revision 1 fields read as their revision 0 meaning on a revision 0
    /// file system.
    pub fn first_ino(&self) -> u32 {
        if self.rev_level() == GOOD_OLD_REV { GOOD_OLD_FIRST_INO } else { self.first_ino_raw() }
    }

    pub fn inode_size(&self) -> u32 {
        if self.rev_level() == GOOD_OLD_REV { GOOD_OLD_INODE_SIZE } else { self.inode_size_raw() as u32 }
    }

    pub fn feature_compat(&self) -> u32 {
        if self.rev_level() == GOOD_OLD_REV { 0 } else { self.feature_compat_raw() }
    }

    pub fn feature_incompat(&self) -> u32 {
        if self.rev_level() == GOOD_OLD_REV { 0 } else { self.feature_incompat_raw() }
    }

    pub fn feature_ro_compat(&self) -> u32 {
        if self.rev_level() == GOOD_OLD_REV { 0 } else { self.feature_ro_compat_raw() }
    }

    fn wide(&self) -> bool {
        self.feature_incompat() & INCOMPAT_64BIT != 0
    }

    /// The block count, with its high half on `64bit` volumes.
    pub fn blocks_count(&self) -> u64 {
        let hi = if self.wide() { self.blocks_count_hi() as u64 } else { 0 };
        self.blocks_count_lo() as u64 | hi << 32
    }

    pub fn set_blocks_count(&mut self, v: u64) {
        self.set_blocks_count_lo(v as u32);
        if self.wide() {
            self.set_blocks_count_hi((v >> 32) as u32);
        }
    }

    pub fn r_blocks_count(&self) -> u64 {
        let hi = if self.wide() { self.r_blocks_count_hi() as u64 } else { 0 };
        self.r_blocks_count_lo() as u64 | hi << 32
    }

    pub fn set_r_blocks_count(&mut self, v: u64) {
        self.set_r_blocks_count_lo(v as u32);
        if self.wide() {
            self.set_r_blocks_count_hi((v >> 32) as u32);
        }
    }

    pub fn set_free_blocks_count(&mut self, v: u64) {
        if self.wide() {
            self.set_free_blocks_count_lo(v as u32);
            self.set_free_blocks_count_hi((v >> 32) as u32);
        } else {
            self.set_free_blocks_count_lo(v.min(u32::MAX as u64) as u32);
        }
    }

    /// Group descriptor size: 32, or `s_desc_size` with `64bit`.
    pub fn desc_size(&self) -> usize {
        if self.wide() { self.desc_size_raw() as usize } else { DESC_SIZE }
    }

    /// `s_backup_bgs` (`sparse_super2`).
    pub fn backup_bgs(&self) -> [u32; 2] {
        [le32(&self.raw[..], 0x24C), le32(&self.raw[..], 0x250)]
    }

    pub fn hash_seed(&self) -> [u8; 16] {
        let mut s = [0; 16];
        s.copy_from_slice(&self.raw[0xEC..0xFC]);
        s
    }

    pub fn uuid(&self) -> [u8; 16] {
        let mut u = [0; 16];
        u.copy_from_slice(&self.raw[104..120]);
        u
    }

    pub fn volume_name(&self) -> [u8; 16] {
        let mut n = [0; 16];
        n.copy_from_slice(&self.raw[120..136]);
        n
    }
}

/// One block group descriptor (32 bytes, or 64 and more with `64bit`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupDesc {
    pub block_bitmap: u64,
    pub inode_bitmap: u64,
    pub inode_table: u64,
    pub free_blocks: u32,
    pub free_inodes: u32,
    pub used_dirs: u32,
    pub flags: u16,
    /// Inodes at the end of the group's table never used (with group
    /// descriptor checksums).
    pub itable_unused: u32,
    pub block_bitmap_csum: u32,
    pub inode_bitmap_csum: u32,
    pub checksum: u16,
}

impl GroupDesc {
    /// Decodes a descriptor of `size` bytes (`b` is at least that long).
    pub fn decode(b: &[u8], size: usize) -> GroupDesc {
        let wide = size >= DESC_SIZE_64;
        let hi16 = |at: usize| if wide { le16(b, at) as u32 } else { 0 };
        let hi32 = |at: usize| if wide { le32(b, at) as u64 } else { 0 };
        GroupDesc {
            block_bitmap: le32(b, 0) as u64 | hi32(0x20) << 32,
            inode_bitmap: le32(b, 4) as u64 | hi32(0x24) << 32,
            inode_table: le32(b, 8) as u64 | hi32(0x28) << 32,
            free_blocks: le16(b, 12) as u32 | hi16(0x2C) << 16,
            free_inodes: le16(b, 14) as u32 | hi16(0x2E) << 16,
            used_dirs: le16(b, 16) as u32 | hi16(0x30) << 16,
            flags: le16(b, 0x12),
            itable_unused: le16(b, 0x1C) as u32 | hi16(0x32) << 16,
            block_bitmap_csum: le16(b, 0x18) as u32 | hi16(0x38) << 16,
            inode_bitmap_csum: le16(b, 0x1A) as u32 | hi16(0x3A) << 16,
            checksum: le16(b, 0x1E),
        }
    }

    /// Writes the known fields into a descriptor slot of `size` bytes.
    pub fn encode(&self, b: &mut [u8], size: usize) {
        put32(b, 0, self.block_bitmap as u32);
        put32(b, 4, self.inode_bitmap as u32);
        put32(b, 8, self.inode_table as u32);
        put16(b, 12, self.free_blocks as u16);
        put16(b, 14, self.free_inodes as u16);
        put16(b, 16, self.used_dirs as u16);
        put16(b, 0x12, self.flags);
        put16(b, 0x18, self.block_bitmap_csum as u16);
        put16(b, 0x1A, self.inode_bitmap_csum as u16);
        put16(b, 0x1C, self.itable_unused as u16);
        put16(b, 0x1E, self.checksum);
        if size >= DESC_SIZE_64 {
            put32(b, 0x20, (self.block_bitmap >> 32) as u32);
            put32(b, 0x24, (self.inode_bitmap >> 32) as u32);
            put32(b, 0x28, (self.inode_table >> 32) as u32);
            put16(b, 0x2C, (self.free_blocks >> 16) as u16);
            put16(b, 0x2E, (self.free_inodes >> 16) as u16);
            put16(b, 0x30, (self.used_dirs >> 16) as u16);
            put16(b, 0x32, (self.itable_unused >> 16) as u16);
            put16(b, 0x38, (self.block_bitmap_csum >> 16) as u16);
            put16(b, 0x3A, (self.inode_bitmap_csum >> 16) as u16);
        }
    }
}

/// Inode field offsets.
pub mod ino {
    pub const MODE: usize = 0;
    pub const UID: usize = 2;
    pub const SIZE: usize = 4;
    pub const ATIME: usize = 8;
    pub const CTIME: usize = 12;
    pub const MTIME: usize = 16;
    pub const DTIME: usize = 20;
    pub const GID: usize = 24;
    pub const LINKS: usize = 26;
    pub const BLOCKS: usize = 28;
    pub const FLAGS: usize = 32;
    pub const BLOCK: usize = 40;
    pub const GENERATION: usize = 100;
    pub const FILE_ACL: usize = 104;
    /// `i_dir_acl` in revision 0; the size's high half for regular files.
    pub const SIZE_HIGH: usize = 108;
    /// Linux's `osd2`: `i_blocks` high 16 bits, `i_file_acl` high 16 bits,
    /// uid/gid high halves, the checksum's low 16 bits.
    pub const BLOCKS_HIGH: usize = 116;
    pub const FILE_ACL_HIGH: usize = 118;
    pub const UID_HIGH: usize = 120;
    pub const GID_HIGH: usize = 122;
    pub const CHECKSUM_LO: usize = 124;
    /// Large inodes (more than 128 bytes): how many bytes past 128 are in use.
    pub const EXTRA_ISIZE: usize = 128;
    pub const CHECKSUM_HI: usize = 130;
    pub const CTIME_EXTRA: usize = 132;
    pub const MTIME_EXTRA: usize = 136;
    pub const ATIME_EXTRA: usize = 140;
    pub const CRTIME: usize = 144;
    pub const CRTIME_EXTRA: usize = 148;
}

/// What the format allows this volume's inodes to carry.
#[derive(Clone, Copy, Debug)]
pub struct InodeFormat {
    pub size: usize,
    /// `i_size_high` holds the size's high half for regular files.
    pub large_size: bool,
    /// uid/gid high halves at 120/122.
    pub id_high: bool,
    /// `huge_file`: `i_blocks` has 48 bits and may count file system blocks.
    pub huge_file: bool,
    /// `64bit`: `i_file_acl` has 48 bits.
    pub wide: bool,
    /// 512-byte units per file system block.
    pub spb: u32,
}

/// An inode, decoded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inode {
    /// Its number (not stored in the record; the checksum seed needs it).
    pub ino: u32,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: Timespec,
    pub ctime: Timespec,
    pub mtime: Timespec,
    pub dtime: u32,
    pub links: u16,
    /// 512-byte units.
    pub blocks: u64,
    pub flags: u32,
    pub block: [u32; N_BLOCKS],
    pub generation: u32,
    pub file_acl: u64,
}

/// ext4's timestamp extension: the low two bits of `*_extra` extend the
/// signed 32-bit seconds by multiples of 2^32; the upper 30 bits are
/// nanoseconds.
pub fn decode_time(base: u32, extra: Option<u32>) -> Timespec {
    let mut sec = base as i32 as i64;
    let mut nsec = 0;
    if let Some(x) = extra {
        sec += ((x & 3) as i64) << 32;
        nsec = (x >> 2).min(999_999_999);
    }
    Timespec::new(sec, nsec)
}

/// The inverse of [`decode_time`]; without room for the extension the
/// seconds are clamped to the signed 32-bit range and nanoseconds dropped.
pub fn encode_time(t: Timespec, extended: bool) -> (u32, u32) {
    if !extended {
        return (t.sec.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32, 0);
    }
    // Representable: [-2^31, 2^34 - 2^31).
    let sec = t.sec.clamp(i32::MIN as i64, (1i64 << 34) - (1i64 << 31) - 1);
    let base = sec as i32;
    let epoch = ((sec - base as i64) >> 32) as u32 & 3;
    (base as u32, epoch | (t.nsec.min(999_999_999) << 2))
}

impl Inode {
    pub fn kind(&self) -> Option<FileType> {
        FileType::from_mode(self.mode as u32)
    }

    pub fn is_dir(&self) -> bool {
        self.kind() == Some(FileType::Directory)
    }

    /// Bytes past 128 the inode uses, if it may use any.
    pub fn extra(raw: &[u8]) -> usize {
        if raw.len() <= GOOD_OLD_INODE_SIZE as usize {
            return 0;
        }
        let e = le16(raw, ino::EXTRA_ISIZE) as usize;
        if e > raw.len() - GOOD_OLD_INODE_SIZE as usize || !e.is_multiple_of(4) { 0 } else { e }
    }

    /// Whether field `off` (4 bytes) is inside the used extra space.
    fn has(raw: &[u8], off: usize) -> bool {
        off + 4 <= GOOD_OLD_INODE_SIZE as usize + Inode::extra(raw)
    }

    pub fn decode(raw: &[u8], fmt: &InodeFormat) -> Inode {
        let mode = le16(raw, ino::MODE);
        let mut block = [0u32; N_BLOCKS];
        for (i, b) in block.iter_mut().enumerate() {
            *b = le32(raw, ino::BLOCK + 4 * i);
        }
        let (uid_hi, gid_hi) =
            if fmt.id_high { (le16(raw, ino::UID_HIGH) as u32, le16(raw, ino::GID_HIGH) as u32) } else { (0, 0) };
        let regular = mode as u32 & poly_vfs::S_IFMT == poly_vfs::S_IFREG;
        let high = if regular && fmt.large_size { le32(raw, ino::SIZE_HIGH) as u64 } else { 0 };
        let time =
            |base: usize, extra: usize| decode_time(le32(raw, base), Inode::has(raw, extra).then(|| le32(raw, extra)));
        let flags = le32(raw, ino::FLAGS);
        let mut blocks = le32(raw, ino::BLOCKS) as u64;
        if fmt.huge_file {
            blocks |= (le16(raw, ino::BLOCKS_HIGH) as u64) << 32;
            if flags & FL_HUGE_FILE != 0 {
                blocks *= fmt.spb as u64;
            }
        }
        let acl_hi = if fmt.wide { le16(raw, ino::FILE_ACL_HIGH) as u64 } else { 0 };
        Inode {
            ino: 0,
            mode,
            uid: le16(raw, ino::UID) as u32 | uid_hi << 16,
            gid: le16(raw, ino::GID) as u32 | gid_hi << 16,
            size: le32(raw, ino::SIZE) as u64 | high << 32,
            atime: time(ino::ATIME, ino::ATIME_EXTRA),
            ctime: time(ino::CTIME, ino::CTIME_EXTRA),
            mtime: time(ino::MTIME, ino::MTIME_EXTRA),
            dtime: le32(raw, ino::DTIME),
            links: le16(raw, ino::LINKS),
            blocks,
            flags,
            block,
            generation: le32(raw, ino::GENERATION),
            file_acl: le32(raw, ino::FILE_ACL) as u64 | acl_hi << 32,
        }
    }

    /// Patches the fields into `raw` (an inode record as read from disk).
    pub fn encode(&self, raw: &mut [u8], fmt: &InodeFormat) {
        put16(raw, ino::MODE, self.mode);
        put16(raw, ino::UID, self.uid as u16);
        put16(raw, ino::GID, self.gid as u16);
        if fmt.id_high {
            put16(raw, ino::UID_HIGH, (self.uid >> 16) as u16);
            put16(raw, ino::GID_HIGH, (self.gid >> 16) as u16);
        }
        put32(raw, ino::SIZE, self.size as u32);
        let regular = self.mode as u32 & poly_vfs::S_IFMT == poly_vfs::S_IFREG;
        if regular && fmt.large_size {
            put32(raw, ino::SIZE_HIGH, (self.size >> 32) as u32);
        }
        for (t, base, extra) in [
            (self.atime, ino::ATIME, ino::ATIME_EXTRA),
            (self.ctime, ino::CTIME, ino::CTIME_EXTRA),
            (self.mtime, ino::MTIME, ino::MTIME_EXTRA),
        ] {
            let ext = Inode::has(raw, extra);
            let (b, x) = encode_time(t, ext);
            put32(raw, base, b);
            if ext {
                put32(raw, extra, x);
            }
        }
        put32(raw, ino::DTIME, self.dtime);
        put16(raw, ino::LINKS, self.links);
        let mut flags = self.flags & !FL_HUGE_FILE;
        if fmt.huge_file {
            let mut count = self.blocks;
            if count >= 1 << 48 {
                flags |= FL_HUGE_FILE;
                count /= fmt.spb as u64;
            }
            put32(raw, ino::BLOCKS, count as u32);
            put16(raw, ino::BLOCKS_HIGH, (count >> 32) as u16);
        } else {
            put32(raw, ino::BLOCKS, self.blocks as u32);
        }
        put32(raw, ino::FLAGS, flags);
        for (i, b) in self.block.iter().enumerate() {
            put32(raw, ino::BLOCK + 4 * i, *b);
        }
        put32(raw, ino::GENERATION, self.generation);
        put32(raw, ino::FILE_ACL, self.file_acl as u32);
        if fmt.wide {
            put16(raw, ino::FILE_ACL_HIGH, (self.file_acl >> 32) as u16);
        }
    }

    /// Prepares a zeroed record for a new inode: large inodes get the extra
    /// fields (timestamps with nanoseconds, creation time, the checksum's
    /// high half) in use.
    pub fn init_record(raw: &mut [u8], created: Timespec) {
        raw.fill(0);
        if raw.len() >= GOOD_OLD_INODE_SIZE as usize + 32 {
            put16(raw, ino::EXTRA_ISIZE, 32);
            let (b, x) = encode_time(created, true);
            put32(raw, ino::CRTIME, b);
            put32(raw, ino::CRTIME_EXTRA, x);
        }
    }

    /// The creation time (`i_crtime`), if the record has room for it.
    pub fn crtime(raw: &[u8]) -> Option<Timespec> {
        Inode::has(raw, ino::CRTIME_EXTRA)
            .then(|| decode_time(le32(raw, ino::CRTIME), Some(le32(raw, ino::CRTIME_EXTRA))))
    }

    /// The `i_block` bytes, for fast symbolic links and extent roots.
    pub fn block_bytes(&self) -> [u8; 4 * N_BLOCKS] {
        let mut out = [0u8; 4 * N_BLOCKS];
        for (i, b) in self.block.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&b.to_le_bytes());
        }
        out
    }

    pub fn set_block_bytes(&mut self, bytes: &[u8]) {
        let mut all = [0u8; 4 * N_BLOCKS];
        all[..bytes.len()].copy_from_slice(bytes);
        for (i, b) in self.block.iter_mut().enumerate() {
            *b = le32(&all, 4 * i);
        }
    }
}

/// Directory entry file type codes (used with `INCOMPAT_FILETYPE`).
pub fn dirent_type(kind: FileType) -> u8 {
    match kind {
        FileType::Regular => 1,
        FileType::Directory => 2,
        FileType::CharDevice => 3,
        FileType::BlockDevice => 4,
        FileType::Fifo => 5,
        FileType::Socket => 6,
        FileType::Symlink => 7,
    }
}

pub fn dirent_kind(t: u8) -> Option<FileType> {
    Some(match t {
        1 => FileType::Regular,
        2 => FileType::Directory,
        3 => FileType::CharDevice,
        4 => FileType::BlockDevice,
        5 => FileType::Fifo,
        6 => FileType::Socket,
        7 => FileType::Symlink,
        _ => return None,
    })
}

/// The space an entry with a name of `name_len` bytes needs.
pub const fn rec_len_for(name_len: usize) -> usize {
    (DIRENT_HEADER + name_len + 3) & !3
}

/// One directory entry as found in a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEntry {
    pub inode: u32,
    pub rec_len: usize,
    pub name_len: usize,
    /// The type code, 0 when the volume does not record types.
    pub file_type: u8,
}

impl RawEntry {
    /// Decodes the entry at `off` in a directory block, checking that it is
    /// well formed: inside the block, `rec_len` a multiple of 4 holding at
    /// least the header and the name. `None` if it is not.
    pub fn decode(block: &[u8], off: usize, filetype: bool) -> Option<RawEntry> {
        if !off.is_multiple_of(4) || off + DIRENT_HEADER > block.len() {
            return None;
        }
        let inode = le32(block, off);
        let rec_len = le16(block, off + 4) as usize;
        let (name_len, file_type) =
            if filetype { (block[off + 6] as usize, block[off + 7]) } else { (le16(block, off + 6) as usize, 0) };
        if rec_len < DIRENT_HEADER
            || !rec_len.is_multiple_of(4)
            || off + rec_len > block.len()
            || name_len > 255
            || DIRENT_HEADER + name_len > rec_len
        {
            return None;
        }
        Some(RawEntry { inode, rec_len, name_len, file_type })
    }

    pub fn name<'b>(&self, block: &'b [u8], off: usize) -> &'b [u8] {
        &block[off + DIRENT_HEADER..off + DIRENT_HEADER + self.name_len]
    }

    /// Bytes the entry needs (0 for an unused one).
    pub fn used(&self) -> usize {
        if self.inode == 0 { 0 } else { rec_len_for(self.name_len) }
    }

    /// Writes an entry at `off`.
    pub fn encode(
        block: &mut [u8],
        off: usize,
        inode: u32,
        rec_len: usize,
        name: &[u8],
        file_type: u8,
        filetype: bool,
    ) {
        put32(block, off, inode);
        put16(block, off + 4, rec_len as u16);
        if filetype {
            block[off + 6] = name.len() as u8;
            block[off + 7] = file_type;
        } else {
            put16(block, off + 6, name.len() as u16);
        }
        block[off + DIRENT_HEADER..off + DIRENT_HEADER + name.len()].copy_from_slice(name);
    }
}

/// Whether a directory block ends with a checksum tail.
pub fn has_dir_tail(block: &[u8]) -> bool {
    let t = block.len() - DIR_TAIL;
    le32(block, t) == 0 && le16(block, t + 4) == DIR_TAIL as u16 && block[t + 6] == 0 && block[t + 7] == DIR_TAIL_TYPE
}

/// Writes an (unchecksummed) tail at the end of a directory block.
pub fn put_dir_tail(block: &mut [u8]) {
    let t = block.len() - DIR_TAIL;
    block[t..].fill(0);
    put16(block, t + 4, DIR_TAIL as u16);
    block[t + 7] = DIR_TAIL_TYPE;
}

/// Whether group `g` holds a copy of the superblock and descriptors: all of
/// them without `sparse_super`, else 0, 1 and powers of 3, 5 and 7.
pub fn has_super(g: u32, sparse: bool) -> bool {
    if !sparse || g <= 1 {
        return true;
    }
    for base in [3u32, 5, 7] {
        let mut p = base;
        while p < g {
            match p.checked_mul(base) {
                Some(n) => p = n,
                None => break,
            }
        }
        if p == g {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_groups() {
        let with: alloc::vec::Vec<u32> = (0..60).filter(|&g| has_super(g, true)).collect();
        assert_eq!(with, [0, 1, 3, 5, 7, 9, 25, 27, 49]);
        assert!((0..60).all(|g| has_super(g, false)));
        assert!(has_super(3u32.pow(20), true));
        assert!(!has_super(u32::MAX, true));
    }

    #[test]
    fn times_round_trip() {
        for t in [
            Timespec::new(0, 0),
            Timespec::new(1_700_000_000, 123_456_789),
            Timespec::new(-1, 5),
            Timespec::new(i32::MIN as i64, 0),
            Timespec::new(i32::MAX as i64 + 1, 1),
            Timespec::new(4_000_000_000, 999_999_999),
            Timespec::new((1 << 34) - (1 << 31) - 1, 0),
        ] {
            let (b, x) = encode_time(t, true);
            assert_eq!(decode_time(b, Some(x)), t, "{t:?}");
        }
        // Without the extension: clamped, no nanoseconds.
        let (b, _) = encode_time(Timespec::new(1 << 40, 7), false);
        assert_eq!(decode_time(b, None), Timespec::new(i32::MAX as i64, 0));
        let (b, _) = encode_time(Timespec::new(-5, 7), false);
        assert_eq!(decode_time(b, None), Timespec::new(-5, 0));
    }

    #[test]
    fn inode_round_trip_keeps_unknown_bytes() {
        let fmt = InodeFormat { size: 256, large_size: true, id_high: true, huge_file: true, wide: true, spb: 8 };
        let mut raw = [0xa5u8; 256];
        Inode::init_record(&mut raw, Timespec::new(5, 6));
        raw[112] = 0x77; // a field the driver does not know
        let mut i = Inode {
            mode: 0o100644,
            uid: 100_000,
            gid: 70_000,
            size: 5 << 32 | 17,
            atime: Timespec::new(1, 2),
            ctime: Timespec::new(3, 4),
            mtime: Timespec::new(5_000_000_000, 6),
            links: 3,
            blocks: 8 << 32 | 8,
            flags: FL_APPEND,
            generation: 9,
            file_acl: 10 << 32 | 10,
            ..Inode::default()
        };
        i.set_block_bytes(b"0123456789abcdefghijklmnopqrstuvwxyz");
        i.encode(&mut raw, &fmt);
        assert_eq!(Inode::decode(&raw, &fmt), i);
        assert_eq!(raw[112], 0x77);
        assert_eq!(Inode::crtime(&raw), Some(Timespec::new(5, 6)));
        assert_eq!(&i.block_bytes()[..36], b"0123456789abcdefghijklmnopqrstuvwxyz");
        // Past 48 bits of sectors, i_blocks counts file system blocks.
        i.blocks = (1 << 50) + 64;
        i.encode(&mut raw, &fmt);
        assert_ne!(le32(&raw, ino::FLAGS) & FL_HUGE_FILE, 0);
        assert_eq!(Inode::decode(&raw, &fmt).blocks, (1 << 50) + 64);
        // A small-inode volume without high ids: 16 bits only, size_high is
        // left alone for non-regular files.
        let small = InodeFormat { size: 128, large_size: false, id_high: false, huge_file: false, wide: false, spb: 2 };
        let mut raw = [0u8; 128];
        i.blocks = 8;
        i.encode(&mut raw, &small);
        let back = Inode::decode(&raw, &small);
        assert_eq!((back.uid, back.gid, back.size, back.file_acl), (100_000 & 0xffff, 70_000 & 0xffff, 17, 10));
        assert_eq!(back.mtime, Timespec::new(i32::MAX as i64, 0));
        assert_eq!(Inode::crtime(&raw), None);
    }

    #[test]
    fn group_descriptors() {
        let d = GroupDesc {
            block_bitmap: 1 << 33 | 5,
            inode_bitmap: 6,
            inode_table: 7,
            free_blocks: 70_000,
            free_inodes: 3,
            used_dirs: 2,
            flags: BG_INODE_ZEROED,
            itable_unused: 80_000,
            block_bitmap_csum: 0xdead_beef,
            inode_bitmap_csum: 0x1234_5678,
            checksum: 0x9abc,
        };
        let mut b = [0u8; 64];
        d.encode(&mut b, 64);
        assert_eq!(GroupDesc::decode(&b, 64), d);
        // 32 bytes keep the low halves only.
        let mut b = [0u8; 32];
        d.encode(&mut b, 32);
        let n = GroupDesc::decode(&b, 32);
        assert_eq!((n.block_bitmap, n.free_blocks, n.block_bitmap_csum), (5, 70_000 & 0xffff, 0xbeef));
    }

    #[test]
    fn directory_entries() {
        let mut block = [0u8; 1024];
        RawEntry::encode(&mut block, 0, 2, 12, b".", 2, true);
        RawEntry::encode(&mut block, 12, 2, 1012, b"..", 2, true);
        let e = RawEntry::decode(&block, 12, true).unwrap();
        assert_eq!((e.inode, e.rec_len, e.name_len, e.file_type), (2, 1012, 2, 2));
        assert_eq!(e.name(&block, 12), b"..");
        assert_eq!(e.used(), 12);
        assert!(!has_dir_tail(&block));
        put_dir_tail(&mut block);
        assert!(has_dir_tail(&block));
        // Bad records: past the block, unaligned, name longer than record.
        put16(&mut block, 12, 0);
        put16(&mut block, 16, 1016);
        assert!(RawEntry::decode(&block, 12, true).is_none());
        put16(&mut block, 16, 1010);
        assert!(RawEntry::decode(&block, 12, true).is_none());
        put16(&mut block, 16, 8);
        block[18] = 1;
        assert!(RawEntry::decode(&block, 12, true).is_none());
        assert!(RawEntry::decode(&block, 1020, true).is_none());
        assert!(RawEntry::decode(&block, 2, true).is_none());
        // Without file types the name length is 16 bits.
        RawEntry::encode(&mut block, 0, 11, 1024, b"lost+found", 0, false);
        let e = RawEntry::decode(&block, 0, false).unwrap();
        assert_eq!((e.name_len, e.file_type), (10, 0));
        assert_eq!(rec_len_for(10), 20);
    }
}
