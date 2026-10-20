//! Several CPUs: bringing the others up, and what the rest of the kernel
//! asks about them. The machine-specific parts (start-up IPIs, the
//! trampoline, per-CPU tables, IPIs) are `arch::smp`; the policy of which
//! CPUs to start is `poly_smp::plan`, the scheduler's side `poly_sched`
//! (ADR 0053).
//!
//! Boot options: `poly.smp=off` keeps the kernel on the boot CPU,
//! `poly.cpus=<n>` starts at most `n` CPUs (the boot CPU included).

use crate::boot_info::BootInfo;

pub use crate::arch::smp::{Exclusive, Summary, online_cpus, summary};

/// Start the other CPUs (after interrupts and the clock are up on the boot
/// CPU). Each joins the scheduler as it comes up.
pub fn init(boot: &BootInfo) -> Summary {
    let platform = crate::acpi::platform();
    crate::arch::smp::start_aps(platform.as_ref(), boot.arg("poly.smp"), boot.arg("poly.cpus"))
}

/// How many CPUs run the scheduler.
pub fn cpus_online() -> usize {
    online_cpus().len()
}

/// Park every other CPU until the guard is dropped (see
/// [`Exclusive`]): for code that must be the only one running, such as a
/// test comparing frame counts.
pub fn exclusive() -> Exclusive {
    Exclusive::new()
}

/// The running CPU's number.
pub fn this_cpu() -> usize {
    crate::thread::this_cpu()
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use crate::sync::{Mutex, SpinLock};
use crate::thread::{self, CpuSet};
use crate::time;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst};

/// How long a test waits for every CPU to do something. Long: the host may
/// deschedule the emulator's CPUs for a while, one at a time.
const DEADLINE_NS: u64 = 20 * time::NANOS_PER_SEC;

/// Ticks of CPU `cpu`'s own timer so far.
fn ticks_of(cpu: usize) -> u64 {
    crate::arch::percpu::cpu(cpu).map_or(0, |c| c.ticks().load(SeqCst))
}

kernel_test! {
    fn every_cpu_the_madt_lists_is_online() -> TestResult {
        let s = summary();
        let online = online_cpus();
        if s.online != s.planned || online.len() != s.online || cpus_online() != s.online {
            return Err("a CPU the plan chose did not come up");
        }
        // Numbered 0..n with no gaps.
        if online != CpuSet::first_n(s.online) {
            return Err("the online CPUs are not numbered from 0 without gaps");
        }
        // The plan accounts for every processor in the MADT (QEMU's lists
        // each once, all enabled: as many as -smp says).
        let listed = crate::acpi::platform().map_or(1, |p| p.processors.len().max(1));
        if s.planned + s.left_out + s.unreachable != listed {
            return Err("the CPUs started and left out do not add up to the MADT's");
        }
        // One per-CPU area per CPU, each with its own APIC ID.
        let areas: alloc::vec::Vec<_> = crate::arch::percpu::all().collect();
        if areas.len() != s.online {
            return Err("there are not as many per-CPU areas as CPUs");
        }
        for (i, a) in areas.iter().enumerate() {
            if a.index() != i || areas[..i].iter().any(|b| b.apic_id() == a.apic_id()) {
                return Err("two CPUs share a number or an APIC ID");
            }
        }
        // Every CPU's own timer ticks.
        let start = time::now_ns();
        let before: alloc::vec::Vec<u64> = online.iter().map(ticks_of).collect();
        while online.iter().zip(&before).any(|(c, &t)| ticks_of(c) < t + 2) {
            if time::now_ns() - start > DEADLINE_NS {
                return Err("a CPU's timer does not tick");
            }
            thread::sleep(core::time::Duration::from_millis(1));
        }
        Ok(())
    }
}

/// CPUs that ran a thread of [`threads_run_on_every_cpu_at_once`], as bits.
static RAN_ON: AtomicU64 = AtomicU64::new(0);
/// Threads of that test that have started.
static ARRIVED: AtomicUsize = AtomicUsize::new(0);

