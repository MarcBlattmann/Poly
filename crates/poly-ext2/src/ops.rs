//! The file system operations, with the `FileSystem` trait's semantics, on
//! one locked [`Ctx`]. Every operation reads the inodes it needs, changes
//! copies and writes them back; nothing is cached between operations except
//! pins, orphans and the allocation state.
//!
//! Write order inside an operation follows `balloc.rs`: link counts go up
//! before a new name is written and down after a name is removed (a crash
//! leaves a count too high, which e2fsck lowers, never too low); blocks and
//! inodes are freed after the writes that unhook them. With a journal the
//! whole operation is in one transaction anyway, except truncations and
//! writes too big for one, which commit between consistent steps (a file
//! being cut down sits on the orphan list meanwhile, so a crash in between
//! is finished at the next mount).
//!
//! **Orphans.** A file whose last name goes while it is open stays until
//! its last release; with a journal it is put on the on-disk orphan list
//! (`s_last_orphan`, linked through `i_dtime`) in the same transaction, so
//! a crash cannot leak it: mounting deletes what is on the list (and
//! finishes truncations of files still linked).

use crate::dir::Slot;
use crate::extent::{Lookup, MAX_LBLK};
use crate::inode::encode_rdev;
use crate::layout::RO_COMPAT_LARGE_FILE;
use crate::layout::{
    FAST_SYMLINK_MAX, FL_APPEND, FL_EXTENTS, FL_IMMUTABLE, FL_INLINE_DATA, Inode, ROOT_INO, encode_time,
};
use crate::{BlockIo, Ctx};
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{Attr, Errno, FileType, NAME_MAX, NewNode, PERM_MASK, RenameFlags, Result, SetAttr, SetTime};

/// Deepest directory nesting `rename` walks up through before deciding the
/// volume has a loop.
const MAX_DEPTH: usize = 1 << 16;
/// Logical blocks a journaled truncation frees per step.
const TRUNCATE_STEP: u64 = 8192;

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

/// How a run of logical blocks reads.
enum Run {
    /// From this disk block on.
    Data(u64),
    /// Zeros.
    Zeros,
}

