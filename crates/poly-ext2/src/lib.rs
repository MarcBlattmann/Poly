//! The ext2, ext3 and ext4 file systems.
//!
//! [`Ext2`] mounts an ext2/3/4 volume from any [`BlockIo`] and implements
//! [`poly_vfs::FileSystem`] on it, so the VFS can put it anywhere in the
//! tree. [`format`] makes a new, empty volume (ext2, or ext4 with a
//! journal). ADR 0032 describes the ext2 core, ADR 0052 the ext4 parts.
//!
//! - **Reading**: revisions 0 and 1, 1/2/4 KiB blocks, inodes of any size
//!   (nanosecond timestamps and creation times in large inodes), 32-bit
//!   uids and gids, 64-bit file sizes, block maps (direct, single, double
//!   and triple indirect) and extent trees of any depth with holes and
//!   uninitialised extents, directories with and without entry file types
//!   (hash-indexed ones through their leaves), fast and slow symbolic
//!   links, device nodes in both device number encodings, FIFOs and
//!   sockets, and inline data (files and directories in the inode).
//! - **ext4 features**: `64bit` (48-bit block numbers, 64-byte group
//!   descriptors), `extent`, `flex_bg` (metadata of a group anywhere),
//!   `sparse_super2`, `metadata_csum` (CRC-32C on the superblock, group
//!   descriptors, bitmaps, inodes, extent blocks, directory blocks and
//!   their index blocks, extended attribute blocks; verified on reading,
//!   computed on writing) and `metadata_csum_seed`, `uninit_bg` (CRC-16
//!   descriptors) and the "uninitialised" group flags, `huge_file`,
//!   `dir_nlink`, `extra_isize`, `inline_data` (read-only), and the jbd2
//!   journal (`has_journal`): replayed at mount, and every change of ours
//!   journaled (metadata in transactions, file data written in place before
//!   the commit: "ordered" mode). The orphan list is processed at mount and
//!   kept for files unlinked while open.
//! - **Features**: unknown `INCOMPAT` features refuse the mount (bigalloc,
//!   meta_bg, encryption, casefolding, `mmp`, `ea_inode`, external
//!   journals too); unknown `RO_COMPAT` features (and a read-only device)
//!   mount read-only. `COMPAT` features are ignored as the format allows; a
//!   hash-indexed directory loses its index (and its index blocks become
//!   empty leaves) when this driver changes it, and a file's extended
//!   attribute block is released (by reference count) when the file goes.
//! - **Writing**: block and inode bitmaps with group-local allocation, free
//!   counts in the group descriptors and the superblock, every namespace
//!   operation of the `FileSystem` trait, sparse writes, truncation, extent
//!   trees that grow, split, merge and shrink, directory entry insertion by
//!   splitting records and removal by merging into the previous one.
//! - **Crash safety**: with a journal, each transaction is atomic and a
//!   crash at any point leaves a volume that is consistent after replay.
//!   Without one, metadata writes are ordered so that a crash at any point
//!   leaves a volume `e2fsck` repairs without losing anything that was
//!   durable (see `balloc.rs` and ADR 0032). Unclean volumes are marked so
//!   in the superblock until [`Ext2::unmount`].
//! - **Never panics on disk contents**: every block number, inode number,
//!   directory record, extent and count read from disk is checked before
//!   use; a corrupted volume answers `EUCLEAN` (a bad checksum `EBADMSG`)
//!   and is marked as having errors, device failures `EIO`.
//!
//! # Locking
//!
//! One lock per volume, held by every operation from start to end, disk
//! I/O included, so each operation is atomic. [`Ext2::mount`] uses a
//! spinlock; [`Ext2::mount_with_lock`] takes any [`RawLock`], which is how
//! the kernel makes threads that wait for the volume sleep.
//!
//! # Mounting from the kernel
//!
//! The kernel's `block::Disk` (a request queue and a buffer cache per disk)
//! becomes a [`BlockIo`] with three one-line methods, and the mounted file
//! system goes into the VFS like tmpfs:
//!
//! ```ignore
//! struct DiskIo(&'static block::Disk);
//! impl poly_ext2::BlockIo for DiskIo {
//!     fn block_size(&self) -> u32 { self.0.info().block_size }
//!     fn blocks(&self) -> u64 { self.0.info().blocks }
//!     fn read_only(&self) -> bool { self.0.info().read_only }
//!     fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> { self.0.read(first, buf).map_err(io) }
//!     fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
//!         let bs = self.0.info().block_size as usize;
//!         buf.chunks(bs).zip(first..).try_for_each(|(b, n)| self.0.write_block(n, b)).map_err(io)
//!     }
//!     fn flush(&self) -> Result<(), IoError> { self.0.sync().map_err(io) }
//! }
//! // The kernel's sleeping mutex as the volume lock (any `poly_vfs::lock::RawLock`).
//! let fs = Ext2::mount_with_lock(DiskIo(block::root()?), clock, MountOptions::default(), RawMutex::new())?;
//! vfs.mount(&ctx, &cred, At::Cwd, b"/mnt", Arc::new(fs), MountFlags::default())?;
//! // Periodically (writeback, a journal commit) and at shutdown: fs.sync(), then fs.unmount().
//! ```
//!
//! [`CachedIo`] does the same over a `poly-block` cache and queue the caller
//! owns, and [`DeviceIo`] straight over a [`poly_block::BlockDevice`].

