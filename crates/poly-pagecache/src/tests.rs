//! Host tests: the cache in front of tmpfs (through a probe that counts
//! and can fail the inner file system's reads and writes), and in front of
//! ext2 on a RAM disk.

extern crate std;

use super::*;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use poly_vfs::{Attr, Clock, DeviceId, DirEntry, FileType, FsInfo, NewNode, RenameFlags, SetAttr, Timespec, Tmpfs};
use std::boxed::Box;
use std::cell::Cell;
use std::thread;
use std::vec::Vec;

std::thread_local! {
    /// Frames alive that this thread allocated minus those it freed.
    static LIVE: Cell<isize> = const { Cell::new(0) };
    /// Allocations this thread may still make (`None`: no limit).
    static BUDGET: Cell<Option<usize>> = const { Cell::new(None) };
}

fn live() -> isize {
    LIVE.with(Cell::get)
}

/// A frame on the host heap.
struct TestFrame(Box<UnsafeCell<[u8; PAGE_SIZE]>>);

// SAFETY: the bytes are reached only through `as_ptr`, whose users (the
// cache) serialise their accesses.
unsafe impl Send for TestFrame {}
// SAFETY: as above.
unsafe impl Sync for TestFrame {}

// SAFETY: a boxed, zeroed array of PAGE_SIZE bytes per frame, alive as
// long as the frame.
unsafe impl PageFrame for TestFrame {
    fn alloc() -> Option<TestFrame> {
        if let Some(left) = BUDGET.with(Cell::get) {
            if left == 0 {
                return None;
            }
            BUDGET.with(|b| b.set(Some(left - 1)));
        }
        LIVE.with(|l| l.set(l.get() + 1));
        Some(TestFrame(Box::new(UnsafeCell::new([0; PAGE_SIZE]))))
    }

    fn as_ptr(&self) -> *mut u8 {
        self.0.get().cast()
    }
}

impl Drop for TestFrame {
    fn drop(&mut self) {
        LIVE.with(|l| l.set(l.get() - 1));
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_700_000_000, 0)
    }
}

/// Tmpfs that counts data reads and writes and can fail writes.
struct Probe {
    fs: Tmpfs,
    reads: AtomicUsize,
    writes: AtomicUsize,
    fail_writes: AtomicBool,
    /// The next data write first unlinks `/f` from the inner file system:
    /// what another thread's `unlink` does while a writeback of `f` holds
    /// its inode lock (the inner unlink runs under the namespace lock only).
    unlink_f_on_write: AtomicBool,
}

