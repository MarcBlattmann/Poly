//! System V semaphore sets (semget(2), semop(2), semctl(2), svipc(7)):
//! the sets of one IPC namespace.
//!
//! A set belongs to its IPC namespace, not to a process: it lives in the
//! namespace's [`SemTable`] from semget(2) until semctl(`IPC_RMID`), which
//! takes effect at once (the key is free again and the id names nothing).
//! Ids are handed out in increasing order (wrapping at `i32::MAX`), so the
//! id just removed is not the next one.
//!
//! The table only decides; it never blocks. A semop(2) that has to wait
//! registers its thread in the set ([`SemSet::wait`]), which is what
//! `GETNCNT` and `GETZCNT` count, and the handler blocks outside the lock.
//! Every change of the values ([`SemSet::try_ops`], `SETVAL`, `SETALL`) and
//! the removal of the set wake every waiter, which then tries again.
//!
//! The table sits behind the namespace lock ([`crate::ns::lock::Mutex`], a
//! spinlock): handlers decide under it and call the kernel outside it.

use super::{IPC_CREAT, IPC_EXCL, IPC_NOWAIT, IPC_PRIVATE, MODE_BITS, Owners, Who, permits};
use crate::errno::Errno;
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// Limits (`linux/sem.h`; the defaults of the `kernel.sem` sysctl): the
/// most semaphores in a set, in all sets of a namespace, the most
/// operations in one semop(2), the most sets, and the largest value.
pub const SEMMSL: i32 = 32000;
pub const SEMMNS: u64 = 1_024_000_000;
pub const SEMOPM: u64 = 500;
pub const SEMMNI: usize = 32000;
pub const SEMVMX: i32 = 32767;

/// semctl commands (`linux/sem.h`).
pub const GETPID: u32 = 11;
pub const GETVAL: u32 = 12;
pub const GETALL: u32 = 13;
pub const GETNCNT: u32 = 14;
pub const GETZCNT: u32 = 15;
pub const SETVAL: u32 = 16;
pub const SETALL: u32 = 17;

/// The semop flag that asks for the operation to be undone at exit
/// (`linux/sem.h`).
pub const SEM_UNDO: u16 = 0x1000;

/// One semaphore.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Semaphore {
    /// `semval`.
    pub val: u16,
    /// `sempid`: global pid of the last process that changed it (0 for a
    /// new set).
    pub pid: i32,
}

/// A thread blocked in semop(2) on a set: the semaphore it waits on and
/// whether it waits for zero (`GETZCNT`) or for an increase (`GETNCNT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Waiter {
    tid: i32,
    num: u16,
    zero: bool,
}

/// One operation of semop(2): `struct sembuf` (`linux/sem.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemBuf {
    pub num: u16,
    pub op: i16,
    pub flg: u16,
}

impl SemBuf {
    /// `sizeof(struct sembuf)`.
    pub const SIZE: usize = 6;

    pub fn from_bytes(b: &[u8]) -> SemBuf {
        SemBuf {
            num: u16::from_ne_bytes([b[0], b[1]]),
            op: i16::from_ne_bytes([b[2], b[3]]),
            flg: u16::from_ne_bytes([b[4], b[5]]),
        }
    }

    /// Whether it changes the value (needs write permission; a wait for
    /// zero needs read).
    pub fn alters(&self) -> bool {
        self.op != 0
    }
}

/// What [`SemSet::try_ops`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// Every operation was applied.
    Applied,
    /// Nothing was applied: semaphore `num` must grow (or, with `zero`,
    /// reach zero) first.
    Wait { num: u16, zero: bool },
}

/// One set.
#[derive(Clone, Debug)]
pub struct SemSet {
    /// The key it was made with.
    pub key: i32,
    pub owners: Owners,
    /// Permission bits.
    pub mode: u16,
    /// Seconds since the epoch of the last semop(2) and the last change.
    pub otime: i64,
    pub ctime: i64,
    pub sems: Vec<Semaphore>,
    waiters: Vec<Waiter>,
}

