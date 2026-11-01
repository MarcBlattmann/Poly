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
use poly_personality_linux::admin::Amount;
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

/// A new tmpfs with mount(2)'s `size` / `nr_inodes` (tmpfs(5)): a count,
/// or a percentage of RAM (of its bytes for `size`, of its pages for
/// `nr_inodes`); 0 is no limit; an option not given keeps the default.
pub fn new_tmpfs_sized(clock: Arc<dyn Clock>, size: Option<Amount>, inodes: Option<Amount>) -> Arc<Tmpfs> {
    let pages = FRAMES.lock().as_ref().map_or(0, |f| f.total_frames());
    let default = |total: u64| (pages > 0).then_some(total / 2);
    let limit = |a: Option<Amount>, total: u64| match a {
        None => default(total),
        Some(Amount::Count(0)) => None,
        Some(Amount::Count(n)) => Some(n),
        Some(Amount::Percent(p)) => Some(total.saturating_mul(p) / 100),
    };
    Arc::new(Tmpfs::with_optional_limits(clock, limit(size, pages * 4096), limit(inodes, pages)))
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

/// The memory file system memfd_create(2)'s files live on (ADR 0111): a
/// tmpfs behind the page cache, like `/dev/shm`, mounted where no path
/// reaches it; made at the first call.
pub fn memfd_mount() -> Arc<poly_vfs::Mount> {
    static MOUNT: SpinLock<Option<Arc<poly_vfs::Mount>>> = SpinLock::new(None);
    let mut m = MOUNT.lock();
    m.get_or_insert_with(|| super::vfs().detached_mount(cached(new_tmpfs(super::clock())))).clone()
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
