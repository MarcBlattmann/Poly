//! System V semaphores: semget(2), semop(2), semctl(2).
//!
//! The sets of the caller's IPC namespace are in its
//! [`SemTable`](crate::ipc::sem::SemTable), which decides under its lock
//! and completes sleeping callers itself; the handlers only read and write
//! user memory, sleep and wake threads, outside the lock.

use crate::abi::Timespec;
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::ipc::sem::{
    GETALL, GETNCNT, GETPID, GETVAL, GETZCNT, IPC_INFO, SEM_INFO, SEM_STAT, SEM_STAT_ANY, SEMOPM, SEMVMX, SETALL,
    SETVAL, SemBuf, SemSet, SemidDs, Semop, seminfo_bytes, with_table,
};
use crate::ipc::shm::{IPC_RMID, IPC_SET, IPC_STAT, READ, WRITE, Who};
use crate::kernel::{Clock, Deadline, LinuxKernel};
use crate::ns::pid::view;
use crate::signal::deliver::with_own;
use crate::sys::shm::{caller, now};
use crate::table::{Handler, nr};
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] =
    &[(nr::SEMGET, semget), (nr::SEMOP, semop), (nr::SEMTIMEDOP, semtimedop), (nr::SEMCTL, semctl)];

/// Wake the threads a table operation completed.
fn wake(k: &mut dyn LinuxKernel, tids: Vec<i32>) {
    for tid in tids {
        k.unblock(tid);
    }
}

/// semget(key, nsems, semflg)
///
/// All three are `int`s (the upper halves of the registers are ignored).
/// The table decides ([`SemTable::get`](crate::ipc::sem::SemTable::get):
/// the order of the errors is there).
pub fn semget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (key, nsems, flags) = (a.int(0), a.int(1), a.uint(2));
    let (ns, _, who) = caller(k);
    let now = now(k);
    with_table(&ns, |t| t.get(key, nsems, flags, &who, now)).map(|id| id as u64)
}

/// semop(semid, sops, nsops)
///
/// `semid` is an `int` and `nsops` an `unsigned int` (the upper halves of
/// the registers are ignored). `EINVAL` if `nsops` is 0, `E2BIG` above
/// [`SEMOPM`], `EFAULT` if the array cannot be read, all before the id is
/// looked up; the table does the rest
/// ([`SemTable::semop`](crate::ipc::sem::SemTable::semop)). A caller that
/// must wait sleeps until whoever makes its array possible completes it,
/// the set is removed (`EIDRM`) or a signal is to be acted on: then the
/// call fails with `EINTR` and is never restarted (signal(7) lists semop
/// among the calls that fail after a handler, `SA_RESTART` or not, and
/// after a stop).
pub fn semop(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    operate(k, a, None)
}

/// semtimedop(semid, sops, nsops, timeout)
///
/// semop with a bound on the sleep: `timeout` (a relative `struct
/// timespec`, only read) is read first of all (`EFAULT`), checked after
/// the array was read (`EINVAL` for a negative or unnormalized one, before
/// the id is looked up), and a sleep that outlasts it fails with `EAGAIN`,
/// nothing applied. A NULL `timeout` is semop.
pub fn semtimedop(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ptr(3).is_null() {
        return operate(k, a, None);
    }
    let mut b = [0u8; Timespec::SIZE];
    k.copy_from_user(a.ptr(3), &mut b)?;
    operate(k, a, Some(Timespec::from_bytes(&b)))
}

