//! I/O priorities (ioprio_set(2), ioprio_get(2), ADR 0121).
//!
//! A priority is `class << 13 | hint << 3 | level` in 16 bits
//! (`linux/ioprio.h`): class none (0, "derived from the nice value"),
//! real time (1), best effort (2) or idle (3), level 0..7 (lower is more
//! important), hint bits 3..12 kept as given. Each thread has one
//! ([`LinuxThread::ioprio`](crate::signal::LinuxThread::ioprio)), copied
//! to the threads and processes it creates. Poly has no I/O scheduler that
//! looks at it yet, so it is recorded and reported only.
//!
//! `which` names one thread (`IOPRIO_WHO_PROCESS`, 0 the caller), the
//! processes of a process group (`IOPRIO_WHO_PGRP`, 0 the caller's) or of
//! a real user id (`IOPRIO_WHO_USER`, 0 being uid 0). A group or user
//! reads the best value of its threads ([`best`]), with class none counted
//! as best effort at level `(nice + 20) / 5`; a zombie member counts as
//! class none at nice 0.

use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, ProcessSelector, Target};
use crate::signal::LinuxProcess;
use crate::table::{Handler, nr};
use alloc::vec::Vec;

pub const IOPRIO_WHO_PROCESS: i32 = 1;
pub const IOPRIO_WHO_PGRP: i32 = 2;
pub const IOPRIO_WHO_USER: i32 = 3;

pub const IOPRIO_CLASS_NONE: u16 = 0;
pub const IOPRIO_CLASS_RT: u16 = 1;
pub const IOPRIO_CLASS_BE: u16 = 2;
pub const IOPRIO_CLASS_IDLE: u16 = 3;
pub const IOPRIO_CLASS_SHIFT: u16 = 13;
pub const IOPRIO_LEVEL_MASK: u16 = 7;

pub const CALLS: &[(u64, Handler)] = &[(nr::IOPRIO_SET, ioprio_set), (nr::IOPRIO_GET, ioprio_get)];

const fn class(v: u16) -> u16 {
    v >> IOPRIO_CLASS_SHIFT
}

/// The value ioprio_set(2) stores for the argument `ioprio` (bits 16 and
/// up dropped): `EINVAL` for classes 4..7 and for class none with a level.
/// Class none is stored as 0.
pub fn parse(ioprio: u32) -> Result<u16, Errno> {
    let v = ioprio as u16;
    match class(v) {
        IOPRIO_CLASS_NONE if v & IOPRIO_LEVEL_MASK != 0 => Err(Errno::EINVAL),
        IOPRIO_CLASS_NONE => Ok(0),
        IOPRIO_CLASS_RT | IOPRIO_CLASS_BE | IOPRIO_CLASS_IDLE => Ok(v),
        _ => Err(Errno::EINVAL),
    }
}

