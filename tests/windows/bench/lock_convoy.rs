//! Lock convoy benchmark (ADR 0242): N threads hammer one lock around a short critical
//! section, for SRW locks (exclusive) and critical sections (spin count 0 and 4000; `std::mutex`
//! and Chrome's `base::Lock` sit on SRW locks). Each line gives the acquisitions per
//! millisecond and the wait for the lock (from the call to the return): median, 99th
//! percentile and maximum, in microseconds.
//!
//! Not a recorded test (its numbers are timings): it lives outside `tests/windows`'s top level,
//! so `win build` builds it only when asked:
//! `cargo xtask pipeline win build --tests-dir tests/windows/bench && cargo xtask pipeline win poly --tests lock_convoy`.
//! Functions from Microsoft Learn (prototypes).
#![no_std]
#![no_main]

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use poly_wtest::*;

type Handle = *mut c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(
        sa: *const c_void,
        stack: usize,
        start: extern "system" fn(*mut c_void) -> u32,
        arg: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> Handle;
    fn WaitForMultipleObjects(n: u32, handles: *const Handle, all: i32, ms: u32) -> u32;
    fn CloseHandle(h: Handle) -> i32;
    fn QueryPerformanceCounter(c: *mut i64) -> i32;
    fn QueryPerformanceFrequency(f: *mut i64) -> i32;
    fn AcquireSRWLockExclusive(l: *mut usize);
    fn ReleaseSRWLockExclusive(l: *mut usize);
    fn InitializeCriticalSectionAndSpinCount(cs: *mut u8, spins: u32) -> i32;
    fn EnterCriticalSection(cs: *mut u8);
    fn LeaveCriticalSection(cs: *mut u8);
    fn DeleteCriticalSection(cs: *mut u8);
}

/// A formatted line.
struct Line {
    b: [u8; 200],
    n: usize,
}

impl core::fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Srw,
    Cs,
}

static SRW: AtomicUsize = AtomicUsize::new(0);
/// `CRITICAL_SECTION`: 40 bytes on x64, 24 on x86.
static mut CS: [u64; 6] = [0; 6];
static KIND: AtomicU32 = AtomicU32::new(0);
static STOP: AtomicU32 = AtomicU32::new(0);
static START: AtomicU32 = AtomicU32::new(0);
/// The protected counter (written only inside the lock).
static mut COUNTER: u64 = 0;
static OPS: AtomicU64 = AtomicU64::new(0);
/// Waits by log2 bucket of their length in 1/8 µs: bucket `b` is `[2^b, 2^(b+1))` eighths.
static HIST: [AtomicU64; 40] = [const { AtomicU64::new(0) }; 40];
static MAX: AtomicU64 = AtomicU64::new(0);
static mut FREQ: i64 = 1;

fn now() -> i64 {
    let mut c = 0;
    // SAFETY: a valid out-pointer.
    unsafe { QueryPerformanceCounter(&mut c) };
    c
}

fn lock(k: Kind) {
    // SAFETY: the lock objects are statics initialised before the threads start.
    unsafe {
        match k {
            Kind::Srw => AcquireSRWLockExclusive(SRW.as_ptr()),
            Kind::Cs => EnterCriticalSection(core::ptr::addr_of_mut!(CS).cast()),
        }
    }
}

fn unlock(k: Kind) {
    // SAFETY: as in `lock`; the caller holds it.
    unsafe {
        match k {
            Kind::Srw => ReleaseSRWLockExclusive(SRW.as_ptr()),
            Kind::Cs => LeaveCriticalSection(core::ptr::addr_of_mut!(CS).cast()),
        }
    }
}