#![no_std]

extern crate alloc;

mod balloc;
mod bmap;
mod crc;
mod csum;
mod dir;
mod extent;
mod format;
mod inline;
mod inode;
mod io;
mod journal;
pub mod layout;
mod ops;
mod sync;
mod vfs;

pub use crc::{crc16, crc32c};
pub use format::{FormatOptions, JournalOptions, format};
pub use io::{BlockIo, CachedIo, DeviceIo};

use crate::io::Disk;
use crate::journal::Journal;
use crate::layout::*;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_vfs::lock::{Lock, RawLock, RawSpinLock};
use poly_vfs::{Clock, Errno, Result, Timespec};

/// How to mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountOptions {
    /// Refuse all changes. (A journal that needs replaying is replayed all
    /// the same, as Linux does, unless the device itself is read-only.)
    pub read_only: bool,
    /// Without a journal: order metadata writes with flushes so a crash
    /// leaves a volume fsck repairs without loss (see the crate
    /// documentation). Off, writes are ordered only at [`Ext2::sync`],
    /// which is faster and still leaves a volume e2fsck can repair, but a
    /// crash may cross-link blocks or lose a renamed file. Volumes with a
    /// journal are always journaled.
    pub ordered: bool,
}

impl Default for MountOptions {
    fn default() -> MountOptions {
        MountOptions { read_only: false, ordered: true }
    }
}

/// Why a volume did not mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountError {
    /// Reading the device failed.
    Io(Errno),
    /// No ext2 superblock (wrong magic number).
    NotExt2,
    /// A revision, block size or feature this driver does not handle.
    Unsupported(&'static str),
    /// `INCOMPAT` feature bits this driver does not know.
    IncompatFeatures(u32),
    /// The superblock, a group descriptor or the journal makes no sense
    /// (or fails its checksum).
    Corrupt(&'static str),
    /// The volume is larger than the device.
    TooSmall,
    /// The journal must be replayed, but the device is read-only.
    NeedsRecovery,
}

impl MountError {
    /// The `mount(2)` error number.
    pub fn errno(&self) -> Errno {
        match self {
            MountError::Io(e) => *e,
            MountError::Corrupt(_) => Errno::EUCLEAN,
            MountError::NeedsRecovery => Errno::EROFS,
            _ => Errno::EINVAL,
        }
    }
}

impl core::fmt::Display for MountError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MountError::Io(e) => write!(f, "I/O error ({e:?})"),
            MountError::NotExt2 => write!(f, "not an ext2/3/4 file system"),
            MountError::Unsupported(what) => write!(f, "unsupported {what}"),
            MountError::IncompatFeatures(bits) => write!(f, "unsupported incompatible features {bits:#x}"),
            MountError::Corrupt(what) => write!(f, "corrupt {what}"),
            MountError::TooSmall => write!(f, "file system larger than the device"),
            MountError::NeedsRecovery => write!(f, "the journal needs recovery but the device is read-only"),
        }
    }
}

/// Numbers about a mounted volume (`statfs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsStats {
    pub block_size: u32,
    pub blocks: u64,
    pub free_blocks: u64,
    pub inodes: u64,
    pub free_inodes: u64,
    pub read_only: bool,
    /// The volume was marked clean (unmounted properly, no errors) when it
    /// was mounted.
    pub was_clean: bool,
    /// Corruption was found since mounting (the volume will be marked as
    /// having errors).
    pub errors: bool,
    pub uuid: [u8; 16],
    pub label: [u8; 16],
    /// Journal transactions replayed at mount.
    pub replayed: u32,
    /// Inodes on the orphan list at mount, deleted or truncated then.
    pub orphans: u32,
    /// Transactions committed since mounting.
    pub commits: u64,
}

/// Which checksums the volume's metadata carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Csum {
    None,
    /// `uninit_bg`: CRC-16 group descriptors only.
    Gdt,
    /// `metadata_csum`, with its seed.
    Meta(u32),
}

/// What the superblock says, checked, fixed for the life of the mount.
#[derive(Debug)]
pub(crate) struct Geometry {
    /// Block size in bytes.
    pub bs: usize,
    pub blocks: u64,
    pub first_data: u64,
    pub bpg: u64,
    pub ipg: u32,
    pub inodes: u32,
    pub groups: u32,
    pub first_ino: u32,
    pub filetype: bool,
    pub sparse_super: bool,
    /// `sparse_super2`: the only groups besides 0 with superblock backups.
    pub backup_bgs: Option<[u32; 2]>,
    /// Inode table blocks per group.
    pub itable_blocks: u64,
    /// Descriptor table blocks, and the blocks reserved after them for
    /// growth (`resize_inode`).
    pub gdt_blocks: u64,
    pub reserved_gdt: u64,
    /// Bytes per group descriptor.
    pub desc_size: usize,
    /// Block pointers per indirect block.
    pub ptrs: u64,
    pub ifmt: InodeFormat,
    /// 512-byte units per block (`i_blocks` counts in them).
    pub spb: u32,
    pub csum: Csum,
    pub uuid: [u8; 16],
    /// New files get extent trees.
    pub extents: bool,
    pub dir_nlink: bool,
    pub link_max: u16,
    pub inline_data: bool,
    pub has_journal: bool,
    /// Every block that is fixed metadata (superblock copies, descriptor
    /// tables and their reserve, bitmaps, inode tables) as sorted,
    /// disjoint `(start, end)` runs.
    pub fixed: Vec<(u64, u64)>,
}

