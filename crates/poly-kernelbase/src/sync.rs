//! Synchronisation: wait timeouts and results, named-object names, and the
//! slim reader/writer lock.
//!
//! Learn: `WaitForSingleObject(Ex)`, `WaitForMultipleObjects(Ex)`, `Sleep`,
//! "Kernel object namespaces", "Slim Reader/Writer (SRW) Locks". Constants
//! are `winbase.h` / `winnt.h` / `ntstatus.h`.

use crate::error::from_status;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_ntobj::NtStatus;

/// `INFINITE`.
pub const INFINITE: u32 = 0xFFFF_FFFF;
/// `WAIT_OBJECT_0`, `WAIT_ABANDONED_0`, `WAIT_IO_COMPLETION`,
/// `WAIT_TIMEOUT`, `WAIT_FAILED`.
pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_ABANDONED_0: u32 = 0x80;
pub const WAIT_IO_COMPLETION: u32 = 0xC0;
pub const WAIT_TIMEOUT: u32 = 0x102;
pub const WAIT_FAILED: u32 = 0xFFFF_FFFF;
/// `MAXIMUM_WAIT_OBJECTS`.
pub const MAXIMUM_WAIT_OBJECTS: u32 = 64;

/// An `NtWait*` / `NtDelayExecution` timeout for a Win32 one in
/// milliseconds: `INFINITE` is none (wait forever); anything else is
/// relative, in 100 ns units, negative (`wdm.h`'s `LARGE_INTEGER`
/// convention).
pub fn timeout(ms: u32) -> Option<i64> {
    if ms == INFINITE { None } else { Some(-(i64::from(ms) * 10_000)) }
}

/// The Win32 result of an `NtWaitFor*Object(s)` status: an object index
/// (`WAIT_OBJECT_0 + n`), an abandoned mutex (`WAIT_ABANDONED_0 + n`),
/// `WAIT_IO_COMPLETION` (an APC ran; `STATUS_USER_APC` or
/// `STATUS_ALERTED`), `WAIT_TIMEOUT`, or a failure as `Err(Win32 error)`
/// (the function returns `WAIT_FAILED` and sets it).
pub fn wait_result(status: NtStatus) -> Result<u32, u32> {
    match status.0 {
        n @ 0x00..=0x3F => Ok(WAIT_OBJECT_0 + n),
        n @ 0x80..=0xBF => Ok(n),
        0xC0 | 0x101 => Ok(WAIT_IO_COMPLETION),
        0x102 => Ok(WAIT_TIMEOUT),
        _ if !status.is_success() => Err(from_status(status)),
        _ => Ok(status.0),
    }
}

/// `NtCreateEvent`'s `EventType`.
pub const NOTIFICATION_EVENT: u32 = 0;
pub const SYNCHRONIZATION_EVENT: u32 = 1;
/// `CreateEventExW`'s flags.
pub const CREATE_EVENT_MANUAL_RESET: u32 = 1;
pub const CREATE_EVENT_INITIAL_SET: u32 = 2;
/// `*_ALL_ACCESS` (`winnt.h`).
pub const EVENT_ALL_ACCESS: u32 = 0x1F_0003;
pub const MUTANT_ALL_ACCESS: u32 = 0x1F_0001;
pub const SEMAPHORE_ALL_ACCESS: u32 = 0x1F_0003;

/// The object-manager name of a named kernel object (Learn: "Kernel object
/// namespaces"). Poly has one session, so the `Global\` and `Local\`
/// namespaces are both `\BaseNamedObjects` (to be confirmed by the spec
/// agent against a single-session Windows, ADR 0079). An empty name means
/// an unnamed object (`None`).
pub fn object_name(name: &[u16]) -> Option<Vec<u16>> {
    if name.is_empty() {
        return None;
    }
    let mut rest = name;
    for prefix in ["Global\\", "Local\\"] {
        let p: Vec<u16> = crate::wide(prefix);
        if rest.len() >= p.len() && crate::text::eq_ignore_case(&rest[..p.len()], &p) {
            rest = &rest[p.len()..];
            break;
        }
    }
    let mut out = crate::wide("\\BaseNamedObjects\\");
    out.extend_from_slice(rest);
    Some(out)
}

