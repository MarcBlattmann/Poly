//! [`CachedFs`]: a file system seen through the page cache.

use crate::{Core, FsId, FsState, Key, PAGE, PAGE_SIZE, Page, PageCache, PageFrame, PageRef, READAHEAD_PAGES};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::Ordering::Relaxed;
use poly_vfs::lock::RawLock;
use poly_vfs::{
    Attr, DirEntry, Errno, FileSystem, FileType, FsInfo, Ino, MAX_FILE_SIZE, NewNode, RenameFlags, Result, SetAttr,
    SetTime,
};

/// A registered file system: what gets mounted in place of the inner one
/// ([`PageCache::register`]). Regular files' data goes through the cache;
/// the rest passes through to the inner file system.
pub struct CachedFs<F: PageFrame, L: RawLock + Default> {
    core: Arc<Core<F, L>>,
    st: Arc<FsState<L>>,
}

impl<F: PageFrame, L: RawLock + Default> core::fmt::Debug for CachedFs<F, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CachedFs").field("id", &self.st.id).field("fs", &self.st.inner.info().name).finish()
    }
}

impl<F: PageFrame, L: RawLock + Default> CachedFs<F, L> {
    pub(crate) fn new(core: Arc<Core<F, L>>, st: Arc<FsState<L>>) -> CachedFs<F, L> {
        CachedFs { core, st }
    }

    /// The id its pages carry in their [`Key`]s.
    pub fn id(&self) -> FsId {
        self.st.id
    }

    /// The file system behind the cache.
    pub fn inner(&self) -> &Arc<dyn FileSystem> {
        &self.st.inner
    }

    /// The cache it is registered with.
    pub fn cache(&self) -> PageCache<F, L> {
        PageCache { core: Arc::clone(&self.core) }
    }

    /// Whether anything changed (data or metadata) since the last call: the
    /// file system needs a `sync`.
    pub fn take_changed(&self) -> bool {
        self.st.changed.swap(false, Relaxed)
    }

    fn changed(&self) {
        self.st.changed.store(true, Relaxed);
    }

    fn key(&self, ino: Ino, index: u64) -> Key {
        Key::new(self.st.id, ino, index)
    }

    /// Page `index` of regular file `ino`, read from the file system if it
    /// is not cached: the frame `read` and `write` use too, for a memory
    /// mapping. `ENXIO` if the page lies wholly past the end of the file,
    /// `EINVAL` for anything but a regular file.
    pub fn page(&self, ino: Ino, index: u64) -> Result<PageRef<F, L>> {
        let lock = self.core.inode_lock(self.st.id, ino);
        let _held = lock.lock();
        let attr = self.st.inner.getattr(ino)?;
        if attr.kind != FileType::Regular {
            return Err(Errno::EINVAL);
        }
        if index >= attr.size.div_ceil(PAGE) {
            return Err(Errno::ENXIO);
        }
        let page = self.get_or_fill(ino, index, attr.size)?;
        Ok(PageRef::new(page, Arc::clone(&self.core)))
    }

    /// Page `index` of `ino` if it is cached; nothing is read.
    pub fn cached_page(&self, ino: Ino, index: u64) -> Option<PageRef<F, L>> {
        self.cache().cached(&self.key(ino, index))
    }

    /// Write back this file system's dirty pages (all, or those of one
    /// inode) to the inner file system, without syncing it. Returns how
    /// many pages were written.
    pub fn writeback(&self, only: Option<Ino>) -> Result<usize> {
        self.core.writeback(&self.st, only)
    }