impl Geometry {
    pub fn group_start(&self, g: u32) -> u64 {
        self.first_data + g as u64 * self.bpg
    }

    pub fn blocks_in_group(&self, g: u32) -> u64 {
        (self.blocks - self.group_start(g)).min(self.bpg)
    }

    pub fn group_of_block(&self, b: u64) -> u32 {
        ((b - self.first_data) / self.bpg) as u32
    }

    pub fn group_of_inode(&self, ino: u32) -> u32 {
        (ino - 1) / self.ipg
    }

    /// Whether group `g` keeps a superblock and descriptor table copy.
    pub fn has_super(&self, g: u32) -> bool {
        match self.backup_bgs {
            Some(b) => g == 0 || (g != 0 && (g == b[0] || g == b[1])),
            None => has_super(g, self.sparse_super),
        }
    }

    /// Whether `b` is one of the fixed metadata blocks.
    pub fn is_fixed(&self, b: u64) -> bool {
        let i = self.fixed.partition_point(|r| r.1 <= b);
        self.fixed.get(i).is_some_and(|r| r.0 <= b)
    }

    /// The block holding the superblock, and its offset there.
    pub fn sb_block(&self) -> (u64, usize) {
        (SUPERBLOCK_OFFSET / self.bs as u64, (SUPERBLOCK_OFFSET % self.bs as u64) as usize)
    }

    /// Group descriptor checksums (and the "uninitialised" flags) in use.
    pub fn gdt_csum(&self) -> bool {
        self.csum != Csum::None
    }

    pub fn meta_seed(&self) -> Option<u32> {
        match self.csum {
            Csum::Meta(s) => Some(s),
            _ => None,
        }
    }
}

/// A free bitmap block being worked on (write-back until the operation
/// ends).
pub(crate) struct BitmapBuf {
    pub group: u32,
    pub data: Vec<u8>,
    pub dirty: bool,
}

/// Everything that changes while mounted, behind the lock.
pub(crate) struct State {
    pub sb: Superblock,
    /// The descriptor table as on disk (unknown fields kept) and decoded.
    pub gdt_raw: Vec<u8>,
    pub groups: Vec<GroupDesc>,
    /// Groups whose descriptors changed since they were written.
    pub dirty_groups: BTreeSet<u32>,
    pub read_only: bool,
    pub ordered: bool,
    pub was_clean: bool,
    pub unmounted: bool,
    /// The superblock changed since it was written.
    pub meta_dirty: bool,
    /// The feature flags changed (backup superblocks need them too).
    pub features_dirty: bool,
    pub large_file: bool,
    pub max_size: u64,
    /// `open` pins per inode.
    pub pins: BTreeMap<u32, u32>,
    /// Inodes whose last name went while they were pinned: deleted at the
    /// last release (on the orphan list on disk, with a journal).
    pub orphans: BTreeSet<u32>,
    /// Blocks (as runs `start -> end`) and inodes freed in the bitmaps, not
    /// to be reused until the removal of the pointers to them is durable
    /// (the next barrier, or the commit of the transaction).
    pub held_blocks: BTreeMap<u64, u64>,
    pub held_inodes: BTreeSet<u32>,
    /// A newly allocated metadata block was written; it must be durable
    /// before anything that may point to it is written (no journal).
    pub fresh_unbarriered: bool,
    pub block_bitmap: Option<BitmapBuf>,
    pub inode_bitmap: Option<BitmapBuf>,
    pub next_generation: u32,
    pub journal: Option<Journal>,
    pub replayed: u32,
    pub orphans_at_mount: u32,
}

/// A mounted ext2/3/4 volume on device `D`, its state behind a lock of
/// type `L` (see [`Ext2::mount_with_lock`]).
pub struct Ext2<D, L = RawSpinLock> {
    disk: Disk<D>,
    geo: Geometry,
    clock: Arc<dyn Clock>,
    errors: AtomicBool,
    state: Lock<L, State>,
}

impl<D, L> core::fmt::Debug for Ext2<D, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ext2").field("geometry", &self.geo).finish_non_exhaustive()
    }
}

/// One operation's view of the mounted volume: the device, the geometry,
/// the locked state and the operation's caches.
pub(crate) struct Ctx<'a, D> {
    pub disk: &'a Disk<D>,
    pub geo: &'a Geometry,
    pub st: &'a mut State,
    pub now: Timespec,
    errors: &'a AtomicBool,
    /// Indirect blocks this operation works on.
    pub ind: bmap::IndCache,
    /// Extent tree blocks this operation read.
    pub ext: extent::ExtCache,
}

