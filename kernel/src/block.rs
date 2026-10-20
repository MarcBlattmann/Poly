//! The block layer in the kernel: every disk behind a request queue and a
//! buffer cache, under a name.
//!
//! The pieces are the host-tested `poly-block` crate (ADR 0014): a
//! [`RequestQueue`] in front of each device (ordering, elevator, merging,
//! splitting) and a [`BufferCache`] on top (write-back, exact LRU, pinning).
//! This module picks their sizes, gives them memory from the heap, puts each
//! device's pair behind a lock and keeps the list of disks. Decisions are in
//! ADR 0017.
//!
//! - `virtio0` is the virtio-blk disk, if there is one ([`root`]): the disk a
//!   file system mounts.
//! - `ram0` is a RAM disk of [`RAM_DISK_BYTES`] from the heap ([`ram`]),
//!   always there, so file system code can be tested without a disk.
//!
//! [`Disk`] is what users hold: whole-block reads and writes through the
//! cache, [`Disk::sync`] to make writes durable, and [`Disk::lock`] for
//! pinned buffers and everything else on [`Cached`]. Writes are write-back:
//! they reach the device when the cache evicts the block or on `sync`, and
//! are durable only after a `sync`.
//!
//! I/O is synchronous and polled: the caller spins, holding the disk's lock,
//! until its request is back (the `Watchdog` on virtio-blk bounds that).
//! Once interrupts drive completions, waiting becomes sleeping on the
//! request's ticket (see `virtio_blk.rs`).

use crate::sync::{SpinLock, SpinLockGuard};
use crate::virtio_blk;
use crate::{kernel_test, kprint, kprintln, pci::Size};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use poly_block::{
    BlockDevice, Buf, BufferCache, CacheError, CacheSlot, CacheStats, Completion, DeviceInfo, DeviceSlot, IoBuf,
    IoError, Part, QueueStats, QueueStorage, RamDisk, Request, RequestQueue, RequestSlot, SetupError, SubmitError,
};

/// Bytes of buffer cache per disk.
pub const CACHE_BYTES: usize = 64 * 1024;
/// Size of the RAM disk `ram0`.
pub const RAM_DISK_BYTES: usize = 256 * 1024;
/// Block size of the RAM disk: one sector, like the test disk.
const RAM_DISK_BLOCK: u32 = 512;
/// User requests a queue holds at once (queued or finished, not collected).
const QUEUE_REQUESTS: usize = 32;
/// Requests a queue keeps at the device at once.
const DEVICE_REQUESTS: usize = 4;
/// Segments per device request: up to this many cache buffers (or pieces
/// of a caller's buffer) go to the device in one request. 4 requests of
/// 16 segments plus header and status take 72 descriptors, which fits the
/// virtio-blk queue, so the queue rarely hears "busy".
const SEGMENTS_PER_REQUEST: usize = 16;

pub const VIRTIO_NAME: &str = "virtio0";
pub const RAM_NAME: &str = "ram0";

/// A device the block layer drives. Boxed, since the two differ in size by
/// hundreds of bytes.
#[derive(Debug)]
pub enum Device<'a> {
    Virtio(Box<virtio_blk::Disk>),
    Ram(Box<RamDisk<'a>>),
}

impl BlockDevice for Device<'_> {
    fn info(&self) -> DeviceInfo {
        match self {
            Device::Virtio(d) => d.info(),
            Device::Ram(d) => d.info(),
        }
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        // SAFETY: our caller's promise, passed on unchanged.
        unsafe {
            match self {
                Device::Virtio(d) => d.submit(req),
                Device::Ram(d) => d.submit(req),
            }
        }
    }

    fn kick(&mut self) {
        match self {
            Device::Virtio(d) => d.kick(),
            Device::Ram(d) => d.kick(),
        }
    }

    fn poll(&mut self) -> Option<Completion> {
        match self {
            Device::Virtio(d) => d.poll(),
            Device::Ram(d) => d.poll(),
        }
    }
}

