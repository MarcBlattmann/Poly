//! System V shared memory segments (shmget(2), shmat(2), shmdt(2),
//! shmctl(2), svipc(7)): the segments of one IPC namespace.
//!
//! A segment belongs to its IPC namespace, not to a process: it lives in
//! the namespace's [`ShmTable`] from shmget(2) until shmctl(`IPC_RMID`)
//! and the last detach. The table keeps what `IPC_STAT` reports (key,
//! owner, creator, mode, size, times, pids); the kernel keeps the memory
//! ([`ShmMemory`], made by [`LinuxKernel::shm_create`]) and counts the
//! attachments, which are its mappings in every process.
//!
//! `IPC_RMID` releases the key at once; the segment stays (key 0,
//! [`SHM_DEST`] in the mode) while it is attached and goes when the
//! kernel reports no attachments left ([`ShmTable::purge`], run by every
//! call that looks at the table). Ids are handed out in increasing order
//! (wrapping at `i32::MAX`), so the id just removed is not the next one.
//!
//! The table sits behind the namespace lock ([`crate::ns::lock::Mutex`], a
//! spinlock): handlers decide under it and call the kernel outside it.
//!
//! [`LinuxKernel::shm_create`]: crate::kernel::LinuxKernel::shm_create

use crate::abi::{PAGE_SIZE, page_round_up};
use crate::errno::Errno;
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub use super::{
    EXEC, IPC_64, IPC_CREAT, IPC_EXCL, IPC_PRIVATE, IPC_RMID, IPC_SET, IPC_STAT, MODE_BITS, Owners, READ, WRITE, Who,
    permits,
};

/// shmget flags (`linux/shm.h`).
pub const SHM_HUGETLB: u32 = 0o4000;
pub const SHM_NORESERVE: u32 = 0o10000;
/// shmat flags (`linux/shm.h`).
pub const SHM_RDONLY: u32 = 0o10000;
pub const SHM_RND: u32 = 0o20000;
pub const SHM_REMAP: u32 = 0o40000;
pub const SHM_EXEC: u32 = 0o100000;
/// Mode bit of a removed segment that is still attached (`linux/shm.h`).
pub const SHM_DEST: u16 = 0o1000;

/// Limits (`linux/shm.h`): the smallest and largest segment, the most
/// segments, and the most pages of all segments together.
pub const SHMMIN: u64 = 1;
pub const SHMMAX: u64 = u64::MAX - (1 << 24);
pub const SHMMNI: usize = 4096;
pub const SHMALL: u64 = u64::MAX - (1 << 24);
/// Attach addresses are multiples of this (`asm-generic/shmparam.h`: the
/// page size on x86-64).
pub const SHMLBA: u64 = PAGE_SIZE;

/// The memory of a segment, as the kernel made it. The table holds one
/// reference; every attachment (a mapping, in any process) is the kernel's.
pub trait ShmMemory: Send + Sync + core::fmt::Debug {
    /// How many attachments the memory has now (`shm_nattch`).
    fn attachments(&self) -> u64;
    /// A number naming this memory among all segments the kernel ever
    /// made: what [`LinuxKernel::shm_detach`] reports.
    ///
    /// [`LinuxKernel::shm_detach`]: crate::kernel::LinuxKernel::shm_detach
    fn serial(&self) -> u64;
    /// The kernel's own type behind the trait object.
    fn as_any(&self) -> &dyn core::any::Any;
}

/// One segment.
#[derive(Clone, Debug)]
pub struct Segment {
    /// The key it was made with; [`IPC_PRIVATE`] once removed.
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    /// Permission bits, plus [`SHM_DEST`] once removed.
    pub mode: u16,
    /// The size asked for, in bytes (attachments cover whole pages).
    pub segsz: u64,
    /// Seconds since the epoch of the last attach, detach and change.
    pub atime: i64,
    pub dtime: i64,
    pub ctime: i64,
    /// Global pids of the creator and of the last attacher or detacher.
    pub cpid: i32,
    pub lpid: i32,
    pub memory: Arc<dyn ShmMemory>,
}

impl Segment {
    /// A new segment of `segsz` bytes with mode `flags & 0777`, made by
    /// `who` (owner and creator), process `pid`, at `now`.
    pub fn new(key: i32, segsz: u64, flags: u32, who: &Who, pid: i32, now: i64, memory: Arc<dyn ShmMemory>) -> Segment {
        Segment {
            key,
            uid: who.euid,
            gid: who.egid,
            cuid: who.euid,
            cgid: who.egid,
            mode: (flags & MODE_BITS) as u16,
            segsz,
            atime: 0,
            dtime: 0,
            ctime: now,
            cpid: pid,
            lpid: 0,
            memory,
        }
    }