fn busy(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

extern "system" fn worker(_: *mut c_void) -> u32 {
    let k = if KIND.load(Ordering::Relaxed) == 0 { Kind::Srw } else { Kind::Cs };
    while START.load(Ordering::Acquire) == 0 {
        core::hint::spin_loop();
    }
    // SAFETY: written before the threads start.
    let freq = unsafe { FREQ };
    let mut ops = 0u64;
    while STOP.load(Ordering::Relaxed) == 0 {
        let t0 = now();
        lock(k);
        let waited = now() - t0;
        // SAFETY: only touched with the lock held.
        unsafe { COUNTER += 1 };
        busy(40);
        unlock(k);
        ops += 1;
        let eighths = (waited.max(0) as u64 * 8_000_000) / freq.max(1) as u64;
        let b = (64 - eighths.max(1).leading_zeros() as usize - 1).min(39);
        HIST[b].fetch_add(1, Ordering::Relaxed);
        MAX.fetch_max(eighths, Ordering::Relaxed);
        busy(200);
    }
    OPS.fetch_add(ops, Ordering::Relaxed);
    0
}

/// The upper end of the bucket that holds the `q`-th fraction of all waits, in µs.
fn quantile(total: u64, q: u64) -> u64 {
    let want = (total * q).div_ceil(1000).max(1);
    let mut seen = 0;
    for (b, h) in HIST.iter().enumerate() {
        seen += h.load(Ordering::Relaxed);
        if seen >= want {
            return (1u64 << (b + 1)) / 8;
        }
    }
    0
}

fn run(t: &mut Test, name: &str, k: Kind, threads: usize, ms: i64) {
    KIND.store(if k == Kind::Srw { 0 } else { 1 }, Ordering::Relaxed);
    STOP.store(0, Ordering::Relaxed);
    START.store(0, Ordering::Relaxed);
    OPS.store(0, Ordering::Relaxed);
    MAX.store(0, Ordering::Relaxed);
    for h in &HIST {
        h.store(0, Ordering::Relaxed);
    }
    let mut hs = [core::ptr::null_mut(); 32];
    for h in hs.iter_mut().take(threads) {
        let mut tid = 0;
        // SAFETY: a valid start routine.
        *h = unsafe { CreateThread(core::ptr::null(), 0, worker, core::ptr::null_mut(), 0, &mut tid) };
    }
    // SAFETY: written before START.
    let freq = unsafe { FREQ };
    let t0 = now();
    START.store(1, Ordering::Release);
    while now() - t0 < ms * freq / 1000 {
        busy(1000);
    }
    STOP.store(1, Ordering::Relaxed);
    let t1 = now();
    // SAFETY: the handles just made.
    unsafe {
        WaitForMultipleObjects(threads as u32, hs.as_ptr(), 1, 60_000);
        for h in hs.iter().take(threads) {
            CloseHandle(*h);
        }
    }
    let ops = OPS.load(Ordering::Relaxed);
    let elapsed_ms = ((t1 - t0) * 1000 / freq).max(1) as u64;
    let mut l = Line { b: [0; 200], n: 0 };
    let _ = write!(
        l,
        "{name} threads={threads} ops/ms={} p50={}us p99={}us max={}us",
        ops / elapsed_ms,
        quantile(ops, 500),
        quantile(ops, 990),
        MAX.load(Ordering::Relaxed) / 8
    );
    // SAFETY: the bytes written above are ASCII.
    t.line(unsafe { core::str::from_utf8_unchecked(&l.b[..l.n]) });
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls into statics before any thread starts.
    unsafe {
        QueryPerformanceFrequency(core::ptr::addr_of_mut!(FREQ));
        InitializeCriticalSectionAndSpinCount(core::ptr::addr_of_mut!(CS).cast(), 0);
    }
    for n in [1usize, 2, 4, 8, 16] {
        run(t, "srw", Kind::Srw, n, 400);
    }
    for n in [1usize, 4, 8, 16] {
        run(t, "cs", Kind::Cs, n, 400);
    }
    // SAFETY: no thread uses it any more.
    unsafe {
        DeleteCriticalSection(core::ptr::addr_of_mut!(CS).cast());
        InitializeCriticalSectionAndSpinCount(core::ptr::addr_of_mut!(CS).cast(), 4000);
    }
    for n in [4usize, 16] {
        run(t, "cs-spin4000", Kind::Cs, n, 400);
    }
    // SAFETY: no thread uses it any more.
    unsafe { DeleteCriticalSection(core::ptr::addr_of_mut!(CS).cast()) };
}