/// Memory for one request queue and one buffer cache.
#[derive(Debug)]
pub struct Tables {
    requests: Vec<RequestSlot>,
    device: Vec<DeviceSlot>,
    segments: Vec<IoBuf>,
    parts: Vec<Part>,
    slots: Vec<CacheSlot>,
    /// The cache's buffers: one heap allocation, so direct-mapped and
    /// physically contiguous, which DMA into it needs (see `dma.rs`).
    data: Vec<u8>,
}

impl Tables {
    /// For a device with blocks of `block_size` bytes and a cache of
    /// `cache_bytes` (at least one buffer).
    pub fn new(block_size: usize, cache_bytes: usize) -> Tables {
        let buffers = (cache_bytes / block_size).max(1);
        let segments = DEVICE_REQUESTS * SEGMENTS_PER_REQUEST;
        Tables {
            requests: vec![RequestSlot::EMPTY; QUEUE_REQUESTS],
            device: vec![DeviceSlot::EMPTY; DEVICE_REQUESTS],
            segments: vec![IoBuf::EMPTY; segments],
            parts: vec![Part::EMPTY; segments],
            slots: vec![CacheSlot::EMPTY; buffers],
            data: vec![0; buffers * block_size],
        }
    }
}

/// Counters of one disk.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub cache: CacheStats,
    pub queue: QueueStats,
    /// Cache buffers, and how many of them hold changes not yet written.
    pub buffers: usize,
    pub dirty: usize,
}

/// A device with its request queue and buffer cache.
pub struct Cached<'a> {
    queue: RequestQueue<'a, Device<'a>>,
    cache: BufferCache<'a>,
}

impl core::fmt::Debug for Cached<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Cached").field("queue", &self.queue).field("cache", &self.cache).finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Queue(SetupError),
    /// The device's block size does not fit the cache.
    BlockSize,
}

impl<'a> Cached<'a> {
    /// Put a queue and a cache in front of `dev` (which has nothing in
    /// flight), in `tables` made for its block size.
    pub fn new(dev: Device<'a>, tables: &'a mut Tables) -> Result<Cached<'a>, Error> {
        let bs = dev.info().block_size as usize;
        let Tables { requests, device, segments, parts, slots, data } = tables;
        let storage = QueueStorage { requests, device, segments, parts };
        let queue = RequestQueue::new(dev, storage).map_err(Error::Queue)?;
        let cache = BufferCache::new(slots, data, bs).ok_or(Error::BlockSize)?;
        Ok(Cached { queue, cache })
    }

    pub fn info(&self) -> &DeviceInfo {
        self.queue.info()
    }

    pub const fn device(&self) -> &Device<'a> {
        self.queue.device()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            cache: *self.cache.stats(),
            queue: *self.queue.stats(),
            buffers: self.cache.capacity(),
            dirty: self.cache.dirty_count(),
        }
    }

    fn block_size(&self) -> usize {
        self.cache.block_size()
    }

    /// Copy block `block` into `buf` (exactly one block long).
    pub fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), CacheError> {
        if buf.len() != self.block_size() {
            return Err(IoError::BadRequest.into());
        }
        let b = self.cache.get(&mut self.queue, block)?;
        buf.copy_from_slice(self.cache.data(&b));
        self.cache.release(b);
        Ok(())
    }

    /// Copy the whole blocks from `first` on into `buf`. Blocks that are not
    /// cached are read ahead together first, so the queue can merge them
    /// into few device requests.
    pub fn read(&mut self, first: u64, buf: &mut [u8]) -> Result<(), CacheError> {
        let bs = self.block_size();
        if buf.is_empty() || !buf.len().is_multiple_of(bs) {
            return Err(IoError::BadRequest.into());
        }
        let count = (buf.len() / bs) as u64;
        if first.checked_add(count).is_none_or(|end| end > self.info().blocks) {
            return Err(IoError::OutOfRange.into());
        }
        if count > 1 {
            self.cache.prefetch(&mut self.queue, first, count);
        }
        for (block, chunk) in (first..).zip(buf.chunks_mut(bs)) {
            self.read_block(block, chunk)?;
        }
        Ok(())
    }

    /// Replace block `block` with `data` (exactly one block long) in the
    /// cache, without reading it first. The device sees it on eviction or
    /// [`Cached::sync`].
    pub fn write_block(&mut self, block: u64, data: &[u8]) -> Result<(), CacheError> {
        if data.len() != self.block_size() {
            return Err(IoError::BadRequest.into());
        }
        let b = self.cache.get_zeroed(&mut self.queue, block)?;
        self.cache.data_mut(&b).copy_from_slice(data);
        self.cache.release(b);
        Ok(())
    }

    /// Write every changed block and flush the device: afterwards
    /// everything written so far is durable.
    pub fn sync(&mut self) -> Result<(), CacheError> {
        self.cache.sync(&mut self.queue)
    }

    /// Forget every cached block that is clean and not pinned; the next
    /// read of each goes to the device.
    pub fn invalidate(&mut self) -> usize {
        self.cache.invalidate()
    }

    /// Block `block`, pinned in the cache until [`Cached::release`].
    pub fn get(&mut self, block: u64) -> Result<Buf, CacheError> {
        self.cache.get(&mut self.queue, block)
    }

    pub fn data(&self, b: &Buf) -> &[u8] {
        self.cache.data(b)
    }

    pub fn release(&mut self, b: Buf) {
        self.cache.release(b)
    }

    /// Read whole blocks straight from the device, past the cache (which
    /// may hold newer data). `buf` must be DMA-able for virtio-blk: heap
    /// memory is, the boot stack is not.
    pub fn read_uncached(&mut self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.queue.read(first, buf)
    }

    /// Write whole blocks straight to the device, past the cache. Blocks the
    /// cache holds are stale afterwards: [`Cached::invalidate`] (and keep
    /// them from being dirty in the first place).
    pub fn write_uncached(&mut self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        self.queue.write(first, buf)
    }
}

