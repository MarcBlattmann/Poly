//! Priorities and scheduling (ADR 0051, ADR 0053, ADR 0113):
//! getpriority(2), setpriority(2), sched_yield(2), sched_getaffinity(2),
//! sched_setaffinity(2) and the scheduling policy calls of sched(7).
//!
//! Every thread has a nice value, a policy, a static priority and the
//! `SCHED_RESET_ON_FORK` flag ([`Sched`], the rules in
//! [`crate::task::sched`]). Poly's scheduler is round-robin without
//! priorities (ADR 0012), so they are recorded and reported but change
//! nothing yet (ADR 0113): real-time policies are accepted under Linux's
//! permission rules and reported back, with no real-time guarantee.
//!
//! The calls name one thread by its id (a process id names the main
//! thread; 0 the caller); an exited thread whose process is a zombie is
//! still seen with the values it had. getpriority(2) and setpriority(2)
//! with `PRIO_PGRP` or `PRIO_USER` cover every thread of every matching
//! process (`PRIO_USER` by each thread's real user id).
//!
//! CPU affinity is real (ADR 0053): the kernel keeps each thread's set of
//! CPUs ([`LinuxKernel::cpu_affinity`]). An affinity mask is a bit set of
//! CPU numbers in memory, `cpusetsize` bytes long, CPU `n` in bit `n % 8`
//! of byte `n / 8` (CPU_SET(3)); the kernel's mask is [`MASK_BYTES`] bytes
//! (64 CPUs), and bits beyond it are ignored. A mask with no CPU the
//! system runs is `EINVAL`. The two calls answer `ESRCH` for a thread id
//! that names no thread, a negative one included (their man page lists no
//! `EINVAL` for the id).

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::fs::rlimit::{RLIMIT_NICE, RLIMIT_RTPRIO};
use crate::kernel::{LinuxKernel, ProcessSelector, Target};
use crate::signal::LinuxProcess;
use crate::table::{Handler, nr};
use crate::task::sched::{self as rules, Changed, Changer, Sched};
use alloc::vec::Vec;

pub use crate::task::sched::{
    NICE_MAX, NICE_MIN, SCHED_BATCH, SCHED_DEADLINE, SCHED_EXT, SCHED_FIFO, SCHED_IDLE, SCHED_OTHER,
    SCHED_RESET_ON_FORK, SCHED_RR,
};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::SCHED_YIELD, sched_yield),
    (nr::GETPRIORITY, getpriority),
    (nr::SETPRIORITY, setpriority),
    (nr::SCHED_SETPARAM, sched_setparam),
    (nr::SCHED_GETPARAM, sched_getparam),
    (nr::SCHED_SETSCHEDULER, sched_setscheduler),
    (nr::SCHED_GETSCHEDULER, sched_getscheduler),
    (nr::SCHED_GET_PRIORITY_MAX, sched_get_priority_max),
    (nr::SCHED_GET_PRIORITY_MIN, sched_get_priority_min),
    (nr::SCHED_RR_GET_INTERVAL, sched_rr_get_interval),
    (nr::SCHED_SETAFFINITY, sched_setaffinity),
    (nr::SCHED_GETAFFINITY, sched_getaffinity),
];

/// `PRIO_*` (`linux/resource.h`).
pub const PRIO_PROCESS: i32 = 0;
pub const PRIO_PGRP: i32 = 1;
pub const PRIO_USER: i32 = 2;

/// sched_yield(): let another thread run on this CPU, if one is waiting;
/// always succeeds.
pub fn sched_yield(k: &mut dyn LinuxKernel, _a: &Args) -> SysResult {
    k.yield_cpu();
    Ok(0)
}

/// Where one thread is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Addr {
    /// By thread id (0: the caller), a zombie's main thread included.
    Thread(i32),
    /// Thread `tid` of process `pid`, found by listing the process.
    In(i32, i32),
}

