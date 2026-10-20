//! The block layer: everything between a file system and a disk driver,
//! without the hardware.
//!
//! - [`BlockDevice`] is what a driver offers: fixed-size blocks, requests
//!   submitted with an id and completed later, possibly out of order. A
//!   polling driver and an interrupt-driven one look the same from above:
//!   [`BlockDevice::poll`] hands back finished requests, called from a wait
//!   loop today and from the interrupt handler later.
//! - [`RequestQueue`] sits in front of one device. It keeps requests from
//!   several users, sorts them (one-way elevator), merges neighbours into one
//!   device request, splits requests the device cannot take in one go, keeps
//!   overlapping requests and flushes in submission order, and tells each
//!   user when its request is done.
//! - [`BufferCache`] keeps recently used blocks in memory, writes dirty ones
//!   back on eviction or [`BufferCache::sync`], and never evicts a block
//!   someone holds.
//! - [`RamDisk`] is a [`BlockDevice`] over plain memory.
//!
//! Nothing here allocates. Tables live in storage the caller provides, so the
//! kernel decides where they go and how big they are.
//!
//! # Memory a request transfers
//!
//! A device works on a request's memory after [`BlockDevice::submit`] has
//! returned, often by DMA. Rust borrows cannot describe "until the device
//! says it is done", so requests carry raw [`IoBuf`]s and submitting is
//! `unsafe`: the caller promises the memory stays valid and untouched until
//! the completion arrives. The synchronous helpers in [`sync`] and on
//! [`RequestQueue`] wrap that in safe functions that hold a borrow and wait.

#![no_std]

mod cache;
mod queue;
mod ramdisk;
pub mod sync;

pub use cache::{Buf, BufferCache, CacheError, CacheSlot, CacheStats};
pub use queue::{DeviceSlot, Part, QueueStats, QueueStorage, RequestQueue, RequestSlot, SetupError, Ticket};
pub use ramdisk::{RAMDISK_COMPLETIONS, RamDisk};

use core::ptr::NonNull;

#[cfg(test)]
mod sim;

/// The smallest block size: one traditional disk sector.
pub const MIN_BLOCK_SIZE: u32 = 512;

/// What a request does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Device to memory.
    Read,
    /// Memory to device.
    Write,
    /// Make every write that completed before this request was submitted
    /// durable. Carries no data. Only devices with
    /// [`DeviceInfo::write_cache`] accept it; on others a completed write is
    /// already durable.
    Flush,
}

/// Chosen by the submitter, handed back in the [`Completion`]. A device
/// does not check that ids in flight are distinct; the submitter keeps them
/// so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// A piece of memory a request transfers to or from, by CPU address. Turning
/// it into a bus address is the driver's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoBuf {
    pub ptr: NonNull<u8>,
    pub len: usize,
}

// SAFETY: an `IoBuf` is only an address and a length; it gives no access by
// itself. Whoever dereferences it does so under the contract of
// `BlockDevice::submit` or `RequestQueue::enqueue`, wherever it runs.
unsafe impl Send for IoBuf {}
// SAFETY: as above; shared references to the address give no access either.
unsafe impl Sync for IoBuf {}

impl IoBuf {
    /// No memory, for flushes and unused table entries.
    pub const EMPTY: IoBuf = IoBuf { ptr: NonNull::dangling(), len: 0 };

    /// For a request that writes into `buf` (a read).
    pub fn from_mut(buf: &mut [u8]) -> IoBuf {
        let len = buf.len();
        IoBuf { ptr: NonNull::from(buf).cast(), len }
    }

    /// For a request that only reads `buf` (a write). The device never
    /// writes through it.
    pub fn from_ref(buf: &[u8]) -> IoBuf {
        IoBuf { ptr: NonNull::from(buf).cast(), len: buf.len() }
    }

    /// The CPU address one past the end.
    pub fn end(&self) -> usize {
        self.ptr.as_ptr() as usize + self.len
    }
}

/// One request as a device sees it.
#[derive(Clone, Copy, Debug)]
pub struct Request<'s> {
    pub id: RequestId,
    pub op: Op,
    /// First block. Ignored for [`Op::Flush`].
    pub lba: u64,
    /// The memory, in order: block `lba` starts at the first byte of the
    /// first segment. Empty for [`Op::Flush`]. The slice itself only needs
    /// to live for the [`BlockDevice::submit`] call; the memory it describes
    /// lives until completion.
    pub segments: &'s [IoBuf],
}

