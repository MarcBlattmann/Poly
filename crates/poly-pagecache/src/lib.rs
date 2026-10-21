//! The page cache: file contents in page-sized frames, per inode, between
//! the VFS and a disk file system.
//!
//! A [`PageCache`] holds pages of every file system registered with it,
//! keyed by file system, inode and page index ([`Key`]). Registering a file
//! system ([`PageCache::register`]) gives a [`CachedFs`]: a
//! [`poly_vfs::FileSystem`] that wraps the real one and is what gets
//! mounted. Its `read` and `write` go through the cache; everything else
//! passes through (with the bookkeeping the cache needs when files go away
//! or shrink). Memory mappings take pages with [`CachedFs::page`], which
//! hands out the *same* frame `read` and `write` use: a [`PageRef`] that
//! keeps the page in memory while it is mapped.
//!
//! ```text
//!   Vfs ── OpenFile::read/write ──▶ CachedFs ──▶ inner FileSystem (ext2) ──▶ disk
//!                                     │  ▲ fill (with read-ahead) / writeback
//!   user mapping ── CachedFs::page ──▶ PageCache: Key → Page { frame, dirty }
//! ```
//!
//! - **Reads** copy from cached pages; a miss reads the page and up to
//!   [`READAHEAD_PAGES`] - 1 uncached pages after it from the inner file
//!   system in one call.
//! - **Writes** copy into cached pages (read first unless the write covers
//!   the page or the page lies wholly past the old end of the file), mark
//!   them dirty and update the size and modification time in the inner
//!   file system at once, so `stat` is always right. The data reaches the
//!   inner file system at **writeback**: [`CachedFs::writeback`],
//!   [`PageCache::writeback_all`] (a kernel thread calls it periodically)
//!   and [`FileSystem::sync`], which then syncs the inner file system too.
//!   Blocks are therefore allocated at writeback ("delayed allocation"): a
//!   full disk shows as an error from the next `sync`, not from `write`.
//! - **Mappings**: a page mapped writable is dirty for as long as it is
//!   mapped so ([`PageRef::map_writable`]), since the CPU writes it behind
//!   the cache's back; writeback writes it each time and keeps it dirty
//!   until the last writable mapping goes.
//! - **Eviction**: clean pages nobody else references (no mapping, no
//!   copy in progress) are dropped least recently used first when the cache
//!   is over its limit ([`PageCache::shrink`]), and when a frame cannot be
//!   allocated. Dropping a page drops its frame, which the kernel's
//!   [`PageFrame`] gives back to the frame allocator.
//! - **Coherence**: every read and write of a file's data goes through
//!   here, so the cache is the truth for file contents. Truncation drops
//!   the pages past the new end and zeroes the rest of the last one; a
//!   file whose inode is deleted (last name removed and last pin dropped)
//!   loses its pages before anything can reuse the inode number.
//!
//! # Locking
//!
//! - The index (pages, LRU order, the dirty set) is behind a spinlock held
//!   only for in-memory work, never across a call into a file system.
//! - Each inode has a lock of type `L` (the kernel's sleeping mutex) held
//!   across everything that reads or changes that file's pages, including
//!   the inner file system's I/O: reads, writes, truncation, writeback,
//!   discarding. So a page is filled once, and writeback never races a
//!   write or a truncation of the same file.
//! - Each file system has a namespace lock of type `L`, held across
//!   operations that may delete an inode, so that its pages are gone before
//!   any other operation can create a file with the same inode number.
//!
//! Lock order: namespace, then inode, then whatever the inner file system
//! takes, then the index. Nothing takes a namespace or inode lock while
//! holding an inode lock of another file.

#![no_std]

extern crate alloc;

mod fs;

pub use fs::CachedFs;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};
use poly_vfs::lock::{Lock, RawLock, RawSpinLock};
use poly_vfs::{Errno, FileSystem, Ino, Result};

/// Bytes per cached page.
pub const PAGE_SIZE: usize = 4096;
pub(crate) const PAGE: u64 = PAGE_SIZE as u64;

/// Pages one miss reads from the inner file system at most: the missed
/// page and the uncached pages after it, up to the end of the file.
pub const READAHEAD_PAGES: usize = 16;

