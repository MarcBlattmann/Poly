//! System V message queues (msgget(2), msgsnd(2), msgrcv(2), msgctl(2),
//! svipc(7)): the queues of one IPC namespace.
//!
//! A queue belongs to its IPC namespace, not to a process: it lives in the
//! namespace's [`MsgTable`] from msgget(2) until msgctl(`IPC_RMID`), which
//! removes it and releases its key at once. Ids are handed out in
//! increasing order (wrapping at `i32::MAX`), so the id just removed is not
//! the next one.
//!
//! A caller that has to wait (a full queue for msgsnd, no matching message
//! for msgrcv) is put on the queue's list of waiters; whoever changes the
//! queue afterwards (a send, a receive, `IPC_SET`, `IPC_RMID`) gets the
//! list back to wake, and every waiter tries again. Each queue has a
//! serial that is never reused, so a waiter whose queue was removed (even
//! if its id has come back since) learns `EIDRM`.
//!
//! The table sits behind the namespace lock ([`crate::ns::lock::Mutex`], a
//! spinlock): handlers decide under it and call the kernel outside it.

use crate::errno::Errno;
use crate::ipc::shm::{IPC_CREAT, IPC_EXCL, IPC_PRIVATE, READ, WRITE, Who};
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Limits (`linux/msg.h`, the defaults of `/proc/sys/kernel/msgmni`,
/// `msgmax` and `msgmnb`): queues in the namespace, bytes in one message,
/// and the default `msg_qbytes` of a new queue.
pub const MSGMNI: usize = 32000;
pub const MSGMAX: u64 = 8192;
pub const MSGMNB: u64 = 16384;

/// `msgflg` bits (`linux/ipc.h`, `linux/msg.h`).
pub const IPC_NOWAIT: u32 = 0o4000;
pub const MSG_NOERROR: u32 = 0o10000;
pub const MSG_EXCEPT: u32 = 0o20000;
pub const MSG_COPY: u32 = 0o40000;

/// One message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msg {
    /// Positive.
    pub mtype: i64,
    pub text: Vec<u8>,
}

/// One queue.
#[derive(Clone, Debug)]
pub struct MsgQueue {
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
    /// Global pids of the last sender and receiver (0: none yet).
    pub lspid: i32,
    pub lrpid: i32,
    /// The most bytes of text the queue holds.
    pub qbytes: u64,
    pub msgs: VecDeque<Msg>,
    /// Bytes of text on the queue.
    cbytes: u64,
    /// Names this queue among all queues the table ever had.
    serial: u64,
    /// Threads waiting to send to or receive from it.
    waiters: Vec<i32>,
}

impl MsgQueue {
    /// Whether `who` may have the access in `request` ([`super::allows`]).
    pub fn allows(&self, who: &Who, request: u32) -> bool {
        super::allows(self.mode, (self.uid, self.cuid), (self.gid, self.cgid), who, request)
    }

    /// Whether `who` may change or remove it (`IPC_SET`, `IPC_RMID`):
    /// its owner, its creator, or a caller with `CAP_SYS_ADMIN`.
    pub fn owned_by(&self, who: &Who) -> bool {
        who.euid == self.uid || who.euid == self.cuid || who.sys_admin
    }

    /// `msg_qnum`.
    pub fn qnum(&self) -> u64 {
        self.msgs.len() as u64
    }

    /// `msg_cbytes`.
    pub fn cbytes(&self) -> u64 {
        self.cbytes
    }

    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// Whether a message of `len` bytes fits: the text on the queue may
    /// not exceed `msg_qbytes`, nor may the number of messages (so that
    /// empty messages cannot fill it without end).
    fn fits(&self, len: u64) -> bool {
        self.cbytes + len <= self.qbytes && self.qnum() < self.qbytes
    }

