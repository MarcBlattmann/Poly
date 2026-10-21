//! System V semaphore sets (semget(2), semop(2), semctl(2), svipc(7)):
//! the sets of one IPC namespace.
//!
//! A set belongs to its IPC namespace: it lives in the namespace's
//! [`SemTable`] from semget(2) until semctl(`IPC_RMID`), which removes it
//! at once. Each set sits in a slot (`0..SEMMNI`, handed out in turn
//! rather than lowest free first); its id is the slot
//! plus [`SEQ_MULTIPLIER`] times a sequence number counted up with every
//! set made, so an id names one set only and a removed id stays invalid.
//!
//! The table also holds what blocked semop(2) callers wait for (each set's
//! queue of [`Pending`] operations), the results the table hands to them
//! when another call completes or ends their wait (by thread id), and
//! every process's `SEM_UNDO` adjustments (by process and set).
//!
//! Waiting follows the futex pattern: with the table locked, an operation
//! that cannot proceed is queued; the caller then lets go of the lock and
//! blocks. Whoever changes values afterwards runs the queue
//! ([`SemTable::update`]), completes what can now proceed on behalf of its
//! callers, leaves their results and names the threads to unblock.
//!
//! The table sits behind the namespace lock ([`crate::ns::lock::Mutex`], a
//! spinlock): handlers decide under it and copy to and from user memory
//! outside it.

use crate::errno::Errno;
pub use crate::ipc::shm::Who;
use crate::ipc::shm::{IPC_CREAT, IPC_EXCL, IPC_PRIVATE};
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// semctl commands (`linux/ipc.h`, `linux/sem.h`).
pub const IPC_RMID: i32 = 0;
pub const IPC_SET: i32 = 1;
pub const IPC_STAT: i32 = 2;
pub const IPC_INFO: i32 = 3;
pub const GETPID: i32 = 11;
pub const GETVAL: i32 = 12;
pub const GETALL: i32 = 13;
pub const GETNCNT: i32 = 14;
pub const GETZCNT: i32 = 15;
pub const SETVAL: i32 = 16;
pub const SETALL: i32 = 17;
pub const SEM_STAT: i32 = 18;
pub const SEM_INFO: i32 = 19;
pub const SEM_STAT_ANY: i32 = 20;

/// semop flags (`linux/ipc.h`, `linux/sem.h`).
pub const IPC_NOWAIT: i16 = 0o4000;
pub const SEM_UNDO: i16 = 0x1000;

/// Limits (`linux/sem.h`, and the defaults of `kernel.sem`): the most
/// sets, semaphores per set, semaphores in all, operations per semop
/// call, undo entries per process, the size of an undo structure, the
/// largest value and the largest undo adjustment.
pub const SEMMNI: usize = 32000;
pub const SEMMSL: i32 = 32000;
pub const SEMMNS: i32 = 1_024_000_000;
pub const SEMOPM: usize = 500;
pub const SEMUME: i32 = 500;
pub const SEMUSZ: i32 = 20;
pub const SEMVMX: i32 = 32767;
pub const SEMAEM: i32 = SEMVMX;

/// How many slots one sequence number covers: an id is
/// `slot + SEQ_MULTIPLIER * seq`, and `SEM_STAT` takes its index modulo
/// this.
pub const SEQ_MULTIPLIER: i32 = 32768;

/// The permission bits of a mode.
const MODE_BITS: u16 = 0o777;
/// Access requests as [`SemSet::allows`] takes them.
pub const READ: u32 = 4;
pub const ALTER: u32 = 2;

/// One semaphore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sem {
    /// `semval`: 0..=[`SEMVMX`].
    pub value: i32,
    /// `sempid`: the global pid of the last process that changed it.
    pub pid: i32,
}

/// One operation of semop(2) (`struct sembuf`, `linux/sem.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemBuf {
    pub num: u16,
    pub op: i16,
    pub flg: i16,
}

impl SemBuf {
    /// `sizeof(struct sembuf)`.
    pub const SIZE: usize = 6;

    pub fn from_bytes(b: &[u8]) -> SemBuf {
        SemBuf {
            num: u16::from_ne_bytes([b[0], b[1]]),
            op: i16::from_ne_bytes([b[2], b[3]]),
            flg: i16::from_ne_bytes([b[4], b[5]]),
        }
    }
}

/// What a blocked operation waits for: semaphore `num` to grow
/// (`semncnt`) or to be zero (`semzcnt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wait {
    pub num: u16,
    pub zero: bool,
}

/// A semop(2) call blocked on a set.
#[derive(Clone, Debug)]
pub struct Pending {
    pub tid: i32,
    pub pid: i32,
    pub ops: Vec<SemBuf>,
    pub wait: Wait,
}