    /// Whether shmctl(`IPC_RMID`) removed it.
    pub fn removed(&self) -> bool {
        self.mode & SHM_DEST != 0
    }

    /// `shm_nattch`.
    pub fn nattch(&self) -> u64 {
        self.memory.attachments()
    }

    /// Pages its attachments cover.
    pub fn pages(&self) -> u64 {
        self.segsz.div_ceil(PAGE_SIZE)
    }

    fn owners(&self) -> Owners {
        Owners { uid: self.uid, gid: self.gid, cuid: self.cuid, cgid: self.cgid }
    }

    /// Whether `who` may have the access in `request` ([`permits`]).
    pub fn allows(&self, who: &Who, request: u32) -> bool {
        permits(&self.owners(), self.mode, who, request)
    }

    /// Whether `who` may change or remove it ([`Owners::owned_by`]).
    pub fn owned_by(&self, who: &Who) -> bool {
        self.owners().owned_by(who)
    }
}

/// What shmget(2) does about a key, decided by [`ShmTable::get`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The live segment with this id.
    Found(i32),
    /// Make a new segment (then [`ShmTable::insert`]).
    Create,
}

/// The segments of one IPC namespace, by id.
#[derive(Debug, Default)]
pub struct ShmTable {
    segments: BTreeMap<i32, Segment>,
    /// The id handed out last (`-1` before the first).
    last: i32,
}

impl ShmTable {
    pub const fn new() -> ShmTable {
        ShmTable { segments: BTreeMap::new(), last: -1 }
    }

