//! TLB invalidation across CPUs.
//!
//! A CPU caches translations in its TLB and paging-structure caches, and
//! nothing keeps another CPU's caches coherent with a page-table change:
//! "the processor does not maintain coherence between the TLBs of different
//! processors" (SDM Vol. 3A §4.10.5). When an entry is made less permissive
//! (unmapped, made read-only, given another frame) every CPU that may hold
//! the old translation must invalidate it before the change can be relied
//! on, in particular before the old frame is reused. The kernel sends those
//! CPUs an IPI (a *TLB shootdown*) and waits for each to acknowledge.
//!
//! - [`Flush`] batches what one operation changed into one range or, past
//!   [`FULL_FLUSH_PAGES`] pages, a flush of everything, so unmapping a
//!   large range costs one shootdown rather than one per page.
//! - [`Mailboxes`] is the protocol (ADR 0254): a mailbox per CPU, any number
//!   of requests in flight. A sender posts a request to each target and
//!   interrupts it, and gets a [`Ticket`]; it waits for the ticket only when
//!   it must. A target empties its mailbox and counts what it took as done.
//!   Whoever waits keeps serving its own mailbox, so two CPUs waiting for each
//!   other cannot deadlock even with interrupts off.

use crate::CpuSet;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Pages above which a [`Flush`] becomes [`Flush::All`].
pub const FULL_FLUSH_PAGES: u64 = 32;

const PAGE: u64 = 4096;

/// Translations to invalidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flush {
    /// Nothing changed.
    #[default]
    None,
    /// The pages of `[start, end)` (page-aligned).
    Range { start: u64, end: u64 },
    /// Everything (of one address space, or every non-global translation).
    All,
}

impl Flush {
    /// Add the page holding `addr`.
    pub fn add_page(&mut self, addr: u64) {
        let page = addr & !(PAGE - 1);
        self.add_range(page, page.saturating_add(PAGE));
    }

    /// Add `[start, end)`, rounded out to whole pages. The result covers
    /// both ranges (and what lies between them); past
    /// [`FULL_FLUSH_PAGES`] pages it becomes [`Flush::All`].
    pub fn add_range(&mut self, start: u64, end: u64) {
        if end <= start {
            return;
        }
        let (start, end) = (start & !(PAGE - 1), end.saturating_add(PAGE - 1) & !(PAGE - 1));
        let (start, end) = match *self {
            Flush::All => return,
            Flush::None => (start, end),
            Flush::Range { start: s, end: e } => (s.min(start), e.max(end)),
        };
        *self = if (end - start) / PAGE > FULL_FLUSH_PAGES { Flush::All } else { Flush::Range { start, end } };
    }

    /// Both together.
    pub fn merge(&mut self, other: Flush) {
        match other {
            Flush::None => {}
            Flush::All => *self = Flush::All,
            Flush::Range { start, end } => self.add_range(start, end),
        }
    }

    pub fn is_none(&self) -> bool {
        *self == Flush::None
    }

    /// The pages to invalidate one by one, or `None` for [`Flush::All`].
    pub fn pages(&self) -> Option<impl Iterator<Item = u64>> {
        let (start, end) = match *self {
            Flush::None => (0, 0),
            Flush::Range { start, end } => (start, end),
            Flush::All => return None,
        };
        Some((start..end).step_by(PAGE as usize))
    }
}

/// One shootdown: what to invalidate, and in which address space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// The address space (the kernel names it, e.g. by its root table);
    /// a target that does not have it loaded has nothing to do. 0 means
    /// kernel mappings, which every CPU has.
    pub space: u64,
    pub flush: Flush,
}

/// Requests a mailbox holds before further ones become one flush of everything.
pub const MAILBOX: usize = 16;

/// What a mailbox holds: the requests not taken out yet, how many, and whether more came than fit.
type Items = ([Request; MAILBOX], usize, bool);

/// One CPU's mailbox.
#[derive(Debug)]
struct Mailbox {
    /// Held while the box is changed (briefly: a copy in or out).
    lock: AtomicBool,
    /// Requests ever posted to this CPU; a request's sequence number is the count after it.
    posted: AtomicU64,
    /// Every request numbered up to this one was carried out.
    done: AtomicU64,
    /// An interrupt is on its way to this CPU: a sender that finds it set does not send another
    /// (the target clears it before it empties the box, so nothing posted is missed).
    interrupted: AtomicBool,
    /// Under `lock`.
    items: UnsafeCell<Items>,
}

/// A request posted to several CPUs: for each target, the sequence number it got there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket<const C: usize> {
    targets: CpuSet,
    seq: [u64; C],
}

impl<const C: usize> Ticket<C> {
    /// The CPUs it was posted to.
    pub fn targets(&self) -> CpuSet {
        self.targets
    }
}