/// Dirty pages one writeback call hands the inner file system at most
/// (consecutive pages of one file).
pub const WRITEBACK_RUN_PAGES: usize = 16;

/// Clean pages evicted to make room when a frame cannot be allocated.
const EVICT_ON_FAILURE: usize = 32;

/// A frame of [`PAGE_SIZE`] bytes that holds one cached page. Dropping it
/// gives the memory back.
///
/// # Safety
///
/// [`PageFrame::as_ptr`] must point to [`PAGE_SIZE`] bytes that are valid
/// for reads and writes for as long as the frame lives, belong to this
/// frame alone (no other `PageFrame` overlaps them), and start out zeroed.
pub unsafe trait PageFrame: Send + Sync + Sized + 'static {
    /// A zeroed frame, or `None` when memory ran out.
    fn alloc() -> Option<Self>;

    /// The frame's first byte.
    fn as_ptr(&self) -> *mut u8;
}

/// Which file system a page belongs to (assigned at registration).
pub type FsId = u32;

/// What a cached page is: page `index` (bytes `index * PAGE_SIZE` on) of
/// inode `ino` of file system `fs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub fs: FsId,
    pub ino: Ino,
    pub index: u64,
}

impl Key {
    pub const fn new(fs: FsId, ino: Ino, index: u64) -> Key {
        Key { fs, ino, index }
    }

    /// The file offset of the page's first byte.
    pub const fn offset(&self) -> u64 {
        self.index * PAGE
    }
}

/// One cached page: its frame and state. Shared (`Arc`) between the index
/// and whoever uses it at the moment (a copy, a mapping).
pub struct Page<F> {
    key: Key,
    frame: F,
    /// Holds data the inner file system does not have yet. Changed only
    /// with the index locked, together with the index's dirty set.
    dirty: AtomicBool,
    /// Writable mappings of the page.
    writable_maps: AtomicU32,
}

impl<F> core::fmt::Debug for Page<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Page")
            .field("key", &self.key)
            .field("dirty", &self.dirty.load(Relaxed))
            .field("writable_maps", &self.writable_maps.load(Relaxed))
            .finish_non_exhaustive()
    }
}

impl<F: PageFrame> Page<F> {
    fn new(key: Key, frame: F) -> Page<F> {
        Page { key, frame, dirty: AtomicBool::new(false), writable_maps: AtomicU32::new(0) }
    }

    pub fn key(&self) -> Key {
        self.key
    }

    pub fn frame(&self) -> &F {
        &self.frame
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Relaxed)
    }

    pub fn writable_maps(&self) -> u32 {
        self.writable_maps.load(Relaxed)
    }

    /// Copy bytes `offset..offset + buf.len()` of the page into `buf`.
    pub fn read(&self, offset: usize, buf: &mut [u8]) {
        assert!(offset <= PAGE_SIZE && buf.len() <= PAGE_SIZE - offset, "read past the end of a page");
        // SAFETY: `PageFrame` promises PAGE_SIZE valid bytes while the frame
        // lives (`self` holds it), and the range is inside them. A user
        // mapping may change the bytes meanwhile; a byte copy through raw
        // pointers takes whatever is there (bytes have no invalid values)
        // and never forms a reference to the frame's memory.
        unsafe { core::ptr::copy_nonoverlapping(self.frame.as_ptr().add(offset), buf.as_mut_ptr(), buf.len()) }
    }

    /// Copy `data` into the page at `offset`.
    pub(crate) fn write(&self, offset: usize, data: &[u8]) {
        assert!(offset <= PAGE_SIZE && data.len() <= PAGE_SIZE - offset, "write past the end of a page");
        // SAFETY: as in `read`; writers inside the cache hold the inode's
        // lock, so they do not overlap each other.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), self.frame.as_ptr().add(offset), data.len()) }
    }

    /// Zero the page from `offset` to its end.
    pub(crate) fn zero_from(&self, offset: usize) {
        let offset = offset.min(PAGE_SIZE);
        // SAFETY: as in `write`.
        unsafe { core::ptr::write_bytes(self.frame.as_ptr().add(offset), 0, PAGE_SIZE - offset) }
    }
}

