//! The block layer in the kernel: every disk behind a request queue and a
//! buffer cache, under a name.
//!
//! The pieces are the host-tested `poly-block` crate (ADR 0014): a
//! [`RequestQueue`] in front of each device (ordering, elevator, merging,
//! splitting) and a [`BufferCache`] on top (write-back, exact LRU, pinning).
//! This module picks their sizes, gives them memory from the heap, locks
//! them and keeps the list of disks. Decisions are in ADR 0017 and ADR 0018.
//!
//! - `virtio0` is the virtio-blk disk, if there is one ([`root`]): the disk a
//!   file system mounts.
//! - `virtio1`, `virtio2`, ... are the other virtio-blk functions, brought
//!   up when something names them ([`open`]: a mount).
//! - `ram0` is a RAM disk of [`RAM_DISK_BYTES`] from the heap ([`ram`]),
//!   always there, so file system code can be tested without a disk.
//!
//! [`Disk`] is what users hold: whole-block reads and writes through the
//! cache, [`Disk::sync`] to make writes durable, [`Disk::lock`] for pinned
//! buffers and everything else on [`Cached`], and uncached transfers.
//! Writes are write-back: they reach the device when the cache evicts the
//! block or on `sync`, and are durable only after a `sync`.
//!
//! # Locks and waiting
//!
//! Each disk has two locks. The request queue (with the device) is behind a
//! [`SpinLock`], held only to enqueue a request, run the queue or collect a
//! result, and taken by the disk's interrupt handler too. The cache is
//! behind a sleeping [`Mutex`], held across the waits for its I/O. Lock
//! order: cache, then queue; nothing else is taken under the queue lock
//! (waiters are woken after it is released).
//!
//! I/O is synchronous for the caller, who waits for its ticket in
//! [`IoQueue::wait`] (an [`Io`] handle, the hook `poly-block`'s cache waits
//! through). How depends on the disk:
//!
//! - **Interrupt-driven** ([`Disk::use_interrupt`], virtio-blk with MSI-X):
//!   the waiter runs the queue once (which starts its request), then sleeps
//!   on the disk's [`WaitQueue`]. The interrupt handler ([`on_interrupt`])
//!   runs the queue, which collects finished requests and starts waiting
//!   ones, and wakes the waiters if any request finished; each collects its
//!   own result. Whoever runs the queue and finishes requests wakes the
//!   waiters, so a completion picked up by a waiter instead of the handler
//!   is not lost either. A lost request raises no interrupt, so the sleep
//!   ends at the device watchdog's deadline at the latest: the waiter then
//!   runs the queue, the watchdog sees its time limit passed, aborts the
//!   device and fails the request.
//! - **Polled** (the RAM disk, which finishes requests as they are
//!   submitted; virtio-blk without MSI-X): the waiter runs the queue,
//!   yielding the CPU between tries. The watchdog checks its limit on every
//!   poll.
//!
//! Either way interrupts stay on while a thread waits, so the tick keeps
//! time and preempts threads during disk I/O.

use crate::arch::irq::Handler;
use crate::sync::{Mutex, MutexGuard, SpinLock};
use crate::thread::{self, WaitQueue};
use crate::virtio_blk;
use crate::{kernel_test, kprint, kprintln, pci::Size, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed, Ordering::SeqCst};
use poly_block::{
    BlockDevice, Buf, BufferCache, CacheError, CacheSlot, CacheStats, Completion, DeviceInfo, DeviceSlot, IoBuf,
    IoError, IoQueue, Op, Part, QueueStats, QueueStorage, RamDisk, Request, RequestQueue, RequestSlot, SetupError,
    SubmitError, Ticket,
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
/// How long an interrupt-driven waiter sleeps at most when the device has
/// no watchdog deadline to offer (nothing outstanding at the device, say):
/// it then runs the queue itself once.
const RECHECK_NS: u64 = time::NANOS_PER_SEC;

pub const VIRTIO_NAME: &str = "virtio0";
pub const RAM_NAME: &str = "ram0";

/// A device the block layer drives. Boxed, since the two differ in size by
/// hundreds of bytes.
#[derive(Debug)]
pub enum Device<'a> {
    Virtio(Box<virtio_blk::Disk>),
    Ram(Box<RamDisk<'a>>),
}

impl Device<'_> {
    /// When the device's watchdog gives up on what it has outstanding, if
    /// it has a watchdog and anything is outstanding.
    pub fn deadline(&self) -> Option<u64> {
        match self {
            Device::Virtio(d) => d.deadline(),
            Device::Ram(_) => None,
        }
    }
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

/// A disk's request queue, with the device in it.
pub type Queue = RequestQueue<'static, Device<'static>>;

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
    /// Interrupts the disk's handler ran for.
    pub interrupts: u64,
    /// Times a waiter went to sleep for a request (interrupt-driven only).
    pub sleeps: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Queue(SetupError),
    /// The device's block size does not fit the cache.
    BlockSize,
}