/// The shootdown mailboxes of up to `C` CPUs (ADR 0254): any number of requests in flight. A
/// sender posts a request to each target's mailbox and interrupts it; it waits for the targets
/// only when it must ([`Mailboxes::wait`]). A target takes everything out of its mailbox at once
/// ([`Mailboxes::service`]) and then counts it done, so a [`Ticket`] is complete once every target
/// has counted past the number its request got there.
#[derive(Debug)]
pub struct Mailboxes<const C: usize> {
    boxes: [Mailbox; C],
    /// Requests posted and mailboxes emptied, for statistics.
    sent: AtomicU64,
    served: AtomicU64,
}

// SAFETY: a mailbox's `items` is only touched by whoever holds its `lock` (Acquire / Release).
unsafe impl<const C: usize> Sync for Mailboxes<C> {}

impl<const C: usize> Default for Mailboxes<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl Mailbox {
    const fn new() -> Mailbox {
        Mailbox {
            lock: AtomicBool::new(false),
            posted: AtomicU64::new(0),
            done: AtomicU64::new(0),
            interrupted: AtomicBool::new(false),
            items: UnsafeCell::new(([Request { space: 0, flush: Flush::None }; MAILBOX], 0, false)),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Items) -> R) -> R {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is ours until it is released below.
        let r = f(unsafe { &mut *self.items.get() });
        self.lock.store(false, Ordering::Release);
        r
    }
}

impl<const C: usize> Mailboxes<C> {
    pub const fn new() -> Self {
        Mailboxes { boxes: [const { Mailbox::new() }; C], sent: AtomicU64::new(0), served: AtomicU64::new(0) }
    }

    /// Post `req` to every CPU in `targets` other than `me`, interrupt them with `notify(targets)`
    /// and return without waiting: `None` when there is nobody to tell or nothing to do.
    pub fn post(&self, me: usize, targets: CpuSet, req: Request, notify: impl FnOnce(CpuSet)) -> Option<Ticket<C>> {
        let t = self.post_quiet(me, targets, req)?;
        self.hurry(&t, notify);
        Some(t)
    }

    /// Post `req` like [`post`](Self::post) without interrupting anyone: the targets carry it out
    /// when they next look at their mailboxes (the kernel's timer tick, a context switch), together
    /// with whatever else came meanwhile, or sooner if somebody [`hurry`](Self::hurry)s them.
    pub fn post_quiet(&self, me: usize, targets: CpuSet, req: Request) -> Option<Ticket<C>> {
        let mut targets = targets.intersection(CpuSet::first_n(C));
        targets.remove(me);
        if targets.is_empty() || req.flush.is_none() {
            return None;
        }
        let mut seq = [0u64; C];
        for cpu in targets.iter() {
            let b = &self.boxes[cpu];
            seq[cpu] = b.with(|(items, n, overflow)| {
                if *n < MAILBOX {
                    items[*n] = req;
                    *n += 1;
                } else {
                    *overflow = true;
                }
                // Numbered under the lock, in the order the box holds them.
                b.posted.fetch_add(1, Ordering::AcqRel) + 1
            });
        }
        self.sent.fetch_add(1, Ordering::Relaxed);
        Some(Ticket { targets, seq })
    }

    /// Interrupt, with `notify`, the targets of `t` that have not carried it out yet and that no
    /// interrupt is on its way to already: requests posted in a burst are carried out together,
    /// after one interrupt.
    pub fn hurry(&self, t: &Ticket<C>, notify: impl FnOnce(CpuSet)) {
        let mut wake = CpuSet::default();
        for cpu in t.targets.iter() {
            let b = &self.boxes[cpu];
            if b.done.load(Ordering::Acquire) < t.seq[cpu] && !b.interrupted.swap(true, Ordering::SeqCst) {
                wake.insert(cpu);
            }
        }
        if !wake.is_empty() {
            notify(wake);
        }
    }

