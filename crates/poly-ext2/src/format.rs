//! Making a new ext2 volume (what `mke2fs` does): the superblock and its
//! backups, the group descriptor tables, bitmaps and zeroed inode tables,
//! a root directory and `lost+found`.

use crate::BlockIo;
use crate::io::Disk;
use crate::layout::*;
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Errno, FileType, Result, Timespec};

/// How to lay out a new volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// 1024, 2048 or 4096.
    pub block_size: u32,
    /// Revision 1 ("dynamic"); revision 0 has 128-byte inodes and no
    /// feature flags.
    pub dynamic: bool,
    /// Bytes per inode record: a power of two from 128 to the block size
    /// (revision 1 only).
    pub inode_size: u32,
    /// One inode per this many bytes of the volume.
    pub bytes_per_inode: u32,
    /// Blocks per group (a multiple of 8, at most 8 times the block size);
    /// `None` for the largest.
    pub blocks_per_group: Option<u32>,
    /// Record file types in directory entries (`filetype`).
    pub filetype: bool,
    /// Superblock backups only in groups 0, 1 and powers of 3, 5, 7.
    pub sparse_super: bool,
    /// Files may be 2 GiB or larger (`large_file`; set on demand anyway).
    pub large_file: bool,
    pub uuid: [u8; 16],
    pub label: [u8; 16],
    /// Creation time, for the root directory and the superblock.
    pub time: Timespec,
}

impl Default for FormatOptions {
    fn default() -> FormatOptions {
        FormatOptions {
            block_size: 4096,
            dynamic: true,
            inode_size: 256,
            bytes_per_inode: 8192,
            blocks_per_group: None,
            filetype: true,
            sparse_super: true,
            large_file: true,
            uuid: [0; 16],
            label: [0; 16],
            time: Timespec::default(),
        }
    }
}

const LOST_AND_FOUND: u32 = 11;

