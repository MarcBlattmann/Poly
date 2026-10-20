//! Advisory file locks (ADR 0059): POSIX record locks and open file
//! description locks (fcntl(2) `F_SETLK`, `F_OFD_SETLK`, ...) and whole-file
//! locks (flock(2)), as a table the kernel keeps for every file.
//!
//! The table only decides; the kernel blocks waiters and wakes them when a
//! lock goes, and tells the table when an owner is gone. The rules are
//! fcntl(2)'s and flock(2)'s:
//!
//! - A record lock covers a byte range of a file (to "infinity" when its
//!   length is 0). Two locks conflict when they have different owners,
//!   their ranges overlap and at least one is a write lock. A new lock or
//!   an unlock of an owner replaces that owner's locks in its range
//!   ("converting an existing lock"), splitting and merging as needed, so
//!   an owner's locks never overlap.
//! - The owner of a traditional record lock is the process (all its
//!   threads; the kernel passes an id of the process's descriptor table);
//!   the owner of an open file description lock is the description. Both
//!   kinds conflict with each other ("OFD locks ... conflict with
//!   traditional record locks").
//! - flock(2) locks are whole-file, shared or exclusive, owned by the open
//!   file description, and never interact with record locks ("there is no
//!   interaction between the types of lock placed by flock() and fcntl()").
//!   A conversion is not atomic: the old lock goes first.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A file, as the kernel names it: its file system's device number and its
/// inode number.
pub type FileKey = (u64, u64);

/// Who holds a lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockOwner {
    /// A process (traditional POSIX record locks).
    Process(usize),
    /// An open file description (`F_OFD_*` and flock(2) locks).
    Description(usize),
}

/// Read (shared) or write (exclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LockKind {
    Read,
    Write,
}

/// A record lock: `[start, end)`, `end == u64::MAX` meaning to the end of
/// any file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecordLock {
    pub owner: LockOwner,
    /// What `F_GETLK` reports as `l_pid`: the owning process, -1 for an
    /// open file description lock.
    pub pid: i32,
    pub kind: LockKind,
    pub start: u64,
    pub end: u64,
}

impl RecordLock {
    fn overlaps(&self, start: u64, end: u64) -> bool {
        self.start < end && start < self.end
    }

    fn conflicts(&self, owner: LockOwner, kind: LockKind, start: u64, end: u64) -> bool {
        self.owner != owner && self.overlaps(start, end) && (self.kind == LockKind::Write || kind == LockKind::Write)
    }
}

/// A flock(2) lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WholeLock {
    owner: LockOwner,
    exclusive: bool,
}

#[derive(Clone, Debug, Default)]
struct FileLocks {
    /// Sorted by owner, then start.
    records: Vec<RecordLock>,
    whole: Vec<WholeLock>,
}

impl FileLocks {
    fn is_empty(&self) -> bool {
        self.records.is_empty() && self.whole.is_empty()
    }
}

/// The locks of every file. See the module documentation.
#[derive(Clone, Debug, Default)]
pub struct LockTable {
    files: BTreeMap<FileKey, FileLocks>,
}

impl LockTable {
    pub const fn new() -> LockTable {
        LockTable { files: BTreeMap::new() }
    }

    /// Whether no file has a lock (the kernel skips its work on closes).
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// `F_GETLK`: the first lock of another owner that would keep `owner`
    /// from taking a `kind` lock on `[start, end)`, if any.
    pub fn test(&self, file: FileKey, owner: LockOwner, kind: LockKind, start: u64, end: u64) -> Option<RecordLock> {
        let locks = self.files.get(&file)?;
        locks.records.iter().find(|l| l.conflicts(owner, kind, start, end)).copied()
    }

