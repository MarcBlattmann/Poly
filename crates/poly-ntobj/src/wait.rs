//! Dispatcher objects and waits, as pure logic.
//!
//! The crate decides which waits are satisfied; the kernel does the
//! sleeping. A wait that cannot be satisfied at once returns
//! [`WaitOutcome::Blocked`] with a [`WaitId`]; the kernel puts the thread to
//! sleep. Every later operation that satisfies blocked waits queues a
//! [`Wake`], which the kernel takes with [`ObjectManager::pop_wake`] after
//! the operation and uses to make the thread runnable with the wake's
//! status. Timeouts, alerts and APCs are the kernel's: it ends a blocked
//! wait with [`ObjectManager::cancel_wait`].
//!
//! # Signal state per type
//!
//! - **Notification event**: stays signaled until reset; setting it
//!   satisfies every wait it can.
//! - **Synchronization event**: satisfying one wait resets it.
//! - **Mutant**: signaled when it has no owner. Satisfying a wait makes the
//!   waiting thread the owner (recursion count + 1); its owner can always
//!   wait on it again. Releasing it as often as it was acquired frees it.
//!   If the owner ends while owning it, it is *abandoned*: freed, and the
//!   next wait it satisfies returns `STATUS_ABANDONED_WAIT_0 + index`
//!   (Learn: "WaitForMultipleObjects", "Mutex Objects").
//! - **Semaphore**: signaled while its count is above zero; each satisfied
//!   wait takes one.
//! - **Timer**: like an event of its kind, signaled by the kernel when it
//!   expires ([`ObjectManager::timer_expired`]).
//! - **Opaque waitable objects** (processes, threads, later files): a
//!   notification-style state the kernel sets ([`ObjectManager::set_signaled`]).
//!
//! # Satisfying waits
//!
//! - Wait-any is satisfied by the signaled object with the smallest index
//!   and returns `STATUS_WAIT_0 + index` (Learn: "WaitForMultipleObjects":
//!   "the array index of the signaled object with the smallest index value
//!   of all the signaled objects").
//! - Wait-all is satisfied only when every object is signaled at once, and
//!   then takes all of them atomically. It never holds part of the set, so
//!   a later wait-any can take an object a blocked wait-all also needs. It
//!   returns `STATUS_WAIT_0`, or `STATUS_ABANDONED_WAIT_0 + index` of the
//!   first abandoned mutant (Learn only says "at least one of the objects is
//!   an abandoned mutex"; which index is to be confirmed by the spec agent).
//! - Each object keeps its blocked waits in arrival order. When it becomes
//!   signaled they are tried oldest first, and each one that can now be
//!   satisfied is. Windows documents that waits are *not* guaranteed FIFO
//!   (kernel-mode APCs can reorder them); this strict FIFO, the order in
//!   which several abandoned mutants are freed when their owner ends
//!   (ascending object id), and the rest of the fairness details are to be
//!   confirmed by the spec agent against real Windows.
//! - After every operation no blocked wait is satisfiable (the randomized
//!   model test checks this).
//! - A wait on up to [`MAXIMUM_WAIT_OBJECTS`] (64, `winnt.h`) objects; zero
//!   or more is `STATUS_INVALID_PARAMETER`. The same object twice in a
//!   wait-all is `STATUS_INVALID_PARAMETER_MIX`; twice in a wait-any is
//!   allowed (Learn says the array "may not contain multiple copies of the
//!   same handle"; both statuses are to be confirmed by the spec agent).
//! - A non-waitable object (directory, link, opaque object without a signal
//!   state) is `STATUS_OBJECT_TYPE_MISMATCH` (to be confirmed).
//! - A mutant owned by the waiting thread whose recursion count is at
//!   [`MUTANT_LIMIT`] fails the wait with `STATUS_MUTANT_LIMIT_EXCEEDED`
//!   (`KeWaitForSingleObject` raises it; what `NtWaitFor*` returns is to be
//!   confirmed by the spec agent).
//!
//! # PulseEvent
//!
//! Sets the event, satisfies the waits that can be satisfied at once (all
//! of them for a notification event, one for a synchronization event), and
//! resets it; with no such waits it only resets (Learn: "PulseEvent").
//! Learn calls it unreliable because a waiter briefly taken out of its wait
//! by an APC misses the pulse; here a waiter is only absent while the kernel
//! has cancelled its wait, so the same holds.

use alloc::vec::Vec;
use core::fmt;

use crate::ObjectManager;
use crate::object::{Body, EventType, ObjectId};
use crate::status::*;