/// A named disk with its queue and cache, for the kernel's lifetime.
#[derive(Debug)]
pub struct Disk {
    name: &'static str,
    /// What it is, for the log ("RAM disk", "virtio-blk 00:03.0").
    what: String,
    info: DeviceInfo,
    cached: SpinLock<Cached<'static>>,
}

impl Disk {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// The queue and cache, for pinned buffers and the rest of [`Cached`].
    /// Every I/O waits with the lock held.
    pub fn lock(&self) -> SpinLockGuard<'_, Cached<'static>> {
        self.cached.lock()
    }

    pub fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), CacheError> {
        self.lock().read_block(block, buf)
    }

    pub fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), CacheError> {
        self.lock().read(first, buf)
    }

    pub fn write_block(&self, block: u64, data: &[u8]) -> Result<(), CacheError> {
        self.lock().write_block(block, data)
    }

    pub fn sync(&self) -> Result<(), CacheError> {
        self.lock().sync()
    }

    pub fn stats(&self) -> Stats {
        self.lock().stats()
    }
}

static DISKS: SpinLock<Vec<&'static Disk>> = SpinLock::new(Vec::new());

/// Give `dev` a queue and a cache of [`CACHE_BYTES`] and list it as `name`.
fn register(name: &'static str, what: String, dev: Device<'static>) -> Result<&'static Disk, Error> {
    let info = dev.info();
    let tables = Box::leak(Box::new(Tables::new(info.block_size as usize, CACHE_BYTES)));
    let cached = Cached::new(dev, tables)?;
    let disk: &'static Disk = Box::leak(Box::new(Disk { name, what, info, cached: SpinLock::new(cached) }));
    DISKS.lock().push(disk);
    Ok(disk)
}

/// Every disk, in the order they were found.
pub fn disks() -> Vec<&'static Disk> {
    DISKS.lock().clone()
}

pub fn find(name: &str) -> Option<&'static Disk> {
    DISKS.lock().iter().copied().find(|d| d.name == name)
}

/// The disk a file system mounts: the virtio-blk disk, if there is one.
pub fn root() -> Option<&'static Disk> {
    find(VIRTIO_NAME)
}

/// The RAM disk.
pub fn ram() -> Option<&'static Disk> {
    find(RAM_NAME)
}