/// One set.
#[derive(Clone, Debug)]
pub struct SemSet {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    /// Permission bits.
    pub mode: u16,
    pub seq: u16,
    /// Seconds since the epoch of the last semop(2) and the last change.
    pub otime: i64,
    pub ctime: i64,
    pub sems: Vec<Sem>,
    /// Blocked semop(2) calls, oldest first.
    pub queue: Vec<Pending>,
}

impl SemSet {
    /// A new set of `nsems` semaphores (all 0) with mode `flags & 0777`,
    /// made by `who` (owner and creator) at `now`.
    pub fn new(key: i32, nsems: usize, flags: u32, who: &Who, now: i64) -> SemSet {
        SemSet {
            key,
            uid: who.euid,
            gid: who.egid,
            cuid: who.euid,
            cgid: who.egid,
            mode: flags as u16 & MODE_BITS,
            seq: 0,
            otime: 0,
            ctime: now,
            sems: vec![Sem::default(); nsems],
            queue: Vec::new(),
        }
    }

    pub fn nsems(&self) -> usize {
        self.sems.len()
    }

    /// Whether `who` may have the access in `request` ([`READ`],
    /// [`ALTER`]): only the bits of the caller's class count (owner if
    /// its effective uid is the owner's or the creator's, else group if
    /// its effective or a supplementary gid is the set's group, else
    /// other). `CAP_IPC_OWNER` passes everything.
    pub fn allows(&self, who: &Who, request: u32) -> bool {
        if request == 0 || who.ipc_owner {
            return true;
        }
        let mode = u32::from(self.mode);
        let granted = if who.euid == self.uid || who.euid == self.cuid {
            mode >> 6
        } else if who.egid == self.gid || who.groups.contains(&self.gid) {
            mode >> 3
        } else {
            mode
        } & 7;
        request & !granted == 0
    }

    /// Whether `who` may change or remove it (`IPC_SET`, `IPC_RMID`): its
    /// owner, its creator, or a caller with `CAP_SYS_ADMIN`.
    pub fn owned_by(&self, who: &Who) -> bool {
        who.euid == self.uid || who.euid == self.cuid || who.sys_admin
    }

    /// How many blocked calls wait on semaphore `num` to grow (`zero`
    /// false: `GETNCNT`) or to be zero (`GETZCNT`).
    pub fn waiting(&self, num: usize, zero: bool) -> usize {
        self.queue.iter().filter(|p| usize::from(p.wait.num) == num && p.wait.zero == zero).count()
    }
}

/// How an attempt at a semop(2) operation list went.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Attempt {
    /// It can proceed: the new values and undo adjustments.
    Done(Vec<i32>, Option<Vec<i32>>),
    /// It must wait (the operation that cannot proceed, and its flags).
    Blocked(Wait, i16),
    Failed(Errno),
}

/// What [`SemTable::semop`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Semop {
    /// Completed.
    Done,
    /// Queued: block until the table has a result for the thread.
    Queued,
}

/// `struct seminfo` (`linux/sem.h`): what `IPC_INFO` and `SEM_INFO`
/// write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SemInfo {
    pub semmap: i32,
    pub semmni: i32,
    pub semmns: i32,
    pub semmnu: i32,
    pub semmsl: i32,
    pub semopm: i32,
    pub semume: i32,
    pub semusz: i32,
    pub semvmx: i32,
    pub semaem: i32,
}

impl SemInfo {
    /// `sizeof(struct seminfo)`.
    pub const SIZE: usize = 40;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let fields = [
            self.semmap,
            self.semmni,
            self.semmns,
            self.semmnu,
            self.semmsl,
            self.semopm,
            self.semume,
            self.semusz,
            self.semvmx,
            self.semaem,
        ];
        let mut b = [0u8; Self::SIZE];
        for (i, f) in fields.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&f.to_ne_bytes());
        }
        b
    }
}

/// The sets of one IPC namespace, by slot.
#[derive(Debug)]
pub struct SemTable {
    slots: Vec<Option<SemSet>>,
    /// The sequence number the next set gets.
    next_seq: u16,
    /// Where the search for the next set's slot starts. Slots are handed
    /// out in turn, after the one given last (wrapping at [`SEMMNI`]), not
    /// lowest free first, so a scan from index 0 meets empty slots below
    /// the newest sets, as on the oracle. It starts past slot 0, as in a
    /// namespace that has been used before (the oracle's shared one).
    next_slot: usize,
    /// `SEM_UNDO` adjustments: by (global pid, set id), one per semaphore.
    undo: BTreeMap<(i32, i32), Vec<i32>>,
    /// Results for blocked semop(2) callers, by thread id.
    results: BTreeMap<i32, Result<(), Errno>>,
}