/// semop(2) and semtimedop(2), with the timeout already read.
fn operate(k: &mut dyn LinuxKernel, a: &Args, timeout: Option<Timespec>) -> SysResult {
    let (id, nsops) = (a.int(0), a.uint(2));
    if nsops == 0 {
        return Err(Errno::EINVAL);
    }
    if nsops > SEMOPM {
        return Err(Errno::E2BIG);
    }
    let mut bytes = vec![0u8; nsops as usize * SemBuf::SIZE];
    k.copy_from_user(a.ptr(1), &mut bytes)?;
    if timeout.is_some_and(|t| !t.is_valid()) {
        return Err(Errno::EINVAL);
    }
    let deadline = match timeout {
        Some(t) => Some(Deadline { clock: Clock::Monotonic, at: k.clock(Clock::Monotonic)?.saturating_add(t) }),
        None => None,
    };
    let ops = SemBuf::parse(&bytes);
    let (ns, _, who) = caller(k);
    let (pid, tid, now) = (k.pid(), k.tid(), now(k));
    let ticket = match with_table(&ns, |t| t.semop(id, ops, &who, pid, tid, now))? {
        Semop::Done(woken) => {
            wake(k, woken);
            return Ok(0);
        }
        Semop::Sleep(ticket) => ticket,
    };
    loop {
        // A signal sent from now on notifies the thread and ends the
        // block; one already pending is seen below.
        k.clear_signal_notification();
        if let Some(result) = with_table(&ns, |t| t.poll(ticket)) {
            return result.map(|()| 0);
        }
        // A signal that is ignored (a traced process keeps those until
        // they are taken) does not end the wait.
        if with_own(k, |p| p.has_acting(tid)) {
            return match with_table(&ns, |t| t.cancel(ticket)) {
                Some(result) => result.map(|()| 0),
                None => Err(Errno::EINTR),
            };
        }
        if let Some(d) = deadline
            && k.clock(Clock::Monotonic).is_ok_and(|now| (now.sec, now.nsec) >= (d.at.sec, d.at.nsec))
        {
            return match with_table(&ns, |t| t.cancel(ticket)) {
                Some(result) => result.map(|()| 0),
                None => Err(Errno::EAGAIN),
            };
        }
        k.block(deadline);
    }
}

/// The calling process is exiting: its `SEM_UNDO` adjustments are added
/// back to their semaphores (semop(2)), which may complete sleepers.
pub fn process_exit(k: &mut dyn LinuxKernel) {
    let ns = with_own(k, |p| p.ns.ipc.clone());
    let (pid, now) = (k.pid(), now(k));
    let woken = with_table(&ns, |t| t.exit(pid, now));
    wake(k, woken);
}

/// Look at set `id` under the lock: `EINVAL` if there is none, `EACCES`
/// unless `who` has `access`.
fn with_set<R>(
    ns: &Option<alloc::sync::Arc<crate::ns::IpcNs>>,
    id: i32,
    who: &Who,
    access: u32,
    f: impl FnOnce(&SemSet) -> Result<R, Errno>,
) -> Result<R, Errno> {
    with_table(ns, |t| {
        let s = t.set(id).ok_or(Errno::EINVAL)?;
        if !s.allows(who, access) {
            return Err(Errno::EACCES);
        }
        f(s)
    })
}

