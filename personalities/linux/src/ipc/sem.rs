//! System V semaphore sets (semget(2), semop(2), semctl(2), svipc(7)):
//! the sets of one IPC namespace, the callers sleeping in semop(2), and
//! the `SEM_UNDO` adjustments of every process.
//!
//! A set belongs to its IPC namespace: it lives in the namespace's
//! [`SemTable`] from semget(2) until semctl(`IPC_RMID`), which removes it
//! at once. Ids are handed out in increasing order (wrapping at
//! `i32::MAX`), so the id just removed is not the next one.
//!
//! semop(2) applies an array of operations all at once or not at all
//! ([`SemTable::semop`]). A call that has to wait is queued as a sleeper;
//! whoever changes the values afterwards (semop, `SETVAL`, `SETALL`, a
//! process exit that undoes adjustments) completes every sleeper whose
//! whole array has become possible before it returns ([`SemTable::wake`]),
//! in the order they went to sleep, and the sleeper only collects the
//! result. A sleeper counts in `semncnt` or `semzcnt` of the semaphore of
//! the first operation of its array that cannot proceed now.
//!
//! Adjustments (`semadj`) are kept per process (by global pid) and
//! semaphore; the process's exit adds them back ([`SemTable::exit`]).
//!
//! The table sits behind the namespace lock ([`crate::ns::lock::Mutex`], a
//! spinlock): handlers decide under it and call the kernel outside it.

use crate::errno::Errno;
use crate::ipc::shm::{IPC_CREAT, IPC_EXCL, IPC_PRIVATE, READ, WRITE, Who};
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Limits (`linux/sem.h`, the defaults of `/proc/sys/kernel/sem`): the
/// most semaphores in a set, in the namespace, operations in one call,
/// and sets in the namespace.
pub const SEMMSL: i32 = 32000;
pub const SEMMNS: u64 = 1_024_000_000;
pub const SEMOPM: u32 = 500;
pub const SEMMNI: usize = 32000;
/// The largest semaphore value and adjustment (`linux/sem.h`).
pub const SEMVMX: i32 = 32767;
pub const SEMAEM: i32 = SEMVMX;

/// `sem_flg` bits (`linux/ipc.h`, `linux/sem.h`).
pub const IPC_NOWAIT: i16 = 0o4000;
pub const SEM_UNDO: i16 = 0x1000;

/// semctl commands (`linux/sem.h`).
pub const GETPID: u32 = 11;
pub const GETVAL: u32 = 12;
pub const GETALL: u32 = 13;
pub const GETNCNT: u32 = 14;
pub const GETZCNT: u32 = 15;
pub const SETVAL: u32 = 16;
pub const SETALL: u32 = 17;

/// One operation: `struct sembuf` (`linux/sem.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemBuf {
    pub num: u16,
    pub op: i16,
    pub flg: i16,
}

impl SemBuf {
    /// `sizeof(struct sembuf)`.
    pub const SIZE: usize = 6;

    pub const fn new(num: u16, op: i16, flg: i16) -> SemBuf {
        SemBuf { num, op, flg }
    }

    /// The operations in `bytes` (a whole number of elements).
    pub fn parse(bytes: &[u8]) -> Vec<SemBuf> {
        bytes
            .chunks_exact(Self::SIZE)
            .map(|b| SemBuf {
                num: u16::from_ne_bytes([b[0], b[1]]),
                op: i16::from_ne_bytes([b[2], b[3]]),
                flg: i16::from_ne_bytes([b[4], b[5]]),
            })
            .collect()
    }

    fn undo(&self) -> bool {
        self.flg & SEM_UNDO != 0 && self.op != 0
    }
}

/// One semaphore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sem {
    /// 0..=[`SEMVMX`].
    pub val: i32,
    /// Global pid of the last process that operated on it (`sempid`).
    pub pid: i32,
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
    /// Seconds since the epoch of the last semop and of the last change.
    pub otime: i64,
    pub ctime: i64,
    pub sems: Vec<Sem>,
    /// Names this set among all sets the table ever had (ids come back
    /// after wrapping; adjustments must not).
    serial: u64,
}

impl SemSet {
    /// Whether `who` may have the access in `request` ([`super::allows`]).
    pub fn allows(&self, who: &Who, request: u32) -> bool {
        super::allows(self.mode, (self.uid, self.cuid), (self.gid, self.cgid), who, request)
    }

    /// Whether `who` may change or remove it (`IPC_SET`, `IPC_RMID`):
    /// its owner, its creator, or a caller with `CAP_SYS_ADMIN`.
    pub fn owned_by(&self, who: &Who) -> bool {
        who.euid == self.uid || who.euid == self.cuid || who.sys_admin
    }

    /// `sem_nsems`.
    pub fn nsems(&self) -> usize {
        self.sems.len()
    }
}

/// A caller sleeping in semop.
#[derive(Debug)]
struct Sleeper {
    ticket: u64,
    tid: i32,
    pid: i32,
    /// The set's serial.
    serial: u64,
    ops: Vec<SemBuf>,
    /// Set by whoever completes it: the call's result.
    result: Option<Result<(), Errno>>,
}