    /// Drop this file system's clean pages that nobody holds. Returns how
    /// many went.
    pub fn evict_clean(&self) -> usize {
        let keys: Vec<Ino> = {
            let index = self.core.index.lock();
            let lo = Key::new(self.st.id, 0, 0);
            let hi = Key::new(self.st.id, Ino::MAX, u64::MAX);
            let mut inos: Vec<Ino> = index.pages.range(lo..=hi).map(|(k, _)| k.ino).collect();
            inos.dedup();
            inos
        };
        let mut evicted = 0;
        for ino in keys {
            let lock = self.core.inode_lock(self.st.id, ino);
            let _held = lock.lock();
            let dropped: Vec<Arc<Page<F>>> = {
                let mut index = self.core.index.lock();
                let victims: Vec<Key> = index
                    .keys_of(self.st.id, ino, 0)
                    .into_iter()
                    .filter(|k| {
                        let page = &index.pages[k].page;
                        !page.is_dirty() && Arc::strong_count(page) == 1
                    })
                    .collect();
                victims.iter().filter_map(|k| index.remove(k)).collect()
            };
            evicted += dropped.len();
        }
        self.core.c.evicted.fetch_add(evicted as u64, Relaxed);
        evicted
    }

    /// Pages of this file system in the cache, and how many are dirty.
    pub fn page_counts(&self) -> (usize, usize) {
        let index = self.core.index.lock();
        let lo = Key::new(self.st.id, 0, 0);
        let hi = Key::new(self.st.id, Ino::MAX, u64::MAX);
        (index.pages.range(lo..=hi).count(), index.dirty.range(lo..=hi).count())
    }

    /// The cached page for `index`, or read it (and up to
    /// [`READAHEAD_PAGES`] - 1 uncached pages after it, within `size`) from
    /// the inner file system. The caller holds the inode's lock and knows
    /// the page starts before `size`.
    fn get_or_fill(&self, ino: Ino, index: u64, size: u64) -> Result<Arc<Page<F>>> {
        if let Some(p) = self.core.lookup(&self.key(ino, index)) {
            return Ok(p);
        }
        let end = size.div_ceil(PAGE);
        let mut run = 1;
        while run < READAHEAD_PAGES
            && index + (run as u64) < end
            && !self.core.is_cached(&self.key(ino, index + run as u64))
        {
            run += 1;
        }
        let mut frames = Vec::with_capacity(run);
        for i in 0..run {
            match self.core.alloc_frame() {
                Ok(f) => frames.push(f),
                Err(e) if i == 0 => return Err(e),
                // Read ahead only as far as memory allows.
                Err(_) => break,
            }
        }
        let start = index * PAGE;
        let len = ((frames.len() as u64 * PAGE).min(size - start)) as usize;
        let mut buf = vec![0u8; len];
        crate::read_full(&*self.st.inner, ino, start, &mut buf)?;
        let count = frames.len();
        let mut first = None;
        for (i, frame) in frames.into_iter().enumerate() {
            let at = i * PAGE_SIZE;
            let page = self.core.insert(self.key(ino, index + i as u64), frame);
            page.write(0, &buf[at..(at + PAGE_SIZE).min(len)]);
            first.get_or_insert(page);
        }
        self.core.count_filled(count);
        Ok(first.expect("at least one frame"))
    }

    /// A page to overwrite part of: cached, or a fresh zeroed one when
    /// nothing of it is on the inner file system (`fresh`), or read.
    fn page_for_write(&self, ino: Ino, index: u64, size: u64, fresh: bool) -> Result<Arc<Page<F>>> {
        if !fresh {
            return self.get_or_fill(ino, index, size);
        }
        if let Some(p) = self.core.lookup(&self.key(ino, index)) {
            return Ok(p);
        }
        let frame = self.core.alloc_frame()?;
        Ok(self.core.insert(self.key(ino, index), frame))
    }

    /// Zero the cached last page of a file of `size` bytes past its end:
    /// what the file holds there must read as zeros once it grows, whatever
    /// a mapping wrote there meanwhile. The caller holds the inode's lock.
    fn zero_tail(&self, ino: Ino, size: u64) {
        let in_page = (size % PAGE) as usize;
        if in_page == 0 {
            return;
        }
        if let Some(page) = self.core.index.lock().get(&self.key(ino, size / PAGE)) {
            page.zero_from(in_page);
        }
    }