/// A page in the index, and its place in the LRU order.
struct Slot<F> {
    page: Arc<Page<F>>,
    stamp: u64,
}

/// Every cached page, the LRU order and the dirty set.
struct Index<F> {
    pages: BTreeMap<Key, Slot<F>>,
    /// Use stamps, oldest first, to the page last used then.
    lru: BTreeMap<u64, Key>,
    dirty: BTreeSet<Key>,
    clock: u64,
}

impl<F> Index<F> {
    fn new() -> Index<F> {
        Index { pages: BTreeMap::new(), lru: BTreeMap::new(), dirty: BTreeSet::new(), clock: 0 }
    }

    fn stamp(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// The page, marked as just used.
    fn get(&mut self, key: &Key) -> Option<Arc<Page<F>>> {
        let stamp = self.stamp();
        let slot = self.pages.get_mut(key)?;
        self.lru.remove(&slot.stamp);
        slot.stamp = stamp;
        self.lru.insert(stamp, *key);
        Some(Arc::clone(&slot.page))
    }

    fn insert(&mut self, page: Arc<Page<F>>) {
        let key = page.key;
        let stamp = self.stamp();
        // The inode lock keeps a key from being filled twice; replace
        // cleanly anyway.
        if let Some(old) = self.pages.insert(key, Slot { page, stamp }) {
            self.lru.remove(&old.stamp);
            self.dirty.remove(&key);
        }
        self.lru.insert(stamp, key);
    }

    fn remove(&mut self, key: &Key) -> Option<Arc<Page<F>>> {
        let slot = self.pages.remove(key)?;
        self.lru.remove(&slot.stamp);
        self.dirty.remove(key);
        slot.page.dirty.store(false, Relaxed);
        Some(slot.page)
    }

    /// Whether `page` is the page the index holds for its key (and not one
    /// that was dropped from the index while someone still held it).
    fn holds(&self, page: &Arc<Page<F>>) -> bool {
        self.pages.get(&page.key).is_some_and(|s| Arc::ptr_eq(&s.page, page))
    }

    fn mark_dirty(&mut self, page: &Arc<Page<F>>) {
        if self.holds(page) {
            page.dirty.store(true, Relaxed);
            self.dirty.insert(page.key);
        }
    }

    /// Clears the dirty mark; returns whether it was set.
    fn take_dirty(&mut self, page: &Arc<Page<F>>) -> bool {
        if !self.holds(page) {
            return false;
        }
        self.dirty.remove(&page.key);
        page.dirty.swap(false, Relaxed)
    }

    /// The keys of inode `ino` of `fs` from page `from` on.
    fn keys_of(&self, fs: FsId, ino: Ino, from: u64) -> Vec<Key> {
        self.pages.range(Key::new(fs, ino, from)..=Key::new(fs, ino, u64::MAX)).map(|(k, _)| *k).collect()
    }

    fn has_inode(&self, fs: FsId, ino: Ino) -> bool {
        self.pages.range(Key::new(fs, ino, 0)..=Key::new(fs, ino, u64::MAX)).next().is_some()
    }
}

/// Counters of a [`PageCache`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Pages in the cache, and how many of them are dirty.
    pub pages: usize,
    pub dirty: usize,
    /// Pages the cache tries not to exceed ([`PageCache::set_limit`]).
    pub limit: usize,
    /// File systems registered.
    pub filesystems: usize,
    /// Page lookups by reads, writes and mappings that found the page, and
    /// that did not.
    pub hits: u64,
    pub misses: u64,
    /// Pages read from inner file systems (read-ahead included).
    pub filled: u64,
    /// Pages written back to inner file systems.
    pub written: u64,
    /// Clean pages dropped to make room.
    pub evicted: u64,
    /// Pages dropped because their file shrank or went away.
    pub discarded: u64,
    /// Writebacks that failed (the data is lost; the next `sync` of the
    /// file system reports the error).
    pub write_errors: u64,
}

#[derive(Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    filled: AtomicU64,
    written: AtomicU64,
    evicted: AtomicU64,
    discarded: AtomicU64,
    write_errors: AtomicU64,
}

