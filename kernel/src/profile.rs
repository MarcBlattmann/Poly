//! `poly.debug.profile`: a sampling profile of the kernel, for finding
//! where system time goes (docs/DEBUGGING.md). Every timer tick that
//! interrupts kernel code counts its instruction address and the return
//! address of its frame (the caller); at power-off the most frequent ones
//! are printed, to be looked up in the kernel's symbols
//! (`addr2line -f -C -e <kernel> <address>`):
//!
//! ```text
//! profile: 12345 ticks: 10000 kernel, 2345 user
//! profile: pc 0xffffffff80123456 4000
//! profile: caller 0xffffffff80123abc 3900
//! ```
//!
//! Linux system calls are counted and timed too (ADR 0148): per call number,
//! how often it ran and the wall time it took (a call that blocks counts the
//! time it slept), the ones that took longest in total first:
//!
//! ```text
//! profile: syscall futex 120345 calls 8123456 us
//! ```
//!
//! Disk requests and page faults by kind are timed the same
//! way (`profile: event disk-read 4000 times 800000 us`).
//!
//! Execs and exits of Linux processes are logged with the time
//! (`profile: proc 12345 ms exec pid 32 /usr/lib/chromium/chromium
//! --type=zygote`), a timeline of what ran when.
//!
//! Off unless the flag is given: then each sample is a few atomic
//! operations in the timer interrupt. The tables are fixed-size open
//! addressing hash tables; a full table drops samples (counted).

use crate::boot_info::BootInfo;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Kernel argument that turns the profile on.
pub const PROFILE_FLAG: &str = "poly.debug.profile";

static ON: AtomicBool = AtomicBool::new(false);

const SLOTS: usize = 4096;

/// One table: addresses (0 for a free slot) and their counts.
struct Table {
    keys: [AtomicU64; SLOTS],
    counts: [AtomicU64; SLOTS],
}

impl Table {
    const fn new() -> Table {
        Table { keys: [const { AtomicU64::new(0) }; SLOTS], counts: [const { AtomicU64::new(0) }; SLOTS] }
    }