    /// Segments that exist, removed ones still attached included.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn segment(&self, id: i32) -> Option<&Segment> {
        self.segments.get(&id)
    }

    pub fn segment_mut(&mut self, id: i32) -> Option<&mut Segment> {
        self.segments.get_mut(&id)
    }

    /// The segment whose memory has this [`ShmMemory::serial`].
    pub fn by_serial(&mut self, serial: u64) -> Option<&mut Segment> {
        self.segments.values_mut().find(|s| s.memory.serial() == serial)
    }

    /// The live segment with `key` (never [`IPC_PRIVATE`]).
    fn by_key(&self, key: i32) -> Option<(i32, &Segment)> {
        self.segments.iter().find(|(_, s)| s.key == key && !s.removed()).map(|(id, s)| (*id, s))
    }

    /// Take out the removed segments without attachments: their ids are
    /// gone. The caller drops them once it has let go of the lock.
    pub fn purge(&mut self) -> Vec<Segment> {
        let dead: Vec<i32> =
            self.segments.iter().filter(|(_, s)| s.removed() && s.nattch() == 0).map(|(id, _)| *id).collect();
        dead.into_iter().filter_map(|id| self.segments.remove(&id)).collect()
    }

    /// shmget(2)'s decision for `key`, `size` and `flags` (the low 32 bits
    /// of `shmflg`), in this order:
    ///
    /// - [`IPC_PRIVATE`]: always a new segment, whatever the flags;
    /// - another key that no live segment has: `ENOENT` without
    ///   [`IPC_CREAT`] (before any size check), else a new segment;
    /// - a key in use: `EEXIST` with `IPC_CREAT | IPC_EXCL`, then `EINVAL`
    ///   if `size` is above its size, then `EACCES` if the access asked
    ///   for in `flags & 0777` is not granted ([`Segment::allows`]).
    ///
    /// A new segment needs `SHMMIN <= size <= SHMMAX` and a page-rounded
    /// size a file offset can hold (`EINVAL`), the privilege for
    /// [`SHM_HUGETLB`] (`EPERM`; Poly has no huge pages, so with it the
    /// call is `ENOMEM`), and room in the namespace ([`SHMMNI`] segments,
    /// [`SHMALL`] pages: `ENOSPC`).
    pub fn get(&self, key: i32, size: u64, flags: u32, who: &Who) -> Result<Lookup, Errno> {
        if key != IPC_PRIVATE {
            match self.by_key(key) {
                Some((id, s)) => {
                    if flags & (IPC_CREAT | IPC_EXCL) == IPC_CREAT | IPC_EXCL {
                        return Err(Errno::EEXIST);
                    }
                    if size > s.segsz {
                        return Err(Errno::EINVAL);
                    }
                    if !s.allows(who, flags) {
                        return Err(Errno::EACCES);
                    }
                    return Ok(Lookup::Found(id));
                }
                None if flags & IPC_CREAT == 0 => return Err(Errno::ENOENT),
                None => {}
            }
        }
        if !(SHMMIN..=SHMMAX).contains(&size) {
            return Err(Errno::EINVAL);
        }
        // The memory is a file of this many bytes: at most the largest
        // file offset (off_t).
        let bytes = page_round_up(size).ok_or(Errno::EINVAL)?;
        if bytes > i64::MAX as u64 {
            return Err(Errno::EINVAL);
        }
        if flags & SHM_HUGETLB != 0 {
            return Err(if who.ipc_lock { Errno::ENOMEM } else { Errno::EPERM });
        }
        self.room_for(bytes / PAGE_SIZE)?;
        Ok(Lookup::Create)
    }

    /// `ENOSPC` unless one more segment of `pages` fits.
    fn room_for(&self, pages: u64) -> Result<(), Errno> {
        let total = self.segments.values().fold(0u64, |t, s| t.saturating_add(s.pages()));
        if self.segments.len() >= SHMMNI || total.saturating_add(pages) > SHMALL {
            return Err(Errno::ENOSPC);
        }
        Ok(())
    }

    /// Add `segment`, made after [`ShmTable::get`] said
    /// [`Lookup::Create`], and return its id. If the table changed
    /// meanwhile so that it no longer fits (its key is in use now, or the
    /// namespace is full), the segment comes back and the caller asks
    /// [`ShmTable::get`] again.
    pub fn insert(&mut self, segment: Segment) -> Result<i32, Segment> {
        if (segment.key != IPC_PRIVATE && self.by_key(segment.key).is_some()) || self.room_for(segment.pages()).is_err()
        {
            return Err(segment);
        }
        let mut id = self.last;
        loop {
            id = if id == i32::MAX { 0 } else { id + 1 };
            if !self.segments.contains_key(&id) {
                break;
            }
        }
        self.last = id;
        self.segments.insert(id, segment);
        Ok(id)
    }

    /// shmctl(`IPC_RMID`) of segment `id`: its key is released now; it
    /// goes at once if nothing is attached (returned, for the caller to
    /// drop), else at the last detach.
    pub fn remove(&mut self, id: i32) -> Option<Segment> {
        let s = self.segments.get_mut(&id)?;
        s.key = IPC_PRIVATE;
        s.mode |= SHM_DEST;
        if s.nattch() == 0 { self.segments.remove(&id) } else { None }
    }
}

/// The segments of the initial IPC namespace.
static INITIAL: Mutex<ShmTable> = Mutex::new(ShmTable::new());

/// Run `f` on the segment table of the IPC namespace `ns` (`None`: the
/// initial one), locked. `f` must not block or call the kernel.
pub fn with_table<R>(ns: &Option<Arc<IpcNs>>, f: impl FnOnce(&mut ShmTable) -> R) -> R {
    match ns {
        Some(ns) => f(&mut ns.shm().lock()),
        None => f(&mut INITIAL.lock()),
    }
}

/// `struct shmid64_ds` on x86-64 (`asm-generic/shmbuf.h`, with `struct
/// ipc64_perm` from `asm-generic/ipcbuf.h`): what `IPC_STAT` writes and
/// `IPC_SET` reads. Ids and pids as the caller sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShmidDs {
    pub key: i32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub mode: u16,
    pub seq: u16,
    pub segsz: u64,
    pub atime: i64,
    pub dtime: i64,
    pub ctime: i64,
    pub cpid: i32,
    pub lpid: i32,
    pub nattch: u64,
}