/// semctl(semid, semnum, cmd, arg)
///
/// semctl(2). A negative id is `EINVAL` for every command, and so is an
/// id that names no set, an unknown command (also one with the `IPC_64`
/// bit) and a `semnum` outside the set. Reading needs read permission
/// (checked before `semnum`), setting values alter permission (checked
/// after it) (`EACCES`); `IPC_SET` and `IPC_RMID` need the owner, the
/// creator or `CAP_SYS_ADMIN` (`EPERM`).
///
/// - `IPC_INFO` and `SEM_INFO` write a `struct seminfo` and return the
///   highest index in use; `SEM_STAT` and `SEM_STAT_ANY` are `IPC_STAT`
///   of the set at an index ([`crate::ipc::index_of`]) and return its id
///   (`SEM_STAT_ANY` needs no permission).
///
/// - `IPC_STAT` writes the set's `struct semid64_ds` to `arg`.
/// - `IPC_SET` takes the owner's uid and gid and the permission bits from
///   the one at `arg`; `sem_ctime` is now.
/// - `IPC_RMID` removes the set at once; its sleepers fail with `EIDRM`.
/// - `GETVAL`, `GETPID`, `GETNCNT`, `GETZCNT` return `semval`, `sempid`,
///   `semncnt`, `semzcnt` of semaphore `semnum`; `GETALL` writes every
///   `semval` to the `unsigned short` array at `arg`.
/// - `SETVAL` sets semaphore `semnum` to the `int` in `arg` (its range is
///   checked before the id), `SETALL`
///   every semaphore from the array at `arg` (`ERANGE` above 32767, or
///   below 0): `sempid` becomes the caller, `sem_ctime` now, every
///   process's adjustment for the semaphores is dropped, and sleepers
///   that can proceed now do.
pub fn semctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, num, cmd, arg) = (a.int(0), a.int(1), a.uint(2), a.ulong(3));
    if id < 0 {
        return Err(Errno::EINVAL);
    }
    let (ns, creds, who) = caller(k);
    let in_set = |s: &SemSet| u16::try_from(num).ok().filter(|n| usize::from(*n) < s.nsems()).ok_or(Errno::EINVAL);
    match cmd {
        IPC_STAT | SEM_STAT | SEM_STAT_ANY => {
            let (id, mut ds) = with_table(&ns, |t| {
                let id = if cmd == IPC_STAT { id } else { t.by_index(id).ok_or(Errno::EINVAL)? };
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if cmd != SEM_STAT_ANY && !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok((id, SemidDs::of(s)))
            })?;
            ds.uid = creds.uid_view(ds.uid);
            ds.gid = creds.gid_view(ds.gid);
            ds.cuid = creds.uid_view(ds.cuid);
            ds.cgid = creds.gid_view(ds.cgid);
            k.copy_to_user(a.ptr(3), &ds.to_bytes())?;
            Ok(if cmd == IPC_STAT { 0 } else { id as u64 })
        }
        IPC_INFO | SEM_INFO => {
            let (info, max) =
                with_table(&ns, |t| (seminfo_bytes(if cmd == SEM_INFO { Some(t) } else { None }), t.max_index()));
            k.copy_to_user(a.ptr(3), &info)?;
            Ok(max as u64)
        }
        IPC_SET => {
            let mut bytes = [0u8; SemidDs::SIZE];
            k.copy_from_user(a.ptr(3), &mut bytes)?;
            let (uid, gid, mode) = SemidDs::perm_from_bytes(&bytes);
            let now = now(k);
            with_table(&ns, |t| {
                let s = t.set_mut(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
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
                t.remove(id)
            })?;
            wake(k, woken);
            Ok(0)
        }
        GETVAL => with_set(&ns, id, &who, READ, |s| Ok(s.sems[usize::from(in_set(s)?)].val as u64)),
        GETPID => {
            let pid = with_set(&ns, id, &who, READ, |s| Ok(s.sems[usize::from(in_set(s)?)].pid))?;
            Ok(if pid == 0 { 0 } else { view(k, pid) as u64 })
        }
        GETNCNT | GETZCNT => {
            let (n, z) = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok(t.counts(id, in_set(s)?))
            })?;
            Ok(if cmd == GETNCNT { n } else { z })
        }
        GETALL => {
            let vals = with_set(&ns, id, &who, READ, |s| Ok(s.sems.iter().map(|v| v.val as u16).collect::<Vec<_>>()))?;
            let bytes: Vec<u8> = vals.iter().flat_map(|v| v.to_ne_bytes()).collect();
            k.copy_to_user(a.ptr(3), &bytes)?;
            Ok(0)
        }
        SETVAL => {
            let val = arg as i32;
            if !(0..=SEMVMX).contains(&val) {
                return Err(Errno::ERANGE);
            }
            let (pid, now) = (k.pid(), now(k));
            let woken = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                let num = in_set(s)?;
                if !s.allows(&who, WRITE) {
                    return Err(Errno::EACCES);
                }
                t.setval(id, num, val, pid, now)
            })?;
            wake(k, woken);
            Ok(0)
        }
        SETALL => {
            let nsems = with_set(&ns, id, &who, WRITE, |s| Ok(s.nsems()))?;
            let mut bytes = vec![0u8; nsems * 2];
            k.copy_from_user(a.ptr(3), &mut bytes)?;
            let vals: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_ne_bytes([b[0], b[1]])).collect();
            let (pid, now) = (k.pid(), now(k));
            let woken = with_table(&ns, |t| match t.set(id) {
                Some(s) if s.nsems() == vals.len() => t.setall(id, &vals, pid, now),
                _ => Err(Errno::EINVAL),
            })?;
            wake(k, woken);
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}
