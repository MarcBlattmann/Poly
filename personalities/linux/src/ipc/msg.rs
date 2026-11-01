//! System V message queues (msgget(2), msgsnd(2), msgrcv(2), msgctl(2),
//! svipc(7)): the queues of one IPC namespace and the callers sleeping on
//! them.
//!
//! A queue belongs to its IPC namespace: it lives in the namespace's
//! [`MsgTable`] from msgget(2) until msgctl(`IPC_RMID`), which removes it
//! and its messages at once. Ids are handed out in increasing order
//! (wrapping at `i32::MAX`), so the id just removed is not the next one;
//! the "index" the `*_STAT` commands take is an id modulo 32768.
//!
//! A message fits a queue while `msg_cbytes + size <= msg_qbytes` and
//! `msg_qnum + 1 <= msg_qbytes` (msg_qbytes caps the count of messages
//! too). A sender that does not fit sleeps; a receiver that finds no
//! message of the kind it asks for sleeps. Sleepers never retry
//! themselves: whoever changes the queue completes them before it returns
//! and they only collect the result (as [`super::sem`] does):
//!
//! - a sent message goes straight to the oldest sleeping receiver it
//!   matches whose buffer is large enough (or that said `MSG_NOERROR`), and
//!   never counts in `msg_qnum`; a matching receiver whose buffer is too
//!   small fails with `E2BIG` on the way and the message goes on;
//! - a receive, or an `IPC_SET` that raises `msg_qbytes`, lets the oldest
//!   sleeping senders whose message now fits send it (each re-checked for
//!   write permission, as the woken sender would);
//! - an `IPC_SET` that takes read permission away fails sleeping receivers
//!   with `EACCES`;
//! - `IPC_RMID` fails every sleeper with `EIDRM`.
//!
//! The table sits behind the namespace lock ([`crate::ns::lock::Mutex`], a
//! spinlock): handlers decide under it and call the kernel outside it.

use super::index_of;
use crate::errno::Errno;
use crate::ipc::shm::{IPC_CREAT, IPC_EXCL, IPC_PRIVATE, READ, WRITE, Who};
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Limits (`linux/msg.h`, the defaults of `/proc/sys/kernel/msg*`): the
/// largest message, the default `msg_qbytes`, and the most queues.
pub const MSGMAX: u64 = 8192;
pub const MSGMNB: u64 = 16384;
pub const MSGMNI: usize = 32000;
/// The other `struct msginfo` values `IPC_INFO` reports (`linux/msg.h`).
pub const MSGPOOL: i32 = (MSGMNI as i32 * MSGMNB as i32) / 1024;
pub const MSGMAP: i32 = MSGMNB as i32;
pub const MSGSSZ: i32 = 16;
pub const MSGTQL: i32 = MSGMNB as i32;
pub const MSGSEG: u16 = 0xffff;

/// msgsnd(2) / msgrcv(2) flags (`linux/ipc.h`, `linux/msg.h`).
pub const IPC_NOWAIT: u32 = 0o4000;
pub const MSG_NOERROR: u32 = 0o10000;
pub const MSG_EXCEPT: u32 = 0o20000;
pub const MSG_COPY: u32 = 0o40000;

/// msgctl commands beyond `IPC_RMID`, `IPC_SET`, `IPC_STAT` (`linux/ipc.h`,
/// `linux/msg.h`).
pub const IPC_INFO: u32 = 3;
pub const MSG_STAT: u32 = 11;
pub const MSG_INFO: u32 = 12;
pub const MSG_STAT_ANY: u32 = 13;

/// One message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// 1 and up.
    pub mtype: i64,
    pub text: Vec<u8>,
}

/// One queue.
#[derive(Clone, Debug)]
pub struct Queue {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    /// Permission bits.
    pub mode: u16,
    /// Seconds since the epoch of the last msgsnd, msgrcv and change.
    pub stime: i64,
    pub rtime: i64,
    pub ctime: i64,
    /// The byte limit (`msg_qbytes`).
    pub qbytes: u64,
    /// Global pids of the last sender and receiver.
    pub lspid: i32,
    pub lrpid: i32,
    pub messages: VecDeque<Message>,
    /// Names this queue among all queues the table ever had.
    serial: u64,
}

impl Queue {
    /// Whether `who` may have the access in `request` ([`super::allows`]).
    pub fn allows(&self, who: &Who, request: u32) -> bool {
        super::allows(self.mode, (self.uid, self.cuid), (self.gid, self.cgid), who, request)
    }