impl Probe {
    fn new() -> Arc<Probe> {
        Arc::new(Probe {
            fs: Tmpfs::new(Arc::new(FixedClock)),
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            fail_writes: AtomicBool::new(false),
            unlink_f_on_write: AtomicBool::new(false),
        })
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    /// What the inner file system holds, past the cache.
    fn contents(&self, ino: Ino) -> Vec<u8> {
        let mut buf = vec![0u8; self.fs.getattr(ino).unwrap().size as usize];
        let n = self.fs.read(ino, 0, &mut buf).unwrap();
        assert_eq!(n, buf.len());
        buf
    }
}

impl FileSystem for Probe {
    fn info(&self) -> FsInfo {
        self.fs.info()
    }
    fn root(&self) -> Ino {
        self.fs.root()
    }
    fn lookup(&self, dir: Ino, name: &[u8]) -> poly_vfs::Result<Ino> {
        self.fs.lookup(dir, name)
    }
    fn getattr(&self, ino: Ino) -> poly_vfs::Result<Attr> {
        self.fs.getattr(ino)
    }
    fn setattr(&self, ino: Ino, set: &SetAttr) -> poly_vfs::Result<Attr> {
        self.fs.setattr(ino, set)
    }
    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> poly_vfs::Result<Ino> {
        self.fs.create(dir, name, node)
    }
    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> poly_vfs::Result<Ino> {
        self.fs.mkdir(dir, name, perm, uid, gid)
    }
    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> poly_vfs::Result<Ino> {
        self.fs.symlink(dir, name, target, uid, gid)
    }
    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> poly_vfs::Result<()> {
        self.fs.link(dir, name, ino)
    }
    fn unlink(&self, dir: Ino, name: &[u8]) -> poly_vfs::Result<()> {
        self.fs.unlink(dir, name)
    }
    fn rmdir(&self, dir: Ino, name: &[u8]) -> poly_vfs::Result<()> {
        self.fs.rmdir(dir, name)
    }
    fn rename(&self, od: Ino, on: &[u8], nd: Ino, nn: &[u8], flags: RenameFlags) -> poly_vfs::Result<()> {
        self.fs.rename(od, on, nd, nn, flags)
    }
    fn readlink(&self, ino: Ino) -> poly_vfs::Result<Vec<u8>> {
        self.fs.readlink(ino)
    }
    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> poly_vfs::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.fs.read(ino, offset, buf)
    }
    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> poly_vfs::Result<usize> {
        if self.fail_writes.load(Ordering::Relaxed) {
            return Err(Errno::ENOSPC);
        }
        if self.unlink_f_on_write.swap(false, Ordering::Relaxed) {
            self.fs.unlink(self.fs.root(), b"f").unwrap();
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.fs.write(ino, offset, buf)
    }
    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> poly_vfs::Result<()> {
        self.fs.readdir(dir, pos, emit)
    }
    fn open(&self, ino: Ino) -> poly_vfs::Result<()> {
        self.fs.open(ino)
    }
    fn release(&self, ino: Ino) {
        self.fs.release(ino)
    }
}

type Cached = CachedFs<TestFrame, RawSpinLock>;

fn setup(limit: usize) -> (PageCache<TestFrame>, Arc<Probe>, Arc<Cached>) {
    let cache = PageCache::new(limit);
    let probe = Probe::new();
    let fs = cache.register(Arc::clone(&probe) as Arc<dyn FileSystem>);
    (cache, probe, fs)
}

const REG: NewNode = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::new(0, 0) };

fn pattern(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(13).wrapping_add(seed) ^ (i / 4096) as u8).collect()
}

fn read_all(fs: &dyn FileSystem, ino: Ino) -> Vec<u8> {
    let mut buf = vec![0u8; fs.getattr(ino).unwrap().size as usize];
    let n = fs.read(ino, 0, &mut buf).unwrap();
    assert_eq!(n, buf.len());
    buf
}

/// A file written straight into the inner file system.
fn inner_file(probe: &Probe, name: &[u8], data: &[u8]) -> Ino {
    let ino = probe.fs.create(probe.fs.root(), name, &REG).unwrap();
    probe.fs.write(ino, 0, data).unwrap();
    ino
}

#[test]
fn reads_fill_once_with_read_ahead_and_then_hit() {
    let (cache, probe, fs) = setup(1000);
    let data = pattern(1, 10_000);
    let ino = inner_file(&probe, b"f", &data);
    assert_eq!(read_all(&*fs, ino), data);
    // Three pages, read in one call.
    assert_eq!(probe.reads(), 1);
    assert_eq!(fs.page_counts(), (3, 0));
    assert_eq!(read_all(&*fs, ino), data);
    let mut small = [0u8; 10];
    assert_eq!(fs.read(ino, 9_995, &mut small).unwrap(), 5);
    assert_eq!(&small[..5], &data[9_995..]);
    assert_eq!(fs.read(ino, 10_000, &mut small).unwrap(), 0);
    assert_eq!(probe.reads(), 1, "hits do not reach the file system");
    let s = cache.stats();
    assert_eq!(s.filled, 3);
    assert!(s.hits >= 3 && s.misses == 1, "{s:?}");
}