/// `CreateSemaphoreW`'s counts: Learn: the maximum must be greater than
/// zero, the initial count between zero and the maximum.
pub fn valid_semaphore(initial: i32, maximum: i32) -> bool {
    maximum > 0 && (0..=maximum).contains(&initial)
}

// ----- the slim reader/writer lock --------------------------------------------

/// How a thread waits for an SRW lock: [`Pause::pause`] is called with the
/// number of failed attempts so far, and may spin, yield the processor or
/// sleep.
pub trait Pause {
    fn pause(&self, attempt: u32);
}

/// An `SRWLOCK` is one pointer-sized word, zero when free
/// (`SRWLOCK_INIT`). Poly's encoding (the structure is opaque, `winnt.h`):
/// bit 0 is held exclusively, bit 1 a writer is waiting (readers then hold
/// back, so writers are not starved), and the rest counts shared owners.
pub mod srw {
    pub const EXCLUSIVE: usize = 1;
    pub const WRITER_WAITING: usize = 2;
    pub const READER: usize = 4;
}

/// `TryAcquireSRWLockExclusive`.
pub fn try_acquire_exclusive(word: &AtomicUsize) -> bool {
    let s = word.load(Ordering::Relaxed);
    s & !srw::WRITER_WAITING == 0
        && word.compare_exchange(s, srw::EXCLUSIVE, Ordering::Acquire, Ordering::Relaxed).is_ok()
}

/// `AcquireSRWLockExclusive`.
pub fn acquire_exclusive(word: &AtomicUsize, p: &dyn Pause) {
    let mut attempt = 0;
    while !try_acquire_exclusive(word) {
        word.fetch_or(srw::WRITER_WAITING, Ordering::Relaxed);
        p.pause(attempt);
        attempt = attempt.saturating_add(1);
    }
}

/// `ReleaseSRWLockExclusive`. A waiting writer's bit survives so it keeps
/// its priority over new readers.
pub fn release_exclusive(word: &AtomicUsize) {
    word.fetch_and(!srw::EXCLUSIVE, Ordering::Release);
}

/// `TryAcquireSRWLockShared`.
pub fn try_acquire_shared(word: &AtomicUsize) -> bool {
    let s = word.load(Ordering::Relaxed);
    s & (srw::EXCLUSIVE | srw::WRITER_WAITING) == 0
        && word.compare_exchange(s, s + srw::READER, Ordering::Acquire, Ordering::Relaxed).is_ok()
}

/// `AcquireSRWLockShared`.
pub fn acquire_shared(word: &AtomicUsize, p: &dyn Pause) {
    let mut attempt = 0;
    while !try_acquire_shared(word) {
        p.pause(attempt);
        attempt = attempt.saturating_add(1);
    }
}

/// `ReleaseSRWLockShared`.
pub fn release_shared(word: &AtomicUsize) {
    word.fetch_sub(srw::READER, Ordering::Release);
}

// ----- condition variables -----------------------------------------------------

/// A `CONDITION_VARIABLE` is one pointer-sized word, zero when initialised
/// (`CONDITION_VARIABLE_INIT`, `winnt.h`). Poly's encoding (the structure
/// is opaque): a count of wakes. A sleeper notes the count, releases its
/// lock and waits until the count moves or its time runs out; both wake
/// functions move it, so `WakeConditionVariable` may wake more than one
/// thread, which Learn allows ("Condition variables are subject to
/// spurious wakeups ... and stolen wakeups"). Waiters back off like the SRW
/// lock's (ADR 0079).
pub fn condition_sleep(
    word: &AtomicUsize,
    release: &mut dyn FnMut(),
    acquire: &mut dyn FnMut(),
    expired: &dyn Fn() -> bool,
    p: &dyn Pause,
) -> bool {
    let seen = word.load(Ordering::Acquire);
    release();
    let mut attempt = 0;
    let woken = loop {
        if word.load(Ordering::Acquire) != seen {
            break true;
        }
        if expired() {
            break false;
        }
        p.pause(attempt);
        attempt = attempt.saturating_add(1);
    };
    acquire();
    woken
}