    /// Whether `who` may change or remove it (`IPC_SET`, `IPC_RMID`): its
    /// owner, its creator, or a caller with `CAP_SYS_ADMIN`.
    pub fn owned_by(&self, who: &Who) -> bool {
        who.euid == self.uid || who.euid == self.cuid || who.sys_admin
    }

    /// `msg_qnum`.
    pub fn qnum(&self) -> u64 {
        self.messages.len() as u64
    }

    /// `msg_cbytes`.
    pub fn cbytes(&self) -> u64 {
        self.messages.iter().map(|m| m.text.len() as u64).sum()
    }

    /// Whether one more message of `size` bytes fits.
    pub fn fits(&self, size: u64) -> bool {
        self.cbytes() + size <= self.qbytes && self.qnum() < self.qbytes
    }
}

/// What a receive asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    /// msgrcv(2)'s `msgtyp`.
    pub msgtyp: i64,
    /// Its `msgflg`.
    pub flags: u32,
    /// The text bytes the buffer holds.
    pub size: u64,
}

impl Want {
    /// Whether a message of type `mtype` is one this receive takes
    /// (msgrcv(2): 0 any, above 0 that type or with `MSG_EXCEPT` any
    /// other, below 0 any type up to its absolute value).
    pub fn matches(&self, mtype: i64) -> bool {
        match self.msgtyp {
            0 => true,
            t if t > 0 && self.flags & MSG_EXCEPT != 0 => mtype != t,
            t if t > 0 => mtype == t,
            t => mtype <= t.checked_neg().unwrap_or(i64::MAX),
        }
    }

    /// The index of the message in `q` this receive takes now: the first
    /// that matches, for a negative `msgtyp` the first of the lowest type.
    fn select(&self, q: &Queue) -> Option<usize> {
        if self.msgtyp < 0 {
            let mut best: Option<(usize, i64)> = None;
            for (i, m) in q.messages.iter().enumerate() {
                if self.matches(m.mtype) && best.is_none_or(|(_, t)| m.mtype < t) {
                    best = Some((i, m.mtype));
                }
            }
            return best.map(|(i, _)| i);
        }
        q.messages.iter().position(|m| self.matches(m.mtype))
    }

    /// `msg` as this receive gets it: `E2BIG` if it is longer than the
    /// buffer without `MSG_NOERROR`, else cut to the buffer.
    fn take(&self, mut msg: Message) -> Result<Message, (Errno, Message)> {
        if msg.text.len() as u64 > self.size {
            if self.flags & MSG_NOERROR == 0 {
                return Err((Errno::E2BIG, msg));
            }
            msg.text.truncate(self.size as usize);
        }
        Ok(msg)
    }
}

/// What a sleeper waits to do.
#[derive(Debug)]
enum Kind {
    Send(Message),
    Recv(Want),
}

/// A caller sleeping in msgsnd or msgrcv.
#[derive(Debug)]
struct Sleeper {
    ticket: u64,
    tid: i32,
    pid: i32,
    who: Who,
    /// The queue's serial.
    serial: u64,
    kind: Kind,
    /// Set by whoever completes it: a receiver's message, or the error.
    result: Option<Result<Option<Message>, Errno>>,
}

/// What msgsnd or msgrcv does after the table decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    /// Done with this result (a received message, or `()` for a send);
    /// wake these threads (sleepers it completed).
    Done(T, Vec<i32>),
    /// Sleep; collect the result with [`MsgTable::poll`] under this ticket.
    Sleep(u64),
}

/// The message queues of one IPC namespace, by id.
#[derive(Debug, Default)]
pub struct MsgTable {
    queues: BTreeMap<i32, Queue>,
    /// The id handed out last (`-1` before the first).
    last: i32,
    serials: u64,
    tickets: u64,
    sleepers: Vec<Sleeper>,
}