impl Request<'_> {
    /// Check the request against the device's limits. Returns the number of
    /// blocks it transfers (0 for a flush). Drivers call this first thing
    /// in [`BlockDevice::submit`].
    pub fn check(&self, info: &DeviceInfo) -> Result<u32, IoError> {
        if self.op == Op::Flush {
            if !self.segments.is_empty() {
                return Err(IoError::BadRequest);
            }
            return if info.write_cache { Ok(0) } else { Err(IoError::Unsupported) };
        }
        if self.op == Op::Write && info.read_only {
            return Err(IoError::ReadOnly);
        }
        if self.segments.is_empty() || self.segments.len() > info.max_segments as usize {
            return Err(IoError::BadRequest);
        }
        let mut bytes = 0u64;
        for s in self.segments {
            if s.len == 0 || s.len > info.max_segment_len as usize {
                return Err(IoError::BadRequest);
            }
            bytes += s.len as u64;
        }
        let bs = info.block_size as u64;
        if !bytes.is_multiple_of(bs) || bytes / bs > info.max_blocks as u64 {
            return Err(IoError::BadRequest);
        }
        let blocks = bytes / bs;
        match self.lba.checked_add(blocks) {
            Some(end) if end <= info.blocks => Ok(blocks as u32),
            _ => Err(IoError::OutOfRange),
        }
    }
}

/// A finished request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub id: RequestId,
    pub result: Result<(), IoError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoError {
    /// The device reported that the transfer failed (a media or backend
    /// error). For a read the memory's contents are undefined.
    Media,
    /// The device does not support the operation.
    Unsupported,
    /// A write to a read-only device.
    ReadOnly,
    /// The blocks lie (partly) beyond the end of the device.
    OutOfRange,
    /// The request breaks the rules: no data, a length that is not a whole
    /// number of blocks, too many blocks or segments, a segment too long.
    BadRequest,
    /// The driver cannot hand this memory to the device (no bus address).
    BadBuffer,
    /// The device broke its protocol or stopped working, and was reset.
    /// Requests in flight at the time fail with this, and so does every
    /// later one.
    DeviceFailed,
    /// No room for another request, and nothing in flight would make room
    /// (finished requests nobody collected hold every slot).
    Busy,
}

/// Why [`BlockDevice::submit`] did not take a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The device is full right now. Poll for completions and retry.
    Busy,
    /// The request can never succeed. It was not started.
    Rejected(IoError),
}

/// Shape and limits of a device. Constant while the device is in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Bytes per block: a power of two, at least [`MIN_BLOCK_SIZE`]. Every
    /// transfer is a whole number of blocks.
    pub block_size: u32,
    /// Capacity in blocks.
    pub blocks: u64,
    pub read_only: bool,
    /// Completed writes may sit in a volatile cache until an [`Op::Flush`].
    /// Only such devices accept flushes.
    pub write_cache: bool,
    /// Most blocks one request may transfer.
    pub max_blocks: u32,
    /// Most segments one request may have.
    pub max_segments: u16,
    /// Most bytes one segment may have.
    pub max_segment_len: u32,
}

impl DeviceInfo {
    /// Plausible numbers: a valid block size, every limit at least 1, and
    /// room in one request's segments for at least one block.
    pub fn is_valid(&self) -> bool {
        self.block_size.is_power_of_two()
            && self.block_size >= MIN_BLOCK_SIZE
            && self.max_blocks >= 1
            && self.max_segments >= 1
            && self.max_segment_len >= 1
            && self.max_segments as u64 * self.max_segment_len as u64 >= self.block_size as u64
    }

    /// Capacity in bytes.
    pub fn bytes(&self) -> u64 {
        self.blocks.saturating_mul(self.block_size as u64)
    }
}

/// A block device driver.
///
/// Requests go in with [`BlockDevice::submit`] and come back, in any order,
/// from [`BlockDevice::poll`]. A driver may not start them before
/// [`BlockDevice::kick`], so it can batch its doorbell writes. Every
/// accepted request comes back exactly once, also when the device fails
/// (then with [`IoError::DeviceFailed`]).
pub trait BlockDevice {
    fn info(&self) -> DeviceInfo;