/// `WakeConditionVariable` and `WakeAllConditionVariable`.
pub fn condition_wake(word: &AtomicUsize) {
    word.fetch_add(1, Ordering::Release);
}

/// `CONDITION_VARIABLE_LOCKMODE_SHARED` (`winbase.h`).
pub const CONDITION_VARIABLE_LOCKMODE_SHARED: u32 = 0x1;

// ----- one-time initialisation ---------------------------------------------------

/// `INIT_ONCE_CHECK_ONLY`, `INIT_ONCE_ASYNC`, `INIT_ONCE_INIT_FAILED`,
/// `INIT_ONCE_CTX_RESERVED_BITS` (`winbase.h`).
pub const INIT_ONCE_CHECK_ONLY: u32 = 0x1;
pub const INIT_ONCE_ASYNC: u32 = 0x2;
pub const INIT_ONCE_INIT_FAILED: u32 = 0x4;
pub const INIT_ONCE_CTX_RESERVED_BITS: u32 = 2;

/// An `INIT_ONCE` is one pointer-sized word, zero when initialised
/// (`INIT_ONCE_STATIC_INIT`). Poly's encoding: bit 0 an initialisation is
/// running, bit 1 it is done; once done, the bits above the
/// `INIT_ONCE_CTX_RESERVED_BITS` hold the context (Learn: "the low
/// INIT_ONCE_CTX_RESERVED_BITS bits of the context must be zero").
pub mod once {
    pub const RUNNING: usize = 1;
    pub const DONE: usize = 2;
}

/// What `InitOnceBeginInitialize` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begin {
    /// Done before: the stored context.
    Done(usize),
    /// The caller must initialise (it now owns the run).
    Pending,
    /// `INIT_ONCE_CHECK_ONLY` and not done: the function fails (Learn);
    /// Poly reports `ERROR_GEN_FAILURE` (to be confirmed by the spec
    /// agent).
    NotDone,
}

/// `InitOnceBeginInitialize` (synchronous): wait while another thread
/// runs the initialisation. Asynchronous initialisation is not supported
/// (`flags` with `INIT_ONCE_ASYNC`: `Err(ERROR_INVALID_PARAMETER)`, to be
/// confirmed against what Windows supports).
pub fn once_begin(word: &AtomicUsize, flags: u32, p: &dyn Pause) -> Result<Begin, u32> {
    if flags & !(INIT_ONCE_CHECK_ONLY | INIT_ONCE_ASYNC) != 0 || flags & INIT_ONCE_ASYNC != 0 {
        return Err(crate::error::ERROR_INVALID_PARAMETER);
    }
    let mut attempt = 0;
    loop {
        let s = word.load(Ordering::Acquire);
        if s & once::DONE != 0 {
            return Ok(Begin::Done(s & !3));
        }
        if flags & INIT_ONCE_CHECK_ONLY != 0 {
            return Ok(Begin::NotDone);
        }
        if s & once::RUNNING == 0
            && word.compare_exchange(s, once::RUNNING, Ordering::Acquire, Ordering::Relaxed).is_ok()
        {
            return Ok(Begin::Pending);
        }
        p.pause(attempt);
        attempt = attempt.saturating_add(1);
    }
}