/// `MAXIMUM_WAIT_OBJECTS` (`winnt.h`).
pub const MAXIMUM_WAIT_OBJECTS: usize = 64;

/// Highest recursion count of a mutant (`MINLONG` signal state in Windows'
/// terms: a 32-bit signed count). To be confirmed by the spec agent.
pub const MUTANT_LIMIT: u32 = i32::MAX as u32;

/// A thread, as the kernel names it (for example a `poly_sched::ThreadId`
/// packed into 64 bits). Mutant ownership and waits are per thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThreadRef(pub u64);

/// A blocked wait.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WaitId {
    index: u32,
    generation: u32,
}

impl fmt::Debug for WaitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "wait{}.{}", self.index, self.generation)
    }
}

/// `WAIT_TYPE` (`wdm.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitType {
    All,
    Any,
}

/// How long the caller is willing to wait. Only "not at all" matters here;
/// the kernel runs the clock for the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    Infinite,
    /// A zero timeout: never block, answer `STATUS_TIMEOUT` instead.
    Zero,
    /// Any other timeout; the kernel cancels the wait when it expires.
    Finite,
}

/// What a wait call did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// Finished at once with this status (`STATUS_WAIT_0 + i`,
    /// `STATUS_ABANDONED_WAIT_0 + i` or `STATUS_TIMEOUT`).
    Done(NtStatus),
    /// The thread must sleep until a [`Wake`] for this wait or until the
    /// kernel cancels it.
    Blocked(WaitId),
}

/// A blocked wait was satisfied: the kernel makes `thread` runnable and
/// its wait returns `status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wake {
    pub wait: WaitId,
    pub thread: ThreadRef,
    pub status: NtStatus,
}

/// Why the kernel ends a blocked wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitCancel {
    /// The timeout expired: `STATUS_TIMEOUT`.
    Timeout,
    /// The thread was alerted (`NtAlertThread`): `STATUS_ALERTED`.
    /// Alertable waits only.
    Alerted,
    /// A user-mode APC was queued: `STATUS_USER_APC`. Alertable waits only.
    UserApc,
    /// The thread is ending; the status is meaningless.
    Terminated,
}

#[derive(Debug)]
pub(crate) struct WaitBlock {
    thread: ThreadRef,
    objects: Vec<ObjectId>,
    wait_type: WaitType,
    alertable: bool,
}

/// An event's or timer's state (`NtQueryEvent`'s `EVENT_BASIC_INFORMATION`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventInfo {
    pub kind: EventType,
    pub signaled: bool,
}

/// A mutant's state (`NtQueryMutant`'s `MUTANT_BASIC_INFORMATION`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MutantInfo {
    pub owner: Option<ThreadRef>,
    pub recursion: u32,
    pub abandoned: bool,
}

/// A semaphore's state (`NtQuerySemaphore`'s
/// `SEMAPHORE_BASIC_INFORMATION`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemaphoreInfo {
    pub count: i32,
    pub maximum: i32,
}

/// Whether an object would satisfy a wait by `thread` now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ready {
    No,
    Yes,
    Abandoned,
}

impl ObjectManager {
    fn ready(&self, id: ObjectId, thread: ThreadRef) -> Ready {
        match &self.obj(id).body {
            Body::Event { signaled, .. } | Body::Timer { signaled, .. } | Body::Opaque { signaled: Some(signaled) } => {
                if *signaled {
                    Ready::Yes
                } else {
                    Ready::No
                }
            }
            Body::Semaphore { count, .. } => {
                if *count > 0 {
                    Ready::Yes
                } else {
                    Ready::No
                }
            }
            Body::Mutant(m) => match m.owner {
                None if m.abandoned => Ready::Abandoned,
                None => Ready::Yes,
                Some(t) if t == thread && m.count < MUTANT_LIMIT => Ready::Yes,
                Some(_) => Ready::No,
            },
            _ => Ready::No,
        }
    }

    /// Signaled for some thread: worth trying its waiters.
    fn signaled(&self, id: ObjectId) -> bool {
        match &self.obj(id).body {
            Body::Mutant(m) => m.owner.is_none(),
            _ => self.ready(id, ThreadRef(0)) != Ready::No,
        }
    }

    /// The side effect of satisfying a wait.
    fn consume(&mut self, id: ObjectId, thread: ThreadRef) {
        match &mut self.obj_mut(id).body {
            Body::Event { kind: EventType::Synchronization, signaled }
            | Body::Timer { kind: EventType::Synchronization, signaled, .. } => *signaled = false,
            Body::Semaphore { count, .. } => *count -= 1,
            Body::Mutant(m) => {
                m.owner = Some(thread);
                m.count += 1;
                m.abandoned = false;
            }
            _ => {}
        }
    }

