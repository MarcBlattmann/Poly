//! Giving up on a device that stops answering; see [`Watchdog`].

use crate::{BlockDevice, Completion, DeviceInfo, Request, SubmitError};

/// A device that can be told to give up on the requests it has in flight.
pub trait Abort: BlockDevice {
    /// Stop working on everything in flight and fail it: each such request
    /// comes back from [`BlockDevice::poll`] (with an error) once the device
    /// can no longer touch its memory, which for DMA hardware means after a
    /// reset. Later requests may fail too; a driver that cannot recover says
    /// so with [`crate::IoError::DeviceFailed`].
    fn abort(&mut self);
}

impl<D: Abort + ?Sized> Abort for &mut D {
    fn abort(&mut self) {
        (**self).abort()
    }
}

/// Wraps a device and aborts it when requests are outstanding but
/// `budget` polls in a row bring nothing back.
///
/// Without a timer a poll count is the only clock there is, so the budget is
/// a number of [`BlockDevice::poll`] calls, which a waiting
/// [`crate::RequestQueue`] makes in a tight loop. Whatever completes resets
/// the count, so a busy device that keeps finishing requests never trips it,
/// however long its queue. If the abort itself cannot take the requests back
/// (the device does not finish its reset), the watchdog tries again after
/// another budget.
pub struct Watchdog<D> {
    dev: D,
    budget: u64,
    /// Polls in a row that returned nothing while requests were outstanding.
    idle: u64,
    /// Requests accepted and not yet handed back.
    outstanding: u64,
    aborts: u64,
}

impl<D> core::fmt::Debug for Watchdog<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Watchdog")
            .field("budget", &self.budget)
            .field("idle", &self.idle)
            .field("outstanding", &self.outstanding)
            .field("aborts", &self.aborts)
            .finish()
    }
}

impl<D: Abort> Watchdog<D> {
    /// `dev`, which must have nothing in flight, aborted after `budget`
    /// fruitless polls (at least 1).
    pub fn new(dev: D, budget: u64) -> Watchdog<D> {
        Watchdog { dev, budget: budget.max(1), idle: 0, outstanding: 0, aborts: 0 }
    }

    pub const fn device(&self) -> &D {
        &self.dev
    }

    /// The device, for driver-specific calls. Submitting or polling through
    /// it would hide requests from the watchdog.
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    /// How often the watchdog has given up on the device.
    pub const fn aborts(&self) -> u64 {
        self.aborts
    }
}

impl<D: Abort> BlockDevice for Watchdog<D> {
    fn info(&self) -> DeviceInfo {
        self.dev.info()
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        // SAFETY: our caller's promise, passed on unchanged.
        unsafe { self.dev.submit(req) }?;
        self.outstanding += 1;
        Ok(())
    }

    fn kick(&mut self) {
        self.dev.kick()
    }

    fn poll(&mut self) -> Option<Completion> {
        if let Some(c) = self.dev.poll() {
            self.idle = 0;
            self.outstanding = self.outstanding.saturating_sub(1);
            return Some(c);
        }
        if self.outstanding == 0 {
            return None;
        }
        self.idle += 1;
        if self.idle < self.budget {
            return None;
        }
        self.idle = 0;
        self.aborts += 1;
        self.dev.abort();
        self.poll_after_abort()
    }
}

impl<D: Abort> Watchdog<D> {
    /// The first completion an abort released, if any, without counting
    /// another idle poll.
    fn poll_after_abort(&mut self) -> Option<Completion> {
        let c = self.dev.poll()?;
        self.outstanding = self.outstanding.saturating_sub(1);
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DeviceSlot, IoBuf, IoError, Op, Part, QueueStorage, RamDisk, RequestId, RequestQueue, RequestSlot, sync,
    };
    extern crate std;
    use std::{vec, vec::Vec};

    /// Takes requests and never finishes them, until aborted; it can also
    /// refuse to let go at first, like a device stuck in its reset.
    struct BlackHole {
        info: DeviceInfo,
        held: Vec<RequestId>,
        failed: Vec<RequestId>,
        stuck_aborts: u32,
        aborts: u32,
        polls: u64,
    }

