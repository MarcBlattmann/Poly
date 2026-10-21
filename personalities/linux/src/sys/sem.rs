//! System V semaphores: semget(2), semop(2), semtimedop(2), semctl(2).
//!
//! The sets of the caller's IPC namespace are in its
//! [`SemTable`](crate::ipc::sem::SemTable), which decides under its lock;
//! the handlers copy to and from user memory outside it and unblock the
//! threads the table names after letting go of it.

use crate::abi::Timespec;
use crate::args::Args;
use crate::cred::Creds;
use crate::errno::{Errno, SysResult};
use crate::ipc::sem::*;
use crate::kernel::{Clock, Deadline, LinuxKernel, UserPtr, Wakeup};
use crate::ns::IpcNs;
use crate::ns::pid::view;
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] =
    &[(nr::SEMGET, semget), (nr::SEMOP, semop), (nr::SEMTIMEDOP, semtimedop), (nr::SEMCTL, semctl)];

/// The caller's IPC namespace, credentials and standing in the namespace.
fn caller(k: &mut dyn LinuxKernel) -> (Option<Arc<IpcNs>>, Creds, Who) {
    let tid = k.tid();
    let (ns, creds) = with_own(k, |p| (p.ns.ipc.clone(), p.thread(tid).creds.clone()));
    let owner = ns.as_ref().and_then(|n| n.owner().clone());
    let who = Who::new(&creds, &owner);
    (ns, creds, who)
}

/// The wall clock in seconds, for the set times.
fn now(k: &dyn LinuxKernel) -> i64 {
    k.clock(Clock::Realtime).map_or(0, |t| t.sec)
}

fn wake(k: &mut dyn LinuxKernel, tids: Vec<i32>) {
    for tid in tids {
        k.unblock(tid);
    }
}

/// semget(key, nsems, semflg)
///
/// All three are `int`s (the upper halves of the registers are ignored).
/// The table decides ([`SemTable::semget`]: the order of the errors is
/// there); a new set has `nsems` semaphores of value 0, its creator's
/// ids, mode `semflg & 0777` and the current time as `sem_ctime`.
pub fn semget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (key, nsems, flags) = (a.int(0), a.int(1), a.uint(2));
    let (ns, _, who) = caller(k);
    let now = now(k);
    with_table(&ns, |t| t.semget(key, nsems, flags, &who, now)).map(|id| id as u64)
}

/// semop(semid, sops, nsops)
pub fn semop(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    do_semop(k, a.int(0), a.ptr(1), a.uint(2), None)
}

/// semtimedop(semid, sops, nsops, timeout): semop(2) with a relative
/// `timeout` (null: none) after which a wait fails with `EAGAIN`.
pub fn semtimedop(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let timeout = a.ptr(3);
    let deadline = if timeout.is_null() {
        None
    } else {
        let mut raw = [0; Timespec::SIZE];
        k.copy_from_user(timeout, &mut raw)?;
        let t = Timespec::from_bytes(&raw);
        if !t.is_valid() {
            return Err(Errno::EINVAL);
        }
        Some(Deadline { clock: Clock::Monotonic, at: k.clock(Clock::Monotonic)?.saturating_add(t) })
    };
    do_semop(k, a.int(0), a.ptr(1), a.uint(2), deadline)
}

