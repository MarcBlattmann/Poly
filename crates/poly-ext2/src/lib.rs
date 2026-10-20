//! The ext2 file system.
//!
//! [`Ext2`] mounts an ext2 volume from any [`BlockIo`] and implements
//! [`poly_vfs::FileSystem`] on it, so the VFS can put it anywhere in the
//! tree. [`format`] makes a new, empty volume.
//!
//! - **Reading**: revisions 0 and 1, 1/2/4 KiB blocks, inodes of any size
//!   (with ext4's nanosecond timestamps in large inodes), 32-bit uids and
//!   gids, 64-bit file sizes, direct and single/double/triple indirect block
//!   maps with holes, directories with and without entry file types
//!   (hash-indexed ones too, read linearly), fast and slow symbolic links,
//!   device nodes in both device number encodings, FIFOs and sockets.
//! - **Features**: unknown `INCOMPAT` features refuse the mount; unknown
//!   `RO_COMPAT` features (and a read-only device) mount read-only. `COMPAT`
//!   features are ignored as the format allows; a hash-indexed directory
//!   loses its index flag when this driver changes it, and a file's extended
//!   attribute block is released (by reference count) when the file goes.
//! - **Writing**: block and inode bitmaps with group-local allocation
//!   (directories spread over groups, files next to their directory, blocks
//!   next to the previous block of the file), free counts in the group
//!   descriptors and the superblock, every namespace operation of the
//!   `FileSystem` trait, sparse writes, truncation with indirect block
//!   management, directory entry insertion by splitting records and removal
//!   by merging into the previous one.
//! - **Crash safety without a journal**: metadata writes are ordered so that
//!   a crash at any point leaves a volume `e2fsck` repairs without losing
//!   anything that was durable (see `balloc.rs` and ADR 0032): a block or
//!   inode is reused only after the removal of every on-disk pointer to it is
//!   durable, a new indirect, directory or symbolic link block is durable
//!   before anything on disk points to it, and `rename` makes the new name
//!   durable before removing the old one. Unclean volumes are marked so in
//!   the superblock until [`Ext2::unmount`].
//! - **Never panics on disk contents**: every block number, inode number,
//!   directory record and count read from disk is checked before use; a
//!   corrupted volume answers `EUCLEAN` (and is marked as having errors),
//!   device failures `EIO`.
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
//! // Periodically (writeback) and at shutdown: fs.sync(), then fs.unmount().
//! ```
//!
//! [`CachedIo`] does the same over a `poly-block` cache and queue the caller
//! owns, and [`DeviceIo`] straight over a [`poly_block::BlockDevice`].

#![no_std]

extern crate alloc;

mod balloc;
mod bmap;
mod dir;
mod format;
mod inode;
mod io;
pub mod layout;
mod ops;
mod sync;
mod vfs;

pub use format::{FormatOptions, format};
pub use io::{BlockIo, CachedIo, DeviceIo};

use crate::io::Disk;
use crate::layout::*;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_vfs::lock::{Lock, RawLock, RawSpinLock};
use poly_vfs::{Clock, Errno, Result, Timespec};

/// How to mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountOptions {
    /// Refuse all changes (and do not touch the volume at all).
    pub read_only: bool,
    /// Order metadata writes with flushes so a crash leaves a volume fsck
    /// repairs without loss (see the crate documentation). Off, writes are
    /// ordered only at [`Ext2::sync`], which is faster and still leaves a
    /// volume e2fsck can repair, but a crash may cross-link blocks or lose
    /// a renamed file.
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
    /// A revision or block size this driver does not handle.
    Unsupported(&'static str),
    /// `INCOMPAT` feature bits this driver does not know.
    IncompatFeatures(u32),
    /// The superblock or a group descriptor makes no sense.
    Corrupt(&'static str),
    /// The volume is larger than the device.
    TooSmall,
}

impl MountError {
    /// The `mount(2)` error number.
    pub fn errno(&self) -> Errno {
        match self {
            MountError::Io(e) => *e,
            MountError::Corrupt(_) => Errno::EUCLEAN,
            _ => Errno::EINVAL,
        }
    }
}

impl core::fmt::Display for MountError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MountError::Io(e) => write!(f, "I/O error ({e:?})"),
            MountError::NotExt2 => write!(f, "not an ext2 file system"),
            MountError::Unsupported(what) => write!(f, "unsupported {what}"),
            MountError::IncompatFeatures(bits) => write!(f, "unsupported incompatible features {bits:#x}"),
            MountError::Corrupt(what) => write!(f, "corrupt {what}"),
            MountError::TooSmall => write!(f, "file system larger than the device"),
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
    /// Inode table blocks per group.
    pub itable_blocks: u64,
    /// Descriptor table blocks, and the blocks reserved after them for
    /// growth (`resize_inode`).
    pub gdt_blocks: u64,
    pub reserved_gdt: u64,
    /// Block pointers per indirect block.
    pub ptrs: u64,
    pub ifmt: InodeFormat,
    /// 512-byte units per block (`i_blocks` counts in them).
    pub spb: u32,
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
}

