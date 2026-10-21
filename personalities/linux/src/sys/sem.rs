//! System V semaphores: semget(2), semop(2), semctl(2).
//!
//! The sets of the caller's IPC namespace are in its
//! [`SemTable`](crate::ipc::sem::SemTable), which decides under its lock.
//! A semop(2) that has to wait registers in the set and blocks outside
//! the lock; whatever changes the set wakes its waiters, which try again.

use super::shm::{caller, now};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::ipc::sem::{
    Done, GETALL, GETNCNT, GETPID, GETVAL, GETZCNT, SEMOPM, SETALL, SETVAL, SemBuf, SemSet, SemidDs, with_table,
};
use crate::ipc::{IPC_64, IPC_RMID, IPC_SET, IPC_STAT, READ, WRITE};
use crate::kernel::{LinuxKernel, Wakeup};
use crate::ns::pid::view;
use crate::table::{Handler, nr};
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::SEMGET, semget), (nr::SEMOP, semop), (nr::SEMCTL, semctl)];

/// Wake the threads in `tids`.
fn wake(k: &mut dyn LinuxKernel, tids: Vec<i32>) {
    for tid in tids {
        k.unblock(tid);
    }
}

/// semget(key, nsems, semflg)
///
/// All three arguments are `int`s (the upper halves of the registers are
/// ignored). The table decides ([`SemTable::get`](crate::ipc::sem::SemTable::get):
/// the order of the errors is there); a new set gets its creator's
/// effective ids, mode `semflg & 0777`, `nsems` semaphores of value 0 and
/// the current time as `sem_ctime`. No file descriptor is involved.
pub fn semget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (key, nsems, flags) = (a.int(0), a.int(1), a.uint(2));
    let (ns, _, who) = caller(k);
    let now = now(k);
    with_table(&ns, |t| t.get(key, nsems, flags, &who, now)).map(|id| id as u64)
}

/// semop(semid, sops, nsops)
///
/// semop(2): `nsops` 0 is `EINVAL`, above `SEMOPM` `E2BIG`; the
/// operations are read from `sops` (`EFAULT`). Then the id must name a set
/// (`EINVAL`), every `sem_num` must be below its size (`EFBIG`), and the
/// caller needs write permission if an operation changes a value, else
/// read (`EACCES`, unless `CAP_IPC_OWNER`). The operations apply together
/// or not at all ([`SemSet::try_ops`]); a call that has to wait blocks
/// until the set changes and tries again, fails with `EINTR` when a signal
/// comes (semop is never restarted, signal(7)), and with `EIDRM` if the
/// set is removed meanwhile. `SEM_UNDO` is accepted but not implemented:
/// nothing is undone at exit.
pub fn semop(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, sops, nsops) = (a.int(0), a.ptr(1), a.ulong(2));
    if nsops == 0 {
        return Err(Errno::EINVAL);
    }
    if nsops > SEMOPM {
        return Err(Errno::E2BIG);
    }
    let mut bytes = vec![0u8; nsops as usize * SemBuf::SIZE];
    k.copy_from_user(sops, &mut bytes)?;
    let ops: Vec<SemBuf> = bytes.chunks_exact(SemBuf::SIZE).map(SemBuf::from_bytes).collect();
    let highest = ops.iter().map(|o| usize::from(o.num)).max().unwrap_or(0);
    let access = if ops.iter().any(SemBuf::alters) { WRITE } else { READ };
    let (ns, _, who) = caller(k);
    let (tid, pid) = (k.tid(), k.pid());
    let mut waited = false;
    loop {
        let now = now(k);
        let (applied, woken) = with_table(&ns, |t| {
            let Some(s) = t.set_mut(id) else {
                return (Err(if waited { Errno::EIDRM } else { Errno::EINVAL }), Vec::new());
            };
            if highest >= s.nsems() {
                return (Err(Errno::EFBIG), Vec::new());
            }
            if !s.allows(&who, access) {
                return (Err(Errno::EACCES), Vec::new());
            }
            match s.try_ops(&ops, pid, now) {
                Err(e) => (Err(e), Vec::new()),
                Ok((Done::Applied, woken)) => (Ok(true), woken),
                Ok((Done::Wait { num, zero }, _)) => {
                    s.wait(tid, num, zero);
                    (Ok(false), Vec::new())
                }
            }
        });
        wake(k, woken);
        if applied? {
            return Ok(0);
        }
        waited = true;
        let why = k.block(None);
        with_table(&ns, |t| {
            if let Some(s) = t.set_mut(id) {
                s.unwait(tid);
            }
        });
        if why == Wakeup::Signal {
            return Err(Errno::EINTR);
        }
    }
}

