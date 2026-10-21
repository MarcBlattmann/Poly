//! Event counters, as eventfd(2) describes them: an unsigned 64-bit
//! counter that writes add to and reads take.
//!
//! - A read needs an 8-byte buffer (`EINVAL`); with the counter at 0 it
//!   would block. Otherwise it returns the counter and sets it to 0, or in
//!   semaphore mode (`EFD_SEMAPHORE`) returns 1 and decrements it.
//! - A write adds an 8-byte value; `0xffffffffffffffff` is `EINVAL`, and a
//!   sum above `0xfffffffffffffffe` would block.
//! - Readable while the counter is above 0, writable while it is below the
//!   maximum (a write of 1 would succeed).

use crate::object::FileObject;
use crate::poll::{PollEvents, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::sync::Arc;

/// The largest value the counter holds (eventfd(2)).
pub const EVENTFD_MAX: u64 = 0xffff_ffff_ffff_fffe;

/// One event counter.
pub struct EventFd {
    count: SpinMutex<u64>,
    semaphore: bool,
    queue: Arc<WaitQueue>,
}

impl EventFd {
    pub fn new(initial: u64, semaphore: bool) -> EventFd {
        EventFd { count: SpinMutex::new(initial.min(EVENTFD_MAX)), semaphore, queue: Arc::new(WaitQueue::new()) }
    }

    /// The counter now.
    pub fn count(&self) -> u64 {
        *self.count.lock()
    }
}

impl core::fmt::Debug for EventFd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EventFd").field("count", &self.count()).field("semaphore", &self.semaphore).finish()
    }
}

impl FileObject for EventFd {
    fn seek_stays_at_zero(&self) -> bool {
        true
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 8 {
            return Err(Errno::EINVAL);
        }
        let value = {
            let mut c = self.count.lock();
            if *c == 0 {
                return Err(Errno::EAGAIN);
            }
            let value = if self.semaphore { 1 } else { *c };
            *c -= value;
            value
        };
        buf[..8].copy_from_slice(&value.to_ne_bytes());
        self.queue.notify(PollEvents::WRITABLE);
        Ok(8)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let Some(bytes) = buf.get(..8) else { return Err(Errno::EINVAL) };
        let value = u64::from_ne_bytes(bytes.try_into().expect("8 bytes"));
        if value == u64::MAX {
            return Err(Errno::EINVAL);
        }
        {
            let mut c = self.count.lock();
            if value > EVENTFD_MAX - *c {
                return Err(Errno::EAGAIN);
            }
            *c += value;
        }
        if value > 0 {
            self.queue.notify(PollEvents::READABLE);
        }
        Ok(8)
    }

    fn poll(&self) -> PollEvents {
        let c = self.count();
        let mut events = PollEvents::NONE;
        if c > 0 {
            events = events | PollEvents::READABLE;
        }
        if c < EVENTFD_MAX {
            events = events | PollEvents::WRITABLE;
        }
        events
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }
}
