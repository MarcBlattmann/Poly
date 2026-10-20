//! The file system operations, with the `FileSystem` trait's semantics, on
//! one locked [`Ctx`]. Every operation reads the inodes it needs, changes
//! copies and writes them back; nothing is cached between operations except
//! pins and the allocation state.
//!
//! Write order inside an operation follows `balloc.rs`: link counts go up
//! before a new name is written and down after a name is removed (a crash
//! leaves a count too high, which e2fsck lowers, never too low); blocks and
//! inodes are freed after the writes that unhook them.

use crate::dir::Slot;
use crate::inode::encode_rdev;
use crate::layout::RO_COMPAT_LARGE_FILE;
use crate::layout::{FAST_SYMLINK_MAX, FL_APPEND, FL_IMMUTABLE, Inode, LINK_MAX, ROOT_INO, encode_time};
use crate::{BlockIo, Ctx};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Attr, Errno, FileType, NAME_MAX, NewNode, PERM_MASK, RenameFlags, Result, SetAttr, SetTime};

/// Deepest directory nesting `rename` walks up through before deciding the
/// volume has a loop.
const MAX_DEPTH: usize = 1 << 16;

fn check_name(name: &[u8]) -> Result<()> {
    if name.len() > NAME_MAX {
        Err(Errno::ENAMETOOLONG)
    } else if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') || name.contains(&0) {
        Err(Errno::EINVAL)
    } else {
        Ok(())
    }
}

/// The old and new directory of a rename, which may be one inode.
struct Dirs {
    old: Inode,
    new: Option<Inode>,
}

impl Dirs {
    fn new_dir(&mut self) -> &mut Inode {
        match &mut self.new {
            Some(n) => n,
            None => &mut self.old,
        }
    }
}