/// A free bitmap block being worked on (write-back until the operation
/// ends).
pub(crate) struct BitmapBuf {
    pub block: u64,
    pub data: Vec<u8>,
    pub dirty: bool,
}

/// Everything that changes while mounted, behind the lock.
pub(crate) struct State {
    pub sb: Superblock,
    /// The descriptor table as on disk (unknown fields kept) and decoded.
    pub gdt_raw: Vec<u8>,
    pub groups: Vec<GroupDesc>,
    pub read_only: bool,
    pub ordered: bool,
    pub was_clean: bool,
    pub unmounted: bool,
    /// The superblock or descriptors changed since they were written.
    pub meta_dirty: bool,
    /// The feature flags changed (backup superblocks need them too).
    pub features_dirty: bool,
    pub large_file: bool,
    pub max_size: u64,
    /// `open` pins per inode.
    pub pins: BTreeMap<u32, u32>,
    /// Blocks (as runs) and inodes freed, waiting for the removal of the
    /// pointers to them to be durable before they may be reused.
    pub pending_blocks: Vec<(u64, u64)>,
    pub pending_inodes: Vec<(u32, bool)>,
    /// A newly allocated metadata block was written; it must be durable
    /// before anything that may point to it is written.
    pub fresh_unbarriered: bool,
    pub block_bitmap: Option<BitmapBuf>,
    pub inode_bitmap: Option<BitmapBuf>,
    pub next_generation: u32,
}

/// A mounted ext2 volume on device `D`, its state behind a lock of type
/// `L` (see [`Ext2::mount_with_lock`]).
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
}

impl<D: BlockIo> Ctx<'_, D> {
    /// Records that the volume is corrupt; returns the error to report.
    pub fn corrupt(&self) -> Errno {
        self.errors.store(true, Ordering::Relaxed);
        Errno::EUCLEAN
    }

    /// Whether `b` may be a data, indirect or directory block: inside the
    /// volume and not one of the fixed metadata blocks of its group.
    pub fn block_ok(&self, b: u64) -> bool {
        let g = &self.geo;
        if b < g.first_data || b >= g.blocks {
            return false;
        }
        let group = g.group_of_block(b);
        let d = &self.st.groups[group as usize];
        if b == d.block_bitmap as u64 || b == d.inode_bitmap as u64 {
            return false;
        }
        let table = d.inode_table as u64;
        if b >= table && b < table + g.itable_blocks {
            return false;
        }
        if has_super(group, g.sparse_super) {
            let start = g.group_start(group);
            if b < start + 1 + g.gdt_blocks + g.reserved_gdt {
                return false;
            }
        }
        true
    }

    pub fn read_block(&self, b: u64) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; self.geo.bs];
        self.disk.read(b, &mut buf)?;
        Ok(buf)
    }

    /// Writes the superblock and the descriptor table if they changed.
    pub fn write_super(&mut self) -> Result<()> {
        if !self.st.meta_dirty {
            return Ok(());
        }
        let geo = self.geo;
        let (free_b, free_i) =
            self.st.groups.iter().fold((0u64, 0u64), |(b, i), d| (b + d.free_blocks as u64, i + d.free_inodes as u64));
        let st = &mut *self.st;
        st.sb.set_free_blocks_count(free_b.min(u32::MAX as u64) as u32);
        st.sb.set_free_inodes_count(free_i.min(u32::MAX as u64) as u32);
        st.sb.set_wtime(encode_time(self.now, false).0);
        if self.errors.load(Ordering::Relaxed) {
            st.sb.set_state(st.sb.state() | STATE_ERROR);
        }
        for (i, d) in st.groups.iter().enumerate() {
            d.encode(&mut st.gdt_raw[i * DESC_SIZE..(i + 1) * DESC_SIZE]);
        }
        self.disk.write_bytes(SUPERBLOCK_OFFSET, &st.sb.raw)?;
        self.disk.write(geo.first_data + 1, &st.gdt_raw)?;
        if st.features_dirty {
            // Backup superblocks carry the feature flags e2fsck compares.
            for g in 1..geo.groups {
                if !has_super(g, geo.sparse_super) {
                    continue;
                }
                let mut b = vec![0u8; geo.bs];
                self.disk.read(geo.group_start(g), &mut b)?;
                put32(&mut b, 100, st.sb.feature_ro_compat_raw());
                self.disk.write(geo.group_start(g), &b)?;
            }
            st.features_dirty = false;
        }
        st.meta_dirty = false;
        Ok(())
    }

    /// Writes everything back and makes it durable.
    pub fn sync(&mut self) -> Result<()> {
        self.ind_flush()?;
        self.flush_bitmaps()?;
        self.barrier()?;
        self.flush_bitmaps()?;
        self.write_super()?;
        self.disk.flush()
    }
}

