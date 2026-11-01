//! System V message queues: msgget(2), msgsnd(2), msgrcv(2), msgctl(2).
//!
//! The queues of the caller's IPC namespace are in its
//! [`MsgTable`](crate::ipc::msg::MsgTable), which decides under its lock
//! and completes sleeping callers itself; the handlers only read and write
//! user memory, sleep and wake threads, outside the lock.

use crate::args::Args;
use crate::cred::cap;
use crate::errno::{Errno, SysResult};
use crate::ipc::msg::{
    IPC_INFO, IPC_NOWAIT, MSG_COPY, MSG_EXCEPT, MSG_INFO, MSG_STAT, MSG_STAT_ANY, MSGMAX, Message, MsqidDs, Outcome,
    Want, msginfo_bytes, with_table,
};
use crate::ipc::shm::{IPC_RMID, IPC_SET, IPC_STAT, READ};
use crate::kernel::{LinuxKernel, UserPtr};
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

/// The size of `long mtype` at the start of `struct msgbuf`.
const MTYPE: u64 = 8;

/// Wake the threads a table operation completed.
fn wake(k: &mut dyn LinuxKernel, tids: Vec<i32>) {
    for tid in tids {
        k.unblock(tid);
    }
}

/// Sleep on `ticket` until a waker completes it or a signal is to be
/// acted on: `EINTR` then, never restarted (signal(7) lists msgsnd and
/// msgrcv among the calls that fail after a handler, `SA_RESTART` or
/// not), unless the result came meanwhile.
fn sleep(k: &mut dyn LinuxKernel, ns: &Option<Arc<IpcNs>>, ticket: u64) -> Result<Option<Message>, Errno> {
    let tid = k.tid();
    loop {
        // A signal sent from now on notifies the thread and ends the
        // block; one already pending is seen below.
        k.clear_signal_notification();
        if let Some(result) = with_table(ns, |t| t.poll(ticket)) {
            return result;
        }
        if with_own(k, |p| p.has_deliverable(tid)) {
            return with_table(ns, |t| t.cancel(ticket)).unwrap_or(Err(Errno::EINTR));
        }
        k.block(None);
    }
}

/// msgget(key, msgflg)
///
/// Both are `int`s (the upper halves of the registers are ignored). The
/// table decides ([`MsgTable::get`](crate::ipc::msg::MsgTable::get): the
/// order of the errors is there).
pub fn msgget(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (key, flags) = (a.int(0), a.uint(1));
    let (ns, _, who) = caller(k);
    let now = now(k);
    with_table(&ns, |t| t.get(key, flags, &who, now)).map(|id| id as u64)
}

/// msgsnd(msqid, msgp, msgsz, msgflg)
///
/// `msqid` and `msgflg` are `int`s, `msgsz` a full `size_t`. In order:
/// `EFAULT` if `mtype` cannot be read; `EINVAL` for a negative id or a
/// size above [`MSGMAX`]; `EINVAL` for an `mtype` below 1; `EFAULT` if the
/// text cannot be read; then the table
/// ([`MsgTable::send`](crate::ipc::msg::MsgTable::send)). A sender that
/// does not fit sleeps until a waker sends its message for it, the queue
/// is removed (`EIDRM`) or a signal comes (`EINTR`).
pub fn msgsnd(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, msgp, size, flags) = (a.int(0), a.ptr(1), a.ulong(2), a.uint(3));
    let mut mtype = [0u8; 8];
    k.copy_from_user(msgp, &mut mtype)?;
    let mtype = i64::from_ne_bytes(mtype);
    if id < 0 || size > MSGMAX {
        return Err(Errno::EINVAL);
    }
    if mtype < 1 {
        return Err(Errno::EINVAL);
    }
    let mut text = vec![0u8; size as usize];
    if size > 0 {
        k.copy_from_user(UserPtr(msgp.0.wrapping_add(MTYPE)), &mut text)?;
    }
    let (ns, _, who) = caller(k);
    let (pid, tid, now) = (k.pid(), k.tid(), now(k));
    let msg = Message { mtype, text };
    match with_table(&ns, |t| t.send(id, msg, flags & IPC_NOWAIT != 0, &who, (pid, tid), now))? {
        Outcome::Done((), woken) => {
            wake(k, woken);
            Ok(0)
        }
        Outcome::Sleep(ticket) => sleep(k, &ns, ticket).map(|_| 0),
    }
}

