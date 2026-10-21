//! System V message queues: msgget(2), msgsnd(2), msgrcv(2), msgctl(2).
//!
//! The queues of the caller's IPC namespace are in its
//! [`MsgTable`](crate::ipc::msg::MsgTable), which decides under its lock;
//! the handlers only read and write user memory, sleep and wake threads,
//! outside the lock.

use crate::args::Args;
use crate::cred::cap;
use crate::errno::{Errno, SysResult};
use crate::ipc::msg::{IPC_NOWAIT, MSG_COPY, MSG_EXCEPT, MSGMAX, MSGMNB, Msg, MsgTable, MsqidDs, Step, with_table};
use crate::ipc::shm::{IPC_64, IPC_RMID, IPC_SET, IPC_STAT, READ};
use crate::kernel::LinuxKernel;
use crate::ns::IpcNs;
use crate::ns::pid::view;
use crate::signal::deliver::with_own;
use crate::sys::shm::{caller, now};
use crate::table::{Handler, nr};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] =
    &[(nr::MSGGET, msgget), (nr::MSGSND, msgsnd), (nr::MSGRCV, msgrcv), (nr::MSGCTL, msgctl)];

/// `sizeof(long)`: the `mtype` that heads `struct msgbuf` (`linux/msg.h`).
const MTYPE: u64 = 8;

/// Wake the threads a table operation reported.
fn wake(k: &mut dyn LinuxKernel, tids: Vec<i32>) {
    for tid in tids {
        k.unblock(tid);
    }
}

/// msgget(key, msgflg)
///
/// Both are `int`s (the upper halves of the registers are ignored). The
/// table decides ([`MsgTable::get`]: the order of the errors is there).
pub fn msgget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (key, flags) = (a.int(0), a.uint(1));
    let (ns, _, who) = caller(k);
    let now = now(k);
    with_table(&ns, |t| t.get(key, flags, &who, now)).map(|id| id as u64)
}

/// Ask the table with `attempt` until it is done with queue `id`: in
/// between, the caller sleeps until a change to the queue wakes it, or a
/// signal is to be acted on, which ends the call with `EINTR` (signal(7)
/// lists msgsnd and msgrcv among the calls never restarted). `attempt`
/// gets the serial of the queue the caller waited on (`None` at first).
fn wait_for<T>(
    k: &mut dyn LinuxKernel,
    ns: &Option<Arc<IpcNs>>,
    id: i32,
    mut attempt: impl FnMut(&mut MsgTable, Option<u64>) -> Result<Step<T>, Errno>,
) -> Result<T, Errno> {
    let tid = k.tid();
    let mut serial = None;
    loop {
        // A signal sent from now on notifies the thread and ends the
        // block; one already pending is seen below.
        k.clear_signal_notification();
        match with_table(ns, |t| attempt(t, serial))? {
            Step::Done(value, woken) => {
                wake(k, woken);
                return Ok(value);
            }
            Step::Wait(s) => serial = Some(s),
        }
        if with_own(k, |p| p.has_deliverable(tid)) {
            with_table(ns, |t| t.cancel(id, tid));
            return Err(Errno::EINTR);
        }
        k.block(None);
    }
}

/// msgsnd(msqid, msgp, msgsz, msgflg)
///
/// `msqid` and `msgflg` are `int`s. `EINVAL` if `msgsz` is above
/// [`MSGMAX`] (or negative), `EFAULT` if the `mtype` and `msgsz` bytes of
/// text at `msgp` cannot be read, `EINVAL` if `mtype` is below 1; the
/// table does the rest ([`MsgTable::send`]), and a full queue without
/// `IPC_NOWAIT` is waited on.
pub fn msgsnd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, size, flags) = (a.int(0), a.ulong(2), a.uint(3));
    if size > MSGMAX {
        return Err(Errno::EINVAL);
    }
    let mut bytes = vec![0u8; (MTYPE + size) as usize];
    k.copy_from_user(a.ptr(1), &mut bytes)?;
    let mut mtype = [0u8; MTYPE as usize];
    mtype.copy_from_slice(&bytes[..MTYPE as usize]);
    let msg = Msg { mtype: i64::from_ne_bytes(mtype), text: bytes.split_off(MTYPE as usize) };
    if msg.mtype < 1 {
        return Err(Errno::EINVAL);
    }
    let (ns, _, who) = caller(k);
    let (pid, tid, now) = (k.pid(), k.tid(), now(k));
    wait_for(k, &ns, id, |t, serial| t.send(id, serial, &msg, flags, &who, pid, tid, now)).map(|()| 0)
}