impl<D: BlockIo> Ctx<'_, D> {
    /// Records that the volume is corrupt; returns the error to report.
    pub fn corrupt(&self) -> Errno {
        self.errors.store(true, Ordering::Relaxed);
        Errno::EUCLEAN
    }

    /// Records a checksum mismatch (corruption); returns `EBADMSG`.
    pub fn bad_csum(&self) -> Errno {
        self.errors.store(true, Ordering::Relaxed);
        Errno::EBADMSG
    }

    /// Whether `b` may be a data, indirect, extent or directory block:
    /// inside the volume and not one of the fixed metadata blocks.
    pub fn block_ok(&self, b: u64) -> bool {
        b >= self.geo.first_data && b < self.geo.blocks && !self.geo.is_fixed(b)
    }

    /// A metadata block: the running transaction's copy if it has one.
    pub fn read_block(&self, b: u64) -> Result<Vec<u8>> {
        if let Some(data) = self.st.journal.as_ref().and_then(|j| j.get(b)) {
            return Ok(data.to_vec());
        }
        let mut buf = vec![0u8; self.geo.bs];
        self.disk.read(b, &mut buf)?;
        Ok(buf)
    }

    /// Puts the superblock (with its checksum) into `raw`.
    fn seal_super(&mut self) {
        let geo = self.geo;
        let (free_b, free_i) =
            self.st.groups.iter().fold((0u64, 0u64), |(b, i), d| (b + d.free_blocks as u64, i + d.free_inodes as u64));
        let st = &mut *self.st;
        st.sb.set_free_blocks_count(free_b);
        st.sb.set_free_inodes_count(free_i.min(u32::MAX as u64) as u32);
        st.sb.set_wtime(encode_time(self.now, false).0);
        if self.errors.load(Ordering::Relaxed) {
            st.sb.set_state(st.sb.state() | STATE_ERROR);
        }
        if geo.meta_seed().is_some() {
            let c = csum::sb(&st.sb.raw);
            st.sb.set_checksum(c);
        }
    }

    /// Writes the superblock and the changed group descriptors (into the
    /// running transaction with a journal, in place without).
    pub fn write_super(&mut self) -> Result<()> {
        let geo = self.geo;
        if !self.st.meta_dirty && self.st.dirty_groups.is_empty() {
            return Ok(());
        }
        self.seal_super();
        let ds = geo.desc_size;
        let dirty = core::mem::take(&mut self.st.dirty_groups);
        let mut gdt_blocks = BTreeSet::new();
        for &g in &dirty {
            let st = &mut *self.st;
            let mut d = st.groups[g as usize];
            let raw = &mut st.gdt_raw[g as usize * ds..(g as usize + 1) * ds];
            d.encode(raw, ds);
            d.checksum = csum::group_desc(geo, g, raw);
            put16(raw, 0x1E, d.checksum);
            st.groups[g as usize] = d;
            gdt_blocks.insert(g as u64 * ds as u64 / geo.bs as u64);
        }
        let journaled = self.st.journal.is_some();
        let (sb_block, sb_off) = geo.sb_block();
        if journaled {
            let mut b = self.read_block(sb_block)?;
            b[sb_off..sb_off + SUPERBLOCK_SIZE].copy_from_slice(&self.st.sb.raw);
            self.journal_put(sb_block, b);
        } else {
            self.disk.write_bytes(SUPERBLOCK_OFFSET, &self.st.sb.raw)?;
        }
        for i in gdt_blocks {
            let at = i as usize * geo.bs;
            let data = self.st.gdt_raw[at..at + geo.bs].to_vec();
            let b = geo.first_data + 1 + i;
            if journaled {
                self.journal_put(b, data);
            } else {
                self.disk.write(b, &data)?;
            }
        }
        if self.st.features_dirty {
            // Backup superblocks carry the feature flags e2fsck compares.
            for g in 1..geo.groups {
                if !geo.has_super(g) {
                    continue;
                }
                let mut b = vec![0u8; geo.bs];
                self.disk.read(geo.group_start(g), &mut b)?;
                put32(&mut b, 100, self.st.sb.feature_ro_compat_raw());
                if geo.meta_seed().is_some() {
                    let c = csum::sb(&b[..SUPERBLOCK_SIZE]);
                    put32(&mut b, 1020, c);
                }
                self.disk.write(geo.group_start(g), &b)?;
            }
            self.st.features_dirty = false;
        }
        self.st.meta_dirty = false;
        Ok(())
    }

    /// Writes the superblock in place now (mounting, unmounting), with its
    /// checksum, and makes it durable.
    pub fn write_super_now(&mut self) -> Result<()> {
        self.seal_super();
        self.disk.write_bytes(SUPERBLOCK_OFFSET, &self.st.sb.raw)?;
        self.disk.flush()
    }

    /// Writes everything back and makes it durable: commits the running
    /// transaction with a journal.
    pub fn sync(&mut self) -> Result<()> {
        self.ind_flush()?;
        self.flush_bitmaps()?;
        if self.st.journal.is_some() {
            self.st.meta_dirty = true;
            self.write_super()?;
            return self.commit();
        }
        self.barrier()?;
        self.flush_bitmaps()?;
        self.st.meta_dirty = true;
        self.write_super()?;
        self.disk.flush()
    }
}

