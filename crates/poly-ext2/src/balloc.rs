//! Block and inode allocation, and the order in which metadata reaches the
//! disk.
//!
//! # Allocation
//!
//! Each group has a block bitmap and an inode bitmap (bit set: in use) and
//! counts of free blocks and inodes in its descriptor; the superblock's
//! totals are the sums, written with the descriptors. Policies:
//!
//! - **Blocks** go to the first free block at or after a goal (the block
//!   after the file's previous block, or the start of its inode's group),
//!   then in the following groups, wrapping around.
//! - **Directories** spread: the group with the fewest directories among
//!   those with at least the average number of free inodes and free blocks.
//! - **Other inodes** stay in their directory's group if it has room, else
//!   go to a group found by quadratic probing, else to any group.
//!
//! With group descriptor checksums (`uninit_bg`, `metadata_csum`) a group
//! may say its bitmaps were never written (`BLOCK_UNINIT`, `INODE_UNINIT`):
//! they are worked out instead (the group's own fixed metadata in use,
//! nothing else), and written, with the flag cleared, when first changed.
//! `bg_itable_unused` shrinks when an inode past the used part of the
//! table is handed out. Bitmaps carry CRC-32C checksums in the descriptor
//! with `metadata_csum`.
//!
//! A bitmap is never trusted blindly: a "free" bit for a block that is
//! fixed metadata, or a count saying free where the bitmap is full, marks
//! the volume as having errors and is skipped.
//!
//! # Freeing and reuse
//!
//! A freed block or inode is cleared in its bitmap at once but *held*: not
//! handed out again until the removal of every pointer to it is durable
//! (the next barrier without a journal, the commit of the running
//! transaction with one). Otherwise a crash could leave two owners of one
//! block. With a journal, freeing a block that was journaled as metadata
//! also writes a revoke record, so replay never copies old metadata over
//! what the block holds next.
//!
//! # Write ordering without a journal
//!
//! A crash may leave any subset of the writes since the last flush on disk.
//! e2fsck rebuilds bitmaps and counts from the inodes, so those may be
//! stale; what it cannot undo is two files sharing a block, a pointer to
//! garbage, or a file without a name. Three rules prevent them:
//!
//! 1. **No reuse before the old pointer is gone** (holding, above).
//! 2. **No pointer to an uninitialised block.** A newly allocated indirect,
//!    extent, directory or symbolic link block is written as "fresh";
//!    before any structure that may point to one is written
//!    ([`Ctx::write_meta`]: inodes, existing indirect, extent and directory
//!    blocks), a barrier makes the fresh blocks durable.
//! 3. **No moment without a name.** `rename` writes the new entry, takes a
//!    barrier, then removes the old one (see `ops.rs`).
//!
//! A barrier is a device flush (with the kernel's write-back buffer cache
//! below, a `sync` of the disk) followed by releasing the held frees. Data
//! blocks are not ordered (as with ext2 on Linux, a crash may show stale
//! data in a block that was being allocated). With a journal none of this
//! is needed: metadata goes into the running transaction, which commits as
//! a whole, and barriers only matter to the commit (`journal.rs`).

