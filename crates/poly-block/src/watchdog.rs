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

/// The time a [`Watchdog`] measures silence in: any unit, as long as it
/// never goes backwards. Any `Fn() -> u64` is one, such as a kernel's
/// nanosecond clock.
pub trait Clock {
    fn now(&self) -> u64;
}

impl<F: Fn() -> u64> Clock for F {
    fn now(&self) -> u64 {
        self()
    }
}

/// Wraps a device and aborts it when requests are outstanding but none has
/// come back for `limit` (in the [`Clock`]'s units).
///
/// The silence starts when a request goes out while nothing else is
/// outstanding, and starts again whenever one comes back, so a busy device
/// that keeps finishing requests never trips it, however long its queue.
/// The limit is checked whenever someone polls: a polling waiter does so in
/// a loop; an interrupt-driven kernel polls from the device's interrupt
/// handler and, since a lost request raises no interrupt, also when a
/// sleeping waiter reaches [`Watchdog::deadline`]. If the abort itself
/// cannot take the requests back (the device does not finish its reset),
/// the watchdog tries again after another `limit`.
pub struct Watchdog<D, C> {
    dev: D,
    clock: C,
    limit: u64,
    /// When the current silence began; `None` while nothing is outstanding.
    since: Option<u64>,
    /// Requests accepted and not yet handed back.
    outstanding: u64,
    aborts: u64,
}

impl<D, C> core::fmt::Debug for Watchdog<D, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Watchdog")
            .field("limit", &self.limit)
            .field("since", &self.since)
            .field("outstanding", &self.outstanding)
            .field("aborts", &self.aborts)
            .finish()
    }
}

impl<D: Abort, C: Clock> Watchdog<D, C> {
    /// `dev`, which must have nothing in flight, aborted after `limit` (at
    /// least 1) of silence as `clock` measures it.
    pub fn new(dev: D, clock: C, limit: u64) -> Watchdog<D, C> {
        Watchdog { dev, clock, limit: limit.max(1), since: None, outstanding: 0, aborts: 0 }
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

    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// Change the limit (at least 1). It applies to the current silence
    /// too.
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit.max(1);
    }

    /// When the watchdog gives up unless something comes back first: the
    /// time by which a sleeping waiter should poll. `None` while nothing is
    /// outstanding.
    pub fn deadline(&self) -> Option<u64> {
        self.since.map(|s| s.saturating_add(self.limit))
    }

    /// Requests accepted and not yet handed back.
    pub const fn outstanding(&self) -> u64 {
        self.outstanding
    }

    /// A request came back: the silence starts again.
    fn completed(&mut self) {
        self.outstanding = self.outstanding.saturating_sub(1);
        self.since = if self.outstanding > 0 { Some(self.clock.now()) } else { None };
    }
}

impl<D: Abort, C: Clock> BlockDevice for Watchdog<D, C> {
    fn info(&self) -> DeviceInfo {
        self.dev.info()
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        // SAFETY: our caller's promise, passed on unchanged.
        unsafe { self.dev.submit(req) }?;
        if self.outstanding == 0 {
            self.since = Some(self.clock.now());
        }
        self.outstanding += 1;
        Ok(())
    }

    fn kick(&mut self) {
        self.dev.kick()
    }

    fn poll(&mut self) -> Option<Completion> {
        if let Some(c) = self.dev.poll() {
            self.completed();
            return Some(c);
        }
        if self.outstanding == 0 {
            self.since = None;
            return None;
        }
        let now = self.clock.now();
        let since = *self.since.get_or_insert(now);
        if now.saturating_sub(since) < self.limit {
            return None;
        }
        // Another full limit before the next try, should this abort not
        // take the requests back.
        self.since = Some(now);
        self.aborts += 1;
        self.dev.abort();
        let c = self.dev.poll()?;
        self.completed();
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
    use core::cell::Cell;
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

    /// A clock that advances by one every time it is read: the watchdog
    /// then counts polls, as a waiter spinning on the device makes them.
    fn ticking(t: &Cell<u64>) -> impl Fn() -> u64 + '_ {
        move || {
            t.set(t.get() + 1);
            t.get()
        }
    }

    #[test]
    fn lost_requests_fail_after_the_limit() {
        let t = Cell::new(0);
        let mut w = Watchdog::new(BlackHole::new(), ticking(&t), 100);
        let mut buf = [0u8; 512];
        // Nothing outstanding: idle polls cost nothing, not even a clock
        // read.
        for _ in 0..1000 {
            assert_eq!(w.poll(), None);
        }
        assert_eq!((w.aborts(), t.get(), w.deadline()), (0, 0, None));
        assert_eq!(sync::read(&mut w, 0, &mut buf), Err(IoError::DeviceFailed));
        assert_eq!(w.aborts(), 1);
        // The request was submitted, then polled until the limit ran out.
        assert_eq!(w.device().polls, 1000 + 100 + 1);
        assert_eq!((w.outstanding(), w.deadline()), (0, None));
    }

    #[test]
    fn the_deadline_follows_the_last_completion() {
        let now = Cell::new(1000);
        let mut w = Watchdog::new(BlackHole::new(), || now.get(), 50);
        let mut buf = [0u8; 512];
        let seg = [IoBuf::from_mut(&mut buf)];
        let req = |id| Request { id: RequestId(id), op: Op::Read, lba: 0, segments: &seg };
        // SAFETY: the black hole never touches the memory.
        unsafe {
            w.submit(&req(1)).unwrap();
            now.set(1010);
            w.submit(&req(2)).unwrap();
        }
        // The silence began with the first request.
        assert_eq!(w.deadline(), Some(1050));
        // Polls do nothing until the deadline, however many.
        now.set(1049);
        for _ in 0..10_000 {
            assert_eq!(w.poll(), None);
        }
        // One comes back: the silence starts again.
        w.dev.failed.push(RequestId(1));
        w.dev.held.retain(|&id| id != RequestId(1));
        assert!(w.poll().is_some());
        assert_eq!((w.outstanding(), w.deadline()), (1, Some(1099)));
        // A shorter limit applies to the current silence at once.
        w.set_limit(10);
        assert_eq!(w.deadline(), Some(1059));
        now.set(1059);
        assert_eq!(w.poll(), Some(Completion { id: RequestId(2), result: Err(IoError::DeviceFailed) }));
        assert_eq!((w.aborts(), w.outstanding(), w.deadline(), w.limit()), (1, 0, None, 10));
        w.set_limit(0);
        assert_eq!(w.limit(), 1);
    }

    #[test]
    fn a_device_stuck_in_its_reset_is_aborted_again() {
        let mut dev = BlackHole::new();
        dev.stuck_aborts = 2;
        let t = Cell::new(0);
        let mut w = Watchdog::new(dev, ticking(&t), 10);
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
        // A RAM disk completes inside submit, so a limit of one clock tick
        // is plenty.
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
        let t = Cell::new(0);
        let mut w = Watchdog::new(NeverAbort(disk), ticking(&t), 1);
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
