//! Out of memory (ADR 0250): what happens when user memory keeps growing.
//!
//! User pages and page cache pages stop at the kernel's reserve (`frames::allocate_user_frame`):
//! a program that asks for more gets a failed allocation (a page fault it cannot resolve, a failed
//! commit), and the kernel itself goes on. When that happens this thread runs:
//!
//! 1. It drops clean page cache pages, which is free memory in all but name.
//! 2. If free frames are still at the low watermark, it ends the process with the highest score,
//!    except the desktop's own programs (native ones) and the first process
//!    (`poly_mem::pressure::choose_victim`): a Linux or macOS process as by `SIGKILL`, a Windows
//!    one as by `NtTerminateProcess` with `STATUS_NO_MEMORY`. It logs the process and waits for it
//!    to end (at most 5 s) before it ends another.
//!
//! The score is the process's memory plus its adjustment in thousandths of all memory, as Linux
//! scores (ADR 0250 addendum): a Linux process's own `oom_score_adj` (Chromium writes 300 and
//! more for its renderers), a Windows process's from its relations (`nt::oom_score_adjs`: a child
//! in a job its parent made). So a browser's renderers go before the browser, and the desktop
//! keeps working when a browser takes all memory.

use crate::process::{self, ExitStatus};
use crate::thread::{self, WaitQueue};
use crate::{frames, kprintln, time};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_mem::pressure::{Candidate, choose_victim};

static WAKE: WaitQueue = WaitQueue::new();
static WOKEN: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// `poly.mm.stats`: a line about memory every [`STATS_INTERVAL_NS`].
static STATS: AtomicBool = AtomicBool::new(false);
const STATS_INTERVAL_NS: u64 = 10 * time::NANOS_PER_SEC;
/// Processes the policy has ended since boot.
static KILLS: AtomicU64 = AtomicU64::new(0);

/// `SIGKILL`.
const SIGKILL: u32 = 9;
/// How long the policy waits for a process it ended before it may end another.
const WAIT_FOR_END_NS: u64 = 5 * time::NANOS_PER_SEC;

/// A user or page cache allocation was refused: run the policy soon. Safe in any context that
/// may wake a thread.
pub fn wake() {
    WOKEN.store(true, Ordering::SeqCst);
    if STARTED.load(Ordering::Relaxed) {
        WAKE.wake_all();
    }
}

/// Processes the policy has ended since boot.
pub fn kills() -> u64 {
    KILLS.load(Ordering::Relaxed)
}

/// Start the policy's thread. Once, after the file systems (the page cache) are up.
pub fn init(boot: &crate::boot_info::BootInfo) {
    STATS.store(boot.has_flag("poly.mm.stats"), Ordering::Relaxed);
    match thread::spawn_named("oom", oom_main, 0) {
        Ok(id) => {
            let _ = thread::detach(id);
            STARTED.store(true, Ordering::SeqCst);
            kprintln!(
                "mm: {} frames ({} MiB) kept for the kernel; user memory stops there (ADR 0250)",
                frames::reserve(),
                (frames::reserve() * 4096) >> 20
            );
        }
        Err(_) => kprintln!("mm: no thread for the out-of-memory policy"),
    }
}

fn free_frames() -> u64 {
    frames::FRAMES.lock().as_ref().map_or(0, |f| f.free_frames())
}

fn oom_main(_: usize) -> usize {
    let mut next_stats = time::now_ns().saturating_add(STATS_INTERVAL_NS);
    loop {
        if STATS.load(Ordering::Relaxed) {
            WAKE.wait_until_deadline(|| WOKEN.load(Ordering::SeqCst), next_stats);
            if time::now_ns() >= next_stats {
                next_stats = time::now_ns().saturating_add(STATS_INTERVAL_NS);
                log_stats();
            }
        } else {
            WAKE.wait_until(|| WOKEN.load(Ordering::SeqCst));
        }
        if WOKEN.swap(false, Ordering::SeqCst) {
            relieve();
        }
    }
}

/// `poly.mm.stats`: where memory is.
fn log_stats() {
    let mib = |frames: u64| (frames * 4096) >> 20;
    let (commit, limit) = crate::nt::commit().unwrap_or((0, 0));
    kprintln!(
        "mm: free {} MiB, user {} MiB, cache {} MiB, heap {} MiB, nt commit {} of {} MiB, refused {}, ended {}",
        mib(free_frames()),
        mib(crate::user::user_frames()),
        mib(crate::fs::cache::cache().stats().pages as u64),
        mib(crate::heap::stats().pages_held as u64),
        commit >> 20,
        if limit == u64::MAX { 0 } else { limit >> 20 },
        frames::refused(),
        kills()
    );
}

