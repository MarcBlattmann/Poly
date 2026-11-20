//! `poly.debug.timeline[=<MiB>]` (ADR 0247): a ring of records of what every CPU ran, who woke
//! whom, the system calls, keys and frames ([`poly_timeline`]), for finding where the time of one
//! key press goes. Off unless the flag is given; then each record is a handful of stores.
//!
//! The ring is one physically contiguous block (32 MiB unless a size is given). Its physical
//! address is printed at boot (`timeline: ring at phys 0x… bytes …`); the host copies it out of
//! the guest's memory with QEMU's `pmemsave` while it is frozen. The Pause key freezes it (the
//! key does not reach any program) and thaws it again. Names that the records do not carry are
//! printed as they become known: kernel threads, the command lines of NT processes and the
//! names Windows programs give their threads (`timeline: name …`, `timeline: proc …`).
//! `tools/timeline/timeline.py` reads both.

use crate::boot_info::BootInfo;
use crate::sync::SetOnce;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_timeline::{Kind, Meta, Record, Ring};

/// Kernel argument that turns the timeline on; `=<MiB>` sizes the ring.
pub const FLAG: &str = "poly.debug.timeline";
const DEFAULT_MIB: u64 = 32;

static ON: AtomicBool = AtomicBool::new(false);
static RING: SetOnce<Ring<'static>> = SetOnce::new();

/// The thread each CPU runs, as records name it (set at every switch).
static CURRENT: [AtomicU64; crate::thread::MAX_CPUS] = [const { AtomicU64::new(0) }; crate::thread::MAX_CPUS];

/// The evdev code of the Pause key (`linux/input-event-codes.h`).
const KEY_PAUSE: u16 = 119;
/// `EV_KEY` (`linux/input-event-codes.h`).
const EV_KEY: u16 = 1;

/// Whether the timeline is recording or frozen (the flag was given and the ring exists).
#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn init(boot: &BootInfo) {
    let mib = match boot.arg(FLAG) {
        Some(v) => v.parse::<u64>().unwrap_or(DEFAULT_MIB).clamp(1, 1024),
        None if boot.has_flag(FLAG) => DEFAULT_MIB,
        None => return,
    };
    let bytes = mib << 20;
    let Some(phys) = crate::frames::allocate_contiguous(bytes / 4096) else {
        crate::kprintln!("timeline: no {mib} MiB of contiguous memory");
        return;
    };
    let Some(virt) = crate::arch::phys_range_to_virt(phys, bytes) else { return };
    // SAFETY: frames just allocated for the ring and never freed, mapped writable by the
    // direct map; nothing else refers to them.
    let words = unsafe { core::slice::from_raw_parts_mut(virt as *mut u64, (bytes / 8) as usize) };
    let ring = Ring::new(words);
    let capacity = ring.capacity();
    if RING.set(ring).is_err() {
        return;
    }
    ON.store(true, Ordering::SeqCst);
    crate::kprintln!("timeline: ring at phys {phys:#x} bytes {bytes} records {capacity} (Pause freezes and thaws it)");
    for t in crate::thread::list() {
        name_kernel_thread(t.id.slot(), t.name);
    }
    crate::nt::name_timeline_queues();
    // The system calls' names, for the reader.
    for &(_, number, name) in poly_personality_nt::services::all() {
        crate::kprintln!("timeline: sys {} {number:#x} {name}", poly_timeline::NT);
    }
    for index in 0..0x100u64 {
        if let Some(name) = poly_personality_nt::win32k::service_name(index) {
            crate::kprintln!("timeline: sys {} {:#x} {name}", poly_timeline::NT, 0x1000 | index);
        }
    }
    for &(tag, name) in poly_win32k::proto::Request::TAGS {
        crate::kprintln!("timeline: user32 {tag} {name}");
    }
    for &(number, name) in poly_personality_linux::table::SYSCALLS.iter() {
        crate::kprintln!("timeline: sys {} {number:#x} {name}", poly_timeline::LINUX);
    }
}

/// Print a kernel thread's name (at boot for those that exist, then as each is spawned).
pub fn name_kernel_thread(slot: usize, name: &str) {
    if on() {
        crate::kprintln!("timeline: name {:#x} {name}", poly_timeline::ident(poly_timeline::KERNEL_PID, slot as u32));
    }
}

/// Print a user thread's name (`SetThreadDescription`, `prctl(PR_SET_NAME)`).
pub fn name_thread(pid: u32, tid: u32, name: &str) {
    if on() {
        crate::kprintln!("timeline: name {:#x} {}", poly_timeline::ident(pid, tid), name.trim());
    }
}