/// A thread's value as a group or user read sees it: class none becomes
/// best effort at the level its nice value gives.
pub const fn effective(v: u16, nice: i32) -> u16 {
    if class(v) == IOPRIO_CLASS_NONE { (IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | ((nice + 20) / 5) as u16 } else { v }
}

/// How good `v` is (lower is better): real time, best effort, idle, then
/// the level.
const fn rank(v: u16) -> (u16, u16) {
    let c = match class(v) {
        IOPRIO_CLASS_RT => 0,
        IOPRIO_CLASS_BE => 1,
        _ => 2,
    };
    (c, v & IOPRIO_LEVEL_MASK)
}

/// The better of two effective values.
pub fn best(a: u16, b: u16) -> u16 {
    if rank(b) < rank(a) { b } else { a }
}

/// One thread a call names: process id and thread id.
type Member = (i32, i32);

/// The threads `which` / `who` name: `EINVAL` for an unknown `which`,
/// `ESRCH` if none. A zombie process is one member with thread id 0.
fn members(k: &mut dyn LinuxKernel, which: i32, who: i32) -> Result<Vec<Member>, Errno> {
    let (pids, uid) = match which {
        IOPRIO_WHO_PROCESS => {
            let tid = if who == 0 { k.tid() } else { who };
            if tid < 0 {
                return Err(Errno::ESRCH);
            }
            let mut found = None;
            k.with_process(Target::Thread(tid), &mut |_, info| found = Some(info.pid))?;
            return found.map(|pid| alloc::vec![(pid, tid)]).ok_or(Errno::ESRCH);
        }
        IOPRIO_WHO_PGRP => (k.process_ids(ProcessSelector::Group(if who == 0 { k.pgid() } else { who })), None),
        IOPRIO_WHO_USER => (k.process_ids(ProcessSelector::All), Some(who as u32)),
        _ => return Err(Errno::EINVAL),
    };
    let mut found = Vec::new();
    for pid in pids {
        let _ = k.with_process(Target::Process(pid), &mut |p, _| {
            let tids: Vec<i32> = p.thread_ids().collect();
            if tids.is_empty() {
                let real = p.process_creds(pid).map(|c| c.uid);
                if uid.is_none() || real == uid {
                    found.push((pid, 0));
                }
            }
            for tid in tids {
                if uid.is_none_or(|u| p.thread(tid).creds.uid == u) {
                    found.push((pid, tid));
                }
            }
        });
    }
    if found.is_empty() { Err(Errno::ESRCH) } else { Ok(found) }
}

/// Run `f` on member `m`'s process; `ESRCH` if it is gone.
fn with_member<R>(k: &mut dyn LinuxKernel, m: Member, mut f: impl FnMut(&mut LinuxProcess) -> R) -> Result<R, Errno> {
    let mut out = None;
    k.with_process(Target::Process(m.0), &mut |p, _| out = Some(f(p)))?;
    out.ok_or(Errno::ESRCH)
}

/// ioprio_set(which, who, ioprio): in order `EINVAL` (class and level),
/// `EPERM` (real time without `CAP_SYS_ADMIN` or `CAP_SYS_NICE`), `EINVAL`
/// (`which`), `ESRCH`, then per member `EPERM` unless its real user id is
/// the caller's real or effective one or the caller has `CAP_SYS_NICE`;
/// members are changed in turn up to the first that fails.
pub fn ioprio_set(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let value = parse(a.uint(2))?;
    let me = cred::current(k);
    if class(value) == IOPRIO_CLASS_RT && !me.capable(cap::SYS_ADMIN) && !me.capable(cap::SYS_NICE) {
        return Err(Errno::EPERM);
    }
    let found = members(k, a.int(0), a.int(1))?;
    let nice_cap = me.capable(cap::SYS_NICE);
    for m in found {
        let r = with_member(k, m, |p| {
            if m.1 == 0 || !p.has_thread(m.1) {
                return Ok(());
            }
            let t = p.thread_mut(m.1);
            if !nice_cap && t.creds.uid != me.uid && t.creds.uid != me.euid {
                return Err(Errno::EPERM);
            }
            t.ioprio = value;
            Ok(())
        });
        match r {
            Ok(Ok(())) | Err(_) => {}
            Ok(Err(e)) => return Err(e),
        }
    }
    Ok(0)
}

/// ioprio_get(which, who): one thread's stored value, or the best
/// effective value of a group's or user's threads. `EINVAL` (`which`),
/// then `ESRCH`.
pub fn ioprio_get(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let which = a.int(0);
    let found = members(k, which, a.int(1))?;
    let mut result: Option<u16> = None;
    for m in found {
        let v = with_member(k, m, |p| {
            if m.1 == 0 || !p.has_thread(m.1) {
                return effective(0, 0);
            }
            let t = p.thread(m.1);
            if which == IOPRIO_WHO_PROCESS { t.ioprio } else { effective(t.ioprio, t.sched.nice) }
        });
        if let Ok(v) = v {
            result = Some(result.map_or(v, |r| best(r, v)));
        }
    }
    result.map(u64::from).ok_or(Errno::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BE: u16 = IOPRIO_CLASS_BE << 13;
    const RT: u16 = IOPRIO_CLASS_RT << 13;
    const IDLE: u16 = IOPRIO_CLASS_IDLE << 13;

    #[test]
    fn values() {
        assert_eq!(parse(u32::from(BE) | 3), Ok(BE | 3));
        assert_eq!(parse(u32::from(BE) | 0x55 << 3 | 1), Ok(BE | 0x55 << 3 | 1), "hints kept");
        assert_eq!(parse(0x14001), Ok(0x4001), "bits 16 and up dropped");
        assert_eq!(parse(0), Ok(0));
        assert_eq!(parse(1), Err(Errno::EINVAL), "class none with a level");
        assert_eq!(parse(4 << 13), Err(Errno::EINVAL));
        assert_eq!(parse(u32::MAX), Err(Errno::EINVAL));
    }

    #[test]
    fn best_of_a_group() {
        assert_eq!(effective(0, 0), BE | 4);
        assert_eq!(effective(0, 10), BE | 6);
        assert_eq!(effective(0, 19), BE | 7);
        assert_eq!(effective(IDLE, 0), IDLE);
        assert_eq!(best(BE | 3, effective(0, 0)), BE | 3);
        assert_eq!(best(BE | 7, effective(0, 0)), BE | 4);
        assert_eq!(best(IDLE, effective(0, 0)), BE | 4);
        assert_eq!(best(BE, RT | 7), RT | 7);
    }
}
