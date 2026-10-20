//! How the file system reaches its disk.
//!
//! [`BlockIo`] is the one trait the driver needs: whole device blocks read
//! and written synchronously, and a flush that makes everything written so
//! far durable. It takes `&self` (implementations lock inside), so a mounted
//! file system can share its disk with whoever else uses it.
//!
//! Two implementations come with the crate, over `poly-block`:
//!
//! - [`CachedIo`] puts a [`BufferCache`] in front of an [`IoQueue`] (a
//!   [`poly_block::RequestQueue`], or a kernel's handle on a shared one):
//!   reads and writes go through the cache, `flush` is the cache's `sync`.
//!   This is what the kernel's `block::Disk` is too, so the kernel's own
//!   `BlockIo` for a `&'static Disk` is three one-line methods (see the crate
//!   documentation).
//! - [`DeviceIo`] talks straight to a [`BlockDevice`] (a [`poly_block::RamDisk`],
//!   for instance), one synchronous request at a time.
//!
//! Inside the crate, `Disk` converts the file system's blocks (1-4 KiB) to
//! the device's (512 bytes and up), with read-modify-write when a device
//! block is bigger than a file system block.

use crate::sync::SpinMutex;
use alloc::vec;
use poly_block::{BlockDevice, BufferCache, CacheError, DeviceInfo, IoError, IoQueue};
use poly_vfs::Errno;

/// Synchronous whole-block access to a disk.
///
/// Block numbers and lengths are in the device's blocks
/// ([`BlockIo::block_size`]); buffers are whole blocks. Writes may sit in a
/// volatile cache until [`BlockIo::flush`] returns.
pub trait BlockIo: Send + Sync {
    /// Bytes per device block: a power of two of at least 512.
    fn block_size(&self) -> u32;

    /// Capacity in device blocks.
    fn blocks(&self) -> u64;

    /// The device refuses writes.
    fn read_only(&self) -> bool {
        false
    }

    /// Read the whole blocks from `first` into `buf`.
    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError>;

    /// Write the whole blocks from `first` from `buf`.
    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError>;

    /// Make every write that returned before this call durable: an ordering
    /// barrier as well as a durability point.
    fn flush(&self) -> Result<(), IoError>;
}

impl<T: BlockIo + ?Sized> BlockIo for &T {
    fn block_size(&self) -> u32 {
        (**self).block_size()
    }
    fn blocks(&self) -> u64 {
        (**self).blocks()
    }
    fn read_only(&self) -> bool {
        (**self).read_only()
    }
    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        (**self).read(first, buf)
    }
    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        (**self).write(first, buf)
    }
    fn flush(&self) -> Result<(), IoError> {
        (**self).flush()
    }
}

impl<T: BlockIo + ?Sized> BlockIo for alloc::sync::Arc<T> {
    fn block_size(&self) -> u32 {
        (**self).block_size()
    }
    fn blocks(&self) -> u64 {
        (**self).blocks()
    }
    fn read_only(&self) -> bool {
        (**self).read_only()
    }
    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        (**self).read(first, buf)
    }
    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        (**self).write(first, buf)
    }
    fn flush(&self) -> Result<(), IoError> {
        (**self).flush()
    }
}

/// Checks a transfer against the device: whole blocks, inside the disk.
fn check(info: &DeviceInfo, first: u64, len: usize) -> Result<u64, IoError> {
    let bs = info.block_size as usize;
    if len == 0 || !len.is_multiple_of(bs) {
        return Err(IoError::BadRequest);
    }
    let count = (len / bs) as u64;
    match first.checked_add(count) {
        Some(end) if end <= info.blocks => Ok(count),
        _ => Err(IoError::OutOfRange),
    }
}

/// A [`BlockDevice`] used directly, one synchronous request at a time
/// (`poly_block::sync`), split to the device's limits. The device must not
/// be used by anything else meanwhile.
pub struct DeviceIo<D> {
    dev: SpinMutex<D>,
    info: DeviceInfo,
}

impl<D> core::fmt::Debug for DeviceIo<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceIo").field("info", &self.info).finish()
    }
}