/// Run `f` on the process of the thread `addr` names and the thread's id;
/// `ESRCH` if there is no such thread or `f` says so (`None`).
fn with_thread<R>(
    k: &mut dyn LinuxKernel,
    addr: Addr,
    mut f: impl FnMut(&mut LinuxProcess, i32) -> Option<R>,
) -> Result<R, Errno> {
    let mut out = None;
    match addr {
        Addr::Thread(0) => {
            let tid = k.tid();
            k.with_process(Target::Current, &mut |p, _| out = f(p, tid))?;
        }
        Addr::Thread(tid) if tid < 0 => return Err(Errno::ESRCH),
        Addr::Thread(tid) => {
            if k.with_process(Target::Thread(tid), &mut |p, _| out = f(p, tid)).is_err() {
                // No live thread has the id; a zombie process whose main
                // thread it was is still seen.
                k.with_process(Target::Process(tid), &mut |p, _| out = f(p, tid))?;
            }
        }
        Addr::In(pid, tid) => k.with_process(Target::Process(pid), &mut |p, _| out = f(p, tid))?,
    }
    out.ok_or(Errno::ESRCH)
}

/// Thread `tid` of `p` as the permission rules see it.
fn changed(p: &LinuxProcess, tid: i32) -> Option<Changed> {
    let sched = p.sched_of(tid)?;
    let creds = if p.has_thread(tid) { &p.thread(tid).creds } else { p.process_creds(tid)? };
    let limit = |r| p.limits.get(r).map_or(0, |l| l.cur);
    Some(Changed {
        uid: creds.uid,
        euid: creds.euid,
        permitted: creds.caps.permitted,
        sched,
        rtprio_limit: limit(RLIMIT_RTPRIO),
        nice_limit: limit(RLIMIT_NICE),
    })
}

/// Give thread `tid` of `p` new attributes (nothing for a zombie).
fn store(p: &mut LinuxProcess, tid: i32, sched: Sched) {
    if p.has_thread(tid) {
        p.thread_mut(tid).sched = sched;
    }
}

/// The caller, as the rules see it. Priorities are the machine's:
/// `CAP_SYS_NICE` in the initial user namespace (ADR 0054).
fn changer(k: &mut dyn LinuxKernel) -> Changer {
    let me = cred::current(k);
    Changer { privileged: me.capable_initial(cap::SYS_NICE), euid: me.euid, permitted: me.caps.permitted }
}

/// The threads `which` / `who` name (getpriority(2): 0 is the caller,
/// its group or its real user id): `EINVAL` for an unknown `which`.
fn targets(k: &mut dyn LinuxKernel, which: i32, who: i32) -> Result<Vec<Addr>, Errno> {
    let (pids, uid) = match which {
        PRIO_PROCESS => return Ok(alloc::vec![Addr::Thread(who)]),
        PRIO_PGRP => (k.process_ids(ProcessSelector::Group(if who == 0 { k.pgid() } else { who })), None),
        PRIO_USER => {
            let uid = if who == 0 { cred::current(k).uid } else { who as u32 };
            (k.process_ids(ProcessSelector::All), Some(uid))
        }
        _ => return Err(Errno::EINVAL),
    };
    let mut found = Vec::new();
    for pid in pids {
        let _ = k.with_process(Target::Process(pid), &mut |p, _| {
            let mut tids: Vec<i32> = p.thread_ids().collect();
            if tids.is_empty() {
                tids.push(pid);
            }
            for tid in tids {
                let matches = match (uid, changed(p, tid)) {
                    (_, None) => false,
                    (None, Some(_)) => true,
                    (Some(uid), Some(t)) => t.uid == uid,
                };
                if matches {
                    found.push(Addr::In(pid, tid));
                }
            }
        });
    }
    Ok(found)
}

/// getpriority(which, who): the highest priority (lowest nice value) of
/// the threads named, as the system call returns it: `20 - nice`, 1 to
/// 40 (getpriority(2) NOTES; the C library turns it back into a nice
/// value). `ESRCH` if none matched, `EINVAL` for an unknown `which`.
pub fn getpriority(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let mut best: Option<i32> = None;
    for addr in targets(k, a.int(0), a.int(1))? {
        if let Ok(nice) = with_thread(k, addr, |p, tid| p.sched_of(tid).map(|s| s.nice)) {
            best = Some(best.map_or(nice, |b| b.min(nice)));
        }
    }
    let nice = best.ok_or(Errno::ESRCH)?;
    Ok((20 - nice) as u64)
}