impl Default for SemTable {
    fn default() -> SemTable {
        SemTable::new()
    }
}

impl SemTable {
    pub const fn new() -> SemTable {
        SemTable { slots: Vec::new(), next_seq: 0, next_slot: 1, undo: BTreeMap::new(), results: BTreeMap::new() }
    }

    fn slot_of(id: i32) -> Option<usize> {
        if id < 0 { None } else { Some((id % SEQ_MULTIPLIER) as usize) }
    }

    fn id_of(slot: usize, seq: u16) -> i32 {
        i32::from(seq) * SEQ_MULTIPLIER + slot as i32
    }

    /// The live set with id `id`.
    pub fn set(&self, id: i32) -> Option<&SemSet> {
        let slot = Self::slot_of(id)?;
        self.slots.get(slot)?.as_ref().filter(|s| Self::id_of(slot, s.seq) == id)
    }

    pub fn set_mut(&mut self, id: i32) -> Option<&mut SemSet> {
        let slot = Self::slot_of(id)?;
        self.slots.get_mut(slot)?.as_mut().filter(|s| Self::id_of(slot, s.seq) == id)
    }

    /// `SEM_STAT`'s lookup: the set in slot `index` modulo
    /// [`SEQ_MULTIPLIER`] and its id.
    pub fn by_index(&self, index: i32) -> Option<(i32, &SemSet)> {
        let slot = Self::slot_of(index)?;
        let s = self.slots.get(slot)?.as_ref()?;
        Some((Self::id_of(slot, s.seq), s))
    }

