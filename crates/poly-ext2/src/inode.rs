//! Inodes in the inode tables: finding, reading, writing, initialising,
//! and turning them into the VFS's attributes.

use crate::layout::{FAST_SYMLINK_MAX, FL_EXTENTS, FL_INLINE_DATA, Inode};
use crate::{BlockIo, Ctx};
use alloc::vec;
use poly_vfs::{Attr, DeviceId, Errno, FileType, Ino, Result};

/// The device number in `i_block` (ext4 wiki, "Inode i_block": the old
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
    /// Byte offset of inode `ino`'s record on the disk.
    fn inode_at(&self, ino: u32) -> Result<u64> {
        if ino == 0 || ino > self.geo.inodes {
            return Err(self.corrupt());
        }
        let g = self.geo.group_of_inode(ino);
        let index = ((ino - 1) % self.geo.ipg) as u64;
        let table = self.st.groups[g as usize].inode_table as u64;
        Ok(table * self.geo.bs as u64 + index * self.geo.ifmt.size as u64)
    }

    /// Inode `ino` as on disk, whatever its state.
    pub fn read_inode(&mut self, ino: u32) -> Result<Inode> {
        let at = self.inode_at(ino)?;
        let mut raw = vec![0u8; self.geo.ifmt.size];
        self.disk.read_bytes(at, &mut raw)?;
        Ok(Inode::decode(&raw, &self.geo.ifmt))
    }

    /// Writes the inode's fields over its record (other bytes stay).
    pub fn write_inode(&mut self, ino: u32, inode: &Inode) -> Result<()> {
        let at = self.inode_at(ino)?;
        let mut raw = vec![0u8; self.geo.ifmt.size];
        self.disk.read_bytes(at, &mut raw)?;
        inode.encode(&mut raw, &self.geo.ifmt);
        self.write_meta_bytes(at, &raw)
    }

    /// Writes a new inode over a cleared record.
    pub fn init_inode(&mut self, ino: u32, inode: &Inode) -> Result<()> {
        let at = self.inode_at(ino)?;
        let mut raw = vec![0u8; self.geo.ifmt.size];
        Inode::init_record(&mut raw, self.now);
        inode.encode(&mut raw, &self.geo.ifmt);
        self.write_meta_bytes(at, &raw)
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
    /// makes sense for ext2.
    pub fn live_inode(&mut self, ino: u32) -> Result<Inode> {
        let i = self.read_inode(ino)?;
        if i.mode == 0 || (i.links == 0 && !self.pinned(ino)) {
            return Err(Errno::ENOENT);
        }
        if i.kind().is_none() || i.flags & (FL_EXTENTS | FL_INLINE_DATA) != 0 {
            return Err(self.corrupt());
        }
        Ok(i)
    }

    /// A symbolic link whose target lives in `i_block`.
    pub fn is_fast_symlink(&self, i: &Inode) -> bool {
        let acl = if i.file_acl != 0 { self.geo.spb } else { 0 };
        i.kind() == Some(FileType::Symlink) && i.blocks.saturating_sub(acl) == 0
    }

    /// Whether `i_block` holds a block map (not a device number or a fast
    /// symbolic link).
    pub fn has_block_map(&self, i: &Inode) -> bool {
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
            blocks: i.blocks as u64,
            blksize: self.geo.bs as u32,
        })
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
