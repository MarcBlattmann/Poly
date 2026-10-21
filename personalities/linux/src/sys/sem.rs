//! System V semaphores: semget(2), semop(2), semctl(2).
//!
//! The sets of the caller's IPC namespace are in its
//! [`SemTable`](crate::ipc::sem::SemTable), which decides under its lock
//! and completes sleeping callers itself; the handlers only read and write
//! user memory, sleep and wake threads, outside the lock.

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::ipc::sem::{
    GETALL, GETNCNT, GETPID, GETVAL, GETZCNT, SEMOPM, SETALL, SETVAL, SemBuf, SemSet, SemidDs, Semop, with_table,
};
use crate::ipc::shm::{IPC_64, IPC_RMID, IPC_SET, IPC_STAT, READ, WRITE, Who};
use crate::kernel::LinuxKernel;
use crate::ns::pid::view;
use crate::signal::deliver::with_own;
use crate::sys::shm::{caller, now};
use crate::table::{Handler, nr};
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SEMGET, semget), (nr::SEMOP, semop), (nr::SEMCTL, semctl)];

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
    let (id, nsops) = (a.int(0), a.uint(2));
    if nsops == 0 {
        return Err(Errno::EINVAL);
    }
    if nsops > SEMOPM {
        return Err(Errno::E2BIG);
    }
    let mut bytes = vec![0u8; nsops as usize * SemBuf::SIZE];
    k.copy_from_user(a.ptr(1), &mut bytes)?;
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
        if with_own(k, |p| p.has_deliverable(tid)) {
            return match with_table(&ns, |t| t.cancel(ticket)) {
                Some(result) => result.map(|()| 0),
                None => Err(Errno::EINTR),
            };
        }
        k.block(None);
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
/// semctl(2), for `IPC_STAT`, `IPC_SET`, `IPC_RMID`, `GETVAL`, `GETPID`,
/// `GETNCNT`, `GETZCNT`, `GETALL`, `SETVAL` and `SETALL` (`IPC_64` in
/// `cmd` is ignored: x86-64 has one layout of `struct semid_ds`). An id
/// that names no set is `EINVAL`, and so is every other command
/// (`IPC_INFO`, `SEM_INFO`, `SEM_STAT` are not implemented) and a
/// `semnum` outside the set. Reading needs read permission, setting values
/// alter permission (`EACCES`); `IPC_SET` and `IPC_RMID` need the owner,
/// the creator or `CAP_SYS_ADMIN` (`EPERM`).
///
/// - `IPC_STAT` writes the set's `struct semid64_ds` to `arg`.
/// - `IPC_SET` takes the owner's uid and gid and the permission bits from
///   the one at `arg`; `sem_ctime` is now.
/// - `IPC_RMID` removes the set at once; its sleepers fail with `EIDRM`.
/// - `GETVAL`, `GETPID`, `GETNCNT`, `GETZCNT` return `semval`, `sempid`,
///   `semncnt`, `semzcnt` of semaphore `semnum`; `GETALL` writes every
///   `semval` to the `unsigned short` array at `arg`.
/// - `SETVAL` sets semaphore `semnum` to the `int` in `arg`, `SETALL`
///   every semaphore from the array at `arg` (`ERANGE` above 32767, or
///   below 0): `sempid` becomes the caller, `sem_ctime` now, every
///   process's adjustment for the semaphores is dropped, and sleepers
///   that can proceed now do.
pub fn semctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, num, cmd, arg) = (a.int(0), a.int(1), a.uint(2) & !IPC_64, a.ulong(3));
    let (ns, creds, who) = caller(k);
    let in_set = |s: &SemSet| u16::try_from(num).ok().filter(|n| usize::from(*n) < s.nsems()).ok_or(Errno::EINVAL);
    match cmd {
        IPC_STAT => {
            let mut ds = with_set(&ns, id, &who, READ, |s| Ok(SemidDs::of(s)))?;
            ds.uid = creds.uid_view(ds.uid);
            ds.gid = creds.gid_view(ds.gid);
            ds.cuid = creds.uid_view(ds.cuid);
            ds.cgid = creds.gid_view(ds.cgid);
            k.copy_to_user(a.ptr(3), &ds.to_bytes())?;
            Ok(0)
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
            let (pid, now) = (k.pid(), now(k));
            let woken = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, WRITE) {
                    return Err(Errno::EACCES);
                }
                t.setval(id, in_set(s)?, arg as i32, pid, now)
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
