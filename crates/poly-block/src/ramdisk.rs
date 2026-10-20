//! A block device over plain memory.
//!
//! Requests run to the end inside [`BlockDevice::submit`]; their
//! completions wait in a small ring until [`BlockDevice::poll`] collects
//! them, so a RAM disk looks like any other device to the layers above.
//! There is no write cache: a completed write is as durable as the memory.

use crate::{BlockDevice, Completion, DeviceInfo, MIN_BLOCK_SIZE, Op, Request, RequestId, SubmitError};

/// Completions a [`RamDisk`] holds before it reports [`SubmitError::Busy`].
pub const RAMDISK_COMPLETIONS: usize = 32;

pub struct RamDisk<'a> {
    mem: &'a mut [u8],
    info: DeviceInfo,
    done: [Completion; RAMDISK_COMPLETIONS],
    /// Oldest completion not yet collected.
    head: usize,
    count: usize,
}

impl core::fmt::Debug for RamDisk<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RamDisk").field("info", &self.info).field("pending", &self.count).finish()
    }
}

impl<'a> RamDisk<'a> {
    /// A disk of the whole blocks in `mem`. `None` if `block_size` is not a
    /// power of two of at least [`MIN_BLOCK_SIZE`]. Limits start out as
    /// generous as possible; see [`RamDisk::set_limits`].
    pub fn new(mem: &'a mut [u8], block_size: u32) -> Option<RamDisk<'a>> {
        if !block_size.is_power_of_two() || block_size < MIN_BLOCK_SIZE {
            return None;
        }
        let info = DeviceInfo {
            block_size,
            blocks: (mem.len() / block_size as usize) as u64,
            read_only: false,
            write_cache: false,
            max_blocks: u32::MAX / block_size,
            max_segments: u16::MAX,
            max_segment_len: u32::MAX,
        };
        let empty = Completion { id: RequestId(0), result: Ok(()) };
        Some(RamDisk { mem, info, done: [empty; RAMDISK_COMPLETIONS], head: 0, count: 0 })
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.info.read_only = read_only;
    }

    /// Pretend to be a device with smaller per-request limits, to exercise
    /// the layers above. Returns false (and changes nothing) if the limits
    /// would not let even one block through.
    pub fn set_limits(&mut self, max_blocks: u32, max_segments: u16, max_segment_len: u32) -> bool {
        let info = DeviceInfo { max_blocks, max_segments, max_segment_len, ..self.info };
        if !info.is_valid() {
            return false;
        }
        self.info = info;
        true
    }

    /// The disk's contents.
    pub fn contents(&self) -> &[u8] {
        self.mem
    }

    pub fn contents_mut(&mut self) -> &mut [u8] {
        self.mem
    }
}

impl BlockDevice for RamDisk<'_> {
    fn info(&self) -> DeviceInfo {
        self.info
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        req.check(&self.info).map_err(SubmitError::Rejected)?;
        if self.count == RAMDISK_COMPLETIONS {
            return Err(SubmitError::Busy);
        }
        let mut at = req.lba as usize * self.info.block_size as usize;
        for seg in req.segments {
            // `check` has put the whole transfer inside the disk.
            let disk = &mut self.mem[at..at + seg.len];
            match req.op {
                // SAFETY: the caller guarantees the segment is valid for
                // writes and that nobody else uses it; it cannot overlap the
                // disk memory we hold a unique borrow of.
                Op::Read => unsafe { core::ptr::copy_nonoverlapping(disk.as_ptr(), seg.ptr.as_ptr(), seg.len) },
                // SAFETY: as above, valid for reads.
                Op::Write => unsafe { core::ptr::copy_nonoverlapping(seg.ptr.as_ptr(), disk.as_mut_ptr(), seg.len) },
                Op::Flush => unreachable!("`check` refuses flushes without a write cache"),
            }
            at += seg.len;
        }
        let tail = (self.head + self.count) % RAMDISK_COMPLETIONS;
        self.done[tail] = Completion { id: req.id, result: Ok(()) };
        self.count += 1;
        Ok(())
    }

    fn poll(&mut self) -> Option<Completion> {
        if self.count == 0 {
            return None;
        }
        let c = self.done[self.head];
        self.head = (self.head + 1) % RAMDISK_COMPLETIONS;
        self.count -= 1;
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IoBuf, IoError};
    extern crate std;
    use std::vec;

    #[test]
    fn geometry() {
        let mut mem = vec![0u8; 10_000];
        let d = RamDisk::new(&mut mem, 1024).unwrap();
        assert_eq!(d.info().blocks, 9);
        assert!(d.info().is_valid());
        assert!(RamDisk::new(&mut mem, 1000).is_none());
        assert!(RamDisk::new(&mut mem, 256).is_none());
    }

    #[test]
    fn scatter_gather_and_ring() {
        let mut mem = vec![0u8; 64 * 512];
        let mut d = RamDisk::new(&mut mem, 512).unwrap();
        let mut a = [1u8; 512];
        let mut b = [2u8; 1024];
        let segs = [IoBuf::from_mut(&mut a), IoBuf::from_mut(&mut b)];
        // SAFETY: the buffers outlive the requests, which complete inside
        // `submit`.
        unsafe {
            d.submit(&Request { id: RequestId(7), op: Op::Write, lba: 4, segments: &segs }).unwrap();
        }
        assert_eq!(d.poll(), Some(Completion { id: RequestId(7), result: Ok(()) }));
        assert_eq!(d.poll(), None);
        assert!(d.contents()[4 * 512..5 * 512].iter().all(|&x| x == 1));
        assert!(d.contents()[5 * 512..7 * 512].iter().all(|&x| x == 2));
        // Fill the completion ring.
        let mut one = [0u8; 512];
        let seg = [IoBuf::from_mut(&mut one)];
        for i in 0..RAMDISK_COMPLETIONS as u64 {
            // SAFETY: as above.
            unsafe { d.submit(&Request { id: RequestId(i), op: Op::Read, lba: i, segments: &seg }).unwrap() };
        }
        // SAFETY: as above.
        let r = unsafe { d.submit(&Request { id: RequestId(99), op: Op::Read, lba: 0, segments: &seg }) };
        assert_eq!(r, Err(SubmitError::Busy));
        for i in 0..RAMDISK_COMPLETIONS as u64 {
            assert_eq!(d.poll().unwrap().id, RequestId(i));
        }
        assert_eq!(d.poll(), None);
    }

    #[test]
    fn limits() {
        let mut mem = vec![0u8; 64 * 512];
        let mut d = RamDisk::new(&mut mem, 512).unwrap();
        assert!(!d.set_limits(0, 1, 512));
        assert!(!d.set_limits(4, 1, 511));
        assert!(d.set_limits(4, 2, 256));
        let mut buf = [0u8; 512];
        let seg = [IoBuf::from_mut(&mut buf)];
        // SAFETY: rejected before touching memory.
        let r = unsafe { d.submit(&Request { id: RequestId(1), op: Op::Read, lba: 0, segments: &seg }) };
        assert_eq!(r, Err(SubmitError::Rejected(IoError::BadRequest)));
        // SAFETY: as above.
        let r = unsafe { d.submit(&Request { id: RequestId(1), op: Op::Flush, lba: 0, segments: &[] }) };
        assert_eq!(r, Err(SubmitError::Rejected(IoError::Unsupported)));
    }
}