impl MsgTable {
    pub const fn new() -> MsgTable {
        MsgTable { queues: BTreeMap::new(), last: -1, serials: 0, tickets: 0, sleepers: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.queues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    pub fn queue(&self, id: i32) -> Option<&Queue> {
        self.queues.get(&id)
    }

    pub fn queue_mut(&mut self, id: i32) -> Option<&mut Queue> {
        self.queues.get_mut(&id)
    }

    /// The id of the queue at `index` ([`index_of`]).
    pub fn by_index(&self, index: i32) -> Option<i32> {
        self.queues.keys().copied().find(|id| index_of(*id) == index)
    }

    /// The highest index in use, 0 if none (what `IPC_INFO` returns).
    pub fn max_index(&self) -> i32 {
        self.queues.keys().map(|id| index_of(*id)).max().unwrap_or(0)
    }

    /// Messages and bytes queued in all queues (`MSG_INFO`).
    pub fn totals(&self) -> (u64, u64) {
        self.queues.values().fold((0, 0), |(n, b), q| (n + q.qnum(), b + q.cbytes()))
    }

    /// msgget(2) of `key` and `flags` (the low 32 bits of `msgflg`), made
    /// by `who` at `now`: [`IPC_PRIVATE`] always makes a new queue; another
    /// key that no queue has is `ENOENT` without [`IPC_CREAT`], else a new
    /// queue; a key in use is `EEXIST` with `IPC_CREAT | IPC_EXCL`, then
    /// `EACCES` if the access asked for in `flags & 0777` is not granted.
    /// A new queue needs room ([`MSGMNI`]: `ENOSPC`); its mode is
    /// `flags & 0777`, its byte limit [`MSGMNB`].
    pub fn get(&mut self, key: i32, flags: u32, who: &Who, now: i64) -> Result<i32, Errno> {
        if key != IPC_PRIVATE {
            match self.queues.iter().find(|(_, q)| q.key == key) {
                Some((id, q)) => {
                    if flags & (IPC_CREAT | IPC_EXCL) == IPC_CREAT | IPC_EXCL {
                        return Err(Errno::EEXIST);
                    }
                    if !q.allows(who, flags) {
                        return Err(Errno::EACCES);
                    }
                    return Ok(*id);
                }
                None if flags & IPC_CREAT == 0 => return Err(Errno::ENOENT),
                None => {}
            }
        }
        if self.queues.len() >= MSGMNI {
            return Err(Errno::ENOSPC);
        }
        let mut id = self.last;
        loop {
            id = if id == i32::MAX { 0 } else { id + 1 };
            if !self.queues.contains_key(&id) {
                break;
            }
        }
        self.last = id;
        self.serials += 1;
        let queue = Queue {
            key,
            uid: who.euid,
            gid: who.egid,
            cuid: who.euid,
            cgid: who.egid,
            mode: (flags & 0o777) as u16,
            stime: 0,
            rtime: 0,
            ctime: now,
            qbytes: MSGMNB,
            lspid: 0,
            lrpid: 0,
            messages: VecDeque::new(),
            serial: self.serials,
        };
        self.queues.insert(id, queue);
        Ok(id)
    }

    /// Put `msg`, sent by process `pid` at `now`, into queue `id`, which
    /// has room for it: to the oldest sleeping receiver it matches that
    /// can take it, else at the tail. Receivers whose buffer is too small
    /// fail with `E2BIG` on the way. Returns the threads to wake.
    fn deliver(&mut self, id: i32, msg: Message, pid: i32, now: i64) -> Vec<i32> {
        let mut woken = Vec::new();
        let Some(serial) = self.queues.get(&id).map(|q| q.serial) else { return woken };
        let mut msg = Some(msg);
        for s in self.sleepers.iter_mut().filter(|s| s.serial == serial && s.result.is_none()) {
            let Kind::Recv(want) = s.kind else { continue };
            let Some(m) = msg.take() else { break };
            if !want.matches(m.mtype) {
                msg = Some(m);
                continue;
            }
            match want.take(m) {
                Ok(m) => {
                    s.result = Some(Ok(Some(m)));
                    woken.push(s.tid);
                    if let Some(q) = self.queues.get_mut(&id) {
                        q.lrpid = s.pid;
                        q.rtime = now;
                    }
                }
                Err((e, m)) => {
                    s.result = Some(Err(e));
                    woken.push(s.tid);
                    msg = Some(m);
                }
            }
        }
        if let Some(q) = self.queues.get_mut(&id) {
            if let Some(m) = msg {
                q.messages.push_back(m);
            }
            q.lspid = pid;
            q.stime = now;
        }
        woken
    }

    /// Complete the sleeping senders of queue `id` that may send now, at
    /// `now`, oldest first: one that lost write permission fails with
    /// `EACCES`, one whose message fits sends it. Returns the threads to
    /// wake.
    fn flush_senders(&mut self, id: i32, now: i64) -> Vec<i32> {
        let mut woken = Vec::new();
        let Some(serial) = self.queues.get(&id).map(|q| q.serial) else { return woken };
        let mut i = 0;
        while i < self.sleepers.len() {
            let s = &self.sleepers[i];
            if s.serial != serial || s.result.is_some() {
                i += 1;
                continue;
            }
            let Kind::Send(msg) = &s.kind else {
                i += 1;
                continue;
            };
            let q = &self.queues[&id];
            if !q.allows(&s.who, WRITE) {
                self.sleepers[i].result = Some(Err(Errno::EACCES));
                woken.push(self.sleepers[i].tid);
            } else if q.fits(msg.text.len() as u64) {
                let (tid, pid) = (s.tid, s.pid);
                let Kind::Send(msg) =
                    core::mem::replace(&mut self.sleepers[i].kind, Kind::Recv(Want { msgtyp: 0, flags: 0, size: 0 }))
                else {
                    unreachable!()
                };
                self.sleepers[i].result = Some(Ok(None));
                woken.push(tid);
                woken.extend(self.deliver(id, msg, pid, now));
            }
            i += 1;
        }
        woken
    }

    /// msgsnd(2) of `msg` (checked and read already) to queue `id` by
    /// `who`, thread `tid` of process `pid`, at `now`. `EINVAL` if no queue
    /// has the id, `EACCES` without write permission; then a message that
    /// does not fit is `EAGAIN` with `nowait`, else the caller sleeps.
    pub fn send(
        &mut self,
        id: i32,
        msg: Message,
        nowait: bool,
        who: &Who,
        (pid, tid): (i32, i32),
        now: i64,
    ) -> Result<Outcome<()>, Errno> {
        let q = self.queues.get(&id).ok_or(Errno::EINVAL)?;
        if !q.allows(who, WRITE) {
            return Err(Errno::EACCES);
        }
        if !q.fits(msg.text.len() as u64) {
            if nowait {
                return Err(Errno::EAGAIN);
            }
            self.tickets += 1;
            let serial = q.serial;
            self.sleepers.push(Sleeper {
                ticket: self.tickets,
                tid,
                pid,
                who: who.clone(),
                serial,
                kind: Kind::Send(msg),
                result: None,
            });
            return Ok(Outcome::Sleep(self.tickets));
        }
        Ok(Outcome::Done((), self.deliver(id, msg, pid, now)))
    }

    /// msgrcv(2) from queue `id` of what `want` asks for, by `who`, thread
    /// `tid` of process `pid`, at `now` (the flag checks are done). In
    /// order: `EINVAL` if no queue has the id, `EACCES` without read
    /// permission; with `MSG_COPY` a copy of the message at position
    /// `msgtyp` (`ENOMSG` past the end); else the message selected
    /// ([`Want::matches`]): `E2BIG` if it is too long (it stays), else it
    /// leaves the queue and sleeping senders get their chance. Without a
    /// message: `ENOMSG` with `IPC_NOWAIT`, else the caller sleeps.
    pub fn receive(
        &mut self,
        id: i32,
        want: Want,
        who: &Who,
        pid: i32,
        tid: i32,
        now: i64,
    ) -> Result<Outcome<Message>, Errno> {
        let q = self.queues.get(&id).ok_or(Errno::EINVAL)?;
        if !q.allows(who, READ) {
            return Err(Errno::EACCES);
        }
        if want.flags & MSG_COPY != 0 {
            let m = usize::try_from(want.msgtyp).ok().and_then(|i| q.messages.get(i)).ok_or(Errno::ENOMSG)?;
            return want.take(m.clone()).map(|m| Outcome::Done(m, Vec::new())).map_err(|(e, _)| e);
        }
        let Some(i) = want.select(q) else {
            if want.flags & IPC_NOWAIT != 0 {
                return Err(Errno::ENOMSG);
            }
            self.tickets += 1;
            let serial = q.serial;
            self.sleepers.push(Sleeper {
                ticket: self.tickets,
                tid,
                pid,
                who: who.clone(),
                serial,
                kind: Kind::Recv(want),
                result: None,
            });
            return Ok(Outcome::Sleep(self.tickets));
        };
        if q.messages[i].text.len() as u64 > want.size && want.flags & MSG_NOERROR == 0 {
            return Err(Errno::E2BIG);
        }
        let q = self.queues.get_mut(&id).ok_or(Errno::EINVAL)?;
        let msg = q.messages.remove(i).ok_or(Errno::EINVAL)?;
        q.lrpid = pid;
        q.rtime = now;
        let msg = want.take(msg).map_err(|(e, _)| e)?;
        Ok(Outcome::Done(msg, self.flush_senders(id, now)))
    }

    /// The result of the sleeper with `ticket` if it has one (it then
    /// leaves the queue).
    pub fn poll(&mut self, ticket: u64) -> Option<Result<Option<Message>, Errno>> {
        let i = self.sleepers.iter().position(|s| s.ticket == ticket && s.result.is_some())?;
        self.sleepers.remove(i).result
    }

    /// The sleeper with `ticket` gives up (a signal): it leaves the queue.
    /// Its result if it got one meanwhile, which then stands.
    pub fn cancel(&mut self, ticket: u64) -> Option<Result<Option<Message>, Errno>> {
        let i = self.sleepers.iter().position(|s| s.ticket == ticket)?;
        self.sleepers.remove(i).result
    }

    /// msgctl(`IPC_SET`) of queue `id` by `who`, after the ids were
    /// mapped by `map` (which says `EINVAL` for ids the caller's user
    /// namespace lacks): `EINVAL` if there is no such queue, `EPERM` unless
    /// `who` owns it, `EPERM` for a `qbytes` above [`MSGMNB`] without
    /// `CAP_SYS_RESOURCE` (`sys_resource`), then the ids. Senders that fit
    /// now send, receivers that lost read permission fail with `EACCES`.
    /// Returns the threads to wake.
    pub fn set(
        &mut self,
        id: i32,
        (uid, gid, mode, qbytes): (u32, u32, u16, u64),
        who: &Who,
        sys_resource: bool,
        map: impl FnOnce(u32, u32) -> Result<(u32, u32), Errno>,
        now: i64,
    ) -> Result<Vec<i32>, Errno> {
        let q = self.queues.get_mut(&id).ok_or(Errno::EINVAL)?;
        if !q.owned_by(who) {
            return Err(Errno::EPERM);
        }
        if qbytes > MSGMNB && !sys_resource {
            return Err(Errno::EPERM);
        }
        let (uid, gid) = map(uid, gid)?;
        q.uid = uid;
        q.gid = gid;
        q.mode = mode & 0o777;
        q.qbytes = qbytes;
        q.ctime = now;
        let serial = q.serial;
        let q = q.clone();
        let mut woken = Vec::new();
        for s in self.sleepers.iter_mut().filter(|s| s.serial == serial && s.result.is_none()) {
            if matches!(s.kind, Kind::Recv(_)) && !q.allows(&s.who, READ) {
                s.result = Some(Err(Errno::EACCES));
                woken.push(s.tid);
            }
        }
        woken.extend(self.flush_senders(id, now));
        Ok(woken)
    }

    /// msgctl(`IPC_RMID`) of queue `id`: it goes at once with its messages;
    /// its sleepers fail with `EIDRM`. Returns the threads to wake
    /// (`EINVAL` if no queue has the id).
    pub fn remove(&mut self, id: i32) -> Result<Vec<i32>, Errno> {
        let q = self.queues.remove(&id).ok_or(Errno::EINVAL)?;
        let mut woken = Vec::new();
        for s in self.sleepers.iter_mut().filter(|s| s.serial == q.serial && s.result.is_none()) {
            s.result = Some(Err(Errno::EIDRM));
            woken.push(s.tid);
        }
        Ok(woken)
    }

    /// Whether the queue a ticket waits on still exists (for tests).
    #[cfg(test)]
    fn sleeping(&self, ticket: u64) -> bool {
        self.sleepers
            .iter()
            .any(|s| s.ticket == ticket && s.result.is_none() && self.queues.values().any(|q| q.serial == s.serial))
    }
}

/// The queues of the initial IPC namespace.
static INITIAL: Mutex<MsgTable> = Mutex::new(MsgTable::new());

/// Run `f` on the message queue table of the IPC namespace `ns` (`None`:
/// the initial one), locked. `f` must not block or call the kernel.
pub fn with_table<R>(ns: &Option<Arc<IpcNs>>, f: impl FnOnce(&mut MsgTable) -> R) -> R {
    match ns {
        Some(ns) => f(&mut ns.msg().lock()),
        None => f(&mut INITIAL.lock()),
    }
}

/// `struct msqid64_ds` on x86-64 (`asm-generic/msgbuf.h`, with `struct
/// ipc64_perm` from `asm-generic/ipcbuf.h`): what `IPC_STAT` writes and
/// `IPC_SET` reads. Ids and pids as the caller sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MsqidDs {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub mode: u16,
    pub seq: u16,
    pub stime: i64,
    pub rtime: i64,
    pub ctime: i64,
    pub cbytes: u64,
    pub qnum: u64,
    pub qbytes: u64,
    pub lspid: i32,
    pub lrpid: i32,
}

impl MsqidDs {
    /// `sizeof(struct msqid64_ds)`.
    pub const SIZE: usize = 120;