/// One run of the policy after a refusal.
fn relieve() {
    let low = frames::low_watermark();
    if free_frames() > low {
        return;
    }
    // Clean page cache pages first.
    let cache = crate::fs::cache::cache();
    let pages = cache.stats().pages;
    if pages > 0 {
        cache.shrink(pages / 4);
        if free_frames() > low {
            return;
        }
    }
    if TESTING.load(Ordering::SeqCst) {
        return;
    }
    // One victim at a time (as Linux's killer does not choose another while one is dying): a
    // process that is ending and still holds memory is giving it back, so wait for that (up to
    // the time a victim is given) before ending another. Without this, the policy that ran again
    // right after ending a large child found it "ending" and took the next one, its parent.
    let deadline = time::now_ns().saturating_add(WAIT_FOR_END_NS);
    while time::now_ns() < deadline
        && free_frames() <= low
        && candidates().iter().any(|c| c.ending && !c.protected && c.resident > 0)
    {
        thread::sleep(core::time::Duration::from_millis(50));
    }
    if free_frames() > low {
        return;
    }
    let all = candidates();
    let Some(pid) = choose_victim(&all, frames::total_frames()) else {
        kprintln!("mm: out of memory, and no process the policy may end holds any");
        return;
    };
    let Some(victim) = process::find(pid) else { return };
    let (resident, adj) = all.iter().find(|c| c.pid == pid).map_or((0, 0), |c| (c.resident, c.adj));
    let line = crate::fs::proc::cmdline_of(pid);
    let name = match crate::nt::image_of(pid) {
        Some(image) => image,
        None => String::from_utf8_lossy(line.split(|&b| b == 0).next().unwrap_or(&[])).into_owned(),
    };
    kprintln!(
        "mm: out of memory ({} frames free): ending process {pid} ({}) holding {} MiB, score adjustment {adj}",
        free_frames(),
        name,
        (resident * 4096) >> 20
    );
    KILLS.fetch_add(1, Ordering::Relaxed);
    match victim.personality {
        poly_abi::Personality::Nt => {
            let _ = crate::nt::terminate_for_memory(pid);
        }
        _ => {
            victim.group_exit(ExitStatus::Signaled { signal: SIGKILL, core: false });
        }
    }
    let _ = victim.wait_until(time::now_ns().saturating_add(WAIT_FOR_END_NS));
    // Still short: run again.
    if free_frames() <= frames::policy_reserve() {
        WOKEN.store(true, Ordering::SeqCst);
    }
}

/// Whether the policy never ends `p`, and its memory may go further into the kernel's reserve:
/// Poly's native programs (the desktop, the compositor, the shell) and the first process.
pub fn is_protected(p: &process::Process) -> bool {
    p.personality == poly_abi::Personality::Native || p.pid <= 1
}

/// A page fault of process `p` found no frame: wake the policy and try again while it frees
/// memory (`retry` resolves the fault once more), for up to 5 s for the desktop's programs and 2 s
/// for others, instead of ending the thread at once (as Linux has the allocating task wait for its
/// out-of-memory killer). A process that is being ended stops waiting. Returns the last result.
pub fn wait_for_memory<E>(
    p: &process::Process,
    mut retry: impl FnMut() -> Result<(), E>,
    out_of_memory: impl Fn(&E) -> bool,
) -> Result<(), E> {
    let mut result = retry();
    let patience = if is_protected(p) { WAIT_FOR_END_NS } else { WAIT_FOR_END_NS * 2 / 5 };
    let deadline = time::now_ns().saturating_add(patience);
    while result.as_ref().is_err_and(&out_of_memory) && !p.is_exiting() && time::now_ns() < deadline {
        wake();
        thread::sleep(core::time::Duration::from_millis(50));
        result = retry();
    }
    result
}

/// The processes the policy might end, with what they hold (a Windows process its own pages,
/// `nt::own_pages`; any other its resident ones) and their score adjustments (a Linux process its
/// `oom_score_adj`, a Windows process `nt::oom_score_adjs`).
fn candidates() -> Vec<Candidate> {
    let nt = crate::nt::own_pages();
    let nt_adj = crate::nt::oom_score_adjs();
    process::all()
        .into_iter()
        .map(|p| Candidate {
            adj: match &p.linux {
                Some(l) => l.with(|s| i32::from(s.oom_score_adj)),
                None => nt_adj.iter().find(|&&(pid, _)| pid == p.pid).map_or(0, |&(_, a)| a),
            },
            pid: p.pid,
            resident: nt
                .iter()
                .find(|&&(pid, _)| pid == p.pid)
                .map(|&(_, own)| own)
                .unwrap_or_else(|| crate::fs::proc::memory_of(p.pid).map_or(0, |m| m.resident_pages())),
            protected: is_protected(&p),
            ending: p.is_exiting() || p.has_ended(),
        })
        .collect()
}

/// While set, [`relieve`] ends no process: the self-tests below take memory on purpose.
static TESTING: AtomicBool = AtomicBool::new(false);

