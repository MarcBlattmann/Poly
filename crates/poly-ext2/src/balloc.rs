//! Block and inode allocation, and the order in which metadata reaches the
//! disk.
//!
//! # Allocation
//!
//! Each group has a block bitmap and an inode bitmap (bit set: in use) and
//! counts of free blocks and inodes in its descriptor; the superblock's
//! totals are the sums, written with the descriptors at sync. Policies:
//!
//! - **Blocks** go to the first free block at or after a goal (the block
//!   after the file's previous block, or the start of its inode's group),
//!   then in the following groups, wrapping around.
//! - **Directories** spread: the group with the fewest directories among
//!   those with at least the average number of free inodes and free blocks.
//! - **Other inodes** stay in their directory's group if it has room, else
//!   go to a group found by quadratic probing, else to any group.
//!
//! A bitmap is never trusted blindly: a "free" bit for a block that is the
//! group's own metadata, or a count saying free where the bitmap is full,
//! marks the volume as having errors and is skipped.
//!
//! # Write ordering without a journal
//!
//! A crash may leave any subset of the writes since the last flush on disk.
//! e2fsck rebuilds bitmaps and counts from the inodes, so those may be
//! stale; what it cannot undo is two files sharing a block, a pointer to
//! garbage, or a file without a name. Three rules prevent them:
//!
//! 1. **No reuse before the old pointer is gone.** A freed block or inode
//!    goes on a pending list and becomes allocatable only after the next
//!    barrier, i.e. after the writes that removed every pointer to it are
//!    durable. ([`Ctx::free_blocks`], [`Ctx::free_inode`].)
//! 2. **No pointer to an uninitialised block.** A newly allocated indirect,
//!    directory or symbolic link block is written as "fresh"; before any
//!    structure that may point to one is written ([`Ctx::write_meta`]:
//!    inodes, existing indirect and directory blocks), a barrier makes the
//!    fresh blocks durable.
//! 3. **No moment without a name.** `rename` writes the new entry, takes a
//!    barrier, then removes the old one (see `ops.rs`).
//!
//! A barrier is a device flush (with the kernel's write-back buffer cache
//! below, a `sync` of the disk) followed by releasing the pending frees.
//! Data blocks are not ordered (as with ext2 on Linux, a crash may show
//! stale data in a block that was being allocated). New inodes need no
//! barrier before their directory entry: an entry pointing at an inode that
//! did not reach the disk points at a free (zeroed or deleted) inode, which
//! e2fsck removes.

use crate::layout::{has_super, le32};
use crate::{BitmapBuf, BlockIo, Ctx};
use alloc::vec::Vec;
use poly_vfs::{Errno, Result};

/// Pending block runs beyond which a barrier is taken at once, so a big
/// deletion does not hold an unbounded list.
const PENDING_LIMIT: usize = 4096;