    /// `write` with the inode's lock held.
    fn write_locked(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        let attr = self.st.inner.getattr(ino)?;
        if attr.kind != FileType::Regular {
            return self.st.inner.write(ino, offset, buf);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let end = offset.checked_add(buf.len() as u64).filter(|&e| e <= MAX_FILE_SIZE).ok_or(Errno::EFBIG)?;
        self.changed();
        let old = attr.size;
        if end > old {
            self.zero_tail(ino, old);
        }
        // Size and times now, so `stat` is right; the data at writeback.
        let set = SetAttr { size: (end > old).then_some(end), mtime: Some(SetTime::Now), ..SetAttr::default() };
        let size = self.st.inner.setattr(ino, &set)?.size;
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done as u64;
            let (index, in_page) = (pos / PAGE, (pos % PAGE) as usize);
            let n = (PAGE_SIZE - in_page).min(buf.len() - done);
            // Nothing to read if the write covers the page or the page lies
            // wholly past the old end of the file.
            let fresh = n == PAGE_SIZE || index * PAGE >= old;
            let page = match self.page_for_write(ino, index, size, fresh) {
                Ok(p) => p,
                Err(e) if done == 0 => return Err(e),
                Err(_) => break,
            };
            page.write(in_page, &buf[done..done + n]);
            self.core.mark_dirty(&page);
            done += n;
        }
        Ok(done)
    }

    /// Drop the pages of `ino` if the inner file system no longer has the
    /// inode. The caller holds the namespace lock.
    fn forget_if_gone(&self, ino: Ino) {
        if !self.core.has_inode(self.st.id, ino) {
            return;
        }
        if matches!(self.st.inner.getattr(ino), Err(Errno::ENOENT)) {
            self.discard_inode(ino);
        }
    }

    fn discard_inode(&self, ino: Ino) {
        if !self.core.has_inode(self.st.id, ino) {
            return;
        }
        let lock = self.core.inode_lock(self.st.id, ino);
        let _held = lock.lock();
        self.core.discard(self.st.id, ino, 0);
    }
}

impl<F: PageFrame, L: RawLock + Default> Drop for CachedFs<F, L> {
    fn drop(&mut self) {
        self.core.unregister(self.st.id);
    }
}

impl<F: PageFrame, L: RawLock + Default> FileSystem for CachedFs<F, L> {
    fn info(&self) -> FsInfo {
        self.st.inner.info()
    }