    fn add(&self, key: u64) {
        if key == 0 {
            return;
        }
        let mut i = (key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 52) as usize % SLOTS;
        for _ in 0..64 {
            match self.keys[i].compare_exchange(0, key, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    self.counts[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(k) if k == key => {
                    self.counts[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => i = (i + 1) % SLOTS,
            }
        }
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }

    /// The `n` most frequent entries, most frequent first.
    fn top(&self, out: &mut [(u64, u64)]) {
        for (k, c) in self.keys.iter().zip(&self.counts) {
            let (k, c) = (k.load(Ordering::Relaxed), c.load(Ordering::Relaxed));
            if k == 0 {
                continue;
            }
            if let Some(min) = out.iter_mut().min_by_key(|e| e.1).filter(|e| e.1 < c) {
                *min = (k, c);
            }
        }
        out.sort_unstable_by_key(|e| core::cmp::Reverse(e.1));
    }
}

/// Linux system call numbers counted (higher ones are not).
const CALLS: usize = 512;
static CALL_COUNT: [AtomicU64; CALLS] = [const { AtomicU64::new(0) }; CALLS];
static CALL_NS: [AtomicU64; CALLS] = [const { AtomicU64::new(0) }; CALLS];

/// Whether the profile is on (callers time a system call only then).
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Linux system call `number` took `ns` nanoseconds.
pub fn syscall(number: u64, ns: u64) {
    let i = (number as usize).min(CALLS - 1);
    CALL_COUNT[i].fetch_add(1, Ordering::Relaxed);
    CALL_NS[i].fetch_add(ns, Ordering::Relaxed);
}

/// Things whose count and time the profile adds up besides system calls
/// (ADR 0148).
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Event {
    /// A disk read request, from queueing to its result.
    DiskRead,
    DiskWrite,
    DiskFlush,
    /// Waiting for a process's memory lock to resolve a page fault.
    FaultLock,
    /// Page faults by what resolved them: a zero page, a copy of a file page
    /// read into a frame of its own, a mapped page-cache page (read from the
    /// disk if it was not cached), copy on write, a permission fix.
    FaultZero,
    FaultFileCopy,
    FaultCached,
    FaultCow,
    FaultPerm,
    /// Of a cached-page fault, the part spent getting the page from the page
    /// cache (its lock, the read from the file system).
    FaultFill,
    /// Of a cached-page fault, mapping the page itself, and fault-around.
    FaultMap,
    FaultAround,
    /// Of those, the page cache did not hold the page: read from the file system.
    FaultMiss,
    /// Clearing a new frame (a page of memory), and getting one from the
    /// frame allocator.
    ZeroFrame,
    AllocFrame,
    /// Clearing a new frame for the page cache.
    ZeroCacheFrame,
    /// A reschedule IPI sent (the time is the sender's: the write to the
    /// interrupt controller, which a virtual machine traps).
    Ipi,
    /// An idle CPU halted until its next interrupt.
    Halt,
}

const EVENTS: [&str; 18] = [
    "disk-read",
    "disk-write",
    "disk-flush",
    "fault-lock",
    "fault-zero",
    "fault-file-copy",
    "fault-cached",
    "fault-cow",
    "fault-perm",
    "fault-fill",
    "fault-map",
    "fault-around",
    "fault-miss",
    "zero-frame",
    "alloc-frame",
    "zero-cache-frame",
    "ipi-send",
    "idle-halt",
];
static EVENT_COUNT: [AtomicU64; EVENTS.len()] = [const { AtomicU64::new(0) }; EVENTS.len()];
static EVENT_NS: [AtomicU64; EVENTS.len()] = [const { AtomicU64::new(0) }; EVENTS.len()];
static EVENT_BYTES: [AtomicU64; EVENTS.len()] = [const { AtomicU64::new(0) }; EVENTS.len()];

/// `event` moved `n` bytes.
pub fn bytes(event: Event, n: usize) {
    EVENT_BYTES[event as usize].fetch_add(n as u64, Ordering::Relaxed);
}

/// `event` took `ns` nanoseconds.
pub fn event(event: Event, ns: u64) {
    EVENT_COUNT[event as usize].fetch_add(1, Ordering::Relaxed);
    EVENT_NS[event as usize].fetch_add(ns, Ordering::Relaxed);
}

/// The clock, if the profile is on: pass it to [`since`] when the event is over.
pub fn start() -> Option<u64> {
    ON.load(Ordering::Relaxed).then(crate::time::now_ns)
}

/// `event` took the time since `start` (from [`start`]).
pub fn since(e: Event, start: Option<u64>) {
    if let Some(t) = start {
        event(e, crate::time::now_ns().saturating_sub(t));
    }
}

/// A line of the process timeline: `what` (`exec`, `exit`) happened to
/// process `pid` now, with `detail` (the program and its arguments, the
/// status): `profile: proc 12345 ms exec pid 32 /usr/lib/chromium/chromium`.
pub fn process(what: &str, pid: u32, detail: &[u8]) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let mut line = alloc::string::String::new();
    for &b in detail.iter().take(160) {
        line.push(if (0x20..0x7f).contains(&b) { b as char } else { ' ' });
    }
    crate::kprintln!("profile: proc {} ms {what} pid {pid} {line}", crate::time::now_ns() / 1_000_000);
}

static PCS: Table = Table::new();
static CALLERS: Table = Table::new();
static KERNEL: AtomicU64 = AtomicU64::new(0);
static USER: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);

// ---- where the threads of a process wait (ADR 0148) -----------------------

/// What a thread is in the middle of, per thread slot: 0 nothing (user
/// mode, or the kernel outside the two below), `n + 1` Linux system call
/// `n`, [`IN_FAULT`] a page fault.
static WHAT: [AtomicU32; crate::thread::MAX_THREADS] = [const { AtomicU32::new(0) }; crate::thread::MAX_THREADS];
/// A page fault: waiting for the memory's lock, the part getting a file's
/// page from the page cache, copying a file's page for a private mapping,
/// anything else.
pub const IN_FAULT_LOCK: u32 = 1000;
pub const IN_FAULT_CACHE: u32 = 1001;
pub const IN_FAULT_COPY: u32 = 1002;
pub const IN_FAULT: u32 = 1003;

/// The running thread starts `what`; returns what it was in before.
pub fn enter(what: u32) -> u32 {
    WHAT[crate::thread::current().slot()].swap(what, Ordering::Relaxed)
}

/// The running thread is back in what it was in before.
pub fn leave(before: u32) {
    WHAT[crate::thread::current().slot()].store(before, Ordering::Relaxed);
}

/// How often the sampler looks at the threads.
const SAMPLE_NS: u64 = 4_000_000;
/// Processes the sampler keeps a table for.
const SAMPLED: usize = 64;
/// Table columns: threads running, runnable (waiting for a CPU), blocked
/// in the kernel outside system calls and page faults (a lock, say), blocked
/// in a page fault in one of the four ways above; then a column for each
/// system call.
const COLUMNS: usize = FIRST_CALL + CALLS;
const FIRST_CALL: usize = 7;

struct Sampled {
    pid: AtomicU32,
    column: [AtomicU32; COLUMNS],
}

static SAMPLES: [Sampled; SAMPLED] =
    [const { Sampled { pid: AtomicU32::new(0), column: [const { AtomicU32::new(0) }; COLUMNS] } }; SAMPLED];

static SAMPLER_WAIT: crate::thread::WaitQueue = crate::thread::WaitQueue::new();

fn start_sampler() {
    if let Ok(id) = crate::thread::spawn_named("profile", sampler_main, 0) {
        let _ = crate::thread::detach(id);
    }
}

fn sampler_main(_: usize) -> usize {
    loop {
        let deadline = crate::time::now_ns().saturating_add(SAMPLE_NS);
        SAMPLER_WAIT.wait_until_deadline(|| false, deadline);
        for t in crate::thread::list() {
            let slot = t.id.slot();
            let Some((pid, tid)) = crate::cputime::ids(slot) else { continue };
            // The first thread of each process: what its program waits for.
            if tid != pid {
                continue;
            }
            let found = SAMPLES.iter().find(|r| r.pid.load(Ordering::Relaxed) == pid);
            let Some(row) = found.or_else(|| {
                SAMPLES.iter().find(|r| r.pid.compare_exchange(0, pid, Ordering::Relaxed, Ordering::Relaxed).is_ok())
            }) else {
                continue;
            };
            let what = WHAT[slot].load(Ordering::Relaxed);
            let column = match t.state {
                crate::thread::State::Running => 0,
                crate::thread::State::Runnable => 1,
                crate::thread::State::Blocked if what == 0 => 2,
                crate::thread::State::Blocked if what >= IN_FAULT_LOCK => 3 + (what - IN_FAULT_LOCK) as usize,
                crate::thread::State::Blocked => FIRST_CALL + (what as usize - 1).min(CALLS - 1),
                crate::thread::State::Exited => continue,
            };
            row.column[column].fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn dump_samples(out: &mut dyn Write) {
    for row in &SAMPLES {
        let pid = row.pid.load(Ordering::Relaxed);
        let total: u32 = row.column.iter().map(|c| c.load(Ordering::Relaxed)).sum();
        if pid == 0 || total < 50 {
            continue;
        }
        let pct = |n: u32| u64::from(n) * 1000 / u64::from(total);
        let get = |i: usize| row.column[i].load(Ordering::Relaxed);
        let _ = write!(
            out,
            "profile: main thread of pid {pid}, {total} samples of {} ms: running {}.{}%, waiting for a CPU {}.{}%, blocked in the kernel {}.{}%, blocked in page faults: for the lock {}.{}%, in the page cache {}.{}%, copying {}.{}%, other {}.{}%",
            SAMPLE_NS / 1_000_000,
            pct(get(0)) / 10,
            pct(get(0)) % 10,
            pct(get(1)) / 10,
            pct(get(1)) % 10,
            pct(get(2)) / 10,
            pct(get(2)) % 10,
            pct(get(3)) / 10,
            pct(get(3)) % 10,
            pct(get(4)) / 10,
            pct(get(4)) % 10,
            pct(get(5)) / 10,
            pct(get(5)) % 10,
            pct(get(6)) / 10,
            pct(get(6)) % 10,
        );
        let mut calls = [(0usize, 0u32); 4];
        for nr in 0..CALLS {
            let n = row.column[FIRST_CALL + nr].load(Ordering::Relaxed);
            if let Some(min) = calls.iter_mut().min_by_key(|e| e.1).filter(|e| e.1 < n) {
                *min = (nr, n);
            }
        }
        calls.sort_unstable_by_key(|e| core::cmp::Reverse(e.1));
        for (nr, n) in calls.iter().filter(|e| e.1 > 0) {
            let name = poly_personality_linux::table::name(*nr as u64).unwrap_or("?");
            let _ = write!(out, ", {name} {}.{}%", pct(*n) / 10, pct(*n) % 10);
        }
        let _ = writeln!(out);
    }
}

/// Kernel argument that times clearing memory at boot.
pub const MEMBENCH_FLAG: &str = "poly.debug.membench";

/// Read the flag (once, at boot).
pub fn init(boot: &BootInfo) {
    ON.store(boot.has_flag(PROFILE_FLAG), Ordering::Relaxed);
    if boot.has_flag(PROFILE_FLAG) {
        start_sampler();
    }
    if boot.has_flag(MEMBENCH_FLAG) {
        membench();
    }
}

/// `poly.debug.membench`: how fast the machine clears memory it has not
/// touched yet (a virtual machine's RAM is backed when first used) and then
/// memory it has: 64 MiB of frames, twice. Printed as MB/s.
fn membench() {
    const FRAMES: u64 = 16384;
    let Some(phys) = crate::frames::allocate_contiguous(FRAMES) else {
        crate::kprintln!("membench: no 64 MiB of contiguous frames");
        return;
    };
    let Some(virt) = crate::arch::phys_range_to_virt(phys, FRAMES * 4096) else { return };
    let bytes = (FRAMES * 4096) as usize;
    let mut speed = [0u64; 3];
    for s in &mut speed {
        let t = crate::time::now_ns();
        // SAFETY: frames just allocated for this test, mapped writable by
        // the direct map, and nothing else uses them.
        unsafe { core::ptr::write_bytes(virt as *mut u8, 0, bytes) };
        let ns = crate::time::now_ns().saturating_sub(t).max(1);
        *s = bytes as u64 * 1000 / ns;
    }
    let _ = crate::frames::free_contiguous(phys, FRAMES);
    crate::kprintln!(
        "membench: clearing 64 MiB: first time {} MB/s, again {} MB/s, and {} MB/s",
        speed[0],
        speed[1],
        speed[2]
    );
}

/// A timer tick interrupted code at `pc` with frame pointer `fp`.
/// `caller` reads the word at an address on the interrupted stack (the
/// frame's return address), `None` if it is not readable.
pub fn sample(user: bool, pc: u64, caller: impl FnOnce() -> Option<u64>) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    if user {
        USER.fetch_add(1, Ordering::Relaxed);
        return;
    }
    KERNEL.fetch_add(1, Ordering::Relaxed);
    PCS.add(pc);
    if let Some(c) = caller() {
        CALLERS.add(c);
    }
}

/// Print the profile (at power-off).
pub fn dump(out: &mut dyn Write) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let (k, u) = (KERNEL.load(Ordering::Relaxed), USER.load(Ordering::Relaxed));
    let _ =
        writeln!(out, "profile: {} ticks: {k} kernel, {u} user, {} dropped", k + u, DROPPED.load(Ordering::Relaxed));
    let mut top = [(0u64, 0u64); 60];
    PCS.top(&mut top);
    for (pc, n) in top.iter().filter(|e| e.1 > 0) {
        let _ = writeln!(out, "profile: pc {pc:#x} {n}");
    }
    dump_samples(out);
    for d in crate::block::disks() {
        let q = d.queue_stats();
        let _ = writeln!(
            out,
            "profile: disk {} {} requests, {} to the device, {} merged, {} split",
            d.name(),
            q.requests,
            q.device_requests,
            q.merged,
            q.split
        );
    }
    let c = crate::fs::cache::cache().stats();
    let _ = writeln!(
        out,
        "profile: page cache {} pages, {} hits, {} misses, {} read from disk, {} written, {} evicted",
        c.pages, c.hits, c.misses, c.filled, c.written, c.evicted
    );
    for (i, name) in EVENTS.iter().enumerate() {
        let n = EVENT_COUNT[i].load(Ordering::Relaxed);
        if n > 0 {
            let _ = writeln!(
                out,
                "profile: event {name} {n} times {} us {} bytes",
                EVENT_NS[i].load(Ordering::Relaxed) / 1000,
                EVENT_BYTES[i].load(Ordering::Relaxed)
            );
        }
    }
    let mut calls = [(0u64, 0u64); 40];
    for (i, ns) in CALL_NS.iter().enumerate() {
        let ns = ns.load(Ordering::Relaxed);
        if let Some(min) = calls.iter_mut().min_by_key(|e| e.1).filter(|e| e.1 < ns) {
            *min = (i as u64, ns);
        }
    }
    calls.sort_unstable_by_key(|e| core::cmp::Reverse(e.1));
    for (nr, ns) in calls.iter().filter(|e| e.1 > 0) {
        let name = poly_personality_linux::table::name(*nr).unwrap_or("?");
        let n = CALL_COUNT[*nr as usize].load(Ordering::Relaxed);
        let _ = writeln!(out, "profile: syscall {name} ({nr}) {n} calls {} us", ns / 1000);
    }
    let mut top = [(0u64, 0u64); 60];
    CALLERS.top(&mut top);
    for (pc, n) in top.iter().filter(|e| e.1 > 0) {
        let _ = writeln!(out, "profile: caller {pc:#x} {n}");
    }
}