/// How a disk's waiters wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// Run the queue, yield, try again.
    Polled,
    /// Sleep until the interrupt on this vector finishes requests.
    Interrupt(u8),
}

/// A named disk with its queue and cache.
pub struct Disk {
    name: &'static str,
    /// What it is, for the log ("RAM disk", "virtio-blk 00:03.0").
    what: String,
    info: DeviceInfo,
    /// Taken for a moment at a time, also by the interrupt handler.
    queue: SpinLock<Queue>,
    /// Held across waits.
    cache: Mutex<BufferCache<'static>>,
    /// Threads waiting for one of this disk's requests to finish.
    waiters: WaitQueue,
    /// Bumped whenever requests finished: what sleeping waiters wait for.
    finished: AtomicU64,
    /// The interrupt vector once interrupts drive completions, 0 while
    /// polled (a device vector is never 0).
    vector: AtomicU8,
    interrupts: AtomicU64,
    sleeps: AtomicU64,
}

impl core::fmt::Debug for Disk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Disk").field("name", &self.name).field("what", &self.what).field("info", &self.info).finish()
    }
}

impl Disk {
    /// Give `dev` (which has nothing in flight) a queue and a cache of
    /// [`CACHE_BYTES`], polled. Everything lives as long as the kernel. Not
    /// listed: [`register`] does that.
    pub fn new(name: &'static str, what: String, dev: Device<'static>) -> Result<&'static Disk, Error> {
        Disk::with_cache(name, what, dev, CACHE_BYTES)
    }

    fn with_cache(
        name: &'static str,
        what: String,
        dev: Device<'static>,
        cache_bytes: usize,
    ) -> Result<&'static Disk, Error> {
        let info = dev.info();
        let tables = Box::leak(Box::new(Tables::new(info.block_size as usize, cache_bytes)));
        let Tables { requests, device, segments, parts, slots, data } = tables;
        let storage = QueueStorage { requests, device, segments, parts };
        let queue = RequestQueue::new(dev, storage).map_err(Error::Queue)?;
        let cache = BufferCache::new(slots, data, info.block_size as usize).ok_or(Error::BlockSize)?;
        Ok(Box::leak(Box::new(Disk {
            name,
            what,
            info,
            queue: SpinLock::new(queue),
            cache: Mutex::new(cache),
            waiters: WaitQueue::new(),
            finished: AtomicU64::new(0),
            vector: AtomicU8::new(0),
            interrupts: AtomicU64::new(0),
            sleeps: AtomicU64::new(0),
        })))
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// What the disk is: "RAM disk", "virtio-blk 00:03.0".
    pub fn description(&self) -> &str {
        &self.what
    }

    pub const fn info(&self) -> &DeviceInfo {
        &self.info
    }

    /// The cache, for pinned buffers and the rest of [`Cached`]. A sleeping
    /// lock, held across I/O.
    pub fn lock(&self) -> Cached<'_> {
        Cached { disk: self, cache: self.cache.lock() }
    }

    /// Run `f` on the device, with the queue's spinlock held (interrupts
    /// off): `f` must be short and must not block.
    pub fn with_device<R>(&self, f: impl FnOnce(&mut Device<'static>) -> R) -> R {
        f(self.queue.lock().device_mut())
    }

    /// A handle for queueing requests and waiting for them, past the
    /// cache.
    pub fn io(&self) -> Io<'_> {
        Io { disk: self, pending: 0 }
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

    /// Read whole blocks straight from the device, past the cache (which
    /// may hold newer data). `buf` must be DMA-able for virtio-blk: heap
    /// memory is, the boot stack is not.
    pub fn read_uncached(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.io().read(first, buf)
    }

    /// Write whole blocks straight to the device, past the cache. Blocks the
    /// cache holds are stale afterwards: [`Cached::invalidate`] (and keep
    /// them from being dirty in the first place).
    pub fn write_uncached(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        self.io().write(first, buf)
    }

    pub fn stats(&self) -> Stats {
        self.lock().stats()
    }

    pub fn waiting(&self) -> Waiting {
        match self.vector.load(SeqCst) {
            0 => Waiting::Polled,
            v => Waiting::Interrupt(v),
        }
    }

    /// The handler and data to install on the vector that will signal this
    /// disk's completions (see [`on_interrupt`]).
    pub fn interrupt_handler(&'static self) -> (Handler, usize) {
        (on_interrupt, self as *const Disk as usize)
    }

    /// From now on interrupts on `vector` (running [`Disk::interrupt_handler`])
    /// signal completions, so waiters sleep. Call once the device
    /// interrupts. Runs the queue once, for whatever finished meanwhile.
    pub fn use_interrupt(&self, vector: u8) {
        self.vector.store(vector, SeqCst);
        self.run(None);
    }

    /// Back to polling, before the interrupt is taken away. Sleeping waiters
    /// are woken to poll.
    pub fn use_polling(&self) {
        self.vector.store(0, SeqCst);
        self.finished.fetch_add(1, SeqCst);
        self.waiters.wake_all();
    }

    /// Run the queue: collect finished requests, start waiting ones. With a
    /// ticket, also collect its result if it is there. If requests finished,
    /// wake the waiters (after releasing the queue). Also returns the
    /// device's watchdog deadline.
    fn run(&self, ticket: Option<Ticket>) -> (Option<Result<(), IoError>>, Option<u64>) {
        let (result, finished, deadline) = {
            let mut q = self.queue.lock();
            let before = q.stats().finished;
            q.process();
            let result = ticket.and_then(|t| q.take(t));
            (result, q.stats().finished != before, q.device().deadline())
        };
        if finished {
            self.finished.fetch_add(1, SeqCst);
            self.waiters.wake_all();
        }
        (result, deadline)
    }

    /// Wait until the request is back and return its result (the ticket is
    /// spent).
    fn wait(&self, t: Ticket) -> Result<(), IoError> {
        loop {
            // Read before running the queue, so a completion that comes in
            // between is seen by the sleep below.
            let seen = self.finished.load(SeqCst);
            let (result, deadline) = self.run(Some(t));
            if let Some(r) = result {
                return r;
            }
            self.pause(seen, deadline);
        }
    }

    /// Wait for requests to finish: sleep until they do (or until the
    /// deadline, when interrupts drive the disk), else yield once.
    fn pause(&self, seen: u64, deadline: Option<u64>) {
        if self.waiting() == Waiting::Polled {
            thread::yield_now();
            return;
        }
        self.sleeps.fetch_add(1, Relaxed);
        let deadline = deadline.unwrap_or_else(|| time::now_ns().saturating_add(RECHECK_NS));
        self.waiters.wait_until_deadline(|| self.finished.load(SeqCst) != seen, deadline);
    }
}