impl<D: BlockIo> Ctx<'_, D> {
    fn touch(&self, i: &mut Inode) {
        i.mtime = self.now;
        i.ctime = self.now;
    }

    /// One more subdirectory in `d` (`dir_nlink`: past the limit, the
    /// count becomes 1, "many").
    fn inc_dir_links(&self, d: &mut Inode) {
        if self.geo.dir_nlink && (d.links == 1 || d.links >= self.geo.link_max - 1) {
            d.links = 1;
        } else {
            d.links = d.links.saturating_add(1);
        }
    }

    fn dec_dir_links(&self, d: &mut Inode) {
        if !(self.geo.dir_nlink && d.links == 1) {
            d.links = d.links.saturating_sub(1).max(2);
        }
    }

    /// Whether `d` may get another subdirectory.
    fn dir_link_room(&self, d: &Inode) -> bool {
        self.geo.dir_nlink || d.links < self.geo.link_max
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

    fn new_inode(&mut self, ino: u32, kind: FileType, perm: u16, uid: u32, gid: u32) -> Inode {
        self.st.next_generation = self.st.next_generation.wrapping_add(1);
        let mut i = Inode {
            ino,
            mode: kind.mode_bits() as u16 | (perm & PERM_MASK),
            uid,
            gid,
            atime: self.now,
            ctime: self.now,
            mtime: self.now,
            links: 1,
            generation: self.st.next_generation,
            ..Inode::default()
        };
        if self.geo.extents && matches!(kind, FileType::Regular | FileType::Directory) {
            self.ext_init(&mut i);
        }
        i
    }

    /// Checks that `name` can be added to `dir`.
    fn check_new(&mut self, dir: &Inode, name: &[u8]) -> Result<()> {
        check_name(name)?;
        if dir.flags & FL_IMMUTABLE != 0 {
            return Err(Errno::EPERM);
        }
        if dir.flags & FL_INLINE_DATA != 0 {
            return Err(Errno::EROFS);
        }
        if self.find_entry(dir, name)?.is_some() {
            return Err(Errno::EEXIST);
        }
        Ok(())
    }

    /// Takes back an inode that never got a name.
    fn abandon(&mut self, ino: u32, i: Inode) -> Result<()> {
        self.evict(ino, i)
    }

    /// Adds the entry for a just-initialised inode, or takes the inode
    /// back; then updates the directory.
    fn link_new(&mut self, dir: u32, d: &mut Inode, name: &[u8], ino: u32, i: Inode) -> Result<()> {
        let kind = i.kind().unwrap_or(FileType::Regular);
        if let Err(e) = self.add_entry(d, name, ino, kind) {
            self.abandon(ino, i)?;
            return Err(e);
        }
        if kind == FileType::Directory {
            self.inc_dir_links(d);
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
        let mut i = self.new_inode(ino, node.kind, node.perm, node.uid, node.gid);
        if matches!(node.kind, FileType::CharDevice | FileType::BlockDevice) {
            encode_rdev(node.rdev, &mut i.block);
        }
        self.init_inode(ino, &i)?;
        self.link_new(dir, &mut d, name, ino, i)?;
        Ok(ino)
    }

    pub fn mkdir(&mut self, dir: u32, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<u32> {
        self.check_ids(uid, gid)?;
        let mut d = self.live_dir(dir)?;
        self.check_new(&d, name)?;
        if !self.dir_link_room(&d) {
            return Err(Errno::EMLINK);
        }
        let ino = self.alloc_inode(dir, true)?;
        let mut i = self.new_inode(ino, FileType::Directory, perm, uid, gid);
        i.links = 2;
        let mut goal = self.group_data_goal(self.geo.group_of_inode(ino));
        let block = match self.bmap_alloc(&mut i, 0, &mut goal) {
            Ok((b, _)) => b,
            Err(e) => {
                self.init_inode(ino, &i)?;
                self.abandon(ino, i)?;
                return Err(e);
            }
        };
        let mut data = self.new_dir_block(ino, dir);
        self.write_new_dir_block(&i, block, &mut data)?;
        i.size = self.geo.bs as u64;
        self.ind_flush()?;
        self.init_inode(ino, &i)?;
        self.link_new(dir, &mut d, name, ino, i)?;
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
        let mut i = self.new_inode(ino, FileType::Symlink, 0o777, uid, gid);
        i.size = target.len() as u64;
        if target.len() < FAST_SYMLINK_MAX {
            i.set_block_bytes(target);
        } else {
            if self.geo.extents {
                self.ext_init(&mut i);
            }
            let mut goal = self.group_data_goal(self.geo.group_of_inode(ino));
            let block = match self.bmap_alloc(&mut i, 0, &mut goal) {
                Ok((b, _)) => b,
                Err(e) => {
                    self.init_inode(ino, &i)?;
                    self.abandon(ino, i)?;
                    return Err(e);
                }
            };
            let mut data = vec![0u8; self.geo.bs];
            data[..target.len()].copy_from_slice(target);
            self.write_fresh(block, &data)?;
        }
        self.init_inode(ino, &i)?;
        self.link_new(dir, &mut d, name, ino, i)?;
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
        if t.links >= self.geo.link_max {
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
        self.dec_dir_links(&mut d);
        self.touch(&mut d);
        self.write_inode(dir, &d)?;
        self.drop_link(slot.entry.inode, i)
    }

    /// Drops one name of `ino` (a directory loses its only one), freeing it
    /// if that was the last and nothing pins it (else it becomes an orphan).
    fn drop_link(&mut self, ino: u32, mut i: Inode) -> Result<()> {
        i.links = if i.is_dir() { 0 } else { i.links.saturating_sub(1) };
        i.ctime = self.now;
        if i.links > 0 {
            return self.write_inode(ino, &i);
        }
        if !self.pinned(ino) {
            return self.evict(ino, i);
        }
        self.st.orphans.insert(ino);
        if self.st.journal.is_some() {
            self.orphan_add(ino, &mut i);
        }
        self.write_inode(ino, &i)
    }

    /// Deletes an orphan whose last pin went (or at unmount).
    pub fn release_orphan(&mut self, ino: u32) -> Result<()> {
        self.st.orphans.remove(&ino);
        let i = self.read_inode(ino)?;
        if i.mode != 0 && i.links == 0 { self.evict(ino, i) } else { Ok(()) }
    }

    // ----- the on-disk orphan list (with a journal) -----

    /// Puts `ino` at the head of the orphan list (`i` written by the caller).
    fn orphan_add(&mut self, ino: u32, i: &mut Inode) {
        i.dtime = self.st.sb.last_orphan();
        self.st.sb.set_last_orphan(ino);
        self.st.meta_dirty = true;
    }

    /// Takes `ino` off the orphan list if it is on it (`i` written by the
    /// caller).
    fn orphan_remove(&mut self, ino: u32, i: &mut Inode) -> Result<()> {
        let mut prev: Option<u32> = None;
        let mut cur = self.st.sb.last_orphan();
        for _ in 0..self.geo.inodes {
            if cur == 0 {
                return Ok(());
            }
            if cur == ino {
                let next = i.dtime;
                match prev {
                    None => {
                        self.st.sb.set_last_orphan(next);
                        self.st.meta_dirty = true;
                    }
                    Some(p) => {
                        let mut pi = self.read_inode(p)?;
                        pi.dtime = next;
                        self.write_inode(p, &pi)?;
                    }
                }
                i.dtime = 0;
                return Ok(());
            }
            prev = Some(cur);
            cur = self.read_inode(cur)?.dtime;
        }
        Err(self.corrupt())
    }

    /// Mounting: deletes the orphans with no links, finishes truncating the
    /// others. Returns how many there were.
    pub fn process_orphans(&mut self) -> Result<u32> {
        let mut cur = self.st.sb.last_orphan();
        if cur == 0 {
            return Ok(0);
        }
        self.st.sb.set_last_orphan(0);
        self.st.meta_dirty = true;
        let mut n = 0;
        while cur != 0 && n < self.geo.inodes {
            if self.ino(cur as u64).is_err() {
                self.corrupt();
                break;
            }
            let mut i = match self.read_inode(cur) {
                Ok(i) => i,
                Err(Errno::EUCLEAN | Errno::EBADMSG) => break,
                Err(e) => return Err(e),
            };
            let next = i.dtime;
            n += 1;
            if i.mode == 0 {
                break;
            }
            i.dtime = 0;
            if i.links == 0 {
                self.evict(cur, i)?;
            } else {
                let keep = i.size.div_ceil(self.geo.bs as u64);
                if self.has_block_map(&i) {
                    self.cut(cur, &mut i, keep)?;
                } else {
                    self.write_inode(cur, &i)?;
                }
            }
            self.ind_flush()?;
            self.flush_bitmaps()?;
            cur = next;
        }
        Ok(n)
    }

    /// Frees the blocks of inode `ino` (`i`) from logical block `keep` on
    /// and writes the inode. With a journal, extent-mapped files are cut in
    /// steps from the end, committing between steps when the transaction
    /// fills up (the inode on the orphan list meanwhile).
    fn cut(&mut self, ino: u32, i: &mut Inode, keep: u64) -> Result<()> {
        let stepped = self.st.journal.is_some() && i.flags & FL_EXTENTS != 0;
        let mut end = if stepped { self.ext_end(i)? } else { keep };
        let mut listed = false;
        loop {
            let from = if stepped { end.saturating_sub(TRUNCATE_STEP).max(keep) } else { keep };
            let mut freed = Vec::new();
            self.truncate_map(i, from, &mut freed)?;
            self.ind_flush()?;
            self.write_inode(ino, i)?;
            self.free_blocks(&freed)?;
            if from <= keep {
                break;
            }
            end = from;
            if self.journal_tight() {
                if !listed && !self.st.orphans.contains(&ino) {
                    self.orphan_add(ino, i);
                    self.write_inode(ino, i)?;
                    listed = true;
                }
                self.sync()?;
            }
        }
        if listed {
            self.orphan_remove(ino, i)?;
            self.write_inode(ino, i)?;
        }
        Ok(())
    }

    /// Deletes an inode nothing names or pins: frees its blocks and itself.
    pub fn evict(&mut self, ino: u32, mut i: Inode) -> Result<()> {
        let dir = i.is_dir();
        i.links = 0;
        if self.st.journal.is_some() {
            self.orphan_remove(ino, &mut i)?;
        }
        if self.has_block_map(&i) {
            self.cut(ino, &mut i, 0)?;
        }
        let mut freed = Vec::new();
        if i.file_acl != 0 {
            match self.release_xattr(i.file_acl) {
                Ok(Some(b)) => freed.push(b),
                Ok(None) | Err(Errno::EUCLEAN) => {}
                Err(e) => return Err(e),
            }
            i.blocks = i.blocks.saturating_sub(self.geo.spb as u64);
            i.file_acl = 0;
        }
        // A deletion time below the inode count would read as an orphan
        // list link (e2fsck takes it for one): never write one that small,
        // whatever the clock says.
        i.dtime = encode_time(self.now, false).0.max(self.geo.inodes.saturating_add(1));
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
        if (dirs.old.flags | dirs.new_dir().flags) & FL_INLINE_DATA != 0 {
            return Err(Errno::EROFS);
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
                let gaining = if src_dir { dirs.new_dir().clone() } else { dirs.old.clone() };
                if !self.dir_link_room(&gaining) {
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
                    let mut n = dirs.new_dir().clone();
                    self.inc_dir_links(&mut n);
                    *dirs.new_dir() = n;
                    let mut o = dirs.old.clone();
                    self.dec_dir_links(&mut o);
                    dirs.old = o;
                } else {
                    let mut o = dirs.old.clone();
                    self.inc_dir_links(&mut o);
                    dirs.old = o;
                    let mut n = dirs.new_dir().clone();
                    self.dec_dir_links(&mut n);
                    *dirs.new_dir() = n;
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
                if src_dir && !same && !self.dir_link_room(&dirs.new_dir().clone()) {
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
            let mut o = dirs.old.clone();
            self.dec_dir_links(&mut o);
            dirs.old = o;
            let mut n = dirs.new_dir().clone();
            self.inc_dir_links(&mut n);
            *dirs.new_dir() = n;
        }
        si.ctime = self.now;
        self.write_inode(src, &si)?;
        if let Some((dst_ino, di)) = replaced {
            if di.is_dir() {
                let mut n = dirs.new_dir().clone();
                self.dec_dir_links(&mut n);
                *dirs.new_dir() = n;
            }
            self.finish_rename(old_dir, new_dir, dirs)?;
            // The name points elsewhere on disk before the replaced inode
            // can lose its last link (else a crash could leave the name
            // pointing at a deleted inode).
            self.barrier()?;
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
        if i.flags & FL_INLINE_DATA != 0 {
            let mut data = self.inline_data(&i)?;
            if len > data.len() {
                return Err(self.corrupt());
            }
            data.truncate(len);
            return Ok(data);
        }
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

    /// How logical block `lblk` reads, and for how many blocks (at most
    /// `max`) the same way.
    fn run(&mut self, i: &Inode, lblk: u64, max: u64) -> Result<(Run, u64)> {
        if i.flags & FL_EXTENTS != 0 {
            return Ok(match self.ext_lookup(i, lblk)? {
                Lookup::Mapped(m) if m.uninit => (Run::Zeros, m.run.min(max)),
                Lookup::Mapped(m) => (Run::Data(m.pblk), m.run.min(max)),
                Lookup::Hole { until } => (Run::Zeros, until.saturating_sub(lblk).clamp(1, max)),
            });
        }
        let Some(p) = self.bmap(i, lblk)? else { return Ok((Run::Zeros, 1)) };
        let mut n = 1;
        while n < max {
            match self.bmap(i, lblk + n)? {
                Some(q) if q == p + n => n += 1,
                _ => break,
            }
        }
        Ok((Run::Data(p), n))
    }

    pub fn read(&mut self, ino: u32, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let mut i = self.regular(ino)?;
        if offset >= i.size {
            return Ok(0);
        }
        let n = buf.len().min((i.size - offset).min(usize::MAX as u64) as usize);
        if i.flags & FL_INLINE_DATA != 0 {
            let data = self.inline_data(&i)?;
            let end = offset as usize + n;
            if end > data.len() {
                return Err(self.corrupt());
            }
            buf[..n].copy_from_slice(&data[offset as usize..end]);
            return Ok(n);
        }
        let bs = self.geo.bs;
        let mut done = 0;
        while done < n {
            let pos = offset + done as u64;
            let lblk = pos / bs as u64;
            let in_blk = (pos % bs as u64) as usize;
            let chunk = (bs - in_blk).min(n - done);
            if chunk == bs {
                // Whole blocks: a physically contiguous run at once.
                let whole = ((n - done) / bs).min(64) as u64;
                let (run, count) = self.run(&i, lblk, whole)?;
                let len = count as usize * bs;
                match run {
                    Run::Zeros => buf[done..done + len].fill(0),
                    Run::Data(p) => self.disk.read(p, &mut buf[done..done + len])?,
                }
                done += len;
                continue;
            }
            match self.run(&i, lblk, 1)?.0 {
                Run::Zeros => buf[done..done + chunk].fill(0),
                Run::Data(p) => {
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
        if i.flags & FL_INLINE_DATA != 0 {
            return Err(Errno::EROFS);
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
            if pos / bs as u64 >= MAX_LBLK && i.flags & FL_EXTENTS != 0 {
                error = Some(Errno::EFBIG);
                break;
            }
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
            if self.journal_tight() && done < buf.len() {
                // Too much for one transaction: what is written so far
                // becomes durable as a consistent step.
                let new_end = offset + done as u64;
                if new_end > i.size {
                    i.size = new_end;
                    self.note_size(new_end);
                }
                self.ind_flush()?;
                self.write_inode(ino, &i)?;
                self.sync()?;
            }
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
        self.ind_flush()?;
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
        if set.uid.is_some() || set.gid.is_some() {
            self.check_ids(set.uid.unwrap_or(i.uid), set.gid.unwrap_or(i.gid))?;
        }
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
            if i.flags & FL_INLINE_DATA != 0 {
                return Err(Errno::EROFS);
            }
            let bs = self.geo.bs as u64;
            if size < i.size {
                i.size = size;
                i.mtime = self.now;
                i.ctime = self.now;
                self.cut(ino, &mut i, size.div_ceil(bs))?;
                self.zero_tail(&i, size)?;
            } else if size > i.size {
                self.zero_tail(&i, i.size)?;
                self.note_size(size);
            }
            i.size = size;
            i.mtime = self.now;
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