impl<D: BlockIo> Ext2<D> {
    /// Mounts the volume on `dev`, its state behind a spinlock. Unless
    /// read-only, the volume is marked "not clean" on disk until
    /// [`Ext2::unmount`].
    pub fn mount(dev: D, clock: Arc<dyn Clock>, opts: MountOptions) -> core::result::Result<Ext2<D>, MountError> {
        Ext2::mount_with_lock(dev, clock, opts, RawSpinLock::new())
    }
}

/// Checks the superblock and works out the geometry.
fn geometry<D: BlockIo>(disk: &mut Disk<D>, sb: &Superblock) -> core::result::Result<Geometry, MountError> {
    if sb.magic() != MAGIC {
        return Err(MountError::NotExt2);
    }
    let rev = sb.rev_level();
    if rev > DYNAMIC_REV {
        return Err(MountError::Unsupported("revision"));
    }
    if sb.log_block_size() > 2 {
        return Err(MountError::Unsupported("block size"));
    }
    let bs = 1024usize << sb.log_block_size();
    let incompat = sb.feature_incompat();
    if incompat & !INCOMPAT_SUPPORTED != 0 {
        return Err(MountError::IncompatFeatures(incompat & !INCOMPAT_SUPPORTED));
    }
    let compat = sb.feature_compat();
    let ro_compat = sb.feature_ro_compat();
    if ro_compat & 0x200 != 0 {
        return Err(MountError::Unsupported("bigalloc"));
    }
    let csum = if ro_compat & RO_COMPAT_METADATA_CSUM != 0 {
        if sb.checksum_type() != 1 {
            return Err(MountError::Unsupported("checksum type"));
        }
        if csum::sb(&sb.raw) != sb.checksum() {
            return Err(MountError::Corrupt("superblock checksum"));
        }
        let seed = if incompat & INCOMPAT_CSUM_SEED != 0 { sb.checksum_seed() } else { crc::crc32c(!0, &sb.uuid()) };
        Csum::Meta(seed)
    } else if ro_compat & RO_COMPAT_GDT_CSUM != 0 {
        Csum::Gdt
    } else {
        Csum::None
    };
    let first_data = sb.first_data_block() as u64;
    if first_data != (bs == 1024) as u64 {
        return Err(MountError::Corrupt("first data block"));
    }
    let bpg = sb.blocks_per_group() as u64;
    let ipg = sb.inodes_per_group();
    let bits = 8 * bs as u64;
    if bpg < 8 || bpg > bits || ipg == 0 || ipg as u64 > bits || !ipg.is_multiple_of(8) && csum != Csum::None {
        return Err(MountError::Corrupt("group size"));
    }
    let isz = sb.inode_size() as usize;
    if !isz.is_power_of_two() || isz < GOOD_OLD_INODE_SIZE as usize || isz > bs {
        return Err(MountError::Corrupt("inode size"));
    }
    let blocks = sb.blocks_count();
    if blocks <= first_data {
        return Err(MountError::Corrupt("block count"));
    }
    let groups = (blocks - first_data).div_ceil(bpg);
    if groups > u32::MAX as u64 {
        return Err(MountError::Corrupt("block count"));
    }
    let inodes = sb.inodes_count();
    let first_ino = sb.first_ino();
    if inodes as u64 > groups * ipg as u64 || first_ino <= ROOT_INO || first_ino > inodes {
        return Err(MountError::Corrupt("inode count"));
    }
    disk.set_block_size(bs);
    if blocks > disk.capacity() {
        return Err(MountError::TooSmall);
    }
    let desc_size = sb.desc_size();
    if desc_size < DESC_SIZE || !desc_size.is_power_of_two() || desc_size > bs {
        return Err(MountError::Corrupt("descriptor size"));
    }
    if incompat & INCOMPAT_64BIT != 0 && desc_size < DESC_SIZE_64 {
        return Err(MountError::Corrupt("descriptor size"));
    }
    if compat & COMPAT_HAS_JOURNAL != 0 && sb.journal_inum() == 0 {
        return Err(MountError::Unsupported("external journal"));
    }
    let gdt_blocks = (groups * desc_size as u64).div_ceil(bs as u64);
    let reserved_gdt = if compat & COMPAT_RESIZE_INODE != 0 { sb.reserved_gdt_blocks() as u64 } else { 0 };
    let ptrs = (bs / 4) as u64;
    let creator = sb.creator_os();
    let ext4ish = incompat & (INCOMPAT_EXTENTS | INCOMPAT_64BIT | INCOMPAT_FLEX_BG) != 0
        || ro_compat & (RO_COMPAT_HUGE_FILE | RO_COMPAT_DIR_NLINK | RO_COMPAT_METADATA_CSUM) != 0;
    let spb = (bs / 512) as u32;
    Ok(Geometry {
        bs,
        blocks,
        first_data,
        bpg,
        ipg,
        inodes,
        groups: groups as u32,
        first_ino,
        filetype: incompat & INCOMPAT_FILETYPE != 0,
        sparse_super: ro_compat & RO_COMPAT_SPARSE_SUPER != 0,
        backup_bgs: (compat & COMPAT_SPARSE_SUPER2 != 0).then(|| sb.backup_bgs()),
        itable_blocks: (ipg as u64 * isz as u64).div_ceil(bs as u64),
        gdt_blocks,
        reserved_gdt,
        desc_size,
        ptrs,
        ifmt: InodeFormat {
            size: isz,
            large_size: rev >= DYNAMIC_REV,
            id_high: creator == OS_LINUX || creator == OS_HURD,
            huge_file: ro_compat & RO_COMPAT_HUGE_FILE != 0,
            wide: incompat & INCOMPAT_64BIT != 0,
            spb,
        },
        spb,
        csum,
        uuid: sb.uuid(),
        extents: incompat & INCOMPAT_EXTENTS != 0,
        dir_nlink: ro_compat & RO_COMPAT_DIR_NLINK != 0,
        link_max: if ext4ish { EXT4_LINK_MAX } else { LINK_MAX },
        inline_data: incompat & INCOMPAT_INLINE_DATA != 0,
        has_journal: compat & COMPAT_HAS_JOURNAL != 0,
        fixed: Vec::new(),
    })
}