    fn live(&self) -> impl Iterator<Item = (usize, &SemSet)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
    }

    /// How many sets there are.
    pub fn len(&self) -> usize {
        self.live().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The highest slot in use (0 if none): what `IPC_INFO` and
    /// `SEM_INFO` return.
    pub fn max_index(&self) -> i32 {
        self.live().last().map_or(0, |(i, _)| i as i32)
    }

    /// What `IPC_INFO` (`sem_info` false) or `SEM_INFO` reports: the
    /// limits; `SEM_INFO` has the sets in use in `semusz` and their
    /// semaphores in `semaem`.
    pub fn info(&self, sem_info: bool) -> SemInfo {
        let mut i = SemInfo {
            semmap: SEMMNS,
            semmni: SEMMNI as i32,
            semmns: SEMMNS,
            semmnu: SEMMNS,
            semmsl: SEMMSL,
            semopm: SEMOPM as i32,
            semume: SEMUME,
            semusz: SEMUSZ,
            semvmx: SEMVMX,
            semaem: SEMAEM,
        };
        if sem_info {
            i.semusz = self.len() as i32;
            i.semaem = self.total_sems() as i32;
        }
        i
    }

    fn total_sems(&self) -> usize {
        self.live().map(|(_, s)| s.nsems()).sum()
    }

    /// The live set with `key` (never [`IPC_PRIVATE`]).
    fn by_key(&self, key: i32) -> Option<(i32, &SemSet)> {
        self.live().find(|(_, s)| s.key == key).map(|(i, s)| (Self::id_of(i, s.seq), s))
    }

    /// semget(2) of `key` with `nsems` semaphores and `flags` (the low 32
    /// bits of `semflg`), by `who` at `now`: the id of the set found or
    /// made. In this order:
    ///
    /// - `nsems` below 0 or above [`SEMMSL`]: `EINVAL`;
    /// - a key no set has (always for [`IPC_PRIVATE`]): `ENOENT` without
    ///   [`IPC_CREAT`] (not for `IPC_PRIVATE`), `EINVAL` for 0
    ///   semaphores, `ENOSPC` when [`SEMMNI`] sets or [`SEMMNS`]
    ///   semaphores would be exceeded; else a new set, mode `flags & 0777`;
    /// - a key in use: `EEXIST` with `IPC_CREAT | IPC_EXCL`, `EINVAL` if
    ///   `nsems` is above the set's, `EACCES` if the access asked for in
    ///   `flags & 0777` (any class) is not granted.
    pub fn semget(&mut self, key: i32, nsems: i32, flags: u32, who: &Who, now: i64) -> Result<i32, Errno> {
        if !(0..=SEMMSL).contains(&nsems) {
            return Err(Errno::EINVAL);
        }
        if key != IPC_PRIVATE {
            match self.by_key(key) {
                Some((id, s)) => {
                    if flags & (IPC_CREAT | IPC_EXCL) == IPC_CREAT | IPC_EXCL {
                        return Err(Errno::EEXIST);
                    }
                    if nsems as usize > s.nsems() {
                        return Err(Errno::EINVAL);
                    }
                    let r = flags & 0o777;
                    if !s.allows(who, (r >> 6 | r >> 3 | r) & 7) {
                        return Err(Errno::EACCES);
                    }
                    return Ok(id);
                }
                None if flags & IPC_CREAT == 0 => return Err(Errno::ENOENT),
                None => {}
            }
        }
        if nsems == 0 {
            return Err(Errno::EINVAL);
        }
        if self.len() >= SEMMNI || self.total_sems() + nsems as usize > SEMMNS as usize {
            return Err(Errno::ENOSPC);
        }
        let slot = (0..SEMMNI)
            .map(|i| (self.next_slot + i) % SEMMNI)
            .find(|&i| self.slots.get(i).is_none_or(Option::is_none))
            .expect("fewer than SEMMNI sets");
        if self.slots.len() <= slot {
            self.slots.resize_with(slot + 1, || None);
        }
        self.next_slot = (slot + 1) % SEMMNI;
        let mut set = SemSet::new(key, nsems as usize, flags, who, now);
        set.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let id = Self::id_of(slot, set.seq);
        self.slots[slot] = Some(set);
        Ok(id)
    }

    /// semctl(`IPC_RMID`) of set `id` (which must exist): it goes at once,
    /// with every process's adjustments for it; its blocked callers get
    /// `EIDRM`. Returns the threads to unblock.
    pub fn remove(&mut self, id: i32) -> Vec<i32> {
        let Some(slot) = Self::slot_of(id) else { return Vec::new() };
        if self.set(id).is_none() {
            return Vec::new();
        }
        let set = self.slots[slot].take().expect("checked");
        while self.slots.last().is_some_and(Option::is_none) {
            self.slots.pop();
        }
        self.undo.retain(|(_, s), _| *s != id);
        let mut woken = Vec::new();
        for p in set.queue {
            self.results.insert(p.tid, Err(Errno::EIDRM));
            woken.push(p.tid);
        }
        woken
    }

    /// Clear every process's adjustment of semaphore `num` of set `id`
    /// (all of them for `None`): `SETVAL` and `SETALL` do.
    fn clear_undo(&mut self, id: i32, num: Option<usize>) {
        for ((_, s), adj) in self.undo.iter_mut() {
            if *s == id {
                match num {
                    Some(n) => adj[n] = 0,
                    None => adj.iter_mut().for_each(|a| *a = 0),
                }
            }
        }
    }

    /// semctl(`SETVAL`) of semaphore `num` of set `id` (both checked) to
    /// `value` (checked) by process `pid` at `now`: the value, `sempid`
    /// and `sem_ctime` change, adjustments of the semaphore are cleared
    /// and blocked calls that can now proceed complete. Returns the
    /// threads to unblock.
    pub fn setval(&mut self, id: i32, num: usize, value: i32, pid: i32, now: i64) -> Vec<i32> {
        let Some(s) = self.set_mut(id) else { return Vec::new() };
        s.sems[num] = Sem { value, pid };
        s.ctime = now;
        self.clear_undo(id, Some(num));
        self.update(id, now)
    }

    /// semctl(`SETALL`) of set `id` (checked) to `values` (one per
    /// semaphore, checked) by process `pid` at `now`, as
    /// [`SemTable::setval`] for every semaphore.
    pub fn setall(&mut self, id: i32, values: &[i32], pid: i32, now: i64) -> Vec<i32> {
        let Some(s) = self.set_mut(id) else { return Vec::new() };
        for (sem, v) in s.sems.iter_mut().zip(values) {
            *sem = Sem { value: *v, pid };
        }
        s.ctime = now;
        self.clear_undo(id, None);
        self.update(id, now)
    }

    /// Try `ops` on set `id` (operations checked against the set) for
    /// process `pid`, without changing anything.
    fn attempt(&self, id: i32, pid: i32, ops: &[SemBuf]) -> Attempt {
        let s = self.set(id).expect("a live set");
        let mut values: Vec<i32> = s.sems.iter().map(|x| x.value).collect();
        let mut undo: Option<Vec<i32>> = None;
        for o in ops {
            let n = usize::from(o.num);
            let (cur, op) = (values[n], i32::from(o.op));
            if op == 0 {
                if cur != 0 {
                    return Attempt::Blocked(Wait { num: o.num, zero: true }, o.flg);
                }
                continue;
            }
            let new = cur + op;
            if new < 0 {
                return Attempt::Blocked(Wait { num: o.num, zero: false }, o.flg);
            }
            if new > SEMVMX {
                return Attempt::Failed(Errno::ERANGE);
            }
            if o.flg & SEM_UNDO != 0 {
                let adj = undo
                    .get_or_insert_with(|| self.undo.get(&(pid, id)).cloned().unwrap_or_else(|| vec![0; s.nsems()]));
                let a = adj[n] - op;
                if !(-(SEMAEM + 1)..=SEMAEM).contains(&a) {
                    return Attempt::Failed(Errno::ERANGE);
                }
                adj[n] = a;
            }
            values[n] = new;
        }
        Attempt::Done(values, undo)
    }

    /// Apply a [`Attempt::Done`] of `ops` by `pid` at `now`.
    fn commit(&mut self, id: i32, pid: i32, ops: &[SemBuf], values: Vec<i32>, undo: Option<Vec<i32>>, now: i64) {
        if let Some(adj) = undo {
            self.undo.insert((pid, id), adj);
        }
        let s = self.set_mut(id).expect("a live set");
        for (sem, v) in s.sems.iter_mut().zip(values) {
            sem.value = v;
        }
        for o in ops {
            s.sems[usize::from(o.num)].pid = pid;
        }
        s.otime = now;
    }

    /// semop(2) of `ops` on set `id` by thread `tid` of process `pid`
    /// (`who`) at `now`. `EINVAL` for no such set, `EFBIG` for a
    /// semaphore number past the set's, `EACCES` without alter permission
    /// (read permission if no operation changes a value), `ERANGE` for a
    /// value above [`SEMVMX`] or an adjustment beyond [`SEMAEM`], `EAGAIN`
    /// if it would wait and the operation that waits has [`IPC_NOWAIT`].
    /// Completed calls may let blocked ones proceed: the threads to
    /// unblock are added to `woken`.
    pub fn semop(
        &mut self,
        id: i32,
        ops: &[SemBuf],
        who: &Who,
        (tid, pid): (i32, i32),
        now: i64,
        woken: &mut Vec<i32>,
    ) -> Result<Semop, Errno> {
        let s = self.set(id).ok_or(Errno::EINVAL)?;
        if ops.iter().any(|o| usize::from(o.num) >= s.nsems()) {
            return Err(Errno::EFBIG);
        }
        let request = if ops.iter().any(|o| o.op != 0) { ALTER } else { READ };
        if !s.allows(who, request) {
            return Err(Errno::EACCES);
        }
        match self.attempt(id, pid, ops) {
            Attempt::Done(values, undo) => {
                let changed = ops.iter().any(|o| o.op != 0);
                self.commit(id, pid, ops, values, undo, now);
                if changed {
                    woken.extend(self.update(id, now));
                }
                Ok(Semop::Done)
            }
            Attempt::Failed(e) => Err(e),
            Attempt::Blocked(_, flg) if flg & IPC_NOWAIT != 0 => Err(Errno::EAGAIN),
            Attempt::Blocked(wait, _) => {
                self.results.remove(&tid);
                let s = self.set_mut(id).expect("a live set");
                s.queue.push(Pending { tid, pid, ops: ops.to_vec(), wait });
                Ok(Semop::Queued)
            }
        }
    }

    /// Run the queue of set `id` after its values changed: every blocked
    /// call that can now proceed completes (oldest first, and again from
    /// the start after each completion), one that would overflow fails
    /// with `ERANGE`; the others stay, waiting for what blocks them now.
    /// Returns the threads to unblock.
    pub fn update(&mut self, id: i32, now: i64) -> Vec<i32> {
        let mut woken = Vec::new();
        'scan: while let Some(len) = self.set(id).map(|s| s.queue.len()) {
            for i in 0..len {
                let p = &self.set(id).expect("live").queue[i];
                let (tid, pid, ops) = (p.tid, p.pid, p.ops.clone());
                match self.attempt(id, pid, &ops) {
                    Attempt::Blocked(wait, _) => self.set_mut(id).expect("live").queue[i].wait = wait,
                    Attempt::Failed(e) => {
                        self.set_mut(id).expect("live").queue.remove(i);
                        self.results.insert(tid, Err(e));
                        woken.push(tid);
                        continue 'scan;
                    }
                    Attempt::Done(values, undo) => {
                        self.set_mut(id).expect("live").queue.remove(i);
                        self.commit(id, pid, &ops, values, undo, now);
                        self.results.insert(tid, Ok(()));
                        woken.push(tid);
                        continue 'scan;
                    }
                }
            }
            break;
        }
        woken
    }

    /// The result left for thread `tid`, if its blocked call has one.
    pub fn take_result(&mut self, tid: i32) -> Option<Result<(), Errno>> {
        self.results.remove(&tid)
    }

    /// Take thread `tid`'s blocked call off its queue (a signal or a
    /// timeout ended the wait); `false` if it was not queued.
    pub fn cancel(&mut self, tid: i32) -> bool {
        for s in self.slots.iter_mut().flatten() {
            if let Some(i) = s.queue.iter().position(|p| p.tid == tid) {
                s.queue.remove(i);
                return true;
            }
        }
        false
    }

    /// Process `pid` exits: its adjustments are applied (each value kept
    /// within 0..=[`SEMVMX`]; `sempid` names it for each semaphore
    /// adjusted) and dropped, and blocked calls that can then proceed
    /// complete. Returns the threads to unblock.
    pub fn exit(&mut self, pid: i32, now: i64) -> Vec<i32> {
        let mine: Vec<(i32, Vec<i32>)> =
            self.undo.iter().filter(|((p, _), _)| *p == pid).map(|((_, id), adj)| (*id, adj.clone())).collect();
        self.undo.retain(|(p, _), _| *p != pid);
        let mut woken = Vec::new();
        for (id, adj) in mine {
            let Some(s) = self.set_mut(id) else { continue };
            let mut changed = false;
            for (sem, a) in s.sems.iter_mut().zip(&adj) {
                if *a != 0 {
                    sem.value = (sem.value + a).clamp(0, SEMVMX);
                    sem.pid = pid;
                    changed = true;
                }
            }
            if changed {
                woken.extend(self.update(id, now));
            }
        }
        woken
    }
}