use crate::layout::{BG_BLOCK_UNINIT, BG_INODE_UNINIT, XATTR_MAGIC, le32, put32};
use crate::{BitmapBuf, BlockIo, Ctx, csum};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Errno, Result};

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

    /// Writes a metadata block that may point to fresh blocks: makes the
    /// fresh ones durable first (rule 2). With a journal: into the running
    /// transaction.
    pub fn write_meta(&mut self, block: u64, data: &[u8]) -> Result<()> {
        if self.st.journal.is_some() {
            self.journal_put(block, data.to_vec());
            return Ok(());
        }
        self.ind_write_fresh()?;
        if self.st.fresh_unbarriered {
            self.barrier()?;
        }
        self.disk.write(block, data)
    }

    /// Writes a newly allocated metadata block nothing on disk points to yet.
    pub fn write_fresh(&mut self, block: u64, data: &[u8]) -> Result<()> {
        if self.st.journal.is_some() {
            self.journal_put(block, data.to_vec());
            return Ok(());
        }
        self.disk.write(block, data)?;
        if self.st.ordered {
            self.st.fresh_unbarriered = true;
        }
        Ok(())
    }

    /// Without a journal: makes everything written so far durable (an
    /// ordering point: nothing written later reaches the disk before it),
    /// then lets held frees be reused. Without ordering, only releases them.
    /// With a journal: nothing (the commit is the only ordering point).
    pub fn barrier(&mut self) -> Result<()> {
        if self.st.journal.is_some() {
            return Ok(());
        }
        if self.st.ordered {
            self.disk.flush()?;
        }
        self.st.fresh_unbarriered = false;
        self.release_held();
        Ok(())
    }

    /// Lets the held frees be reused (their removal is durable).
    pub fn release_held(&mut self) {
        self.st.held_blocks.clear();
        self.st.held_inodes.clear();
    }

    // ----- bitmaps -----

    /// A group's bitmap as it would be had it been written: for blocks,
    /// the fixed metadata inside the group in use; for inodes, all free.
    /// Bits past the group's end are set.
    fn synthetic_bitmap(&self, g: u32, inode: bool) -> Vec<u8> {
        let geo = self.geo;
        let mut bm = vec![0u8; geo.bs];
        let end_bit = if inode {
            geo.ipg as usize
        } else {
            let start = geo.group_start(g);
            let n = geo.blocks_in_group(g);
            let i = geo.fixed.partition_point(|r| r.1 <= start);
            for &(s, e) in &geo.fixed[i..] {
                if s >= start + n {
                    break;
                }
                for b in s.max(start)..e.min(start + n) {
                    set_bit(&mut bm, (b - start) as usize, true);
                }
            }
            n as usize
        };
        for i in end_bit..8 * geo.bs {
            set_bit(&mut bm, i, true);
        }
        bm
    }

    fn load_bitmap(&mut self, g: u32, inode: bool) -> Result<()> {
        let slot = if inode { &self.st.inode_bitmap } else { &self.st.block_bitmap };
        if slot.as_ref().is_some_and(|b| b.group == g) {
            return Ok(());
        }
        self.flush_bitmap(inode)?;
        let d = self.st.groups[g as usize];
        let uninit = if inode { BG_INODE_UNINIT } else { BG_BLOCK_UNINIT };
        let data = if self.geo.gdt_csum() && d.flags & uninit != 0 {
            self.synthetic_bitmap(g, inode)
        } else {
            let block = if inode { d.inode_bitmap } else { d.block_bitmap };
            let data = self.read_block(block)?;
            if let Some(seed) = self.geo.meta_seed() {
                let (bits, want) =
                    if inode { (self.geo.ipg as u64, d.inode_bitmap_csum) } else { (self.geo.bpg, d.block_bitmap_csum) };
                let mut got = csum::bitmap(seed, &data, bits);
                if self.geo.desc_size < crate::layout::DESC_SIZE_64 {
                    got &= 0xFFFF;
                }
                if got != want {
                    return Err(self.bad_csum());
                }
            }
            data
        };
        let slot = if inode { &mut self.st.inode_bitmap } else { &mut self.st.block_bitmap };
        *slot = Some(BitmapBuf { group: g, data, dirty: false });
        Ok(())
    }

    fn bitmap(&mut self, inode: bool) -> &mut BitmapBuf {
        let slot = if inode { &mut self.st.inode_bitmap } else { &mut self.st.block_bitmap };
        slot.as_mut().expect("bitmap loaded")
    }

    /// Writes the bitmap back if it changed: its checksum into the group
    /// descriptor, the group's "uninitialised" flag cleared.
    fn flush_bitmap(&mut self, inode: bool) -> Result<()> {
        let slot = if inode { &mut self.st.inode_bitmap } else { &mut self.st.block_bitmap };
        let Some(b) = slot.as_mut().filter(|b| b.dirty) else { return Ok(()) };
        b.dirty = false;
        let g = b.group;
        let data = b.data.clone();
        let geo = self.geo;
        let d = &mut self.st.groups[g as usize];
        let block = if inode {
            d.flags &= !BG_INODE_UNINIT;
            d.inode_bitmap
        } else {
            d.flags &= !BG_BLOCK_UNINIT;
            d.block_bitmap
        };
        if let Some(seed) = geo.meta_seed() {
            // 32-byte descriptors keep the low halves only.
            let mask = if geo.desc_size < crate::layout::DESC_SIZE_64 { 0xFFFF } else { u32::MAX };
            if inode {
                d.inode_bitmap_csum = csum::bitmap(seed, &data, geo.ipg as u64) & mask;
            } else {
                d.block_bitmap_csum = csum::bitmap(seed, &data, geo.bpg) & mask;
            }
        }
        self.st.dirty_groups.insert(g);
        self.write_meta(block, &data)
    }

    /// Writes back the bitmap blocks this operation changed.
    pub fn flush_bitmaps(&mut self) -> Result<()> {
        self.flush_bitmap(false)?;
        self.flush_bitmap(true)
    }

    // ----- blocks -----

    /// Allocates a block near `goal`. `ENOSPC` when there is none (after
    /// letting held frees go, without a journal).
    pub fn alloc_block(&mut self, goal: u64) -> Result<u64> {
        if let Some(b) = self.try_alloc_block(goal)? {
            return Ok(b);
        }
        if !self.st.held_blocks.is_empty() && self.st.journal.is_none() {
            self.barrier()?;
            if let Some(b) = self.try_alloc_block(goal)? {
                return Ok(b);
            }
        }
        Err(Errno::ENOSPC)
    }

    /// Free blocks in the descriptors that are not held.
    pub fn free_block_count(&self) -> u64 {
        let free: u64 = self.st.groups.iter().map(|d| d.free_blocks as u64).sum();
        let held: u64 = self.st.held_blocks.iter().map(|(s, e)| e - s).sum();
        free.saturating_sub(held)
    }

    /// The end of the held run containing `b`, if it is held.
    fn held_until(&self, b: u64) -> Option<u64> {
        self.st.held_blocks.range(..=b).next_back().filter(|(_, e)| **e > b).map(|(_, e)| *e)
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
        self.load_bitmap(g, false)?;
        let base = self.geo.group_start(g);
        let mut saw_held = false;
        loop {
            let found = find_zero(&self.bitmap(false).data, start, end);
            let Some(bit) = found else {
                if start == 0 && end == self.geo.blocks_in_group(g) as usize && !saw_held {
                    // The descriptor said there was room.
                    self.corrupt();
                    self.st.groups[g as usize].free_blocks = 0;
                    self.st.dirty_groups.insert(g);
                }
                return Ok(None);
            };
            let b = base + bit as u64;
            if let Some(until) = self.held_until(b) {
                saw_held = true;
                start = (until - base) as usize;
                continue;
            }
            let bm = self.bitmap(false);
            set_bit(&mut bm.data, bit, true);
            bm.dirty = true;
            if !self.block_ok(b) {
                // A metadata block marked free: keep it marked, look on.
                self.corrupt();
                start = bit + 1;
                continue;
            }
            let d = &mut self.st.groups[g as usize];
            d.free_blocks = d.free_blocks.saturating_sub(1);
            self.st.dirty_groups.insert(g);
            self.st.meta_dirty = true;
            return Ok(Some(b));
        }
    }

    /// Frees blocks whose pointers have been removed (the writes issued):
    /// cleared in the bitmaps now, reusable after the next barrier or
    /// commit.
    pub fn free_blocks(&mut self, blocks: &[u64]) -> Result<()> {
        for &b in blocks {
            if !self.block_ok(b) {
                self.corrupt();
                continue;
            }
            self.journal_forget(b);
            let g = self.geo.group_of_block(b);
            self.load_bitmap(g, false)?;
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
            self.st.dirty_groups.insert(g);
            self.st.meta_dirty = true;
            // Hold it: extend the run ending at `b`, or start one.
            let prev = self.st.held_blocks.range(..=b).next_back().map(|(s, e)| (*s, *e));
            match prev {
                Some((s, e)) if e == b => {
                    self.st.held_blocks.insert(s, b + 1);
                }
                Some((_, e)) if e > b => {}
                _ => {
                    self.st.held_blocks.insert(b, b + 1);
                }
            }
        }
        if !self.st.ordered && self.st.journal.is_none() {
            self.barrier()?;
        }
        Ok(())
    }

    /// Frees inode `ino` (its record already written as deleted).
    pub fn free_inode(&mut self, ino: u32, dir: bool) -> Result<()> {
        let g = self.geo.group_of_inode(ino);
        self.load_bitmap(g, true)?;
        let i = ((ino - 1) % self.geo.ipg) as usize;
        let bm = self.bitmap(true);
        if !bit(&bm.data, i) {
            self.corrupt();
            return Ok(());
        }
        set_bit(&mut bm.data, i, false);
        bm.dirty = true;
        let d = &mut self.st.groups[g as usize];
        d.free_inodes = d.free_inodes.saturating_add(1);
        if dir {
            d.used_dirs = d.used_dirs.saturating_sub(1);
        }
        self.st.dirty_groups.insert(g);
        self.st.meta_dirty = true;
        self.st.held_inodes.insert(ino);
        if !self.st.ordered && self.st.journal.is_none() {
            self.barrier()?;
        }
        Ok(())
    }

    /// A goal for the data of inodes in group `g`: after the group's inode
    /// table when that is in the group, else the group's start.
    pub fn group_data_goal(&self, g: u32) -> u64 {
        let d = &self.st.groups[g as usize];
        let start = self.geo.group_start(g);
        let after = d.inode_table + self.geo.itable_blocks;
        if after > start && after < start + self.geo.blocks_in_group(g) { after } else { start }
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
                .min_by_key(|&g| (groups[g as usize].used_dirs, u32::MAX - groups[g as usize].free_blocks));
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
        if !self.st.held_inodes.is_empty() && self.st.journal.is_none() {
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
            let mut start = (geo.first_ino as u64 - 1).saturating_sub(first).min(geo.ipg as u64) as usize;
            let end = (geo.inodes as u64).saturating_sub(first).min(geo.ipg as u64) as usize;
            self.load_bitmap(g, true)?;
            let found = loop {
                if start >= end {
                    break None;
                }
                match find_zero(&self.bitmap(true).data, start, end) {
                    Some(i) if self.st.held_inodes.contains(&((first + i as u64 + 1) as u32)) => start = i + 1,
                    other => break other,
                }
            };
            let Some(i) = found else {
                let held = (first..first + geo.ipg as u64).any(|n| self.st.held_inodes.contains(&((n + 1) as u32)));
                if !held {
                    self.corrupt();
                    self.st.groups[g as usize].free_inodes = 0;
                    self.st.dirty_groups.insert(g);
                }
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
            if geo.gdt_csum() {
                let used = geo.ipg - d.itable_unused.min(geo.ipg);
                if i as u32 >= used {
                    d.itable_unused = geo.ipg - i as u32 - 1;
                }
            }
            self.st.dirty_groups.insert(g);
            self.st.meta_dirty = true;
            return Ok(Some((first + i as u64 + 1) as u32));
        }
        Ok(None)
    }

    /// Releases an extended attribute block (`i_file_acl`) a deleted inode
    /// held: decrements its reference count, frees it at zero. Returns the
    /// block if it is to be freed.
    pub fn release_xattr(&mut self, block: u64) -> Result<Option<u64>> {
        let b = block;
        if !self.block_ok(b) {
            return Err(self.corrupt());
        }
        let mut data = self.read_block(b)?;
        if le32(&data, 0) != XATTR_MAGIC {
            return Err(self.corrupt());
        }
        let refs = le32(&data, 4);
        if refs > 1 {
            put32(&mut data, 4, refs - 1);
            if let Some(seed) = self.geo.meta_seed() {
                let c = csum::xattr_block(seed, b, &data);
                put32(&mut data, 0x10, c);
            }
            self.write_meta(b, &data)?;
            return Ok(None);
        }
        Ok(Some(b))
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
