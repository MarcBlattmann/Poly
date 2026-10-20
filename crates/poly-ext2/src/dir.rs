//! Directories: a file of blocks, each a chain of variable-length records
//! (inode, record length, name length, file type, name). The records of a
//! block cover it exactly; an unused record has inode 0. With
//! `metadata_csum` each block ends with a 12-byte tail holding the block's
//! CRC-32C (seeded per inode), which the records do not cover.
//!
//! - **Insertion** takes the first record with room: an unused record big
//!   enough, or the slack after a used record's name, which is split off
//!   into the new record. Without room, the directory grows by a block
//!   holding one record that covers it.
//! - **Removal** merges the record into the previous one in its block; the
//!   first record of a block is marked unused instead.
//! - **Positions** for `readdir` are byte offsets in the directory. An
//!   offset left inside a record by a removal still works: listing resumes
//!   at the first record starting at or after it.
//! - **Hash-indexed directories** (`FL_INDEX`, `dir_index`) are read
//!   through their leaves: the index blocks look like blocks of unused
//!   records (the root's `..` record covers the index). The hash functions
//!   are not part of the public format documentation, so this driver does
//!   not maintain indexes: before it changes an indexed directory it drops
//!   the index, as the format allows: the flag goes, and with checksums the
//!   index blocks become ordinary (empty) leaf blocks with tails. `e2fsck
//!   -D` rebuilds indexes.
//! - **Inline directories** (`inline_data`) are read from the inode
//!   (`inline.rs`).

use crate::csum;
use crate::layout::{
    DIR_TAIL, DIRENT_HEADER, FL_INDEX, FL_INLINE_DATA, Inode, RawEntry, dirent_kind, dirent_type, has_dir_tail,
    le16, le32, put_dir_tail, put16, put32, rec_len_for,
};
use crate::{BlockIo, Ctx};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Errno, FileType, Result};

/// A directory entry and where it is.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot {
    pub phys: u64,
    pub off: usize,
    /// The previous record in the same block.
    pub prev: Option<usize>,
    pub entry: RawEntry,
}

/// One listed entry: inode, type, name, position of the next one.
pub(crate) type Listed = (u32, FileType, Vec<u8>, u64);