/// The interrupt handler of a disk whose device interrupts when requests
/// finish; `data` is the `&'static Disk` ([`Disk::interrupt_handler`]).
/// Runs the queue and wakes the waiters if requests finished. Runs with
/// interrupts off; takes only the queue's spinlock and, after it, the
/// scheduler's (to wake).
fn on_interrupt(data: usize) {
    // SAFETY: `Disk::interrupt_handler` hands out this handler only with the
    // address of a `&'static Disk`.
    let disk = unsafe { &*(data as *const Disk) };
    disk.interrupts.fetch_add(1, Relaxed);
    disk.run(None);
}

/// One user's way to a disk's queue: requests in, results out, waiting as
/// the disk does (sleeping when interrupts drive it). The buffer cache
/// does its I/O through one.
#[derive(Debug)]
pub struct Io<'d> {
    disk: &'d Disk,
    /// Tickets handed out and not yet waited for.
    pending: usize,
}

impl IoQueue for Io<'_> {
    fn info(&self) -> DeviceInfo {
        self.disk.info
    }

    unsafe fn enqueue(&mut self, op: Op, lba: u64, buf: IoBuf) -> Result<Ticket, SubmitError> {
        loop {
            let seen = self.disk.finished.load(SeqCst);
            // SAFETY: our caller's promise, which is the queue's.
            match unsafe { self.disk.queue.lock().enqueue(op, lba, buf) } {
                Ok(t) => {
                    self.pending += 1;
                    return Ok(t);
                }
                // Every request slot belongs to other users, and frees up as
                // their requests finish and they collect them. (With tickets
                // of its own, our user must collect those first.)
                Err(SubmitError::Busy) if self.pending == 0 => {
                    let (_, deadline) = self.disk.run(None);
                    let soon = time::now_ns().saturating_add(time::NANOS_PER_TICK);
                    self.disk.pause(seen, Some(deadline.map_or(soon, |d| d.min(soon))));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn wait(&mut self, t: Ticket) -> Result<(), IoError> {
        self.pending = self.pending.saturating_sub(1);
        self.disk.wait(t)
    }
}

/// A disk's cache, locked, with the disk's queue to do its I/O through.
pub struct Cached<'d> {
    disk: &'d Disk,
    cache: MutexGuard<'d, BufferCache<'static>>,
}

impl Cached<'_> {
    pub fn info(&self) -> &DeviceInfo {
        &self.disk.info
    }

    pub fn stats(&self) -> Stats {
        let d = self.disk;
        Stats {
            cache: *self.cache.stats(),
            queue: *d.queue.lock().stats(),
            buffers: self.cache.capacity(),
            dirty: self.cache.dirty_count(),
            interrupts: d.interrupts.load(Relaxed),
            sleeps: d.sleeps.load(Relaxed),
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
        let b = self.cache.get(&mut self.disk.io(), block)?;
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
            self.cache.prefetch(&mut self.disk.io(), first, count);
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
        let b = self.cache.get_zeroed(&mut self.disk.io(), block)?;
        self.cache.data_mut(&b).copy_from_slice(data);
        self.cache.release(b);
        Ok(())
    }

    /// Write every changed block and flush the device: afterwards
    /// everything written so far is durable.
    pub fn sync(&mut self) -> Result<(), CacheError> {
        self.cache.sync(&mut self.disk.io())
    }

    /// Forget every cached block that is clean and not pinned; the next
    /// read of each goes to the device.
    pub fn invalidate(&mut self) -> usize {
        self.cache.invalidate()
    }

    /// Block `block`, pinned in the cache until [`Cached::release`].
    pub fn get(&mut self, block: u64) -> Result<Buf, CacheError> {
        self.cache.get(&mut self.disk.io(), block)
    }

    pub fn data(&self, b: &Buf) -> &[u8] {
        self.cache.data(b)
    }

    pub fn release(&mut self, b: Buf) {
        self.cache.release(b)
    }

    /// [`Disk::read_uncached`], with the cache held.
    pub fn read_uncached(&mut self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.disk.read_uncached(first, buf)
    }

    /// [`Disk::write_uncached`], with the cache held.
    pub fn write_uncached(&mut self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        self.disk.write_uncached(first, buf)
    }
}

static DISKS: SpinLock<Vec<&'static Disk>> = SpinLock::new(Vec::new());

/// Give `dev` a queue and a cache of [`CACHE_BYTES`] and list it as `name`.
fn register(name: &'static str, what: String, dev: Device<'static>) -> Result<&'static Disk, Error> {
    let disk = Disk::new(name, what, dev)?;
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

/// Serialises [`open`], so a disk is brought up once.
static OPENING: Mutex<()> = Mutex::new(());

/// The disk called `name`, bringing it up first if it is a virtio-blk disk
/// that has not been started: `virtio<n>` is the `n`-th virtio-blk function
/// in PCI order. `virtio0` comes up at boot ([`init`]); the others only
/// when something asks for them (a mount), so a device nobody names is
/// left alone. `None` if there is no such disk or it cannot be brought up.
pub fn open(name: &str) -> Option<&'static Disk> {
    if let Some(d) = find(name) {
        return Some(d);
    }
    let n: usize = name.strip_prefix("virtio")?.parse().ok()?;
    let _once = OPENING.lock();
    if let Some(d) = find(name) {
        return Some(d);
    }
    let found = virtio_blk::bring_up_device(virtio_blk::nth_function(n)?)?;
    let what = format!("virtio-blk {}", found.bdf);
    // Disks live as long as the kernel, and so do their names.
    let name: &'static str = Box::leak(format!("virtio{n}").into_boxed_str());
    match register(name, what, Device::Virtio(Box::new(found.disk))) {
        Ok(disk) => {
            virtio_blk::start(disk, found.bdf, found.msix);
            log_disk(disk);
            Some(disk)
        }
        Err(e) => {
            kprintln!("block: {name}: no queue or cache: {e:?}");
            None
        }
    }
}

/// The boot log's line about a disk.
fn log_disk(d: &Disk) {
    let i = d.info();
    let s = d.stats();
    let waiting = match d.waiting() {
        Waiting::Polled => String::from("polled"),
        Waiting::Interrupt(v) => format!("interrupt-driven (vector {v:#x})"),
    };
    kprintln!(
        "block: {}: {}, {} blocks of {} bytes ({}){}{}; queue {QUEUE_REQUESTS} requests, cache {} buffers ({}); {waiting}",
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

/// The disk a file system mounts: the virtio-blk disk, if there is one.
pub fn root() -> Option<&'static Disk> {
    find(VIRTIO_NAME)
}

/// The RAM disk.
pub fn ram() -> Option<&'static Disk> {
    find(RAM_NAME)
}

/// Bring up the disks: the virtio-blk one (if any, interrupt-driven if it
/// can be) and the RAM disk, print one line for each and the first bytes of
/// the root disk. Call once, after `pci::init`, with interrupts enabled.
pub fn init() {
    if let Some(found) = virtio_blk::probe() {
        let what = format!("virtio-blk {}", found.bdf);
        match register(VIRTIO_NAME, what, Device::Virtio(Box::new(found.disk))) {
            Ok(disk) => virtio_blk::start(disk, found.bdf, found.msix),
            Err(e) => kprintln!("block: {VIRTIO_NAME}: no queue or cache: {e:?}"),
        }
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
        log_disk(d);
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
        let virtio = disks().iter().any(|d| d.with_device(|dev| matches!(dev, Device::Virtio(_))));
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
        // A disk lives as long as the kernel, so this one's few KiB leak.
        let mem: &'static mut [u8] = Box::leak(pattern(0, 16, SECTOR).into_boxed_slice());
        let ptr = mem.as_ptr();
        let mut disk = RamDisk::new(mem, SECTOR as u32).ok_or("no RAM disk")?;
        disk.set_read_only(true);
        let d = Disk::with_cache("ro", "read-only RAM disk".into(), Device::Ram(Box::new(disk)), 4 * SECTOR)
            .map_err(|_| "no queue or cache")?;
        {
            let mut c = d.lock();
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
        // SAFETY: the RAM disk owns the memory, and nothing uses it now (the
        // disk is idle and only this test knows it); it is only read here.
        let mem = unsafe { core::slice::from_raw_parts(ptr, 16 * SECTOR) };
        if *mem != pattern(0, 16, SECTOR)[..] { Err("a read-only disk changed") } else { Ok(()) }
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

/// What one reader thread of `concurrent_readers_get_their_own_data` reads.
struct ReadJob {
    disk: &'static Disk,
    /// First block of its range of 60.
    base: u64,
    /// For an interrupt-driven disk, the disk's sleep count when the test
    /// started: the reader goes on past its four rounds until a waiter has
    /// slept since (or `READ_BUDGET_NS` has passed).
    sleeps_before: Option<u64>,
}

/// Thread body: read the job's range a few times, through the cache in
/// runs of 5 blocks and past it in runs of 3, dropping the cache now and
/// then. Exit code: blocks that came back wrong, or `usize::MAX` on an
/// error.
fn read_job(job: usize) -> usize {
    // SAFETY: the test passes the address of a `ReadJob` that it keeps
    // until it has joined this thread.
    let job = unsafe { &*(job as *const ReadJob) };
    let d = job.disk;
    let bs = d.info().block_size as usize;
    let mut wrong = 0;
    let mut buf = vec![0u8; 5 * bs];
    let deadline = time::now_ns() + READ_BUDGET_NS;
    let slept = || job.sleeps_before.is_none_or(|before| d.stats().sleeps > before);
    for round in 0.. {
        if round >= 4 && (slept() || time::now_ns() > deadline) {
            break;
        }
        for first in (job.base..job.base + 60).step_by(5) {
            if d.read(first, &mut buf).is_err() || d.read_uncached(first + 1, &mut buf[..3 * bs]).is_err() {
                return usize::MAX;
            }
            let expect = pattern(first + 1, 3, bs);
            wrong += buf[..3 * bs].chunks(bs).zip(expect.chunks(bs)).filter(|(a, b)| a != b).count();
            if d.read(first, &mut buf).is_err() {
                return usize::MAX;
            }
            wrong += buf.chunks(bs).zip(pattern(first, 5, bs).chunks(bs)).filter(|(a, b)| a != b).count();
        }
        if round % 2 == 1 {
            d.lock().invalidate();
        }
    }
    wrong
}

kernel_test! {
    fn concurrent_readers_get_their_own_data(env: &TestEnv) -> TestResult {
        for d in test_disks(env)? {
            // Overlapping ranges, so the threads share cached blocks and
            // queue requests for the same blocks.
            // A busy host can finish every request before its waiter gets to
            // sleep; the readers then read on until one did.
            let before = d.stats();
            let sleeps_before = (d.waiting() != Waiting::Polled).then_some(before.sleeps);
            let jobs = [ReadJob { disk: d, base: 200, sleeps_before }, ReadJob { disk: d, base: 230, sleeps_before }];
            let mut ids = Vec::new();
            for job in &jobs {
                ids.push(thread::spawn(read_job, job as *const ReadJob as usize).map_err(|_| "spawn failed")?);
            }
            let mut codes = Vec::new();
            for id in ids {
                codes.push(thread::join(id).map_err(|_| "join failed")?);
            }
            let after = d.stats();
            if codes.contains(&usize::MAX) {
                kprintln!("block: {}: a concurrent read failed", d.name());
                return Err("a read failed while another thread read the same disk");
            }
            if codes.iter().any(|&c| c != 0) {
                kprintln!("block: {}: concurrent readers got {codes:?} wrong blocks", d.name());
                return Err("a thread read the wrong data while another read the same disk");
            }
            // At least the uncached reads: 2 threads, 4 rounds, 12 each.
            if after.queue.finished - before.queue.finished < 2 * 4 * 12 {
                return Err("the concurrent readers did not go to the device");
            }
            if d.waiting() != Waiting::Polled && after.sleeps == before.sleeps {
                return Err("readers of an interrupt-driven disk never slept");
            }
        }
        Ok(())
    }
}

static BURNING: AtomicBool = AtomicBool::new(false);
static BURNED: AtomicU64 = AtomicU64::new(0);

/// CPU-bound: counts until told to stop, never blocking or yielding.
fn burn(_: usize) -> usize {
    while BURNING.load(SeqCst) {
        BURNED.fetch_add(1, SeqCst);
    }
    0
}

/// Reads `read_many` makes.
const MANY_READS: u64 = 40;
/// Reads that must give the CPU-bound thread a turn. A busy host completes
/// many requests before the reader sleeps, and other kernel threads (the
/// screen terminal) take some of the turns, so `read_many` keeps going (for
/// up to `READ_BUDGET_NS`) until this many did; waiting with interrupts off
/// gives none however long it reads.
const READS_WITH_A_TURN: usize = 10;
const READ_BUDGET_NS: u64 = 3 * time::NANOS_PER_SEC;

/// Thread body: `MANY_READS` uncached reads from the disk at address
/// `disk`. Exit code: in how many of them the CPU-bound thread made
/// progress, or `usize::MAX` if one failed or returned the wrong data.
fn read_many(disk: usize) -> usize {
    // SAFETY: the test passes the address of a `&'static Disk`.
    let d = unsafe { &*(disk as *const Disk) };
    let bs = d.info().block_size as usize;
    let mut buf = vec![0u8; 4 * bs];
    let mut progressed = 0;
    let deadline = time::now_ns() + READ_BUDGET_NS;
    for i in 0.. {
        if i >= MANY_READS && (progressed >= READS_WITH_A_TURN || time::now_ns() > deadline) {
            break;
        }
        let first = (i * 37) % (d.info().blocks - 4);
        let burned = BURNED.load(SeqCst);
        if d.read_uncached(first, &mut buf).is_err() || buf != pattern(first, 4, bs) {
            return usize::MAX;
        }
        progressed += (BURNED.load(SeqCst) != burned) as usize;
    }
    progressed
}

kernel_test! {
    /// While one thread does many disk reads, a CPU-bound thread keeps
    /// running: the reader gives up the CPU while it waits (sleeping until
    /// the interrupt, or yielding when polled), and the tick keeps coming.
    /// When a waiter spun with the disk's spinlock held, interrupts were off
    /// and nothing else ran during a read.
    fn a_cpu_bound_thread_runs_during_disk_reads(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(TEST_DISK_FLAG) {
            return Ok(());
        }
        let d = root().ok_or("no virtio-blk disk")?;
        BURNING.store(true, SeqCst);
        BURNED.store(0, SeqCst);
        let before = d.stats();
        // Both on this CPU, which they have to share: on different CPUs
        // the burner would run beside the reader whether it waits with
        // interrupts on or not.
        let _pin = thread::pin_current();
        let (t0, ticks0) = (time::now_ns(), time::ticks());
        let burner = thread::spawn_here(burn, 0).map_err(|_| "spawn failed")?;
        let reader = thread::spawn_here(read_many, d as *const Disk as usize);
        let progressed = reader.map(thread::join);
        let (t1, ticks1) = (time::now_ns(), time::ticks());
        BURNING.store(false, SeqCst);
        thread::join(burner).map_err(|_| "join failed")?;
        let progressed = progressed.map_err(|_| "spawn failed")?.map_err(|_| "join failed")?;
        let after = d.stats();
        let ms = (t1 - t0) / time::NANOS_PER_MILLI;
        kprintln!(
            "block: {}: reads in {ms} ms beside a CPU-bound thread: it ran during {progressed} of them; {} ticks, {} sleeps, {} interrupts",
            d.name(),
            ticks1 - ticks0,
            after.sleeps - before.sleeps,
            after.interrupts - before.interrupts,
        );
        if progressed == usize::MAX {
            return Err("a read failed or returned the wrong data");
        }
        // A fast or busy host can finish a request before the reader goes to
        // sleep, so not every read gives the other thread a turn; `read_many`
        // reads on until enough did. The regression this guards against,
        // waiting with interrupts off, gives about zero.
        if progressed < READS_WITH_A_TURN {
            return Err("the CPU-bound thread did not run while the reader waited for the disk");
        }
        // The CPU-bound thread never yields or blocks, so each of its turns
        // ended with the tick taking the CPU back after a full time slice:
        // counted in ticks, which a busy host stretches in clock time (it
        // merges the ticks that came due while it held the emulator off) but
        // cannot shorten. With the tick stalled there is no way back to the
        // reader at all, and the boot times out.
        if ticks1 - ticks0 < progressed as u64 * u64::from(thread::TIME_SLICE_TICKS) {
            return Err("the timer tick stalled during disk reads");
        }
        if d.waiting() != Waiting::Polled && after.sleeps - before.sleeps == 0 {
            return Err("the reader of an interrupt-driven disk did not sleep");
        }
        Ok(())
    }
}
