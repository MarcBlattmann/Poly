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
/// agent against a single-session Windows, ADR 0076). An empty name means
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
