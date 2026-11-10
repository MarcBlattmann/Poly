//! Critical sections: `RTL_CRITICAL_SECTION` (`winnt.h`) and
//! `RtlInitializeCriticalSection*`, `RtlEnterCriticalSection`,
//! `RtlTryEnterCriticalSection`, `RtlLeaveCriticalSection`,
//! `RtlDeleteCriticalSection`, as Learn documents their Win32 counterparts
//! ("Critical Section Objects", `EnterCriticalSection`, ...): a lock one
//! thread owns at a time, recursively (each enter by the owner needs its
//! own leave), that a waiting thread may spin on before it blocks.
//!
//! # Fields (`winnt.h` layout, 40 bytes on x64)
//!
//! | Offset | Field | Poly's use |
//! |---|---|---|
//! | 0x00 | `DebugInfo` | 0 (no debug information is kept) |
//! | 0x08 | `LockCount` | -1 when free, -2 held with nothing waiting (observed on Windows 11 for the loader lock, spec-chrome-elf-null); each further enter by the owner or a waiter lowers it by one |
//! | 0x0C | `RecursionCount` | how often the owner entered |
//! | 0x10 | `OwningThread` | the owner's thread id (`CLIENT_ID.UniqueThread`), 0 when free |
//! | 0x18 | `LockSemaphore` | the auto-reset event waiters block on, created when first needed |
//! | 0x20 | `SpinCount` | spin iterations before blocking (low 24 bits); the top byte holds the `RTL_CRITICAL_SECTION_FLAG_*` bits |
//!
//! `LockCount` counts downwards from -1: each would-be owner decrements
//! it; the one that moved it from -1 to -2 owns the section; a leave that
//! finds others counted hands ownership to exactly one waiter by setting
//! the event. The two values programs can observe (free = -1, held once
//! = -2) are those of a Windows 11 process (spec-chrome-elf-null); the
//! values with waiters are Poly's own continuation of the scheme.

use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};

/// `RTL_CRITICAL_SECTION_ALL_FLAG_BITS` (`winnt.h`): the top byte of
/// `SpinCount`.
pub const FLAG_BITS: u64 = 0xFF00_0000;

/// `RTL_CRITICAL_SECTION_FLAG_STATIC_INIT` (`winnt.h`): the section was not made by
/// `RtlInitializeCriticalSection` but is part of an image's data (the loader lock is one).
pub const FLAG_STATIC_INIT: u64 = 0x0400_0000;

/// `LockCount` of a free section.
pub const FREE: i32 = -1;
/// `LockCount` of a section its owner holds once, with no waiter.
pub const HELD: i32 = -2;

/// What a critical section needs from the system: the thread id, and an
/// auto-reset event to block on (in `ntdll`: `NtCreateEvent`
/// (`SynchronizationEvent`), `NtWaitForSingleObject`, `NtSetEvent`,
/// `NtClose`).
pub trait Waiter {
    /// The calling thread's id (never 0).
    fn current_thread(&self) -> u64;
    /// A new auto-reset event, not signalled; 0 if none could be made.
    fn create_event(&self) -> u64;
    fn wait(&self, event: u64);
    fn set(&self, event: u64);
    fn close(&self, event: u64);
    /// Give the processor away (used while an event cannot be made).
    fn yield_now(&self) {}
}

/// `RTL_CRITICAL_SECTION`.
#[repr(C)]
#[derive(Debug)]
pub struct CriticalSection {
    pub debug_info: AtomicU64,
    pub lock_count: AtomicI32,
    pub recursion_count: AtomicI32,
    pub owning_thread: AtomicU64,
    pub lock_semaphore: AtomicU64,
    pub spin_count: AtomicU64,
}

/// `RTL_CRITICAL_SECTION_DEBUG` (`winnt.h`; 48 bytes on x64): what `DebugInfo` points to.
/// Poly keeps one only for sections that programs expect to have it (the loader lock, whose
/// `DebugInfo` is non-NULL in a Windows process, spec-chrome-elf-null item 4).
#[repr(C)]
#[derive(Debug)]
pub struct CriticalSectionDebug {
    /// `Type`: `RTL_CRITSECT_TYPE` (0).
    pub kind: u16,
    pub creator_back_trace_index: u16,
    /// `CriticalSection`: back to the section.
    pub critical_section: AtomicU64,
    /// `ProcessLocksList` (`LIST_ENTRY`: `Flink`, `Blink`); an empty list points at itself.
    pub process_locks_list: [AtomicU64; 2],
    pub entry_count: u32,
    pub contention_count: u32,
    pub flags: u32,
    pub creator_back_trace_index_high: u16,
    pub identifier: u16,
}

impl Default for CriticalSectionDebug {
    fn default() -> Self {
        CriticalSectionDebug::new()
    }
}

