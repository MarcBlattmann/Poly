//! seccomp user notifications (seccomp_unotify(2), ADR 0168): the queue
//! between the threads a filter stops with `SECCOMP_RET_USER_NOTIF` and the
//! supervisor that holds the filter's notification descriptor.
//!
//! - A stopped call is a notification: an id, the thread that made the call
//!   and its `struct seccomp_data`. It is **pending** until the supervisor
//!   reads it (`SECCOMP_IOCTL_NOTIF_RECV`), then **sent** until the
//!   supervisor answers (`SECCOMP_IOCTL_NOTIF_SEND`): a value the call
//!   returns, an error, or the word to run the call after all
//!   (`SECCOMP_USER_NOTIF_FLAG_CONTINUE`). The stopped thread takes the
//!   answer and the notification is gone.
//! - An id is valid (`SECCOMP_IOCTL_NOTIF_ID_VALID`) while its thread waits:
//!   not once it was answered, and not when the thread gave up (a signal).
//! - Both sides sleep in the kernel ([`LinuxKernel::seccomp_wait`]), which
//!   the listener wakes through its [`Notifier`] whenever something
//!   changed; poll(2) of the descriptor waits on the same: readable while
//!   a notification is pending, writable while one waits for its answer.
//!
//! [`LinuxKernel::seccomp_wait`]: crate::kernel::LinuxKernel::seccomp_wait
//! - When the descriptor is gone, nobody can answer: the stopped calls and
//!   every later one fail with `ENOSYS`.

use super::SeccompData;
use super::abi::SECCOMP_USER_NOTIF_FLAG_CONTINUE;
use crate::errno::Errno;
use crate::ns::lock::Mutex;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

/// What the kernel hangs on a listener: it is told whenever a sleeper may
/// have something to do, and finds its own wait queue behind `as_any`.
pub trait Notifier: Send + Sync {
    /// A notification arrived, was answered or withdrawn, or the
    /// descriptor went.
    fn changed(&self);
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

/// What the supervisor answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The call returns this value.
    Value(i64),
    /// The call fails with this (negative) error.
    Error(i32),
    /// The call runs.
    Continue,
}

impl Reply {
    /// The answer in a `struct seccomp_notif_resp`: `EINVAL` for flags
    /// other than `CONTINUE` and for a positive `error` (it is 0 or a
    /// negated errno). With `CONTINUE`, `val` and `error` do not matter.
    pub fn of(val: i64, error: i32, flags: u32) -> Result<Reply, Errno> {
        if flags & !SECCOMP_USER_NOTIF_FLAG_CONTINUE != 0 {
            return Err(Errno::EINVAL);
        }
        if flags & SECCOMP_USER_NOTIF_FLAG_CONTINUE != 0 {
            return Ok(Reply::Continue);
        }
        match error {
            0 => Ok(Reply::Value(val)),
            e if e < 0 => Ok(Reply::Error(e)),
            _ => Err(Errno::EINVAL),
        }
    }
}

/// A notification as the supervisor reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notification {
    pub id: u64,
    /// The thread that made the call (its id in the initial pid namespace).
    pub tid: i32,
    pub data: SeccompData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Pending,
    Sent,
    Answered(Reply),
}

struct Entry {
    notification: Notification,
    stage: Stage,
}

struct State {
    next_id: u64,
    entries: Vec<Entry>,
    closed: bool,
}

/// The notifications of one filter.
pub struct Listener {
    state: Mutex<State>,
    notifier: Arc<dyn Notifier>,
}

impl core::fmt::Debug for Listener {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Listener").finish_non_exhaustive()
    }
}

impl Listener {
    pub fn new(notifier: Arc<dyn Notifier>) -> Arc<Listener> {
        Arc::new(Listener {
            // Ids are opaque to the supervisor; they are never 0 and never
            // repeat on a listener.
            state: Mutex::new(State { next_id: 0x5ec0_0000_0000_0001, entries: Vec::new(), closed: false }),
            notifier,
        })
    }

    /// The kernel's side of the listener.
    pub fn notifier(&self) -> &Arc<dyn Notifier> {
        &self.notifier
    }

    fn changed(&self) {
        self.notifier.changed();
    }

    /// The call `data` of thread `tid` waits for the supervisor: its id.
    /// `None` when the descriptor is gone (the call is `ENOSYS`).
    pub fn submit(&self, tid: i32, data: SeccompData) -> Option<u64> {
        let id = {
            let mut s = self.state.lock();
            if s.closed {
                return None;
            }
            let id = s.next_id;
            s.next_id += 1;
            s.entries.push(Entry { notification: Notification { id, tid, data }, stage: Stage::Pending });
            id
        };
        self.changed();
        Some(id)
    }

    /// The oldest notification nobody has read yet, which is then sent.
    pub fn receive(&self) -> Option<Notification> {
        let mut s = self.state.lock();
        let e = s.entries.iter_mut().find(|e| e.stage == Stage::Pending)?;
        e.stage = Stage::Sent;
        Some(e.notification)
    }

    /// The supervisor could not take notification `id` after all (its
    /// buffer faulted): it is pending again.
    pub fn unreceive(&self, id: u64) {
        if let Some(e) = self.state.lock().entries.iter_mut().find(|e| e.notification.id == id)
            && e.stage == Stage::Sent
        {
            e.stage = Stage::Pending;
        }
        self.changed();
    }

    /// Whether the thread of notification `id` still waits for an answer.
    pub fn valid(&self, id: u64) -> bool {
        let s = self.state.lock();
        s.entries.iter().any(|e| e.notification.id == id && e.stage == Stage::Sent)
    }

    /// The supervisor answers notification `id`: `ENOENT` unless it was
    /// read and is still waiting.
    pub fn send(&self, id: u64, reply: Reply) -> Result<(), Errno> {
        {
            let mut s = self.state.lock();
            let e = s
                .entries
                .iter_mut()
                .find(|e| e.notification.id == id && e.stage == Stage::Sent)
                .ok_or(Errno::ENOENT)?;
            e.stage = Stage::Answered(reply);
        }
        self.changed();
        Ok(())
    }

    /// For the stopped thread: the answer to its notification `id`, once
    /// (the notification is then gone). `Err(ENOSYS)` when no answer can
    /// come any more.
    pub fn answer(&self, id: u64) -> Result<Option<Reply>, Errno> {
        let mut s = self.state.lock();
        let Some(at) = s.entries.iter().position(|e| e.notification.id == id) else { return Err(Errno::ENOSYS) };
        match s.entries[at].stage {
            Stage::Answered(reply) => {
                s.entries.remove(at);
                Ok(Some(reply))
            }
            _ if s.closed => {
                s.entries.remove(at);
                Err(Errno::ENOSYS)
            }
            _ => Ok(None),
        }
    }

    /// The stopped thread gives up waiting (a signal): its id is no
    /// longer valid.
    pub fn abandon(&self, id: u64) {
        self.state.lock().entries.retain(|e| e.notification.id != id);
        self.changed();
    }

    /// The descriptor is gone.
    pub fn close(&self) {
        self.state.lock().closed = true;
        self.changed();
    }

    /// Whether a notification waits to be read / to be answered (poll(2):
    /// readable / writable).
    pub fn waiting(&self) -> (bool, bool) {
        let s = self.state.lock();
        (s.entries.iter().any(|e| e.stage == Stage::Pending), s.entries.iter().any(|e| e.stage == Stage::Sent))
    }

    /// Whether [`receive`](Self::receive) would return a notification.
    pub fn has_pending(&self) -> bool {
        self.waiting().0
    }
}
