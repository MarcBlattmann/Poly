//! Collecting descriptors in flight that nobody can receive any more.
//!
//! A socket's open file stays alive while a message in some receive queue
//! holds it. When socket A is sent over socket B and B over A (or a socket
//! over itself) and the processes then close their descriptors, each
//! socket keeps the other's file alive from its queue: neither file is
//! ever dropped, so neither socket is ever closed. unix(7) counts such
//! descriptors as "in flight"; nothing can receive them.
//!
//! [`Namespace::collect`] finds them as a mark phase over the sockets that
//! hold descriptors:
//!
//! 1. For every open file in a queue, count the queue references to it.
//!    A Unix socket's file whose every reference (`Arc` strong count) is
//!    such a queue reference is a *candidate*: no process holds it.
//! 2. A candidate is *reachable* if a queue of a socket that is not a
//!    candidate's (one a process can still receive from) holds it, or a
//!    queue of a reachable candidate does.
//! 3. The queues of unreachable candidates are purged: their messages
//!    and the open files in them are dropped, which closes the sockets of
//!    the cycle.
//!
//! References taken while the scan runs (a system call holding a file)
//! only make a file look held, so the collection errs on the side of
//! keeping things. The kernel calls it after a close while anything is in
//! flight.

use crate::{Namespace, Packet, UnixFile, UnixSocket};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

#[derive(Default)]
struct FileRefs {
    in_queues: usize,
    strong: usize,
    /// The socket of a Unix socket's file.
    socket: Option<usize>,
}

impl Namespace {
    /// Purge the queues of sockets only reachable through other such
    /// queues (see the module documentation); returns how many open files
    /// were released. Cheap when nothing is in flight.
    pub fn collect(&self) -> usize {
        if self.in_flight() == 0 {
            return 0;
        }
        let one = self.collecting.lock();
        let holders: Vec<Arc<UnixSocket>> = {
            let mut h = self.holding.lock();
            h.retain(|w| w.strong_count() > 0);
            h.iter().filter_map(alloc::sync::Weak::upgrade).collect()
        };
        let mut files: BTreeMap<usize, FileRefs> = BTreeMap::new();
        let mut edges: Vec<Vec<usize>> = Vec::with_capacity(holders.len());
        for h in &holders {
            let mut out = Vec::new();
            let mut s = h.state.lock();
            for f in s.rx.iter().flat_map(|p| p.rights.iter()) {
                let key = Arc::as_ptr(f) as usize;
                let r = files.entry(key).or_default();
                r.in_queues += 1;
                r.strong = Arc::strong_count(f);
                if r.socket.is_none() {
                    r.socket = f.object_as::<UnixFile>().map(|u| Arc::as_ptr(&u.sock) as usize);
                }
                out.push(key);
            }
            if out.is_empty() {
                s.holds_rights = false;
            }
            drop(s);
            edges.push(out);
        }
        // Holders by socket address.
        let index: BTreeMap<usize, usize> =
            holders.iter().enumerate().map(|(i, h)| (Arc::as_ptr(h) as usize, i)).collect();
        let candidate = |key: &usize| files.get(key).is_some_and(|r| r.socket.is_some() && r.strong == r.in_queues);
        let candidate_sockets: BTreeMap<usize, usize> =
            files.iter().filter(|(k, _)| candidate(k)).filter_map(|(k, r)| r.socket.map(|s| (s, *k))).collect();
        // Mark from every holder a process can still reach.
        let mut live: BTreeMap<usize, ()> = BTreeMap::new();
        let mut work: VecDeque<usize> = VecDeque::new();
        for (i, h) in holders.iter().enumerate() {
            if !candidate_sockets.contains_key(&(Arc::as_ptr(h) as usize)) {
                for k in edges[i].iter().filter(|k| candidate(k)) {
                    if live.insert(*k, ()).is_none() {
                        work.push_back(*k);
                    }
                }
            }
        }
        while let Some(k) = work.pop_front() {
            let Some(sock) = files.get(&k).and_then(|r| r.socket) else { continue };
            let Some(&i) = index.get(&sock) else { continue };
            for k2 in edges[i].iter().filter(|k| candidate(k)) {
                if live.insert(*k2, ()).is_none() {
                    work.push_back(*k2);
                }
            }
        }
        // Purge the rest.
        let mut purged: Vec<Packet> = Vec::new();
        for (sock, key) in &candidate_sockets {
            if live.contains_key(key) {
                continue;
            }
            let Some(&i) = index.get(sock) else { continue };
            let mut s = holders[i].state.lock();
            s.rx_bytes = 0;
            purged.extend(s.rx.drain(..));
            s.holds_rights = false;
        }
        let released: usize = purged.iter().map(|p| p.rights.len()).sum();
        self.in_flight.fetch_sub(released, Ordering::SeqCst);
        // Forget holders without descriptors (they come back on the next
        // send with descriptors).
        // (Checked with the list unlocked: a socket whose last reference
        // goes here is dropped, with its queue, outside every lock.)
        let listed = core::mem::take(&mut *self.holding.lock());
        let kept: alloc::vec::Vec<_> =
            listed.into_iter().filter(|w| w.upgrade().is_some_and(|s| s.state.lock().holds_rights)).collect();
        self.holding.lock().extend(kept);
        drop(one);
        drop(holders);
        // No lock held: dropping the files closes the sockets of the cycle.
        drop(purged);
        released
    }
}