/// Bring up the disks: the virtio-blk one (if any) and the RAM disk, print
/// one line for each and the first bytes of the root disk. Call once, after
/// `pci::init`.
pub fn init() {
    if let Some((bdf, disk)) = virtio_blk::probe()
        && let Err(e) = register(VIRTIO_NAME, format!("virtio-blk {bdf}"), Device::Virtio(Box::new(disk)))
    {
        kprintln!("block: {VIRTIO_NAME}: no queue or cache: {e:?}");
    }
    let mem = Box::leak(vec![0u8; RAM_DISK_BYTES].into_boxed_slice());
    match RamDisk::new(mem, RAM_DISK_BLOCK) {
        Some(d) => {
            if let Err(e) = register(RAM_NAME, "RAM disk".into(), Device::Ram(Box::new(d))) {
                kprintln!("block: {RAM_NAME}: no queue or cache: {e:?}");
            }
        }
        None => kprintln!("block: {RAM_NAME}: bad block size"),
    }
    for d in disks() {
        let i = d.info();
        let s = d.stats();
        kprintln!(
            "block: {}: {}, {} blocks of {} bytes ({}){}{}; queue {QUEUE_REQUESTS} requests, cache {} buffers ({})",
            d.name,
            d.what,
            i.blocks,
            i.block_size,
            Size(i.bytes()),
            if i.write_cache { ", write cache" } else { "" },
            if i.read_only { ", read-only" } else { "" },
            s.buffers,
            Size((s.buffers * i.block_size as usize) as u64),
        );
    }
    let Some(root) = root() else { return };
    let mut first = vec![0u8; root.info().block_size as usize];
    match root.read_block(0, &mut first) {
        Ok(()) => {
            kprint!("block: {}: block 0:", root.name);
            for b in &first[..16] {
                kprint!(" {b:02x}");
            }
            let text = first[..16].iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '.' });
            kprint!("  ");
            text.for_each(|c| kprint!("{c}"));
            kprintln!();
        }
        Err(e) => kprintln!("block: {}: reading block 0 failed: {e:?}", root.name),
    }
}

// Tests. They run against the RAM disk always and against the virtio-blk
// disk when the boot test attached the test image, and leave both as they
// found them.

/// The boot test attaches the disk image `cargo xtask` builds and says so
/// with this flag; without it the disk tests only check what is there.
pub const TEST_DISK_FLAG: &str = "poly.test.disk";
/// Size of that image in sectors (1 MiB).
const TEST_DISK_SECTORS: u64 = 2048;
const SECTOR: usize = 512;

/// Sector `sector` of the test image: "POLY", the sector number (le32),
/// then bytes that depend on the sector and the position. Must match
/// `test_disk_sector` in tools/xtask.
fn test_pattern(sector: u64) -> [u8; SECTOR] {
    let mut s = [0u8; SECTOR];
    s[..4].copy_from_slice(b"POLY");
    s[4..8].copy_from_slice(&(sector as u32).to_le_bytes());
    for (i, b) in s.iter_mut().enumerate().skip(8) {
        *b = (sector as u32).wrapping_mul(31).wrapping_add(i as u32) as u8;
    }
    s
}

/// What the test pattern puts into `count` blocks of `bs` bytes from block
/// `first`.
fn pattern(first: u64, count: u64, bs: usize) -> Vec<u8> {
    let per_block = (bs / SECTOR) as u64;
    (first * per_block..(first + count) * per_block).flat_map(test_pattern).collect()
}

/// The pattern changed in a way that depends on `block`, for writes.
fn changed(block: u64, bs: usize) -> Vec<u8> {
    let mut v = pattern(block, 1, bs);
    v[..4].copy_from_slice(b"WRIT");
    v[8..].iter_mut().for_each(|b| *b ^= 0xa5 ^ block as u8);
    v
}