    pub fn of(q: &Queue) -> MsqidDs {
        MsqidDs {
            key: q.key,
            uid: q.uid,
            gid: q.gid,
            cuid: q.cuid,
            cgid: q.cgid,
            mode: q.mode,
            seq: 0,
            stime: q.stime,
            rtime: q.rtime,
            ctime: q.ctime,
            cbytes: q.cbytes(),
            qnum: q.qnum(),
            qbytes: q.qbytes,
            lspid: q.lspid,
            lrpid: q.lrpid,
        }
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.key.to_ne_bytes());
        b[4..8].copy_from_slice(&self.uid.to_ne_bytes());
        b[8..12].copy_from_slice(&self.gid.to_ne_bytes());
        b[12..16].copy_from_slice(&self.cuid.to_ne_bytes());
        b[16..20].copy_from_slice(&self.cgid.to_ne_bytes());
        b[20..22].copy_from_slice(&self.mode.to_ne_bytes());
        b[24..26].copy_from_slice(&self.seq.to_ne_bytes());
        b[48..56].copy_from_slice(&self.stime.to_ne_bytes());
        b[56..64].copy_from_slice(&self.rtime.to_ne_bytes());
        b[64..72].copy_from_slice(&self.ctime.to_ne_bytes());
        b[72..80].copy_from_slice(&self.cbytes.to_ne_bytes());
        b[80..88].copy_from_slice(&self.qnum.to_ne_bytes());
        b[88..96].copy_from_slice(&self.qbytes.to_ne_bytes());
        b[96..100].copy_from_slice(&self.lspid.to_ne_bytes());
        b[100..104].copy_from_slice(&self.lrpid.to_ne_bytes());
        b
    }