    fn waitable(&self, id: ObjectId) -> bool {
        matches!(
            self.obj(id).body,
            Body::Event { .. }
                | Body::Mutant(_)
                | Body::Semaphore { .. }
                | Body::Timer { .. }
                | Body::Opaque { signaled: Some(_) }
        )
    }

    /// Try to satisfy a wait now; on success apply the side effects and
    /// return its status.
    fn try_satisfy(&mut self, thread: ThreadRef, objects: &[ObjectId], wait_type: WaitType) -> Option<NtStatus> {
        match wait_type {
            WaitType::Any => {
                let (i, ready) =
                    objects.iter().map(|&o| self.ready(o, thread)).enumerate().find(|(_, r)| *r != Ready::No)?;
                self.consume(objects[i], thread);
                Some(if ready == Ready::Abandoned { NtStatus::abandoned(i) } else { NtStatus::wait(i) })
            }
            WaitType::All => {
                let mut abandoned = None;
                for (i, &o) in objects.iter().enumerate() {
                    match self.ready(o, thread) {
                        Ready::No => return None,
                        Ready::Abandoned => abandoned = abandoned.or(Some(i)),
                        Ready::Yes => {}
                    }
                }
                for &o in objects {
                    self.consume(o, thread);
                }
                Some(abandoned.map_or(STATUS_WAIT_0, NtStatus::abandoned))
            }
        }
    }

    /// `KeWaitForMultipleObjects` on objects: satisfy the wait now, time out
    /// at once (`Timeout::Zero`), or block.
    pub fn wait_objects(
        &mut self,
        thread: ThreadRef,
        objects: &[ObjectId],
        wait_type: WaitType,
        alertable: bool,
        timeout: Timeout,
    ) -> Result<WaitOutcome> {
        if objects.is_empty() || objects.len() > MAXIMUM_WAIT_OBJECTS {
            return Err(STATUS_INVALID_PARAMETER);
        }
        for &o in objects {
            if !self.is_alive(o) {
                return Err(STATUS_INVALID_HANDLE);
            }
            if !self.waitable(o) {
                return Err(STATUS_OBJECT_TYPE_MISMATCH);
            }
            if let Body::Mutant(m) = &self.obj(o).body
                && m.owner == Some(thread)
                && m.count >= MUTANT_LIMIT
            {
                return Err(STATUS_MUTANT_LIMIT_EXCEEDED);
            }
        }
        if wait_type == WaitType::All && (1..objects.len()).any(|i| objects[..i].contains(&objects[i])) {
            return Err(STATUS_INVALID_PARAMETER_MIX);
        }
        if let Some(status) = self.try_satisfy(thread, objects, wait_type) {
            return Ok(WaitOutcome::Done(status));
        }
        if timeout == Timeout::Zero {
            return Ok(WaitOutcome::Done(STATUS_TIMEOUT));
        }
        let (index, generation) =
            self.waits.insert(WaitBlock { thread, objects: objects.to_vec(), wait_type, alertable });
        let id = WaitId { index, generation };
        for (i, &o) in objects.iter().enumerate() {
            if !objects[..i].contains(&o) {
                self.add_pointer(o);
                self.obj_mut(o).header.waiters.push_back(id);
            }
        }
        Ok(WaitOutcome::Blocked(id))
    }

    /// Take a blocked wait off its objects' queues and drop its references
    /// (deferred).
    fn retire(&mut self, id: WaitId) -> WaitBlock {
        let block = self.waits.remove(id.index, id.generation).expect("live wait");
        for (i, &o) in block.objects.iter().enumerate() {
            if !block.objects[..i].contains(&o) {
                self.obj_mut(o).header.waiters.retain(|&w| w != id);
                self.deferred.push(o);
            }
        }
        block
    }

    /// Satisfy as many of `object`'s waiters as its state allows, oldest
    /// first.
    fn settle(&mut self, object: ObjectId) {
        let mut i = 0;
        while self.signaled(object) {
            let Some(&w) = self.obj(object).header.waiters.get(i) else { break };
            let block = self.waits.get(w.index, w.generation).expect("live wait");
            let (thread, wait_type) = (block.thread, block.wait_type);
            let objects = block.objects.clone();
            match self.try_satisfy(thread, &objects, wait_type) {
                Some(status) => {
                    self.retire(w);
                    self.wakes.push_back(Wake { wait: w, thread, status });
                }
                None => i += 1,
            }
        }
    }

