//! Inodes in the inode tables: finding, reading, writing (with their
//! checksums), initialising, and turning them into the VFS's attributes.

use crate::layout::{
    BG_INODE_UNINIT, FAST_SYMLINK_MAX, FL_EA_INODE, FL_EXTENTS, FL_INLINE_DATA, Inode, ino as off,
};
use crate::{BlockIo, Ctx, csum};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Attr, DeviceId, Errno, FileType, Ino, Result};

/// The device number in `i_block` (ext4 docs, "Inode i_block": the old
/// 16-bit encoding in word 0, or the new 32-bit one in word 1).
pub fn decode_rdev(block: &[u32; 15]) -> DeviceId {
    if block[0] != 0 {
        DeviceId::new((block[0] >> 8) & 0xff, block[0] & 0xff)
    } else {
        let x = block[1];
        DeviceId::new((x >> 8) & 0xfff, (x & 0xff) | ((x >> 12) & 0xfff00))
    }
}

pub fn encode_rdev(dev: DeviceId, block: &mut [u32; 15]) {
    if dev.major < 256 && dev.minor < 256 {
        block[0] = dev.major << 8 | dev.minor;
        block[1] = 0;
    } else {
        block[0] = 0;
        block[1] = (dev.minor & 0xff) | (dev.major & 0xfff) << 8 | (dev.minor & !0xff) << 12;
    }
}