impl SemSet {
    /// A new set of `nsems` semaphores, all 0, with mode `flags & 0777`,
    /// made by `who` (owner and creator) at `now`.
    pub fn new(key: i32, nsems: usize, flags: u32, who: &Who, now: i64) -> SemSet {
        SemSet {
            key,
            owners: Owners { uid: who.euid, gid: who.egid, cuid: who.euid, cgid: who.egid },
            mode: (flags & MODE_BITS) as u16,
            otime: 0,
            ctime: now,
            sems: vec![Semaphore::default(); nsems],
            waiters: Vec::new(),
        }
    }

    pub fn nsems(&self) -> usize {
        self.sems.len()
    }

    /// Whether `who` may have the access in `request` ([`permits`]).
    pub fn allows(&self, who: &Who, request: u32) -> bool {
        permits(&self.owners, self.mode, who, request)
    }

    /// Whether `who` may change or remove it ([`Owners::owned_by`]).
    pub fn owned_by(&self, who: &Who) -> bool {
        self.owners.owned_by(who)
    }

    /// The semaphore `num` (`EINVAL` past the end).
    pub fn semaphore(&self, num: i32) -> Result<&Semaphore, Errno> {
        usize::try_from(num).ok().and_then(|n| self.sems.get(n)).ok_or(Errno::EINVAL)
    }

    /// Set semaphore `num` to `val` (`SETVAL`) for process `pid` at `now`:
    /// `ERANGE` unless `0 <= val <= SEMVMX`, `EINVAL` past the end. Returns
    /// the waiters to wake.
    pub fn set_value(&mut self, num: i32, val: i32, pid: i32, now: i64) -> Result<Vec<i32>, Errno> {
        if !(0..=SEMVMX).contains(&val) {
            return Err(Errno::ERANGE);
        }
        let n = usize::try_from(num).ok().filter(|n| *n < self.sems.len()).ok_or(Errno::EINVAL)?;
        self.sems[n] = Semaphore { val: val as u16, pid };
        self.ctime = now;
        Ok(self.waiters())
    }

    /// Set every semaphore (`SETALL`) for process `pid` at `now`: `ERANGE`
    /// if a value is above `SEMVMX`. Returns the waiters to wake.
    pub fn set_all(&mut self, vals: &[u16], pid: i32, now: i64) -> Result<Vec<i32>, Errno> {
        debug_assert_eq!(vals.len(), self.sems.len());
        if vals.iter().any(|v| i32::from(*v) > SEMVMX) {
            return Err(Errno::ERANGE);
        }
        for (s, v) in self.sems.iter_mut().zip(vals) {
            *s = Semaphore { val: *v, pid };
        }
        self.ctime = now;
        Ok(self.waiters())
    }

    /// Apply `ops` all together or not at all, as semop(2) does for process
    /// `pid` at `now`. Every `num` must be below [`SemSet::nsems`] (the
    /// handler checks: `EFBIG`). In order, an operation that would take a
    /// value above `SEMVMX` is `ERANGE`; one that would take it below 0, or
    /// a wait for zero on a value that is not, is `EAGAIN` with
    /// `IPC_NOWAIT` in its flags and [`Done::Wait`] without. On success the
    /// changed semaphores record `pid`, `sem_otime` is `now`, and the
    /// waiters to wake are returned.
    pub fn try_ops(&mut self, ops: &[SemBuf], pid: i32, now: i64) -> Result<(Done, Vec<i32>), Errno> {
        let mut vals: BTreeMap<u16, i32> = BTreeMap::new();
        for op in ops {
            let v = *vals.entry(op.num).or_insert(i32::from(self.sems[usize::from(op.num)].val));
            let new = v + i32::from(op.op);
            let wait = if op.op == 0 { v != 0 } else { new < 0 };
            if wait {
                if u32::from(op.flg) & IPC_NOWAIT != 0 {
                    return Err(Errno::EAGAIN);
                }
                return Ok((Done::Wait { num: op.num, zero: op.op == 0 }, Vec::new()));
            }
            if new > SEMVMX {
                return Err(Errno::ERANGE);
            }
            vals.insert(op.num, new);
        }
        for op in ops.iter().filter(|op| op.alters()) {
            let s = &mut self.sems[usize::from(op.num)];
            *s = Semaphore { val: vals[&op.num] as u16, pid };
        }
        self.otime = now;
        Ok((Done::Applied, self.waiters()))
    }