    /// Carry out what waits in `me`'s mailbox: `apply` each request, or `apply_all` once instead
    /// when more came than the box holds. True if there was anything.
    pub fn service(&self, me: usize, mut apply: impl FnMut(&Request), apply_all: impl FnOnce()) -> bool {
        let Some(b) = self.boxes.get(me) else { return false };
        // Before looking: a request posted from now on interrupts again.
        b.interrupted.store(false, Ordering::SeqCst);
        if b.done.load(Ordering::Acquire) == b.posted.load(Ordering::Acquire) {
            return false;
        }
        let (taken, n, overflow, upto) = b.with(|(items, n, overflow)| {
            let r = (*items, *n, *overflow, b.posted.load(Ordering::Acquire));
            *n = 0;
            *overflow = false;
            r
        });
        if overflow {
            apply_all();
        } else {
            for r in &taken[..n] {
                apply(r);
            }
        }
        // Everything numbered up to `upto` was in the box when it was emptied.
        b.done.fetch_max(upto, Ordering::AcqRel);
        self.served.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Whether every target of `t` has carried its request out.
    pub fn complete(&self, t: &Ticket<C>) -> bool {
        t.targets.iter().all(|cpu| self.boxes[cpu].done.load(Ordering::Acquire) >= t.seq[cpu])
    }

    /// Return once `t` is complete. `wait()` runs on every turn and must serve the caller's own
    /// mailbox, so two CPUs waiting for each other both make progress.
    pub fn wait(&self, t: &Ticket<C>, mut wait: impl FnMut()) {
        while !self.complete(t) {
            wait();
            core::hint::spin_loop();
        }
    }

    /// Whether requests wait for `cpu`.
    pub fn is_pending(&self, cpu: usize) -> bool {
        self.boxes.get(cpu).is_some_and(|b| b.done.load(Ordering::Acquire) != b.posted.load(Ordering::Acquire))
    }

    /// Requests posted and mailboxes emptied so far.
    pub fn stats(&self) -> (u64, u64) {
        (self.sent.load(Ordering::Relaxed), self.served.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::vec::Vec;

    #[test]
    fn flushes_batch_into_one_range_or_everything() {
        let mut f = Flush::None;
        assert!(f.is_none());
        assert_eq!(f.pages().unwrap().count(), 0);
        f.add_page(0x40_0123);
        assert_eq!(f, Flush::Range { start: 0x40_0000, end: 0x40_1000 });
        f.add_page(0x40_5000);
        assert_eq!(f, Flush::Range { start: 0x40_0000, end: 0x40_6000 });
        assert_eq!(f.pages().unwrap().collect::<Vec<_>>().len(), 6);
        f.add_range(0x3f_f800, 0x40_0001);
        assert_eq!(f, Flush::Range { start: 0x3f_f000, end: 0x40_6000 }, "rounded out to pages");
        f.add_range(5, 5);
        assert_eq!(f, Flush::Range { start: 0x3f_f000, end: 0x40_6000 }, "an empty range adds nothing");
        f.add_range(0x40_0000, 0x40_0000 + (FULL_FLUSH_PAGES + 1) * PAGE);
        assert_eq!(f, Flush::All);
        assert!(f.pages().is_none());
        f.add_page(0);
        assert_eq!(f, Flush::All);
        let mut g = Flush::None;
        g.merge(Flush::Range { start: 0x1000, end: 0x2000 });
        g.merge(Flush::None);
        assert_eq!(g, Flush::Range { start: 0x1000, end: 0x2000 });
        g.merge(Flush::All);
        assert_eq!(g, Flush::All);
    }

    #[test]
    fn nothing_is_sent_to_nobody() {
        let s = Mailboxes::<4>::new();
        let req = Request { space: 1, flush: Flush::All };
        assert!(s.post(0, CpuSet::one(0), req, |_| panic!("only self")).is_none());
        assert!(s.post(0, CpuSet::one(1), Request { space: 1, flush: Flush::None }, |_| panic!("nothing")).is_none());
        assert_eq!(s.stats(), (0, 0));
        assert!(!s.service(9, |_| {}, || {}), "out of range");
    }

    #[test]
    fn a_request_reaches_each_target_once_and_completes_when_all_have_it() {
        let s = Mailboxes::<4>::new();
        let req = Request { space: 7, flush: Flush::Range { start: 0, end: 0x1000 } };
        let t = s.post(0, CpuSet::from_bits(0b1110), req, |t| assert_eq!(t, CpuSet::from_bits(0b1110))).unwrap();
        assert!(!s.complete(&t));
        let mut seen = Vec::new();
        for cpu in 1..3 {
            assert!(s.service(cpu, |r| seen.push((cpu, *r)), || panic!("no overflow")));
        }
        assert!(!s.complete(&t), "CPU 3 has not");
        assert!(s.is_pending(3) && !s.is_pending(1));
        s.service(3, |r| seen.push((3, *r)), || {});
        assert!(s.complete(&t));
        assert_eq!(seen, [(1, req), (2, req), (3, req)]);
        assert!(!s.service(1, |_| panic!("served twice"), || {}));
    }

    #[test]
    fn many_in_flight_and_a_full_mailbox_becomes_one_flush_of_everything() {
        let s = Mailboxes::<2>::new();
        let mut sent = 0;
        let tickets: Vec<_> = (0..MAILBOX as u64 + 5)
            .map(|i| s.post(0, CpuSet::one(1), Request { space: i + 1, flush: Flush::All }, |_| sent += 1).unwrap())
            .collect();
        assert_eq!(sent, 1, "one interrupt for a burst");
        assert!(tickets.iter().all(|t| !s.complete(t)));
        let (mut each, mut all) = (0, 0);
        s.service(1, |_| each += 1, || all += 1);
        assert_eq!((each, all), (0, 1), "overflowed: one full flush");
        assert!(tickets.iter().all(|t| s.complete(t)));
        let mut again = false;
        let t = s.post(0, CpuSet::one(1), Request { space: 9, flush: Flush::All }, |_| again = true).unwrap();
        assert!(again, "after the box was emptied, a new request interrupts again");
        s.service(1, |r| assert_eq!(r.space, 9), || panic!("not full"));
        assert!(s.complete(&t));
    }

    #[test]
    fn a_quiet_request_interrupts_nobody_until_hurried_and_then_only_who_has_not_done_it() {
        let s = Mailboxes::<4>::new();
        let t = s.post_quiet(0, CpuSet::from_bits(0b1110), Request { space: 3, flush: Flush::All }).unwrap();
        s.service(1, |_| {}, || {});
        let mut woken = CpuSet::default();
        s.hurry(&t, |w| woken = w);
        assert_eq!(woken, CpuSet::from_bits(0b1100), "CPU 1 has it done");
        let mut again = false;
        s.hurry(&t, |_| again = true);
        assert!(!again, "an interrupt is on its way already");
        s.service(2, |_| {}, || {});
        s.service(3, |_| {}, || {});
        assert!(s.complete(&t));
    }

    #[test]
    fn a_later_request_does_not_complete_an_earlier_one_still_in_the_box() {
        let s = Mailboxes::<3>::new();
        let a = s.post(1, CpuSet::one(0), Request { space: 1, flush: Flush::All }, |_| {}).unwrap();
        let b = s.post(2, CpuSet::one(0), Request { space: 2, flush: Flush::All }, |_| {}).unwrap();
        assert!(!s.complete(&a) && !s.complete(&b));
        s.service(0, |_| {}, || {});
        assert!(s.complete(&a) && s.complete(&b));
    }

    /// Several threads stand in for CPUs: each posts to random others, keeps a few tickets in
    /// flight and waits for them while serving its own mailbox, as the kernel's waits do. A
    /// completed ticket means every target applied the request; nobody may deadlock.
    #[test]
    fn concurrent_shootdowns_complete_and_are_seen_by_every_target() {
        const CPUS: usize = 4;
        const ROUNDS: u64 = 300;
        let s = Arc::new(Mailboxes::<CPUS>::new());
        let applied: Arc<[[AtomicU64; CPUS]; CPUS]> =
            Arc::new(core::array::from_fn(|_| core::array::from_fn(|_| AtomicU64::new(0))));
        let done = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..CPUS)
            .map(|me| {
                let (s, applied, done) = (Arc::clone(&s), Arc::clone(&applied), Arc::clone(&done));
                std::thread::spawn(move || {
                    let serve = |s: &Mailboxes<CPUS>| {
                        s.service(
                            me,
                            |r| {
                                let (shooter, id) = ((r.space >> 32) as usize, r.space & 0xffff_ffff);
                                applied[me][shooter].fetch_max(id, Ordering::SeqCst);
                            },
                            || {
                                // Everything: every request posted so far counts as applied.
                                for shooter in 0..CPUS {
                                    applied[me][shooter].store(u64::MAX >> 1, Ordering::SeqCst);
                                }
                            },
                        );
                    };
                    let mut rng = 0x9e37_79b9_7f4a_7c15u64 ^ me as u64;
                    let mut out = std::collections::VecDeque::new();
                    for id in 1..=ROUNDS {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        let targets = CpuSet::from_bits(rng & 0xf).without(CpuSet::one(me));
                        let req = Request { space: (me as u64) << 32 | id, flush: Flush::All };
                        if let Some(t) = s.post(me, targets, req, |_| {}) {
                            out.push_back((t, id));
                        }
                        while out.len() > 3 || (id == ROUNDS && !out.is_empty()) {
                            let (t, id) = out.pop_front().unwrap();
                            s.wait(&t, || {
                                serve(&s);
                                std::thread::yield_now();
                            });
                            for c in t.targets().iter() {
                                assert!(applied[c][me].load(Ordering::SeqCst) >= id, "CPU {c} missed {id} of {me}");
                            }
                        }
                    }
                    done.fetch_add(1, Ordering::SeqCst);
                    while done.load(Ordering::SeqCst) < CPUS {
                        serve(&s);
                        std::thread::yield_now();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let (sent, served) = s.stats();
        assert!(sent > 0 && served > 0);
    }
}