/// setpriority(which, who, prio): every thread named gets nice value
/// `prio` (clamped to -20..19). Per thread `EPERM` or `EACCES`
/// ([`rules::check_nice_change`]); the call fails with the error of a
/// thread that could not be changed (the others are), `ESRCH` if none
/// matched.
pub fn setpriority(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let nice = a.int(2).clamp(NICE_MIN, NICE_MAX);
    let found = targets(k, a.int(0), a.int(1))?;
    let who = changer(k);
    let mut matched = false;
    let mut result = Ok(0);
    for addr in found {
        let r = with_thread(k, addr, |p, tid| {
            let t = changed(p, tid)?;
            Some(rules::check_nice_change(&who, &t, nice).map(|()| store(p, tid, Sched { nice, ..t.sched })))
        });
        match r {
            Err(_) => {}
            Ok(Ok(())) => matched = true,
            Ok(Err(e)) => {
                matched = true;
                result = Err(e);
            }
        }
    }
    if !matched {
        return Err(Errno::ESRCH);
    }
    result
}

/// A thread id argument of the sched_* calls: `EINVAL` if negative.
fn thread_id(a: &Args, i: usize) -> Result<Addr, Errno> {
    match a.int(i) {
        id if id < 0 => Err(Errno::EINVAL),
        id => Ok(Addr::Thread(id)),
    }
}

/// The `struct sched_param` (`int sched_priority`) at `a.ptr(i)`, after
/// the thread id in argument 0: `EINVAL` for a NULL pointer or a negative
/// id, then `EFAULT`.
fn param_in(k: &dyn LinuxKernel, a: &Args, i: usize) -> Result<(Addr, i32), Errno> {
    if a.ptr(i).is_null() {
        return Err(Errno::EINVAL);
    }
    let addr = thread_id(a, 0)?;
    let mut b = [0u8; 4];
    k.copy_from_user(a.ptr(i), &mut b)?;
    Ok((addr, i32::from_ne_bytes(b)))
}

/// Set the policy (`None`: the thread's own, flag kept) and priority of
/// the thread at `addr`: `ESRCH`, then `EINVAL` (policy, priority), then
/// `EPERM`.
fn set_sched(k: &mut dyn LinuxKernel, addr: Addr, policy: Option<i32>, priority: i32) -> SysResult {
    let who = changer(k);
    with_thread(k, addr, |p, tid| {
        let t = changed(p, tid)?;
        let set = || -> Result<Sched, Errno> {
            let (policy, reset) = match policy {
                Some(arg) => rules::parse_policy(arg)?,
                None => (t.sched.policy, t.sched.reset_on_fork),
            };
            rules::check_priority(policy, priority)?;
            rules::check_policy_change(&who, &t, policy, priority, reset)?;
            Ok(Sched { policy, priority, reset_on_fork: reset, ..t.sched })
        };
        Some(set().map(|s| store(p, tid, s)))
    })?
    .map(|()| 0)
}

/// sched_setscheduler(pid, policy, param): in order `EINVAL` (NULL
/// `param`, negative `pid`), `EFAULT`, `ESRCH`, `EINVAL` (policy,
/// priority), `EPERM` ([`rules::check_policy_change`]).
pub fn sched_setscheduler(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (addr, priority) = param_in(k, a, 2)?;
    set_sched(k, addr, Some(a.int(1)), priority)
}

/// sched_setparam(pid, param): sched_setscheduler with the thread's own
/// policy and `SCHED_RESET_ON_FORK` flag.
pub fn sched_setparam(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (addr, priority) = param_in(k, a, 1)?;
    set_sched(k, addr, None, priority)
}

/// sched_getscheduler(pid): the policy, with `SCHED_RESET_ON_FORK` or-ed
/// in while it is set.
pub fn sched_getscheduler(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = thread_id(a, 0)?;
    let s = with_thread(k, addr, |p, tid| p.sched_of(tid))?;
    Ok(s.reported_policy() as u64)
}