#[test]
fn read_ahead_stops_at_cached_pages_and_at_the_end() {
    let (_cache, probe, fs) = setup(1000);
    let ino = inner_file(&probe, b"f", &pattern(2, 40 * PAGE_SIZE - 100));
    let mut b = [0u8; 1];
    fs.read(ino, 5 * PAGE, &mut b).unwrap();
    assert_eq!(fs.page_counts().0, READAHEAD_PAGES);
    fs.read(ino, 0, &mut b).unwrap();
    assert_eq!(fs.page_counts().0, READAHEAD_PAGES + 5, "stops at page 5, which is cached");
    fs.read(ino, 39 * PAGE, &mut b).unwrap();
    assert_eq!(fs.page_counts().0, READAHEAD_PAGES + 6, "the last page alone");
    assert_eq!(probe.reads(), 3);
    assert_eq!(read_all(&*fs, ino), pattern(2, 40 * PAGE_SIZE - 100));
}

#[test]
fn writes_stay_in_the_cache_until_writeback() {
    let (cache, probe, fs) = setup(1000);
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    assert_eq!(fs.write(ino, 0, b"hello").unwrap(), 5);
    // The size is there at once, the data only in the cache.
    assert_eq!(fs.getattr(ino).unwrap().size, 5);
    assert_eq!(probe.contents(ino), [0; 5]);
    assert_eq!(probe.writes(), 0);
    assert_eq!(fs.page_counts(), (1, 1));
    assert_eq!(read_all(&*fs, ino), b"hello");
    assert!(fs.take_changed() && !fs.take_changed());
    assert_eq!(fs.writeback(None).unwrap(), 1);
    assert_eq!(probe.contents(ino), b"hello");
    assert_eq!(fs.page_counts(), (1, 0));
    assert_eq!(fs.writeback(None).unwrap(), 0, "nothing dirty left");
    assert_eq!(cache.stats().written, 1);
}

#[test]
fn writeback_writes_runs_of_pages_in_one_call() {
    let (cache, probe, fs) = setup(1000);
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    let data = pattern(3, 20 * PAGE_SIZE + 7);
    fs.write(ino, 0, &data).unwrap();
    assert_eq!(probe.reads(), 0, "whole pages and pages past the end are not read first");
    assert_eq!(cache.writeback_all().unwrap(), 21);
    // 16 pages, then 5.
    assert_eq!(probe.writes(), 2);
    assert_eq!(probe.contents(ino), data);
}

#[test]
fn partial_writes_read_the_page_first() {
    let (_cache, probe, fs) = setup(1000);
    let data = pattern(4, 3 * PAGE_SIZE);
    let ino = inner_file(&probe, b"f", &data);
    fs.write(ino, PAGE + 10, b"middle").unwrap();
    let mut want = data.clone();
    want[PAGE_SIZE + 10..PAGE_SIZE + 16].copy_from_slice(b"middle");
    assert_eq!(read_all(&*fs, ino), want);
    fs.sync().unwrap();
    assert_eq!(probe.contents(ino), want);
}

#[test]
fn holes_and_growth_read_as_zeros() {
    let (_cache, probe, fs) = setup(1000);
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(ino, 0, &[0xaa; 100]).unwrap();
    fs.truncate(ino, 50).unwrap();
    // What a mapping might scribble past the end of the last page.
    fs.page(ino, 0).unwrap().write(50, &[0xbb; 100]);
    fs.write(ino, 3 * PAGE + 1, b"x").unwrap();
    let got = read_all(&*fs, ino);
    assert_eq!(got.len() as u64, 3 * PAGE + 2);
    assert!(got[..50].iter().all(|&b| b == 0xaa));
    assert!(got[50..got.len() - 1].iter().all(|&b| b == 0), "the old tail and the hole read as zeros");
    assert_eq!(got[got.len() - 1], b'x');
    fs.sync().unwrap();
    assert_eq!(probe.contents(ino), got);
}

#[test]
fn truncation_drops_pages_past_the_end_and_zeroes_the_last() {
    let (_cache, probe, fs) = setup(1000);
    let before = live();
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(ino, 0, &pattern(5, 3 * PAGE_SIZE)).unwrap();
    assert_eq!(live() - before, 3);
    fs.truncate(ino, 1000).unwrap();
    assert_eq!(fs.page_counts(), (1, 1));
    assert_eq!(live() - before, 1, "frames of dropped pages are freed");
    fs.truncate(ino, 2000).unwrap();
    let got = read_all(&*fs, ino);
    assert_eq!(&got[..1000], &pattern(5, 1000)[..]);
    assert!(got[1000..].iter().all(|&b| b == 0));
    fs.sync().unwrap();
    assert_eq!(probe.contents(ino), got);
}