impl<D: BlockIo> Ctx<'_, D> {
    pub fn dir_blocks(&self, dir: &Inode) -> Result<u64> {
        let bs = self.geo.bs as u64;
        if !dir.size.is_multiple_of(bs) || dir.size > u32::MAX as u64 {
            return Err(self.corrupt());
        }
        Ok(dir.size / bs)
    }

    fn dir_seed(&self, dir: &Inode) -> Option<u32> {
        self.geo.meta_seed().map(|s| csum::inode_seed(s, dir.ino, dir.generation))
    }

    /// Whether `block` (logical block `lblk` of `dir`) is a hash index
    /// block: the root (block 0) or an interior node (one unused record
    /// covering the block).
    fn is_dx(&self, dir: &Inode, lblk: u64, block: &[u8]) -> bool {
        dir.flags & FL_INDEX != 0
            && (lblk == 0 && le16(block, 12 + 4) as usize == self.geo.bs - 12
                || le32(block, 0) == 0 && le16(block, 4) as usize == self.geo.bs)
    }

    /// Directory block `lblk` and its checksum checked.
    fn dir_block(&mut self, dir: &Inode, lblk: u64) -> Result<(u64, Vec<u8>)> {
        let Some(phys) = self.bmap(dir, lblk)? else {
            // A hole in a directory.
            return Err(self.corrupt());
        };
        let block = self.read_block(phys)?;
        if let Some(seed) = self.dir_seed(dir) {
            if has_dir_tail(&block) {
                if le32(&block, self.geo.bs - 4) != csum::dir_block(seed, &block) {
                    return Err(self.bad_csum());
                }
            } else if self.is_dx(dir, lblk, &block) {
                self.check_dx(seed, lblk, &block)?;
            } else {
                // A leaf without room for its checksum.
                return Err(self.corrupt());
            }
        }
        Ok((phys, block))
    }

    /// Checks a hash index block's checksum (in the tail after its
    /// `limit` entries).
    fn check_dx(&self, seed: u32, lblk: u64, block: &[u8]) -> Result<()> {
        let count_off = if lblk == 0 { 24 + block.get(29).map_or(8, |&l| l as usize) } else { 8 };
        if count_off + 4 > block.len() {
            return Err(self.corrupt());
        }
        let (limit, count) = (le16(block, count_off) as usize, le16(block, count_off + 2) as usize);
        let tail = count_off + limit * 8;
        if count > limit || tail + 8 > block.len() {
            return Err(self.corrupt());
        }
        if le32(block, tail + 4) != csum::dx_block(seed, block, count_off, count) {
            return Err(self.bad_csum());
        }
        Ok(())
    }

    /// Where a block's records end: before the tail if it has one.
    fn records_end(&self, block: &[u8]) -> usize {
        if self.geo.meta_seed().is_some() && has_dir_tail(block) { block.len() - DIR_TAIL } else { block.len() }
    }

    /// The records of a directory block, checked, with their offsets.
    fn records(&self, block: &[u8]) -> Result<Vec<(usize, RawEntry)>> {
        let end = self.records_end(block);
        let mut out = Vec::new();
        let mut off = 0;
        while off < end {
            let e = RawEntry::decode(&block[..end], off, self.geo.filetype).ok_or_else(|| self.corrupt())?;
            if e.inode > self.geo.inodes {
                return Err(self.corrupt());
            }
            out.push((off, e));
            off += e.rec_len;
        }
        Ok(out)
    }

    /// Writes a directory block, its tail checksum set.
    fn write_dir_block(&mut self, dir: &Inode, phys: u64, block: &mut [u8], fresh: bool) -> Result<()> {
        if let Some(seed) = self.dir_seed(dir) {
            if !has_dir_tail(block) {
                return Err(self.corrupt());
            }
            let c = csum::dir_block(seed, block);
            put32(block, self.geo.bs - 4, c);
        }
        if fresh { self.write_fresh(phys, block) } else { self.write_meta(phys, block) }
    }

    /// Drops a directory's hash index before it changes (see the module
    /// documentation).
    fn unindex(&mut self, dir: &mut Inode) -> Result<()> {
        if dir.flags & FL_INDEX == 0 {
            return Ok(());
        }
        if self.geo.meta_seed().is_some() {
            let bs = self.geo.bs;
            let indexed = dir.clone();
            for lblk in 0..self.dir_blocks(dir)? {
                let Some(phys) = self.bmap(dir, lblk)? else { return Err(self.corrupt()) };
                let block = self.read_block(phys)?;
                if has_dir_tail(&block) || !self.is_dx(&indexed, lblk, &block) {
                    continue;
                }
                let mut new = vec![0u8; bs];
                if lblk == 0 {
                    let ft = dirent_type(FileType::Directory);
                    let filetype = self.geo.filetype;
                    RawEntry::encode(&mut new, 0, le32(&block, 0), 12, b".", ft, filetype);
                    RawEntry::encode(&mut new, 12, le32(&block, 12), bs - 12 - DIR_TAIL, b"..", ft, filetype);
                } else {
                    put16(&mut new, 4, (bs - DIR_TAIL) as u16);
                }
                put_dir_tail(&mut new);
                self.write_dir_block(dir, phys, &mut new, false)?;
            }
        }
        dir.flags &= !FL_INDEX;
        Ok(())
    }

    pub fn find_entry(&mut self, dir: &Inode, name: &[u8]) -> Result<Option<Slot>> {
        if dir.flags & FL_INLINE_DATA != 0 {
            return self.inline_find(dir, name);
        }
        for lblk in 0..self.dir_blocks(dir)? {
            let (phys, block) = self.dir_block(dir, lblk)?;
            let mut prev = None;
            for (off, e) in self.records(&block)? {
                if e.inode != 0 && e.name(&block, off) == name {
                    return Ok(Some(Slot { phys, off, prev, entry: e }));
                }
                prev = Some(off);
            }
        }
        Ok(None)
    }

    /// Adds an entry (the name must not exist). May grow the directory,
    /// changing `dir`, which the caller writes back.
    pub fn add_entry(&mut self, dir: &mut Inode, name: &[u8], ino: u32, kind: FileType) -> Result<()> {
        if dir.flags & FL_INLINE_DATA != 0 {
            return Err(Errno::EROFS);
        }
        self.unindex(dir)?;
        let needed = rec_len_for(name.len());
        let ftype = dirent_type(kind);
        let filetype = self.geo.filetype;
        let blocks = self.dir_blocks(dir)?;
        for lblk in 0..blocks {
            let (phys, mut block) = self.dir_block(dir, lblk)?;
            for (off, e) in self.records(&block)? {
                if e.inode == 0 && e.rec_len >= needed {
                    RawEntry::encode(&mut block, off, ino, e.rec_len, name, ftype, filetype);
                } else if e.inode != 0 && e.rec_len - e.used() >= needed {
                    let used = e.used();
                    put16(&mut block, off + 4, used as u16);
                    RawEntry::encode(&mut block, off + used, ino, e.rec_len - used, name, ftype, filetype);
                } else {
                    continue;
                }
                return self.write_dir_block(dir, phys, &mut block, false);
            }
        }
        // Grow by a block.
        let bs = self.geo.bs;
        if dir.size + bs as u64 > u32::MAX as u64 {
            return Err(Errno::ENOSPC);
        }
        let mut goal = match blocks.checked_sub(1) {
            Some(last) => self.bmap(dir, last)?.map_or(0, |b| b + 1),
            None => 0,
        };
        let (phys, fresh) = self.bmap_alloc(dir, blocks, &mut goal)?;
        let mut block = vec![0u8; bs];
        let tail = if self.geo.meta_seed().is_some() { DIR_TAIL } else { 0 };
        RawEntry::encode(&mut block, 0, ino, bs - tail, name, ftype, filetype);
        if tail != 0 {
            put_dir_tail(&mut block);
        }
        self.write_dir_block(dir, phys, &mut block, fresh)?;
        dir.size += bs as u64;
        Ok(())
    }

    /// Removes the entry at `slot`.
    pub fn remove_slot(&mut self, dir: &mut Inode, slot: &Slot) -> Result<()> {
        self.unindex(dir)?;
        let mut block = self.read_block(slot.phys)?;
        match slot.prev {
            Some(p) => {
                let pe = RawEntry::decode(&block, p, self.geo.filetype).ok_or_else(|| self.corrupt())?;
                put16(&mut block, p + 4, (pe.rec_len + slot.entry.rec_len) as u16);
            }
            None => put32(&mut block, slot.off, 0),
        }
        self.write_dir_block(dir, slot.phys, &mut block, false)
    }

    /// Points the entry at `slot` at another inode.
    pub fn retarget_slot(&mut self, dir: &mut Inode, slot: &Slot, ino: u32, kind: FileType) -> Result<()> {
        self.unindex(dir)?;
        let mut block = self.read_block(slot.phys)?;
        put32(&mut block, slot.off, ino);
        if self.geo.filetype {
            block[slot.off + DIRENT_HEADER - 1] = dirent_type(kind);
        }
        self.write_dir_block(dir, slot.phys, &mut block, false)
    }

    /// Whether the directory has nothing but `.` and `..`.
    pub fn dir_is_empty(&mut self, dir: &Inode) -> Result<bool> {
        if dir.flags & FL_INLINE_DATA != 0 {
            return Ok(self.inline_dir(dir)?.len() <= 2);
        }
        for lblk in 0..self.dir_blocks(dir)? {
            let (_, block) = self.dir_block(dir, lblk)?;
            for (off, e) in self.records(&block)? {
                let name = e.name(&block, off);
                if e.inode != 0 && name != b"." && name != b".." {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Up to `max` entries from position `pos` on.
    pub fn list_dir(&mut self, dir: &Inode, pos: u64, max: usize) -> Result<Vec<Listed>> {
        if dir.flags & FL_INLINE_DATA != 0 {
            let all = self.inline_dir(dir)?;
            let mut out = Vec::new();
            for (i, (ino, ft, name)) in all.into_iter().enumerate().skip(pos as usize).take(max) {
                let kind = match dirent_kind(ft).filter(|_| self.geo.filetype) {
                    Some(k) => k,
                    None => self.read_inode(ino)?.kind().ok_or_else(|| self.corrupt())?,
                };
                out.push((ino, kind, name, i as u64 + 1));
            }
            return Ok(out);
        }
        let bs = self.geo.bs as u64;
        let blocks = self.dir_blocks(dir)?;
        let mut out = Vec::new();
        let mut lblk = pos / bs;
        let mut from = (pos % bs) as usize;
        while lblk < blocks && out.len() < max {
            let (_, block) = self.dir_block(dir, lblk)?;
            for (off, e) in self.records(&block)? {
                if off < from || e.inode == 0 {
                    continue;
                }
                let kind = match dirent_kind(e.file_type).filter(|_| self.geo.filetype) {
                    Some(k) => k,
                    None => self.read_inode(e.inode)?.kind().ok_or_else(|| self.corrupt())?,
                };
                out.push((e.inode, kind, e.name(&block, off).to_vec(), lblk * bs + (off + e.rec_len) as u64));
                if out.len() >= max {
                    break;
                }
            }
            lblk += 1;
            from = 0;
        }
        Ok(out)
    }

    /// The first block of a new directory: `.` and `..` (and the tail).
    pub fn new_dir_block(&self, ino: u32, parent: u32) -> Vec<u8> {
        let bs = self.geo.bs;
        let mut block = vec![0u8; bs];
        let ft = dirent_type(FileType::Directory);
        let tail = if self.geo.meta_seed().is_some() { DIR_TAIL } else { 0 };
        RawEntry::encode(&mut block, 0, ino, 12, b".", ft, self.geo.filetype);
        RawEntry::encode(&mut block, 12, parent, bs - 12 - tail, b"..", ft, self.geo.filetype);
        if tail != 0 {
            put_dir_tail(&mut block);
        }
        block
    }

    /// Writes a new directory's first block (checksummed for `dir`).
    pub fn write_new_dir_block(&mut self, dir: &Inode, phys: u64, block: &mut [u8]) -> Result<()> {
        self.write_dir_block(dir, phys, block, true)
    }
}
