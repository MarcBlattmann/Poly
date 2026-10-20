//! Kernel time: a monotonic clock in nanoseconds since boot and the periodic
//! scheduler tick, both provided by the architecture (on x86-64:
//! `arch/x86_64/time.rs`, a TSC or HPET clock and the local APIC timer).
//! `thread::sleep` builds on them.

use crate::arch;

pub use crate::arch::time::{NANOS_PER_TICK, TICK_HZ};

pub const NANOS_PER_SEC: u64 = 1_000_000_000;
pub const NANOS_PER_MILLI: u64 = 1_000_000;

/// Nanoseconds since the clock started during boot (0 before). Never goes
/// backwards, also when read from different threads.
pub fn now_ns() -> u64 {
    arch::time::now_ns()
}

/// Timer interrupts since the tick started.
pub fn ticks() -> u64 {
    arch::time::ticks()
}

/// Whether the clock and the tick are running (after `arch::init_interrupts`).
pub fn is_running() -> bool {
    arch::time::info().is_some()
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use crate::thread;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};

kernel_test! {
    fn clock_never_goes_backwards() -> TestResult {
        let mut last = now_ns();
        if last == 0 {
            return Err("the clock is not running");
        }
        for _ in 0..200_000 {
            let now = now_ns();
            if now < last {
                return Err("the clock went backwards");
            }
            last = now;
        }
        Ok(())
    }
}

kernel_test! {
    fn timer_ticks_at_the_expected_rate() -> TestResult {
        // Count tick interrupts over ten 20 ms windows of clock time. A tick
        // that comes due while the previous one is still pending merges with
        // it, so an emulator the host deschedules loses ticks: never accept
        // more ticks than time allows, but judge the low side by the best
        // window, generously. (`arch` checks the timer's period precisely.)
        const WINDOW_NS: u64 = 20 * NANOS_PER_MILLI;
        let mut best = 0;
        let mut total = (0, 0);
        for _ in 0..10 {
            let (t0, k0) = (now_ns(), ticks());
            while now_ns() - t0 < WINDOW_NS {
                core::hint::spin_loop();
            }
            let (t1, k1) = (now_ns(), ticks());
            if k1 - k0 > (t1 - t0) / NANOS_PER_TICK + 2 {
                crate::kprintln!("time: {} ticks in {} us", k1 - k0, (t1 - t0) / 1000);
                return Err("the timer ticks faster than TICK_HZ");
            }
            best = best.max((k1 - k0) * NANOS_PER_SEC / (t1 - t0));
            total = (total.0 + k1 - k0, total.1 + t1 - t0);
        }
        crate::kprintln!(
            "time: {} ticks in {} us, best 20 ms window {best} Hz (nominal {TICK_HZ} Hz)",
            total.0,
            total.1 / 1000
        );
        if best < TICK_HZ / 4 {
            return Err("the timer ticks far slower than TICK_HZ");
        }
        Ok(())
    }
}

/// Latest clock reading published by `publish_times`.
static PUBLISHED: AtomicU64 = AtomicU64::new(0);
static STOP_PUBLISHING: AtomicBool = AtomicBool::new(false);
static PUBLISHER_FAILED: AtomicBool = AtomicBool::new(false);

/// Reads the clock in a loop (getting preempted along the way) and
/// publishes each reading.
fn publish_times(_: usize) -> usize {
    let mut last = 0;
    while !STOP_PUBLISHING.load(SeqCst) {
        let now = now_ns();
        if now < last {
            PUBLISHER_FAILED.store(true, SeqCst);
        }
        last = now;
        PUBLISHED.fetch_max(now, SeqCst);
    }
    0
}

kernel_test! {
    fn clock_is_monotonic_across_threads() -> TestResult {
        STOP_PUBLISHING.store(false, SeqCst);
        PUBLISHER_FAILED.store(false, SeqCst);
        PUBLISHED.store(0, SeqCst);
        let t = thread::spawn(publish_times, 0).map_err(|_| "spawn failed")?;
        // Whatever the other thread read before we look must not be later
        // than what we read after looking. Both threads run with preemption,
        // so the reads interleave at arbitrary points. Keep going for a few
        // of the other thread's turns (a busy host can make that slow).
        let start = now_ns();
        let (mut turns, mut seen) = (0, 0);
        let mut result = Ok(());
        while turns < 3 && now_ns() - start < 2 * NANOS_PER_SEC {
            let theirs = PUBLISHED.load(SeqCst);
            let mine = now_ns();
            if mine < theirs {
                result = Err("a clock reading was earlier than one another thread took before it");
                break;
            }
            if theirs != seen {
                turns += 1;
                seen = theirs;
            }
        }
        if turns == 0 {
            result = result.and(Err("the other thread never ran: no preemption"));
        }
        STOP_PUBLISHING.store(true, SeqCst);
        thread::join(t).map_err(|_| "join failed")?;
        if PUBLISHER_FAILED.load(SeqCst) {
            return Err("the clock went backwards on the other thread");
        }
        result
    }
}