    /// What `IPC_SET` takes from the structure: `msg_perm.uid`,
    /// `msg_perm.gid`, `msg_perm.mode` and `msg_qbytes`.
    pub fn set_from_bytes(b: &[u8; Self::SIZE]) -> (u32, u32, u16, u64) {
        let word = |at: usize| u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let mut q = [0u8; 8];
        q.copy_from_slice(&b[88..96]);
        (word(4), word(8), u16::from_ne_bytes([b[20], b[21]]), u64::from_ne_bytes(q))
    }
}

/// `struct msginfo` (`linux/msg.h`), 32 bytes: what `IPC_INFO` (the
/// limits) and `MSG_INFO` (with use counts in `msgpool`, `msgmap` and
/// `msgtql`) write.
pub fn msginfo_bytes(table: Option<&MsgTable>) -> [u8; 32] {
    let (pool, map, tql) = match table {
        Some(t) => {
            let (n, b) = t.totals();
            (t.len() as i32, n as i32, b as i32)
        }
        None => (MSGPOOL, MSGMAP, MSGTQL),
    };
    let ints = [pool, map, MSGMAX as i32, MSGMNB as i32, MSGMNI as i32, MSGSSZ, tql];
    let mut b = [0u8; 32];
    for (i, v) in ints.iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
    }
    b[28..30].copy_from_slice(&MSGSEG.to_ne_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(uid: u32) -> Who {
        Who { euid: uid, egid: uid, groups: Arc::from([]), ipc_owner: false, sys_admin: false, ipc_lock: false }
    }

    fn msg(mtype: i64, text: &[u8]) -> Message {
        Message { mtype, text: text.to_vec() }
    }

    fn want(msgtyp: i64, flags: u32, size: u64) -> Want {
        Want { msgtyp, flags, size }
    }

    const ME: i32 = 100;

    fn table() -> (MsgTable, i32, Who) {
        let mut t = MsgTable::new();
        let who = user(1001);
        let id = t.get(IPC_PRIVATE, 0o600, &who, 1_000).unwrap();
        (t, id, who)
    }

    fn send(t: &mut MsgTable, id: i32, m: Message, nowait: bool, who: &Who) -> Result<Outcome<()>, Errno> {
        t.send(id, m, nowait, who, (ME, ME), 2_000)
    }

    fn recv(t: &mut MsgTable, id: i32, w: Want, who: &Who) -> Result<Message, Errno> {
        match t.receive(id, w, who, ME, ME, 3_000)? {
            Outcome::Done(m, _) => Ok(m),
            Outcome::Sleep(_) => panic!("sleeps"),
        }
    }

    #[test]
    fn get_rules() {
        let mut t = MsgTable::new();
        let me = user(1001);
        assert_eq!(t.get(7, 0o600, &me, 0), Err(Errno::ENOENT));
        let id = t.get(7, 0o640 | IPC_CREAT, &me, 5).unwrap();
        assert_eq!(t.get(7, 0, &me, 0), Ok(id));
        assert_eq!(t.get(7, 0o066, &me, 0), Ok(id));
        assert_eq!(t.get(7, 0o100, &me, 0), Err(Errno::EACCES));
        assert_eq!(t.get(7, 0o700 | IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        let q = t.queue(id).unwrap();
        assert_eq!((q.mode, q.qbytes, q.qnum(), q.ctime, q.stime), (0o640, MSGMNB, 0, 5, 0));
        let p = t.get(IPC_PRIVATE, IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        assert_ne!(p, id);
        t.remove(p).unwrap();
        assert_ne!(t.get(IPC_PRIVATE, 0, &me, 0).unwrap(), p);
        t.remove(id).unwrap();
        assert_eq!(t.get(7, 0, &me, 0), Err(Errno::ENOENT));
    }

    #[test]
    fn selection_and_sizes() {
        let (mut t, id, me) = table();
        for (ty, text) in [(5, &b"five"[..]), (2, b"two"), (9, b"nine"), (2, b"two2"), (1, b"one")] {
            assert!(matches!(send(&mut t, id, msg(ty, text), true, &me), Ok(Outcome::Done(..))));
        }
        let q = t.queue(id).unwrap();
        assert_eq!((q.qnum(), q.cbytes(), q.lspid, q.stime), (5, 18, ME, 2_000));
        assert_eq!(recv(&mut t, id, want(2, 0, 10), &me), Ok(msg(2, b"two")));
        assert_eq!(recv(&mut t, id, want(5, MSG_EXCEPT, 10), &me), Ok(msg(9, b"nine")));
        assert_eq!(recv(&mut t, id, want(-4, 0, 10), &me), Ok(msg(1, b"one")));
        assert_eq!(recv(&mut t, id, want(i64::MIN, 0, 10), &me), Ok(msg(2, b"two2")));
        assert_eq!(recv(&mut t, id, want(3, IPC_NOWAIT, 10), &me), Err(Errno::ENOMSG));
        assert_eq!(recv(&mut t, id, want(0, 0, 3), &me), Err(Errno::E2BIG));
        assert_eq!(recv(&mut t, id, want(0, MSG_COPY | IPC_NOWAIT, 10), &me), Ok(msg(5, b"five")));
        assert_eq!(recv(&mut t, id, want(1, MSG_COPY | IPC_NOWAIT, 10), &me), Err(Errno::ENOMSG));
        assert_eq!(t.queue(id).unwrap().lrpid, ME);
        assert_eq!(recv(&mut t, id, want(0, MSG_NOERROR, 2), &me), Ok(msg(5, b"fi")));
        assert_eq!(t.queue(id).unwrap().qnum(), 0);
    }

    #[test]
    fn room_counts_bytes_and_messages() {
        let (mut t, id, me) = table();
        assert!(send(&mut t, id, msg(1, &[0; 8192]), true, &me).is_ok());
        assert!(send(&mut t, id, msg(1, &[0; 8192]), true, &me).is_ok());
        assert_eq!(send(&mut t, id, msg(1, b"x"), true, &me), Err(Errno::EAGAIN));
        assert!(send(&mut t, id, msg(1, b""), true, &me).is_ok());
        t.queue_mut(id).unwrap().qbytes = 3;
        t.queue_mut(id).unwrap().messages.clear();
        for _ in 0..3 {
            assert!(send(&mut t, id, msg(1, b""), true, &me).is_ok());
        }
        assert_eq!(send(&mut t, id, msg(1, b""), true, &me), Err(Errno::EAGAIN));
        t.queue_mut(id).unwrap().mode = 0o400;
        assert_eq!(send(&mut t, id, msg(1, b""), true, &me), Err(Errno::EACCES));
        assert_eq!(send(&mut t, id + 1, msg(1, b""), true, &me), Err(Errno::EINVAL));
    }

    #[test]
    fn sleeping_receiver_gets_the_message_directly() {
        let (mut t, id, me) = table();
        let Ok(Outcome::Sleep(r)) = t.receive(id, want(2, 0, 100), &me, 200, 201, 0) else { panic!() };
        assert!(t.sleeping(r));
        assert_eq!(send(&mut t, id, msg(1, b"one"), false, &me), Ok(Outcome::Done((), Vec::new())));
        assert_eq!(t.poll(r), None);
        assert_eq!(send(&mut t, id, msg(2, b"two"), false, &me), Ok(Outcome::Done((), alloc::vec![201])));
        assert_eq!(t.poll(r), Some(Ok(Some(msg(2, b"two")))));
        let q = t.queue(id).unwrap();
        assert_eq!((q.qnum(), q.lrpid, q.rtime, q.lspid), (1, 200, 2_000, ME));
        // A receiver with a small buffer fails and the message is queued.
        t.queue_mut(id).unwrap().messages.clear();
        let Ok(Outcome::Sleep(r)) = t.receive(id, want(0, 0, 2), &me, 200, 201, 0) else { panic!() };
        assert_eq!(send(&mut t, id, msg(5, b"toolong"), false, &me), Ok(Outcome::Done((), alloc::vec![201])));
        assert_eq!(t.poll(r), Some(Err(Errno::E2BIG)));
        assert_eq!(t.queue(id).unwrap().qnum(), 1);
    }

    #[test]
    fn sleeping_sender_goes_when_room_comes() {
        let (mut t, id, me) = table();
        t.queue_mut(id).unwrap().qbytes = 10;
        assert!(send(&mut t, id, msg(1, &[0; 10]), true, &me).is_ok());
        let Ok(Outcome::Sleep(s)) = t.send(id, msg(2, &[1; 5]), false, &me, (300, 301), 0) else { panic!() };
        let woken = t.set(id, (1001, 1001, 0o600, 15), &me, false, |u, g| Ok((u, g)), 4_000).unwrap();
        assert_eq!(woken, alloc::vec![301]);
        assert_eq!(t.poll(s), Some(Ok(None)));
        let q = t.queue(id).unwrap();
        assert_eq!((q.qnum(), q.cbytes(), q.lspid), (2, 15, 300));
        let Ok(Outcome::Sleep(s)) = t.send(id, msg(3, b"x"), false, &me, (300, 301), 0) else { panic!() };
        let Ok(Outcome::Done(_, woken)) = t.receive(id, want(0, 0, 100), &me, ME, ME, 0) else { panic!() };
        assert_eq!((woken, t.poll(s)), (alloc::vec![301], Some(Ok(None))));
    }

    #[test]
    fn set_rules_and_removal() {
        let (mut t, id, me) = table();
        let other = user(2002);
        let ok = |u, g| Ok((u, g));
        assert_eq!(t.set(id, (1, 1, 0o600, 10), &other, false, ok, 0), Err(Errno::EPERM));
        assert_eq!(t.set(id, (1, 1, 0o600, MSGMNB + 1), &me, false, ok, 0), Err(Errno::EPERM));
        assert_eq!(t.set(id, (1, 1, 0o600, 10), &me, false, |_, _| Err(Errno::EINVAL), 0), Err(Errno::EINVAL));
        assert_eq!(t.queue(id).unwrap().qbytes, MSGMNB);
        assert!(t.set(id, (1001, 1001, 0o600, MSGMNB + 1), &me, true, ok, 0).is_ok());
        let Ok(Outcome::Sleep(r)) = t.receive(id, want(0, 0, 10), &me, 400, 401, 0) else { panic!() };
        let Ok(Outcome::Sleep(r2)) = t.receive(id, want(0, 0, 10), &me, 402, 403, 0) else { panic!() };
        assert_eq!(t.cancel(r2), None);
        assert_eq!(t.remove(id), Ok(alloc::vec![401]));
        assert_eq!(t.poll(r), Some(Err(Errno::EIDRM)));
        assert_eq!(t.remove(id), Err(Errno::EINVAL));
    }

    #[test]
    fn indexes_and_layouts() {
        let (t, id, _) = table();
        assert_eq!((t.by_index(index_of(id)), t.by_index(999), t.max_index()), (Some(id), None, index_of(id)));
        let ds = MsqidDs { key: -3, uid: 1, gid: 2, qbytes: 77, lspid: 9, lrpid: 8, ..MsqidDs::default() };
        let b = ds.to_bytes();
        assert_eq!(&b[96..104], &[9, 0, 0, 0, 8, 0, 0, 0]);
        assert_eq!(MsqidDs::set_from_bytes(&b), (1, 2, 0, 77));
        let info = msginfo_bytes(None);
        assert_eq!(&info[0..4], &512_000i32.to_ne_bytes());
        assert_eq!(&info[28..30], &[0xff, 0xff]);
    }
}
