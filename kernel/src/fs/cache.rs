//! The page cache in the kernel (ADR 0040): the host-tested
//! `poly-pagecache` crate with frames from the frame allocator, the
//! kernel's sleeping mutex as its inode and namespace locks, the writeback
//! thread, and files as backing for memory mappings ([`FileSource`]).
//!
//! - **Frames.** A [`KernelFrame`] is one frame from `FRAMES`, zeroed,
//!   reached through the direct map; dropping it frees it. [`frames`]
//!   counts those the cache holds (mapped ones included).
//! - **One cache** ([`cache`]) for every disk file system, made at boot
//!   with a limit of a quarter of RAM. Clean pages beyond it are evicted
//!   least recently used first; the writeback thread also evicts half the
//!   clean pages when free memory falls below 1/16 of RAM.
//! - **Writeback.** The thread `writeback` wakes every
//!   [`WRITEBACK_INTERVAL_NS`] (or when [`kick`]ed) and syncs every disk
//!   file system that changed or has dirty pages: its dirty pages go to
//!   the file system, then the file system and its disk are synced, so
//!   data is durable within a few seconds. `sync` and unmounting do the
//!   same at once.
//! - **Mappings.** [`FileSource`] is a VFS file as a `user::PageSource`.
//!   On a disk file system or a tmpfs (which is behind the cache too,
//!   `memfs`, ADR 0064) it hands out the cache's pages, so a mapping and
//!   `read`/`write` share one frame, and `MAP_SHARED` works; elsewhere
//!   (procfs, sysfs) it reads private copies as before.

use super::disk;
use crate::arch;
use crate::frames::FRAMES;
use crate::sync::{RawMutex, SetOnce, SpinLock};
use crate::thread::{self, WaitQueue};
use crate::user::memory::{IoError, PageSource};
use crate::{kprintln, time};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_pagecache::{CachedFs, PAGE_SIZE, PageCache, PageFrame, PageRef};
use poly_vfs::NodeRef;

/// The page cache as the kernel instantiates it.
pub type Cache = PageCache<KernelFrame, RawMutex>;
/// A file system behind the cache (what a disk mount puts in the VFS).
pub type CachedDiskFs = CachedFs<KernelFrame, RawMutex>;
/// A cached page held by a mapping.
pub type CachedPage = PageRef<KernelFrame, RawMutex>;

/// Frames the page cache holds.
static CACHE_FRAMES: AtomicU64 = AtomicU64::new(0);

/// How many frames the page cache holds right now (pages in the cache, and
/// pages dropped from it that a mapping still holds).
pub fn frames() -> u64 {
    CACHE_FRAMES.load(Ordering::Relaxed)
}

/// One frame from the frame allocator holding a cached page.
#[derive(Debug)]
pub struct KernelFrame {
    phys: u64,
}

impl KernelFrame {
    /// The frame's physical address (what a user mapping maps).
    pub fn phys(&self) -> u64 {
        self.phys
    }
}

// SAFETY: every frame comes from `FRAMES` (allocated for this value alone
// and zeroed below) and stays allocated until the value is dropped; it is
// RAM, and `as_ptr` reaches it through the direct map, which maps all RAM
// writable.
unsafe impl PageFrame for KernelFrame {
    fn alloc() -> Option<KernelFrame> {
        let phys = FRAMES.lock().as_mut().and_then(|f| f.allocate(0))?;
        let frame = KernelFrame { phys };
        CACHE_FRAMES.fetch_add(1, Ordering::Relaxed);
        // SAFETY: a frame just allocated, so nothing else uses it; the
        // direct map maps it writable.
        unsafe { frame.as_ptr().write_bytes(0, PAGE_SIZE) };
        Some(frame)
    }

    fn as_ptr(&self) -> *mut u8 {
        arch::phys_to_virt(self.phys).expect("page cache frame outside the direct map") as *mut u8
    }
}

impl Drop for KernelFrame {
    fn drop(&mut self) {
        CACHE_FRAMES.fetch_sub(1, Ordering::Relaxed);
        if let Some(f) = FRAMES.lock().as_mut() {
            f.free(self.phys, 0).expect("page cache frame freed twice");
        }
    }
}

static CACHE: SetOnce<Cache> = SetOnce::new();

/// The page cache. Panics before [`init`].
pub fn cache() -> &'static Cache {
    CACHE.get().expect("fs::cache used before fs::init")
}

/// How often the writeback thread looks for work.
pub const WRITEBACK_INTERVAL_NS: u64 = 5 * time::NANOS_PER_SEC;

/// Wakes the writeback thread early.
static KICK: WaitQueue = WaitQueue::new();
static KICKED: AtomicBool = AtomicBool::new(false);
/// Writeback passes finished.
static PASSES: AtomicU64 = AtomicU64::new(0);

