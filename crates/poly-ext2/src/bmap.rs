//! Block maps: which disk block holds a file's logical block.
//!
//! `i_block[0..12]` point at the first twelve blocks; `i_block[12]`,
//! `[13]` and `[14]` at a single, double and triple indirect block, each a
//! block full of 32-bit block numbers. A zero anywhere is a hole: the
//! blocks below it read as zeros and are allocated when written.
//!
//! An operation keeps the indirect blocks it works on in an [`IndCache`]
//! and writes them back at its end (or when the cache fills): newly
//! allocated ones as "fresh" blocks first, then existing ones through
//! `write_meta`, which orders them after the fresh ones (see `balloc.rs`).

use crate::layout::{DIND_BLOCK, IND_BLOCK, Inode, NDIR_BLOCKS, TIND_BLOCK, le32, put32};
use crate::{BlockIo, Ctx};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Errno, Result};

/// Indirect blocks one operation keeps at most.
const IND_CACHE: usize = 8;

struct Ind {
    block: u64,
    data: Vec<u8>,
    dirty: bool,
    /// Allocated by this operation: nothing on disk points to it yet.
    fresh: bool,
}

#[derive(Default)]
pub(crate) struct IndCache {
    items: Vec<Ind>,
}

/// Where logical block `lblk` hangs: the `i_block` slot, then the index in
/// each indirect block on the way (`depth` of them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MapPath {
    pub slot: usize,
    pub depth: usize,
    pub idx: [usize; 3],
}

pub(crate) fn map_path(lblk: u64, p: u64) -> Option<MapPath> {
    if lblk < NDIR_BLOCKS as u64 {
        return Some(MapPath { slot: lblk as usize, depth: 0, idx: [0; 3] });
    }
    let mut l = lblk - NDIR_BLOCKS as u64;
    if l < p {
        return Some(MapPath { slot: IND_BLOCK, depth: 1, idx: [l as usize, 0, 0] });
    }
    l -= p;
    if l < p * p {
        return Some(MapPath { slot: DIND_BLOCK, depth: 2, idx: [(l / p) as usize, (l % p) as usize, 0] });
    }
    l -= p * p;
    if l < p * p * p {
        let idx = [(l / (p * p)) as usize, ((l / p) % p) as usize, (l % p) as usize];
        return Some(MapPath { slot: TIND_BLOCK, depth: 3, idx });
    }
    None
}