/// A failed operation as a test failure, printed first.
fn ok<T, E: core::fmt::Debug>(disk: &str, what: &'static str, r: Result<T, E>) -> Result<T, &'static str> {
    r.map_err(|e| {
        kprintln!("block: {disk}: {what}: {e:?}");
        what
    })
}

/// The disks to test, each holding the test pattern and nothing dirty or
/// cached: the RAM disk (filled with the pattern first) and, with
/// `poly.test.disk`, the virtio-blk disk.
fn test_disks(env: &crate::ktest::TestEnv) -> Result<Vec<&'static Disk>, &'static str> {
    let mut list = Vec::new();
    let ram = ram().ok_or("no RAM disk")?;
    {
        let mut c = ram.lock();
        let bs = c.info().block_size as usize;
        let all = pattern(0, c.info().blocks, bs);
        if c.stats().dirty != 0 {
            return Err("the RAM disk has dirty blocks between tests");
        }
        ok(RAM_NAME, "fill with the pattern", c.write_uncached(0, &all))?;
        c.invalidate();
    }
    list.push(ram);
    if env.boot.has_flag(TEST_DISK_FLAG) {
        let disk = root().ok_or("poly.test.disk is set but there is no virtio-blk disk")?;
        if disk.info().blocks * (disk.info().block_size as usize / SECTOR) as u64 != TEST_DISK_SECTORS {
            return Err("the test disk does not have 2048 sectors");
        }
        let mut c = disk.lock();
        if c.stats().dirty != 0 {
            return Err("the virtio-blk disk has dirty blocks between tests");
        }
        c.invalidate();
        drop(c);
        list.push(disk);
    }
    Ok(list)
}

kernel_test! {
    fn disks_are_registered(env: &TestEnv) -> TestResult {
        let ram = ram().ok_or("no RAM disk")?;
        let i = ram.info();
        if i.bytes() != RAM_DISK_BYTES as u64 || i.block_size != RAM_DISK_BLOCK || i.read_only || i.write_cache {
            return Err("the RAM disk does not have the expected shape");
        }
        let virtio = disks().iter().any(|d| matches!(d.lock().device(), Device::Virtio(_)));
        if root().is_some() != virtio || (env.boot.has_flag(TEST_DISK_FLAG) && root().is_none()) {
            return Err("the root disk is not the virtio-blk disk");
        }
        for d in disks() {
            let s = d.stats();
            if s.buffers != CACHE_BYTES / d.info().block_size as usize || find(d.name()).is_none() {
                return Err("a disk's cache does not have the configured size");
            }
        }
        Ok(())
    }
}

kernel_test! {
    fn reads_go_through_the_cache(env: &TestEnv) -> TestResult {
        for d in test_disks(env)? {
            let name = d.name();
            let bs = d.info().block_size as usize;
            let last = d.info().blocks - 1;
            let mut buf = vec![0u8; bs];
            let s0 = d.stats();
            for block in [0, last] {
                ok(name, "read", d.read_block(block, &mut buf))?;
                if buf != pattern(block, 1, bs) {
                    return Err("the first or last block does not hold the test pattern");
                }
            }
            let s1 = d.stats();
            if s1.cache.misses != s0.cache.misses + 2 || s1.cache.hits != s0.cache.hits {
                return Err("reading two uncached blocks did not count two misses");
            }
            ok(name, "read again", d.read_block(0, &mut buf))?;
            let s2 = d.stats();
            if s2.cache.hits != s1.cache.hits + 1 || s2.cache.misses != s1.cache.misses {
                return Err("reading a cached block did not count a hit");
            }
            // 20 blocks: read ahead together, merged into fewer requests.
            let mut long = vec![0u8; 20 * bs];
            ok(name, "multi-block read", d.read(3, &mut long))?;
            if long != pattern(3, 20, bs) {
                return Err("a multi-block read returned the wrong data");
            }
            let s3 = d.stats();
            if s3.cache.read_ahead != s2.cache.read_ahead + 20 || s3.queue.merged == s2.queue.merged {
                return Err("a multi-block read was not read ahead in merged requests");
            }
            // Refused before anything reaches the device.
            let out_of_range = Err(CacheError::Io(IoError::OutOfRange));
            let bad = Err(CacheError::Io(IoError::BadRequest));
            if d.read_block(last + 1, &mut buf) != out_of_range
                || d.read(last, &mut long[..2 * bs]) != out_of_range
                || d.read(u64::MAX, &mut buf) != out_of_range
                || d.read_block(0, &mut buf[..100]) != bad
                || d.read(0, &mut long[..bs + 1]) != bad
            {
                return Err("a read past the end or of a partial block was not refused");
            }
        }
        Ok(())
    }
}

kernel_test! {
    fn writes_reach_the_disk_on_sync(env: &TestEnv) -> TestResult {
        const BLOCK: u64 = 1;
        for d in test_disks(env)? {
            let name = d.name();
            let bs = d.info().block_size as usize;
            let (original, new) = (pattern(BLOCK, 1, bs), changed(BLOCK, bs));
            let mut buf = vec![0u8; bs];
            ok(name, "write", d.write_block(BLOCK, &new))?;
            ok(name, "read from the device", d.lock().read_uncached(BLOCK, &mut buf))?;
            if buf != original || d.stats().dirty != 1 {
                return Err("a write reached the device before the block was synced or evicted");
            }
            ok(name, "read back", d.read_block(BLOCK, &mut buf))?;
            if buf != new {
                return Err("the cache did not return what was written");
            }
            let s0 = d.stats();
            ok(name, "sync", d.sync())?;
            let s1 = d.stats();
            if s1.dirty != 0 || s1.cache.write_backs != s0.cache.write_backs + 1 {
                return Err("sync did not write the dirty block exactly once");
            }
            // One write, and a flush if the device has a write cache (on
            // other devices the queue finishes flushes itself).
            let flushes = d.info().write_cache as u64;
            if s1.queue.device_requests != s0.queue.device_requests + 1 + flushes {
                return Err("sync did not send exactly one write and the flush the device needs");
            }
            // Drop the cache: the block must now come from the disk.
            d.lock().invalidate();
            ok(name, "re-read", d.read_block(BLOCK, &mut buf))?;
            if buf != new || d.stats().cache.misses != s1.cache.misses + 1 {
                return Err("after dropping the cache the written block did not come back from the disk");
            }
            for n in [BLOCK - 1, BLOCK + 1] {
                ok(name, "read neighbour", d.read_block(n, &mut buf))?;
                if buf != pattern(n, 1, bs) {
                    return Err("writing one block changed another");
                }
            }
            // Put the disk back the way it was.
            ok(name, "restore", d.write_block(BLOCK, &original))?;
            ok(name, "sync", d.sync())?;
            let mut c = d.lock();
            ok(name, "read restored", c.read_uncached(BLOCK, &mut buf))?;
            if buf != original {
                return Err("the restored block did not reach the disk");
            }
            c.invalidate();
        }
        Ok(())
    }
}

kernel_test! {
    fn eviction_writes_dirty_blocks_back(env: &TestEnv) -> TestResult {
        const FIRST: u64 = 100;
        for d in test_disks(env)? {
            let name = d.name();
            let bs = d.info().block_size as usize;
            let buffers = d.stats().buffers as u64;
            // More dirty blocks than the cache holds.
            let blocks = FIRST..FIRST + buffers + 16;
            if blocks.end > d.info().blocks {
                return Err("the disk is too small for the eviction test");
            }
            let s0 = d.stats();
            for b in blocks.clone() {
                ok(name, "write", d.write_block(b, &changed(b, bs)))?;
            }
            let s1 = d.stats();
            if s1.cache.write_backs < s0.cache.write_backs + 16 || s1.cache.evictions < s0.cache.evictions + 16 {
                return Err("filling the cache with dirty blocks wrote none back");
            }
            let mut c = d.lock();
            let mut buf = vec![0u8; bs];
            // The least recently used blocks were evicted and written back;
            // the last one written is still only in the cache.
            ok(name, "read evicted", c.read_uncached(FIRST, &mut buf))?;
            if buf != changed(FIRST, bs) {
                return Err("an evicted dirty block did not reach the disk");
            }
            ok(name, "read cached", c.read_uncached(blocks.end - 1, &mut buf))?;
            if buf != pattern(blocks.end - 1, 1, bs) {
                return Err("a block still in the cache reached the disk before sync");
            }
            ok(name, "sync", c.sync())?;
            let mut all = vec![0u8; blocks.clone().count() * bs];
            ok(name, "read all", c.read_uncached(FIRST, &mut all))?;
            if all.chunks(bs).zip(blocks.clone()).any(|(got, b)| got != changed(b, bs)) {
                return Err("after sync the disk does not hold every block written");
            }
            // Put the disk back.
            let original = pattern(FIRST, blocks.clone().count() as u64, bs);
            for (b, data) in blocks.clone().zip(original.chunks(bs)) {
                ok(name, "restore", c.write_block(b, data))?;
            }
            ok(name, "sync", c.sync())?;
            ok(name, "read restored", c.read_uncached(FIRST, &mut all))?;
            if all != original {
                return Err("the restored blocks did not reach the disk");
            }
            c.invalidate();
        }
        Ok(())
    }
}

kernel_test! {
    fn pinned_blocks_are_never_evicted(env: &TestEnv) -> TestResult {
        for d in test_disks(env)? {
            let name = d.name();
            let mut c = d.lock();
            let buffers = c.stats().buffers as u64;
            let mut pinned = Vec::new();
            for b in 0..buffers {
                pinned.push(ok(name, "pin", c.get(b))?);
            }
            let mut buf = vec![0u8; c.info().block_size as usize];
            let full = c.read_block(buffers, &mut buf) == Err(CacheError::AllPinned);
            // Pinned blocks can still be read.
            let hit = c.read_block(1, &mut buf).is_ok() && buf == pattern(1, 1, buf.len());
            let data_ok = c.data(&pinned[2]) == pattern(2, 1, buf.len()).as_slice();
            for b in pinned {
                c.release(b);
            }
            if !full || !hit || !data_ok {
                return Err("with every buffer pinned, a new block was cached or a pinned one lost");
            }
            ok(name, "read after release", c.read_block(buffers, &mut buf))?;
            c.invalidate();
        }
        Ok(())
    }
}

kernel_test! {
    fn read_only_disks_refuse_writes() -> TestResult {
        let mut mem = pattern(0, 64, SECTOR);
        let mut disk = RamDisk::new(&mut mem, SECTOR as u32).ok_or("no RAM disk")?;
        disk.set_read_only(true);
        let mut tables = Tables::new(SECTOR, 8 * SECTOR);
        {
            let mut c = Cached::new(Device::Ram(Box::new(disk)), &mut tables).map_err(|_| "no queue or cache")?;
            let mut buf = vec![0u8; SECTOR];
            ok(RAM_NAME, "read", c.read_block(5, &mut buf))?;
            let ro = Err(CacheError::Io(IoError::ReadOnly));
            if buf != pattern(5, 1, SECTOR)
                || c.write_block(5, &changed(5, SECTOR)) != ro
                || c.write_uncached(5, &buf) != Err(IoError::ReadOnly)
                || c.stats().dirty != 0
                || c.sync().is_err()
            {
                return Err("a read-only disk took a write or failed a read");
            }
        }
        if mem != pattern(0, 64, SECTOR) { Err("a read-only disk changed") } else { Ok(()) }
    }
}

kernel_test! {
    fn dma_straight_into_caller_buffers(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(TEST_DISK_FLAG) {
            return Ok(());
        }
        let disk = root().ok_or("no virtio-blk disk")?;
        let mut c = disk.lock();
        // Two sectors straddling a page boundary of a heap buffer: one
        // request, one segment, two frames.
        let page = poly_mem::PAGE_SIZE as usize;
        let mut buf = vec![0u8; 2 * page];
        let at = page - SECTOR;
        ok(VIRTIO_NAME, "read across a page boundary", c.read_uncached(7, &mut buf[at..at + 2 * SECTOR]))?;
        if buf[at..at + 2 * SECTOR] != pattern(7, 2, SECTOR)[..] || buf[..at].iter().any(|&b| b != 0) {
            return Err("a read into a buffer crossing a page boundary returned the wrong data");
        }
        // The boot stack is not in the direct map: refused, not corrupted.
        let mut local = [0u8; SECTOR];
        if (local.as_ptr() as u64) >= crate::arch::KERNEL_VMA
            && c.read_uncached(7, &mut local) != Err(IoError::BadBuffer)
        {
            return Err("a buffer outside the direct map was handed to the device");
        }
        ok(VIRTIO_NAME, "read after a refused buffer", c.read_uncached(7, &mut buf[..SECTOR]))?;
        if buf[..SECTOR] != test_pattern(7) { Err("the device did not recover from a refused buffer") } else { Ok(()) }
    }
}