    /// The index of the message msgrcv(2) takes for `msgtyp` and `flags`:
    /// 0, the first; above 0, the first of that type (with [`MSG_EXCEPT`],
    /// the first of another type); below 0, the first of the lowest type
    /// up to `-msgtyp`.
    fn pick(&self, msgtyp: i64, flags: u32) -> Option<usize> {
        if msgtyp == 0 {
            return if self.msgs.is_empty() { None } else { Some(0) };
        }
        if msgtyp > 0 {
            let except = flags & MSG_EXCEPT != 0;
            return self.msgs.iter().position(|m| (m.mtype == msgtyp) != except);
        }
        let limit = msgtyp.unsigned_abs();
        let mut best: Option<(usize, i64)> = None;
        for (i, m) in self.msgs.iter().enumerate() {
            if m.mtype.unsigned_abs() <= limit && best.is_none_or(|(_, t)| m.mtype < t) {
                best = Some((i, m.mtype));
            }
        }
        best.map(|(i, _)| i)
    }

    fn take_waiters(&mut self) -> Vec<i32> {
        core::mem::take(&mut self.waiters)
    }

    fn add_waiter(&mut self, tid: i32) {
        if !self.waiters.contains(&tid) {
            self.waiters.push(tid);
        }
    }
}

/// What msgsnd and msgrcv do after asking the table.
#[derive(Debug, PartialEq, Eq)]
pub enum Step<T> {
    /// Done: the result, and the threads to wake.
    Done(T, Vec<i32>),
    /// Sleep until woken, then ask again for the queue with this serial.
    Wait(u64),
}

/// The message queues of one IPC namespace, by id.
#[derive(Debug, Default)]
pub struct MsgTable {
    queues: BTreeMap<i32, MsgQueue>,
    /// The id handed out last (`-1` before the first).
    last: i32,
    serials: u64,
}

impl MsgTable {
    pub const fn new() -> MsgTable {
        MsgTable { queues: BTreeMap::new(), last: -1, serials: 0 }
    }