/// semop(2): `EINVAL` for a negative id or no operations, `E2BIG` for
/// more than [`SEMOPM`], `EFAULT` for operations that cannot be read; the
/// table checks the rest ([`SemTable::semop`]). The operations happen
/// all at once or not at all; until they can, the caller waits (unless
/// `IPC_NOWAIT`). A wait ends when another call lets them proceed (0),
/// when the set is removed (`EIDRM`), at the deadline (`EAGAIN`), or with
/// a signal: `EINTR` after a handler, whatever its `SA_RESTART`
/// (signal(7): semop(2) "is never restarted after being interrupted by a
/// signal handler"); restarted when no handler runs.
fn do_semop(k: &mut dyn LinuxKernel, id: i32, sops: UserPtr, nsops: u32, deadline: Option<Deadline>) -> SysResult {
    if id < 0 || nsops == 0 {
        return Err(Errno::EINVAL);
    }
    if nsops as usize > SEMOPM {
        return Err(Errno::E2BIG);
    }
    let mut raw = vec![0u8; nsops as usize * SemBuf::SIZE];
    k.copy_from_user(sops, &mut raw)?;
    let ops: Vec<SemBuf> = raw.chunks_exact(SemBuf::SIZE).map(SemBuf::from_bytes).collect();
    let (ns, _, who) = caller(k);
    let (tid, pid, now) = (k.tid(), k.pid(), now(k));
    let mut woken = Vec::new();
    let r = with_table(&ns, |t| t.semop(id, &ops, &who, (tid, pid), now, &mut woken));
    wake(k, woken);
    if r? == Semop::Done {
        return Ok(0);
    }
    loop {
        let why = k.block(deadline);
        let r = with_table(&ns, |t| {
            t.take_result(tid).or_else(|| match why {
                Wakeup::Woken => None, // an old unblock: wait on
                Wakeup::TimedOut => {
                    t.cancel(tid);
                    Some(Err(Errno::EAGAIN))
                }
                Wakeup::Signal => {
                    t.cancel(tid);
                    Some(Err(Errno::RESTART_IF_NO_HANDLER))
                }
            })
        });
        if let Some(r) = r {
            return r.map(|()| 0);
        }
    }
}

/// The calling process is exiting: its `SEM_UNDO` adjustments are applied
/// (semop(2): "automatically undone when the process terminates").
pub fn process_exit(k: &mut dyn LinuxKernel) {
    let ns = with_own(k, |p| p.ns.ipc.clone());
    let (pid, now) = (k.pid(), now(k));
    let woken = with_table(&ns, |t| t.exit(pid, now));
    wake(k, woken);
}