/// Writes a new, empty ext2 volume over the whole device.
pub fn format<D: BlockIo>(dev: &D, opts: &FormatOptions) -> Result<()> {
    let bs = opts.block_size as usize;
    if !matches!(bs, 1024 | 2048 | 4096) {
        return Err(Errno::EINVAL);
    }
    let isz = if opts.dynamic { opts.inode_size as usize } else { GOOD_OLD_INODE_SIZE as usize };
    if !isz.is_power_of_two() || isz < GOOD_OLD_INODE_SIZE as usize || isz > bs || opts.bytes_per_inode < 1024 {
        return Err(Errno::EINVAL);
    }
    if dev.read_only() {
        return Err(Errno::EROFS);
    }
    let dbs = dev.block_size();
    if !dbs.is_power_of_two() || dbs < 512 {
        return Err(Errno::EINVAL);
    }
    let disk = Disk::new(dev, bs);
    let first_data = (bs == 1024) as u64;
    let bpg = opts.blocks_per_group.unwrap_or(8 * bs as u32) as u64;
    if bpg < 256 || bpg > 8 * bs as u64 || !bpg.is_multiple_of(8) {
        return Err(Errno::EINVAL);
    }
    let mut blocks = disk.capacity().min(u32::MAX as u64);
    if blocks < first_data + 64 {
        return Err(Errno::ENOSPC);
    }
    let (sparse, filetype, large_file) =
        if opts.dynamic { (opts.sparse_super, opts.filetype, opts.large_file) } else { (false, false, false) };
    let mut groups = (blocks - first_data).div_ceil(bpg);
    let per_block = (bs / isz) as u64;
    let wanted = (blocks * bs as u64 / opts.bytes_per_inode as u64).max(32);
    let mut ipg = wanted.div_ceil(groups).max(16);
    ipg = ipg.div_ceil(per_block.max(8)) * per_block.max(8);
    ipg = ipg.min(8 * bs as u64);
    let itb = ipg / per_block;
    let gdt_blocks = |groups: u64| (groups * DESC_SIZE as u64).div_ceil(bs as u64);
    let overhead = |g: u64, groups: u64| {
        let sb = if has_super(g as u32, sparse) { 1 + gdt_blocks(groups) } else { 0 };
        sb + 2 + itb
    };
    // A last group too small for its own metadata and some data goes.
    let last = groups - 1;
    let last_size = blocks - (first_data + last * bpg);
    if last_size < overhead(last, groups) + 16 {
        if groups == 1 {
            return Err(Errno::ENOSPC);
        }
        groups -= 1;
        blocks = first_data + groups * bpg;
    }
    let gdtb = gdt_blocks(groups);
    if overhead(0, groups) + 2 > bpg.min(blocks - first_data) || ipg * groups > u32::MAX as u64 {
        return Err(Errno::ENOSPC);
    }
    let inodes = ipg * groups;
    let start = |g: u64| first_data + g * bpg;
    let size_of = |g: u64| (blocks - start(g)).min(bpg);
    let meta = |g: u64| start(g) + if has_super(g as u32, sparse) { 1 + gdtb } else { 0 };
    let data_start = |g: u64| meta(g) + 2 + itb;
    let root_block = data_start(0);
    let lf_block = root_block + 1;

    let mut descs = Vec::with_capacity(groups as usize);
    let mut free_total = 0u64;
    for g in 0..groups {
        let used = data_start(g) - start(g) + if g == 0 { 2 } else { 0 };
        let mut bb = vec![0u8; bs];
        for i in (0..used as usize).chain(size_of(g) as usize..8 * bs) {
            bb[i / 8] |= 1 << (i % 8);
        }
        disk.write(meta(g), &bb)?;
        let mut ib = vec![0u8; bs];
        let used_inodes = if g == 0 { LOST_AND_FOUND as usize } else { 0 };
        for i in (0..used_inodes).chain(ipg as usize..8 * bs) {
            ib[i / 8] |= 1 << (i % 8);
        }
        disk.write(meta(g) + 1, &ib)?;
        let zeros = vec![0u8; bs * 16];
        let mut b = meta(g) + 2;
        while b < data_start(g) {
            let n = (data_start(g) - b).min(16);
            disk.write(b, &zeros[..n as usize * bs])?;
            b += n;
        }
        let free = size_of(g) - used;
        free_total += free;
        descs.push(GroupDesc {
            block_bitmap: meta(g) as u32,
            inode_bitmap: (meta(g) + 1) as u32,
            inode_table: (meta(g) + 2) as u32,
            free_blocks: free as u16,
            free_inodes: (ipg as usize - used_inodes) as u16,
            used_dirs: if g == 0 { 2 } else { 0 },
        });
    }

    // Root and lost+found.
    let ifmt = InodeFormat { size: isz, large_size: opts.dynamic, id_high: true };
    let spb = (bs / 512) as u32;
    let dir = |links: u16, perm: u16, block: u64| Inode {
        mode: FileType::Directory.mode_bits() as u16 | perm,
        size: bs as u64,
        atime: opts.time,
        ctime: opts.time,
        mtime: opts.time,
        links,
        blocks: spb,
        block: {
            let mut b = [0u32; N_BLOCKS];
            b[0] = block as u32;
            b
        },
        ..Inode::default()
    };
    let ft = dirent_type(FileType::Directory);
    let mut rb = vec![0u8; bs];
    RawEntry::encode(&mut rb, 0, ROOT_INO, 12, b".", ft, filetype);
    RawEntry::encode(&mut rb, 12, ROOT_INO, 12, b"..", ft, filetype);
    RawEntry::encode(&mut rb, 24, LOST_AND_FOUND, bs - 24, b"lost+found", ft, filetype);
    disk.write(root_block, &rb)?;
    let mut lb = vec![0u8; bs];
    RawEntry::encode(&mut lb, 0, LOST_AND_FOUND, 12, b".", ft, filetype);
    RawEntry::encode(&mut lb, 12, ROOT_INO, bs - 12, b"..", ft, filetype);
    disk.write(lf_block, &lb)?;
    for (ino, inode) in [(ROOT_INO, dir(3, 0o755, root_block)), (LOST_AND_FOUND, dir(2, 0o700, lf_block))] {
        let at = descs[0].inode_table as u64 * bs as u64 + (ino as u64 - 1) * isz as u64;
        let mut raw = vec![0u8; isz];
        Inode::init_record(&mut raw, opts.time);
        inode.encode(&mut raw, &ifmt);
        disk.write_bytes(at, &raw)?;
    }

    // Superblock and descriptors, with their backups.
    let mut sb = Superblock::new([0; SUPERBLOCK_SIZE]);
    sb.set_inodes_count(inodes as u32);
    sb.set_blocks_count(blocks as u32);
    sb.set_free_blocks_count(free_total as u32);
    sb.set_free_inodes_count((inodes - LOST_AND_FOUND as u64) as u32);
    sb.set_first_data_block(first_data as u32);
    let log = (bs / 1024).trailing_zeros();
    sb.set_log_block_size(log);
    sb.set_log_frag_size(log);
    sb.set_blocks_per_group(bpg as u32);
    sb.set_frags_per_group(bpg as u32);
    sb.set_inodes_per_group(ipg as u32);
    sb.set_wtime(encode_time(opts.time, false).0);
    sb.set_lastcheck(encode_time(opts.time, false).0);
    sb.set_max_mnt_count(u16::MAX);
    sb.set_magic(MAGIC);
    sb.set_state(STATE_VALID);
    sb.set_errors(1);
    sb.set_creator_os(OS_LINUX);
    if opts.dynamic {
        sb.set_rev_level(DYNAMIC_REV);
        sb.set_first_ino(LOST_AND_FOUND);
        sb.set_inode_size(isz as u16);
        sb.set_feature_incompat(if filetype { INCOMPAT_FILETYPE } else { 0 });
        let ro = if sparse { RO_COMPAT_SPARSE_SUPER } else { 0 } | if large_file { RO_COMPAT_LARGE_FILE } else { 0 };
        sb.set_feature_ro_compat(ro);
        sb.raw[104..120].copy_from_slice(&opts.uuid);
        sb.raw[120..136].copy_from_slice(&opts.label);
        if isz >= GOOD_OLD_INODE_SIZE as usize + 32 {
            sb.set_min_extra_isize(32);
            sb.set_want_extra_isize(32);
        }
    }
    let mut gdt = vec![0u8; gdtb as usize * bs];
    for (i, d) in descs.iter().enumerate() {
        d.encode(&mut gdt[i * DESC_SIZE..(i + 1) * DESC_SIZE]);
    }
    for g in 0..groups {
        if !has_super(g as u32, sparse) {
            continue;
        }
        sb.set_block_group_nr(g as u16);
        let at = if g == 0 { SUPERBLOCK_OFFSET } else { start(g) * bs as u64 };
        disk.write_bytes(at, &sb.raw)?;
        disk.write(start(g) + 1, &gdt)?;
    }
    disk.flush()
}