/// semctl(semid, semnum, cmd, arg)
///
/// semctl(2), for `IPC_STAT`, `IPC_SET`, `IPC_RMID`, `GETVAL`, `GETPID`,
/// `GETNCNT`, `GETZCNT`, `GETALL`, `SETVAL` and `SETALL` (`IPC_64` in
/// `cmd` is ignored: x86-64 has one layout of `struct semid_ds`). `arg` is
/// the union semun passed by value: an `int` for `SETVAL`, else a pointer.
/// An id that names no set is `EINVAL`, and so is every other command
/// (`IPC_INFO`, `SEM_INFO` and `SEM_STAT` are not implemented).
///
/// - `IPC_STAT`: needs read permission (`EACCES`); writes the set's
///   `struct semid64_ds` to `arg` (`EFAULT`), ids as the caller sees them.
/// - `IPC_SET`: reads `arg` (`EFAULT`); the owner, the creator or a caller
///   with `CAP_SYS_ADMIN` (`EPERM`) sets the owner's uid and gid (`EINVAL`
///   if the caller's user namespace has no such id) and the permission
///   bits; `sem_ctime` is now.
/// - `IPC_RMID`: the same permission; the set goes at once and its
///   waiters wake with `EIDRM`.
/// - `GETVAL`, `GETPID`, `GETNCNT`, `GETZCNT`: need read permission;
///   `semnum` must be a semaphore of the set (`EINVAL`). `GETPID` is the
///   pid as the caller sees it.
/// - `GETALL`: needs read permission; writes every value (`unsigned
///   short`) to `arg` (`EFAULT`).
/// - `SETVAL`: needs write permission; `semnum` as above, the value must
///   be in `0..=SEMVMX` (`ERANGE`). The semaphore records the caller as
///   its last pid; `sem_ctime` is now; waiters try again.
/// - `SETALL`: needs write permission; reads the values from `arg`
///   (`EFAULT`), each at most `SEMVMX` (`ERANGE`); as `SETVAL` for all.
pub fn semctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, num, cmd, arg) = (a.int(0), a.int(1), a.uint(2) & !IPC_64, a.ptr(3));
    let (ns, creds, who) = caller(k);
    match cmd {
        IPC_STAT => {
            let ds = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok(stat(s))
            })?;
            let ds = SemidDs {
                uid: creds.uid_view(ds.uid),
                gid: creds.gid_view(ds.gid),
                cuid: creds.uid_view(ds.cuid),
                cgid: creds.gid_view(ds.cgid),
                ..ds
            };
            k.copy_to_user(arg, &ds.to_bytes())?;
            Ok(0)
        }
        IPC_SET => {
            let mut bytes = [0u8; SemidDs::SIZE];
            k.copy_from_user(arg, &mut bytes)?;
            let (uid, gid, mode) = SemidDs::perm_from_bytes(&bytes);
            let now = now(k);
            with_table(&ns, |t| {
                let s = t.set_mut(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                let (uid, gid) = (creds.kuid(uid)?, creds.kgid(gid)?);
                s.owners.uid = uid;
                s.owners.gid = gid;
                s.mode = mode & 0o777;
                s.ctime = now;
                Ok(0)
            })
        }
        IPC_RMID => {
            let gone = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                Ok(t.remove(id))
            })?;
            if let Some(s) = gone {
                wake(k, s.waiters());
            }
            Ok(0)
        }
        GETVAL | GETPID | GETNCNT | GETZCNT => {
            let value = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                let sem = s.semaphore(num)?;
                Ok(match cmd {
                    GETVAL => u64::from(sem.val),
                    GETPID => sem.pid as u64,
                    GETNCNT => s.waiting(num as u16, false),
                    _ => s.waiting(num as u16, true),
                })
            })?;
            if cmd == GETPID && value != 0 {
                return Ok(view(k, value as i32) as u64);
            }
            Ok(value)
        }
        GETALL => {
            let vals: Vec<u8> = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok(s.sems.iter().flat_map(|x| x.val.to_ne_bytes()).collect())
            })?;
            k.copy_to_user(arg, &vals)?;
            Ok(0)
        }
        SETVAL => {
            let (val, pid, now) = (a.int(3), k.pid(), now(k));
            let woken = with_table(&ns, |t| {
                let s = t.set_mut(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, WRITE) {
                    return Err(Errno::EACCES);
                }
                s.set_value(num, val, pid, now)
            })?;
            wake(k, woken);
            Ok(0)
        }
        SETALL => {
            let nsems = with_table(&ns, |t| {
                let s = t.set(id).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, WRITE) {
                    return Err(Errno::EACCES);
                }
                Ok(s.nsems())
            })?;
            let mut bytes = vec![0u8; nsems * 2];
            k.copy_from_user(arg, &mut bytes)?;
            let vals: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_ne_bytes([b[0], b[1]])).collect();
            let (pid, now) = (k.pid(), now(k));
            let woken = with_table(&ns, |t| {
                // Removed meanwhile (ids are not reused soon): as if it had
                // been before the call.
                let s = t.set_mut(id).filter(|s| s.nsems() == nsems).ok_or(Errno::EINVAL)?;
                if !s.allows(&who, WRITE) {
                    return Err(Errno::EACCES);
                }
                s.set_all(&vals, pid, now)
            })?;
            wake(k, woken);
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}

/// What `IPC_STAT` reports of `s`, with kernel ids.
fn stat(s: &SemSet) -> SemidDs {
    SemidDs {
        key: s.key,
        uid: s.owners.uid,
        gid: s.owners.gid,
        cuid: s.owners.cuid,
        cgid: s.owners.cgid,
        mode: s.mode,
        seq: 0,
        otime: s.otime,
        ctime: s.ctime,
        nsems: s.nsems() as u64,
    }
}