crate::kernel_test! {
    /// User memory stops at the kernel's reserve (ADR 0250): user frames are refused once only the
    /// reserve is free, the kernel heap still allocates then, and once they are freed user memory
    /// can be had again.
    ///
    /// Other CPUs free frames while it runs (the page cache gives pages back once free memory is
    /// low, `fs::cache::kick`; the policy's thread shrinks it; programs of the boot end), so every
    /// check reads the free count and asks for a frame under one hold of the frame lock: free
    /// memory cannot move between the two.
    fn user_memory_stops_at_the_reserve() -> TestResult {
        TESTING.store(true, Ordering::SeqCst);
        let mut held: Vec<u64> = Vec::new();
        let result = (|| {
            let floor = frames::user_floor();
            held.try_reserve(free_frames() as usize).map_err(|_| "no room to note the frames")?;
            // Most of the way with the allocator everyone uses (and its refusal, which wakes the
            // page cache and the policy)...
            while let Some(phys) = frames::allocate_user_frame() {
                held.push(phys);
            }
            // ...and to the stop under the lock, taking what the others gave back meanwhile. The
            // vector has room for what is taken under the lock (the heap is not asked then).
            let at_stop = loop {
                if held.len() == held.capacity() {
                    held.try_reserve(4096).map_err(|_| "no room to note the frames")?;
                }
                let mut guard = frames::FRAMES.lock();
                let f = guard.as_mut().ok_or("no frame allocator")?;
                let mut stopped = false;
                while held.len() < held.capacity() {
                    match frames::take_user_frame(f, floor) {
                        Some(phys) => held.push(phys),
                        None => {
                            stopped = true;
                            break;
                        }
                    }
                }
                if stopped {
                    break f.free_frames();
                }
            };
            if held.is_empty() || at_stop > floor {
                return Err("user frames did not stop at the reserve");
            }
            // The kernel may still allocate: a 1 MiB buffer and a thousand small ones.
            let mut big: Vec<u8> = Vec::new();
            if big.try_reserve_exact(1 << 20).is_err() {
                return Err("the kernel heap failed at the reserve");
            }
            let small: Vec<alloc::boxed::Box<[u8; 64]>> = (0..1000).map(|_| alloc::boxed::Box::new([0u8; 64])).collect();
            // A user frame is refused while no more than the reserve is free (and handed out if
            // the others have given back more by now).
            let (free, got) = {
                let mut guard = frames::FRAMES.lock();
                let f = guard.as_mut().ok_or("no frame allocator")?;
                let free = f.free_frames();
                let got = frames::take_user_frame(f, floor);
                if let Some(phys) = got {
                    f.free(phys, 0).map_err(|_| "a user frame could not be freed")?;
                }
                (free, got)
            };
            drop(small);
            drop(big);
            if got.is_some() != (free > floor) {
                return Err("a user frame was handed out at the reserve, or refused above it");
            }
            Ok(())
        })();
        if let Some(f) = frames::FRAMES.lock().as_mut() {
            for &phys in &held {
                let _ = f.free(phys, 0);
            }
        }
        held.clear();
        let result = result.and_then(|()| match frames::allocate_user_frame() {
            Some(phys) => {
                if let Some(f) = frames::FRAMES.lock().as_mut() {
                    let _ = f.free(phys, 0);
                }
                Ok(())
            }
            None => Err("user memory could not be had again after it was freed"),
        });
        TESTING.store(false, Ordering::SeqCst);
        result
    }
}

crate::kernel_test! {
    /// A storm of threads (ADR 0250): spawning until the table or the stacks run out fails cleanly
    /// (`TableFull` or `NoMemory`), every thread that started ends and is joined, and spawning works
    /// again afterwards.
    fn a_storm_of_threads_fails_cleanly_and_recovers() -> TestResult {
        static RELEASE: AtomicBool = AtomicBool::new(false);
        static GATE: WaitQueue = WaitQueue::new();
        fn waiter(_: usize) -> usize {
            GATE.wait_until(|| RELEASE.load(Ordering::SeqCst));
            7
        }
        RELEASE.store(false, Ordering::SeqCst);
        let mut ids = Vec::new();
        let stop = loop {
            match thread::spawn_named("storm", waiter, 0) {
                Ok(id) => ids.push(id),
                Err(e) => break e,
            }
            if ids.len() > crate::thread::MAX_THREADS {
                break thread::SpawnError::TableFull;
            }
        };
        RELEASE.store(true, Ordering::SeqCst);
        GATE.wake_all();
        let mut joined = 0;
        for id in ids.iter().copied() {
            if thread::join(id) == Ok(7) {
                joined += 1;
            }
        }
        let after = thread::spawn_named("storm", waiter, 0).map_err(|_| "spawning fails after the storm")?;
        let _ = thread::join(after);
        if !matches!(stop, thread::SpawnError::TableFull | thread::SpawnError::NoMemory) || ids.is_empty() {
            return Err("the storm stopped for another reason");
        }
        if joined != ids.len() {
            return Err("not every thread of the storm ended");
        }
        Ok(())
    }
}