/// `InitOnceComplete` (synchronous) by the thread running the
/// initialisation: done with `context` (its low bits must be clear), or,
/// with `INIT_ONCE_INIT_FAILED`, back to not started. Not running is
/// `ERROR_GEN_FAILURE`; bad flags or context `ERROR_INVALID_PARAMETER`.
pub fn once_complete(word: &AtomicUsize, flags: u32, context: usize) -> Result<(), u32> {
    use crate::error::*;
    if flags & !(INIT_ONCE_INIT_FAILED | INIT_ONCE_ASYNC) != 0 || flags & INIT_ONCE_ASYNC != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let failed = flags & INIT_ONCE_INIT_FAILED != 0;
    if context & 3 != 0 || (failed && context != 0) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if word.load(Ordering::Acquire) & (once::RUNNING | once::DONE) != once::RUNNING {
        return Err(ERROR_GEN_FAILURE);
    }
    word.store(if failed { 0 } else { context | once::DONE }, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::*;
    extern crate std;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn timeouts() {
        assert_eq!(timeout(INFINITE), None);
        assert_eq!(timeout(0), Some(0));
        assert_eq!(timeout(1500), Some(-15_000_000));
    }

    #[test]
    fn wait_results() {
        assert_eq!(wait_result(NtStatus(0)), Ok(WAIT_OBJECT_0));
        assert_eq!(wait_result(NtStatus(3)), Ok(3));
        assert_eq!(wait_result(NtStatus(0x80)), Ok(WAIT_ABANDONED_0));
        assert_eq!(wait_result(NtStatus(0x81)), Ok(0x81));
        assert_eq!(wait_result(NtStatus(0xC0)), Ok(WAIT_IO_COMPLETION));
        assert_eq!(wait_result(NtStatus(0x102)), Ok(WAIT_TIMEOUT));
        assert_eq!(wait_result(NtStatus(0xC000_0008)), Err(ERROR_INVALID_HANDLE));
        assert_eq!(wait_result(NtStatus(0xC000_000D)), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn object_names() {
        let s = |n: &str| object_name(&crate::wide(n)).map(|v| std::string::String::from_utf16_lossy(&v));
        assert_eq!(s("Ev"), Some("\\BaseNamedObjects\\Ev".into()));
        assert_eq!(s("Global\\Ev"), Some("\\BaseNamedObjects\\Ev".into()));
        assert_eq!(s("local\\Ev"), Some("\\BaseNamedObjects\\Ev".into()));
        assert_eq!(s(""), None);
    }

    #[test]
    fn semaphore_counts() {
        assert!(valid_semaphore(0, 1));
        assert!(valid_semaphore(5, 5));
        assert!(!valid_semaphore(6, 5));
        assert!(!valid_semaphore(-1, 5));
        assert!(!valid_semaphore(0, 0));
    }

    struct Yield;
    impl Pause for Yield {
        fn pause(&self, _: u32) {
            std::thread::yield_now();
        }
    }

    #[test]
    fn srw_single_thread() {
        let w = AtomicUsize::new(0);
        assert!(try_acquire_shared(&w));
        assert!(try_acquire_shared(&w));
        assert!(!try_acquire_exclusive(&w));
        release_shared(&w);
        release_shared(&w);
        assert!(try_acquire_exclusive(&w));
        assert!(!try_acquire_shared(&w));
        assert!(!try_acquire_exclusive(&w));
        release_exclusive(&w);
        assert_eq!(w.load(Ordering::Relaxed), 0);
        // A waiting writer holds new readers back.
        assert!(try_acquire_shared(&w));
        w.fetch_or(srw::WRITER_WAITING, Ordering::Relaxed);
        assert!(!try_acquire_shared(&w));
        release_shared(&w);
        assert!(try_acquire_exclusive(&w));
        release_exclusive(&w);
        assert_eq!(w.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn init_once_runs_once() {
        let w = AtomicUsize::new(0);
        assert_eq!(once_begin(&w, INIT_ONCE_CHECK_ONLY, &Yield), Ok(Begin::NotDone));
        assert_eq!(once_complete(&w, 0, 0x100), Err(ERROR_GEN_FAILURE));
        assert_eq!(once_begin(&w, 0, &Yield), Ok(Begin::Pending));
        // A failed run lets the next caller try again.
        assert_eq!(once_complete(&w, INIT_ONCE_INIT_FAILED, 0), Ok(()));
        assert_eq!(once_begin(&w, 0, &Yield), Ok(Begin::Pending));
        assert_eq!(once_complete(&w, 0, 0x101), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(once_complete(&w, 0, 0x100), Ok(()));
        assert_eq!(once_begin(&w, 0, &Yield), Ok(Begin::Done(0x100)));
        assert_eq!(once_begin(&w, INIT_ONCE_CHECK_ONLY, &Yield), Ok(Begin::Done(0x100)));
        assert_eq!(once_complete(&w, 0, 0), Err(ERROR_GEN_FAILURE));
        assert_eq!(once_begin(&w, INIT_ONCE_ASYNC, &Yield), Err(ERROR_INVALID_PARAMETER));
        // Concurrent callers: one runs, the others wait for its context.
        let w = Arc::new(AtomicUsize::new(0));
        let runs = Arc::new(AtomicU64::new(0));
        let threads: std::vec::Vec<_> = (0..4)
            .map(|_| {
                let (w, runs) = (w.clone(), runs.clone());
                std::thread::spawn(move || match once_begin(&w, 0, &Yield).unwrap() {
                    Begin::Pending => {
                        runs.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        once_complete(&w, 0, 0x40).unwrap();
                        0x40
                    }
                    Begin::Done(c) => c,
                    Begin::NotDone => unreachable!(),
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), 0x40);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn condition_variables_wake_and_time_out() {
        let cv = AtomicUsize::new(0);
        let (mut released, mut acquired) = (0, 0);
        // Nobody wakes it: the timeout ends the sleep, the lock comes back.
        let woke = condition_sleep(&cv, &mut || released += 1, &mut || acquired += 1, &|| true, &Yield);
        assert!(!woke);
        assert_eq!((released, acquired), (1, 1));
        let cv = Arc::new(AtomicUsize::new(0));
        let lock = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(AtomicU64::new(0));
        let sleeper = {
            let (cv, lock, ready) = (cv.clone(), lock.clone(), ready.clone());
            std::thread::spawn(move || {
                acquire_exclusive(&lock, &Yield);
                while ready.load(Ordering::SeqCst) == 0 {
                    let woke = condition_sleep(
                        &cv,
                        &mut || release_exclusive(&lock),
                        &mut || acquire_exclusive(&lock, &Yield),
                        &|| false,
                        &Yield,
                    );
                    assert!(woke);
                }
                release_exclusive(&lock);
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(5));
        acquire_exclusive(&lock, &Yield);
        ready.store(1, Ordering::SeqCst);
        condition_wake(&cv);
        release_exclusive(&lock);
        sleeper.join().unwrap();
    }

    #[test]
    fn srw_excludes_writers_across_threads() {
        let w = Arc::new(AtomicUsize::new(0));
        let counter = Arc::new(AtomicU64::new(0));
        let inside = Arc::new(AtomicUsize::new(0));
        let threads: std::vec::Vec<_> = (0..6)
            .map(|i| {
                let (w, counter, inside) = (w.clone(), counter.clone(), inside.clone());
                std::thread::spawn(move || {
                    for _ in 0..2000 {
                        if i % 2 == 0 {
                            acquire_exclusive(&w, &Yield);
                            assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                            let v = counter.load(Ordering::Relaxed);
                            counter.store(v + 1, Ordering::Relaxed);
                            inside.fetch_sub(1, Ordering::SeqCst);
                            release_exclusive(&w);
                        } else {
                            acquire_shared(&w, &Yield);
                            let _ = counter.load(Ordering::Relaxed);
                            release_shared(&w);
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(counter.load(Ordering::Relaxed), 3 * 2000);
        assert_eq!(w.load(Ordering::Relaxed) & !srw::WRITER_WAITING, 0);
    }
}
