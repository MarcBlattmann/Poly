//! tmpfs behind the page cache (ADR 0064): the kernel's memory file
//! systems (`/`, `/dev/shm`, tmpfs mounts) are mounted as the page cache's
//! [`CachedDiskFs`] over a [`Tmpfs`], like a disk file system. Reads,
//! writes and memory mappings of their files then share the cache's pages,
//! so `MAP_SHARED` mappings of tmpfs files work as on a disk mount (mmap(2);
//! tmpfs(5): "it is possible to ... mmap(2)" its files), where they were
//! refused with `ENODEV` before.
//!
//! The tmpfs keeps every file's data; the cache holds the pages in use, and
//! dirty ones reach the tmpfs at writeback ([`writeback_pass`], from the
//! writeback thread), after which they are clean and may be evicted like
//! any cached page.
//!
//! Sizes: like tmpfs(5)'s defaults, each instance holds at most half of
//! RAM in data and half of RAM's page count in inodes (`ENOSPC` beyond),
//! and `statfs` reports those totals.

use super::cache::{self, CachedDiskFs};
use crate::frames::FRAMES;
use crate::sync::SpinLock;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use poly_vfs::{Clock, FileSystem, Tmpfs};

/// Every memory file system behind the cache, for writeback.
static MEMORY_FS: SpinLock<Vec<Weak<CachedDiskFs>>> = SpinLock::new(Vec::new());

/// A new tmpfs with tmpfs(5)'s default limits: half of RAM for data, half
/// of RAM's pages as inodes.
pub fn new_tmpfs(clock: Arc<dyn Clock>) -> Arc<Tmpfs> {
    let pages = FRAMES.lock().as_ref().map_or(0, |f| f.total_frames());
    if pages == 0 {
        return Arc::new(Tmpfs::new(clock));
    }
    Arc::new(Tmpfs::with_limits(clock, pages / 2 * 4096, pages / 2))
}

/// `tmpfs` behind the page cache: what gets mounted.
pub fn cached(tmpfs: Arc<Tmpfs>) -> Arc<CachedDiskFs> {
    let fs = cache::cache().register(tmpfs);
    let mut all = MEMORY_FS.lock();
    all.retain(|w| w.strong_count() > 0);
    all.push(Arc::downgrade(&fs));
    fs
}

/// The cached memory file system that `fs` (a mounted file system) is, if
/// it is one.
pub fn cached_fs_of(fs: &Arc<dyn FileSystem>) -> Option<Arc<CachedDiskFs>> {
    let want = Arc::as_ptr(fs) as *const ();
    MEMORY_FS.lock().iter().filter_map(Weak::upgrade).find(|m| Arc::as_ptr(m) as *const () == want)
}

/// Write the dirty pages of every memory file system back into its tmpfs.
pub fn writeback_pass() {
    let all: Vec<Arc<CachedDiskFs>> = MEMORY_FS.lock().iter().filter_map(Weak::upgrade).collect();
    for fs in all {
        let changed = fs.take_changed();
        if changed || fs.page_counts().1 > 0 {
            let _ = fs.writeback(None);
        }
    }
}