/// What the cache keeps about a registered file system.
pub(crate) struct FsState<L> {
    pub(crate) id: FsId,
    pub(crate) inner: Arc<dyn FileSystem>,
    /// Held across operations that may delete an inode (see the crate docs).
    pub(crate) ns: Lock<L, ()>,
    /// The first writeback error since the last `sync`.
    pub(crate) error: Lock<RawSpinLock, Option<Errno>>,
    /// Something changed since [`CachedFs::take_changed`] last looked.
    pub(crate) changed: AtomicBool,
}

/// The lock of each inode that has (or had lately) pages.
type InodeLocks<L> = BTreeMap<(FsId, Ino), Arc<Lock<L, ()>>>;

/// The state every handle shares.
pub(crate) struct Core<F, L> {
    index: Lock<RawSpinLock, Index<F>>,
    inodes: Lock<RawSpinLock, InodeLocks<L>>,
    filesystems: Lock<RawSpinLock, BTreeMap<FsId, Arc<FsState<L>>>>,
    next_fs: AtomicU32,
    limit: AtomicUsize,
    c: Counters,
}

impl<F: PageFrame, L: RawLock + Default> Core<F, L> {
    /// The lock of inode `ino` of `fs` (made on first use).
    pub(crate) fn inode_lock(&self, fs: FsId, ino: Ino) -> Arc<Lock<L, ()>> {
        Arc::clone(self.inodes.lock().entry((fs, ino)).or_insert_with(|| Arc::new(Lock::new(()))))
    }

    /// The cached page for `key`, counted as a hit or a miss.
    pub(crate) fn lookup(&self, key: &Key) -> Option<Arc<Page<F>>> {
        let page = self.index.lock().get(key);
        let counter = if page.is_some() { &self.c.hits } else { &self.c.misses };
        counter.fetch_add(1, Relaxed);
        page
    }

    pub(crate) fn is_cached(&self, key: &Key) -> bool {
        self.index.lock().pages.contains_key(key)
    }

    pub(crate) fn has_inode(&self, fs: FsId, ino: Ino) -> bool {
        self.index.lock().has_inode(fs, ino)
    }

    /// A zeroed frame; when memory is short, evicts clean pages and tries
    /// once more.
    pub(crate) fn alloc_frame(&self) -> Result<F> {
        if let Some(f) = F::alloc() {
            return Ok(f);
        }
        let pages = self.index.lock().pages.len();
        self.shrink(pages.saturating_sub(EVICT_ON_FAILURE));
        F::alloc().ok_or(Errno::ENOMEM)
    }

    /// A new page for `key` in `frame`, put into the index.
    pub(crate) fn insert(&self, key: Key, frame: F) -> Arc<Page<F>> {
        let page = Arc::new(Page::new(key, frame));
        let over = {
            let mut index = self.index.lock();
            index.insert(Arc::clone(&page));
            index.pages.len() > self.limit.load(Relaxed)
        };
        if over {
            self.shrink(self.limit.load(Relaxed));
        }
        page
    }

    pub(crate) fn mark_dirty(&self, page: &Arc<Page<F>>) {
        self.index.lock().mark_dirty(page);
    }

    /// Drop pages of inode `ino` of `fs` from page `from` on, dirty or not.
    /// The caller holds the inode's lock. Pages someone still holds (a
    /// mapping) live on outside the cache until they are let go.
    pub(crate) fn discard(&self, fs: FsId, ino: Ino, from: u64) -> usize {
        let dropped: Vec<_> = {
            let mut index = self.index.lock();
            let keys = index.keys_of(fs, ino, from);
            keys.iter().filter_map(|k| index.remove(k)).collect()
        };
        self.c.discarded.fetch_add(dropped.len() as u64, Relaxed);
        dropped.len()
    }

    /// Evict clean pages that nobody but the index holds, least recently
    /// used first, until at most `target` pages are cached. Returns how
    /// many went. Their frames are freed after the index is unlocked.
    pub(crate) fn shrink(&self, target: usize) -> usize {
        let evicted: Vec<Arc<Page<F>>> = {
            let mut index = self.index.lock();
            let excess = index.pages.len().saturating_sub(target);
            let victims: Vec<Key> = index
                .lru
                .values()
                .filter(|k| {
                    let slot = &index.pages[*k];
                    !slot.page.is_dirty() && Arc::strong_count(&slot.page) == 1
                })
                .take(excess)
                .copied()
                .collect();
            victims.iter().filter_map(|k| index.remove(k)).collect()
        };
        self.c.evicted.fetch_add(evicted.len() as u64, Relaxed);
        evicted.len()
    }

