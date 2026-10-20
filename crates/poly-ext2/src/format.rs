//! Making a new volume (what `mke2fs` does): the superblock and its
//! backups, the group descriptor tables, bitmaps and zeroed inode tables,
//! a root directory and `lost+found`; for ext4 also checksums everywhere,
//! extent trees and a journal.
//!
//! The ext4 flavour has the features `mke2fs -t ext4` turns on by default
//! except `flex_bg` (each group keeps its own bitmaps and inode table),
//! `resize_inode` and `dir_index`: `64bit`, `extent`, `metadata_csum`,
//! `huge_file`, `dir_nlink`, `extra_isize`, and `has_journal` with a
//! checksum v3 journal when asked for one.

use crate::BlockIo;
use crate::crc::crc32c;
use crate::csum;
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
    /// ext4: `64bit`, `extent`, `metadata_csum`, `huge_file`, `dir_nlink`,
    /// `extra_isize` (revision 1, `filetype` and `sparse_super` implied).
    pub ext4: bool,
    /// A journal of this many blocks (at least 1024), or none (0).
    pub journal_blocks: u32,
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
            ext4: false,
            journal_blocks: 0,
            uuid: [0; 16],
            label: [0; 16],
            time: Timespec::default(),
        }
    }
}

impl FormatOptions {
    /// ext4 as `mke2fs -t ext4` makes it (see the module documentation),
    /// with a journal of `journal_blocks`.
    pub fn ext4(block_size: u32, journal_blocks: u32) -> FormatOptions {
        FormatOptions { block_size, ext4: true, journal_blocks, ..FormatOptions::default() }
    }
}

/// Kept for symmetry with [`FormatOptions::journal_blocks`]: how big a
/// journal `format` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalOptions {
    pub blocks: u32,
}

const LOST_AND_FOUND: u32 = 11;

/// An inode's `i_block` as one extent (root, depth 0), or as a direct
/// pointer.
fn map_one(ext4: bool, start: u64, len: u64) -> [u32; N_BLOCKS] {
    let mut b = [0u32; N_BLOCKS];
    if ext4 {
        let mut raw = [0u8; 60];
        put16(&mut raw, 0, crate::extent::MAGIC);
        put16(&mut raw, 4, crate::extent::ROOT_MAX as u16);
        let mut at = 12;
        let (mut s, mut l, mut n) = (start, len, 0u32);
        while l > 0 {
            let part = l.min(crate::extent::INIT_MAX as u64);
            put32(&mut raw, at, n);
            put16(&mut raw, at + 4, part as u16);
            put16(&mut raw, at + 6, (s >> 32) as u16);
            put32(&mut raw, at + 8, s as u32);
            at += 12;
            s += part;
            l -= part;
            n += part as u32;
        }
        put16(&mut raw, 2, ((at - 12) / 12) as u16);
        for (i, w) in b.iter_mut().enumerate() {
            *w = le32(&raw, 4 * i);
        }
    } else {
        b[0] = start as u32;
    }
    b
}