impl CriticalSectionDebug {
    pub const fn new() -> CriticalSectionDebug {
        CriticalSectionDebug {
            kind: 0,
            creator_back_trace_index: 0,
            critical_section: AtomicU64::new(0),
            process_locks_list: [AtomicU64::new(0), AtomicU64::new(0)],
            entry_count: 0,
            contention_count: 0,
            flags: 0,
            creator_back_trace_index_high: 0,
            identifier: 0,
        }
    }
}

impl Default for CriticalSection {
    fn default() -> Self {
        CriticalSection::new()
    }
}

impl CriticalSection {
    /// A free critical section with no spinning.
    pub const fn new() -> CriticalSection {
        CriticalSection {
            debug_info: AtomicU64::new(0),
            lock_count: AtomicI32::new(FREE),
            recursion_count: AtomicI32::new(0),
            owning_thread: AtomicU64::new(0),
            lock_semaphore: AtomicU64::new(0),
            spin_count: AtomicU64::new(0),
        }
    }

    /// `RtlInitializeCriticalSectionAndSpinCount`: reset to free with this
    /// spin count (flag bits kept in the top byte).
    pub fn init(&self, spin_count: u64) {
        self.debug_info.store(0, Ordering::Relaxed);
        self.lock_count.store(FREE, Ordering::Relaxed);
        self.recursion_count.store(0, Ordering::Relaxed);
        self.owning_thread.store(0, Ordering::Relaxed);
        self.lock_semaphore.store(0, Ordering::Relaxed);
        self.spin_count.store(spin_count & (FLAG_BITS | 0x00FF_FFFF), Ordering::Release);
    }

    /// Mark a section that lives in an image's data as statically initialised and give it
    /// its debug record: `DebugInfo` points to `debug`, `debug` points back, its list is
    /// empty, and `SpinCount` gets `RTL_CRITICAL_SECTION_FLAG_STATIC_INIT`. The lock state
    /// is not touched. Both must stay where they are (statics).
    pub fn make_static(&self, debug: &CriticalSectionDebug) {
        let list = (&raw const debug.process_locks_list) as u64;
        debug.critical_section.store(self as *const CriticalSection as u64, Ordering::Relaxed);
        debug.process_locks_list[0].store(list, Ordering::Relaxed);
        debug.process_locks_list[1].store(list, Ordering::Relaxed);
        self.debug_info.store(debug as *const CriticalSectionDebug as u64, Ordering::Relaxed);
        self.spin_count.fetch_or(FLAG_STATIC_INIT, Ordering::Release);
    }

    fn spins(&self) -> u64 {
        self.spin_count.load(Ordering::Relaxed) & 0x00FF_FFFF
    }

    fn own(&self, me: u64) {
        self.owning_thread.store(me, Ordering::Relaxed);
        self.recursion_count.store(1, Ordering::Relaxed);
    }

    /// The event, made now if no thread has made it yet.
    fn event(&self, w: &dyn Waiter) -> u64 {
        loop {
            let e = self.lock_semaphore.load(Ordering::Acquire);
            if e != 0 {
                return e;
            }
            let new = w.create_event();
            if new == 0 {
                w.yield_now();
                continue;
            }
            match self.lock_semaphore.compare_exchange(0, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return new,
                Err(theirs) => {
                    w.close(new);
                    return theirs;
                }
            }
        }
    }