impl ShmidDs {
    /// `sizeof(struct shmid64_ds)`.
    pub const SIZE: usize = 112;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.key.to_ne_bytes());
        b[4..8].copy_from_slice(&self.uid.to_ne_bytes());
        b[8..12].copy_from_slice(&self.gid.to_ne_bytes());
        b[12..16].copy_from_slice(&self.cuid.to_ne_bytes());
        b[16..20].copy_from_slice(&self.cgid.to_ne_bytes());
        b[20..22].copy_from_slice(&self.mode.to_ne_bytes());
        b[24..26].copy_from_slice(&self.seq.to_ne_bytes());
        b[48..56].copy_from_slice(&self.segsz.to_ne_bytes());
        b[56..64].copy_from_slice(&self.atime.to_ne_bytes());
        b[64..72].copy_from_slice(&self.dtime.to_ne_bytes());
        b[72..80].copy_from_slice(&self.ctime.to_ne_bytes());
        b[80..84].copy_from_slice(&self.cpid.to_ne_bytes());
        b[84..88].copy_from_slice(&self.lpid.to_ne_bytes());
        b[88..96].copy_from_slice(&self.nattch.to_ne_bytes());
        b
    }

    /// What `IPC_SET` takes from the structure: `shm_perm.uid`,
    /// `shm_perm.gid` and `shm_perm.mode`.
    pub fn perm_from_bytes(b: &[u8; Self::SIZE]) -> (u32, u32, u16) {
        let word = |at: usize| u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        (word(4), word(8), u16::from_ne_bytes([b[20], b[21]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug, Default)]
    struct Mem {
        attached: AtomicU64,
        serial: u64,
    }

    impl ShmMemory for Mem {
        fn attachments(&self) -> u64 {
            self.attached.load(Ordering::Relaxed)
        }
        fn serial(&self) -> u64 {
            self.serial
        }
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
    }

    fn user(uid: u32, gid: u32) -> Who {
        Who { euid: uid, egid: gid, groups: Arc::from([]), ipc_owner: false, sys_admin: false, ipc_lock: false }
    }

    fn root() -> Who {
        Who { ipc_owner: true, sys_admin: true, ipc_lock: true, ..user(0, 0) }
    }

    /// shmget as the handler does it, without the kernel.
    fn get(t: &mut ShmTable, key: i32, size: u64, flags: u32, who: &Who) -> Result<i32, Errno> {
        match t.get(key, size, flags, who)? {
            Lookup::Found(id) => Ok(id),
            Lookup::Create => {
                let mem = Arc::new(Mem { serial: t.len() as u64, ..Mem::default() });
                Ok(t.insert(Segment::new(key, size, flags, who, 100, 1_000, mem)).expect("fits"))
            }
        }
    }

    fn attach(t: &ShmTable, id: i32, n: i64) {
        let s = t.segment(id).unwrap();
        let mem = s.memory.as_any().downcast_ref::<Mem>().unwrap();
        mem.attached.fetch_add(n as u64, Ordering::Relaxed);
    }

    #[test]
    fn private_key_makes_a_new_segment_whatever_the_flags() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        let a = get(&mut t, IPC_PRIVATE, 16, 0o600, &me).unwrap();
        let b = get(&mut t, IPC_PRIVATE, 16, 0o600 | IPC_CREAT | IPC_EXCL, &me).unwrap();
        assert_ne!(a, b);
        assert!(a >= 0 && b >= 0);
    }

    #[test]
    fn ids_are_distinct_and_a_removed_id_is_not_next() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        let ids: Vec<i32> = (0..64).map(|_| get(&mut t, IPC_PRIVATE, 16, 0o600, &me).unwrap()).collect();
        for (i, a) in ids.iter().enumerate() {
            assert!(!ids[..i].contains(a));
        }
        for id in &ids {
            assert!(t.remove(*id).is_some(), "unattached: gone at once");
        }
        assert!(t.is_empty());
        let a = get(&mut t, IPC_PRIVATE, 16, 0o600, &me).unwrap();
        t.remove(a);
        let b = get(&mut t, IPC_PRIVATE, 16, 0o600, &me).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn keyed_lookup_flags() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        assert_eq!(get(&mut t, 7, 100, 0o600, &me), Err(Errno::ENOENT));
        // ENOENT is checked before the size.
        assert_eq!(t.get(7, 0, 0, &me), Err(Errno::ENOENT));
        assert_eq!(t.get(7, u64::MAX, 0, &me), Err(Errno::ENOENT));
        let id = get(&mut t, 7, 100, 0o600 | IPC_CREAT, &me).unwrap();
        assert_eq!(get(&mut t, 7, 100, 0, &me), Ok(id));
        assert_eq!(get(&mut t, 7, 100, IPC_CREAT, &me), Ok(id));
        assert_eq!(get(&mut t, 7, 100, IPC_EXCL, &me), Ok(id), "IPC_EXCL alone is ignored");
        assert_eq!(get(&mut t, 7, 100, IPC_CREAT | IPC_EXCL, &me), Err(Errno::EEXIST));
        // The key is the low 32 bits as an int: -7 is a key of its own.
        let neg = get(&mut t, -7, 100, 0o600 | IPC_CREAT, &me).unwrap();
        assert_ne!(neg, id);
        assert_eq!(t.segment(neg).unwrap().key, -7);
    }

    #[test]
    fn sizes_of_new_segments() {
        let t = ShmTable::new();
        let me = user(1001, 1001);
        for flags in [0o600, 0o600 | IPC_CREAT | IPC_EXCL] {
            assert_eq!(t.get(IPC_PRIVATE, 0, flags, &me), Err(Errno::EINVAL));
        }
        assert_eq!(t.get(IPC_PRIVATE, u64::MAX, 0o600, &me), Err(Errno::EINVAL));
        assert_eq!(t.get(IPC_PRIVATE, 1 << 63, 0o600, &me), Err(Errno::EINVAL));
        assert_eq!(t.get(5, 0, 0o600 | IPC_CREAT, &me), Err(Errno::EINVAL));
        assert_eq!(t.get(5, u64::MAX, 0o600 | IPC_CREAT, &me), Err(Errno::EINVAL));
        for size in [1, 4096, 1 << 20] {
            assert_eq!(t.get(IPC_PRIVATE, size, 0o600, &me), Ok(Lookup::Create));
        }
    }

    #[test]
    fn sizes_of_existing_segments_and_the_order_of_errors() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        let id = get(&mut t, 5, 5000, 0o600 | IPC_CREAT, &me).unwrap();
        for size in [5000, 1, 0] {
            assert_eq!(get(&mut t, 5, size, 0, &me), Ok(id));
        }
        assert_eq!(get(&mut t, 5, 5001, 0, &me), Err(Errno::EINVAL));
        assert_eq!(get(&mut t, 5, 8192, 0o600 | IPC_CREAT, &me), Err(Errno::EINVAL));
        assert_eq!(get(&mut t, 5, 8192, 0o600 | IPC_CREAT | IPC_EXCL, &me), Err(Errno::EEXIST));
        assert_eq!(t.segment(id).unwrap().segsz, 5000);

        let ro = get(&mut t, 6, 100, 0o400 | IPC_CREAT, &me).unwrap();
        assert_eq!(get(&mut t, 6, 200, 0o600, &me), Err(Errno::EINVAL), "size before permission");
        assert_eq!(get(&mut t, 6, 200, 0o600 | IPC_CREAT | IPC_EXCL, &me), Err(Errno::EEXIST));
        assert_eq!(get(&mut t, 6, 100, 0o600, &me), Err(Errno::EACCES));
        assert_eq!(get(&mut t, 6, 100, 0o400, &me), Ok(ro));
    }

    #[test]
    fn mode_is_the_low_nine_bits() {
        let mut t = ShmTable::new();
        let me = user(1001, 1002);
        for (flags, mode) in [
            (0o754 | IPC_CREAT | IPC_EXCL, 0o754),
            (0o600 | SHM_NORESERVE, 0o600),
            (0o600 | 0x10_0000, 0o600),
            (0o600 | 0x8000_0000, 0o600),
            (0o777, 0o777),
        ] {
            let id = get(&mut t, IPC_PRIVATE, 100, flags, &me).unwrap();
            let s = t.segment(id).unwrap();
            assert_eq!(s.mode, mode);
            assert_eq!((s.uid, s.cuid, s.gid, s.cgid), (1001, 1001, 1002, 1002));
            assert_eq!((s.segsz, s.cpid, s.lpid, s.atime, s.dtime, s.ctime), (100, 100, 0, 0, 0, 1_000));
        }
    }

    #[test]
    fn access_merges_the_classes_and_checks_the_callers() {
        let mut t = ShmTable::new();
        let owner = user(1001, 1001);
        let id = get(&mut t, 1, 100, 0o640 | IPC_CREAT | IPC_EXCL, &owner).unwrap();
        for ok in [0, 0o600, 0o400, 0o040, 0o020, 0o006] {
            assert_eq!(get(&mut t, 1, 0, ok, &owner), Ok(id), "{ok:o}");
        }
        for denied in [0o100, 0o001, 0o777, 0o700 | IPC_CREAT] {
            assert_eq!(get(&mut t, 1, 0, denied, &owner), Err(Errno::EACCES), "{denied:o}");
            assert_eq!(get(&mut t, 1, 0, denied, &root()), Ok(id), "root: {denied:o}");
        }
        // Group: read only; other: nothing.
        let group = user(2000, 1001);
        assert_eq!(get(&mut t, 1, 0, 0o400, &group), Ok(id));
        assert_eq!(get(&mut t, 1, 0, 0o200, &group), Err(Errno::EACCES));
        let member = Who { groups: Arc::from([1001]), ..user(2000, 3000) };
        assert_eq!(get(&mut t, 1, 0, 0o004, &member), Ok(id), "a supplementary group");
        let other = user(2000, 2000);
        assert_eq!(get(&mut t, 1, 0, 0o004, &other), Err(Errno::EACCES));
        assert_eq!(get(&mut t, 1, 0, 0, &other), Ok(id));

        let mode0 = get(&mut t, 3, 100, IPC_CREAT | IPC_EXCL, &owner).unwrap();
        assert_eq!(get(&mut t, 3, 0, 0, &owner), Ok(mode0));
        assert_eq!(get(&mut t, 3, 0, 0o400, &owner), Err(Errno::EACCES));
    }

    #[test]
    fn removing_an_attached_segment_releases_its_key() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        let id = get(&mut t, 5, 4096, 0o600 | IPC_CREAT | IPC_EXCL, &me).unwrap();
        attach(&t, id, 1);
        assert!(t.remove(id).is_none(), "still attached");
        let s = t.segment(id).unwrap();
        assert_eq!((s.key, s.mode, s.nattch()), (0, SHM_DEST | 0o600, 1));
        assert_eq!(get(&mut t, 5, 0, 0, &me), Err(Errno::ENOENT));
        let id2 = get(&mut t, 5, 100, 0o600 | IPC_CREAT | IPC_EXCL, &me).unwrap();
        assert_ne!(id2, id);
        assert_eq!(get(&mut t, 5, 0, 0, &me), Ok(id2));
        assert!(t.purge().is_empty());
        attach(&t, id, -1);
        assert_eq!(t.purge().len(), 1);
        assert!(t.segment(id).is_none());
        assert_eq!(get(&mut t, 5, 0, 0, &me), Ok(id2));
    }

    #[test]
    fn insert_refuses_a_key_taken_meanwhile() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        assert_eq!(t.get(9, 10, IPC_CREAT, &me), Ok(Lookup::Create));
        let id = get(&mut t, 9, 10, IPC_CREAT, &me).unwrap();
        let late = Segment::new(9, 10, IPC_CREAT, &me, 1, 0, Arc::new(Mem::default()));
        assert!(t.insert(late).is_err());
        assert_eq!(t.get(9, 10, IPC_CREAT, &me), Ok(Lookup::Found(id)));
    }

    #[test]
    fn the_namespace_holds_at_most_shmmni_segments() {
        let mut t = ShmTable::new();
        let me = user(1001, 1001);
        for _ in 0..SHMMNI {
            get(&mut t, IPC_PRIVATE, 1, 0o600, &me).unwrap();
        }
        assert_eq!(t.get(IPC_PRIVATE, 1, 0o600, &me), Err(Errno::ENOSPC));
    }

    #[test]
    fn hugetlb_needs_privilege_and_has_no_pages() {
        let t = ShmTable::new();
        assert_eq!(t.get(IPC_PRIVATE, 1 << 21, SHM_HUGETLB | 0o600, &user(1001, 1001)), Err(Errno::EPERM));
        assert_eq!(t.get(IPC_PRIVATE, 1 << 21, SHM_HUGETLB | 0o600, &root()), Err(Errno::ENOMEM));
    }

    #[test]
    fn shmid_ds_layout() {
        let ds = ShmidDs {
            key: -7,
            uid: 1,
            gid: 2,
            cuid: 3,
            cgid: 4,
            mode: 0o1600,
            seq: 0,
            segsz: 5000,
            atime: 6,
            dtime: 7,
            ctime: 8,
            cpid: 9,
            lpid: 10,
            nattch: 11,
        };
        let b = ds.to_bytes();
        assert_eq!(&b[0..4], &(-7i32).to_ne_bytes());
        assert_eq!(&b[20..22], &0o1600u16.to_ne_bytes());
        assert_eq!(&b[48..56], &5000u64.to_ne_bytes());
        assert_eq!(&b[80..84], &9i32.to_ne_bytes());
        assert_eq!(&b[88..96], &11u64.to_ne_bytes());
        assert_eq!(ShmidDs::perm_from_bytes(&b), (1, 2, 0o1600));
    }
}