/// The first clear bit in `bm` in `[start, end)`.
fn find_zero(bm: &[u8], start: usize, end: usize) -> Option<usize> {
    let mut i = start;
    while i < end {
        if i.is_multiple_of(8) && bm[i / 8] == 0xff {
            i += 8;
            continue;
        }
        if bm[i / 8] & (1 << (i % 8)) == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn bit(bm: &[u8], i: usize) -> bool {
    bm[i / 8] & (1 << (i % 8)) != 0
}

fn set_bit(bm: &mut [u8], i: usize, v: bool) {
    if v {
        bm[i / 8] |= 1 << (i % 8);
    } else {
        bm[i / 8] &= !(1 << (i % 8));
    }
}

impl<D: BlockIo> Ctx<'_, D> {
    // ----- ordering -----

    /// Writes a block that may point to fresh blocks: makes the fresh ones
    /// durable first (rule 2).
    pub fn write_meta(&mut self, block: u64, data: &[u8]) -> Result<()> {
        self.ind_write_fresh()?;
        if self.st.fresh_unbarriered {
            self.barrier()?;
        }
        self.disk.write(block, data)
    }

    /// [`Ctx::write_meta`] for part of a block, at a byte offset.
    pub fn write_meta_bytes(&mut self, at: u64, data: &[u8]) -> Result<()> {
        self.ind_write_fresh()?;
        if self.st.fresh_unbarriered {
            self.barrier()?;
        }
        self.disk.write_bytes(at, data)
    }

    /// Writes a newly allocated metadata block nothing on disk points to yet.
    pub fn write_fresh(&mut self, block: u64, data: &[u8]) -> Result<()> {
        self.disk.write(block, data)?;
        if self.st.ordered {
            self.st.fresh_unbarriered = true;
        }
        Ok(())
    }

    /// Makes everything written so far durable (an ordering point: nothing
    /// written later reaches the disk before it), then lets pending frees
    /// be reused. Without ordering, only releases them.
    pub fn barrier(&mut self) -> Result<()> {
        if self.st.ordered {
            self.disk.flush()?;
        }
        self.st.fresh_unbarriered = false;
        self.release_pending()
    }

    // ----- bitmaps -----

    fn load_bitmap(&mut self, block: u64, inode: bool) -> Result<()> {
        let slot = if inode { &self.st.inode_bitmap } else { &self.st.block_bitmap };
        if slot.as_ref().is_some_and(|b| b.block == block) {
            return Ok(());
        }
        self.flush_bitmap(inode)?;
        let data = self.read_block(block)?;
        let slot = if inode { &mut self.st.inode_bitmap } else { &mut self.st.block_bitmap };
        *slot = Some(BitmapBuf { block, data, dirty: false });
        Ok(())
    }

    fn bitmap(&mut self, inode: bool) -> &mut BitmapBuf {
        let slot = if inode { &mut self.st.inode_bitmap } else { &mut self.st.block_bitmap };
        slot.as_mut().expect("bitmap loaded")
    }

    fn flush_bitmap(&mut self, inode: bool) -> Result<()> {
        let slot = if inode { &mut self.st.inode_bitmap } else { &mut self.st.block_bitmap };
        if let Some(b) = slot
            && b.dirty
        {
            self.disk.write(b.block, &b.data)?;
            b.dirty = false;
        }
        Ok(())
    }

    /// Writes back the bitmap blocks this operation changed.
    pub fn flush_bitmaps(&mut self) -> Result<()> {
        self.flush_bitmap(false)?;
        self.flush_bitmap(true)
    }

    // ----- blocks -----

    /// Allocates a block near `goal`. `ENOSPC` when there is none (after
    /// letting pending frees go).
    pub fn alloc_block(&mut self, goal: u64) -> Result<u64> {
        if let Some(b) = self.try_alloc_block(goal)? {
            return Ok(b);
        }
        if !self.st.pending_blocks.is_empty() {
            self.barrier()?;
            if let Some(b) = self.try_alloc_block(goal)? {
                return Ok(b);
            }
        }
        Err(Errno::ENOSPC)
    }

    fn try_alloc_block(&mut self, goal: u64) -> Result<Option<u64>> {
        let geo = self.geo;
        let goal = if goal < geo.first_data || goal >= geo.blocks { geo.first_data } else { goal };
        let g0 = geo.group_of_block(goal);
        let goal_bit = (goal - geo.group_start(g0)) as usize;
        for i in 0..=geo.groups {
            let g = (g0 + i) % geo.groups;
            let (start, end) = match i {
                0 => (goal_bit, geo.blocks_in_group(g) as usize),
                _ if i == geo.groups => (0, goal_bit),
                _ => (0, geo.blocks_in_group(g) as usize),
            };
            if start >= end || self.st.groups[g as usize].free_blocks == 0 {
                continue;
            }
            if let Some(b) = self.alloc_in_group(g, start, end)? {
                return Ok(Some(b));
            }
        }
        Ok(None)
    }

    fn alloc_in_group(&mut self, g: u32, mut start: usize, end: usize) -> Result<Option<u64>> {
        let bm_block = self.st.groups[g as usize].block_bitmap as u64;
        self.load_bitmap(bm_block, false)?;
        loop {
            let found = find_zero(&self.bitmap(false).data, start, end);
            let Some(bit) = found else {
                if start == 0 && end == self.geo.blocks_in_group(g) as usize {
                    // The descriptor said there was room.
                    self.corrupt();
                    self.st.groups[g as usize].free_blocks = 0;
                    self.st.meta_dirty = true;
                }
                return Ok(None);
            };
            let bm = self.bitmap(false);
            set_bit(&mut bm.data, bit, true);
            bm.dirty = true;
            let b = self.geo.group_start(g) + bit as u64;
            if !self.block_ok(b) {
                // A metadata block marked free: keep it marked, look on.
                self.corrupt();
                start = bit + 1;
                continue;
            }
            let d = &mut self.st.groups[g as usize];
            d.free_blocks = d.free_blocks.saturating_sub(1);
            self.st.meta_dirty = true;
            return Ok(Some(b));
        }
    }

    /// Frees blocks whose pointers have been removed (the writes issued):
    /// they become allocatable after the next barrier.
    pub fn free_blocks(&mut self, blocks: &[u64]) -> Result<()> {
        for &b in blocks {
            match self.st.pending_blocks.last_mut() {
                Some((start, len)) if *start + *len == b => *len += 1,
                _ => self.st.pending_blocks.push((b, 1)),
            }
        }
        if !self.st.ordered || self.st.pending_blocks.len() > PENDING_LIMIT {
            self.barrier()?;
        }
        Ok(())
    }

    /// Frees inode `ino` (its record already written as deleted).
    pub fn free_inode(&mut self, ino: u32, dir: bool) -> Result<()> {
        self.st.pending_inodes.push((ino, dir));
        if !self.st.ordered {
            self.barrier()?;
        }
        Ok(())
    }

    fn release_pending(&mut self) -> Result<()> {
        let mut runs = core::mem::take(&mut self.st.pending_blocks);
        runs.sort_unstable();
        for (start, len) in runs {
            for b in start..start + len {
                if !self.block_ok(b) {
                    self.corrupt();
                    continue;
                }
                let g = self.geo.group_of_block(b);
                let bm_block = self.st.groups[g as usize].block_bitmap as u64;
                self.load_bitmap(bm_block, false)?;
                let i = (b - self.geo.group_start(g)) as usize;
                let bm = self.bitmap(false);
                if !bit(&bm.data, i) {
                    // Freed twice: two pointers to one block.
                    self.corrupt();
                    continue;
                }
                set_bit(&mut bm.data, i, false);
                bm.dirty = true;
                let d = &mut self.st.groups[g as usize];
                d.free_blocks = d.free_blocks.saturating_add(1);
                self.st.meta_dirty = true;
            }
        }
        let inodes = core::mem::take(&mut self.st.pending_inodes);
        for (ino, dir) in inodes {
            let g = self.geo.group_of_inode(ino);
            let bm_block = self.st.groups[g as usize].inode_bitmap as u64;
            self.load_bitmap(bm_block, true)?;
            let i = ((ino - 1) % self.geo.ipg) as usize;
            let bm = self.bitmap(true);
            if !bit(&bm.data, i) {
                self.corrupt();
                continue;
            }
            set_bit(&mut bm.data, i, false);
            bm.dirty = true;
            let d = &mut self.st.groups[g as usize];
            d.free_inodes = d.free_inodes.saturating_add(1);
            if dir {
                d.used_dirs = d.used_dirs.saturating_sub(1);
            }
            self.st.meta_dirty = true;
        }
        Ok(())
    }

    /// The first block of group `g` after its fixed metadata: a goal for
    /// the data of inodes there.
    pub fn group_data_goal(&self, g: u32) -> u64 {
        let d = &self.st.groups[g as usize];
        (d.inode_table as u64 + self.geo.itable_blocks).max(self.geo.group_start(g))
    }

    // ----- inodes -----

    /// The groups to try for a new inode, best first.
    fn inode_groups(&self, parent: u32, dir: bool) -> Vec<u32> {
        let n = self.geo.groups;
        let groups = &self.st.groups;
        let pg = self.geo.group_of_inode(parent).min(n - 1);
        let mut order = Vec::with_capacity(n as usize + 1);
        if dir {
            let free_i: u64 = groups.iter().map(|d| d.free_inodes as u64).sum();
            let free_b: u64 = groups.iter().map(|d| d.free_blocks as u64).sum();
            let (avg_i, avg_b) = (free_i / n as u64, free_b / n as u64);
            let best = (0..n)
                .map(|i| (pg + i) % n)
                .filter(|&g| {
                    let d = &groups[g as usize];
                    d.free_inodes > 0 && d.free_inodes as u64 >= avg_i && d.free_blocks as u64 >= avg_b
                })
                .min_by_key(|&g| (groups[g as usize].used_dirs, u16::MAX - groups[g as usize].free_blocks));
            order.extend(best);
        } else {
            let has_room = |g: u32| groups[g as usize].free_inodes > 0 && groups[g as usize].free_blocks > 0;
            if has_room(pg) {
                order.push(pg);
            } else {
                let mut step = 1u64;
                while step < n as u64 {
                    let g = ((pg as u64 + step) % n as u64) as u32;
                    if has_room(g) {
                        order.push(g);
                        break;
                    }
                    step *= 2;
                }
            }
        }
        order.extend((0..n).map(|i| (pg + i) % n));
        order
    }

    /// Allocates an inode for a new file (`dir`: a directory) in directory
    /// `parent`.
    pub fn alloc_inode(&mut self, parent: u32, dir: bool) -> Result<u32> {
        if let Some(ino) = self.try_alloc_inode(parent, dir)? {
            return Ok(ino);
        }
        if !self.st.pending_inodes.is_empty() {
            self.barrier()?;
            if let Some(ino) = self.try_alloc_inode(parent, dir)? {
                return Ok(ino);
            }
        }
        Err(Errno::ENOSPC)
    }

    fn try_alloc_inode(&mut self, parent: u32, dir: bool) -> Result<Option<u32>> {
        let geo = self.geo;
        let mut tried = Vec::new();
        for g in self.inode_groups(parent, dir) {
            if tried.contains(&g) || self.st.groups[g as usize].free_inodes == 0 {
                continue;
            }
            tried.push(g);
            let first = g as u64 * geo.ipg as u64;
            // Bits for this group that are real inodes at or above the first
            // non-reserved one.
            let start = (geo.first_ino as u64 - 1).saturating_sub(first).min(geo.ipg as u64) as usize;
            let end = (geo.inodes as u64).saturating_sub(first).min(geo.ipg as u64) as usize;
            if start >= end {
                continue;
            }
            let bm_block = self.st.groups[g as usize].inode_bitmap as u64;
            self.load_bitmap(bm_block, true)?;
            let Some(i) = find_zero(&self.bitmap(true).data, start, end) else {
                self.corrupt();
                self.st.groups[g as usize].free_inodes = 0;
                self.st.meta_dirty = true;
                continue;
            };
            let bm = self.bitmap(true);
            set_bit(&mut bm.data, i, true);
            bm.dirty = true;
            let d = &mut self.st.groups[g as usize];
            d.free_inodes -= 1;
            if dir {
                d.used_dirs = d.used_dirs.saturating_add(1);
            }
            self.st.meta_dirty = true;
            return Ok(Some((first + i as u64 + 1) as u32));
        }
        Ok(None)
    }

    /// Releases an extended attribute block (`i_file_acl`) a deleted inode
    /// held: decrements its reference count, frees it at zero. Returns the
    /// block if it is to be freed.
    pub fn release_xattr(&mut self, block: u32) -> Result<Option<u64>> {
        let b = block as u64;
        if !self.block_ok(b) {
            return Err(self.corrupt());
        }
        let mut data = self.read_block(b)?;
        if le32(&data, 0) != crate::layout::XATTR_MAGIC {
            return Err(self.corrupt());
        }
        let refs = le32(&data, 4);
        if refs > 1 {
            crate::layout::put32(&mut data, 4, refs - 1);
            self.disk.write(b, &data)?;
            return Ok(None);
        }
        Ok(Some(b))
    }

    /// Whether group `g` keeps a superblock copy (for tests and format).
    #[allow(dead_code)]
    pub fn group_has_super(&self, g: u32) -> bool {
        has_super(g, self.geo.sparse_super)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_search() {
        let mut bm = [0xffu8; 4];
        assert_eq!(find_zero(&bm, 0, 32), None);
        set_bit(&mut bm, 19, false);
        assert_eq!(find_zero(&bm, 0, 32), Some(19));
        assert_eq!(find_zero(&bm, 19, 32), Some(19));
        assert_eq!(find_zero(&bm, 20, 32), None);
        assert_eq!(find_zero(&bm, 0, 19), None);
        assert!(!bit(&bm, 19));
        set_bit(&mut bm, 19, true);
        assert!(bit(&bm, 19));
        assert_eq!(find_zero(&[0u8; 2], 3, 16), Some(3));
    }
}