    /// `RtlEnterCriticalSection`.
    pub fn enter(&self, w: &dyn Waiter) {
        let me = w.current_thread();
        for _ in 0..self.spins() {
            if self.lock_count.compare_exchange(FREE, HELD, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                self.own(me);
                return;
            }
            if self.owning_thread.load(Ordering::Relaxed) == me {
                break;
            }
            core::hint::spin_loop();
        }
        if self.lock_count.fetch_sub(1, Ordering::Acquire) == FREE {
            self.own(me);
            return;
        }
        if self.owning_thread.load(Ordering::Relaxed) == me {
            self.recursion_count.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // Someone else owns it: wait for the hand-off.
        let e = self.event(w);
        w.wait(e);
        self.own(me);
    }

    /// `RtlTryEnterCriticalSection`.
    pub fn try_enter(&self, w: &dyn Waiter) -> bool {
        let me = w.current_thread();
        if self.lock_count.compare_exchange(FREE, HELD, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            self.own(me);
            return true;
        }
        if self.owning_thread.load(Ordering::Relaxed) == me {
            self.lock_count.fetch_sub(1, Ordering::Relaxed);
            self.recursion_count.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// `RtlLeaveCriticalSection`. Leaving a section the caller does not own
    /// is a program error that Learn leaves undefined; Poly ignores it.
    pub fn leave(&self, w: &dyn Waiter) {
        if self.owning_thread.load(Ordering::Relaxed) != w.current_thread() {
            return;
        }
        if self.recursion_count.fetch_sub(1, Ordering::Relaxed) > 1 {
            self.lock_count.fetch_add(1, Ordering::Release);
            return;
        }
        self.owning_thread.store(0, Ordering::Relaxed);
        if self.lock_count.fetch_add(1, Ordering::Release) < HELD {
            // Waiters are counted: hand the section to one of them.
            let e = self.event(w);
            w.set(e);
        }
    }

    /// `RtlDeleteCriticalSection`: close the event, if one was made.
    pub fn delete(&self, w: &dyn Waiter) {
        let e = self.lock_semaphore.swap(0, Ordering::AcqRel);
        if e != 0 {
            w.close(e);
        }
    }

    /// `RtlIsCriticalSectionLocked`.
    pub fn is_locked(&self) -> bool {
        self.lock_count.load(Ordering::Relaxed) != FREE
    }

    /// `RtlIsCriticalSectionLockedByThread`: owned by the calling thread.
    pub fn is_owned_by(&self, thread: u64) -> bool {
        self.owning_thread.load(Ordering::Relaxed) == thread
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Condvar, Mutex};
    use std::vec::Vec;

    /// A test event: signalled flag and its condition variable.
    type Event = Arc<(Mutex<bool>, Condvar)>;

    /// Events as (signalled, condvar) pairs; thread ids from a thread-local.
    #[derive(Default)]
    struct Host {
        events: Mutex<HashMap<u64, Event>>,
        next: AtomicU64,
        made: AtomicUsize,
    }

    std::thread_local!(static TID: u64 = {
        static NEXT: AtomicU64 = AtomicU64::new(100);
        NEXT.fetch_add(4, Ordering::Relaxed)
    });

    impl Host {
        fn ev(&self, e: u64) -> Event {
            self.events.lock().unwrap().get(&e).expect("an event").clone()
        }
    }

    impl Waiter for Host {
        fn current_thread(&self) -> u64 {
            TID.with(|t| *t)
        }
        fn create_event(&self) -> u64 {
            let h = self.next.fetch_add(4, Ordering::Relaxed) + 4;
            self.events.lock().unwrap().insert(h, Arc::new((Mutex::new(false), Condvar::new())));
            self.made.fetch_add(1, Ordering::Relaxed);
            h
        }
        fn wait(&self, e: u64) {
            let ev = self.ev(e);
            let mut s = ev.0.lock().unwrap();
            while !*s {
                s = ev.1.wait(s).unwrap();
            }
            *s = false;
        }
        fn set(&self, e: u64) {
            let ev = self.ev(e);
            *ev.0.lock().unwrap() = true;
            ev.1.notify_one();
        }
        fn close(&self, e: u64) {
            self.events.lock().unwrap().remove(&e);
        }
    }

    #[test]
    fn layout_is_winnt_h() {
        assert_eq!(core::mem::size_of::<CriticalSection>(), 40);
        assert_eq!(core::mem::offset_of!(CriticalSection, lock_count), 0x08);
        assert_eq!(core::mem::offset_of!(CriticalSection, recursion_count), 0x0C);
        assert_eq!(core::mem::offset_of!(CriticalSection, owning_thread), 0x10);
        assert_eq!(core::mem::offset_of!(CriticalSection, lock_semaphore), 0x18);
        assert_eq!(core::mem::offset_of!(CriticalSection, spin_count), 0x20);
    }

    /// The values a Windows 11 process shows for its loader lock (spec-chrome-elf-null items 4
    /// and 5): free = (-1, 0, 0), held once = (-2, 1, the owner's id).
    #[test]
    fn free_and_held_fields_are_the_observed_ones() {
        let h = Host::default();
        let cs = CriticalSection::new();
        let at = |cs: &CriticalSection| {
            (
                cs.lock_count.load(Ordering::Relaxed),
                cs.recursion_count.load(Ordering::Relaxed),
                cs.owning_thread.load(Ordering::Relaxed),
            )
        };
        assert_eq!(at(&cs), (-1, 0, 0));
        cs.enter(&h);
        assert_eq!(at(&cs), (-2, 1, h.current_thread()));
        cs.enter(&h);
        assert_eq!(at(&cs), (-3, 2, h.current_thread()));
        cs.leave(&h);
        assert_eq!(at(&cs), (-2, 1, h.current_thread()));
        cs.leave(&h);
        assert_eq!(at(&cs), (-1, 0, 0));
    }

    /// `winnt.h`'s `RTL_CRITICAL_SECTION_DEBUG` on x64, and what a static section shows
    /// (spec-chrome-elf-null item 4: `DebugInfo` non-NULL, `SpinCount` 0x4000000).
    #[test]
    fn a_static_section_has_a_debug_record_and_the_static_flag() {
        assert_eq!(core::mem::size_of::<CriticalSectionDebug>(), 48);
        assert_eq!(core::mem::offset_of!(CriticalSectionDebug, critical_section), 0x08);
        assert_eq!(core::mem::offset_of!(CriticalSectionDebug, process_locks_list), 0x10);
        assert_eq!(core::mem::offset_of!(CriticalSectionDebug, entry_count), 0x20);
        assert_eq!(core::mem::offset_of!(CriticalSectionDebug, identifier), 0x2E);
        let h = Host::default();
        let cs = CriticalSection::new();
        let debug = CriticalSectionDebug::new();
        cs.make_static(&debug);
        assert_eq!(cs.debug_info.load(Ordering::Relaxed), &raw const debug as u64);
        assert_eq!(debug.critical_section.load(Ordering::Relaxed), &raw const cs as u64);
        let list = &raw const debug.process_locks_list as u64;
        assert_eq!(debug.process_locks_list[0].load(Ordering::Relaxed), list);
        assert_eq!(debug.process_locks_list[1].load(Ordering::Relaxed), list);
        assert_eq!(cs.spin_count.load(Ordering::Relaxed), 0x0400_0000);
        // The flag is not a spin count, and the section still works and stays free.
        assert_eq!(cs.spins(), 0);
        assert_eq!(cs.lock_count.load(Ordering::Relaxed), FREE);
        cs.enter(&h);
        assert_eq!(cs.lock_count.load(Ordering::Relaxed), HELD);
        cs.leave(&h);
        assert!(!cs.is_locked());
    }

    #[test]
    fn recursion_and_try_enter() {
        let h = Host::default();
        let cs = CriticalSection::new();
        assert!(!cs.is_locked());
        cs.enter(&h);
        cs.enter(&h);
        assert!(cs.try_enter(&h));
        assert_eq!(cs.recursion_count.load(Ordering::Relaxed), 3);
        assert!(cs.is_owned_by(h.current_thread()));
        cs.leave(&h);
        cs.leave(&h);
        assert!(cs.is_locked());
        cs.leave(&h);
        assert!(!cs.is_locked());
        assert_eq!(cs.owning_thread.load(Ordering::Relaxed), 0);
        assert_eq!(h.made.load(Ordering::Relaxed), 0, "no event without contention");
        // A stray leave changes nothing.
        cs.leave(&h);
        assert_eq!(cs.lock_count.load(Ordering::Relaxed), -1);
    }

    #[test]
    fn another_thread_cannot_try_enter() {
        let h = Arc::new(Host::default());
        let cs = Arc::new(CriticalSection::new());
        cs.enter(&*h);
        let (h2, cs2) = (h.clone(), cs.clone());
        assert!(!std::thread::spawn(move || cs2.try_enter(&*h2)).join().unwrap());
        cs.leave(&*h);
        let (h2, cs2) = (h.clone(), cs.clone());
        assert!(
            std::thread::spawn(move || {
                let ok = cs2.try_enter(&*h2);
                cs2.leave(&*h2);
                ok
            })
            .join()
            .unwrap()
        );
    }

    #[test]
    fn mutual_exclusion_under_contention() {
        for spin in [0u64, 4000] {
            let h = Arc::new(Host::default());
            let cs = Arc::new(CriticalSection::new());
            cs.init(spin);
            let inside = Arc::new(AtomicUsize::new(0));
            let total = Arc::new(AtomicUsize::new(0));
            let threads: Vec<_> = (0..6)
                .map(|_| {
                    let (h, cs, inside, total) = (h.clone(), cs.clone(), inside.clone(), total.clone());
                    std::thread::spawn(move || {
                        for i in 0..2000 {
                            cs.enter(&*h);
                            if i % 3 == 0 {
                                cs.enter(&*h); // recursion under contention
                            }
                            assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0, "two owners at once");
                            total.fetch_add(1, Ordering::Relaxed);
                            inside.fetch_sub(1, Ordering::SeqCst);
                            if i % 3 == 0 {
                                cs.leave(&*h);
                            }
                            cs.leave(&*h);
                        }
                    })
                })
                .collect();
            for t in threads {
                t.join().unwrap();
            }
            assert_eq!(total.load(Ordering::Relaxed), 12000);
            assert_eq!(cs.lock_count.load(Ordering::Relaxed), -1);
            assert!(h.made.load(Ordering::Relaxed) <= 6, "at most one event kept, losers closed");
            cs.delete(&*h);
            assert!(h.events.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn spin_count_keeps_the_flag_byte() {
        let cs = CriticalSection::new();
        cs.init(0x8100_0FA0);
        assert_eq!(cs.spin_count.load(Ordering::Relaxed), 0x8100_0FA0);
        assert_eq!(cs.spins(), 0xFA0);
    }
}