/// msgrcv(msqid, msgp, msgsz, msgtyp, msgflg)
///
/// `msqid` and `msgflg` are `int`s. `EINVAL` if `msgsz` is negative, or
/// for [`MSG_COPY`] without [`IPC_NOWAIT`] or with [`MSG_EXCEPT`]; the
/// table chooses the message ([`MsgTable::receive`]), and none without
/// `IPC_NOWAIT` is waited on. Its `mtype` and text go to `msgp`
/// (`EFAULT`: the message is gone all the same); returns the bytes of
/// text written.
pub fn msgrcv(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, size, msgtyp, flags) = (a.int(0), a.long(2), a.long(3), a.uint(4));
    let Ok(size) = usize::try_from(size) else { return Err(Errno::EINVAL) };
    if flags & MSG_COPY != 0 && (flags & IPC_NOWAIT == 0 || flags & MSG_EXCEPT != 0) {
        return Err(Errno::EINVAL);
    }
    let (ns, _, who) = caller(k);
    let (pid, tid, now) = (k.pid(), k.tid(), now(k));
    let msg = wait_for(k, &ns, id, |t, serial| t.receive(id, serial, size, msgtyp, flags, &who, pid, tid, now))?;
    let mut bytes = Vec::with_capacity(MTYPE as usize + msg.text.len());
    bytes.extend_from_slice(&msg.mtype.to_ne_bytes());
    bytes.extend_from_slice(&msg.text);
    k.copy_to_user(a.ptr(1), &bytes)?;
    Ok(msg.text.len() as u64)
}

/// msgctl(msqid, cmd, buf)
///
/// msgctl(2), for `IPC_STAT`, `IPC_SET` and `IPC_RMID` (`IPC_64` in `cmd`
/// is ignored: x86-64 has one layout of `struct msqid_ds`). An id that
/// names no queue is `EINVAL`, and so is every other command (`IPC_INFO`,
/// `MSG_INFO`, `MSG_STAT` are not implemented).
///
/// - `IPC_STAT` needs read permission (`EACCES`) and writes the queue's
///   `struct msqid64_ds` to `buf`.
/// - `IPC_SET` and `IPC_RMID` need the owner, the creator or
///   `CAP_SYS_ADMIN` (`EPERM`).
/// - `IPC_SET` takes the owner's uid and gid, the permission bits and
///   `msg_qbytes` from the one at `buf` (`EPERM` for a `msg_qbytes` above
///   [`MSGMNB`] without `CAP_SYS_RESOURCE`); `msg_ctime` is now. Waiting
///   callers try again.
/// - `IPC_RMID` removes the queue and its messages at once; its waiting
///   callers fail with `EIDRM`.
pub fn msgctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, cmd) = (a.int(0), a.uint(1) & !IPC_64);
    let (ns, creds, who) = caller(k);
    match cmd {
        IPC_STAT => {
            let mut ds = with_table(&ns, |t| {
                let q = t.queue(id).ok_or(Errno::EINVAL)?;
                if !q.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok(MsqidDs::of(q))
            })?;
            ds.uid = creds.uid_view(ds.uid);
            ds.gid = creds.gid_view(ds.gid);
            ds.cuid = creds.uid_view(ds.cuid);
            ds.cgid = creds.gid_view(ds.cgid);
            ds.lspid = if ds.lspid == 0 { 0 } else { view(k, ds.lspid) };
            ds.lrpid = if ds.lrpid == 0 { 0 } else { view(k, ds.lrpid) };
            k.copy_to_user(a.ptr(2), &ds.to_bytes())?;
            Ok(0)
        }
        IPC_SET => {
            let mut bytes = [0u8; MsqidDs::SIZE];
            k.copy_from_user(a.ptr(2), &mut bytes)?;
            let (uid, gid, mode, qbytes) = MsqidDs::set_from_bytes(&bytes);
            let resource = creds.capable(cap::SYS_RESOURCE);
            let now = now(k);
            let woken = with_table(&ns, |t| {
                let q = t.queue_mut(id).ok_or(Errno::EINVAL)?;
                if !q.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                if qbytes > MSGMNB && !resource {
                    return Err(Errno::EPERM);
                }
                let (uid, gid) = (creds.kuid(uid)?, creds.kgid(gid)?);
                q.uid = uid;
                q.gid = gid;
                q.mode = mode & 0o777;
                q.qbytes = qbytes;
                q.ctime = now;
                Ok(t.waiters(id))
            })?;
            wake(k, woken);
            Ok(0)
        }
        IPC_RMID => {
            let woken = with_table(&ns, |t| {
                let q = t.queue(id).ok_or(Errno::EINVAL)?;
                if !q.owned_by(&who) {
                    return Err(Errno::EPERM);
                }
                t.remove(id)
            })?;
            wake(k, woken);
            Ok(0)
        }
        _ => Err(Errno::EINVAL),
    }
}