    /// Hand a request to the device. On error nothing was started.
    ///
    /// # Safety
    ///
    /// Until [`BlockDevice::poll`] returns the completion for `req.id`,
    /// every segment's memory must stay valid, for writes if the request is
    /// a [`Op::Read`] and for reads if it is a [`Op::Write`], and nothing
    /// but the device may write it (nor, for a read, read it).
    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError>;

    /// Start what was submitted since the last kick.
    fn kick(&mut self) {}

    /// The next finished request, if any. Never blocks.
    fn poll(&mut self) -> Option<Completion>;
}

impl<D: BlockDevice + ?Sized> BlockDevice for &mut D {
    fn info(&self) -> DeviceInfo {
        (**self).info()
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        // SAFETY: the caller upholds the contract for us.
        unsafe { (**self).submit(req) }
    }

    fn kick(&mut self) {
        (**self).kick()
    }

    fn poll(&mut self) -> Option<Completion> {
        (**self).poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DeviceInfo {
        DeviceInfo {
            block_size: 512,
            blocks: 100,
            read_only: false,
            write_cache: false,
            max_blocks: 8,
            max_segments: 2,
            max_segment_len: 2048,
        }
    }

    fn req(op: Op, lba: u64, segments: &[IoBuf]) -> Request<'_> {
        Request { id: RequestId(1), op, lba, segments }
    }

    #[test]
    fn request_checks() {
        let mut mem = [0u8; 4096];
        let (a, b) = mem.split_at_mut(2048);
        let (a, b) = (IoBuf::from_mut(a), IoBuf::from_mut(b));
        let i = info();
        assert!(i.is_valid());
        assert_eq!(req(Op::Read, 0, &[a]).check(&i), Ok(4));
        assert_eq!(req(Op::Write, 92, &[a, b]).check(&i), Ok(8));
        assert_eq!(req(Op::Write, 93, &[a, b]).check(&i), Err(IoError::OutOfRange));
        assert_eq!(req(Op::Read, u64::MAX, &[a]).check(&i), Err(IoError::OutOfRange));
        assert_eq!(req(Op::Read, 0, &[]).check(&i), Err(IoError::BadRequest));
        assert_eq!(req(Op::Read, 0, &[a, b, a]).check(&i), Err(IoError::BadRequest));
        let odd = IoBuf { len: 100, ..a };
        assert_eq!(req(Op::Read, 0, &[odd]).check(&i), Err(IoError::BadRequest));
        let long = IoBuf { len: 2560, ..a };
        assert_eq!(req(Op::Read, 0, &[long]).check(&i), Err(IoError::BadRequest));
        assert_eq!(req(Op::Read, 0, &[IoBuf::EMPTY]).check(&i), Err(IoError::BadRequest));
        // Two half-blocks make a block.
        let half = IoBuf { len: 256, ..a };
        assert_eq!(req(Op::Read, 0, &[half, half]).check(&i), Ok(1));
        assert_eq!(req(Op::Flush, 0, &[]).check(&i), Err(IoError::Unsupported));
        let ro = DeviceInfo { read_only: true, write_cache: true, ..i };
        assert_eq!(req(Op::Write, 0, &[a]).check(&ro), Err(IoError::ReadOnly));
        assert_eq!(req(Op::Read, 0, &[a]).check(&ro), Ok(4));
        assert_eq!(req(Op::Flush, 0, &[]).check(&ro), Ok(0));
        assert_eq!(req(Op::Flush, 0, &[a]).check(&ro), Err(IoError::BadRequest));
        let few = DeviceInfo { max_blocks: 3, ..i };
        assert_eq!(req(Op::Read, 0, &[a]).check(&few), Err(IoError::BadRequest));
    }

    #[test]
    fn info_validity() {
        let i = info();
        assert!(!DeviceInfo { block_size: 256, ..i }.is_valid());
        assert!(!DeviceInfo { block_size: 1536, ..i }.is_valid());
        assert!(!DeviceInfo { max_blocks: 0, ..i }.is_valid());
        assert!(!DeviceInfo { max_segments: 0, ..i }.is_valid());
        assert!(!DeviceInfo { max_segment_len: 0, ..i }.is_valid());
        // Two segments of 255 bytes cannot carry a 512-byte block.
        assert!(!DeviceInfo { max_segment_len: 255, ..i }.is_valid());
        assert!(DeviceInfo { max_segment_len: 256, ..i }.is_valid());
        assert_eq!(i.bytes(), 51200);
    }
}