#[test]
fn read_and_mapping_share_the_page() {
    let (cache, probe, fs) = setup(1000);
    let ino = inner_file(&probe, b"f", &pattern(6, 2 * PAGE_SIZE));
    let mut b = [0u8; 1];
    fs.read(ino, PAGE, &mut b).unwrap();
    let cached = fs.cached_page(ino, 1).expect("read cached the page");
    let mapped = fs.page(ino, 1).unwrap();
    assert!(mapped.same(&cached));
    assert_eq!(mapped.frame().as_ptr(), cached.frame().as_ptr());
    assert_eq!(mapped.key(), Key::new(fs.id(), ino, 1));
    assert_eq!(probe.reads(), 1);
    // What `write` puts there, the mapping sees, and the other way round.
    fs.write(ino, PAGE + 1, b"W").unwrap();
    let mut seen = [0u8; 1];
    mapped.read(1, &mut seen);
    assert_eq!(&seen, b"W");
    mapped.write(2, b"M");
    fs.read(ino, PAGE + 2, &mut seen).unwrap();
    assert_eq!(&seen, b"M");
    assert_eq!(fs.page(ino, 2).unwrap_err(), Errno::ENXIO);
    let dir = fs.mkdir(fs.root(), b"d", 0o755, 0, 0).unwrap();
    assert_eq!(fs.page(dir, 0).unwrap_err(), Errno::EINVAL);
    assert!(cache.stats().hits >= 2);
}

#[test]
fn writable_mappings_keep_pages_dirty_until_unmapped() {
    let (_cache, probe, fs) = setup(1000);
    let ino = inner_file(&probe, b"f", &[1; 100]);
    let page = fs.page(ino, 0).unwrap();
    page.map_writable();
    assert!(page.is_dirty());
    page.write(0, b"cpu");
    assert_eq!(fs.writeback(None).unwrap(), 1);
    assert_eq!(&probe.contents(ino)[..3], b"cpu");
    assert!(page.is_dirty(), "still mapped writable");
    page.write(0, b"CPU");
    fs.writeback(None).unwrap();
    assert_eq!(&probe.contents(ino)[..3], b"CPU");
    page.write(3, b"!");
    page.unmap_writable();
    assert_eq!(fs.writeback(None).unwrap(), 1);
    assert!(!page.is_dirty());
    assert_eq!(&probe.contents(ino)[..4], b"CPU!");
    assert_eq!(probe.contents(ino).len(), 100, "writeback never writes past the end");
}

#[test]
fn eviction_drops_clean_unused_pages_least_recently_used_first() {
    let (cache, probe, fs) = setup(8);
    let before = live();
    let ino = inner_file(&probe, b"f", &pattern(7, 32 * PAGE_SIZE));
    let held = fs.page(ino, 0).unwrap();
    let mut b = [0u8; 1];
    for i in 0..32 {
        fs.read(ino, i * PAGE, &mut b).unwrap();
    }
    let s = cache.stats();
    assert!(s.pages <= 8, "{s:?}");
    assert!(s.evicted >= 24, "{s:?}");
    assert!(live() - before <= 8);
    assert!(held.is_cached(), "a held page is never evicted");
    // The most recent pages stayed.
    assert!(fs.cached_page(ino, 31).is_some());
    // Dirty pages are not evicted either.
    let other = fs.create(fs.root(), b"g", &REG).unwrap();
    fs.write(other, 0, &pattern(8, 4 * PAGE_SIZE)).unwrap();
    let pages = cache.stats().pages;
    assert_eq!(cache.shrink(0), pages - 5);
    assert_eq!(cache.stats().pages, 5, "the held page and the four dirty ones");
    assert_eq!(live() - before, 5);
    fs.sync().unwrap();
    drop(held);
    assert_eq!(cache.shrink(0), 5);
    assert_eq!(live(), before);
    assert_eq!(read_all(&*fs, other), pattern(8, 4 * PAGE_SIZE));
}