    /// `F_SETLK`: give `owner` a `kind` lock on `[start, end)` (`None`:
    /// unlock the range), replacing its own locks there. `Err` with a
    /// conflicting lock of another owner, changing nothing.
    pub fn set(
        &mut self,
        file: FileKey,
        owner: LockOwner,
        pid: i32,
        kind: Option<LockKind>,
        start: u64,
        end: u64,
    ) -> Result<(), RecordLock> {
        if start >= end {
            return Ok(());
        }
        if let Some(kind) = kind
            && let Some(c) = self.test(file, owner, kind, start, end)
        {
            return Err(c);
        }
        let locks = self.files.entry(file).or_default();
        let mut mine: Vec<RecordLock> = Vec::new();
        let mut others: Vec<RecordLock> = Vec::new();
        for l in locks.records.drain(..) {
            if l.owner == owner { mine.push(l) } else { others.push(l) }
        }
        // Cut the range out of the owner's locks.
        let mut kept: Vec<RecordLock> = Vec::new();
        for l in mine {
            if !l.overlaps(start, end) {
                kept.push(l);
                continue;
            }
            if l.start < start {
                kept.push(RecordLock { end: start, ..l });
            }
            if l.end > end {
                kept.push(RecordLock { start: end, ..l });
            }
        }
        if let Some(kind) = kind {
            kept.push(RecordLock { owner, pid, kind, start, end });
        }
        // Merge neighbours of the same kind.
        kept.sort_by_key(|l| l.start);
        let mut merged: Vec<RecordLock> = Vec::new();
        for l in kept {
            match merged.last_mut() {
                Some(last) if last.kind == l.kind && last.end == l.start => last.end = l.end,
                _ => merged.push(l),
            }
        }
        others.extend(merged);
        others.sort_by_key(|l| (l.owner, l.start));
        locks.records = others;
        if locks.is_empty() {
            self.files.remove(&file);
        }
        Ok(())
    }

    /// The record locks of `owner` on `file`, in order (tests, procfs).
    pub fn records_of(&self, file: FileKey, owner: LockOwner) -> Vec<RecordLock> {
        self.files.get(&file).map_or_else(Vec::new, |l| l.records.iter().filter(|r| r.owner == owner).copied().collect())
    }

    /// flock(2) `LOCK_SH` / `LOCK_EX`: `Err(())` if another description
    /// holds a lock that conflicts, after the owner's old lock (if any)
    /// was given up.
    pub fn flock(&mut self, file: FileKey, owner: LockOwner, exclusive: bool) -> Result<(), ()> {
        self.unflock(file, owner);
        let locks = self.files.entry(file).or_default();
        if locks.whole.iter().any(|w| w.owner != owner && (w.exclusive || exclusive)) {
            if locks.is_empty() {
                self.files.remove(&file);
            }
            return Err(());
        }
        locks.whole.push(WholeLock { owner, exclusive });
        Ok(())
    }

    /// Whether `flock` would succeed now (it gives up the old lock first).
    pub fn flock_would_block(&self, file: FileKey, owner: LockOwner, exclusive: bool) -> bool {
        self.files
            .get(&file)
            .is_some_and(|l| l.whole.iter().any(|w| w.owner != owner && (w.exclusive || exclusive)))
    }

    /// flock(2) `LOCK_UN`.
    pub fn unflock(&mut self, file: FileKey, owner: LockOwner) {
        if let Some(locks) = self.files.get_mut(&file) {
            locks.whole.retain(|w| w.owner != owner);
            if locks.is_empty() {
                self.files.remove(&file);
            }
        }
    }

    /// Every lock of `owner` on `file` goes (a process closed a descriptor
    /// of the file). Returns whether any went.
    pub fn release(&mut self, file: FileKey, owner: LockOwner) -> bool {
        let Some(locks) = self.files.get_mut(&file) else { return false };
        let before = locks.records.len() + locks.whole.len();
        locks.records.retain(|l| l.owner != owner);
        locks.whole.retain(|w| w.owner != owner);
        let released = locks.records.len() + locks.whole.len() != before;
        if locks.is_empty() {
            self.files.remove(&file);
        }
        released
    }

    /// Every lock of every owner `alive` says is gone goes (descriptions
    /// that were released). Returns whether any went.
    pub fn retain_owners(&mut self, mut alive: impl FnMut(LockOwner) -> bool) -> bool {
        let mut released = false;
        self.files.retain(|_, locks| {
            let before = locks.records.len() + locks.whole.len();
            locks.records.retain(|l| alive(l.owner));
            locks.whole.retain(|w| alive(w.owner));
            released |= locks.records.len() + locks.whole.len() != before;
            !locks.is_empty()
        });
        released
    }

    /// The description owners that hold a lock somewhere.
    pub fn descriptions(&self) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for locks in self.files.values() {
            let owners = locks.records.iter().map(|l| l.owner).chain(locks.whole.iter().map(|w| w.owner));
            for o in owners {
                if let LockOwner::Description(id) = o
                    && !out.contains(&id)
                {
                    out.push(id);
                }
            }
        }
        out
    }
}