    impl BlackHole {
        fn new() -> BlackHole {
            let info = DeviceInfo {
                block_size: 512,
                blocks: 64,
                read_only: false,
                write_cache: true,
                max_blocks: 8,
                max_segments: 4,
                max_segment_len: 4096,
            };
            BlackHole { info, held: Vec::new(), failed: Vec::new(), stuck_aborts: 0, aborts: 0, polls: 0 }
        }
    }

    impl BlockDevice for BlackHole {
        fn info(&self) -> DeviceInfo {
            self.info
        }

        unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
            req.check(&self.info).map_err(SubmitError::Rejected)?;
            self.held.push(req.id);
            Ok(())
        }

        fn poll(&mut self) -> Option<Completion> {
            self.polls += 1;
            self.failed.pop().map(|id| Completion { id, result: Err(IoError::DeviceFailed) })
        }
    }

    impl Abort for BlackHole {
        fn abort(&mut self) {
            self.aborts += 1;
            if self.stuck_aborts > 0 {
                self.stuck_aborts -= 1;
                return;
            }
            self.failed.append(&mut self.held);
        }
    }

    #[test]
    fn lost_requests_fail_after_the_budget() {
        let mut w = Watchdog::new(BlackHole::new(), 100);
        let mut buf = [0u8; 512];
        // Nothing outstanding: idle polls cost nothing.
        for _ in 0..1000 {
            assert_eq!(w.poll(), None);
        }
        assert_eq!(w.aborts(), 0);
        assert_eq!(sync::read(&mut w, 0, &mut buf), Err(IoError::DeviceFailed));
        assert_eq!(w.aborts(), 1);
        // The request was submitted, then polled until the budget ran out.
        assert_eq!(w.device().polls, 1000 + 100 + 1);
        assert_eq!(w.outstanding, 0);
    }

    #[test]
    fn a_device_stuck_in_its_reset_is_aborted_again() {
        let mut dev = BlackHole::new();
        dev.stuck_aborts = 2;
        let mut w = Watchdog::new(dev, 10);
        let mut requests = [RequestSlot::EMPTY; 4];
        let mut device = [DeviceSlot::EMPTY; 4];
        let mut segments = [IoBuf::EMPTY; 8];
        let mut parts = [Part::EMPTY; 8];
        let storage =
            QueueStorage { requests: &mut requests, device: &mut device, segments: &mut segments, parts: &mut parts };
        let mut q = RequestQueue::new(&mut w, storage).unwrap();
        let mut a = vec![0u8; 1024];
        let mut b = vec![0u8; 512];
        // SAFETY: the buffers outlive the requests, collected below.
        let (ta, tb) = unsafe {
            (
                q.enqueue(Op::Read, 0, IoBuf::from_mut(&mut a)).unwrap(),
                q.enqueue(Op::Read, 10, IoBuf::from_mut(&mut b)).unwrap(),
            )
        };
        assert_eq!(q.wait(ta), Err(IoError::DeviceFailed));
        assert_eq!(q.wait(tb), Err(IoError::DeviceFailed));
        let _ = q.into_device();
        assert_eq!(w.aborts(), 3);
        assert_eq!(w.device().aborts, 3);
    }

    #[test]
    fn a_working_device_is_never_aborted() {
        let mut mem = vec![0u8; 64 * 512];
        let disk = RamDisk::new(&mut mem, 512).unwrap();
        // A RAM disk completes inside submit, so a budget of one poll is
        // plenty.
        struct NeverAbort<'a>(RamDisk<'a>);
        impl BlockDevice for NeverAbort<'_> {
            fn info(&self) -> DeviceInfo {
                self.0.info()
            }
            unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
                // SAFETY: passed on from our caller.
                unsafe { self.0.submit(req) }
            }
            fn poll(&mut self) -> Option<Completion> {
                self.0.poll()
            }
        }
        impl Abort for NeverAbort<'_> {
            fn abort(&mut self) {
                panic!("aborted a working device");
            }
        }
        let mut w = Watchdog::new(NeverAbort(disk), 1);
        let data = [7u8; 2048];
        let mut back = [0u8; 2048];
        for lba in 0..32 {
            sync::write(&mut w, lba, &data).unwrap();
            sync::read(&mut w, lba, &mut back).unwrap();
            assert_eq!(back, data);
        }
        assert_eq!(w.aborts(), 0);
    }
}