impl<D: BlockDevice> DeviceIo<D> {
    /// `dev` must be idle.
    pub fn new(dev: D) -> DeviceIo<D> {
        let info = dev.info();
        DeviceIo { dev: SpinMutex::new(dev), info }
    }

    pub fn into_inner(self) -> D {
        self.dev.into_inner()
    }

    /// Run `f` on the device (for tests: look at a RAM disk's contents).
    pub fn with_device<R>(&self, f: impl FnOnce(&mut D) -> R) -> R {
        f(&mut self.dev.lock())
    }

    /// Blocks per request: the device's limits, with one segment.
    fn chunk(&self) -> usize {
        let bs = self.info.block_size as usize;
        (self.info.max_blocks as usize).min(self.info.max_segment_len as usize / bs).max(1)
    }
}

impl<D: BlockDevice + Send> BlockIo for DeviceIo<D> {
    fn block_size(&self) -> u32 {
        self.info.block_size
    }
    fn blocks(&self) -> u64 {
        self.info.blocks
    }
    fn read_only(&self) -> bool {
        self.info.read_only
    }
    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        check(&self.info, first, buf.len())?;
        let step = self.chunk() * self.info.block_size as usize;
        let mut dev = self.dev.lock();
        for (i, part) in buf.chunks_mut(step).enumerate() {
            poly_block::sync::read(&mut *dev, first + (i * self.chunk()) as u64, part)?;
        }
        Ok(())
    }
    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        check(&self.info, first, buf.len())?;
        let step = self.chunk() * self.info.block_size as usize;
        let mut dev = self.dev.lock();
        for (i, part) in buf.chunks(step).enumerate() {
            poly_block::sync::write(&mut *dev, first + (i * self.chunk()) as u64, part)?;
        }
        Ok(())
    }
    fn flush(&self) -> Result<(), IoError> {
        poly_block::sync::flush(&mut *self.dev.lock())
    }
}

/// A [`BufferCache`] in front of an [`IoQueue`], behind a lock: reads hit
/// the cache (runs of uncached blocks are read ahead together), writes
/// replace cached blocks and reach the device on eviction or `flush`, which
/// is [`BufferCache::sync`].
pub struct CachedIo<'a, Q> {
    inner: SpinMutex<(BufferCache<'a>, Q)>,
    info: DeviceInfo,
}

impl<Q> core::fmt::Debug for CachedIo<'_, Q> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CachedIo").field("info", &self.info).finish()
    }
}

fn cache_error(e: CacheError) -> IoError {
    match e {
        CacheError::Io(e) => e,
        CacheError::AllPinned => IoError::Busy,
    }
}

impl<'a, Q: IoQueue> CachedIo<'a, Q> {
    /// `cache` must have the device's block size (`None` otherwise).
    pub fn new(cache: BufferCache<'a>, queue: Q) -> Option<CachedIo<'a, Q>> {
        let info = queue.info();
        if cache.block_size() != info.block_size as usize {
            return None;
        }
        Some(CachedIo { inner: SpinMutex::new((cache, queue)), info })
    }

    pub fn into_inner(self) -> (BufferCache<'a>, Q) {
        self.inner.into_inner()
    }

    /// Run `f` on the cache and the queue.
    pub fn with<R>(&self, f: impl FnOnce(&mut BufferCache<'a>, &mut Q) -> R) -> R {
        let mut g = self.inner.lock();
        let (c, q) = &mut *g;
        f(c, q)
    }
}

impl<Q: IoQueue + Send> BlockIo for CachedIo<'_, Q> {
    fn block_size(&self) -> u32 {
        self.info.block_size
    }
    fn blocks(&self) -> u64 {
        self.info.blocks
    }
    fn read_only(&self) -> bool {
        self.info.read_only
    }
    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let count = check(&self.info, first, buf.len())?;
        let bs = self.info.block_size as usize;
        let mut g = self.inner.lock();
        let (cache, q) = &mut *g;
        if count > 1 {
            cache.prefetch(q, first, count);
        }
        for (block, part) in (first..).zip(buf.chunks_mut(bs)) {
            let b = cache.get(q, block).map_err(cache_error)?;
            part.copy_from_slice(cache.data(&b));
            cache.release(b);
        }
        Ok(())
    }
    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        check(&self.info, first, buf.len())?;
        let bs = self.info.block_size as usize;
        let mut g = self.inner.lock();
        let (cache, q) = &mut *g;
        for (block, part) in (first..).zip(buf.chunks(bs)) {
            let b = cache.get_zeroed(q, block).map_err(cache_error)?;
            cache.data_mut(&b).copy_from_slice(part);
            cache.release(b);
        }
        Ok(())
    }
    fn flush(&self) -> Result<(), IoError> {
        let mut g = self.inner.lock();
        let (cache, q) = &mut *g;
        cache.sync(q).map_err(cache_error)
    }
}