/// sched_getparam(pid, param): the static priority (0 under the normal
/// policies). `EINVAL` for a NULL `param` or a negative `pid`, then
/// `ESRCH`, then `EFAULT`.
pub fn sched_getparam(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ptr(1).is_null() {
        return Err(Errno::EINVAL);
    }
    let addr = thread_id(a, 0)?;
    let s = with_thread(k, addr, |p, tid| p.sched_of(tid))?;
    k.copy_to_user(a.ptr(1), &s.priority.to_ne_bytes())?;
    Ok(0)
}

/// sched_get_priority_max(policy): 99 for the real-time policies, 0 for
/// the others (sched(7)), `EINVAL` for unknown ones.
pub fn sched_get_priority_max(_k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    rules::priority_range(a.int(0)).map(|(_, max)| max as u64)
}

/// sched_get_priority_min(policy): 1 for the real-time policies, 0 for
/// the others.
pub fn sched_get_priority_min(_k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    rules::priority_range(a.int(0)).map(|(min, _)| min as u64)
}

/// sched_rr_get_interval(pid, tp): the thread's quantum as a `struct
/// timespec` ([`rules::rr_interval_ns`]). `EINVAL` (negative `pid`),
/// `ESRCH`, then `EFAULT` (a NULL `tp` too).
pub fn sched_rr_get_interval(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let addr = thread_id(a, 0)?;
    let s = with_thread(k, addr, |p, tid| p.sched_of(tid))?;
    if a.ptr(1).is_null() {
        return Err(Errno::EFAULT);
    }
    let ns = rules::rr_interval_ns(s.policy);
    let mut ts = [0u8; 16];
    ts[..8].copy_from_slice(&(ns / 1_000_000_000).to_ne_bytes());
    ts[8..].copy_from_slice(&(ns % 1_000_000_000).to_ne_bytes());
    k.copy_to_user(a.ptr(1), &ts)?;
    Ok(0)
}

/// The size of the kernel's CPU mask: one `unsigned long`, 64 CPUs.
pub const MASK_BYTES: usize = 8;

/// The thread id an affinity call names: 0 is the caller; `ESRCH` for a
/// negative one.
fn affinity_target(k: &dyn LinuxKernel, pid: i32) -> Result<i32, Errno> {
    match pid {
        0 => Ok(k.tid()),
        p if p < 0 => Err(Errno::ESRCH),
        p => Ok(p),
    }
}

/// sched_getaffinity(pid, cpusetsize, mask): the raw call returns the
/// number of bytes it wrote (sched_setaffinity(2) NOTES); `EINVAL` if
/// `cpusetsize` is smaller than the kernel's mask or not a multiple of the
/// size of a long.
pub fn sched_getaffinity(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    // `cpusetsize` is an `unsigned int` in the kernel's call.
    let size = a.uint(1) as usize;
    if size < MASK_BYTES || !size.is_multiple_of(8) {
        return Err(Errno::EINVAL);
    }
    let tid = affinity_target(k, a.int(0))?;
    let mask = k.cpu_affinity(tid)?;
    k.copy_to_user(a.ptr(2), &mask.to_le_bytes())?;
    Ok(MASK_BYTES as u64)
}

/// sched_setaffinity(pid, cpusetsize, mask): the thread runs only on the
/// mask's CPUs from now on (a running thread moves at once if it is the
/// caller, else at its CPU's next tick); `EINVAL` if the mask names no CPU
/// the system runs.
///
/// The mask is read first (`EFAULT` even for a thread that does not
/// exist), then the thread looked up (`ESRCH`, also for an empty mask),
/// then the mask judged.
pub fn sched_setaffinity(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let len = (a.uint(1) as usize).min(MASK_BYTES);
    let mut bytes = [0u8; MASK_BYTES];
    if len > 0 {
        k.copy_from_user(a.ptr(2), &mut bytes[..len])?;
    }
    let tid = affinity_target(k, a.int(0))?;
    let mask = u64::from_le_bytes(bytes);
    if mask == 0 {
        k.cpu_affinity(tid)?;
        return Err(Errno::EINVAL);
    }
    k.set_cpu_affinity(tid, mask)?;
    Ok(0)
}
