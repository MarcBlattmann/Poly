//! Signal files (signalfd(2), ADR 0042): a [`FileObject`] whose reads take
//! the reading thread's pending signals of its mask out of the
//! personality's state, as `struct signalfd_siginfo`s.
//!
//! The file belongs to no process: like a descriptor inherited across
//! `fork`, it reads the signals of whichever thread reads it. Every signal
//! queued for any process notifies one wait queue all signal files share
//! ([`signal_queued`]); waiters then check their own pending signals, so a
//! wake-up for another process's signal costs a look, nothing more.

use crate::sync::SetOnce;
use crate::user;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_personality_linux::kernel::SigSet;
use poly_personality_linux::sys::signalfd::{SIGNALFD_SIGINFO_SIZE, siginfo_bytes};
use poly_vfs::{Errno, FileObject, PollEvents, Result, WaitQueue};

static QUEUE: SetOnce<Arc<WaitQueue>> = SetOnce::new();

/// The queue every signal file's waiters register on.
fn queue() -> &'static Arc<WaitQueue> {
    if let Some(q) = QUEUE.get() {
        return q;
    }
    let _ = QUEUE.set(Arc::new(WaitQueue::new()));
    loop {
        // Another thread may be storing its queue this very moment.
        if let Some(q) = QUEUE.get() {
            return q;
        }
        core::hint::spin_loop();
    }
}

/// A signal was queued for process `pid` (blocked or not): readers of
/// signal files look again.
pub fn signal_queued(_pid: i32) {
    if let Some(q) = QUEUE.get() {
        q.notify(PollEvents::READABLE);
    }
}

/// One signal file.
#[derive(Debug)]
pub struct SignalFd {
    mask: AtomicU64,
    queue: Arc<WaitQueue>,
}

impl SignalFd {
    pub fn new(mask: SigSet) -> SignalFd {
        SignalFd { mask: AtomicU64::new(mask.0), queue: Arc::clone(queue()) }
    }

    /// signalfd(2) on an existing signal file: a new mask.
    pub fn set_mask(&self, mask: SigSet) {
        self.mask.store(mask.0, Ordering::SeqCst);
        // Signals pending already may match now.
        self.queue.notify(PollEvents::READABLE);
    }

    pub fn mask(&self) -> SigSet {
        SigSet(self.mask.load(Ordering::SeqCst))
    }

    /// Run `f` on the reading thread's personality state, with its id.
    fn with_reader<R>(f: impl FnOnce(&mut poly_personality_linux::LinuxProcess, i32) -> R) -> Option<R> {
        let process = user::current_process()?;
        let tid = user::current_tid()? as i32;
        let linux = process.linux.as_ref()?;
        Some(linux.with(|p| if p.has_thread(tid) { Some(f(p, tid)) } else { None })).flatten()
    }
}

impl FileObject for SignalFd {
    /// signalfd(2): as many `signalfd_siginfo`s as fit and are pending
    /// (`EINVAL` for a buffer smaller than one), the lowest signal first
    /// as delivery takes them; none pending would block.
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < SIGNALFD_SIGINFO_SIZE {
            return Err(Errno::EINVAL);
        }
        let mask = self.mask();
        let mut n = 0;
        while n + SIGNALFD_SIGINFO_SIZE <= buf.len() {
            let taken = Self::with_reader(|p, tid| {
                let info = p.dequeue_from(tid, mask)?;
                let timer = poly_personality_linux::time::timers::signal_taken(p, &info);
                Some((info, timer))
            });
            let Some(Some((mut info, timer))) = taken else { break };
            // A POSIX timer's signal: its overrun count is fixed now.
            if let (Some(t), Some(process)) = (timer, user::current_process()) {
                super::timers::count_taken(&process, t, Some(&mut info));
            }
            buf[n..n + SIGNALFD_SIGINFO_SIZE].copy_from_slice(&siginfo_bytes(&info));
            n += SIGNALFD_SIGINFO_SIZE;
        }
        if n == 0 { Err(Errno::EAGAIN) } else { Ok(n) }
    }

    fn write(&self, _: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    /// Readable while the polling thread has a pending signal of the mask.
    fn poll(&self) -> PollEvents {
        let mask = self.mask();
        let pending = Self::with_reader(|p, tid| !p.pending(tid).intersection(mask).is_empty());
        if pending == Some(true) { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }
}