impl<D: BlockIo> Ext2<D> {
    /// Mounts the ext2 volume on `dev`, its state behind a spinlock. Unless
    /// read-only, the volume is marked "not clean" on disk until
    /// [`Ext2::unmount`].
    pub fn mount(dev: D, clock: Arc<dyn Clock>, opts: MountOptions) -> core::result::Result<Ext2<D>, MountError> {
        Ext2::mount_with_lock(dev, clock, opts, RawSpinLock::new())
    }
}

impl<D: BlockIo, L: RawLock> Ext2<D, L> {
    /// [`Ext2::mount`] with the volume's state behind `lock`, which every
    /// operation holds while it runs, disk I/O included: a kernel passes
    /// its sleeping mutex, so threads waiting for the volume sleep while the
    /// holder waits for the disk.
    pub fn mount_with_lock(
        dev: D,
        clock: Arc<dyn Clock>,
        opts: MountOptions,
        lock: L,
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
        let first_data = sb.first_data_block() as u64;
        if first_data != (bs == 1024) as u64 {
            return Err(MountError::Corrupt("first data block"));
        }
        let bpg = sb.blocks_per_group() as u64;
        let ipg = sb.inodes_per_group();
        let bits = 8 * bs as u64;
        if bpg < 8 || bpg > bits || ipg == 0 || ipg as u64 > bits {
            return Err(MountError::Corrupt("group size"));
        }
        let isz = sb.inode_size() as usize;
        if !isz.is_power_of_two() || isz < GOOD_OLD_INODE_SIZE as usize || isz > bs {
            return Err(MountError::Corrupt("inode size"));
        }
        let blocks = sb.blocks_count() as u64;
        if blocks <= first_data {
            return Err(MountError::Corrupt("block count"));
        }
        let groups = (blocks - first_data).div_ceil(bpg);
        let inodes = sb.inodes_count();
        let first_ino = sb.first_ino();
        if inodes as u64 > groups * ipg as u64 || first_ino <= ROOT_INO || first_ino > inodes {
            return Err(MountError::Corrupt("inode count"));
        }
        disk.set_block_size(bs);
        if blocks > disk.capacity() {
            return Err(MountError::TooSmall);
        }
        let compat = sb.feature_compat();
        let ro_compat = sb.feature_ro_compat();
        let gdt_blocks = (groups * DESC_SIZE as u64).div_ceil(bs as u64);
        // `s_reserved_gdt_blocks`, with `COMPAT_RESIZE_INODE`.
        let reserved_gdt = if compat & 0x10 != 0 { le16(&sb.raw, 206) as u64 } else { 0 };
        if 1 + gdt_blocks + reserved_gdt >= bpg {
            return Err(MountError::Corrupt("descriptor table size"));
        }
        let itable_blocks = (ipg as u64 * isz as u64).div_ceil(bs as u64);
        let mut gdt_raw = vec![0u8; gdt_blocks as usize * bs];
        disk.read(first_data + 1, &mut gdt_raw).map_err(MountError::Io)?;
        let mut descs = Vec::with_capacity(groups as usize);
        for g in 0..groups as usize {
            let d = GroupDesc::decode(&gdt_raw[g * DESC_SIZE..(g + 1) * DESC_SIZE]);
            let inside = |b: u32| (b as u64) >= first_data && (b as u64) < blocks;
            if !inside(d.block_bitmap)
                || !inside(d.inode_bitmap)
                || !inside(d.inode_table)
                || d.inode_table as u64 + itable_blocks > blocks
                || d.free_blocks as u64 > bpg
                || d.free_inodes as u32 > ipg
            {
                return Err(MountError::Corrupt("group descriptor"));
            }
            descs.push(d);
        }
        let ptrs = (bs / 4) as u64;
        let large_file = ro_compat & RO_COMPAT_LARGE_FILE != 0;
        let map_blocks = 12 + ptrs + ptrs * ptrs + ptrs * ptrs * ptrs;
        let mut max_size = (map_blocks * bs as u64).min(u32::MAX as u64 * 512);
        if rev == GOOD_OLD_REV {
            max_size = max_size.min(i32::MAX as u64);
        }
        let creator = sb.creator_os();
        let geo = Geometry {
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
            itable_blocks,
            gdt_blocks,
            reserved_gdt,
            ptrs,
            ifmt: InodeFormat {
                size: isz,
                large_size: rev >= DYNAMIC_REV,
                id_high: creator == OS_LINUX || creator == OS_HURD,
            },
            spb: (bs / 512) as u32,
        };
        let read_only = opts.read_only || disk.dev.read_only() || ro_compat & !RO_COMPAT_SUPPORTED != 0;
        let now = clock.now();
        let state = State {
            was_clean: sb.state() & (STATE_VALID | STATE_ERROR) == STATE_VALID,
            sb,
            gdt_raw,
            groups: descs,
            read_only,
            ordered: opts.ordered,
            unmounted: false,
            meta_dirty: false,
            features_dirty: false,
            large_file,
            max_size,
            pins: BTreeMap::new(),
            pending_blocks: Vec::new(),
            pending_inodes: Vec::new(),
            fresh_unbarriered: false,
            block_bitmap: None,
            inode_bitmap: None,
            next_generation: now.sec as u32 ^ now.nsec,
        };
        let fs = Ext2 { disk, geo, clock, errors: AtomicBool::new(false), state: Lock::with_raw(lock, state) };
        {
            let mut st = fs.state.lock();
            let mut ctx = fs.ctx(&mut st);
            match ctx.read_inode(ROOT_INO) {
                Ok(root) if root.is_dir() && root.links > 0 => {}
                Ok(_) => return Err(MountError::Corrupt("root directory")),
                Err(e) => return Err(MountError::Io(e)),
            }
            if !ctx.st.read_only {
                let sb = &mut ctx.st.sb;
                sb.set_state(sb.state() & !STATE_VALID);
                sb.set_mnt_count(sb.mnt_count().wrapping_add(1));
                sb.set_mtime(encode_time(now, false).0);
                ctx.st.meta_dirty = true;
                ctx.write_super().map_err(MountError::Io)?;
                ctx.disk.flush().map_err(MountError::Io)?;
            }
        }
        Ok(fs)
    }