/// What evaluating an array against the current values says.
#[derive(Debug, PartialEq, Eq)]
enum Attempt {
    /// Every operation can proceed: the new values and adjustments of the
    /// semaphores it changes.
    Ready { values: Vec<(u16, i32)>, adjs: Vec<(u16, i32)> },
    /// The first operation that cannot proceed would leave the range.
    Range,
    /// The first operation that cannot proceed, at this index, must wait.
    Wait(usize),
}

/// What semop does after [`SemTable::semop`].
#[derive(Debug, PartialEq, Eq)]
pub enum Semop {
    /// Applied; wake these threads (sleepers it completed).
    Done(Vec<i32>),
    /// Sleep; collect the result with [`SemTable::poll`] under this ticket.
    Sleep(u64),
}

/// The semaphore sets of one IPC namespace, by id.
#[derive(Debug, Default)]
pub struct SemTable {
    sets: BTreeMap<i32, SemSet>,
    /// The id handed out last (`-1` before the first).
    last: i32,
    serials: u64,
    tickets: u64,
    sleepers: Vec<Sleeper>,
    /// Adjustments by global pid, set serial and semaphore; never 0.
    adjs: BTreeMap<(i32, u64, u16), i32>,
}

impl SemTable {
    pub const fn new() -> SemTable {
        SemTable {
            sets: BTreeMap::new(),
            last: -1,
            serials: 0,
            tickets: 0,
            sleepers: Vec::new(),
            adjs: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn set(&self, id: i32) -> Option<&SemSet> {
        self.sets.get(&id)
    }

    pub fn set_mut(&mut self, id: i32) -> Option<&mut SemSet> {
        self.sets.get_mut(&id)
    }

    /// The live set with `key` (never [`IPC_PRIVATE`]).
    fn by_key(&self, key: i32) -> Option<(i32, &SemSet)> {
        self.sets.iter().find(|(_, s)| s.key == key).map(|(id, s)| (*id, s))
    }

    /// semget(2) of `key`, `nsems` and `flags` (the low 32 bits of
    /// `semflg`), made by `who` at `now`, in this order:
    ///
    /// - `nsems` below 0 or above [`SEMMSL`]: `EINVAL`;
    /// - [`IPC_PRIVATE`]: always a new set, whatever the flags;
    /// - another key that no set has: `ENOENT` without [`IPC_CREAT`],
    ///   else a new set;
    /// - a key in use: `EEXIST` with `IPC_CREAT | IPC_EXCL`, then `EINVAL`
    ///   if `nsems` is above its size, then `EACCES` if the access asked
    ///   for in `flags & 0777` is not granted.
    ///
    /// A new set needs at least one semaphore (`EINVAL`) and room in the
    /// namespace ([`SEMMNI`] sets, [`SEMMNS`] semaphores: `ENOSPC`); its
    /// values, pids and `sem_otime` are 0, its mode `flags & 0777`.
    pub fn get(&mut self, key: i32, nsems: i32, flags: u32, who: &Who, now: i64) -> Result<i32, Errno> {
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
                    if !s.allows(who, flags) {
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
        let total = self.sets.values().map(|s| s.nsems() as u64).sum::<u64>();
        if self.sets.len() >= SEMMNI || total + nsems as u64 > SEMMNS {
            return Err(Errno::ENOSPC);
        }
        let mut id = self.last;
        loop {
            id = if id == i32::MAX { 0 } else { id + 1 };
            if !self.sets.contains_key(&id) {
                break;
            }
        }
        self.last = id;
        self.serials += 1;
        let set = SemSet {
            key,
            uid: who.euid,
            gid: who.egid,
            cuid: who.euid,
            cgid: who.egid,
            mode: (flags & 0o777) as u16,
            otime: 0,
            ctime: now,
            sems: alloc::vec![Sem::default(); nsems as usize],
            serial: self.serials,
        };
        self.sets.insert(id, set);
        Ok(id)
    }

    /// Evaluate `ops` of process `pid` against `set` as it is now, in
    /// array order, each operation seeing what the earlier ones left.
    fn evaluate(&self, set: &SemSet, pid: i32, ops: &[SemBuf]) -> Attempt {
        let mut values: Vec<(u16, i32)> = Vec::new();
        let mut adjs: Vec<(u16, i32)> = Vec::new();
        for (i, op) in ops.iter().enumerate() {
            let at = values.iter().position(|v| v.0 == op.num);
            let cur = at.map_or(set.sems[usize::from(op.num)].val, |a| values[a].1);
            let delta = i32::from(op.op);
            let new = cur + delta;
            if delta > 0 && new > SEMVMX {
                return Attempt::Range;
            }
            if (delta < 0 && new < 0) || (delta == 0 && cur != 0) {
                return Attempt::Wait(i);
            }
            if op.undo() {
                let a_at = adjs.iter().position(|a| a.0 == op.num);
                let adj =
                    a_at.map_or_else(|| self.adjs.get(&(pid, set.serial, op.num)).copied().unwrap_or(0), |a| adjs[a].1);
                let new_adj = adj - delta;
                if !(-(SEMAEM + 1)..=SEMAEM).contains(&new_adj) {
                    return Attempt::Range;
                }
                match a_at {
                    Some(a) => adjs[a].1 = new_adj,
                    None => adjs.push((op.num, new_adj)),
                }
            }
            match at {
                Some(a) => values[a].1 = new,
                None => values.push((op.num, new)),
            }
        }
        Attempt::Ready { values, adjs }
    }

    /// Apply what [`Attempt::Ready`] said to set `id`, for process `pid`
    /// at `now`: every semaphore `ops` names gets `sempid` = `pid`.
    fn apply(&mut self, id: i32, pid: i32, ops: &[SemBuf], values: &[(u16, i32)], adjs: &[(u16, i32)], now: i64) {
        let Some(set) = self.sets.get_mut(&id) else { return };
        for (num, val) in values {
            set.sems[usize::from(*num)].val = *val;
        }
        for op in ops {
            set.sems[usize::from(op.num)].pid = pid;
        }
        set.otime = now;
        let serial = set.serial;
        for (num, adj) in adjs {
            if *adj == 0 {
                self.adjs.remove(&(pid, serial, *num));
            } else {
                self.adjs.insert((pid, serial, *num), *adj);
            }
        }
    }

    /// semop(2) of `ops` (1 to [`SEMOPM`] of them, already read) on set
    /// `id`, by `who`, thread `tid` of process `pid`, at `now`. In order:
    /// `EINVAL` if no set has the id; `EFBIG` if any `sem_num` is not
    /// below its size; `EACCES` without alter permission (some `sem_op`
    /// is not 0) or else read permission; then the first operation that
    /// cannot proceed decides: `ERANGE` if it would take the value above
    /// [`SEMVMX`] or its adjustment out of range, `EAGAIN` if it must wait
    /// and has [`IPC_NOWAIT`], else the caller sleeps. Nothing is applied
    /// unless everything is.
    pub fn semop(
        &mut self,
        id: i32,
        ops: Vec<SemBuf>,
        who: &Who,
        pid: i32,
        tid: i32,
        now: i64,
    ) -> Result<Semop, Errno> {
        let set = self.sets.get(&id).ok_or(Errno::EINVAL)?;
        if ops.iter().any(|op| usize::from(op.num) >= set.nsems()) {
            return Err(Errno::EFBIG);
        }
        let alter = ops.iter().any(|op| op.op != 0);
        if !set.allows(who, if alter { WRITE } else { READ }) {
            return Err(Errno::EACCES);
        }
        match self.evaluate(set, pid, &ops) {
            Attempt::Ready { values, adjs } => {
                self.apply(id, pid, &ops, &values, &adjs, now);
                Ok(Semop::Done(if alter { self.wake(id, now) } else { Vec::new() }))
            }
            Attempt::Range => Err(Errno::ERANGE),
            Attempt::Wait(i) if ops[i].flg & IPC_NOWAIT != 0 => Err(Errno::EAGAIN),
            Attempt::Wait(_) => {
                self.tickets += 1;
                let serial = set.serial;
                self.sleepers.push(Sleeper { ticket: self.tickets, tid, pid, serial, ops, result: None });
                Ok(Semop::Sleep(self.tickets))
            }
        }
    }

    /// Complete the sleepers on set `id` whose whole array can be applied
    /// now, at `now`, oldest first, until none more can (a completed one
    /// may make another possible); one that would leave the range fails
    /// with `ERANGE`. Returns the threads to wake.
    pub fn wake(&mut self, id: i32, now: i64) -> Vec<i32> {
        let mut woken = Vec::new();
        let Some(serial) = self.sets.get(&id).map(|s| s.serial) else { return woken };
        'scan: loop {
            for i in 0..self.sleepers.len() {
                let s = &self.sleepers[i];
                if s.result.is_some() || s.serial != serial {
                    continue;
                }
                let attempt = self.evaluate(&self.sets[&id], s.pid, &s.ops);
                match attempt {
                    Attempt::Wait(_) => continue,
                    Attempt::Range => self.sleepers[i].result = Some(Err(Errno::ERANGE)),
                    Attempt::Ready { values, adjs } => {
                        let (pid, ops) = (s.pid, core::mem::take(&mut self.sleepers[i].ops));
                        self.apply(id, pid, &ops, &values, &adjs, now);
                        self.sleepers[i].result = Some(Ok(()));
                    }
                }
                woken.push(self.sleepers[i].tid);
                continue 'scan;
            }
            return woken;
        }
    }

    /// The result of the sleeper with `ticket` if it has one (it then
    /// leaves the queue).
    pub fn poll(&mut self, ticket: u64) -> Option<Result<(), Errno>> {
        let i = self.sleepers.iter().position(|s| s.ticket == ticket && s.result.is_some())?;
        self.sleepers.remove(i).result
    }

    /// The sleeper with `ticket` gives up (a signal): it leaves the queue.
    /// Its result if it got one meanwhile, which then stands.
    pub fn cancel(&mut self, ticket: u64) -> Option<Result<(), Errno>> {
        let i = self.sleepers.iter().position(|s| s.ticket == ticket)?;
        self.sleepers.remove(i).result
    }

    /// `semncnt` and `semzcnt` of semaphore `num` of set `id`: the waiting
    /// sleepers whose first operation that cannot proceed now is on it, a
    /// decrement or a wait for zero.
    pub fn counts(&self, id: i32, num: u16) -> (u64, u64) {
        let Some(set) = self.sets.get(&id) else { return (0, 0) };
        let (mut n, mut z) = (0, 0);
        for s in self.sleepers.iter().filter(|s| s.result.is_none() && s.serial == set.serial) {
            match self.evaluate(set, s.pid, &s.ops) {
                Attempt::Wait(i) if s.ops[i].num == num && s.ops[i].op == 0 => z += 1,
                Attempt::Wait(i) if s.ops[i].num == num => n += 1,
                _ => {}
            }
        }
        (n, z)
    }

    /// semctl(`SETVAL`): semaphore `num` of set `id` (both checked) becomes
    /// `val` (`ERANGE` outside 0..=[`SEMVMX`]), its `sempid` `pid`; every
    /// process's adjustment for it is dropped. Returns the threads to wake.
    pub fn setval(&mut self, id: i32, num: u16, val: i32, pid: i32, now: i64) -> Result<Vec<i32>, Errno> {
        if !(0..=SEMVMX).contains(&val) {
            return Err(Errno::ERANGE);
        }
        let set = self.sets.get_mut(&id).ok_or(Errno::EINVAL)?;
        let sem = set.sems.get_mut(usize::from(num)).ok_or(Errno::EINVAL)?;
        *sem = Sem { val, pid };
        set.ctime = now;
        let serial = set.serial;
        self.adjs.retain(|k, _| !(k.1 == serial && k.2 == num));
        Ok(self.wake(id, now))
    }

    /// semctl(`SETALL`): every semaphore of set `id` takes its value from
    /// `vals` (one per semaphore; `ERANGE` if any is above [`SEMVMX`]);
    /// every adjustment for the set is dropped. Returns the threads to wake.
    pub fn setall(&mut self, id: i32, vals: &[u16], pid: i32, now: i64) -> Result<Vec<i32>, Errno> {
        let set = self.sets.get_mut(&id).ok_or(Errno::EINVAL)?;
        if vals.iter().any(|v| i32::from(*v) > SEMVMX) {
            return Err(Errno::ERANGE);
        }
        for (sem, val) in set.sems.iter_mut().zip(vals) {
            *sem = Sem { val: i32::from(*val), pid };
        }
        set.ctime = now;
        let serial = set.serial;
        self.adjs.retain(|k, _| k.1 != serial);
        Ok(self.wake(id, now))
    }

    /// semctl(`IPC_RMID`) of set `id`: it goes at once with its
    /// adjustments; its sleepers fail with `EIDRM`. Returns the threads to
    /// wake (`EINVAL` if no set has the id).
    pub fn remove(&mut self, id: i32) -> Result<Vec<i32>, Errno> {
        let set = self.sets.remove(&id).ok_or(Errno::EINVAL)?;
        self.adjs.retain(|k, _| k.1 != set.serial);
        let mut woken = Vec::new();
        for s in self.sleepers.iter_mut().filter(|s| s.serial == set.serial && s.result.is_none()) {
            s.result = Some(Err(Errno::EIDRM));
            woken.push(s.tid);
        }
        Ok(woken)
    }

    /// Process `pid` exits: each of its adjustments is added to its
    /// semaphore, the result clamped to 0..=[`SEMVMX`], and `sempid`
    /// becomes `pid`. Returns the threads to wake.
    pub fn exit(&mut self, pid: i32, now: i64) -> Vec<i32> {
        let mine: Vec<((i32, u64, u16), i32)> =
            self.adjs.iter().filter(|(k, _)| k.0 == pid).map(|(k, v)| (*k, *v)).collect();
        let mut touched: Vec<i32> = Vec::new();
        for ((_, serial, num), adj) in mine {
            self.adjs.remove(&(pid, serial, num));
            let Some((id, set)) = self.sets.iter_mut().find(|(_, s)| s.serial == serial) else { continue };
            if let Some(sem) = set.sems.get_mut(usize::from(num)) {
                *sem = Sem { val: (sem.val + adj).clamp(0, SEMVMX), pid };
            }
            if !touched.contains(id) {
                touched.push(*id);
            }
        }
        touched.into_iter().flat_map(|id| self.wake(id, now)).collect()
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

/// `struct semid64_ds` on x86-64 (`asm-generic/sembuf.h`, with `struct
/// ipc64_perm` from `asm-generic/ipcbuf.h`): what `IPC_STAT` writes and
/// `IPC_SET` reads. Ids as the caller sees them.
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

    pub fn of(s: &SemSet) -> SemidDs {
        SemidDs {
            key: s.key,
            uid: s.uid,
            gid: s.gid,
            cuid: s.cuid,
            cgid: s.cgid,
            mode: s.mode,
            seq: 0,
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

    fn root() -> Who {
        Who { ipc_owner: true, sys_admin: true, ipc_lock: true, ..user(0, 0) }
    }

    const PID: i32 = 100;
    const TID: i32 = 100;

    /// A table with one private set of `n` semaphores, mode 0600, by 1001.
    fn table(n: i32) -> (SemTable, i32) {
        let mut t = SemTable::new();
        let id = t.get(IPC_PRIVATE, n, 0o600, &user(1001, 1001), 1_000).unwrap();
        (t, id)
    }

    fn op(t: &mut SemTable, id: i32, ops: &[(u16, i16, i16)]) -> Result<Semop, Errno> {
        op_as(t, id, ops, PID, TID)
    }

    fn op_as(t: &mut SemTable, id: i32, ops: &[(u16, i16, i16)], pid: i32, tid: i32) -> Result<Semop, Errno> {
        let ops = ops.iter().map(|o| SemBuf::new(o.0, o.1, o.2)).collect();
        t.semop(id, ops, &user(1001, 1001), pid, tid, 2_000)
    }

    fn ok(t: &mut SemTable, id: i32, ops: &[(u16, i16, i16)]) {
        assert!(matches!(op(t, id, ops), Ok(Semop::Done(_))), "{ops:?}");
    }

    fn val(t: &SemTable, id: i32, n: usize) -> i32 {
        t.set(id).unwrap().sems[n].val
    }

    const NOWAIT: i16 = IPC_NOWAIT;
    const UNDO: i16 = SEM_UNDO;

    #[test]
    fn basic_operations_apply_in_array_order() {
        // c_semop_basic
        let (mut t, id) = table(3);
        ok(&mut t, id, &[(0, 5, 0)]);
        ok(&mut t, id, &[(0, -3, 0)]);
        assert_eq!(val(&t, id, 0), 2);
        ok(&mut t, id, &[(0, -2, 0)]);
        ok(&mut t, id, &[(0, 0, 0)]);
        ok(&mut t, id, &[(0, 4, 0), (1, 7, 0), (2, 1, 0), (0, -1, 0)]);
        assert_eq!((val(&t, id, 0), val(&t, id, 1), val(&t, id, 2)), (3, 7, 1));
        ok(&mut t, id, &[(2, -1, NOWAIT), (2, 1, 0), (2, -1, NOWAIT)]);
        assert_eq!(val(&t, id, 2), 0);
        ok(&mut t, id, &[(2, 1, 0), (2, -1, NOWAIT)]);
        assert_eq!(op(&mut t, id, &[(2, -1, NOWAIT), (2, 1, 0)]), Err(Errno::EAGAIN));
        ok(&mut t, id, &[(0, -3, 0), (0, 0, NOWAIT)]);
        ok(&mut t, id, &[(1, 32760, 0)]);
        assert_eq!(val(&t, id, 1), 32767);
        ok(&mut t, id, &[(1, -32767, 0)]);
        ok(&mut t, id, &[(1, 32767, 0)]);
        assert_eq!(val(&t, id, 1), 32767);
    }

    #[test]
    fn error_order() {
        // c_semop_bad_id, c_semop_efbig
        let (mut t, id) = table(3);
        assert_eq!(op(&mut t, -1, &[(0, 1, 0)]), Err(Errno::EINVAL));
        assert_eq!(op(&mut t, 0x7fff_fff0, &[(0, 1, 0)]), Err(Errno::EINVAL));
        assert_eq!(op(&mut t, -1, &[(3, 1, 0)]), Err(Errno::EINVAL));
        ok(&mut t, id, &[(2, 1, 0)]);
        assert_eq!(op(&mut t, id, &[(3, 1, 0)]), Err(Errno::EFBIG));
        assert_eq!(op(&mut t, id, &[(65535, 0, NOWAIT)]), Err(Errno::EFBIG));
        assert_eq!(op(&mut t, id, &[(0, 1, 0), (1, 1, 0), (3, 1, 0)]), Err(Errno::EFBIG));
        assert_eq!((val(&t, id, 0), val(&t, id, 1)), (0, 0));
        assert_eq!(op(&mut t, id, &[(0, -1, NOWAIT), (9, 1, 0)]), Err(Errno::EFBIG));
        t.setval(id, 1, 32767, PID, 0).unwrap();
        assert_eq!(op(&mut t, id, &[(1, 1, 0), (4, 0, 0)]), Err(Errno::EFBIG));
        t.set_mut(id).unwrap().mode = 0;
        assert_eq!(op(&mut t, id, &[(3, 1, 0)]), Err(Errno::EFBIG));
        assert_eq!(op(&mut t, id, &[(0, 1, 0)]), Err(Errno::EACCES));
        let r = t.semop(id, alloc::vec![SemBuf::new(0, 1, 0)], &root(), PID, TID, 0);
        assert!(matches!(r, Ok(Semop::Done(_))));
        t.remove(id).unwrap();
        assert_eq!(op(&mut t, id, &[(0, 0, NOWAIT)]), Err(Errno::EINVAL));
    }

    #[test]
    fn range() {
        // c_semop_erange
        let (mut t, id) = table(2);
        ok(&mut t, id, &[(0, 32767, 0)]);
        for flg in [0, NOWAIT, UNDO] {
            assert_eq!(op(&mut t, id, &[(0, 1, flg)]), Err(Errno::ERANGE));
        }
        assert_eq!(op(&mut t, id, &[(0, -10, 0), (0, 11, 0)]), Err(Errno::ERANGE));
        ok(&mut t, id, &[(0, -10, 0), (0, 10, 0)]);
        assert_eq!(op(&mut t, id, &[(1, 5, 0), (0, 1, 0)]), Err(Errno::ERANGE));
        assert_eq!(val(&t, id, 1), 0);
        assert_eq!(op(&mut t, id, &[(0, 1, NOWAIT), (1, -1, NOWAIT)]), Err(Errno::ERANGE));
        assert_eq!(op(&mut t, id, &[(1, -1, NOWAIT), (0, 1, NOWAIT)]), Err(Errno::EAGAIN));
        assert_eq!(op(&mut t, id, &[(0, -32768, NOWAIT)]), Err(Errno::EAGAIN));
        ok(&mut t, id, &[(0, -32767, 0)]);
        assert_eq!(val(&t, id, 0), 0);
    }

    #[test]
    fn permissions() {
        // c_semop_permissions
        let (mut t, id) = table(2);
        t.setval(id, 0, 3, PID, 0).unwrap();
        t.set_mut(id).unwrap().mode = 0o400;
        ok(&mut t, id, &[(1, 0, 0)]);
        assert_eq!(op(&mut t, id, &[(0, 0, NOWAIT)]), Err(Errno::EAGAIN));
        assert_eq!(op(&mut t, id, &[(0, 1, 0)]), Err(Errno::EACCES));
        assert_eq!(op(&mut t, id, &[(1, -1, NOWAIT)]), Err(Errno::EACCES));
        assert_eq!(op(&mut t, id, &[(1, 0, 0), (0, -1, 0)]), Err(Errno::EACCES));
        t.set_mut(id).unwrap().mode = 0o200;
        assert_eq!(op(&mut t, id, &[(1, 0, 0)]), Err(Errno::EACCES));
        ok(&mut t, id, &[(0, 1, 0)]);
        ok(&mut t, id, &[(0, -2, 0)]);
        ok(&mut t, id, &[(1, 0, 0), (0, 1, 0)]);
        assert_eq!(val(&t, id, 0), 3);
        for mode in [0, 0o066, 0o100] {
            t.set_mut(id).unwrap().mode = mode;
            assert_eq!(op(&mut t, id, &[(1, 0, 0)]), Err(Errno::EACCES), "{mode:o}");
            assert_eq!(op(&mut t, id, &[(0, 1, 0)]), Err(Errno::EACCES), "{mode:o}");
        }
    }

    #[test]
    fn success_sets_sempid_and_otime_failure_nothing() {
        // c_semop_sempid_otime
        let (mut t, id) = table(3);
        assert_eq!(op(&mut t, id, &[(2, -1, NOWAIT)]), Err(Errno::EAGAIN));
        assert_eq!((t.set(id).unwrap().sems[2].pid, t.set(id).unwrap().otime), (0, 0));
        ok(&mut t, id, &[(2, 0, 0)]);
        let s = t.set(id).unwrap();
        assert_eq!((s.sems[2].pid, s.sems[0].pid, s.otime, s.ctime), (PID, 0, 2_000, 1_000));
        ok(&mut t, id, &[(0, 1, 0), (0, -1, 0)]);
        assert_eq!(t.set(id).unwrap().sems[0].pid, PID);
    }

    #[test]
    fn adjustment_range_and_setval() {
        // c_semop_semadj_range
        let (mut t, id) = table(2);
        ok(&mut t, id, &[(0, 32767, UNDO)]);
        ok(&mut t, id, &[(0, -32767, 0)]);
        ok(&mut t, id, &[(0, 1, UNDO)]);
        ok(&mut t, id, &[(0, -1, 0)]);
        assert_eq!(op(&mut t, id, &[(0, 1, UNDO)]), Err(Errno::ERANGE));
        assert_eq!(op(&mut t, id, &[(0, 1, UNDO | NOWAIT)]), Err(Errno::ERANGE));
        assert_eq!(op(&mut t, id, &[(1, 1, 0), (0, 1, UNDO)]), Err(Errno::ERANGE));
        assert_eq!(val(&t, id, 1), 0);
        assert_eq!(op(&mut t, id, &[(0, -1, UNDO | NOWAIT)]), Err(Errno::EAGAIN));
        ok(&mut t, id, &[(0, 1, 0)]);
        ok(&mut t, id, &[(0, -1, UNDO)]);

        t.setval(id, 1, 32767, PID, 0).unwrap();
        ok(&mut t, id, &[(1, -32767, UNDO)]);
        ok(&mut t, id, &[(1, 1, 0)]);
        assert_eq!(op(&mut t, id, &[(1, -1, UNDO)]), Err(Errno::ERANGE));
        ok(&mut t, id, &[(1, -1, 0)]);
        t.setval(id, 1, 32767, PID, 0).unwrap();
        ok(&mut t, id, &[(1, -1, UNDO)]);
    }

    #[test]
    fn exit_undoes_clamped() {
        // c_semop_undo_exit
        let (mut t, id) = table(1);
        t.setval(id, 0, 5, PID, 0).unwrap();
        let child = 200;
        assert!(op_as(&mut t, id, &[(0, -2, UNDO)], child, child).is_ok());
        assert!(op_as(&mut t, id, &[(0, 3, UNDO)], child, child).is_ok());
        assert!(op_as(&mut t, id, &[(0, -1, 0)], child, child).is_ok());
        assert_eq!(val(&t, id, 0), 5);
        t.exit(child, 0);
        assert_eq!((val(&t, id, 0), t.set(id).unwrap().sems[0].pid), (4, child));

        t.setval(id, 0, 0, PID, 0).unwrap();
        assert!(op_as(&mut t, id, &[(0, 5, UNDO)], 201, 201).is_ok());
        assert!(op_as(&mut t, id, &[(0, -4, 0)], 201, 201).is_ok());
        t.exit(201, 0);
        assert_eq!(val(&t, id, 0), 0);

        assert!(op_as(&mut t, id, &[(0, 10, 0)], 202, 202).is_ok());
        assert!(op_as(&mut t, id, &[(0, -10, UNDO)], 202, 202).is_ok());
        assert!(op_as(&mut t, id, &[(0, 32767, 0)], 202, 202).is_ok());
        t.exit(202, 0);
        assert_eq!(val(&t, id, 0), 32767);

        t.setval(id, 0, 0, PID, 0).unwrap();
        assert!(op_as(&mut t, id, &[(0, 5, UNDO)], 203, 203).is_ok());
        t.setval(id, 0, 9, PID, 0).unwrap();
        t.exit(203, 0);
        assert_eq!(val(&t, id, 0), 9);

        t.setval(id, 0, 0, PID, 0).unwrap();
        ok(&mut t, id, &[(0, 3, UNDO)]);
        t.exit(204, 0);
        assert_eq!(val(&t, id, 0), 3, "another process's exit undoes nothing of ours");
        assert!(op_as(&mut t, id, &[(0, -3, UNDO | NOWAIT)], 205, 205).is_ok());
        t.exit(205, 0);
        assert_eq!(val(&t, id, 0), 3);
    }

    fn sleep(t: &mut SemTable, id: i32, ops: &[(u16, i16, i16)], who: i32) -> u64 {
        match op_as(t, id, ops, who, who).unwrap() {
            Semop::Sleep(ticket) => ticket,
            done => panic!("{done:?}"),
        }
    }

    #[test]
    fn sleepers_are_completed_by_the_waker_and_counted() {
        // c_semop_block_wake
        let (mut t, id) = table(2);
        let a = sleep(&mut t, id, &[(0, -1, 0)], 300);
        assert_eq!((t.counts(id, 0), t.poll(a)), ((1, 0), None));
        assert_eq!(op(&mut t, id, &[(0, 1, 0)]), Ok(Semop::Done(alloc::vec![300])));
        assert_eq!((val(&t, id, 0), t.counts(id, 0)), (0, (0, 0)));
        assert_eq!(t.set(id).unwrap().sems[0].pid, 300);
        assert_eq!(t.poll(a), Some(Ok(())));
        assert_eq!(t.poll(a), None);

        t.setval(id, 0, 2, PID, 0).unwrap();
        let z = sleep(&mut t, id, &[(0, 0, 0)], 301);
        assert_eq!(t.counts(id, 0), (0, 1));
        ok(&mut t, id, &[(0, -1, 0)]);
        assert_eq!((t.counts(id, 0), t.poll(z)), ((0, 1), None));
        ok(&mut t, id, &[(0, -1, 0)]);
        assert_eq!((t.counts(id, 0), t.poll(z)), ((0, 0), Some(Ok(()))));

        t.setval(id, 0, 1, PID, 0).unwrap();
        let m = sleep(&mut t, id, &[(0, -1, 0), (1, -1, 0)], 302);
        assert_eq!((t.counts(id, 0), t.counts(id, 1), val(&t, id, 0)), ((0, 0), (1, 0), 1));
        ok(&mut t, id, &[(1, 1, 0)]);
        assert_eq!(t.poll(m), Some(Ok(())));
        assert_eq!((val(&t, id, 0), val(&t, id, 1)), (0, 0));

        let m = sleep(&mut t, id, &[(0, -1, 0), (1, -1, 0)], 303);
        assert_eq!((t.counts(id, 0), t.counts(id, 1)), ((1, 0), (0, 0)));
        ok(&mut t, id, &[(0, 1, 0)]);
        assert_eq!((t.counts(id, 0), t.counts(id, 1), val(&t, id, 0)), ((0, 0), (1, 0), 1));
        ok(&mut t, id, &[(1, 1, 0)]);
        assert_eq!(t.poll(m), Some(Ok(())));
        assert_eq!((val(&t, id, 0), val(&t, id, 1)), (0, 0));
    }

    #[test]
    fn wake_order() {
        // c_semop_wake_order
        let (mut t, id) = table(1);
        let a = sleep(&mut t, id, &[(0, -1, 0)], 400);
        let b = sleep(&mut t, id, &[(0, -1, 0)], 401);
        assert_eq!(t.counts(id, 0), (2, 0));
        ok(&mut t, id, &[(0, 1, 0)]);
        assert_eq!((t.poll(a), t.poll(b), t.counts(id, 0)), (Some(Ok(())), None, (1, 0)));
        ok(&mut t, id, &[(0, 1, 0)]);
        assert_eq!(t.poll(b), Some(Ok(())));

        let a = sleep(&mut t, id, &[(0, -2, 0)], 402);
        let b = sleep(&mut t, id, &[(0, -1, 0)], 403);
        ok(&mut t, id, &[(0, 1, 0)]);
        assert_eq!((t.poll(a), t.poll(b), t.counts(id, 0), val(&t, id, 0)), (None, Some(Ok(())), (1, 0), 0));
        ok(&mut t, id, &[(0, 2, 0)]);
        assert_eq!((t.poll(a), val(&t, id, 0)), (Some(Ok(())), 0));

        let a = sleep(&mut t, id, &[(0, -1, 0)], 404);
        let b = sleep(&mut t, id, &[(0, -1, 0)], 405);
        assert_eq!(op(&mut t, id, &[(0, 3, 0)]), Ok(Semop::Done(alloc::vec![404, 405])));
        assert_eq!((val(&t, id, 0), t.counts(id, 0)), (1, (0, 0)));
        assert_eq!((t.poll(a), t.poll(b)), (Some(Ok(())), Some(Ok(()))));

        let z = sleep(&mut t, id, &[(0, 0, 0)], 406);
        ok(&mut t, id, &[(0, -1, NOWAIT)]);
        assert_eq!((t.counts(id, 0), t.poll(z)), ((0, 0), Some(Ok(()))));
    }

    #[test]
    fn removal_and_cancel() {
        let (mut t, id) = table(1);
        let a = sleep(&mut t, id, &[(0, -1, 0)], 500);
        let b = sleep(&mut t, id, &[(0, -1, 0)], 501);
        assert_eq!(t.cancel(b), None);
        assert_eq!(t.counts(id, 0), (1, 0));
        assert_eq!(t.remove(id), Ok(alloc::vec![500]));
        assert_eq!(t.poll(a), Some(Err(Errno::EIDRM)));
        assert_eq!(t.remove(id), Err(Errno::EINVAL));
        // A result that came before the cancel stands.
        let id = t.get(IPC_PRIVATE, 1, 0o600, &user(1001, 1001), 0).unwrap();
        let c = sleep(&mut t, id, &[(0, -1, 0)], 502);
        ok(&mut t, id, &[(0, 1, 0)]);
        assert_eq!(t.cancel(c), Some(Ok(())));
    }

    #[test]
    fn exit_wakes_sleepers() {
        let (mut t, id) = table(1);
        assert!(op_as(&mut t, id, &[(0, 1, 0)], 600, 600).is_ok());
        assert!(op_as(&mut t, id, &[(0, -1, UNDO)], 600, 600).is_ok());
        let a = sleep(&mut t, id, &[(0, -1, 0)], 601);
        assert_eq!(t.exit(600, 7), alloc::vec![601]);
        assert_eq!((t.poll(a), val(&t, id, 0), t.set(id).unwrap().sems[0].pid), (Some(Ok(())), 0, 601));
    }

    #[test]
    fn semget_rules() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        assert_eq!(t.get(IPC_PRIVATE, 0, 0o600, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(IPC_PRIVATE, -1, 0o600, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(IPC_PRIVATE, SEMMSL + 1, 0o600, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(7, 1, 0o600, &me, 0), Err(Errno::ENOENT));
        let id = t.get(7, 2, 0o600 | IPC_CREAT, &me, 0).unwrap();
        assert_eq!(t.get(7, 0, 0, &me, 0), Ok(id));
        assert_eq!(t.get(7, 3, 0, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(7, 2, IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        assert_eq!(t.get(7, 2, 0o700, &me, 0), Err(Errno::EACCES));
        let other = t.get(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
        assert_ne!(other, id);
        t.remove(other).unwrap();
        assert_ne!(t.get(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap(), other);
        let s = t.set(id).unwrap();
        assert_eq!((s.mode, s.nsems(), s.otime, s.sems[1]), (0o600, 2, 0, Sem::default()));
    }

    #[test]
    fn sembuf_and_semid_ds_layout() {
        let b = [1, 0, 0xff, 0xff, 0x00, 0x10, 2, 0, 5, 0, 0, 8];
        assert_eq!(SemBuf::parse(&b), alloc::vec![SemBuf::new(1, -1, SEM_UNDO), SemBuf::new(2, 5, 0o4000)]);
        let ds =
            SemidDs { key: -7, uid: 1, gid: 2, cuid: 3, cgid: 4, mode: 0o600, seq: 0, otime: 5, ctime: 6, nsems: 3 };
        let b = ds.to_bytes();
        assert_eq!(&b[0..4], &(-7i32).to_ne_bytes());
        assert_eq!(&b[48..56], &5i64.to_ne_bytes());
        assert_eq!(&b[64..72], &6i64.to_ne_bytes());
        assert_eq!(&b[80..88], &3u64.to_ne_bytes());
        assert_eq!(SemidDs::perm_from_bytes(&b), (1, 2, 0o600));
    }
}