    /// Write back the dirty pages of `st` (all of them, or those of one
    /// inode). Returns how many pages were written; errors are recorded for
    /// the file system's next `sync`, and the first is returned.
    pub(crate) fn writeback(&self, st: &FsState<L>, only: Option<Ino>) -> Result<usize> {
        let keys: Vec<Key> = {
            let index = self.index.lock();
            let (lo, hi) = match only {
                Some(ino) => (Key::new(st.id, ino, 0), Key::new(st.id, ino, u64::MAX)),
                None => (Key::new(st.id, 0, 0), Key::new(st.id, Ino::MAX, u64::MAX)),
            };
            index.dirty.range(lo..=hi).copied().collect()
        };
        let mut written = 0;
        let mut first_error = None;
        let mut rest = keys.as_slice();
        while let Some(first) = rest.first() {
            let n = rest.iter().take_while(|k| k.ino == first.ino).count();
            match self.writeback_inode(st, first.ino, &rest[..n]) {
                Ok(w) => written += w,
                Err(e) => {
                    first_error.get_or_insert(e);
                    let mut slot = st.error.lock();
                    slot.get_or_insert(e);
                }
            }
            rest = &rest[n..];
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(written),
        }
    }

    /// Write back the dirty pages `keys` (sorted) of one inode, in runs of
    /// consecutive pages.
    fn writeback_inode(&self, st: &FsState<L>, ino: Ino, keys: &[Key]) -> Result<usize> {
        let lock = self.inode_lock(st.id, ino);
        let _held = lock.lock();
        let size = match st.inner.getattr(ino) {
            Ok(a) => a.size,
            // Deleted since: its data does not matter any more.
            Err(Errno::ENOENT) => {
                self.discard(st.id, ino, 0);
                return Ok(0);
            }
            Err(e) => return Err(e),
        };
        // The pages still dirty now (a page may have been written back or
        // dropped since the keys were collected), each taken clean.
        let mut pages = Vec::with_capacity(keys.len());
        {
            let mut index = self.index.lock();
            for k in keys {
                if let Some(slot) = index.pages.get(k) {
                    let page = Arc::clone(&slot.page);
                    if index.take_dirty(&page) {
                        pages.push(page);
                    }
                }
            }
        }
        // Pages wholly past the end: only if the file shrank behind the
        // cache's back (truncation through the cache drops them). Dropped.
        let end_page = size.div_ceil(PAGE);
        if pages.iter().any(|p| p.key.index >= end_page) {
            self.discard(st.id, ino, end_page);
            pages.retain(|p| p.key.index < end_page);
        }
        let mut written = 0;
        let mut result = Ok(());
        let mut rest = pages.as_slice();
        while let Some(first) = rest.first() {
            let run = rest
                .iter()
                .enumerate()
                .take_while(|(i, p)| p.key.index == first.key.index + *i as u64)
                .take(WRITEBACK_RUN_PAGES)
                .count();
            let start = first.key.offset();
            let len = ((run as u64 * PAGE).min(size - start)) as usize;
            let mut buf = vec![0u8; len];
            for (i, p) in rest[..run].iter().enumerate() {
                let at = i * PAGE_SIZE;
                let n = PAGE_SIZE.min(len - at);
                p.read(0, &mut buf[at..at + n]);
            }
            match write_all(&*st.inner, ino, start, &buf) {
                Ok(()) => {
                    written += run;
                    self.c.written.fetch_add(run as u64, Relaxed);
                }
                // Deleted after the size was read (an unlink does not take
                // the inode lock): as above, its data does not matter any
                // more, and it is no error for whoever syncs next.
                Err(Errno::ENOENT) if matches!(st.inner.getattr(ino), Err(Errno::ENOENT)) => {
                    self.discard(st.id, ino, 0);
                    return Ok(written);
                }
                Err(e) => {
                    self.c.write_errors.fetch_add(run as u64, Relaxed);
                    if result.is_ok() {
                        result = Err(e);
                    }
                }
            }
            // Pages mapped writable may change at any time: still dirty.
            for p in &rest[..run] {
                if p.writable_maps() > 0 {
                    self.mark_dirty(p);
                }
            }
            rest = &rest[run..];
        }
        result.map(|()| written)
    }