/// msgrcv(msqid, msgp, msgsz, msgtyp, msgflg)
///
/// `msqid` and `msgflg` are `int`s, `msgsz` a full `size_t`, `msgtyp` a
/// full `long`. In order: `EINVAL` for a negative id or a size with the top
/// bit set; `EINVAL` for `MSG_COPY` without `IPC_NOWAIT` or with
/// `MSG_EXCEPT`; then the table
/// ([`MsgTable::receive`](crate::ipc::msg::MsgTable::receive)), sleeping
/// as msgsnd does. The message has left the queue when it is copied out:
/// `EFAULT` then loses it. Returns the text bytes copied.
pub fn msgrcv(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, msgp, size, msgtyp, flags) = (a.int(0), a.ptr(1), a.ulong(2), a.long(3), a.uint(4));
    if id < 0 || (size as i64) < 0 {
        return Err(Errno::EINVAL);
    }
    if flags & MSG_COPY != 0 && (flags & IPC_NOWAIT == 0 || flags & MSG_EXCEPT != 0) {
        return Err(Errno::EINVAL);
    }
    let (ns, _, who) = caller(k);
    let (pid, tid, now) = (k.pid(), k.tid(), now(k));
    let want = Want { msgtyp, flags, size };
    let msg = match with_table(&ns, |t| t.receive(id, want, &who, pid, tid, now))? {
        Outcome::Done(msg, woken) => {
            wake(k, woken);
            msg
        }
        Outcome::Sleep(ticket) => sleep(k, &ns, ticket)?.ok_or(Errno::EIDRM)?,
    };
    let mut out = Vec::with_capacity(MTYPE as usize + msg.text.len());
    out.extend_from_slice(&msg.mtype.to_ne_bytes());
    out.extend_from_slice(&msg.text);
    k.copy_to_user(msgp, &out)?;
    Ok(msg.text.len() as u64)
}

/// msgctl(msqid, cmd, buf)
///
/// msgctl(2). `msqid` and `cmd` are `int`s; a negative one is `EINVAL`
/// for every command, and so is an unknown command (also one with the
/// `IPC_64` bit). Read permission (`EACCES`) for `IPC_STAT` and
/// `MSG_STAT`; `IPC_SET` and `IPC_RMID` need the owner, the creator or
/// `CAP_SYS_ADMIN` (`EPERM`).
///
/// - `IPC_STAT` writes the queue's `struct msqid64_ds` to `buf`.
/// - `IPC_SET` reads one (`EFAULT` before the id is looked up) and takes
///   the owner's uid and gid, the permission bits and `msg_qbytes` (above
///   `MSGMNB` only with `CAP_SYS_RESOURCE`, `EPERM`); `msg_ctime` is now.
/// - `IPC_RMID` removes the queue and its messages; sleepers get `EIDRM`.
/// - `IPC_INFO` and `MSG_INFO` write a `struct msginfo` and return the
///   highest index in use; `MSG_STAT` and `MSG_STAT_ANY` take an index for
///   the id and return the id (`MSG_STAT_ANY` needs no permission).
pub fn msgctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (id, cmd, buf) = (a.int(0), a.int(1), a.ptr(2));
    if id < 0 || cmd < 0 {
        return Err(Errno::EINVAL);
    }
    let cmd = cmd as u32;
    let (ns, creds, who) = caller(k);
    match cmd {
        IPC_STAT | MSG_STAT | MSG_STAT_ANY => {
            let found = with_table(&ns, |t| {
                let id = if cmd == IPC_STAT { id } else { t.by_index(id).ok_or(Errno::EINVAL)? };
                let q = t.queue(id).ok_or(Errno::EINVAL)?;
                if cmd != MSG_STAT_ANY && !q.allows(&who, READ) {
                    return Err(Errno::EACCES);
                }
                Ok((id, MsqidDs::of(q)))
            });
            let (id, mut ds) = found?;
            ds.uid = creds.uid_view(ds.uid);
            ds.gid = creds.gid_view(ds.gid);
            ds.cuid = creds.uid_view(ds.cuid);
            ds.cgid = creds.gid_view(ds.cgid);
            ds.lspid = if ds.lspid == 0 { 0 } else { view(k, ds.lspid) };
            ds.lrpid = if ds.lrpid == 0 { 0 } else { view(k, ds.lrpid) };
            k.copy_to_user(buf, &ds.to_bytes())?;
            Ok(if cmd == IPC_STAT { 0 } else { id as u64 })
        }
        IPC_SET => {
            let mut bytes = [0u8; MsqidDs::SIZE];
            k.copy_from_user(buf, &mut bytes)?;
            let set = MsqidDs::set_from_bytes(&bytes);
            let sys_resource = creds.capable(cap::SYS_RESOURCE);
            let now = now(k);
            let map = |uid, gid| Ok((creds.kuid(uid)?, creds.kgid(gid)?));
            let woken = with_table(&ns, |t| t.set(id, set, &who, sys_resource, map, now))?;
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
        IPC_INFO | MSG_INFO => {
            let (info, max) =
                with_table(&ns, |t| (msginfo_bytes(if cmd == MSG_INFO { Some(t) } else { None }), t.max_index()));
            k.copy_to_user(buf, &info)?;
            Ok(max as u64)
        }
        _ => Err(Errno::EINVAL),
    }
}