/// The sets of the initial IPC namespace.
static INITIAL: Mutex<SemTable> = Mutex::new(SemTable::new());

/// Run `f` on the semaphore table of the IPC namespace `ns` (`None`: the
/// initial one), locked. `f` must not block or call the kernel.
pub fn with_table<R>(ns: &Option<Arc<IpcNs>>, f: impl FnOnce(&mut SemTable) -> R) -> R {
    match ns {
        Some(ns) => f(&mut ns.sem().lock()),
        None => f(&mut INITIAL.lock()),
    }
}

/// `struct semid64_ds` on x86-64 (`asm/sembuf.h`, with `struct
/// ipc64_perm` from `asm-generic/ipcbuf.h`): what `IPC_STAT` and
/// `SEM_STAT` write and `IPC_SET` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SemidDs {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub mode: u16,
    pub seq: u16,
    pub otime: i64,
    pub ctime: i64,
    pub nsems: u64,
}

impl SemidDs {
    /// `sizeof(struct semid64_ds)`.
    pub const SIZE: usize = 104;

    /// What `IPC_STAT` reports of `s` (kernel ids).
    pub fn of(s: &SemSet) -> SemidDs {
        SemidDs {
            key: s.key,
            uid: s.uid,
            gid: s.gid,
            cuid: s.cuid,
            cgid: s.cgid,
            mode: s.mode,
            seq: s.seq,
            otime: s.otime,
            ctime: s.ctime,
            nsems: s.nsems() as u64,
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
        b[48..56].copy_from_slice(&self.otime.to_ne_bytes());
        b[64..72].copy_from_slice(&self.ctime.to_ne_bytes());
        b[80..88].copy_from_slice(&self.nsems.to_ne_bytes());
        b
    }

    /// What `IPC_SET` takes from the structure: `sem_perm.uid`,
    /// `sem_perm.gid` and `sem_perm.mode`.
    pub fn perm_from_bytes(b: &[u8; Self::SIZE]) -> (u32, u32, u16) {
        let word = |at: usize| u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        (word(4), word(8), u16::from_ne_bytes([b[20], b[21]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(uid: u32, gid: u32) -> Who {
        Who { euid: uid, egid: gid, groups: Arc::from([]), ipc_owner: false, sys_admin: false, ipc_lock: false }
    }

    const ME: (i32, i32) = (100, 100);

    fn op(num: u16, op: i16, flg: i16) -> SemBuf {
        SemBuf { num, op, flg }
    }

    fn values(t: &SemTable, id: i32) -> Vec<i32> {
        t.set(id).unwrap().sems.iter().map(|s| s.value).collect()
    }

    #[test]
    fn ids_are_slot_plus_sequence_and_removed_ids_stay_invalid() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let a = t.semget(IPC_PRIVATE, 3, 0o640, &me, 5).unwrap();
        let b = t.semget(IPC_PRIVATE, 2, 0, &me, 5).unwrap();
        assert_eq!((a % SEQ_MULTIPLIER, b % SEQ_MULTIPLIER), (1, 2));
        assert_ne!(a, b);
        assert!(t.by_index(0).is_none(), "a scan from 0 meets an empty slot first");
        assert_eq!(t.by_index(2).map(|(id, _)| id), Some(b));
        assert_eq!(t.by_index(b).map(|(id, _)| id), Some(b), "the id itself is an index too");
        assert_eq!(t.max_index(), 2);
        t.remove(a);
        assert!(t.set(a).is_none());
        let c = t.semget(IPC_PRIVATE, 1, 0o600, &me, 5).unwrap();
        assert_eq!(c % SEQ_MULTIPLIER, 3, "the slot after the last one given, not the freed one");
        assert_ne!(c, a);
        assert!(t.set(a).is_none(), "the old id names nothing");
        assert!(t.set(-1).is_none() && t.by_index(-1).is_none());
        assert_eq!(t.info(true).semusz, 2);
        assert_eq!(t.info(true).semaem, 3);
        assert_eq!(t.info(false).semusz, SEMUSZ);
    }

    #[test]
    fn semget_keys_and_errors() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        assert_eq!(t.semget(7, 2, 0o600, &me, 0), Err(Errno::ENOENT));
        assert_eq!(t.semget(IPC_PRIVATE, 0, 0o600, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.semget(IPC_PRIVATE, -1, 0o600, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.semget(IPC_PRIVATE, SEMMSL + 1, 0o600, &me, 0), Err(Errno::EINVAL));
        let id = t.semget(7, 2, 0o600 | IPC_CREAT, &me, 0).unwrap();
        assert_eq!(t.semget(7, 0, 0, &me, 0), Ok(id));
        assert_eq!(t.semget(7, 3, 0, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.semget(7, 2, IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        assert_eq!(t.semget(7, 2, 0o700, &me, 0), Err(Errno::EACCES));
        assert_eq!(t.set(id).unwrap().key, 7);
    }

    #[test]
    fn access_counts_only_the_callers_class() {
        let mut t = SemTable::new();
        let owner = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 1, 0o077, &owner, 0).unwrap();
        let s = t.set(id).unwrap();
        assert!(!s.allows(&owner, READ), "an owner with mode 0077 is denied");
        assert!(s.allows(&user(2000, 1001), READ | ALTER));
        assert!(s.allows(&user(2000, 2000), READ));
        let root = Who { ipc_owner: true, ..user(0, 0) };
        assert!(s.allows(&root, READ | ALTER));
        let x = t.semget(IPC_PRIVATE, 1, 0o100, &owner, 0).unwrap();
        assert!(!t.set(x).unwrap().allows(&owner, READ), "execute grants nothing");
    }

    #[test]
    fn setval_wakes_what_can_proceed_and_clears_undo() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 2, 0o600, &me, 0).unwrap();
        let mut woken = Vec::new();
        assert_eq!(t.semop(id, &[op(0, -1, 0)], &me, (201, 201), 0, &mut woken), Ok(Semop::Queued));
        assert_eq!(t.semop(id, &[op(0, -3, 0)], &me, (202, 202), 0, &mut woken), Ok(Semop::Queued));
        t.setval(id, 1, 1, 100, 0);
        assert_eq!(t.semop(id, &[op(1, 0, 0)], &me, (203, 203), 0, &mut woken), Ok(Semop::Queued));
        let s = t.set(id).unwrap();
        assert_eq!((s.waiting(0, false), s.waiting(0, true), s.waiting(1, true)), (2, 0, 1));
        assert_eq!(t.setval(id, 0, 1, 100, 9), vec![201]);
        assert_eq!(t.take_result(201), Some(Ok(())));
        assert_eq!(t.set(id).unwrap().sems[0], Sem { value: 0, pid: 201 });
        assert_eq!(t.set(id).unwrap().otime, 9);
        assert_eq!(t.setval(id, 0, 2, 100, 9), Vec::<i32>::new());
        assert_eq!(t.setval(id, 1, 0, 100, 9), vec![203]);
        assert_eq!(t.setval(id, 0, 3, 100, 9), vec![202]);
        assert_eq!(values(&t, id), [0, 0]);
        assert!(woken.is_empty());
    }

    #[test]
    fn nowait_range_and_bounds() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
        let mut w = Vec::new();
        assert_eq!(t.semop(id, &[op(0, -1, IPC_NOWAIT)], &me, ME, 0, &mut w), Err(Errno::EAGAIN));
        assert_eq!(t.semop(id, &[op(1, 1, 0)], &me, ME, 0, &mut w), Err(Errno::EFBIG));
        t.setval(id, 0, SEMVMX, 1, 0);
        assert_eq!(t.semop(id, &[op(0, 1, 0)], &me, ME, 0, &mut w), Err(Errno::ERANGE));
        assert_eq!(t.semop(id + 1, &[op(0, 1, 0)], &me, ME, 0, &mut w), Err(Errno::EINVAL));
        // Atomic: the second operation blocks, so the first does not happen.
        t.setval(id, 0, 1, 1, 0);
        assert_eq!(t.semop(id, &[op(0, -1, 0), op(0, -1, IPC_NOWAIT)], &me, ME, 0, &mut w), Err(Errno::EAGAIN));
        assert_eq!(values(&t, id), [1]);
        let reader = user(1001, 1001);
        t.set_mut(id).unwrap().mode = 0o400;
        assert_eq!(t.semop(id, &[op(0, 0, IPC_NOWAIT)], &reader, ME, 0, &mut w), Err(Errno::EAGAIN));
        assert_eq!(t.semop(id, &[op(0, 1, 0)], &reader, ME, 0, &mut w), Err(Errno::EACCES));
    }

    #[test]
    fn undo_is_applied_at_exit_unless_a_set_cleared_it() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 2, 0o600, &me, 0).unwrap();
        let mut w = Vec::new();
        t.setval(id, 0, 1, 100, 0);
        t.semop(id, &[op(0, 2, SEM_UNDO)], &me, (300, 300), 0, &mut w).unwrap();
        t.setval(id, 0, 10, 100, 0);
        t.exit(300, 0);
        assert_eq!(values(&t, id), [10, 0]);
        t.semop(id, &[op(0, 2, SEM_UNDO)], &me, (301, 301), 0, &mut w).unwrap();
        t.exit(301, 0);
        assert_eq!(values(&t, id), [10, 0]);
        assert_eq!(t.set(id).unwrap().sems[0].pid, 301);
        t.semop(id, &[op(1, 3, SEM_UNDO)], &me, (302, 302), 0, &mut w).unwrap();
        t.setall(id, &[7, 8], 100, 0);
        t.exit(302, 0);
        assert_eq!(values(&t, id), [7, 8]);
        t.semop(id, &[op(0, 1, SEM_UNDO), op(1, 1, SEM_UNDO)], &me, (303, 303), 0, &mut w).unwrap();
        t.setval(id, 0, 20, 100, 0);
        t.exit(303, 0);
        assert_eq!(values(&t, id), [20, 8]);
    }

    #[test]
    fn exit_undo_wakes_waiters_and_clamps() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
        let mut w = Vec::new();
        t.semop(id, &[op(0, 1, SEM_UNDO)], &me, (300, 300), 0, &mut w).unwrap();
        assert_eq!(t.semop(id, &[op(0, 0, 0)], &me, (301, 301), 0, &mut w), Ok(Semop::Queued));
        assert_eq!(t.exit(300, 0), vec![301]);
        assert_eq!(values(&t, id), [0]);
        t.semop(id, &[op(0, 5, SEM_UNDO)], &me, (302, 302), 0, &mut w).unwrap();
        // Another process takes the value down; the undo would go below 0.
        t.semop(id, &[op(0, -4, 0)], &me, (303, 303), 0, &mut w).unwrap();
        t.exit(302, 0);
        assert_eq!(values(&t, id), [0]);
    }

    #[test]
    fn removal_fails_waiters_with_eidrm() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
        let mut w = Vec::new();
        t.semop(id, &[op(0, -1, SEM_UNDO)], &me, (400, 400), 0, &mut w).unwrap();
        assert_eq!(t.remove(id), vec![400]);
        assert_eq!(t.take_result(400), Some(Err(Errno::EIDRM)));
        assert!(t.is_empty());
        assert!(t.remove(id).is_empty());
    }

