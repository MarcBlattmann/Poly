//! Synchronous I/O straight on a [`BlockDevice`], for when nothing else uses
//! it: early boot, probing a new device, tests. Each call submits one
//! request and polls until it is back. Everything else goes through a
//! [`crate::RequestQueue`], which also splits requests the device cannot
//! take in one piece; these helpers do not.
//!
//! The device must be idle: completions of requests submitted elsewhere
//! would be swallowed.

use crate::{BlockDevice, IoBuf, IoError, Op, Request, RequestId, SubmitError};

/// The id these helpers submit with.
pub const SYNC_ID: RequestId = RequestId(u64::MAX);

/// Read whole blocks starting at `lba` into `buf`.
pub fn read<D: BlockDevice + ?Sized>(dev: &mut D, lba: u64, buf: &mut [u8]) -> Result<(), IoError> {
    let seg = [IoBuf::from_mut(buf)];
    // SAFETY: `buf` is borrowed mutably until `run` has seen the request
    // complete, so nothing else touches it meanwhile.
    unsafe { run(dev, Op::Read, lba, &seg) }
}

/// Write whole blocks starting at `lba` from `buf`.
pub fn write<D: BlockDevice + ?Sized>(dev: &mut D, lba: u64, buf: &[u8]) -> Result<(), IoError> {
    let seg = [IoBuf::from_ref(buf)];
    // SAFETY: `buf` is borrowed until `run` has seen the request complete,
    // so nothing writes it meanwhile; the device only reads it.
    unsafe { run(dev, Op::Write, lba, &seg) }
}

/// Make completed writes durable. A no-op on devices without a write cache.
pub fn flush<D: BlockDevice + ?Sized>(dev: &mut D) -> Result<(), IoError> {
    if !dev.info().write_cache {
        return Ok(());
    }
    // SAFETY: a flush transfers no memory.
    unsafe { run(dev, Op::Flush, 0, &[]) }
}

/// # Safety
///
/// As for [`BlockDevice::submit`], until this returns.
unsafe fn run<D: BlockDevice + ?Sized>(dev: &mut D, op: Op, lba: u64, segments: &[IoBuf]) -> Result<(), IoError> {
    let req = Request { id: SYNC_ID, op, lba, segments };
    // SAFETY: passed on from our caller. Every accepted request comes back
    // from `poll`, and we do not return before it has.
    match unsafe { dev.submit(&req) } {
        Ok(()) => {}
        // An idle device with no room: it will never have any.
        Err(SubmitError::Busy) => return Err(IoError::Busy),
        Err(SubmitError::Rejected(e)) => return Err(e),
    }
    dev.kick();
    loop {
        match dev.poll() {
            Some(c) if c.id == SYNC_ID => return c.result,
            Some(_) => {}
            None => core::hint::spin_loop(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RamDisk;
    extern crate std;
    use std::vec;

    #[test]
    fn round_trip_on_a_ram_disk() {
        let mut mem = vec![0u8; 8 * 512];
        let mut disk = RamDisk::new(&mut mem, 512).unwrap();
        let data: std::vec::Vec<u8> = (0..1024).map(|i| i as u8).collect();
        write(&mut disk, 3, &data).unwrap();
        let mut back = [0u8; 1024];
        read(&mut disk, 3, &mut back).unwrap();
        assert_eq!(&back[..], &data[..]);
        assert_eq!(read(&mut disk, 7, &mut back), Err(IoError::OutOfRange));
        assert_eq!(read(&mut disk, 0, &mut back[..100]), Err(IoError::BadRequest));
        // No write cache: nothing to do.
        assert_eq!(flush(&mut disk), Ok(()));
        disk.set_read_only(true);
        assert_eq!(write(&mut disk, 0, &data), Err(IoError::ReadOnly));
        assert_eq!(&mem[3 * 512..5 * 512], &data[..]);
    }
}