/// semctl(semid, semnum, cmd, arg)
///
/// `semid`, `semnum` and `cmd` are `int`s; `arg` is a value (`SETVAL`:
/// its low 32 bits as an `int`) or a pointer. A negative `semid` is
/// `EINVAL` before anything else, as is any command not listed here
/// (also one with `IPC_64`: x86-64 has only the 64-bit layouts).
///
/// - `IPC_STAT`: needs read permission (`EACCES`); writes the set's
///   `struct semid64_ds` (`EFAULT`), ids as the caller sees them.
///   `SEM_STAT` and `SEM_STAT_ANY` (no permission needed) take a slot
///   index for `semid` and return the id of the set there.
/// - `IPC_SET`: reads the structure first (`EFAULT`), then the owner,
///   the creator or a caller with `CAP_SYS_ADMIN` (`EPERM`) sets the
///   owner's uid and gid (`EINVAL` if unmapped, or -1) and `mode &
///   0777`; `sem_ctime` is now.
/// - `IPC_RMID`: the same permission; the set goes at once and its
///   waiters fail with `EIDRM`.
/// - `IPC_INFO`, `SEM_INFO`: write `struct seminfo`; return the highest
///   slot in use.
/// - `GETVAL`, `GETPID`, `GETNCNT`, `GETZCNT`: read permission, then
///   `semnum` within the set (`EINVAL`).
/// - `GETALL`: read permission, then the values to `arg` (`EFAULT`).
/// - `SETVAL`: the value within `0..=SEMVMX` (`ERANGE`, checked first),
///   then the id, `semnum`, alter permission.
/// - `SETALL`: alter permission, then the values from `arg` (`EFAULT`),
///   all within range (`ERANGE`, nothing changes).
///
/// `SETVAL` and `SETALL` set `sempid` and `sem_ctime`, clear every
/// process's adjustments of the semaphores they set and let blocked
/// calls that can now proceed complete.
pub fn semctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, num, cmd, arg) = (a.int(0), a.int(1), a.int(2), a.ulong(3));
    let buf = a.ptr(3);
    if id < 0 {
        return Err(Errno::EINVAL);
    }
    let (ns, creds, who) = caller(k);
    match cmd {
        IPC_INFO | SEM_INFO => {
            let (info, max) = with_table(&ns, |t| (t.info(cmd == SEM_INFO), t.max_index()));
            k.copy_to_user(buf, &info.to_bytes())?;
            Ok(max as u64)
        }
        IPC_STAT | SEM_STAT | SEM_STAT_ANY => {
            let (found, mut ds) = with_table(&ns, |t| {
                let (found, s) = if cmd == IPC_STAT {
                    (id, t.set(id).ok_or(Errno::EINVAL)?)
                } else {
                    t.by_index(id).ok_or(Errno::EINVAL)?
                };
                if cmd != SEM_STAT_ANY && !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok((found, SemidDs::of(s)))
            })?;
            ds.uid = creds.uid_view(ds.uid);
            ds.gid = creds.gid_view(ds.gid);
            ds.cuid = creds.uid_view(ds.cuid);
            ds.cgid = creds.gid_view(ds.cgid);
            k.copy_to_user(buf, &ds.to_bytes())?;
            Ok(if cmd == IPC_STAT { 0 } else { found as u64 })
        }
        IPC_SET => {
            let mut bytes = [0u8; SemidDs::SIZE];
            k.copy_from_user(buf, &mut bytes)?;
            let (uid, gid, mode) = SemidDs::perm_from_bytes(&bytes);
            let now = now(k);
            with_table(&ns, |t| {
                let s = t.set_mut(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                if uid == u32::MAX || gid == u32::MAX {
                    return Err(Errno::EINVAL);
                }
                let (uid, gid) = (creds.kuid(uid)?, creds.kgid(gid)?);
                s.uid = uid;
                s.gid = gid;
                s.mode = mode & 0o777;
                s.ctime = now;
                Ok(0)
            })
        }
        IPC_RMID => {
            let woken = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                Ok(t.remove(id))
            })?;
            wake(k, woken);
            Ok(0)
        }
        GETVAL | GETPID | GETNCNT | GETZCNT => {
            let r = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                let n = usize::try_from(num).ok().filter(|n| *n < s.nsems()).ok_or(Errno::EINVAL)?;
                Ok(match cmd {
                    GETVAL => s.sems[n].value,
                    GETPID => s.sems[n].pid,
                    GETNCNT => s.waiting(n, false) as i32,
                    _ => s.waiting(n, true) as i32,
                })
            })?;
            let r = if cmd == GETPID && r != 0 { view(k, r) } else { r };
            Ok(r as u64)
        }
        GETALL => {
            let values = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok(s.sems.iter().flat_map(|x| (x.value as u16).to_ne_bytes()).collect::<Vec<u8>>())
            })?;
            k.copy_to_user(buf, &values)?;
            Ok(0)
        }
        SETVAL => {
            let value = arg as u32 as i32;
            if !(0..=SEMVMX).contains(&value) {
                return Err(Errno::ERANGE);
            }
            let (pid, now) = (k.pid(), now(k));
            let woken = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                let n = usize::try_from(num).ok().filter(|n| *n < s.nsems()).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, ALTER) {
                    return Err(Errno::EACCES);
                }
                Ok(t.setval(id, n, value, pid, now))
            })?;
            wake(k, woken);
            Ok(0)
        }
        SETALL => {
            let allowed = |t: &SemTable| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, ALTER) {
                    return Err(Errno::EACCES);
                }
                Ok(s.nsems())
            };
            let nsems = with_table(&ns, |t| allowed(t))?;
            let mut raw = vec![0u8; nsems * 2];
            k.copy_from_user(buf, &mut raw)?;
            let values: Vec<i32> = raw.chunks_exact(2).map(|b| i32::from(u16::from_ne_bytes([b[0], b[1]]))).collect();
            let (pid, now) = (k.pid(), now(k));
            let woken = with_table(&ns, |t| {
                if allowed(t)? != nsems {
                    return Err(Errno::EINVAL);
                }
                if values.iter().any(|v| *v > SEMVMX) {
                    return Err(Errno::ERANGE);
                }
                Ok(t.setall(id, &values, pid, now))
            })?;
            wake(k, woken);
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}