    #[test]
    fn cancel_takes_a_waiter_off_the_queue() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.semget(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
        let mut w = Vec::new();
        t.semop(id, &[op(0, -1, 0)], &me, (500, 500), 0, &mut w).unwrap();
        assert!(t.cancel(500));
        assert!(!t.cancel(500));
        assert_eq!(t.set(id).unwrap().waiting(0, false), 0);
    }

    #[test]
    fn layouts() {
        let ds =
            SemidDs { key: -7, uid: 1, gid: 2, cuid: 3, cgid: 4, mode: 0o640, seq: 5, otime: 6, ctime: 7, nsems: 8 };
        let b = ds.to_bytes();
        assert_eq!(&b[0..4], &(-7i32).to_ne_bytes());
        assert_eq!(&b[24..26], &5u16.to_ne_bytes());
        assert_eq!(&b[48..56], &6i64.to_ne_bytes());
        assert_eq!(&b[64..72], &7i64.to_ne_bytes());
        assert_eq!(&b[80..88], &8u64.to_ne_bytes());
        assert!(b[88..].iter().all(|x| *x == 0));
        assert_eq!(SemidDs::perm_from_bytes(&b), (1, 2, 0o640));
        let i = SemTable::new().info(false).to_bytes();
        assert_eq!(&i[4..8], &32000i32.to_ne_bytes());
        assert_eq!(&i[36..40], &32767i32.to_ne_bytes());
        assert_eq!(SemBuf::from_bytes(&[1, 0, 0xff, 0xff, 0, 0x10]), op(1, -1, SEM_UNDO));
    }
}