/// Make the cache and start the writeback thread. Once, from `fs::init`.
pub(super) fn init() {
    let total = FRAMES.lock().as_ref().map_or(0, |f| f.total_frames());
    let limit = (total / 4) as usize;
    if CACHE.set(Cache::new(limit)).is_err() {
        panic!("fs::cache::init called twice");
    }
    match thread::spawn_named("writeback", writeback_main, 0) {
        Ok(id) => {
            let _ = thread::detach(id);
            kprintln!(
                "fs: page cache of up to {limit} pages ({} MiB); writeback every {} s",
                (limit * PAGE_SIZE) >> 20,
                WRITEBACK_INTERVAL_NS / time::NANOS_PER_SEC
            );
        }
        Err(e) => kprintln!("fs: the writeback thread did not start ({e:?}); data is written at sync only"),
    }
}

/// Things to drop in the writeback thread (see [`release_later`]).
static LATER: SpinLock<Vec<Box<dyn Send>>> = SpinLock::new(Vec::new());

/// Drop `what` in the writeback thread, where no lock is held and sleeping
/// is fine: for file references and cached pages let go of where neither
/// is true (a memory map dropped while the scheduler reaps its last
/// thread). Kicks the thread.
pub fn release_later(what: Box<dyn Send>) {
    LATER.lock().push(what);
    kick();
}

/// Drop what [`release_later`] collected.
fn release_now() {
    let later = core::mem::take(&mut *LATER.lock());
    drop(later);
}

/// Wake the writeback thread now.
pub fn kick() {
    KICKED.store(true, Ordering::SeqCst);
    KICK.wake_all();
}

/// Writeback passes the thread has finished.
pub fn passes() -> u64 {
    PASSES.load(Ordering::SeqCst)
}

fn writeback_main(_: usize) -> usize {
    loop {
        let deadline = time::now_ns().saturating_add(WRITEBACK_INTERVAL_NS);
        KICK.wait_until_deadline(|| KICKED.load(Ordering::SeqCst), deadline);
        KICKED.store(false, Ordering::SeqCst);
        release_now();
        disk::writeback_pass();
        super::memfs::writeback_pass();
        relieve_memory_pressure();
        PASSES.fetch_add(1, Ordering::SeqCst);
    }
}

/// Evict clean pages when free memory runs low: half of the cache's pages
/// (as many as are clean and unused) when less than 1/16 of RAM is free.
fn relieve_memory_pressure() {
    let (free, total) = FRAMES.lock().as_ref().map_or((0, 0), |f| (f.free_frames(), f.total_frames()));
    if free >= total / 16 {
        return;
    }
    let c = cache();
    let pages = c.stats().pages;
    let evicted = c.shrink(pages / 2);
    if evicted > 0 {
        kprintln!("fs: memory is low ({free} of {total} frames free): {evicted} clean cached pages evicted");
    }
}

/// A VFS file as the backing of memory mappings (and of `exec`): reads
/// through the file system (and so through the page cache on a disk mount)
/// and, on a disk mount, hands out the cache's pages so mappings share
/// them. Keeps the file's inode pinned while it lives.
pub struct FileSource {
    node: NodeRef,
    cached: Option<Arc<CachedDiskFs>>,
    /// The open file it was made from, which knows the file's name
    /// (`/proc/<pid>/maps`).
    file: Option<Arc<poly_vfs::OpenFile>>,
}

impl core::fmt::Debug for FileSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileSource").field("node", &self.node).field("cached", &self.cached.is_some()).finish()
    }
}

impl FileSource {
    /// The file `node` names.
    pub fn new(node: NodeRef) -> FileSource {
        let fs = node.mount().fs();
        let cached = disk::cached_fs_of(fs).or_else(|| super::memfs::cached_fs_of(fs));
        FileSource { node, cached, file: None }
    }

    /// The file `file` is open on.
    pub fn of(file: &Arc<poly_vfs::OpenFile>) -> FileSource {
        FileSource { file: Some(Arc::clone(file)), ..FileSource::new(file.node().clone()) }
    }

    pub fn node(&self) -> &NodeRef {
        &self.node
    }
}

impl PageSource for FileSource {
    fn len(&self) -> u64 {
        self.node.attr().map_or(0, |a| a.size)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        self.node.mount().fs().read(self.node.ino(), offset, buf).map_err(|_| IoError)
    }

    /// The file's device and inode number: every mapping of the file
    /// names the same futexes (63 bits; inodes above 2^39 may collide).
    fn object_id(&self) -> Option<u64> {
        let st = self.node.stat().ok()?;
        let dev = (u64::from(st.dev.major) << 12 | u64::from(st.dev.minor)) & 0xff_ffff;
        Some(dev << 39 | (st.ino & ((1 << 39) - 1)))
    }

    fn file(&self) -> Option<&Arc<poly_vfs::OpenFile>> {
        self.file.as_ref()
    }

    fn cached_page(&self, offset: u64) -> Option<Result<CachedPage, IoError>> {
        let fs = self.cached.as_ref()?;
        Some(fs.page(self.node.ino(), offset / PAGE_SIZE as u64).map_err(|_| IoError))
    }

    fn shareable(&self) -> bool {
        self.cached.is_some()
    }
}