    /// Settle and drop deferred references: the end of every operation that
    /// can signal.
    fn settle_and_flush(&mut self, object: ObjectId) {
        self.settle(object);
        self.flush_deferred();
    }

    /// The next satisfied wait, oldest first.
    pub fn pop_wake(&mut self) -> Option<Wake> {
        self.wakes.pop_front()
    }

    /// Whether a wait is still blocked.
    pub fn wait_pending(&self, id: WaitId) -> bool {
        self.waits.get(id.index, id.generation).is_some()
    }

    /// Blocked waits.
    pub fn pending_waits(&self) -> usize {
        self.waits.len()
    }

    /// End a blocked wait for a reason of the kernel's and return the status
    /// it finishes with. `STATUS_INVALID_PARAMETER` if the wait is no longer
    /// blocked (a [`Wake`] for it is already queued) or if an alert or APC
    /// ends a wait that is not alertable.
    pub fn cancel_wait(&mut self, id: WaitId, reason: WaitCancel) -> Result<NtStatus> {
        let block = self.waits.get(id.index, id.generation).ok_or(STATUS_INVALID_PARAMETER)?;
        let status = match reason {
            WaitCancel::Timeout => STATUS_TIMEOUT,
            WaitCancel::Alerted | WaitCancel::UserApc if !block.alertable => return Err(STATUS_INVALID_PARAMETER),
            WaitCancel::Alerted => STATUS_ALERTED,
            WaitCancel::UserApc => STATUS_USER_APC,
            WaitCancel::Terminated => STATUS_SUCCESS,
        };
        self.retire(id);
        self.flush_deferred();
        Ok(status)
    }

    /// A thread is ending: cancel its blocked waits and abandon the mutants
    /// it owns, in ascending object id order, waking the waits that can
    /// now have them.
    pub fn thread_terminated(&mut self, thread: ThreadRef) {
        let waits: Vec<WaitId> = self
            .waits
            .iter()
            .filter(|(_, _, b)| b.thread == thread)
            .map(|(index, generation, _)| WaitId { index, generation })
            .collect();
        for w in waits {
            self.retire(w);
        }
        let owned: Vec<ObjectId> = self
            .objects
            .iter()
            .filter(|(_, _, o)| matches!(&o.body, Body::Mutant(m) if m.owner == Some(thread)))
            .map(|(index, generation, _)| ObjectId { index, generation })
            .collect();
        for m in owned {
            if let Body::Mutant(mutant) = &mut self.obj_mut(m).body {
                mutant.owner = None;
                mutant.count = 0;
                mutant.abandoned = true;
            }
            self.settle(m);
        }
        self.threads.remove(&thread);
        self.flush_deferred();
    }

    // ----- events