/// Print the name of a wait queue (a lock's) that [`blocking_on`] records name by address.
pub fn name_queue(address: u64, name: &str) {
    if on() {
        crate::kprintln!("timeline: queue {address:#x} {name}");
    }
}

/// A timer tick interrupted the current thread at `rip` (interrupt context): in user mode (`b`
/// 0), or in the kernel (a system call, a page fault, the idle loop ...), where `b` is the
/// interrupted function's caller (its frame's return address, 1 when unknown) and the reader
/// looks both up in the kernel's symbols.
pub fn sample(rip: u64, user: bool, caller: impl FnOnce() -> Option<u64>) {
    if on() {
        let b = if user { 0 } else { caller().filter(|&c| c > 1).unwrap_or(1) };
        push(Kind::Sample, current(), rip, b);
    }
}

/// Print where an image (executable or DLL) of process `pid` is mapped.
pub fn name_module(pid: u32, base: u64, size: u64, path: &str) {
    if on() {
        crate::kprintln!(
            "timeline: module {pid} {base:#x} {size:#x} {}",
            path.rsplit(['\\', '/']).next().unwrap_or(path)
        );
    }
}

/// The current thread is about to block on the wait queue at `address`.
pub fn blocking_on(address: u64) {
    if on() {
        push(Kind::Block, current(), address, 0);
    }
}

/// Print a process's command line.
pub fn name_process(pid: u32, line: &str) {
    if on() {
        crate::kprintln!("timeline: proc {pid} {}", line.trim());
    }
}

fn push(kind: Kind, ident: u64, a: u64, b: u64) {
    let Some(ring) = RING.get() else { return };
    let cpu = crate::thread::this_cpu();
    let flags = if crate::arch::irq::in_interrupt() { Meta::IN_INTERRUPT } else { 0 };
    ring.push(&Record { t_ns: crate::time::now_ns(), ident, a, b, kind, cpu: cpu as u8, flags });
}

/// The thread running on this CPU, as records name it.
#[inline]
pub fn current() -> u64 {
    CURRENT[crate::thread::this_cpu()].load(Ordering::Relaxed)
}

/// This CPU switched from thread `from` (which became `left`) to `to`.
pub fn switched(from: u64, left: poly_timeline::Left, to: u64) {
    CURRENT[crate::thread::this_cpu()].store(to, Ordering::Relaxed);
    if on() {
        push(Kind::Switch, to, from, left as u64);
    }
}

/// Thread `woken` became runnable.
pub fn woke(woken: u64) {
    if on() {
        push(Kind::Wake, woken, current(), 0);
    }
}

/// The current thread enters (`enter`) or leaves system call `call` ([`poly_timeline::call`]).
pub fn syscall(enter: bool, call: u64, value: u64) {
    if on() {
        push(if enter { Kind::SysEnter } else { Kind::SysExit }, current(), call, value);
    }
}

/// The display sent a frame.
pub fn frame(bytes: u64, rects: u64) {
    if on() {
        push(Kind::Frame, current(), bytes, rects);
    }
}

/// Input events arrived: keys are recorded, and the Pause key freezes or thaws the ring.
/// Returns whether `events` holds a Pause key event, which the caller keeps from programs.
pub fn input(events: &[(u16, u16, i32)]) -> bool {
    if !on() {
        return false;
    }
    let mut pause = false;
    for &(ty, code, value) in events {
        if ty != EV_KEY {
            continue;
        }
        // TEMP-AB (prezeroed-pages, removed before the PR): Scroll Lock switches the pool off/on.
        if code == 70 {
            pause = true;
            if value == 1 {
                let on = !crate::zeropool::TEMP_AB.load(Ordering::Relaxed);
                crate::zeropool::TEMP_AB.store(on, Ordering::Relaxed);
                crate::kprintln!("TEMP-AB pool={on}");
                push(Kind::Key, current(), 70, 1);
            }
            continue;
        }
        if code == KEY_PAUSE {
            pause = true;
            if value == 1
                && let Some(ring) = RING.get()
            {
                let cpu = crate::thread::this_cpu() as u8;
                let frozen = ring.frozen();
                ring.set_frozen(!frozen, crate::time::now_ns(), cpu);
                crate::kprintln!("timeline: {} at record {}", if frozen { "thawed" } else { "frozen" }, ring.written());
            }
            continue;
        }
        push(Kind::Key, current(), u64::from(code), value as u32 as u64);
    }
    pause
}