/// The fixed metadata runs of every group, checked against the volume.
fn fixed_runs(geo: &Geometry, descs: &[GroupDesc]) -> core::result::Result<Vec<(u64, u64)>, MountError> {
    let mut runs = Vec::with_capacity(descs.len() * 4);
    for (g, d) in descs.iter().enumerate() {
        let g = g as u32;
        if geo.has_super(g) {
            let start = geo.group_start(g);
            let end = start + 1 + geo.gdt_blocks + geo.reserved_gdt;
            if end > geo.blocks || 1 + geo.gdt_blocks + geo.reserved_gdt >= geo.bpg {
                return Err(MountError::Corrupt("descriptor table size"));
            }
            // With 1 KiB blocks block 0 is the boot block: below the volume.
            runs.push((start, end));
        }
        runs.push((d.block_bitmap, d.block_bitmap + 1));
        runs.push((d.inode_bitmap, d.inode_bitmap + 1));
        runs.push((d.inode_table, d.inode_table + geo.itable_blocks));
    }
    runs.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(runs.len());
    for (s, e) in runs {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    Ok(merged)
}

impl<D: BlockIo, L: RawLock> Ext2<D, L> {
    /// [`Ext2::mount`] with the volume's state behind `lock`, which every
    /// operation holds while it runs, disk I/O included: a kernel passes
    /// its sleeping mutex, so threads waiting for the volume sleep while the
    /// holder waits for the disk.
    ///
    /// A journal that needs replaying is replayed first; then, unless the
    /// mount is read-only, the orphan list is processed (files deleted while
    /// open are freed, interrupted truncations finished).
    pub fn mount_with_lock(
        dev: D,
        clock: Arc<dyn Clock>,
        opts: MountOptions,
        lock: L,
    ) -> core::result::Result<Ext2<D, L>, MountError> {
        let probe = Ext2::<D, RawSpinLock>::load(dev, Arc::clone(&clock), opts, RawSpinLock::new(), true)?;
        let replayed = probe.recover()?;
        let dev = probe.into_device_unsynced();
        let fs = Ext2::load(dev, clock, opts, lock, false)?;
        fs.start(replayed)?;
        Ok(fs)
    }

    /// Reads and checks the superblock, the descriptors and the journal's
    /// superblock; writes nothing.
    fn load(
        dev: D,
        clock: Arc<dyn Clock>,
        opts: MountOptions,
        lock: L,
        probe: bool,
    ) -> core::result::Result<Ext2<D, L>, MountError> {
        let dbs = dev.block_size();
        if !dbs.is_power_of_two() || dbs < 512 {
            return Err(MountError::Unsupported("device block size"));
        }
        let mut disk = Disk::new(dev, 1024);
        let mut raw = [0u8; SUPERBLOCK_SIZE];
        if disk.dev.blocks().saturating_mul(dbs as u64) < SUPERBLOCK_OFFSET + SUPERBLOCK_SIZE as u64 {
            return Err(MountError::TooSmall);
        }
        disk.read_bytes(SUPERBLOCK_OFFSET, &mut raw).map_err(MountError::Io)?;
        let sb = Superblock::new(raw);
        let mut geo = geometry(&mut disk, &sb)?;
        let bs = geo.bs;
        let ds = geo.desc_size;
        let mut gdt_raw = vec![0u8; geo.gdt_blocks as usize * bs];
        disk.read(geo.first_data + 1, &mut gdt_raw).map_err(MountError::Io)?;
        let mut descs = Vec::with_capacity(geo.groups as usize);
        for g in 0..geo.groups as usize {
            let raw = &gdt_raw[g * ds..(g + 1) * ds];
            let d = GroupDesc::decode(raw, ds);
            let inside = |b: u64| b >= geo.first_data && b < geo.blocks;
            if !inside(d.block_bitmap)
                || !inside(d.inode_bitmap)
                || !inside(d.inode_table)
                || d.inode_table + geo.itable_blocks > geo.blocks
                || d.free_blocks as u64 > geo.bpg
                || d.free_inodes > geo.ipg
                || (geo.gdt_csum() && d.itable_unused > geo.ipg)
            {
                return Err(MountError::Corrupt("group descriptor"));
            }
            if geo.gdt_csum() && csum::group_desc(&geo, g as u32, raw) != d.checksum {
                return Err(MountError::Corrupt("group descriptor checksum"));
            }
            descs.push(d);
        }
        geo.fixed = fixed_runs(&geo, &descs)?;
        let ro_compat = sb.feature_ro_compat();
        let mut max_size = if geo.extents {
            ((1u64 << 32) - 1) * bs as u64
        } else {
            let p = geo.ptrs;
            ((12 + p + p * p + p * p * p) * bs as u64).min(u32::MAX as u64 * 512)
        };
        if geo.ifmt.huge_file {
            max_size = max_size.min(((1u64 << 48) - 1) * 512);
        } else {
            max_size = max_size.min(u32::MAX as u64 * 512);
        }
        if sb.rev_level() == GOOD_OLD_REV {
            max_size = max_size.min(i32::MAX as u64);
        }
        let read_only = opts.read_only
            || disk.dev.read_only()
            || ro_compat & !RO_COMPAT_SUPPORTED != 0
            || sb.feature_incompat() & INCOMPAT_READ_ONLY != 0
            || probe;
        let now = clock.now();
        let state = State {
            was_clean: sb.state() & (STATE_VALID | STATE_ERROR) == STATE_VALID,
            large_file: ro_compat & RO_COMPAT_LARGE_FILE != 0,
            sb,
            gdt_raw,
            groups: descs,
            dirty_groups: BTreeSet::new(),
            read_only,
            ordered: opts.ordered,
            unmounted: false,
            meta_dirty: false,
            features_dirty: false,
            max_size,
            pins: BTreeMap::new(),
            orphans: BTreeSet::new(),
            held_blocks: BTreeMap::new(),
            held_inodes: BTreeSet::new(),
            fresh_unbarriered: false,
            block_bitmap: None,
            inode_bitmap: None,
            next_generation: now.sec as u32 ^ now.nsec,
            journal: None,
            replayed: 0,
            orphans_at_mount: 0,
        };
        let fs = Ext2 { disk, geo, clock, errors: AtomicBool::new(false), state: Lock::with_raw(lock, state) };
        if fs.geo.has_journal {
            let j = {
                let mut st = fs.state.lock();
                let mut ctx = fs.ctx(&mut st);
                Journal::open(&mut ctx)?
            };
            fs.state.lock().journal = Some(j);
        }
        Ok(fs)
    }

    /// Replays the journal if it needs it (the probe mount does this).
    fn recover(&self) -> core::result::Result<u32, MountError> {
        let mut st = self.state.lock();
        let needs = st.sb.feature_incompat() & INCOMPAT_RECOVER != 0
            || st.journal.as_ref().is_some_and(|j| j.needs_recovery());
        if st.journal.is_none() || !needs {
            return Ok(0);
        }
        if self.disk.dev.read_only() {
            return Err(MountError::NeedsRecovery);
        }
        let mut ctx = self.ctx(&mut st);
        let n = journal::recover(&mut ctx)?;
        // The replayed superblock is on disk now: clear the flag there.
        let mut raw = [0u8; SUPERBLOCK_SIZE];
        self.disk.read_bytes(SUPERBLOCK_OFFSET, &mut raw).map_err(MountError::Io)?;
        let mut sb = Superblock::new(raw);
        sb.set_feature_incompat(sb.feature_incompat_raw() & !INCOMPAT_RECOVER);
        if sb.feature_ro_compat() & RO_COMPAT_METADATA_CSUM != 0 {
            let c = csum::sb(&sb.raw);
            sb.set_checksum(c);
        }
        self.disk.write_bytes(SUPERBLOCK_OFFSET, &sb.raw).map_err(MountError::Io)?;
        self.disk.flush().map_err(MountError::Io)?;
        Ok(n)
    }

    /// Checks the root, processes orphans and marks the volume mounted.
    fn start(&self, replayed: u32) -> core::result::Result<(), MountError> {
        let mut st = self.state.lock();
        st.replayed = replayed;
        let mut ctx = self.ctx(&mut st);
        match ctx.read_inode(ROOT_INO) {
            Ok(root) if root.is_dir() && root.links > 0 => {}
            Ok(_) => return Err(MountError::Corrupt("root directory")),
            Err(Errno::EBADMSG) => return Err(MountError::Corrupt("root directory checksum")),
            Err(e) => return Err(MountError::Io(e)),
        }
        if ctx.st.read_only {
            return Ok(());
        }
        let now = ctx.now;
        let sb = &mut ctx.st.sb;
        sb.set_state(sb.state() & !STATE_VALID);
        sb.set_mnt_count(sb.mnt_count().wrapping_add(1));
        sb.set_mtime(encode_time(now, false).0);
        if ctx.st.journal.is_some() {
            sb.set_feature_incompat(sb.feature_incompat_raw() | INCOMPAT_RECOVER);
            let wide = ctx.geo.ifmt.wide;
            let v3 = ctx.geo.meta_seed().is_some();
            ctx.journal_prepare(wide, v3).map_err(MountError::Io)?;
        }
        ctx.write_super_now().map_err(MountError::Io)?;
        let n = ctx.process_orphans().map_err(MountError::Io)?;
        ctx.st.orphans_at_mount = n;
        if n > 0 {
            ctx.sync().map_err(MountError::Io)?;
        }
        Ok(())
    }

    pub(crate) fn ctx<'a>(&'a self, st: &'a mut State) -> Ctx<'a, D> {
        Ctx {
            disk: &self.disk,
            geo: &self.geo,
            st,
            now: self.clock.now(),
            errors: &self.errors,
            ind: bmap::IndCache::default(),
            ext: extent::ExtCache::default(),
        }
    }

    /// Runs a read-only operation.
    pub(crate) fn read_op<R>(&self, f: impl FnOnce(&mut Ctx<'_, D>) -> Result<R>) -> Result<R> {
        let mut st = self.state.lock();
        let mut ctx = self.ctx(&mut st);
        f(&mut ctx)
    }

    /// Runs an operation that may change the volume, then writes back what
    /// it left in its caches. With a journal, a transaction that is getting
    /// big (or freed space that is needed) is committed first.
    pub(crate) fn write_op<R>(&self, f: impl FnOnce(&mut Ctx<'_, D>) -> Result<R>) -> Result<R> {
        let mut st = self.state.lock();
        if st.read_only {
            return Err(Errno::EROFS);
        }
        let mut ctx = self.ctx(&mut st);
        ctx.journal_reserve()?;
        let r = f(&mut ctx);
        let flushed = ctx.ind_flush().and_then(|()| ctx.flush_bitmaps());
        match (r, flushed) {
            (Ok(v), Ok(())) => Ok(v),
            (Err(e), _) | (Ok(_), Err(e)) => Err(e),
        }
    }

    /// Writes everything back and makes it durable (the volume stays marked
    /// "not clean" while mounted). With a journal: commits.
    pub fn sync(&self) -> Result<()> {
        let mut st = self.state.lock();
        if st.read_only {
            return Ok(());
        }
        self.ctx(&mut st).sync()
    }

    /// Syncs and marks the volume clean (or as having errors, if corruption
    /// was found), then refuses all further changes. Reads keep working.
    pub fn unmount(&self) -> Result<()> {
        let mut st = self.state.lock();
        if st.read_only {
            return Ok(());
        }
        let mut ctx = self.ctx(&mut st);
        // Inodes still pinned after their last name went would be found by
        // e2fsck without a name: delete them now (their pins are void).
        ctx.st.pins.clear();
        let orphans: Vec<u32> = ctx.st.orphans.iter().copied().collect();
        for ino in orphans {
            ctx.release_orphan(ino)?;
            ctx.ind_flush()?;
            ctx.flush_bitmaps()?;
        }
        ctx.sync()?;
        if ctx.st.journal.is_some() {
            ctx.journal_clean()?;
            let sb = &mut ctx.st.sb;
            sb.set_feature_incompat(sb.feature_incompat_raw() & !INCOMPAT_RECOVER);
        }
        let sb = &mut ctx.st.sb;
        sb.set_state(sb.state() | STATE_VALID);
        ctx.write_super_now()?;
        ctx.st.read_only = true;
        ctx.st.unmounted = true;
        Ok(())
    }

    /// Unmounts (if not done yet) and hands the device back.
    pub fn into_device(self) -> Result<D> {
        self.unmount()?;
        Ok(self.disk.dev)
    }

    /// Hands the device back without writing anything: what a crash leaves.
    pub fn into_device_unsynced(self) -> D {
        self.disk.dev
    }

    pub fn stats(&self) -> FsStats {
        let st = self.state.lock();
        let (free_blocks, free_inodes) =
            st.groups.iter().fold((0u64, 0u64), |(b, i), d| (b + d.free_blocks as u64, i + d.free_inodes as u64));
        FsStats {
            block_size: self.geo.bs as u32,
            blocks: self.geo.blocks,
            free_blocks,
            inodes: self.geo.inodes as u64,
            free_inodes,
            read_only: st.read_only,
            was_clean: st.was_clean,
            errors: self.errors.load(Ordering::Relaxed),
            uuid: st.sb.uuid(),
            label: st.sb.volume_name(),
            replayed: st.replayed,
            orphans: st.orphans_at_mount,
            commits: st.journal.as_ref().map_or(0, |j| j.commits),
        }
    }

    /// `"ext4"` for volumes with ext4 features, `"ext3"` for journaled
    /// ones without, else `"ext2"`.
    pub fn fs_type(&self) -> &'static str {
        if self.geo.link_max == EXT4_LINK_MAX || self.geo.extents {
            "ext4"
        } else if self.geo.has_journal {
            "ext3"
        } else {
            "ext2"
        }
    }

    pub fn read_only(&self) -> bool {
        self.state.lock().read_only
    }

    pub fn device(&self) -> &D {
        &self.disk.dev
    }
}

#[cfg(test)]
mod tests;