    fn event_mut(&mut self, id: ObjectId) -> Result<(&mut EventType, &mut bool)> {
        match &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Event { kind, signaled } => Ok((kind, signaled)),
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    /// `KeSetEvent`: signal, satisfy waits, return the previous state.
    pub fn set_event(&mut self, id: ObjectId) -> Result<bool> {
        let (_, signaled) = self.event_mut(id)?;
        let previous = core::mem::replace(signaled, true);
        self.settle_and_flush(id);
        Ok(previous)
    }

    /// `KeResetEvent` / `NtResetEvent`: return the previous state.
    pub fn reset_event(&mut self, id: ObjectId) -> Result<bool> {
        let (_, signaled) = self.event_mut(id)?;
        Ok(core::mem::replace(signaled, false))
    }

    /// `KePulseEvent` / `NtPulseEvent`: see the module documentation.
    /// Returns the previous state.
    pub fn pulse_event(&mut self, id: ObjectId) -> Result<bool> {
        let previous = self.set_event(id)?;
        *self.event_mut(id)?.1 = false;
        Ok(previous)
    }

    pub fn event_info(&self, id: ObjectId) -> Result<EventInfo> {
        match &self.get(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Event { kind, signaled } => Ok(EventInfo { kind: *kind, signaled: *signaled }),
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    // ----- mutants

    /// `KeReleaseMutant` / `NtReleaseMutant` by `thread`. Returns the
    /// recursion count before the release. A thread that does not own it
    /// gets `STATUS_MUTANT_NOT_OWNED` (an abandoned mutant has no owner).
    /// What `NtReleaseMutant` reports as its `PreviousCount` is to be
    /// confirmed by the spec agent against real Windows.
    pub fn release_mutant(&mut self, id: ObjectId, thread: ThreadRef) -> Result<u32> {
        let Body::Mutant(m) = &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body else {
            return Err(STATUS_OBJECT_TYPE_MISMATCH);
        };
        if m.owner != Some(thread) {
            return Err(STATUS_MUTANT_NOT_OWNED);
        }
        let previous = m.count;
        m.count -= 1;
        if m.count == 0 {
            m.owner = None;
            self.settle_and_flush(id);
        }
        Ok(previous)
    }

    pub fn mutant_info(&self, id: ObjectId) -> Result<MutantInfo> {
        match &self.get(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Mutant(m) => Ok(MutantInfo { owner: m.owner, recursion: m.count, abandoned: m.abandoned }),
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_mutant_count(&mut self, id: ObjectId, count: u32) {
        if let Body::Mutant(m) = &mut self.obj_mut(id).body {
            m.count = count;
        }
    }

    // ----- semaphores

    /// `KeReleaseSemaphore` / `NtReleaseSemaphore`: add `count`, satisfy
    /// waits, return the previous count. `count` must be at least 1
    /// (`STATUS_INVALID_PARAMETER`, to be confirmed by the spec agent); a
    /// release past the maximum changes nothing and fails with
    /// `STATUS_SEMAPHORE_LIMIT_EXCEEDED` (Learn: "ReleaseSemaphore").
    pub fn release_semaphore(&mut self, id: ObjectId, release: i32) -> Result<i32> {
        let Body::Semaphore { count, maximum } = &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body else {
            return Err(STATUS_OBJECT_TYPE_MISMATCH);
        };
        if release <= 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let previous = *count;
        match previous.checked_add(release) {
            Some(new) if new <= *maximum => *count = new,
            _ => return Err(STATUS_SEMAPHORE_LIMIT_EXCEEDED),
        }
        self.settle_and_flush(id);
        Ok(previous)
    }

    pub fn semaphore_info(&self, id: ObjectId) -> Result<SemaphoreInfo> {
        match &self.get(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Semaphore { count, maximum } => Ok(SemaphoreInfo { count: *count, maximum: *maximum }),
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    // ----- timers (placeholder: the kernel owns the clock)

    /// `NtSetTimer` / `KeSetTimer`: arm the timer and make it not signaled
    /// (Learn: "KeSetTimer"). Returns the previous signal state. The kernel
    /// keeps the due time and period and calls [`Self::timer_expired`].
    pub fn set_timer(&mut self, id: ObjectId) -> Result<bool> {
        match &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Timer { signaled, armed, .. } => {
                *armed = true;
                Ok(core::mem::replace(signaled, false))
            }
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    /// `NtCancelTimer`: disarm without changing the signal state (Learn:
    /// "CancelWaitableTimer"). Returns the signal state.
    pub fn cancel_timer(&mut self, id: ObjectId) -> Result<bool> {
        match &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Timer { signaled, armed, .. } => {
                *armed = false;
                Ok(*signaled)
            }
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    /// The kernel's clock reached an armed timer's due time: signal it and
    /// satisfy waits. `periodic` keeps it armed for the next period.
    pub fn timer_expired(&mut self, id: ObjectId, periodic: bool) -> Result<()> {
        match &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Timer { signaled, armed, .. } => {
                *signaled = true;
                *armed = periodic;
            }
            _ => return Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
        self.settle_and_flush(id);
        Ok(())
    }

    /// A timer's kind and signal state, and whether it is armed.
    pub fn timer_info(&self, id: ObjectId) -> Result<(EventInfo, bool)> {
        match &self.get(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Timer { kind, signaled, armed } => Ok((EventInfo { kind: *kind, signaled: *signaled }, *armed)),
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }

    // ----- opaque waitable objects

    /// Set the signal state of a waitable opaque object (a process or
    /// thread that ended, a file whose I/O completed). Signaling satisfies
    /// every wait it can, like a notification event.
    pub fn set_signaled(&mut self, id: ObjectId, state: bool) -> Result<()> {
        match &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::Opaque { signaled: Some(s) } => *s = state,
            _ => return Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
        if state {
            self.settle_and_flush(id);
        }
        Ok(())
    }

    /// Whether `id` would satisfy a wait by `thread` right now (for tests
    /// and debugging).
    pub fn is_ready_for(&self, id: ObjectId, thread: ThreadRef) -> bool {
        self.get(id).is_some() && self.ready(id, thread) != Ready::No
    }
}