    fn stats(&self) -> Stats {
        let (pages, dirty) = {
            let index = self.index.lock();
            (index.pages.len(), index.dirty.len())
        };
        let c = &self.c;
        Stats {
            pages,
            dirty,
            limit: self.limit.load(Relaxed),
            filesystems: self.filesystems.lock().len(),
            hits: c.hits.load(Relaxed),
            misses: c.misses.load(Relaxed),
            filled: c.filled.load(Relaxed),
            written: c.written.load(Relaxed),
            evicted: c.evicted.load(Relaxed),
            discarded: c.discarded.load(Relaxed),
            write_errors: c.write_errors.load(Relaxed),
        }
    }

    pub(crate) fn count_filled(&self, pages: usize) {
        self.c.filled.fetch_add(pages as u64, Relaxed);
    }

    /// Forget file system `fs`: drop all its pages (dirty ones too) and
    /// inode locks.
    pub(crate) fn unregister(&self, fs: FsId) {
        self.filesystems.lock().remove(&fs);
        let dropped: Vec<_> = {
            let mut index = self.index.lock();
            let keys: Vec<Key> =
                index.pages.range(Key::new(fs, 0, 0)..=Key::new(fs, Ino::MAX, u64::MAX)).map(|(k, _)| *k).collect();
            keys.iter().filter_map(|k| index.remove(k)).collect()
        };
        self.c.discarded.fetch_add(dropped.len() as u64, Relaxed);
        drop(dropped);
        self.inodes.lock().retain(|(f, _), _| *f != fs);
    }

    /// Forget inode locks nobody holds whose inode has no pages.
    fn prune_inode_locks(&self) {
        let mut inodes = self.inodes.lock();
        let index = self.index.lock();
        inodes.retain(|(fs, ino), lock| Arc::strong_count(lock) > 1 || index.has_inode(*fs, *ino));
    }
}

/// Write all of `buf` at `offset` of `ino`.
fn write_all(fs: &dyn FileSystem, ino: Ino, offset: u64, buf: &[u8]) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        match fs.write(ino, offset + done as u64, &buf[done..])? {
            0 => return Err(Errno::EIO),
            n => done += n,
        }
    }
    Ok(())
}

/// Read `buf.len()` bytes at `offset` of `ino` (zeros past the end).
pub(crate) fn read_full(fs: &dyn FileSystem, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        match fs.read(ino, offset + done as u64, &mut buf[done..])? {
            0 => break,
            n => done += n,
        }
    }
    Ok(())
}

/// The page cache (see the crate docs). A handle: clones share the cache.
pub struct PageCache<F, L = RawSpinLock> {
    core: Arc<Core<F, L>>,
}

impl<F, L> Clone for PageCache<F, L> {
    fn clone(&self) -> Self {
        PageCache { core: Arc::clone(&self.core) }
    }
}

impl<F, L> core::fmt::Debug for PageCache<F, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PageCache").field("limit", &self.core.limit.load(Relaxed)).finish_non_exhaustive()
    }
}

impl<F: PageFrame, L: RawLock + Default> PageCache<F, L> {
    /// An empty cache that tries to stay within `limit` pages.
    pub fn new(limit: usize) -> PageCache<F, L> {
        PageCache {
            core: Arc::new(Core {
                index: Lock::new(Index::new()),
                inodes: Lock::new(BTreeMap::new()),
                filesystems: Lock::new(BTreeMap::new()),
                next_fs: AtomicU32::new(1),
                limit: AtomicUsize::new(limit),
                c: Counters::default(),
            }),
        }
    }

