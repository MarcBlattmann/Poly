//! Per-socket readiness notification (ADR 0048): the wait queues of the
//! sockets that are open files, woken when the stack reports that their
//! socket's readiness (or queued input) changed.
//!
//! A socket file registers its queue under its handle ([`watch`]). After
//! every poll of the stack (the network thread's, and the one each socket
//! call makes), with the interface locked, [`collect`] asks the stack
//! which sockets changed since the last look (`Stack::take_changed`);
//! once the lock is released, [`wake`] notifies those sockets' queues.
//! A thread waiting on one socket is therefore woken by changes of that
//! socket only, instead of by every change of every socket (the
//! readiness generation of ADR 0039, which the kernel's own blocking
//! sockets keep using).
//!
//! No change is missed: a waiter registers on its queue, then tries its
//! operation, which polls and collects under the lock; any change after
//! that is collected by a later poll and wakes it.

use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_net::{SocketHandle, Stack};
use poly_vfs::{PollEvents, WaitQueue};

static WATCHED: SpinLock<BTreeMap<SocketHandle, Weak<WaitQueue>>> = SpinLock::new(BTreeMap::new());
/// Sockets watched; nothing is collected while 0.
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Notify `queue` whenever socket `h` may have changed, until [`unwatch`].
pub fn watch(h: SocketHandle, queue: &Arc<WaitQueue>) {
    if WATCHED.lock().insert(h, Arc::downgrade(queue)).is_none() {
        COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

/// Stop notifying for socket `h`.
pub fn unwatch(h: SocketHandle) {
    let removed = WATCHED.lock().remove(&h);
    if removed.is_some() {
        COUNT.fetch_sub(1, Ordering::SeqCst);
    }
    // The weak reference goes with no lock held.
    drop(removed);
}

/// The sockets of `stack` that changed since the last call; empty while
/// nobody watches. Called with the interface locked.
pub(super) fn collect(stack: &mut Stack) -> Vec<SocketHandle> {
    let mut changed = Vec::new();
    if COUNT.load(Ordering::SeqCst) > 0 {
        stack.take_changed(&mut changed);
    }
    changed
}

/// Wake the queues of `changed` (with the interface unlocked: wakers may
/// take locks of their own).
pub(super) fn wake(changed: Vec<SocketHandle>) {
    if changed.is_empty() {
        return;
    }
    let queues: Vec<Arc<WaitQueue>> = {
        let w = WATCHED.lock();
        changed.iter().filter_map(|h| w.get(h).and_then(Weak::upgrade)).collect()
    };
    for q in queues {
        q.notify(PollEvents::ALL);
    }
}

/// Sockets watched (for tests).
pub fn watched() -> usize {
    COUNT.load(Ordering::SeqCst)
}