/// Writes a new, empty volume over the whole device.
pub fn format<D: BlockIo>(dev: &D, opts: &FormatOptions) -> Result<()> {
    let bs = opts.block_size as usize;
    if !matches!(bs, 1024 | 2048 | 4096) {
        return Err(Errno::EINVAL);
    }
    let ext4 = opts.ext4;
    let dynamic = opts.dynamic || ext4;
    let isz = if dynamic { opts.inode_size as usize } else { GOOD_OLD_INODE_SIZE as usize };
    if !isz.is_power_of_two() || isz < GOOD_OLD_INODE_SIZE as usize || isz > bs || opts.bytes_per_inode < 1024 {
        return Err(Errno::EINVAL);
    }
    if opts.journal_blocks != 0 && (opts.journal_blocks < 1024 || !dynamic) {
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
    let (sparse, filetype, large_file) = if ext4 {
        (true, true, true)
    } else if dynamic {
        (opts.sparse_super, opts.filetype, opts.large_file)
    } else {
        (false, false, false)
    };
    let ds = if ext4 { DESC_SIZE_64 } else { DESC_SIZE };
    let mut groups = (blocks - first_data).div_ceil(bpg);
    let per_block = (bs / isz) as u64;
    let wanted = (blocks * bs as u64 / opts.bytes_per_inode as u64).max(32);
    let mut ipg = wanted.div_ceil(groups).max(16);
    ipg = ipg.div_ceil(per_block.max(8)) * per_block.max(8);
    ipg = ipg.min(8 * bs as u64);
    let itb = ipg / per_block;
    let gdt_blocks = |groups: u64| (groups * ds as u64).div_ceil(bs as u64);
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
    // The journal: the first group with room for it after its metadata
    // (and, in group 0, the two directory blocks).
    let jlen = opts.journal_blocks as u64;
    let journal = if jlen == 0 {
        None
    } else {
        let at = (0..groups).find_map(|g| {
            let from = data_start(g) + if g == 0 { 2 } else { 0 };
            (from + jlen <= start(g) + size_of(g)).then_some(from)
        });
        Some(at.ok_or(Errno::ENOSPC)?)
    };
    let seed = ext4.then(|| crc32c(!0, &opts.uuid));

    let mut descs = Vec::with_capacity(groups as usize);
    let mut free_total = 0u64;
    for g in 0..groups {
        let mut used = data_start(g) - start(g) + if g == 0 { 2 } else { 0 };
        let mut bb = vec![0u8; bs];
        let mut mark = |i: u64| bb[(i / 8) as usize] |= 1 << (i % 8);
        for i in (0..used).chain(size_of(g)..8 * bs as u64) {
            mark(i);
        }
        if let Some(j) = journal.filter(|&j| j >= start(g) && j < start(g) + size_of(g)) {
            for b in j..j + jlen {
                mark(b - start(g));
            }
            used += jlen;
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
        let mut d = GroupDesc {
            block_bitmap: meta(g),
            inode_bitmap: meta(g) + 1,
            inode_table: meta(g) + 2,
            free_blocks: free as u32,
            free_inodes: (ipg as usize - used_inodes) as u32,
            used_dirs: if g == 0 { 2 } else { 0 },
            ..GroupDesc::default()
        };
        if let Some(seed) = seed {
            d.flags = BG_INODE_ZEROED;
            d.itable_unused = ipg as u32 - used_inodes as u32;
            d.block_bitmap_csum = csum::bitmap(seed, &bb, bpg);
            d.inode_bitmap_csum = csum::bitmap(seed, &ib, ipg);
        }
        descs.push(d);
    }

    // Root and lost+found (and the journal's inode).
    let ifmt = InodeFormat {
        size: isz,
        large_size: dynamic,
        id_high: true,
        huge_file: ext4,
        wide: ext4,
        spb: (bs / 512) as u32,
    };
    let spb = (bs / 512) as u64;
    let ext_flag = if ext4 { FL_EXTENTS } else { 0 };
    let dir = |ino: u32, links: u16, perm: u16, block: u64| Inode {
        ino,
        mode: FileType::Directory.mode_bits() as u16 | perm,
        size: bs as u64,
        atime: opts.time,
        ctime: opts.time,
        mtime: opts.time,
        links,
        blocks: spb,
        flags: ext_flag,
        block: map_one(ext4, block, 1),
        ..Inode::default()
    };
    let ft = dirent_type(FileType::Directory);
    let tail = if ext4 { DIR_TAIL } else { 0 };
    let mut rb = vec![0u8; bs];
    RawEntry::encode(&mut rb, 0, ROOT_INO, 12, b".", ft, filetype);
    RawEntry::encode(&mut rb, 12, ROOT_INO, 12, b"..", ft, filetype);
    RawEntry::encode(&mut rb, 24, LOST_AND_FOUND, bs - 24 - tail, b"lost+found", ft, filetype);
    let mut lb = vec![0u8; bs];
    RawEntry::encode(&mut lb, 0, LOST_AND_FOUND, 12, b".", ft, filetype);
    RawEntry::encode(&mut lb, 12, ROOT_INO, bs - 12 - tail, b"..", ft, filetype);
    let mut inodes_out = vec![
        (ROOT_INO, dir(ROOT_INO, 3, 0o755, root_block)),
        (LOST_AND_FOUND, dir(LOST_AND_FOUND, 2, 0o700, lf_block)),
    ];
    for (block, data, ino) in [(root_block, &mut rb, ROOT_INO), (lf_block, &mut lb, LOST_AND_FOUND)] {
        if let Some(seed) = seed {
            put_dir_tail(data);
            let c = csum::dir_block(csum::inode_seed(seed, ino, 0), data);
            put32(data, bs - 4, c);
        }
        disk.write(block, data)?;
    }
    if let Some(j) = journal {
        let ji = Inode {
            ino: JOURNAL_INO,
            mode: FileType::Regular.mode_bits() as u16 | 0o600,
            size: jlen * bs as u64,
            atime: opts.time,
            ctime: opts.time,
            mtime: opts.time,
            links: 1,
            blocks: jlen * spb,
            flags: ext_flag,
            block: map_one(ext4, j, jlen),
            ..Inode::default()
        };
        if !ext4 && jlen > NDIR_BLOCKS as u64 {
            // Block-mapped journals are not made here.
            return Err(Errno::EINVAL);
        }
        inodes_out.push((JOURNAL_INO, ji));
        // The journal superblock (v2) and zeroed log blocks.
        let zeros = vec![0u8; bs * 16];
        let mut b = j;
        while b < j + jlen {
            let n = (j + jlen - b).min(16);
            disk.write(b, &zeros[..n as usize * bs])?;
            b += n;
        }
        let mut sb = vec![0u8; bs];
        let be = |sb: &mut Vec<u8>, at: usize, v: u32| sb[at..at + 4].copy_from_slice(&v.to_be_bytes());
        be(&mut sb, 0, crate::journal::MAGIC);
        be(&mut sb, 4, 4);
        be(&mut sb, 0x0C, bs as u32);
        be(&mut sb, 0x10, jlen as u32);
        be(&mut sb, 0x14, 1);
        be(&mut sb, 0x18, 1);
        let incompat = if ext4 { 0x1 | 0x2 | 0x10 } else { 0x1 };
        be(&mut sb, 0x28, incompat);
        sb[0x30..0x40].copy_from_slice(&opts.uuid);
        be(&mut sb, 0x40, 1);
        if ext4 {
            sb[0x50] = 4;
        }
        sb[0x100..0x110].copy_from_slice(&opts.uuid);
        if ext4 {
            let c = crc32c(!0, &sb[..1024]);
            be(&mut sb, 0xFC, c);
        }
        disk.write(j, &sb)?;
    }
    let mut jnl_blocks = None;
    for (ino, inode) in &inodes_out {
        let at = descs[0].inode_table * bs as u64 + (*ino as u64 - 1) * isz as u64;
        let mut raw = vec![0u8; isz];
        Inode::init_record(&mut raw, opts.time);
        inode.encode(&mut raw, &ifmt);
        if let Some(seed) = seed {
            csum::set_inode(seed, *ino, &mut raw);
        }
        disk.write_bytes(at, &raw)?;
        if *ino == JOURNAL_INO {
            jnl_blocks = Some((inode.block, inode.size));
        }
    }

    // Superblock and descriptors, with their backups.
    let mut sb = Superblock::from_bytes(&[]);
    sb.set_inodes_count(inodes as u32);
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
    if dynamic {
        sb.set_rev_level(DYNAMIC_REV);
        sb.set_first_ino(LOST_AND_FOUND);
        sb.set_inode_size(isz as u16);
        let mut incompat = if filetype { INCOMPAT_FILETYPE } else { 0 };
        let mut ro =
            if sparse { RO_COMPAT_SPARSE_SUPER } else { 0 } | if large_file { RO_COMPAT_LARGE_FILE } else { 0 };
        let mut compat = 0;
        if ext4 {
            incompat |= INCOMPAT_EXTENTS | INCOMPAT_64BIT;
            ro |= RO_COMPAT_HUGE_FILE | RO_COMPAT_DIR_NLINK | RO_COMPAT_EXTRA_ISIZE | RO_COMPAT_METADATA_CSUM;
            sb.set_desc_size(ds as u16);
            sb.set_checksum_type(1);
            sb.set_mkfs_time(encode_time(opts.time, false).0);
        }
        if let Some((block, size)) = jnl_blocks {
            compat |= COMPAT_HAS_JOURNAL;
            sb.set_journal_inum(JOURNAL_INO);
            // The journal inode's block map, backed up in s_jnl_blocks.
            sb.set_jnl_backup_type(1);
            for (i, w) in block.iter().enumerate() {
                put32(&mut sb.raw[..], 0x10C + 4 * i, *w);
            }
            put32(&mut sb.raw[..], 0x10C + 60, (size >> 32) as u32);
            put32(&mut sb.raw[..], 0x10C + 64, size as u32);
        }
        sb.set_feature_compat(compat);
        sb.set_feature_incompat(incompat);
        sb.set_feature_ro_compat(ro);
        sb.raw[104..120].copy_from_slice(&opts.uuid);
        sb.raw[120..136].copy_from_slice(&opts.label);
        if isz >= GOOD_OLD_INODE_SIZE as usize + 32 {
            sb.set_min_extra_isize(32);
            sb.set_want_extra_isize(32);
        }
    }
    sb.set_blocks_count(blocks);
    sb.set_free_blocks_count(free_total);
    sb.set_free_inodes_count((inodes - LOST_AND_FOUND as u64) as u32);
    let mut gdt = vec![0u8; gdtb as usize * bs];
    for (i, d) in descs.iter().enumerate() {
        let raw = &mut gdt[i * ds..(i + 1) * ds];
        d.encode(raw, ds);
        if let Some(seed) = seed {
            let c = crc32c(seed, &(i as u32).to_le_bytes());
            let c = crc32c(c, &raw[..0x1E]);
            let c = crc32c(c, &[0, 0]);
            let c = crc32c(c, &raw[0x20..ds]);
            put16(raw, 0x1E, c as u16);
        }
    }
    for g in 0..groups {
        if !has_super(g as u32, sparse) {
            continue;
        }
        sb.set_block_group_nr(g as u16);
        if ext4 {
            let c = csum::sb(&sb.raw[..]);
            sb.set_checksum(c);
        }
        let at = if g == 0 { SUPERBLOCK_OFFSET } else { start(g) * bs as u64 };
        disk.write_bytes(at, &sb.raw[..])?;
        disk.write(start(g) + 1, &gdt)?;
    }
    disk.flush()
}
