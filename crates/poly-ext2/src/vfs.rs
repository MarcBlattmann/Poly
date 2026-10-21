//! [`FileSystem`] for [`Ext2`] (ext2, ext3 and ext4 volumes): each method takes the lock, runs one
//! operation from `ops.rs` and writes back what the operation cached.

use crate::{BlockIo, Ext2};
use alloc::vec::Vec;
use poly_vfs::lock::RawLock;
use poly_vfs::{Attr, DirEntry, Errno, FileSystem, FsInfo, Ino, NewNode, RenameFlags, Result, SetAttr};

/// Entries `readdir` copies out per round, between which the lock is
/// released for the callback.
const READDIR_BATCH: usize = 64;

impl<D: BlockIo, L: RawLock> Ext2<D, L> {
    /// Runs an operation that changes nothing visible but may update access
    /// times when the volume is writable.
    fn access_op<R>(&self, f: impl FnOnce(&mut crate::Ctx<'_, D>) -> Result<R>) -> Result<R> {
        if self.read_only() { self.read_op(f) } else { self.write_op(f) }
    }
}

impl<D: BlockIo, L: RawLock> FileSystem for Ext2<D, L> {
    fn info(&self) -> FsInfo {
        FsInfo { name: self.fs_type(), read_only: self.read_only(), cache_lookups: true }
    }

    fn root(&self) -> Ino {
        crate::layout::ROOT_INO as Ino
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        self.read_op(|c| {
            let dir = c.ino(dir)?;
            c.lookup(dir, name).map(Ino::from)
        })
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        self.read_op(|c| {
            let ino = c.ino(ino)?;
            let i = c.live_inode(ino)?;
            c.attr(ino, &i)
        })
    }

    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        self.write_op(|c| {
            let ino = c.ino(ino)?;
            c.setattr(ino, set)
        })
    }

    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        self.write_op(|c| {
            let dir = c.ino(dir)?;
            c.create(dir, name, node).map(Ino::from)
        })
    }

    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino> {
        self.write_op(|c| {
            let dir = c.ino(dir)?;
            c.mkdir(dir, name, perm, uid, gid).map(Ino::from)
        })
    }

    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino> {
        self.write_op(|c| {
            let dir = c.ino(dir)?;
            c.symlink(dir, name, target, uid, gid).map(Ino::from)
        })
    }

    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        self.write_op(|c| {
            let (dir, ino) = (c.ino(dir)?, c.ino(ino)?);
            c.link(dir, name, ino)
        })
    }

    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.write_op(|c| {
            let dir = c.ino(dir)?;
            c.unlink(dir, name)
        })
    }

    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.write_op(|c| {
            let dir = c.ino(dir)?;
            c.rmdir(dir, name)
        })
    }

    fn rename(&self, old_dir: Ino, old_name: &[u8], new_dir: Ino, new_name: &[u8], flags: RenameFlags) -> Result<()> {
        self.write_op(|c| {
            let (old_dir, new_dir) = (c.ino(old_dir)?, c.ino(new_dir)?);
            c.rename(old_dir, old_name, new_dir, new_name, flags)
        })
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        self.read_op(|c| {
            let ino = c.ino(ino)?;
            c.readlink(ino)
        })
    }

    fn getxattr(&self, ino: Ino, name: &[u8]) -> Result<Vec<u8>> {
        self.read_op(|c| {
            let ino = c.ino(ino)?;
            c.xattr_get(ino, name)
        })
    }

    fn setxattr(&self, ino: Ino, name: &[u8], value: &[u8], how: poly_vfs::xattr::SetMode) -> Result<()> {
        self.write_op(|c| {
            let ino = c.ino(ino)?;
            c.xattr_set(ino, name, Some(value), how)
        })
    }

    fn listxattr(&self, ino: Ino) -> Result<Vec<u8>> {
        self.read_op(|c| {
            let ino = c.ino(ino)?;
            c.xattr_list(ino)
        })
    }

    fn removexattr(&self, ino: Ino, name: &[u8]) -> Result<()> {
        self.write_op(|c| {
            let ino = c.ino(ino)?;
            c.xattr_set(ino, name, None, poly_vfs::xattr::SetMode::default())
        })
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.access_op(|c| {
            let ino = c.ino(ino)?;
            c.read(ino, offset, buf)
        })
    }

    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        self.write_op(|c| {
            let ino = c.ino(ino)?;
            c.write(ino, offset, buf)
        })
    }

    fn write_back(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        self.write_op(|c| {
            let ino = c.ino(ino)?;
            c.write_back(ino, offset, buf)
        })
    }

    fn append(&self, ino: Ino, buf: &[u8]) -> Result<(u64, usize)> {
        self.write_op(|c| {
            let ino = c.ino(ino)?;
            c.append(ino, buf)
        })
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        let start = pos;
        let mut pos = pos;
        loop {
            let batch = match self.read_op(|c| {
                let dir = c.ino(dir)?;
                c.readdir(dir, pos, READDIR_BATCH)
            }) {
                // Removed while this call listed it: what was listed stands.
                Err(Errno::ENOENT) if pos != start => return Ok(()),
                r => r?,
            };
            let full = batch.len() >= READDIR_BATCH;
            for (ino, kind, name, next) in &batch {
                if !emit(&DirEntry { ino: *ino as Ino, kind: *kind, name, next: *next }) {
                    return Ok(());
                }
                pos = *next;
            }
            if !full {
                return Ok(());
            }
        }
    }

    fn open(&self, ino: Ino) -> Result<()> {
        let mut st = self.state.lock();
        let mut c = self.ctx(&mut st);
        let ino = c.ino(ino)?;
        c.live_inode(ino)?;
        let pins = c.st.pins.entry(ino).or_insert(0);
        *pins = pins.checked_add(1).ok_or(Errno::ENFILE)?;
        Ok(())
    }

    fn release(&self, ino: Ino) {
        let Ok(ino) = u32::try_from(ino) else { return };
        let mut st = self.state.lock();
        match st.pins.get_mut(&ino) {
            Some(n) if *n > 1 => {
                *n -= 1;
                return;
            }
            Some(_) => {
                st.pins.remove(&ino);
            }
            None => return,
        }
        if !st.orphans.contains(&ino) {
            return;
        }
        drop(st);
        // The last pin of an inode without names: delete it now. Errors
        // leave it for the orphan list at the next mount (with a journal)
        // or e2fsck.
        let _ = self.write_op(|c| if c.pinned(ino) { Ok(()) } else { c.release_orphan(ino) });
    }

    fn sync(&self) -> Result<()> {
        Ext2::sync(self)
    }

    /// From the group descriptors' free counts ([`Ext2::stats`]); blocks
    /// reserved for root are not held back from `f_bavail`.
    fn statfs(&self) -> Result<poly_vfs::FsStats> {
        let s = self.stats();
        Ok(poly_vfs::FsStats {
            magic: poly_vfs::EXT2_SUPER_MAGIC,
            block_size: u64::from(s.block_size),
            blocks: s.blocks,
            blocks_free: s.free_blocks,
            blocks_avail: s.free_blocks,
            files: s.inodes,
            files_free: s.free_inodes,
            name_max: 255,
            fragment_size: u64::from(s.block_size),
            ..poly_vfs::FsStats::default()
        })
    }
}