    /// Put the cache in front of `inner`: mount the returned file system
    /// instead of `inner`. Dropping the last reference to it drops its
    /// pages, dirty ones too, so sync it before (unmounting does).
    pub fn register(&self, inner: Arc<dyn FileSystem>) -> Arc<CachedFs<F, L>> {
        let id = self.core.next_fs.fetch_add(1, Relaxed);
        let st =
            Arc::new(FsState { id, inner, ns: Lock::new(()), error: Lock::new(None), changed: AtomicBool::new(false) });
        self.core.filesystems.lock().insert(id, Arc::clone(&st));
        Arc::new(CachedFs::new(Arc::clone(&self.core), st))
    }

    pub fn stats(&self) -> Stats {
        self.core.stats()
    }

    pub fn limit(&self) -> usize {
        self.core.limit.load(Relaxed)
    }

    /// Pages the cache keeps at most before it evicts clean ones. Dirty
    /// and mapped pages are never evicted, so it can be exceeded until
    /// writeback cleans them.
    pub fn set_limit(&self, pages: usize) {
        self.core.limit.store(pages, Relaxed);
        self.shrink(pages);
    }

    /// Evict clean, unused pages (least recently used first) until at most
    /// `target` remain or none can go. Returns how many went.
    pub fn shrink(&self, target: usize) -> usize {
        let n = self.core.shrink(target);
        self.core.prune_inode_locks();
        n
    }

    /// Write back every dirty page of every file system. Returns how many
    /// pages were written; the first error (also kept for the file system's
    /// next `sync`).
    pub fn writeback_all(&self) -> Result<usize> {
        let all: Vec<Arc<FsState<L>>> = self.core.filesystems.lock().values().cloned().collect();
        let mut written = 0;
        let mut first_error = None;
        for st in all {
            match self.core.writeback(&st, None) {
                Ok(n) => written += n,
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(written),
        }
    }

    /// The page for `key` if it is cached (no read from the file system).
    pub fn cached(&self, key: &Key) -> Option<PageRef<F, L>> {
        let page = self.core.index.lock().get(key)?;
        Some(PageRef { page, core: Arc::clone(&self.core) })
    }
}

/// A cached page held by its user (a memory mapping): it stays in memory
/// while any `PageRef` to it exists, and is never evicted meanwhile.
pub struct PageRef<F, L> {
    page: Arc<Page<F>>,
    core: Arc<Core<F, L>>,
}

impl<F, L> Clone for PageRef<F, L> {
    fn clone(&self) -> Self {
        PageRef { page: Arc::clone(&self.page), core: Arc::clone(&self.core) }
    }
}

impl<F, L> core::fmt::Debug for PageRef<F, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("PageRef").field(&self.page).finish()
    }
}

impl<F, L> core::ops::Deref for PageRef<F, L> {
    type Target = Page<F>;

    fn deref(&self) -> &Page<F> {
        &self.page
    }
}

impl<F: PageFrame, L: RawLock + Default> PageRef<F, L> {
    pub(crate) fn new(page: Arc<Page<F>>, core: Arc<Core<F, L>>) -> PageRef<F, L> {
        PageRef { page, core }
    }

    /// Whether both are the same page (the same frame).
    pub fn same(&self, other: &PageRef<F, L>) -> bool {
        Arc::ptr_eq(&self.page, &other.page)
    }

    /// Whether the page is still the cache's page for its key: false once
    /// its file shrank past it or went away (the holder keeps a copy that
    /// nothing writes back).
    pub fn is_cached(&self) -> bool {
        self.core.index.lock().holds(&self.page)
    }

    /// The page's contents changed behind the cache's back.
    pub fn mark_dirty(&self) {
        self.core.mark_dirty(&self.page);
    }

    /// The page was mapped writable: the CPU may change it from now on, so
    /// it is dirty until [`PageRef::unmap_writable`] (and written back each
    /// time meanwhile).
    pub fn map_writable(&self) {
        self.page.writable_maps.fetch_add(1, Relaxed);
        self.mark_dirty();
    }

    /// A writable mapping of the page went away; what it wrote is written
    /// back with the next writeback.
    pub fn unmap_writable(&self) {
        let before = self.page.writable_maps.fetch_sub(1, Relaxed);
        assert!(before > 0, "unmap_writable without map_writable");
        self.mark_dirty();
    }
}

#[cfg(test)]
mod tests;