impl<D: BlockIo> Ctx<'_, D> {
    fn touch(&self, i: &mut Inode) {
        i.mtime = self.now;
        i.ctime = self.now;
    }

    /// A directory that may get new entries.
    fn live_dir(&mut self, dir: u32) -> Result<Inode> {
        let d = self.live_inode(dir)?;
        if !d.is_dir() {
            return Err(Errno::ENOTDIR);
        }
        if d.links == 0 {
            return Err(Errno::ENOENT);
        }
        Ok(d)
    }

    fn check_ids(&self, uid: u32, gid: u32) -> Result<()> {
        if !self.geo.ifmt.id_high && (uid > 0xffff || gid > 0xffff) { Err(Errno::EINVAL) } else { Ok(()) }
    }

    fn new_inode(&mut self, kind: FileType, perm: u16, uid: u32, gid: u32) -> Inode {
        self.st.next_generation = self.st.next_generation.wrapping_add(1);
        Inode {
            mode: kind.mode_bits() as u16 | (perm & PERM_MASK),
            uid,
            gid,
            atime: self.now,
            ctime: self.now,
            mtime: self.now,
            links: 1,
            generation: self.st.next_generation,
            ..Inode::default()
        }
    }

    /// Checks that `name` can be added to `dir`.
    fn check_new(&mut self, dir: &Inode, name: &[u8]) -> Result<()> {
        check_name(name)?;
        if dir.flags & FL_IMMUTABLE != 0 {
            return Err(Errno::EPERM);
        }
        if self.find_entry(dir, name)?.is_some() {
            return Err(Errno::EEXIST);
        }
        Ok(())
    }

    /// Takes back an inode that never got a name.
    fn abandon(&mut self, ino: u32, mut i: Inode, blocks: &[u64]) -> Result<()> {
        i.links = 0;
        i.dtime = encode_time(self.now, false).0;
        self.write_inode(ino, &i)?;
        self.free_blocks(blocks)?;
        self.free_inode(ino, i.is_dir())
    }

    /// Adds the entry for a just-initialised inode, or takes the inode
    /// back; then updates the directory.
    fn link_new(&mut self, dir: u32, d: &mut Inode, name: &[u8], ino: u32, i: Inode, blocks: &[u64]) -> Result<()> {
        let kind = i.kind().unwrap_or(FileType::Regular);
        if let Err(e) = self.add_entry(d, name, ino, kind) {
            self.abandon(ino, i, blocks)?;
            return Err(e);
        }
        if kind == FileType::Directory {
            d.links = d.links.saturating_add(1);
        }
        self.touch(d);
        self.write_inode(dir, d)
    }

    pub fn lookup(&mut self, dir: u32, name: &[u8]) -> Result<u32> {
        let d = self.live_inode(dir)?;
        if !d.is_dir() {
            return Err(Errno::ENOTDIR);
        }
        if name == b"." {
            return Ok(dir);
        }
        if name == b".." {
            return match self.find_entry(&d, b"..")? {
                Some(s) => Ok(s.entry.inode),
                None => Err(self.corrupt()),
            };
        }
        if d.links == 0 {
            return Err(Errno::ENOENT);
        }
        match self.find_entry(&d, name)? {
            Some(s) => Ok(s.entry.inode),
            None => Err(Errno::ENOENT),
        }
    }

    pub fn create(&mut self, dir: u32, name: &[u8], node: &NewNode) -> Result<u32> {
        if matches!(node.kind, FileType::Directory | FileType::Symlink) {
            return Err(Errno::EINVAL);
        }
        self.check_ids(node.uid, node.gid)?;
        let mut d = self.live_dir(dir)?;
        self.check_new(&d, name)?;
        let ino = self.alloc_inode(dir, false)?;
        let mut i = self.new_inode(node.kind, node.perm, node.uid, node.gid);
        if matches!(node.kind, FileType::CharDevice | FileType::BlockDevice) {
            encode_rdev(node.rdev, &mut i.block);
        }
        self.init_inode(ino, &i)?;
        self.link_new(dir, &mut d, name, ino, i, &[])?;
        Ok(ino)
    }

    pub fn mkdir(&mut self, dir: u32, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<u32> {
        self.check_ids(uid, gid)?;
        let mut d = self.live_dir(dir)?;
        self.check_new(&d, name)?;
        if d.links >= LINK_MAX {
            return Err(Errno::EMLINK);
        }
        let ino = self.alloc_inode(dir, true)?;
        let mut i = self.new_inode(FileType::Directory, perm, uid, gid);
        let goal = self.group_data_goal(self.geo.group_of_inode(ino));
        let block = match self.alloc_block(goal) {
            Ok(b) => b,
            Err(e) => {
                self.abandon(ino, i, &[])?;
                return Err(e);
            }
        };
        let data = self.new_dir_block(ino, dir);
        self.write_fresh(block, &data)?;
        i.links = 2;
        i.size = self.geo.bs as u64;
        i.blocks = self.geo.spb;
        i.block[0] = block as u32;
        self.init_inode(ino, &i)?;
        self.link_new(dir, &mut d, name, ino, i, &[block])?;
        Ok(ino)
    }

    pub fn symlink(&mut self, dir: u32, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<u32> {
        if target.is_empty() {
            return Err(Errno::ENOENT);
        }
        if target.len() >= self.geo.bs {
            return Err(Errno::ENAMETOOLONG);
        }
        self.check_ids(uid, gid)?;
        let mut d = self.live_dir(dir)?;
        self.check_new(&d, name)?;
        let ino = self.alloc_inode(dir, false)?;
        let mut i = self.new_inode(FileType::Symlink, 0o777, uid, gid);
        i.size = target.len() as u64;
        let mut blocks = Vec::new();
        if target.len() < FAST_SYMLINK_MAX {
            i.set_block_bytes(target);
        } else {
            let goal = self.group_data_goal(self.geo.group_of_inode(ino));
            let block = match self.alloc_block(goal) {
                Ok(b) => b,
                Err(e) => {
                    self.abandon(ino, i, &[])?;
                    return Err(e);
                }
            };
            let mut data = vec![0u8; self.geo.bs];
            data[..target.len()].copy_from_slice(target);
            self.write_fresh(block, &data)?;
            i.block[0] = block as u32;
            i.blocks = self.geo.spb;
            blocks.push(block);
        }
        self.init_inode(ino, &i)?;
        self.link_new(dir, &mut d, name, ino, i, &blocks)?;
        Ok(ino)
    }

    pub fn link(&mut self, dir: u32, name: &[u8], ino: u32) -> Result<()> {
        let mut t = self.live_inode(ino)?;
        if t.is_dir() {
            return Err(Errno::EPERM);
        }
        if t.links == 0 {
            return Err(Errno::ENOENT);
        }
        if t.links >= LINK_MAX {
            return Err(Errno::EMLINK);
        }
        if t.flags & (FL_IMMUTABLE | FL_APPEND) != 0 {
            return Err(Errno::EPERM);
        }
        let mut d = self.live_dir(dir)?;
        self.check_new(&d, name)?;
        let kind = t.kind().unwrap_or(FileType::Regular);
        t.links = t.links.saturating_add(1);
        t.ctime = self.now;
        self.write_inode(ino, &t)?;
        if let Err(e) = self.add_entry(&mut d, name, ino, kind) {
            t.links = t.links.saturating_sub(1);
            self.write_inode(ino, &t)?;
            return Err(e);
        }
        self.touch(&mut d);
        self.write_inode(dir, &d)
    }

    /// The entry `name` in `d` and its inode.
    fn entry_inode(&mut self, d: &Inode, name: &[u8]) -> Result<(Slot, Inode)> {
        let slot = self.find_entry(d, name)?.ok_or(Errno::ENOENT)?;
        let i = self.read_inode(slot.entry.inode)?;
        if i.mode == 0 || i.kind().is_none() {
            return Err(self.corrupt());
        }
        Ok((slot, i))
    }

    pub fn unlink(&mut self, dir: u32, name: &[u8]) -> Result<()> {
        let mut d = self.live_dir(dir)?;
        let (slot, i) = self.entry_inode(&d, name)?;
        if i.is_dir() {
            return Err(Errno::EISDIR);
        }
        if (i.flags | d.flags) & (FL_IMMUTABLE | FL_APPEND) != 0 {
            return Err(Errno::EPERM);
        }
        self.remove_slot(&mut d, &slot)?;
        self.touch(&mut d);
        self.write_inode(dir, &d)?;
        self.drop_link(slot.entry.inode, i)
    }

    pub fn rmdir(&mut self, dir: u32, name: &[u8]) -> Result<()> {
        let mut d = self.live_dir(dir)?;
        let (slot, i) = self.entry_inode(&d, name)?;
        if !i.is_dir() {
            return Err(Errno::ENOTDIR);
        }
        if !self.dir_is_empty(&i)? {
            return Err(Errno::ENOTEMPTY);
        }
        if (i.flags | d.flags) & (FL_IMMUTABLE | FL_APPEND) != 0 {
            return Err(Errno::EPERM);
        }
        self.remove_slot(&mut d, &slot)?;
        d.links = d.links.saturating_sub(1).max(2);
        self.touch(&mut d);
        self.write_inode(dir, &d)?;
        self.drop_link(slot.entry.inode, i)
    }

    /// Drops one name of `ino` (a directory loses its only one), freeing it
    /// if that was the last and nothing pins it.
    fn drop_link(&mut self, ino: u32, mut i: Inode) -> Result<()> {
        i.links = if i.is_dir() { 0 } else { i.links.saturating_sub(1) };
        i.ctime = self.now;
        if i.links == 0 && !self.pinned(ino) { self.evict(ino, i) } else { self.write_inode(ino, &i) }
    }

    /// Deletes an inode nothing names or pins: frees its blocks and itself.
    pub fn evict(&mut self, ino: u32, mut i: Inode) -> Result<()> {
        let dir = i.is_dir();
        let mut freed = Vec::new();
        if self.has_block_map(&i) {
            self.truncate_map(&mut i, 0, &mut freed)?;
        }
        if i.file_acl != 0 {
            match self.release_xattr(i.file_acl) {
                Ok(Some(b)) => freed.push(b),
                Ok(None) | Err(Errno::EUCLEAN) => {}
                Err(e) => return Err(e),
            }
            i.blocks = i.blocks.saturating_sub(self.geo.spb);
            i.file_acl = 0;
        }
        i.links = 0;
        i.dtime = encode_time(self.now, false).0.max(1);
        if i.kind() == Some(FileType::Regular) {
            i.size = 0;
        }
        self.ind_flush()?;
        self.write_inode(ino, &i)?;
        self.free_blocks(&freed)?;
        self.free_inode(ino, dir)
    }

    /// Whether directory `dir` is `ancestor` or below it.
    fn is_within(&mut self, dir: u32, ancestor: u32) -> Result<bool> {
        let mut cur = dir;
        for _ in 0..MAX_DEPTH {
            if cur == ancestor {
                return Ok(true);
            }
            if cur == ROOT_INO {
                return Ok(false);
            }
            let i = self.read_inode(cur)?;
            if !i.is_dir() {
                return Err(self.corrupt());
            }
            let parent = match self.find_entry(&i, b"..")? {
                Some(s) => s.entry.inode,
                None => return Err(self.corrupt()),
            };
            if parent == cur {
                return Ok(false);
            }
            cur = parent;
        }
        Err(self.corrupt())
    }

    /// Points directory `ino`'s `..` at `parent`.
    fn set_parent(&mut self, i: &mut Inode, parent: u32) -> Result<()> {
        let slot = self.find_entry(i, b"..")?.ok_or_else(|| self.corrupt())?;
        self.retarget_slot(i, &slot, parent, FileType::Directory)
    }

    pub fn rename(
        &mut self,
        old_dir: u32,
        old_name: &[u8],
        new_dir: u32,
        new_name: &[u8],
        flags: RenameFlags,
    ) -> Result<()> {
        check_name(new_name)?;
        let same = old_dir == new_dir;
        let mut dirs = Dirs { old: self.live_dir(old_dir)?, new: None };
        if !same {
            dirs.new = Some(self.live_dir(new_dir)?);
        }
        let (src_slot, mut si) = self.entry_inode(&dirs.old, old_name)?;
        let src = src_slot.entry.inode;
        let src_kind = si.kind().unwrap_or(FileType::Regular);
        let src_dir = si.is_dir();
        if (dirs.old.flags | si.flags) & (FL_IMMUTABLE | FL_APPEND) != 0 || dirs.new_dir().flags & FL_IMMUTABLE != 0 {
            return Err(Errno::EPERM);
        }
        let nd = dirs.new_dir().clone();
        let dst = match self.find_entry(&nd, new_name)? {
            Some(slot) => {
                let i = self.read_inode(slot.entry.inode)?;
                if i.mode == 0 || i.kind().is_none() {
                    return Err(self.corrupt());
                }
                Some((slot, i))
            }
            None => None,
        };
        if src_dir && self.is_within(new_dir, src)? {
            return Err(Errno::EINVAL);
        }
        if flags.exchange {
            let (dst_slot, mut di) = dst.ok_or(Errno::ENOENT)?;
            let dst_ino = dst_slot.entry.inode;
            let dst_dir = di.is_dir();
            if dst_dir && self.is_within(old_dir, dst_ino)? {
                return Err(Errno::EINVAL);
            }
            if src == dst_ino {
                return Ok(());
            }
            if di.flags & (FL_IMMUTABLE | FL_APPEND) != 0 {
                return Err(Errno::EPERM);
            }
            let moves = !same && src_dir != dst_dir;
            if moves {
                let gaining = if src_dir { dirs.new_dir().links } else { dirs.old.links };
                if gaining >= LINK_MAX {
                    return Err(Errno::EMLINK);
                }
            }
            let dst_kind = di.kind().unwrap_or(FileType::Regular);
            // Both inodes (and what was written to them) durable before
            // either name moves.
            self.barrier()?;
            self.retarget_slot(dirs.new_dir(), &dst_slot, src, src_kind)?;
            if dst_slot.phys != src_slot.phys {
                self.barrier()?;
            }
            self.retarget_slot(&mut dirs.old, &src_slot, dst_ino, dst_kind)?;
            if !same && src_dir {
                self.set_parent(&mut si, new_dir)?;
            }
            if !same && dst_dir {
                self.set_parent(&mut di, old_dir)?;
            }
            if moves {
                if src_dir {
                    let n = dirs.new_dir();
                    n.links = n.links.saturating_add(1);
                    dirs.old.links = dirs.old.links.saturating_sub(1);
                } else {
                    dirs.old.links = dirs.old.links.saturating_add(1);
                    let n = dirs.new_dir();
                    n.links = n.links.saturating_sub(1);
                }
            }
            si.ctime = self.now;
            di.ctime = self.now;
            self.write_inode(src, &si)?;
            self.write_inode(dst_ino, &di)?;
            return self.finish_rename(old_dir, new_dir, dirs);
        }
        let replaced = match dst {
            Some((dst_slot, di)) => {
                if dst_slot.entry.inode == src {
                    return Ok(());
                }
                if flags.noreplace {
                    return Err(Errno::EEXIST);
                }
                match (src_dir, di.is_dir()) {
                    (true, false) => return Err(Errno::ENOTDIR),
                    (false, true) => return Err(Errno::EISDIR),
                    (true, true) if !self.dir_is_empty(&di)? => return Err(Errno::ENOTEMPTY),
                    _ => {}
                }
                if di.flags & (FL_IMMUTABLE | FL_APPEND) != 0 {
                    return Err(Errno::EPERM);
                }
                // The file taking the name over (its inode, its blocks and
                // the data written to them) is durable before the name
                // points to it: replacing a file by renaming a new one over
                // it never leaves the name pointing at nothing.
                self.barrier()?;
                self.retarget_slot(dirs.new_dir(), &dst_slot, src, src_kind)?;
                if dst_slot.phys != src_slot.phys {
                    self.barrier()?;
                }
                Some((dst_slot.entry.inode, di))
            }
            None => {
                if src_dir && !same && dirs.new_dir().links >= LINK_MAX {
                    return Err(Errno::EMLINK);
                }
                self.add_entry(dirs.new_dir(), new_name, src, src_kind)?;
                // The new name must be reachable and durable before the old
                // one goes: write the directory (it may have grown), then
                // a barrier.
                let nd = dirs.new_dir().clone();
                self.write_inode(new_dir, &nd)?;
                self.barrier()?;
                None
            }
        };
        // Inserting may have split the old entry's record: find it again.
        let src_slot = self.find_entry(&dirs.old, old_name)?.ok_or_else(|| self.corrupt())?;
        self.remove_slot(&mut dirs.old, &src_slot)?;
        if src_dir && !same {
            self.set_parent(&mut si, new_dir)?;
            dirs.old.links = dirs.old.links.saturating_sub(1);
            let n = dirs.new_dir();
            n.links = n.links.saturating_add(1);
        }
        si.ctime = self.now;
        self.write_inode(src, &si)?;
        if let Some((dst_ino, di)) = replaced {
            if di.is_dir() {
                let nd = dirs.new_dir();
                nd.links = nd.links.saturating_sub(1);
            }
            self.finish_rename(old_dir, new_dir, dirs)?;
            return self.drop_link(dst_ino, di);
        }
        self.finish_rename(old_dir, new_dir, dirs)
    }

    fn finish_rename(&mut self, old_dir: u32, new_dir: u32, mut dirs: Dirs) -> Result<()> {
        self.touch(&mut dirs.old);
        self.write_inode(old_dir, &dirs.old)?;
        if let Some(mut n) = dirs.new {
            self.touch(&mut n);
            self.write_inode(new_dir, &n)?;
        }
        Ok(())
    }

    pub fn readlink(&mut self, ino: u32) -> Result<Vec<u8>> {
        let i = self.live_inode(ino)?;
        if i.kind() != Some(FileType::Symlink) {
            return Err(Errno::EINVAL);
        }
        let len = i.size as usize;
        if self.is_fast_symlink(&i) {
            if len >= FAST_SYMLINK_MAX {
                return Err(self.corrupt());
            }
            return Ok(i.block_bytes()[..len].to_vec());
        }
        if i.size >= self.geo.bs as u64 {
            return Err(self.corrupt());
        }
        let b = self.bmap(&i, 0)?.ok_or_else(|| self.corrupt())?;
        let mut data = self.read_block(b)?;
        data.truncate(len);
        Ok(data)
    }

    fn regular(&mut self, ino: u32) -> Result<Inode> {
        let i = self.live_inode(ino)?;
        match i.kind() {
            Some(FileType::Regular) => Ok(i),
            Some(FileType::Directory) => Err(Errno::EISDIR),
            _ => Err(Errno::EINVAL),
        }
    }

    pub fn read(&mut self, ino: u32, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let mut i = self.regular(ino)?;
        if offset >= i.size {
            return Ok(0);
        }
        let n = buf.len().min((i.size - offset).min(usize::MAX as u64) as usize);
        let bs = self.geo.bs;
        let mut done = 0;
        while done < n {
            let pos = offset + done as u64;
            let lblk = pos / bs as u64;
            let in_blk = (pos % bs as u64) as usize;
            let chunk = (bs - in_blk).min(n - done);
            let phys = self.bmap(&i, lblk)?;
            match phys {
                None => buf[done..done + chunk].fill(0),
                Some(p) if chunk == bs => {
                    // Whole blocks: read a physically contiguous run at once.
                    let mut run = 1;
                    while done + (run + 1) * bs <= n && run < 64 {
                        match self.bmap(&i, lblk + run as u64)? {
                            Some(q) if q == p + run as u64 => run += 1,
                            _ => break,
                        }
                    }
                    self.disk.read(p, &mut buf[done..done + run * bs])?;
                    done += run * bs;
                    continue;
                }
                Some(p) => {
                    let data = self.read_block(p)?;
                    buf[done..done + chunk].copy_from_slice(&data[in_blk..in_blk + chunk]);
                }
            }
            done += chunk;
        }
        // Access time as relatime does: when older than the last change or
        // a day old.
        if !self.st.read_only && (i.atime <= i.mtime || i.atime <= i.ctime || i.atime.sec + 86400 <= self.now.sec) {
            i.atime = self.now;
            self.write_inode(ino, &i)?;
        }
        Ok(n)
    }

    /// Zeros the rest of the block holding byte `from` (bytes past the end
    /// of file that a later extension must read as zeros).
    fn zero_tail(&mut self, i: &Inode, from: u64) -> Result<()> {
        let bs = self.geo.bs as u64;
        if from.is_multiple_of(bs) {
            return Ok(());
        }
        if let Some(p) = self.bmap(i, from / bs)? {
            let mut data = self.read_block(p)?;
            data[(from % bs) as usize..].fill(0);
            self.disk.write(p, &data)?;
        }
        Ok(())
    }

    /// Records that a file of `size` bytes exists (`large_file`).
    fn note_size(&mut self, size: u64) {
        if size > i32::MAX as u64 && !self.st.large_file {
            self.st.large_file = true;
            let sb = &mut self.st.sb;
            sb.set_feature_ro_compat(sb.feature_ro_compat_raw() | RO_COMPAT_LARGE_FILE);
            self.st.meta_dirty = true;
            self.st.features_dirty = true;
        }
    }

    pub fn write(&mut self, ino: u32, offset: u64, buf: &[u8]) -> Result<usize> {
        let mut i = self.regular(ino)?;
        if i.flags & FL_IMMUTABLE != 0 || (i.flags & FL_APPEND != 0 && offset != i.size) {
            return Err(Errno::EPERM);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let end = offset.checked_add(buf.len() as u64).ok_or(Errno::EFBIG)?;
        if end > self.st.max_size {
            return Err(Errno::EFBIG);
        }
        if offset > i.size {
            self.zero_tail(&i, i.size)?;
        }
        let bs = self.geo.bs;
        let first = offset / bs as u64;
        let mut goal = match first.checked_sub(1) {
            Some(prev) => self.bmap(&i, prev)?.map(|p| p + 1),
            None => None,
        }
        .unwrap_or_else(|| self.group_data_goal(self.geo.group_of_inode(ino)));
        let mut done = 0;
        let mut error = None;
        while done < buf.len() {
            let pos = offset + done as u64;
            let in_blk = (pos % bs as u64) as usize;
            let chunk = (bs - in_blk).min(buf.len() - done);
            let (p, fresh) = match self.bmap_alloc(&mut i, pos / bs as u64, &mut goal) {
                Ok(x) => x,
                Err(e) => {
                    error = Some(e);
                    break;
                }
            };
            let r = if chunk == bs {
                self.disk.write(p, &buf[done..done + bs])
            } else {
                let mut data = if fresh { Ok(vec![0u8; bs]) } else { self.read_block(p) };
                if let Ok(d) = &mut data {
                    d[in_blk..in_blk + chunk].copy_from_slice(&buf[done..done + chunk]);
                }
                data.and_then(|d| self.disk.write(p, &d))
            };
            if let Err(e) = r {
                error = Some(e);
                break;
            }
            goal = p + 1;
            done += chunk;
        }
        if done > 0 {
            let new_end = offset + done as u64;
            if new_end > i.size {
                i.size = new_end;
                self.note_size(new_end);
            }
            self.touch(&mut i);
        }
        // Blocks allocated before a failure are attached: write them too.
        self.write_inode(ino, &i)?;
        match error {
            Some(e) if done == 0 => Err(e),
            _ => Ok(done),
        }
    }

    pub fn append(&mut self, ino: u32, buf: &[u8]) -> Result<(u64, usize)> {
        let at = self.regular(ino)?.size;
        Ok((at, self.write(ino, at, buf)?))
    }

    pub fn setattr(&mut self, ino: u32, set: &SetAttr) -> Result<Attr> {
        let mut i = self.live_inode(ino)?;
        if i.flags & FL_IMMUTABLE != 0 {
            return Err(Errno::EPERM);
        }
        let mut freed = Vec::new();
        if let Some(size) = set.size {
            match i.kind() {
                Some(FileType::Regular) => {}
                Some(FileType::Directory) => return Err(Errno::EISDIR),
                _ => return Err(Errno::EINVAL),
            }
            if size > self.st.max_size {
                return Err(Errno::EFBIG);
            }
            if i.flags & FL_APPEND != 0 && size != i.size {
                return Err(Errno::EPERM);
            }
            let bs = self.geo.bs as u64;
            if size < i.size {
                self.truncate_map(&mut i, size.div_ceil(bs), &mut freed)?;
                self.zero_tail(&i, size)?;
            } else if size > i.size {
                self.zero_tail(&i, i.size)?;
                self.note_size(size);
            }
            i.size = size;
            i.mtime = self.now;
        }
        if set.uid.is_some() || set.gid.is_some() {
            self.check_ids(set.uid.unwrap_or(i.uid), set.gid.unwrap_or(i.gid))?;
        }
        if let Some(perm) = set.perm {
            i.mode = (i.mode & !PERM_MASK) | (perm & PERM_MASK);
        }
        if let Some(uid) = set.uid {
            i.uid = uid;
        }
        if let Some(gid) = set.gid {
            i.gid = gid;
        }
        let now = self.now;
        let resolve = |t: SetTime| match t {
            SetTime::Now => now,
            SetTime::At(t) => t,
        };
        if let Some(t) = set.atime {
            i.atime = resolve(t);
        }
        if let Some(t) = set.mtime {
            i.mtime = resolve(t);
        }
        i.ctime = set.ctime.unwrap_or(now);
        self.ind_flush()?;
        self.write_inode(ino, &i)?;
        self.free_blocks(&freed)?;
        self.attr(ino, &i)
    }

    /// The entries of a live or removed directory from `pos` on.
    pub fn readdir(&mut self, dir: u32, pos: u64, max: usize) -> Result<Vec<crate::dir::Listed>> {
        let d = self.live_inode(dir)?;
        if !d.is_dir() {
            return Err(Errno::ENOTDIR);
        }
        if d.links == 0 {
            return Ok(Vec::new());
        }
        self.list_dir(&d, pos, max)
    }
}
