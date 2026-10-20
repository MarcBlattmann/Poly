//! Priorities and scheduling (ADR 0051): getpriority(2), setpriority(2),
//! sched_yield(2), sched_getaffinity(2), sched_setaffinity(2) and the
//! scheduling policy calls of sched(7).
//!
//! Poly's scheduler is round-robin without priorities (ADR 0012), so the
//! nice value is kept per process ([`LinuxProcess::nice`], inherited by
//! fork) and reported, but changes nothing yet; the policy is always
//! `SCHED_OTHER`. Real-time policies are refused (`EPERM`): accepting them
//! would promise behaviour Poly does not have.
//!
//! [`LinuxProcess::nice`]: crate::LinuxProcess::nice

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, ProcessSelector, Target};
use crate::table::{Handler, nr};
use alloc::vec::Vec;

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
    (nr::SCHED_SETAFFINITY, sched_setaffinity),
    (nr::SCHED_GETAFFINITY, sched_getaffinity),
];

/// `PRIO_*` (`linux/resource.h`).
pub const PRIO_PROCESS: i32 = 0;
pub const PRIO_PGRP: i32 = 1;
pub const PRIO_USER: i32 = 2;

/// Nice values: getpriority(2), "a value in the range -20 to 19".
pub const NICE_MIN: i32 = -20;
pub const NICE_MAX: i32 = 19;

/// `SCHED_*` (`linux/sched.h`).
pub const SCHED_OTHER: i32 = 0;
pub const SCHED_FIFO: i32 = 1;
pub const SCHED_RR: i32 = 2;
pub const SCHED_BATCH: i32 = 3;
pub const SCHED_IDLE: i32 = 5;
pub const SCHED_DEADLINE: i32 = 6;
/// `SCHED_RESET_ON_FORK`, or-ed into the policy.
pub const SCHED_RESET_ON_FORK: i32 = 0x4000_0000;

/// The CPUs Poly schedules on: one, CPU 0 (`sched_getaffinity` reports it;
/// a mask without it is refused).
pub const CPUS: u32 = 1;

/// sched_yield(): always succeeds; the caller's time slice simply goes on
/// (the scheduler preempts at the next tick).
pub fn sched_yield(_k: &mut dyn LinuxKernel, _a: &Args) -> SysResult {
    Ok(0)
}

/// The processes `which` / `who` name (getpriority(2): 0 is the caller,
/// its group or its real user id), with each one's real and effective
/// user id. Zombies are left out.
fn targets(k: &mut dyn LinuxKernel, which: i32, who: i32) -> Result<Vec<(i32, u32, u32)>, Errno> {
    let ids = match which {
        PRIO_PROCESS if who == 0 => alloc::vec![k.pid()],
        PRIO_PROCESS => alloc::vec![who],
        PRIO_PGRP => k.process_ids(ProcessSelector::Group(if who == 0 { k.pgid() } else { who })),
        PRIO_USER => k.process_ids(ProcessSelector::All),
        _ => return Err(Errno::EINVAL),
    };
    let uid = if which == PRIO_USER && who == 0 { k.credentials().uid } else { who as u32 };
    let mut found = Vec::new();
    for pid in ids {
        let mut info = None;
        if k.with_process(Target::Process(pid), &mut |_, i| info = Some(*i)).is_err() {
            continue;
        }
        let Some(i) = info.filter(|i| !i.zombie) else { continue };
        if which == PRIO_USER && i.uid != uid {
            continue;
        }
        found.push((pid, i.uid, i.euid));
    }
    Ok(found)
}

/// getpriority(which, who): the highest priority (lowest nice value) of
/// the processes named, as the system call returns it: `20 - nice`, 1 to
/// 40 (getpriority(2) NOTES; the C library turns it back into a nice
/// value). `ESRCH` if none matched, `EINVAL` for an unknown `which`.
pub fn getpriority(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let found = targets(k, a.int(0), a.int(1))?;
    let mut best = None;
    for (pid, _, _) in found {
        let _ = k.with_process(Target::Process(pid), &mut |p, _| {
            best = Some(best.map_or(p.nice, |b: i32| b.min(p.nice)));
        });
    }
    let nice = best.ok_or(Errno::ESRCH)?;
    Ok((20 - nice) as u64)
}

/// setpriority(which, who, prio): every process named gets nice value
/// `prio` (clamped to -20..19). getpriority(2): `EPERM` unless the
/// caller's effective user id is the target's real or effective one (or it
/// has `CAP_SYS_NICE`); `EACCES` for a lower nice value without
/// `CAP_SYS_NICE` (`RLIMIT_NICE` is 0 by default, ADR 0043); `ESRCH` if
/// none matched.
pub fn setpriority(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let nice = a.int(2).clamp(NICE_MIN, NICE_MAX);
    let found = targets(k, a.int(0), a.int(1))?;
    if found.is_empty() {
        return Err(Errno::ESRCH);
    }
    let me = cred::current(k);
    // Priorities are the machine's: CAP_SYS_NICE in the initial user
    // namespace (ADR 0054).
    let privileged = me.capable_initial(cap::SYS_NICE);
    let mut result = Ok(0);
    for (pid, uid, euid) in found {
        if !privileged && me.euid != uid && me.euid != euid {
            result = Err(Errno::EPERM);
            continue;
        }
        let _ = k.with_process(Target::Process(pid), &mut |p, _| {
            if nice < p.nice && !privileged {
                result = Err(Errno::EACCES);
            } else {
                p.nice = nice;
            }
        });
    }
    result
}