pub(crate) fn errno(e: IoError) -> Errno {
    match e {
        IoError::ReadOnly => Errno::EROFS,
        _ => Errno::EIO,
    }
}

/// The file system's view of its disk: file system blocks of `bs` bytes.
pub(crate) struct Disk<D> {
    pub(crate) dev: D,
    /// File system block size.
    bs: usize,
    /// Device block size.
    dbs: usize,
    /// File system blocks the device holds.
    capacity: u64,
}

impl<D: BlockIo> Disk<D> {
    pub(crate) fn new(dev: D, bs: usize) -> Disk<D> {
        let dbs = dev.block_size() as usize;
        let capacity = dev.blocks().saturating_mul(dbs as u64) / bs as u64;
        Disk { dev, bs, dbs, capacity }
    }

    pub(crate) fn set_block_size(&mut self, bs: usize) {
        self.bs = bs;
        self.capacity = self.dev.blocks().saturating_mul(self.dbs as u64) / bs as u64;
    }

    pub(crate) fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Read `buf.len()` bytes at byte offset `at`, any alignment.
    pub(crate) fn read_bytes(&self, at: u64, buf: &mut [u8]) -> Result<(), Errno> {
        let dbs = self.dbs as u64;
        let first = at / dbs;
        let end = (at + buf.len() as u64).div_ceil(dbs);
        let mut tmp = vec![0u8; ((end - first) * dbs) as usize];
        self.dev.read(first, &mut tmp).map_err(errno)?;
        let skip = (at - first * dbs) as usize;
        buf.copy_from_slice(&tmp[skip..skip + buf.len()]);
        Ok(())
    }

    /// Write `buf` at byte offset `at`, any alignment (read-modify-write of
    /// partial device blocks).
    pub(crate) fn write_bytes(&self, at: u64, buf: &[u8]) -> Result<(), Errno> {
        let dbs = self.dbs as u64;
        let first = at / dbs;
        let end = (at + buf.len() as u64).div_ceil(dbs);
        let skip = (at - first * dbs) as usize;
        if skip == 0 && buf.len().is_multiple_of(self.dbs) {
            return self.dev.write(first, buf).map_err(errno);
        }
        let mut tmp = vec![0u8; ((end - first) * dbs) as usize];
        self.dev.read(first, &mut tmp).map_err(errno)?;
        tmp[skip..skip + buf.len()].copy_from_slice(buf);
        self.dev.write(first, &tmp).map_err(errno)
    }

    /// Read file system blocks from `block` into `buf` (whole blocks).
    pub(crate) fn read(&self, block: u64, buf: &mut [u8]) -> Result<(), Errno> {
        if block >= self.capacity || (buf.len() / self.bs) as u64 > self.capacity - block {
            return Err(Errno::EIO);
        }
        if self.bs >= self.dbs {
            self.dev.read(block * (self.bs / self.dbs) as u64, buf).map_err(errno)
        } else {
            self.read_bytes(block * self.bs as u64, buf)
        }
    }

    /// Write file system blocks from `block` (whole blocks).
    pub(crate) fn write(&self, block: u64, buf: &[u8]) -> Result<(), Errno> {
        if block >= self.capacity || (buf.len() / self.bs) as u64 > self.capacity - block {
            return Err(Errno::EIO);
        }
        if self.bs >= self.dbs {
            self.dev.write(block * (self.bs / self.dbs) as u64, buf).map_err(errno)
        } else {
            self.write_bytes(block * self.bs as u64, buf)
        }
    }

    pub(crate) fn flush(&self) -> Result<(), Errno> {
        self.dev.flush().map_err(errno)
    }
}