    pub(crate) fn ctx<'a>(&'a self, st: &'a mut State) -> Ctx<'a, D> {
        Ctx {
            disk: &self.disk,
            geo: &self.geo,
            st,
            now: self.clock.now(),
            errors: &self.errors,
            ind: bmap::IndCache::default(),
        }
    }

    /// Runs a read-only operation.
    pub(crate) fn read_op<R>(&self, f: impl FnOnce(&mut Ctx<'_, D>) -> Result<R>) -> Result<R> {
        let mut st = self.state.lock();
        let mut ctx = self.ctx(&mut st);
        f(&mut ctx)
    }

    /// Runs an operation that may change the volume, then writes back what
    /// it left in its caches.
    pub(crate) fn write_op<R>(&self, f: impl FnOnce(&mut Ctx<'_, D>) -> Result<R>) -> Result<R> {
        let mut st = self.state.lock();
        if st.read_only {
            return Err(Errno::EROFS);
        }
        let mut ctx = self.ctx(&mut st);
        let r = f(&mut ctx);
        let flushed = ctx.ind_flush().and_then(|()| ctx.flush_bitmaps());
        match (r, flushed) {
            (Ok(v), Ok(())) => Ok(v),
            (Err(e), _) | (Ok(_), Err(e)) => Err(e),
        }
    }

    /// Writes everything back and makes it durable (the volume stays marked
    /// "not clean" while mounted).
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
        let pinned: Vec<u32> = ctx.st.pins.keys().copied().collect();
        ctx.st.pins.clear();
        for ino in pinned {
            let i = ctx.read_inode(ino)?;
            if i.mode != 0 && i.links == 0 && i.dtime == 0 {
                ctx.evict(ino, i)?;
            }
        }
        ctx.sync()?;
        let sb = &mut ctx.st.sb;
        sb.set_state(sb.state() | STATE_VALID);
        ctx.st.meta_dirty = true;
        ctx.write_super()?;
        ctx.disk.flush()?;
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
        let pending: u64 = st.pending_blocks.iter().map(|r| r.1).sum();
        FsStats {
            block_size: self.geo.bs as u32,
            blocks: self.geo.blocks,
            free_blocks: free_blocks + pending,
            inodes: self.geo.inodes as u64,
            free_inodes: free_inodes + st.pending_inodes.len() as u64,
            read_only: st.read_only,
            was_clean: st.was_clean,
            errors: self.errors.load(Ordering::Relaxed),
            uuid: st.sb.uuid(),
            label: st.sb.volume_name(),
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