/// The thread a sched_* call names: 0 is the caller (`EINVAL` for a
/// negative id, `ESRCH` for one that does not exist).
fn thread_target(k: &mut dyn LinuxKernel, pid: i32) -> Result<Target, Errno> {
    if pid < 0 {
        return Err(Errno::EINVAL);
    }
    let target = if pid == 0 { Target::Current } else { Target::Thread(pid) };
    k.with_process(target, &mut |_, _| {})?;
    Ok(target)
}

/// A `struct sched_param` (`int sched_priority`) at `a.ptr(i)`.
fn param(k: &dyn LinuxKernel, a: &Args, i: usize) -> Result<i32, Errno> {
    if a.ptr(i).is_null() {
        return Err(Errno::EINVAL);
    }
    let mut b = [0u8; 4];
    k.copy_from_user(a.ptr(i), &mut b)?;
    Ok(i32::from_le_bytes(b))
}

/// Whether `policy` with `priority` is valid (sched(7): 0 for the normal
/// policies, 1 to 99 for the real-time ones); `EPERM` for the real-time
/// ones, which Poly cannot honour.
fn check_policy(policy: i32, priority: i32) -> Result<(), Errno> {
    match policy & !SCHED_RESET_ON_FORK {
        SCHED_OTHER | SCHED_BATCH | SCHED_IDLE if priority == 0 => Ok(()),
        SCHED_FIFO | SCHED_RR if (1..=99).contains(&priority) => Err(Errno::EPERM),
        _ => Err(Errno::EINVAL),
    }
}

/// sched_setscheduler(pid, policy, param)
pub fn sched_setscheduler(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let priority = param(k, a, 2)?;
    if a.int(1) < 0 {
        return Err(Errno::EINVAL);
    }
    thread_target(k, a.int(0))?;
    check_policy(a.int(1), priority).map(|()| 0)
}

/// sched_getscheduler(pid): always `SCHED_OTHER`.
pub fn sched_getscheduler(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    thread_target(k, a.int(0))?;
    Ok(SCHED_OTHER as u64)
}

/// sched_setparam(pid, param): the priority must suit `SCHED_OTHER` (0).
pub fn sched_setparam(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let priority = param(k, a, 1)?;
    thread_target(k, a.int(0))?;
    check_policy(SCHED_OTHER, priority).map(|()| 0)
}

/// sched_getparam(pid, param): priority 0.
pub fn sched_getparam(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ptr(1).is_null() {
        return Err(Errno::EINVAL);
    }
    thread_target(k, a.int(0))?;
    k.copy_to_user(a.ptr(1), &0i32.to_le_bytes())?;
    Ok(0)
}

/// sched_get_priority_max(policy): 99 for the real-time policies, 0 for
/// the others (sched(7)), `EINVAL` for unknown ones.
pub fn sched_get_priority_max(_k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    match a.int(0) {
        SCHED_FIFO | SCHED_RR => Ok(99),
        SCHED_OTHER | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE => Ok(0),
        _ => Err(Errno::EINVAL),
    }
}

/// sched_get_priority_min(policy): 1 for the real-time policies, 0 for
/// the others.
pub fn sched_get_priority_min(_k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    match a.int(0) {
        SCHED_FIFO | SCHED_RR => Ok(1),
        SCHED_OTHER | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE => Ok(0),
        _ => Err(Errno::EINVAL),
    }
}

/// The size of the kernel's CPU mask: one `unsigned long` covers
/// [`CPUS`].
const MASK_BYTES: usize = 8;

/// sched_getaffinity(pid, cpusetsize, mask): the raw call returns the
/// number of bytes it wrote (sched_setaffinity(2) NOTES); `EINVAL` if
/// `cpusetsize` is smaller than the kernel's mask or not a multiple of the
/// size of a long.
pub fn sched_getaffinity(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = a.ulong(1) as usize;
    if size < MASK_BYTES || !size.is_multiple_of(8) {
        return Err(Errno::EINVAL);
    }
    thread_target(k, a.int(0))?;
    let mask = ((1u64 << CPUS) - 1).to_le_bytes();
    k.copy_to_user(a.ptr(2), &mask)?;
    Ok(MASK_BYTES as u64)
}

/// sched_setaffinity(pid, cpusetsize, mask): the mask must contain a CPU
/// Poly runs on (`EINVAL` otherwise); the thread keeps running there.
pub fn sched_setaffinity(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let size = (a.ulong(1) as usize).min(4096);
    let mut mask = alloc::vec![0u8; size];
    k.copy_from_user(a.ptr(2), &mut mask)?;
    thread_target(k, a.int(0))?;
    let ours = (0..CPUS as usize).any(|cpu| mask.get(cpu / 8).is_some_and(|b| b & (1 << (cpu % 8)) != 0));
    if ours { Ok(0) } else { Err(Errno::EINVAL) }
}
