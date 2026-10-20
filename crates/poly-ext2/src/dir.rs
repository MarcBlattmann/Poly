//! Directories: a file of blocks, each a chain of variable-length records
//! (inode, record length, name length, file type, name). The records of a
//! block cover it exactly; an unused record has inode 0.
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
//! - A hash-indexed directory (`FL_INDEX`) is read linearly (its index
//!   blocks look like unused records) and loses the flag when changed, as
//!   the format asks of drivers that do not maintain the index.

use crate::layout::{DIRENT_HEADER, FL_INDEX, Inode, RawEntry, dirent_kind, dirent_type, put16, put32, rec_len_for};
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

    fn dir_block(&mut self, dir: &Inode, lblk: u64) -> Result<(u64, Vec<u8>)> {
        match self.bmap(dir, lblk)? {
            Some(phys) => Ok((phys, self.read_block(phys)?)),
            // A hole in a directory.
            None => Err(self.corrupt()),
        }
    }

    /// The records of a directory block, checked, with their offsets.
    fn records(&self, block: &[u8]) -> Result<Vec<(usize, RawEntry)>> {
        let mut out = Vec::new();
        let mut off = 0;
        while off < block.len() {
            let e = RawEntry::decode(block, off, self.geo.filetype).ok_or_else(|| self.corrupt())?;
            if e.inode > self.geo.inodes {
                return Err(self.corrupt());
            }
            out.push((off, e));
            off += e.rec_len;
        }
        Ok(out)
    }

    pub fn find_entry(&mut self, dir: &Inode, name: &[u8]) -> Result<Option<Slot>> {
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
                self.write_meta(phys, &block)?;
                dir.flags &= !FL_INDEX;
                return Ok(());
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
        RawEntry::encode(&mut block, 0, ino, bs, name, ftype, filetype);
        if fresh {
            self.write_fresh(phys, &block)?;
        } else {
            self.write_meta(phys, &block)?;
        }
        dir.size += bs as u64;
        dir.flags &= !FL_INDEX;
        Ok(())
    }

    /// Removes the entry at `slot`.
    pub fn remove_slot(&mut self, dir: &mut Inode, slot: &Slot) -> Result<()> {
        let mut block = self.read_block(slot.phys)?;
        match slot.prev {
            Some(p) => {
                let pe = RawEntry::decode(&block, p, self.geo.filetype).ok_or_else(|| self.corrupt())?;
                put16(&mut block, p + 4, (pe.rec_len + slot.entry.rec_len) as u16);
            }
            None => put32(&mut block, slot.off, 0),
        }
        self.write_meta(slot.phys, &block)?;
        dir.flags &= !FL_INDEX;
        Ok(())
    }

    /// Points the entry at `slot` at another inode.
    pub fn retarget_slot(&mut self, dir: &mut Inode, slot: &Slot, ino: u32, kind: FileType) -> Result<()> {
        let mut block = self.read_block(slot.phys)?;
        put32(&mut block, slot.off, ino);
        if self.geo.filetype {
            block[slot.off + DIRENT_HEADER - 1] = dirent_type(kind);
        }
        self.write_meta(slot.phys, &block)?;
        dir.flags &= !FL_INDEX;
        Ok(())
    }

    /// Whether the directory has nothing but `.` and `..`.
    pub fn dir_is_empty(&mut self, dir: &Inode) -> Result<bool> {
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

    /// The first block of a new directory: `.` and `..`.
    pub fn new_dir_block(&self, ino: u32, parent: u32) -> Vec<u8> {
        let bs = self.geo.bs;
        let mut block = vec![0u8; bs];
        let ft = dirent_type(FileType::Directory);
        RawEntry::encode(&mut block, 0, ino, 12, b".", ft, self.geo.filetype);
        RawEntry::encode(&mut block, 12, parent, bs - 12, b"..", ft, self.geo.filetype);
        block
    }
}