    pub fn len(&self) -> usize {
        self.queues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    pub fn queue(&self, id: i32) -> Option<&MsgQueue> {
        self.queues.get(&id)
    }

    pub fn queue_mut(&mut self, id: i32) -> Option<&mut MsgQueue> {
        self.queues.get_mut(&id)
    }

    /// The live queue with `key` (never [`IPC_PRIVATE`]).
    fn by_key(&self, key: i32) -> Option<(i32, &MsgQueue)> {
        self.queues.iter().find(|(_, q)| q.key == key).map(|(id, q)| (*id, q))
    }

    /// msgget(2) of `key` and `flags` (the low 32 bits of `msgflg`), by
    /// `who` at `now`:
    ///
    /// - [`IPC_PRIVATE`]: always a new queue, whatever the flags;
    /// - another key that no queue has: `ENOENT` without [`IPC_CREAT`],
    ///   else a new queue;
    /// - a key in use: `EEXIST` with `IPC_CREAT | IPC_EXCL`, then `EACCES`
    ///   if the access asked for in `flags & 0777` is not granted; else
    ///   its id, the queue unchanged.
    ///
    /// A new queue needs room in the namespace ([`MSGMNI`] queues:
    /// `ENOSPC`); it is empty, owned and made by `who`, with mode
    /// `flags & 0777`, `msg_qbytes` [`MSGMNB`] and `msg_ctime` `now`.
    pub fn get(&mut self, key: i32, flags: u32, who: &Who, now: i64) -> Result<i32, Errno> {
        if key != IPC_PRIVATE {
            match self.by_key(key) {
                Some((id, q)) => {
                    if flags & (IPC_CREAT | IPC_EXCL) == IPC_CREAT | IPC_EXCL {
                        return Err(Errno::EEXIST);
                    }
                    if !q.allows(who, flags) {
                        return Err(Errno::EACCES);
                    }
                    return Ok(id);
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
        let queue = MsgQueue {
            key,
            uid: who.euid,
            gid: who.egid,
            cuid: who.euid,
            cgid: who.egid,
            mode: (flags & 0o777) as u16,
            stime: 0,
            rtime: 0,
            ctime: now,
            lspid: 0,
            lrpid: 0,
            qbytes: MSGMNB,
            msgs: VecDeque::new(),
            cbytes: 0,
            serial: self.serials,
            waiters: Vec::new(),
        };
        self.queues.insert(id, queue);
        Ok(id)
    }

    /// The queue `id` for a call that first saw serial `serial` (`None`:
    /// a first attempt): `EINVAL` if no queue has the id on a first
    /// attempt, `EIDRM` if the queue the caller waited on is gone.
    fn lookup(&mut self, id: i32, serial: Option<u64>) -> Result<&mut MsgQueue, Errno> {
        match (self.queues.get_mut(&id), serial) {
            (Some(q), Some(s)) if q.serial != s => Err(Errno::EIDRM),
            (Some(q), _) => Ok(q),
            (None, None) => Err(Errno::EINVAL),
            (None, Some(_)) => Err(Errno::EIDRM),
        }
    }

    /// msgsnd(2) of `msg` (already read and checked) to queue `id`, by
    /// `who`, thread `tid` of process `pid`, at `now`; `serial` as for
    /// [`Self::lookup`]. `EACCES` without write permission; if the message
    /// does not fit, `EAGAIN` with [`IPC_NOWAIT`], else wait. A sent
    /// message records `pid` as `msg_lspid` and `now` as `msg_stime`.
    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &mut self,
        id: i32,
        serial: Option<u64>,
        msg: &Msg,
        flags: u32,
        who: &Who,
        pid: i32,
        tid: i32,
        now: i64,
    ) -> Result<Step<()>, Errno> {
        let q = self.lookup(id, serial)?;
        if !q.allows(who, WRITE) {
            return Err(Errno::EACCES);
        }
        if !q.fits(msg.text.len() as u64) {
            if flags & IPC_NOWAIT != 0 {
                return Err(Errno::EAGAIN);
            }
            q.add_waiter(tid);
            return Ok(Step::Wait(q.serial));
        }
        q.cbytes += msg.text.len() as u64;
        q.msgs.push_back(msg.clone());
        q.lspid = pid;
        q.stime = now;
        Ok(Step::Done((), q.take_waiters()))
    }

    /// msgrcv(2) from queue `id` of at most `size` bytes of text, for
    /// `msgtyp` and `flags`, by `who`, thread `tid` of process `pid`, at
    /// `now`; `serial` as for [`Self::lookup`]. `EACCES` without read
    /// permission; then the message chosen ([`MSG_COPY`]: the one at index
    /// `msgtyp`, left on the queue) is `E2BIG` if longer than `size`
    /// without [`MSG_NOERROR`] (it stays), else returned cut to `size`;
    /// with none, `ENOMSG` with [`IPC_NOWAIT`], else wait. A taken message
    /// records `pid` as `msg_lrpid` and `now` as `msg_rtime`.
    #[allow(clippy::too_many_arguments)]
    pub fn receive(
        &mut self,
        id: i32,
        serial: Option<u64>,
        size: usize,
        msgtyp: i64,
        flags: u32,
        who: &Who,
        pid: i32,
        tid: i32,
        now: i64,
    ) -> Result<Step<Msg>, Errno> {
        let q = self.lookup(id, serial)?;
        if !q.allows(who, READ) {
            return Err(Errno::EACCES);
        }
        let copy = flags & MSG_COPY != 0;
        let at = if copy { usize::try_from(msgtyp).ok().filter(|i| *i < q.msgs.len()) } else { q.pick(msgtyp, flags) };
        let Some(at) = at else {
            if flags & IPC_NOWAIT != 0 {
                return Err(Errno::ENOMSG);
            }
            q.add_waiter(tid);
            return Ok(Step::Wait(q.serial));
        };
        if q.msgs[at].text.len() > size && flags & MSG_NOERROR == 0 {
            return Err(Errno::E2BIG);
        }
        let mut msg = if copy {
            q.msgs[at].clone()
        } else {
            let msg = q.msgs.remove(at).unwrap_or(Msg { mtype: 0, text: Vec::new() });
            q.cbytes -= msg.text.len() as u64;
            q.lrpid = pid;
            q.rtime = now;
            msg
        };
        msg.text.truncate(size);
        let woken = if copy { Vec::new() } else { q.take_waiters() };
        Ok(Step::Done(msg, woken))
    }

    /// Thread `tid` stops waiting on queue `id` (a signal came).
    pub fn cancel(&mut self, id: i32, tid: i32) {
        if let Some(q) = self.queues.get_mut(&id) {
            q.waiters.retain(|t| *t != tid);
        }
    }

    /// Take the waiters of queue `id` to wake (after a change).
    pub fn waiters(&mut self, id: i32) -> Vec<i32> {
        self.queues.get_mut(&id).map(MsgQueue::take_waiters).unwrap_or_default()
    }

    /// Remove queue `id` (`IPC_RMID`): its key is free at once and its
    /// messages are gone. Returns its waiters, which then learn `EIDRM`.
    pub fn remove(&mut self, id: i32) -> Result<Vec<i32>, Errno> {
        let mut q = self.queues.remove(&id).ok_or(Errno::EINVAL)?;
        Ok(q.take_waiters())
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
/// `IPC_SET` reads. Ids as the caller sees them.
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

    pub fn of(q: &MsgQueue) -> MsqidDs {
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
            cbytes: q.cbytes,
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
        let mut qbytes = [0u8; 8];
        qbytes.copy_from_slice(&b[88..96]);
        (word(4), word(8), u16::from_ne_bytes([b[20], b[21]]), u64::from_ne_bytes(qbytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(uid: u32, gid: u32) -> Who {
        Who { euid: uid, egid: gid, groups: Arc::from([]), ipc_owner: false, sys_admin: false, ipc_lock: false }
    }

    fn root() -> Who {
        Who { ipc_owner: true, sys_admin: true, ipc_lock: true, ..user(0, 0) }
    }

    const ME: (u32, u32) = (1001, 1001);
    const KEY: i32 = 0x4000_1234;

    fn me() -> Who {
        user(ME.0, ME.1)
    }

    fn msg(mtype: i64, text: &[u8]) -> Msg {
        Msg { mtype, text: text.to_vec() }
    }

    fn send(t: &mut MsgTable, id: i32, m: &Msg) -> Result<Step<()>, Errno> {
        t.send(id, None, m, IPC_NOWAIT, &me(), 100, 100, 5)
    }

    fn recv(t: &mut MsgTable, id: i32, size: usize, msgtyp: i64, flags: u32) -> Result<Msg, Errno> {
        match t.receive(id, None, size, msgtyp, flags | IPC_NOWAIT, &me(), 100, 100, 6)? {
            Step::Done(m, _) => Ok(m),
            Step::Wait(_) => unreachable!(),
        }
    }

    #[test]
    fn keyed_create_and_lookup() {
        // c_msgget_create_lookup
        let mut t = MsgTable::new();
        assert_eq!(t.get(KEY, 0o600, &me(), 1), Err(Errno::ENOENT));
        assert_eq!(t.get(KEY, 0o600 | IPC_EXCL, &me(), 1), Err(Errno::ENOENT));
        assert_eq!(t.get(KEY, 0, &me(), 1), Err(Errno::ENOENT));
        assert!(t.is_empty(), "a failed call creates nothing");
        let id = t.get(KEY, 0o600 | IPC_CREAT, &me(), 1).unwrap();
        assert!(id >= 0);
        for flags in [0, 0o600, 0o600 | IPC_CREAT, IPC_EXCL] {
            assert_eq!(t.get(KEY, flags, &me(), 2), Ok(id), "{flags:o}");
        }
        assert_eq!(t.get(KEY, 0o600 | IPC_CREAT | IPC_EXCL, &me(), 2), Err(Errno::EEXIST));
        let other = t.get(KEY + 1, 0o600 | IPC_CREAT | IPC_EXCL, &me(), 2).unwrap();
        assert_ne!(other, id);
        assert_eq!(t.get(KEY + 1, 0, &me(), 2), Ok(other));
        assert_eq!(t.queue(id).unwrap().key, KEY);
        assert_eq!(t.queue(other).unwrap().key, KEY + 1);
    }

    #[test]
    fn new_queue_fields() {
        // spec: new_queue_stat
        let mut t = MsgTable::new();
        let id = t.get(-7, IPC_CREAT | IPC_NOWAIT | 0o4000 | 0o7777 | 0x8000_0000, &user(5, 6), 1234).unwrap();
        let ds = MsqidDs::of(t.queue(id).unwrap());
        assert_eq!(
            ds,
            MsqidDs {
                key: -7,
                uid: 5,
                gid: 6,
                cuid: 5,
                cgid: 6,
                mode: 0o777,
                ctime: 1234,
                qbytes: MSGMNB,
                ..MsqidDs::default()
            }
        );
        let b = ds.to_bytes();
        assert_eq!(&b[0..4], &(-7i32).to_ne_bytes());
        assert_eq!(&b[20..22], &0o777u16.to_ne_bytes());
        assert_eq!(&b[64..72], &1234i64.to_ne_bytes());
        assert_eq!(&b[88..96], &MSGMNB.to_ne_bytes());
    }

    #[test]
    fn private_always_creates() {
        // c_msgget_ids; spec: private
        let mut t = MsgTable::new();
        let mut ids = Vec::new();
        for flags in [0o600, 0, IPC_CREAT | IPC_EXCL, IPC_EXCL, 0o777 | IPC_CREAT] {
            ids.push(t.get(IPC_PRIVATE, flags, &me(), 1).unwrap());
        }
        for _ in 0..40 {
            ids.push(t.get(IPC_PRIVATE, 0o600, &me(), 1).unwrap());
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "distinct");
        assert!(ids.iter().all(|i| *i >= 0));
        // Mode 0 is fine and no check is made for IPC_PRIVATE.
        let id = t.get(IPC_PRIVATE, 0, &user(2000, 2000), 1).unwrap();
        assert_eq!(t.queue(id).unwrap().mode, 0);
    }

    #[test]
    fn removed_id_is_not_next() {
        // c_msgget_ids, c_msgget_removed_key
        let mut t = MsgTable::new();
        let id = t.get(KEY, 0o600 | IPC_CREAT | IPC_EXCL, &me(), 1).unwrap();
        assert_eq!(send(&mut t, id, &msg(1, b"abc")), Ok(Step::Done((), Vec::new())));
        t.remove(id).unwrap();
        assert_eq!(t.get(KEY, 0o600, &me(), 1), Err(Errno::ENOENT));
        assert_eq!(send(&mut t, id, &msg(1, b"x")), Err(Errno::EINVAL));
        assert!(t.queue(id).is_none());
        let id2 = t.get(KEY, 0o600 | IPC_CREAT | IPC_EXCL, &me(), 1).unwrap();
        assert_ne!(id2, id);
        let q = t.queue(id2).unwrap();
        assert_eq!((q.qnum(), q.cbytes()), (0, 0));
        assert_eq!(recv(&mut t, id2, 16, 0, 0), Err(Errno::ENOMSG));
        assert_eq!(t.get(KEY, 0, &me(), 1), Ok(id2));
        t.remove(id2).unwrap();
        assert_eq!(t.get(KEY, 0, &me(), 1), Err(Errno::ENOENT));
        assert_eq!(t.remove(id2), Err(Errno::EINVAL));
    }

    #[test]
    fn existing_key_flags_and_order() {
        // c_msgget_existing_flags
        let mut t = MsgTable::new();
        let id = t.get(KEY, IPC_CREAT, &me(), 1).unwrap();
        assert_eq!(t.get(KEY, 0, &me(), 2), Ok(id));
        assert_eq!(t.get(KEY, IPC_CREAT, &me(), 2), Ok(id));
        assert_eq!(t.get(KEY, IPC_CREAT | 0o600, &me(), 2), Err(Errno::EACCES));
        assert_eq!(t.get(KEY, IPC_CREAT | IPC_EXCL | 0o600, &me(), 2), Err(Errno::EEXIST), "EEXIST first");
        assert_eq!(t.get(KEY, IPC_CREAT | IPC_EXCL, &me(), 2), Err(Errno::EEXIST));
        let id = t.get(KEY + 1, IPC_CREAT | 0o640, &me(), 1).unwrap();
        assert_eq!(t.get(KEY + 1, IPC_CREAT | 0o604, &me(), 2), Ok(id));
        let q = t.queue(id).unwrap();
        assert_eq!((q.mode, q.ctime), (0o640, 1), "a lookup changes nothing");
        assert_eq!(t.get(KEY + 2, 0o777, &me(), 1), Err(Errno::ENOENT));
        assert_eq!(t.get(KEY + 2, 0, &me(), 1), Err(Errno::ENOENT));
    }

    #[test]
    fn permission_bits_are_merged() {
        // c_msgget_permissions
        let mut t = MsgTable::new();
        let id = t.get(KEY, 0o400 | IPC_CREAT, &me(), 1).unwrap();
        for ask in [0o400, 0o004, 0o040, 0o444, 0] {
            assert_eq!(t.get(KEY, ask, &me(), 1), Ok(id), "{ask:o}");
        }
        for ask in [0o600, 0o200, 0o002, 0o100, 0o001] {
            assert_eq!(t.get(KEY, ask, &me(), 1), Err(Errno::EACCES), "{ask:o}");
        }
        let id = t.get(KEY + 1, 0o066 | IPC_CREAT, &me(), 1).unwrap();
        assert_eq!(t.get(KEY + 1, 0o004, &me(), 1), Err(Errno::EACCES));
        assert_eq!(t.get(KEY + 1, 0o040, &me(), 1), Err(Errno::EACCES));
        assert_eq!(t.get(KEY + 1, 0, &me(), 1), Ok(id));
        // Group and other classes of the queue's mode.
        assert_eq!(t.get(KEY + 1, 0o400, &user(2000, ME.1), 1), Ok(id), "group class");
        assert_eq!(t.get(KEY + 1, 0o400, &user(2000, 2000), 1), Ok(id), "other class");
        let id = t.get(KEY + 2, 0o700 | IPC_CREAT, &me(), 1).unwrap();
        assert_eq!(t.get(KEY + 2, 0o777, &me(), 1), Ok(id));
        assert_eq!(t.get(KEY + 2, 0o111, &me(), 1), Ok(id));
        assert_eq!(t.get(KEY + 2, 0o004, &user(2000, 2000), 1), Err(Errno::EACCES));
    }

    #[test]
    fn root_passes_every_check() {
        // spec: access (CAP_IPC_OWNER)
        let mut t = MsgTable::new();
        let id = t.get(KEY, IPC_CREAT, &me(), 1).unwrap();
        assert_eq!(t.get(KEY, 0o777, &root(), 1), Ok(id));
        assert_eq!(t.get(KEY, 0o777 | IPC_CREAT | IPC_EXCL, &root(), 1), Err(Errno::EEXIST));
    }

    #[test]
    fn current_mode_counts() {
        // c_msgget_ipc_set
        let mut t = MsgTable::new();
        let id = t.get(KEY, 0o600 | IPC_CREAT, &me(), 1).unwrap();
        t.queue_mut(id).unwrap().mode = 0o400;
        assert_eq!(t.get(KEY, 0o600, &me(), 1), Err(Errno::EACCES));
        assert_eq!(t.get(KEY, 0o400, &me(), 1), Ok(id));
        assert_eq!(t.get(KEY, 0o600 | IPC_CREAT, &me(), 1), Err(Errno::EACCES));
        t.queue_mut(id).unwrap().mode = 0o600;
        assert_eq!(t.get(KEY, 0o600, &me(), 1), Ok(id));
    }

    #[test]
    fn messages_go_to_their_queue_only() {
        // c_msgget_ids
        let mut t = MsgTable::new();
        let ids: Vec<i32> = (0..5).map(|_| t.get(IPC_PRIVATE, 0o600, &me(), 1).unwrap()).collect();
        send(&mut t, ids[2], &msg(5, b"q17")).unwrap();
        for (i, id) in ids.iter().enumerate() {
            let q = t.queue(*id).unwrap();
            assert_eq!((q.qnum(), q.cbytes()), if i == 2 { (1, 3) } else { (0, 0) });
        }
        let q = t.queue(ids[2]).unwrap();
        assert_eq!((q.lspid, q.stime, q.lrpid, q.rtime), (100, 5, 0, 0));
        assert_eq!(recv(&mut t, ids[2], 16, 0, 0), Ok(msg(5, b"q17")));
        let q = t.queue(ids[2]).unwrap();
        assert_eq!((q.qnum(), q.cbytes(), q.lrpid, q.rtime), (0, 0, 100, 6));
    }

    #[test]
    fn receive_selection_and_sizes() {
        let mut t = MsgTable::new();
        let id = t.get(IPC_PRIVATE, 0o600, &me(), 1).unwrap();
        for (ty, text) in [(3, &b"three"[..]), (1, b"one"), (2, b"two"), (1, b"uno")] {
            send(&mut t, id, &msg(ty, text)).unwrap();
        }
        assert_eq!(recv(&mut t, id, 16, 9, 0), Err(Errno::ENOMSG));
        assert_eq!(recv(&mut t, id, 16, 1, MSG_COPY), Ok(msg(1, b"one")), "copy by index");
        assert_eq!(recv(&mut t, id, 16, 4, MSG_COPY), Err(Errno::ENOMSG));
        assert_eq!(recv(&mut t, id, 2, 0, 0), Err(Errno::E2BIG));
        assert_eq!(t.queue(id).unwrap().qnum(), 4, "E2BIG leaves it");
        assert_eq!(recv(&mut t, id, 2, 3, MSG_EXCEPT | MSG_NOERROR), Ok(msg(1, b"on")));
        assert_eq!(recv(&mut t, id, 16, -2, 0), Ok(msg(1, b"uno")), "lowest type up to 2");
        assert_eq!(recv(&mut t, id, 16, 2, 0), Ok(msg(2, b"two")));
        assert_eq!(recv(&mut t, id, 16, 0, 0), Ok(msg(3, b"three")));
        assert_eq!(t.queue(id).unwrap().cbytes(), 0);
    }

    #[test]
    fn full_queue_waits_and_wakes() {
        let mut t = MsgTable::new();
        let id = t.get(IPC_PRIVATE, 0o600, &me(), 1).unwrap();
        t.queue_mut(id).unwrap().qbytes = 4;
        send(&mut t, id, &msg(1, b"abc")).unwrap();
        assert_eq!(send(&mut t, id, &msg(1, b"de")), Err(Errno::EAGAIN));
        let serial = match t.send(id, None, &msg(1, b"de"), 0, &me(), 200, 200, 5) {
            Ok(Step::Wait(s)) => s,
            r => panic!("{r:?}"),
        };
        match t.receive(id, None, 16, 0, 0, &me(), 100, 100, 6) {
            Ok(Step::Done(m, woken)) => {
                assert_eq!(m, msg(1, b"abc"));
                assert_eq!(woken, [200]);
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(t.send(id, Some(serial), &msg(1, b"de"), 0, &me(), 200, 200, 5), Ok(Step::Done((), Vec::new())));
        // A receiver waiting on a removed queue learns EIDRM.
        let serial = match t.receive(id, None, 16, 9, 0, &me(), 100, 100, 6) {
            Ok(Step::Wait(s)) => s,
            r => panic!("{r:?}"),
        };
        assert_eq!(t.remove(id), Ok(alloc::vec![100]));
        assert_eq!(t.receive(id, Some(serial), 16, 9, 0, &me(), 100, 100, 6), Err(Errno::EIDRM));
    }

    #[test]
    fn access_for_send_and_receive() {
        let mut t = MsgTable::new();
        let id = t.get(IPC_PRIVATE, 0o400, &me(), 1).unwrap();
        assert_eq!(send(&mut t, id, &msg(1, b"a")), Err(Errno::EACCES));
        assert_eq!(recv(&mut t, id, 16, 0, 0), Err(Errno::ENOMSG));
        t.queue_mut(id).unwrap().mode = 0o200;
        assert!(send(&mut t, id, &msg(1, b"a")).is_ok());
        assert_eq!(recv(&mut t, id, 16, 0, 0), Err(Errno::EACCES));
    }
}