#[test]
fn allocation_failure_evicts_then_reports_out_of_memory() {
    let (cache, probe, fs) = setup(1000);
    let ino = inner_file(&probe, b"f", &pattern(9, 64 * PAGE_SIZE));
    let mut b = [0u8; 1];
    for i in 0..48 {
        fs.read(ino, i * PAGE, &mut b).unwrap();
    }
    BUDGET.with(|x| x.set(Some(0)));
    // No frame at first: clean pages make room, then the read works... once
    // the budget allows it again; with none at all it fails cleanly.
    assert_eq!(fs.read(ino, 60 * PAGE, &mut b).unwrap_err(), Errno::ENOMEM);
    assert!(cache.stats().evicted > 0, "clean pages were evicted to make room");
    BUDGET.with(|x| x.set(Some(1)));
    assert_eq!(fs.read(ino, 61 * PAGE, &mut b).unwrap(), 1, "read-ahead shrinks to what memory allows");
    BUDGET.with(|x| x.set(None));
    let written = fs.create(fs.root(), b"g", &REG).unwrap();
    BUDGET.with(|x| x.set(Some(1)));
    // A write that gets one page of two writes that much.
    assert_eq!(fs.write(written, 0, &[3; 2 * PAGE_SIZE]).unwrap(), PAGE_SIZE);
    BUDGET.with(|x| x.set(None));
}

#[test]
fn unlinked_files_lose_their_pages() {
    let (_cache, _probe, fs) = setup(1000);
    let before = live();
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(ino, 0, &[1; 3 * PAGE_SIZE]).unwrap();
    fs.unlink(fs.root(), b"f").unwrap();
    assert_eq!(fs.page_counts(), (0, 0));
    assert_eq!(live(), before);
    // Open (pinned) while unlinked: the pages live until the last pin.
    let ino = fs.create(fs.root(), b"g", &REG).unwrap();
    fs.open(ino).unwrap();
    fs.write(ino, 0, b"still here").unwrap();
    fs.unlink(fs.root(), b"g").unwrap();
    assert_eq!(read_all(&*fs, ino), b"still here");
    assert_eq!(fs.page_counts().0, 1);
    fs.release(ino);
    assert_eq!(fs.page_counts(), (0, 0));
    assert_eq!(live(), before);
}

#[test]
fn renaming_over_a_cached_file_drops_its_pages() {
    let (_cache, probe, fs) = setup(1000);
    let a = fs.create(fs.root(), b"a", &REG).unwrap();
    let b = fs.create(fs.root(), b"b", &REG).unwrap();
    fs.write(a, 0, b"new").unwrap();
    fs.write(b, 0, b"old").unwrap();
    fs.rename(fs.root(), b"a", fs.root(), b"b", RenameFlags::default()).unwrap();
    assert!(fs.cached_page(b, 0).is_none());
    assert_eq!(fs.lookup(fs.root(), b"b").unwrap(), a);
    fs.sync().unwrap();
    assert_eq!(probe.contents(a), b"new");
}

#[test]
fn writeback_errors_reach_sync_once() {
    let (cache, probe, fs) = setup(1000);
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(ino, 0, b"lost").unwrap();
    probe.fail_writes.store(true, Ordering::Relaxed);
    // The writeback thread meets the error first; sync reports it.
    assert_eq!(cache.writeback_all().unwrap_err(), Errno::ENOSPC);
    assert_eq!(fs.page_counts(), (1, 0), "the page is clean: the data is lost");
    probe.fail_writes.store(false, Ordering::Relaxed);
    assert_eq!(fs.sync().unwrap_err(), Errno::ENOSPC);
    assert_eq!(fs.sync(), Ok(()));
    assert_eq!(cache.stats().write_errors, 1);
}