    fn root(&self) -> Ino {
        self.st.inner.root()
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        self.st.inner.lookup(dir, name)
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        self.st.inner.getattr(ino)
    }

    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        self.changed();
        let Some(size) = set.size else { return self.st.inner.setattr(ino, set) };
        let lock = self.core.inode_lock(self.st.id, ino);
        let _held = lock.lock();
        let old = self.st.inner.getattr(ino)?;
        if old.kind == FileType::Regular && size > old.size {
            self.zero_tail(ino, old.size);
        }
        let attr = self.st.inner.setattr(ino, set)?;
        if attr.kind == FileType::Regular && size < old.size {
            self.core.discard(self.st.id, ino, size.div_ceil(PAGE));
            self.zero_tail(ino, size);
        }
        Ok(attr)
    }

    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        self.changed();
        let _ns = self.st.ns.lock();
        let ino = self.st.inner.create(dir, name, node)?;
        // A new inode has no pages; any left from an earlier inode with
        // this number would be stale (they were dropped when it went, so
        // this is only a safety net).
        self.discard_inode(ino);
        Ok(ino)
    }

    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino> {
        self.changed();
        let _ns = self.st.ns.lock();
        let ino = self.st.inner.mkdir(dir, name, perm, uid, gid)?;
        self.discard_inode(ino);
        Ok(ino)
    }

    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino> {
        self.changed();
        let _ns = self.st.ns.lock();
        let ino = self.st.inner.symlink(dir, name, target, uid, gid)?;
        self.discard_inode(ino);
        Ok(ino)
    }

    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        self.changed();
        let _ns = self.st.ns.lock();
        self.st.inner.link(dir, name, ino)
    }

    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.changed();
        let _ns = self.st.ns.lock();
        let victim = self.st.inner.lookup(dir, name).ok();
        self.st.inner.unlink(dir, name)?;
        if let Some(v) = victim {
            self.forget_if_gone(v);
        }
        Ok(())
    }

    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.changed();
        let _ns = self.st.ns.lock();
        self.st.inner.rmdir(dir, name)
    }

    fn rename(&self, old_dir: Ino, old_name: &[u8], new_dir: Ino, new_name: &[u8], flags: RenameFlags) -> Result<()> {
        self.changed();
        let _ns = self.st.ns.lock();
        // A file renamed over another replaces it: the other may go.
        let victim =
            if flags.exchange || flags.noreplace { None } else { self.st.inner.lookup(new_dir, new_name).ok() };
        self.st.inner.rename(old_dir, old_name, new_dir, new_name, flags)?;
        if let Some(v) = victim {
            self.forget_if_gone(v);
        }
        Ok(())
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        self.st.inner.readlink(ino)
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let lock = self.core.inode_lock(self.st.id, ino);
        let _held = lock.lock();
        let attr = self.st.inner.getattr(ino)?;
        if attr.kind != FileType::Regular {
            return self.st.inner.read(ino, offset, buf);
        }
        if offset >= attr.size || buf.is_empty() {
            return Ok(0);
        }
        let len = (buf.len() as u64).min(attr.size - offset) as usize;
        let mut done = 0;
        while done < len {
            let pos = offset + done as u64;
            let (index, in_page) = (pos / PAGE, (pos % PAGE) as usize);
            let n = (PAGE_SIZE - in_page).min(len - done);
            let page = match self.get_or_fill(ino, index, attr.size) {
                Ok(p) => p,
                Err(e) if done == 0 => return Err(e),
                Err(_) => break,
            };
            page.read(in_page, &mut buf[done..done + n]);
            done += n;
        }
        Ok(done)
    }

    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        let lock = self.core.inode_lock(self.st.id, ino);
        let _held = lock.lock();
        self.write_locked(ino, offset, buf)
    }

    fn append(&self, ino: Ino, buf: &[u8]) -> Result<(u64, usize)> {
        let lock = self.core.inode_lock(self.st.id, ino);
        let _held = lock.lock();
        let at = self.st.inner.getattr(ino)?.size;
        Ok((at, self.write_locked(ino, at, buf)?))
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        self.st.inner.readdir(dir, pos, emit)
    }

    fn open(&self, ino: Ino) -> Result<()> {
        self.st.inner.open(ino)
    }

    fn release(&self, ino: Ino) {
        if !self.core.has_inode(self.st.id, ino) {
            self.st.inner.release(ino);
            return;
        }
        // The last pin of an unlinked file deletes it: drop its pages under
        // the namespace lock, before anything can reuse the number.
        let _ns = self.st.ns.lock();
        self.st.inner.release(ino);
        self.forget_if_gone(ino);
    }

    /// Write back the dirty pages, then sync the inner file system. Reports
    /// the first writeback error since the last `sync` (also from the
    /// writeback thread), once.
    fn sync(&self) -> Result<()> {
        let written = self.writeback(None);
        let synced = self.st.inner.sync();
        let earlier = self.st.error.lock().take();
        written?;
        synced?;
        earlier.map_or(Ok(()), Err)
    }

    /// The inner file system's counts (dirty pages not yet written back
    /// have no blocks yet, so free space may be overstated).
    fn statfs(&self) -> Result<poly_vfs::FsStats> {
        self.st.inner.statfs()
    }
}