    /// Register thread `tid` as waiting on semaphore `num` (for zero with
    /// `zero`), until [`SemSet::unwait`].
    pub fn wait(&mut self, tid: i32, num: u16, zero: bool) {
        self.unwait(tid);
        self.waiters.push(Waiter { tid, num, zero });
    }

    /// Thread `tid` waits no more.
    pub fn unwait(&mut self, tid: i32) {
        self.waiters.retain(|w| w.tid != tid);
    }

    /// `GETNCNT` (`zero` false) or `GETZCNT` (`zero` true) of semaphore
    /// `num`: the threads waiting on it.
    pub fn waiting(&self, num: u16, zero: bool) -> u64 {
        self.waiters.iter().filter(|w| w.num == num && w.zero == zero).count() as u64
    }

    /// The threads waiting on the set, to wake so they try again.
    pub fn waiters(&self) -> Vec<i32> {
        self.waiters.iter().map(|w| w.tid).collect()
    }
}

/// The sets of one IPC namespace, by id.
#[derive(Debug, Default)]
pub struct SemTable {
    sets: BTreeMap<i32, SemSet>,
    /// Semaphores in all sets.
    semaphores: u64,
    /// The id handed out last (`-1` before the first).
    last: i32,
}

impl SemTable {
    pub const fn new() -> SemTable {
        SemTable { sets: BTreeMap::new(), semaphores: 0, last: -1 }
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

    /// The set with `key` (never [`IPC_PRIVATE`]).
    fn by_key(&self, key: i32) -> Option<(i32, &SemSet)> {
        self.sets.iter().find(|(_, s)| s.key == key).map(|(id, s)| (*id, s))
    }

    /// semget(2) for `key`, `nsems` and `flags` (the low 32 bits of each
    /// argument) by `who` at `now`, in this order:
    ///
    /// - `nsems` below 0 or above [`SEMMSL`]: `EINVAL`, whatever the key;
    /// - [`IPC_PRIVATE`]: always a new set, whatever the flags;
    /// - another key that no set has: `ENOENT` without [`IPC_CREAT`], else
    ///   a new set;
    /// - a key in use: `EEXIST` with `IPC_CREAT | IPC_EXCL`, then `EINVAL`
    ///   if `nsems` is above its size, then `EACCES` if the access asked
    ///   for in `flags & 0777` is not granted ([`permits`]); else its id.
    ///
    /// A new set needs `nsems >= 1` (`EINVAL`) and room in the namespace
    /// ([`SEMMNI`] sets, [`SEMMNS`] semaphores: `ENOSPC`).
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
        if self.sets.len() >= SEMMNI || self.semaphores + nsems as u64 > SEMMNS {
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
        self.semaphores += nsems as u64;
        self.sets.insert(id, SemSet::new(key, nsems as usize, flags, who, now));
        Ok(id)
    }

    /// semctl(`IPC_RMID`) of set `id`: gone at once. Returns it (its
    /// waiters are to be woken; they find the id gone).
    pub fn remove(&mut self, id: i32) -> Option<SemSet> {
        let s = self.sets.remove(&id)?;
        self.semaphores -= s.nsems() as u64;
        Some(s)
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

    fn op(num: u16, op: i16, flg: u16) -> SemBuf {
        SemBuf { num, op, flg }
    }

    const NOWAIT: u16 = IPC_NOWAIT as u16;

    #[test]
    fn nsems_range_comes_before_the_key() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        for nsems in [-1, SEMMSL + 1, i32::MAX, i32::MIN] {
            assert_eq!(t.get(5, nsems, 0, &me, 0), Err(Errno::EINVAL), "unused key: {nsems}");
            assert_eq!(t.get(5, nsems, 0o600 | IPC_CREAT, &me, 0), Err(Errno::EINVAL));
            assert_eq!(t.get(IPC_PRIVATE, nsems, 0o600, &me, 0), Err(Errno::EINVAL));
        }
        assert_eq!(t.get(5, 1, 0, &me, 0), Err(Errno::ENOENT), "nothing made");
        t.get(5, 1, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        assert_eq!(t.get(5, -1, 0, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(5, SEMMSL + 1, IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EINVAL), "before EEXIST");
        assert_eq!(t.get(5, -5, IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EINVAL));
        let id = t.get(IPC_PRIVATE, SEMMSL, 0o600, &me, 0).unwrap();
        assert_eq!(t.set(id).unwrap().nsems(), SEMMSL as usize);
    }

    #[test]
    fn creating_needs_a_semaphore() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        assert_eq!(t.get(5, 0, 0o600 | IPC_CREAT, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(5, 0, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(5, 1, 0, &me, 0), Err(Errno::ENOENT));
        assert_eq!(t.get(IPC_PRIVATE, 0, 0o600, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(IPC_PRIVATE, 0, 0o600 | IPC_CREAT, &me, 0), Err(Errno::EINVAL));
        assert!(t.is_empty());
        let id = t.get(5, 1, 0o600 | IPC_CREAT, &me, 0).unwrap();
        assert_eq!(t.get(5, 0, 0, &me, 0), Ok(id));
        assert_eq!(t.get(5, 0, IPC_CREAT, &me, 0), Ok(id));
    }

    #[test]
    fn keyed_lookup_flags() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        assert_eq!(t.get(7, 1, 0, &me, 0), Err(Errno::ENOENT));
        assert_eq!(t.get(7, 0, 0, &me, 0), Err(Errno::ENOENT));
        assert_eq!(t.get(7, 1, 0o600 | IPC_EXCL, &me, 0), Err(Errno::ENOENT), "IPC_EXCL alone");
        let id = t.get(7, 2, 0o600 | IPC_CREAT, &me, 9).unwrap();
        assert_eq!(t.get(7, 2, 0, &me, 0), Ok(id));
        assert_eq!(t.get(7, 1, 0o600 | IPC_CREAT, &me, 0), Ok(id));
        assert_eq!(t.get(7, 2, 0o600 | IPC_EXCL, &me, 0), Ok(id));
        assert_eq!(t.get(7, 2, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        assert_eq!(t.get(7, 0, IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        let s = t.set(id).unwrap();
        assert_eq!((s.key, s.mode, s.nsems(), s.otime, s.ctime), (7, 0o600, 2, 0, 9));
        let neg = t.get(-7, 1, 0o600 | IPC_CREAT, &me, 0).unwrap();
        assert_ne!(neg, id);
        assert_eq!(t.set(neg).unwrap().key, -7);
        let a = t.get(IPC_PRIVATE, 1, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        let b = t.get(IPC_PRIVATE, 1, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        assert!(a != b && a != id && b != neg);
    }

    #[test]
    fn existing_set_sizes_and_the_order_of_errors() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.get(1, 4, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        assert_eq!(t.get(1, 4, 0, &me, 0), Ok(id));
        assert_eq!(t.get(1, 1, 0, &me, 0), Ok(id));
        assert_eq!(t.get(1, 5, 0, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(1, 5, 0o600 | IPC_CREAT, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(1, 5, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        assert_eq!(t.get(1, SEMMSL, 0, &me, 0), Err(Errno::EINVAL));
        assert_eq!(t.get(1, 5, 0o700, &me, 0), Err(Errno::EINVAL), "before EACCES");
        assert_eq!(t.get(1, 4, 0o700, &me, 0), Err(Errno::EACCES));
        assert_eq!(t.get(1, 4, 0o700, &root(), 0), Ok(id));
        assert_eq!(t.get(1, 4, 0o700 | IPC_CREAT | IPC_EXCL, &me, 0), Err(Errno::EEXIST));
        assert_eq!(t.set(id).unwrap().nsems(), 4);
    }

    #[test]
    fn mode_and_owners_of_a_new_set() {
        let mut t = SemTable::new();
        let me = user(1001, 1002);
        for (flags, mode) in [
            (0o777, 0o777),
            (0o7777, 0o777),
            (0o644 | IPC_NOWAIT | 0o10000 | 0o20000, 0o644),
            (0o600 | 0x7ff0_0000, 0o600),
            (0o460 | IPC_CREAT | IPC_EXCL | 0o40000, 0o460),
        ] {
            let id = t.get(IPC_PRIVATE, 3, flags, &me, 5).unwrap();
            let s = t.set(id).unwrap();
            assert_eq!(s.mode, mode, "{flags:o}");
            assert_eq!(s.owners, Owners { uid: 1001, gid: 1002, cuid: 1001, cgid: 1002 });
            assert!(s.sems.iter().all(|x| *x == Semaphore::default()));
            assert_eq!((s.waiting(0, false), s.waiting(2, true)), (0, 0));
        }
    }

    #[test]
    fn access_by_class() {
        let mut t = SemTable::new();
        let owner = user(1001, 1001);
        let id = t.get(1, 1, 0o640 | IPC_CREAT, &owner, 0).unwrap();
        assert_eq!(t.get(1, 0, 0o600, &owner, 0), Ok(id));
        assert_eq!(t.get(1, 0, 0o100, &owner, 0), Err(Errno::EACCES));
        assert_eq!(t.get(1, 0, 0o004, &user(2000, 1001), 0), Ok(id), "group: read");
        assert_eq!(t.get(1, 0, 0o002, &user(2000, 1001), 0), Err(Errno::EACCES));
        assert_eq!(t.get(1, 0, 0o004, &user(2000, 2000), 0), Err(Errno::EACCES));
        assert_eq!(t.get(1, 0, 0, &user(2000, 2000), 0), Ok(id));
    }

    #[test]
    fn ids_are_distinct_and_a_removed_id_is_not_next() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let ids: Vec<i32> = (0..8).map(|_| t.get(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap()).collect();
        for (i, a) in ids.iter().enumerate() {
            assert!(*a >= 0 && !ids[..i].contains(a));
        }
        for id in &ids {
            assert!(t.remove(*id).is_some());
        }
        let mut prev = t.get(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
        for _ in 0..8 {
            t.remove(prev);
            let next = t.get(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap();
            assert_ne!(next, prev);
            prev = next;
        }
    }

    #[test]
    fn removal_frees_the_key_at_once() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.get(3, 2, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        t.set_mut(id).unwrap().set_value(1, 5, 100, 0).unwrap();
        assert!(t.remove(id).is_some());
        assert!(t.remove(id).is_none());
        assert_eq!(t.get(3, 1, 0, &me, 0), Err(Errno::ENOENT));
        let id2 = t.get(3, 2, 0o600 | IPC_CREAT | IPC_EXCL, &me, 0).unwrap();
        assert_ne!(id2, id);
        assert_eq!(t.set(id2).unwrap().sems[1].val, 0);
    }

    #[test]
    fn the_namespace_holds_at_most_semmni_sets() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let ids: Vec<i32> = (0..SEMMNI).map(|_| t.get(IPC_PRIVATE, 1, 0o600, &me, 0).unwrap()).collect();
        assert_eq!(t.get(IPC_PRIVATE, 1, 0o600, &me, 0), Err(Errno::ENOSPC));
        assert_eq!(t.get(5, 1, 0o600 | IPC_CREAT, &me, 0), Err(Errno::ENOSPC));
        t.remove(ids[0]);
        assert!(t.get(IPC_PRIVATE, 1, 0o600, &me, 0).is_ok());
    }

    #[test]
    fn operations_apply_together_or_not_at_all() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.get(IPC_PRIVATE, 5, 0o600, &me, 0).unwrap();
        let s = t.set_mut(id).unwrap();
        assert_eq!(s.try_ops(&[op(2, -1, NOWAIT)], 7, 1), Err(Errno::EAGAIN));
        assert_eq!(s.try_ops(&[op(4, 0, 0)], 7, 1), Ok((Done::Applied, Vec::new())));
        assert_eq!(s.otime, 1);
        assert_eq!(s.sems[4].pid, 0, "a wait for zero changes nothing");
        assert_eq!(s.try_ops(&[op(0, 3, 0), op(1, 2, 0)], 7, 2).unwrap().0, Done::Applied);
        assert_eq!((s.sems[0], s.sems[1]), (Semaphore { val: 3, pid: 7 }, Semaphore { val: 2, pid: 7 }));
        // The second fails: the first is not applied.
        assert_eq!(s.try_ops(&[op(0, -1, 0), op(1, -3, NOWAIT)], 8, 3), Err(Errno::EAGAIN));
        assert_eq!(s.sems[0], Semaphore { val: 3, pid: 7 });
        assert_eq!(s.try_ops(&[op(0, -1, 0), op(1, -3, 0)], 8, 3), Ok((Done::Wait { num: 1, zero: false }, vec![])));
        assert_eq!(s.try_ops(&[op(0, 0, 0)], 8, 3), Ok((Done::Wait { num: 0, zero: true }, vec![])));
        // The same semaphore twice: seen in order.
        assert_eq!(s.try_ops(&[op(0, -3, 0), op(0, 0, 0)], 8, 4).unwrap().0, Done::Applied);
        assert_eq!(s.sems[0], Semaphore { val: 0, pid: 8 });
        assert_eq!(s.try_ops(&[op(3, i16::MAX, 0)], 8, 4).unwrap().0, Done::Applied);
        assert_eq!(s.try_ops(&[op(3, 1, 0)], 8, 4), Err(Errno::ERANGE));
        assert_eq!(s.sems[3].val, SEMVMX as u16);
    }

    #[test]
    fn waiters_are_counted_and_woken() {
        let mut t = SemTable::new();
        let me = user(1001, 1001);
        let id = t.get(IPC_PRIVATE, 2, 0o600, &me, 0).unwrap();
        let s = t.set_mut(id).unwrap();
        s.wait(10, 0, false);
        s.wait(11, 0, false);
        s.wait(12, 1, true);
        assert_eq!((s.waiting(0, false), s.waiting(0, true), s.waiting(1, true)), (2, 0, 1));
        assert_eq!(s.set_value(0, 1, 100, 5), Ok(vec![10, 11, 12]));
        assert_eq!(s.sems[0], Semaphore { val: 1, pid: 100 });
        assert_eq!(s.ctime, 5);
        s.unwait(10);
        assert_eq!(s.waiting(0, false), 1);
        assert_eq!(s.set_value(0, SEMVMX + 1, 100, 5), Err(Errno::ERANGE));
        assert_eq!(s.set_value(0, -1, 100, 5), Err(Errno::ERANGE));
        assert_eq!(s.set_value(2, 1, 100, 5), Err(Errno::EINVAL));
        assert_eq!(s.set_all(&[4, 5], 101, 6), Ok(vec![11, 12]));
        assert_eq!(s.set_all(&[4, 40000], 101, 6), Err(Errno::ERANGE));
        assert_eq!(t.remove(id).unwrap().waiters(), vec![11, 12]);
    }

    #[test]
    fn semid_ds_layout() {
        let ds =
            SemidDs { key: -7, uid: 1, gid: 2, cuid: 3, cgid: 4, mode: 0o600, seq: 0, otime: 5, ctime: 6, nsems: 7 };
        let b = ds.to_bytes();
        assert_eq!(&b[0..4], &(-7i32).to_ne_bytes());
        assert_eq!(&b[20..22], &0o600u16.to_ne_bytes());
        assert_eq!(&b[48..56], &5i64.to_ne_bytes());
        assert_eq!(&b[64..72], &6i64.to_ne_bytes());
        assert_eq!(&b[80..88], &7u64.to_ne_bytes());
        assert!(b[88..].iter().all(|x| *x == 0));
        assert_eq!(SemidDs::perm_from_bytes(&b), (1, 2, 0o600));
    }
}