/// A file deleted while its pages are being written back: its data does
/// not matter any more, so that is no error for the file system's next
/// `sync` (which once failed an unrelated file's fsync with `ENOENT`).
#[test]
fn a_file_deleted_during_its_writeback_is_no_error() {
    let (cache, probe, fs) = setup(1000);
    let before = live();
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(ino, 0, &[7; 3 * PAGE_SIZE]).unwrap();
    probe.unlink_f_on_write.store(true, Ordering::Relaxed);
    // The writeback thread writes `f` back as it goes.
    assert_eq!(cache.writeback_all(), Ok(0));
    assert_eq!(fs.page_counts(), (0, 0), "the deleted file's pages are gone");
    assert_eq!(live(), before);
    // Another file's fsync (a sync of the whole file system) is clean.
    let g = fs.create(fs.root(), b"g", &REG).unwrap();
    fs.write(g, 0, b"kept").unwrap();
    assert_eq!(fs.sync(), Ok(()));
    assert_eq!(probe.contents(g), b"kept");
    assert_eq!(cache.stats().write_errors, 0);
    // The same through sync itself.
    let f = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(f, 0, b"gone").unwrap();
    probe.unlink_f_on_write.store(true, Ordering::Relaxed);
    assert_eq!(fs.sync(), Ok(()));
    assert!(fs.cached_page(f, 0).is_none());
}

/// Files created, written and deleted while the writeback thread runs:
/// no writeback error ever reaches `sync`.
#[test]
fn deleting_files_during_writeback_leaves_no_error() {
    let (cache, _probe, fs) = setup(1000);
    let stop = Arc::new(AtomicBool::new(false));
    let writeback = {
        let (cache, stop) = (cache.clone(), Arc::clone(&stop));
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = cache.writeback_all();
            }
        })
    };
    for i in 0..2000u32 {
        let name = [b'a' + (i % 4) as u8];
        let ino = fs.create(fs.root(), &name, &REG).unwrap();
        fs.write(ino, 0, &pattern(i as u8, 2 * PAGE_SIZE + 100)).unwrap();
        fs.unlink(fs.root(), &name).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    writeback.join().unwrap();
    assert_eq!(fs.sync(), Ok(()));
    assert_eq!(cache.stats().write_errors, 0);
    assert_eq!(fs.page_counts(), (0, 0));
}

#[test]
fn appends_do_not_interleave() {
    let (_cache, _probe, fs) = setup(1000);
    let ino = fs.create(fs.root(), b"log", &REG).unwrap();
    let threads: Vec<_> = (0..4u8)
        .map(|t| {
            let fs = Arc::clone(&fs);
            thread::spawn(move || {
                for i in 0..200u32 {
                    let record = [t; 100];
                    let (at, n) = fs.append(ino, &record).unwrap();
                    assert_eq!(n, 100);
                    assert_eq!(at % 100, 0, "record {i} of thread {t} at {at}");
                }
            })
        })
        .collect();
    threads.into_iter().for_each(|t| t.join().unwrap());
    let got = read_all(&*fs, ino);
    assert_eq!(got.len(), 4 * 200 * 100);
    for record in got.chunks(100) {
        assert!(record.iter().all(|&b| b == record[0]), "a torn record");
    }
}

#[test]
fn concurrent_writers_readers_writeback_and_eviction() {
    let (cache, probe, fs) = setup(64);
    let stop = Arc::new(AtomicBool::new(false));
    let flusher = {
        let (cache, stop) = (cache.clone(), Arc::clone(&stop));
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                cache.writeback_all().unwrap();
                cache.shrink(16);
                thread::yield_now();
            }
        })
    };
    let workers: Vec<_> = (0..4u8)
        .map(|t| {
            let fs = Arc::clone(&fs);
            thread::spawn(move || {
                let ino = fs.create(fs.root(), &[b'f', b'0' + t], &REG).unwrap();
                let data = pattern(t, 40 * PAGE_SIZE + 123);
                for chunk in 0..data.len().div_ceil(5000) {
                    let at = chunk * 5000;
                    let end = (at + 5000).min(data.len());
                    fs.write(ino, at as u64, &data[at..end]).unwrap();
                    let mut back = vec![0u8; end - at];
                    assert_eq!(fs.read(ino, at as u64, &mut back).unwrap(), back.len());
                    assert_eq!(back, &data[at..end]);
                }
                (ino, data)
            })
        })
        .collect();
    let files: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    stop.store(true, Ordering::Relaxed);
    flusher.join().unwrap();
    fs.sync().unwrap();
    for (ino, data) in files {
        assert_eq!(probe.contents(ino), data);
        assert_eq!(read_all(&*fs, ino), data);
    }
}