/// Note the CPU, then wait until every CPU's thread has started (the
/// argument is the CPU this one is pinned to); return whether it was on
/// that CPU throughout and all arrived in time.
fn meet(cpu: usize) -> usize {
    let n = cpus_online();
    let mut right_cpu = this_cpu() == cpu;
    RAN_ON.fetch_or(1 << this_cpu(), SeqCst);
    ARRIVED.fetch_add(1, SeqCst);
    let start = time::now_ns();
    while ARRIVED.load(SeqCst) < n {
        if time::now_ns() - start > DEADLINE_NS {
            return 0;
        }
        thread::yield_now();
        right_cpu &= this_cpu() == cpu;
    }
    usize::from(right_cpu) + 1
}

kernel_test! {
    fn threads_run_on_every_cpu_at_once() -> TestResult {
        // One thread pinned to each CPU, each waiting for all the others to
        // have started: they run at the same time, each where it was put.
        RAN_ON.store(0, SeqCst);
        ARRIVED.store(0, SeqCst);
        let online = online_cpus();
        let mut threads = alloc::vec::Vec::new();
        for cpu in online.iter() {
            threads.push(thread::spawn_on("smp-meet", meet, cpu, CpuSet::one(cpu)).map_err(|_| "spawn failed")?);
        }
        let mut result = Ok(());
        for t in threads {
            match thread::join(t) {
                Ok(2) => {}
                Ok(1) => result = Err("a pinned thread ran on another CPU"),
                Ok(_) => result = result.and(Err("not every CPU's thread started in time")),
                Err(_) => return Err("join failed"),
            }
        }
        result?;
        if RAN_ON.load(SeqCst) != online.bits() { Err("some CPU ran none of the threads") } else { Ok(()) }
    }
}

/// What [`count_under_locks`] counts up, under each kind of lock.
static SPIN_COUNT: SpinLock<u64> = SpinLock::new(0);
static MUTEX_COUNT: Mutex<u64> = Mutex::new(0);
const INCREMENTS: usize = 5_000;

/// Add 1 to both counters [`INCREMENTS`] times, reading and writing each
/// in separate steps with a pause between, so that an increment the lock
/// fails to protect is lost.
fn count_under_locks(_: usize) -> usize {
    for i in 0..INCREMENTS {
        {
            let mut c = SPIN_COUNT.lock();
            let v = *c;
            core::hint::spin_loop();
            *c = v + 1;
        }
        {
            let mut c = MUTEX_COUNT.lock();
            let v = *c;
            if i % 64 == 0 {
                // Sometimes give up the CPU holding the mutex: the others
                // block on it, and are woken (on any CPU) when it is free.
                thread::yield_now();
            }
            *c = v + 1;
        }
    }
    0
}

kernel_test! {
    fn spinlocks_and_mutexes_exclude_across_cpus() -> TestResult {
        *SPIN_COUNT.lock() = 0;
        *MUTEX_COUNT.lock() = 0;
        // Two per CPU: placement spreads new threads over idle CPUs, so the
        // locks are contended across CPUs (unless the host stalls some of
        // the emulator's CPUs throughout, which is no error).
        let n = 2 * cpus_online().max(2);
        let mut threads = alloc::vec::Vec::new();
        for _ in 0..n {
            threads.push(thread::spawn_named("smp-count", count_under_locks, 0).map_err(|_| "spawn failed")?);
        }
        for t in threads {
            thread::join(t).map_err(|_| "join failed")?;
        }
        let expected = (n * INCREMENTS) as u64;
        if *SPIN_COUNT.lock() != expected {
            return Err("increments under a spinlock were lost");
        }
        if *MUTEX_COUNT.lock() != expected {
            return Err("increments under a mutex were lost");
        }
        Ok(())
    }
}