impl<D: BlockIo> Ctx<'_, D> {
    /// The inode table block holding inode `ino`, and the record's offset
    /// in it.
    fn inode_at(&self, ino: u32) -> Result<(u64, usize)> {
        if ino == 0 || ino > self.geo.inodes {
            return Err(self.corrupt());
        }
        let g = self.geo.group_of_inode(ino);
        let index = ((ino - 1) % self.geo.ipg) as u64;
        let table = self.st.groups[g as usize].inode_table;
        let at = index * self.geo.ifmt.size as u64;
        let bs = self.geo.bs as u64;
        Ok((table + at / bs, (at % bs) as usize))
    }

    /// Whether inode `ino`'s record was never initialised: its group's
    /// table is marked unused from there on (`uninit_bg`/`metadata_csum`).
    fn never_used(&self, ino: u32) -> bool {
        if !self.geo.gdt_csum() {
            return false;
        }
        let g = self.geo.group_of_inode(ino);
        let d = &self.st.groups[g as usize];
        let index = (ino - 1) % self.geo.ipg;
        d.flags & BG_INODE_UNINIT != 0 || index >= self.geo.ipg - d.itable_unused.min(self.geo.ipg)
    }

    /// Inode `ino`'s record as on disk (zeros for a never used one).
    pub fn read_inode_raw(&mut self, ino: u32) -> Result<Vec<u8>> {
        let (block, at) = self.inode_at(ino)?;
        let size = self.geo.ifmt.size;
        if self.never_used(ino) {
            return Ok(vec![0u8; size]);
        }
        let data = self.read_block(block)?;
        let raw = data[at..at + size].to_vec();
        if let Some(seed) = self.geo.meta_seed() {
            let i = Inode::decode(&raw, &self.geo.ifmt);
            // Free records keep whatever they had; only inodes in use must
            // check out.
            let in_use = i.mode != 0 && (i.links != 0 || i.dtime == 0);
            if in_use && !csum::inode_ok(seed, ino, &raw) {
                return Err(self.bad_csum());
            }
        }
        Ok(raw)
    }

    /// Inode `ino` as on disk, whatever its state.
    pub fn read_inode(&mut self, ino: u32) -> Result<Inode> {
        let raw = self.read_inode_raw(ino)?;
        let mut i = Inode::decode(&raw, &self.geo.ifmt);
        i.ino = ino;
        Ok(i)
    }

    /// Writes a record (its checksum set) into its inode table block.
    fn put_inode_raw(&mut self, ino: u32, raw: &mut [u8]) -> Result<()> {
        let (block, at) = self.inode_at(ino)?;
        if let Some(seed) = self.geo.meta_seed() {
            csum::set_inode(seed, ino, raw);
        }
        let mut data = self.read_block(block)?;
        data[at..at + raw.len()].copy_from_slice(raw);
        self.write_meta(block, &data)
    }

    /// Writes the inode's fields over its record (other bytes stay).
    pub fn write_inode(&mut self, ino: u32, inode: &Inode) -> Result<()> {
        let mut raw = self.read_inode_raw(ino).or_else(|e| match e {
            // Rewriting an inode whose record failed its checksum repairs it.
            Errno::EBADMSG => {
                let (block, at) = self.inode_at(ino)?;
                Ok(self.read_block(block)?[at..at + self.geo.ifmt.size].to_vec())
            }
            e => Err(e),
        })?;
        inode.encode(&mut raw, &self.geo.ifmt);
        self.put_inode_raw(ino, &mut raw)
    }

    /// Writes a new inode over a cleared record.
    pub fn init_inode(&mut self, ino: u32, inode: &Inode) -> Result<()> {
        let mut raw = vec![0u8; self.geo.ifmt.size];
        Inode::init_record(&mut raw, self.now);
        inode.encode(&mut raw, &self.geo.ifmt);
        self.put_inode_raw(ino, &mut raw)
    }

    pub fn pinned(&self, ino: u32) -> bool {
        self.st.pins.get(&ino).is_some_and(|&n| n > 0)
    }

    /// The VFS's inode number as ours (`ENOENT` for numbers never handed out).
    pub fn ino(&self, ino: Ino) -> Result<u32> {
        match u32::try_from(ino) {
            Ok(i) if i >= 1 && i <= self.geo.inodes => Ok(i),
            _ => Err(Errno::ENOENT),
        }
    }

    /// An inode that exists: in use with a name, or pinned. Checks that it
    /// makes sense for this volume.
    pub fn live_inode(&mut self, ino: u32) -> Result<Inode> {
        let i = self.read_inode(ino)?;
        if i.mode == 0 || (i.links == 0 && !self.pinned(ino)) {
            return Err(Errno::ENOENT);
        }
        if i.kind().is_none()
            || (i.flags & FL_EXTENTS != 0 && !self.geo.extents)
            || (i.flags & FL_INLINE_DATA != 0 && !self.geo.inline_data)
            || i.flags & FL_EA_INODE != 0
        {
            return Err(self.corrupt());
        }
        Ok(i)
    }

    /// A symbolic link whose target lives in `i_block`.
    pub fn is_fast_symlink(&self, i: &Inode) -> bool {
        if i.kind() != Some(FileType::Symlink) || i.flags & (FL_EXTENTS | FL_INLINE_DATA) != 0 {
            return false;
        }
        let acl = if i.file_acl != 0 { self.geo.spb as u64 } else { 0 };
        i.blocks.saturating_sub(acl) == 0
    }

    /// Whether `i_block` holds a block map or an extent tree (not a device
    /// number, a fast symbolic link or inline data).
    pub fn has_block_map(&self, i: &Inode) -> bool {
        if i.flags & FL_INLINE_DATA != 0 {
            return false;
        }
        match i.kind() {
            Some(FileType::Regular | FileType::Directory) => true,
            Some(FileType::Symlink) => !self.is_fast_symlink(i),
            _ => false,
        }
    }

    pub fn attr(&self, ino: u32, i: &Inode) -> Result<Attr> {
        let kind = i.kind().ok_or_else(|| self.corrupt())?;
        let rdev = match kind {
            FileType::CharDevice | FileType::BlockDevice => decode_rdev(&i.block),
            _ => DeviceId::default(),
        };
        if kind == FileType::Symlink && self.is_fast_symlink(i) && i.size >= FAST_SYMLINK_MAX as u64 {
            return Err(self.corrupt());
        }
        // `dir_nlink`: a directory with too many subdirectories to count
        // has a link count of 1.
        Ok(Attr {
            ino: ino as Ino,
            kind,
            perm: i.mode & poly_vfs::PERM_MASK,
            nlink: i.links as u32,
            uid: i.uid,
            gid: i.gid,
            size: i.size,
            rdev,
            atime: i.atime,
            mtime: i.mtime,
            ctime: i.ctime,
            blocks: i.blocks,
            blksize: self.geo.bs as u32,
        })
    }

    /// The record's creation time, if it has one.
    #[allow(dead_code)]
    pub fn crtime(&mut self, ino: u32) -> Result<Option<poly_vfs::Timespec>> {
        let raw = self.read_inode_raw(ino)?;
        Ok(Inode::crtime(&raw).filter(|_| raw.len() > off::CRTIME_EXTRA))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbers() {
        for dev in [DeviceId::new(1, 3), DeviceId::new(8, 255), DeviceId::new(259, 0), DeviceId::new(4095, 0xfffff)] {
            let mut b = [0u32; 15];
            encode_rdev(dev, &mut b);
            assert_eq!(decode_rdev(&b), dev);
            assert_eq!(b[0] == 0, dev.major >= 256 || dev.minor >= 256);
        }
        // The new encoding as documented: minor 0x12345, major 0x123.
        let mut b = [0u32; 15];
        encode_rdev(DeviceId::new(0x123, 0x12345), &mut b);
        assert_eq!(b[1], 0x45 | 0x123 << 8 | 0x12300 << 12);
    }
}