#[test]
fn dropping_the_file_system_drops_its_pages() {
    let (cache, _probe, fs) = setup(1000);
    let before = live();
    let ino = fs.create(fs.root(), b"f", &REG).unwrap();
    fs.write(ino, 0, &[1; 2 * PAGE_SIZE]).unwrap();
    let held = fs.page(ino, 0).unwrap();
    assert_eq!(cache.stats().filesystems, 1);
    drop(fs);
    let s = cache.stats();
    assert_eq!((s.pages, s.dirty, s.filesystems), (0, 0, 0));
    assert_eq!(live() - before, 1, "a held page outlives the cache's interest in it");
    assert!(!held.is_cached());
    held.mark_dirty();
    assert_eq!(cache.stats().dirty, 0, "a page outside the cache is never written back");
    drop(held);
    assert_eq!(live(), before);
}

/// ext2 on a RAM disk behind the cache: what goes through the cache and
/// `sync` is on the disk for a fresh mount without the cache.
#[test]
fn ext2_behind_the_cache_keeps_the_data() {
    use poly_block::RamDisk;
    use poly_ext2::{DeviceIo, Ext2, FormatOptions, MountOptions};

    let mem: &'static mut [u8] = Box::leak(vec![0u8; 8 << 20].into_boxed_slice());
    let io = Arc::new(DeviceIo::new(RamDisk::new(mem, 512).unwrap()));
    poly_ext2::format(&*io, &FormatOptions::default()).unwrap();
    let ext2 = Arc::new(Ext2::mount(Arc::clone(&io), Arc::new(FixedClock), MountOptions::default()).unwrap());
    let cache: PageCache<TestFrame> = PageCache::new(32);
    let fs = cache.register(Arc::clone(&ext2) as Arc<dyn FileSystem>);
    let dir = fs.mkdir(fs.root(), b"dir", 0o755, 0, 0).unwrap();
    let sizes = [0usize, 1, 4095, 4096, 4097, 50_000, 300_000];
    let mut files = Vec::new();
    for (i, &len) in sizes.iter().enumerate() {
        let ino = fs.create(dir, &[b'f', b'0' + i as u8], &REG).unwrap();
        let data = pattern(i as u8, len);
        // In uneven pieces, reading back as it goes.
        for at in (0..len).step_by(7_000) {
            let end = (at + 7_000).min(len);
            fs.write(ino, at as u64, &data[at..end]).unwrap();
        }
        assert_eq!(read_all(&*fs, ino), data);
        files.push((i, data));
    }
    // Overwrite the middle of the big file and truncate another.
    fs.write(fs.lookup(dir, b"f6").unwrap(), 100_000, b"overwritten").unwrap();
    files[6].1[100_000..100_011].copy_from_slice(b"overwritten");
    fs.truncate(fs.lookup(dir, b"f5").unwrap(), 10_000).unwrap();
    files[5].1.truncate(10_000);
    fs.sync().unwrap();
    assert_eq!(fs.page_counts().1, 0);
    ext2.unmount().unwrap();
    drop(fs);
    drop(ext2);
    let again = Ext2::mount(Arc::clone(&io), Arc::new(FixedClock), MountOptions::default()).unwrap();
    assert!(again.stats().was_clean);
    let dir = again.lookup(again.root(), b"dir").unwrap();
    for (i, data) in files {
        let ino = again.lookup(dir, &[b'f', b'0' + i as u8]).unwrap();
        assert_eq!(read_all(&again, ino), data, "file {i}");
    }
}
