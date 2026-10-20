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
//! - [`Shootdowns`] is the mailbox protocol: one request at a time (a lock),
//!   one pending flag per target CPU, set before the IPI and cleared by the
//!   target after it has invalidated. Whoever waits, for the lock or for the
//!   acknowledgements, keeps serving its own mailbox, so two CPUs shooting at
//!   each other (or one shooting while another waits for the lock) cannot
//!   deadlock even with interrupts off.

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

/// The shootdown mailboxes of up to `C` CPUs.
#[derive(Debug)]
pub struct Shootdowns<const C: usize> {
    /// Held by the CPU whose request is out.
    busy: AtomicBool,
    request: UnsafeCell<Request>,
    pending: [AtomicBool; C],
    /// Requests sent and acknowledgements received, for statistics.
    sent: AtomicU64,
    served: AtomicU64,
}

// SAFETY: `request` is written only by the holder of `busy`, and only while
// no target has its pending flag set; targets read it only between seeing
// their flag set (Acquire) and clearing it (Release).
unsafe impl<const C: usize> Sync for Shootdowns<C> {}

impl<const C: usize> Default for Shootdowns<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const C: usize> Shootdowns<C> {
    pub const fn new() -> Self {
        Shootdowns {
            busy: AtomicBool::new(false),
            request: UnsafeCell::new(Request { space: 0, flush: Flush::None }),
            pending: [const { AtomicBool::new(false) }; C],
            sent: AtomicU64::new(0),
            served: AtomicU64::new(0),
        }
    }

    /// Have every CPU in `targets` (other than `me`) carry out `req`, and
    /// return once each has. `notify(targets)` interrupts them (an IPI);
    /// `wait()` is called on every turn of the waiting loops and must serve
    /// `me`'s own mailbox ([`service`](Self::service)) so that a CPU
    /// shooting at `me` meanwhile is not left waiting.
    pub fn shoot(&self, me: usize, targets: CpuSet, req: Request, notify: impl FnOnce(CpuSet), mut wait: impl FnMut()) {
        let mut targets = targets.intersection(CpuSet::first_n(C));
        targets.remove(me);
        if targets.is_empty() || req.flush.is_none() {
            return;
        }
        while self.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            wait();
            core::hint::spin_loop();
        }
        // SAFETY: we hold `busy`, and every pending flag is clear (the
        // previous holder waited for all of its targets), so no target
        // reads the request now.
        unsafe { *self.request.get() = req };
        for cpu in targets.iter() {
            self.pending[cpu].store(true, Ordering::Release);
        }
        self.sent.fetch_add(1, Ordering::Relaxed);
        notify(targets);
        for cpu in targets.iter() {
            while self.pending[cpu].load(Ordering::Acquire) {
                wait();
                core::hint::spin_loop();
            }
        }
        self.busy.store(false, Ordering::Release);
    }

    /// If a request is waiting for `me`, carry it out with `apply` and
    /// acknowledge it. True if there was one.
    pub fn service(&self, me: usize, apply: impl FnOnce(&Request)) -> bool {
        let Some(flag) = self.pending.get(me) else { return false };
        if !flag.load(Ordering::Acquire) {
            return false;
        }
        // SAFETY: our flag is set, so the request is complete and stays
        // unchanged until we clear the flag.
        apply(unsafe { &*self.request.get() });
        self.served.fetch_add(1, Ordering::Relaxed);
        flag.store(false, Ordering::Release);
        true
    }

    /// Whether a request waits for `cpu`.
    pub fn is_pending(&self, cpu: usize) -> bool {
        self.pending.get(cpu).is_some_and(|f| f.load(Ordering::Acquire))
    }

    /// Shootdowns sent and requests carried out by targets so far.
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
        let s = Shootdowns::<4>::new();
        let req = Request { space: 1, flush: Flush::All };
        s.shoot(0, CpuSet::one(0), req, |_| panic!("only self"), || {});
        s.shoot(0, CpuSet::one(1), Request { space: 1, flush: Flush::None }, |_| panic!("nothing to do"), || {});
        assert_eq!(s.stats(), (0, 0));
        assert!(!s.service(9, |_| {}), "out of range");
    }

    #[test]
    fn a_request_reaches_each_target_once() {
        let s = Shootdowns::<4>::new();
        let req = Request { space: 7, flush: Flush::Range { start: 0, end: 0x1000 } };
        let mut seen = Vec::new();
        // Targets serve from inside `wait`, standing in for other CPUs.
        s.shoot(
            0,
            CpuSet::from_bits(0b1110),
            req,
            |t| assert_eq!(t, CpuSet::from_bits(0b1110)),
            || {
                for cpu in 1..4 {
                    s.service(cpu, |r| seen.push((cpu, *r)));
                }
            },
        );
        assert_eq!(seen, [(1, req), (2, req), (3, req)]);
        assert!(!s.is_pending(1));
        assert_eq!(s.stats(), (1, 3));
    }

    /// Several threads stand in for CPUs: each shoots at random others
    /// while serving its own mailbox, as the kernel's waits do. Every
    /// request must be applied by every target before `shoot` returns, and
    /// nobody may deadlock.
    #[test]
    fn concurrent_shootdowns_complete_and_are_seen_by_every_target() {
        const CPUS: usize = 4;
        const ROUNDS: u64 = 300;
        let s = Arc::new(Shootdowns::<CPUS>::new());
        // What each CPU applied last, per shooter: request ids grow.
        let applied: Arc<[[AtomicU64; CPUS]; CPUS]> =
            Arc::new(core::array::from_fn(|_| core::array::from_fn(|_| AtomicU64::new(0))));
        let done = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..CPUS)
            .map(|me| {
                let (s, applied, done) = (Arc::clone(&s), Arc::clone(&applied), Arc::clone(&done));
                std::thread::spawn(move || {
                    let serve = |s: &Shootdowns<CPUS>| {
                        s.service(me, |r| {
                            let (shooter, id) = ((r.space >> 32) as usize, r.space & 0xffff_ffff);
                            applied[me][shooter].store(id, Ordering::SeqCst);
                        });
                    };
                    let mut rng = 0x9e37_79b9_7f4a_7c15u64 ^ me as u64;
                    for id in 1..=ROUNDS {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        let targets = CpuSet::from_bits(rng & 0xf).without(CpuSet::one(me));
                        let req = Request { space: (me as u64) << 32 | id, flush: Flush::All };
                        // Yielding while waiting keeps the test quick on a
                        // host with fewer cores than threads.
                        s.shoot(
                            me,
                            targets,
                            req,
                            |_| {},
                            || {
                                serve(&s);
                                std::thread::yield_now();
                            },
                        );
                        for t in targets.iter() {
                            assert_eq!(
                                applied[t][me].load(Ordering::SeqCst),
                                id,
                                "CPU {t} missed request {id} of {me}"
                            );
                        }
                    }
                    done.fetch_add(1, Ordering::SeqCst);
                    // Keep serving until everyone is done.
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
        assert!(sent > 0 && served >= sent);
    }
}