impl<D: BlockIo> Ctx<'_, D> {
    /// The cache index of indirect block `block`, read if needed.
    fn ind_get(&mut self, block: u64) -> Result<usize> {
        if let Some(i) = self.ind.items.iter().position(|it| it.block == block) {
            return Ok(i);
        }
        if !self.block_ok(block) {
            return Err(self.corrupt());
        }
        self.ind_make_room()?;
        let data = self.read_block(block)?;
        self.ind.items.push(Ind { block, data, dirty: false, fresh: false });
        Ok(self.ind.items.len() - 1)
    }

    /// A newly allocated indirect block, all zeros.
    fn ind_new(&mut self, block: u64) -> Result<usize> {
        self.ind_make_room()?;
        let data = vec![0u8; self.geo.bs];
        self.ind.items.push(Ind { block, data, dirty: true, fresh: true });
        Ok(self.ind.items.len() - 1)
    }

    fn ind_make_room(&mut self) -> Result<()> {
        if self.ind.items.len() >= IND_CACHE {
            self.ind_flush()?;
            self.ind.items.clear();
        }
        Ok(())
    }

    /// Writes the fresh indirect blocks that changed.
    pub fn ind_write_fresh(&mut self) -> Result<()> {
        for i in 0..self.ind.items.len() {
            let it = &self.ind.items[i];
            if it.dirty && it.fresh {
                self.disk.write(it.block, &it.data)?;
                self.ind.items[i].dirty = false;
                if self.st.ordered {
                    self.st.fresh_unbarriered = true;
                }
            }
        }
        Ok(())
    }

    /// Writes back every changed indirect block, fresh ones first.
    pub fn ind_flush(&mut self) -> Result<()> {
        self.ind_write_fresh()?;
        for i in 0..self.ind.items.len() {
            if self.ind.items[i].dirty {
                let block = self.ind.items[i].block;
                let data = core::mem::take(&mut self.ind.items[i].data);
                let r = self.write_meta(block, &data);
                self.ind.items[i].data = data;
                r?;
                self.ind.items[i].dirty = false;
            }
        }
        Ok(())
    }

    /// Forgets a block that is being freed.
    fn ind_forget(&mut self, block: u64) {
        self.ind.items.retain(|it| it.block != block);
    }

    fn ind_entry(&self, i: usize, at: usize) -> u64 {
        le32(&self.ind.items[i].data, at * 4) as u64
    }

    fn ind_set(&mut self, i: usize, at: usize, v: u64) {
        let it = &mut self.ind.items[i];
        put32(&mut it.data, at * 4, v as u32);
        it.dirty = true;
    }

    /// The disk block holding logical block `lblk`, `None` for a hole.
    pub fn bmap(&mut self, inode: &Inode, lblk: u64) -> Result<Option<u64>> {
        let Some(path) = map_path(lblk, self.geo.ptrs) else { return Ok(None) };
        let mut b = inode.block[path.slot] as u64;
        for level in 0..path.depth {
            if b == 0 {
                return Ok(None);
            }
            let i = self.ind_get(b)?;
            b = self.ind_entry(i, path.idx[level]);
        }
        if b == 0 {
            return Ok(None);
        }
        if !self.block_ok(b) {
            return Err(self.corrupt());
        }
        Ok(Some(b))
    }

    fn charge(&self, inode: &mut Inode) -> Result<()> {
        inode.blocks = inode.blocks.checked_add(self.geo.spb).ok_or(Errno::EFBIG)?;
        Ok(())
    }

    /// The disk block for logical block `lblk`, allocated (with any
    /// indirect blocks on the way) if it is a hole; `true` if it was. New
    /// blocks go near `goal`, which moves past them.
    pub fn bmap_alloc(&mut self, inode: &mut Inode, lblk: u64, goal: &mut u64) -> Result<(u64, bool)> {
        let path = map_path(lblk, self.geo.ptrs).ok_or(Errno::EFBIG)?;
        // Check the inode's block count can take what may be added.
        if inode.blocks as u64 + (path.depth as u64 + 1) * self.geo.spb as u64 > u32::MAX as u64 {
            return Err(Errno::EFBIG);
        }
        let top = inode.block[path.slot] as u64;
        if path.depth == 0 {
            if top != 0 {
                if !self.block_ok(top) {
                    return Err(self.corrupt());
                }
                return Ok((top, false));
            }
            let b = self.alloc_block(*goal)?;
            inode.block[path.slot] = b as u32;
            self.charge(inode)?;
            *goal = b + 1;
            return Ok((b, true));
        }
        let mut cur = if top == 0 {
            let b = self.alloc_block(*goal)?;
            inode.block[path.slot] = b as u32;
            self.charge(inode)?;
            *goal = b + 1;
            self.ind_new(b)?
        } else {
            self.ind_get(top)?
        };
        for level in 0..path.depth {
            let at = path.idx[level];
            let entry = self.ind_entry(cur, at);
            let last = level + 1 == path.depth;
            if entry != 0 {
                if last {
                    if !self.block_ok(entry) {
                        return Err(self.corrupt());
                    }
                    return Ok((entry, false));
                }
                cur = self.ind_get(entry)?;
                continue;
            }
            let parent = self.ind.items[cur].block;
            let b = self.alloc_block(*goal)?;
            *goal = b + 1;
            self.charge(inode)?;
            // The parent may have been written back and dropped meanwhile
            // (allocation can take a barrier, which does not touch the
            // cache, but be safe about indexes anyway).
            let p = self.ind_get(parent)?;
            self.ind_set(p, at, b);
            if last {
                return Ok((b, true));
            }
            cur = self.ind_new(b)?;
        }
        unreachable!("the loop returns at the last level")
    }

    /// Frees every block of the map at or past logical block `keep`,
    /// clearing the pointers (in `inode` and in cached indirect blocks,
    /// written back by the operation). The freed blocks are appended to
    /// `freed`; the caller hands them to `free_blocks` after writing the
    /// inode.
    pub fn truncate_map(&mut self, inode: &mut Inode, keep: u64, freed: &mut Vec<u64>) -> Result<()> {
        let p = self.geo.ptrs;
        for slot in (keep.min(NDIR_BLOCKS as u64) as usize)..NDIR_BLOCKS {
            let b = inode.block[slot] as u64;
            if b != 0 {
                self.release_one(inode, b, freed);
                inode.block[slot] = 0;
            }
        }
        let mut base = NDIR_BLOCKS as u64;
        for (slot, level) in [(IND_BLOCK, 1u32), (DIND_BLOCK, 2), (TIND_BLOCK, 3)] {
            let span = p.pow(level);
            let b = inode.block[slot] as u64;
            if b != 0 && keep < base + span && self.truncate_tree(inode, b, level, base, keep, freed)? {
                inode.block[slot] = 0;
            }
            base += span;
        }
        Ok(())
    }

    fn release_one(&mut self, inode: &mut Inode, b: u64, freed: &mut Vec<u64>) {
        if self.block_ok(b) {
            freed.push(b);
            self.ind_forget(b);
        } else {
            self.corrupt();
        }
        inode.blocks = inode.blocks.saturating_sub(self.geo.spb);
    }

    /// Frees the part of the subtree under indirect block `b` (`level` 1-3,
    /// covering logical blocks from `base`) at or past `keep`. Returns
    /// whether `b` itself was freed (nothing below it kept).
    fn truncate_tree(
        &mut self,
        inode: &mut Inode,
        b: u64,
        level: u32,
        base: u64,
        keep: u64,
        freed: &mut Vec<u64>,
    ) -> Result<bool> {
        if !self.block_ok(b) {
            // Drop the bad pointer without freeing anything under it.
            self.corrupt();
            return Ok(true);
        }
        let p = self.geo.ptrs;
        let child_span = p.pow(level - 1);
        let i = self.ind_get(b)?;
        let first = if keep <= base { 0 } else { ((keep - base).div_ceil(child_span)) as usize };
        // A child straddling `keep` is truncated, not freed.
        let straddle = if keep > base && !(keep - base).is_multiple_of(child_span) {
            Some(((keep - base) / child_span) as usize)
        } else {
            None
        };
        if let Some(s) = straddle
            && level > 1
        {
            let child = self.ind_entry(i, s);
            if child != 0 && self.truncate_tree(inode, child, level - 1, base + s as u64 * child_span, keep, freed)? {
                let i = self.ind_get(b)?;
                self.ind_set(i, s, 0);
            }
        }
        for at in first..p as usize {
            let i = self.ind_get(b)?;
            let child = self.ind_entry(i, at);
            if child == 0 {
                continue;
            }
            if level > 1 {
                self.free_tree(inode, child, level - 1, freed)?;
            } else {
                self.release_one(inode, child, freed);
            }
            let i = self.ind_get(b)?;
            self.ind_set(i, at, 0);
        }
        if keep <= base {
            self.release_one(inode, b, freed);
            return Ok(true);
        }
        Ok(false)
    }

    /// Frees a whole subtree.
    fn free_tree(&mut self, inode: &mut Inode, b: u64, level: u32, freed: &mut Vec<u64>) -> Result<()> {
        if !self.block_ok(b) {
            self.corrupt();
            return Ok(());
        }
        let data = match self.ind.items.iter().find(|it| it.block == b) {
            Some(it) => it.data.clone(),
            None => self.read_block(b)?,
        };
        for at in 0..self.geo.ptrs as usize {
            let child = le32(&data, at * 4) as u64;
            if child == 0 {
                continue;
            }
            if level > 1 {
                self.free_tree(inode, child, level - 1, freed)?;
            } else {
                self.release_one(inode, child, freed);
            }
        }
        self.release_one(inode, b, freed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let p = 256;
        assert_eq!(map_path(0, p), Some(MapPath { slot: 0, depth: 0, idx: [0; 3] }));
        assert_eq!(map_path(11, p).unwrap().slot, 11);
        assert_eq!(map_path(12, p), Some(MapPath { slot: 12, depth: 1, idx: [0, 0, 0] }));
        assert_eq!(map_path(12 + 255, p), Some(MapPath { slot: 12, depth: 1, idx: [255, 0, 0] }));
        assert_eq!(map_path(12 + 256, p), Some(MapPath { slot: 13, depth: 2, idx: [0, 0, 0] }));
        assert_eq!(map_path(12 + 256 + 257, p), Some(MapPath { slot: 13, depth: 2, idx: [1, 1, 0] }));
        let t = 12 + 256 + 65536;
        assert_eq!(map_path(t, p), Some(MapPath { slot: 14, depth: 3, idx: [0, 0, 0] }));
        assert_eq!(map_path(t + 65536 + 256 + 1, p), Some(MapPath { slot: 14, depth: 3, idx: [1, 1, 1] }));
        assert_eq!(map_path(t + 256 * 65536 - 1, p), Some(MapPath { slot: 14, depth: 3, idx: [255; 3] }));
        assert_eq!(map_path(t + 256 * 65536, p), None);
        assert_eq!(map_path(u64::MAX, 1024), None);
    }
}
