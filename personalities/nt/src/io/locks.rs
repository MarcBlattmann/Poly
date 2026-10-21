//! NT byte-range locks (`NtLockFile` / `NtUnlockFile`) and the mandatory
//! checks reads and writes make against them.
//!
//! The rules (Learn: "LockFileEx", "UnlockFileEx", "ZwLockFile",
//! "ZwUnlockFile", "ZwReadFile"):
//!
//! - A lock covers `[offset, offset + length)` and may extend past the end
//!   of the file. It belongs to the file object that took it and the `Key`
//!   it was taken with ("Key: a caller-assigned value used to describe groups
//!   of related locks"); together those are the lock's *owner*.
//! - An exclusive lock may not overlap any existing lock; a shared lock may
//!   overlap shared locks only. A request that cannot be granted fails at
//!   once with `STATUS_LOCK_NOT_GRANTED` when `FailImmediately`, else waits
//!   until it can be granted.
//! - Locks are independent records: they are never merged or split, so a
//!   region locked twice needs two unlocks, and an unlock must name exactly
//!   the offset, length and key of one lock its file object holds
//!   (`STATUS_RANGE_NOT_LOCKED` otherwise).
//! - Locks are mandatory: a read overlapping another owner's exclusive lock,
//!   and a write overlapping any lock but an exclusive lock of its own
//!   owner, fail with `STATUS_FILE_LOCK_CONFLICT` ("a shared lock denies all
//!   processes write access to the region, including the process that first
//!   locks it").
//! - A file object's locks go when it closes.
//!
//! Zero-length locks are granted but conflict with nothing (to be confirmed
//! by the spec agent). Offsets and lengths are unsigned 64-bit values
//! (`LockFileEx` takes them as `DWORD` pairs and "lock the whole file" is
//! offset 0, length `MAXDWORD`:`MAXDWORD`); an offset + length past 2^64 - 1
//! is `STATUS_INVALID_LOCK_RANGE` (to be confirmed by the spec agent).
//!
//! These are not poly-vfs's POSIX lock table (`poly_vfs::locks`, ADR 0059):
//! POSIX locks merge, split and convert per owner, NT locks do not, and NT
//! locks are mandatory. The two tables do not see each other: a Linux
//! process's `fcntl` locks and an NT process's locks on one file do not
//! conflict (a later milestone can join them).

use super::NodeKey;
use super::consts::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;

/// Who holds a lock: a file object (its cookie) and the request's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockOwner {
    pub file: u64,
    pub key: u32,
}

/// One granted lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteLock {
    pub owner: LockOwner,
    pub offset: u64,
    pub length: u64,
    pub exclusive: bool,
}

impl ByteLock {
    fn end(&self) -> u64 {
        self.offset + self.length
    }

    fn overlaps(&self, offset: u64, length: u64) -> bool {
        self.length != 0 && length != 0 && self.offset < offset + length && offset < self.end()
    }
}

/// Check a lock range: offset + length must fit in 64 bits.
pub fn check_range(offset: u64, length: u64) -> Result<(u64, u64), NtStatus> {
    offset.checked_add(length).ok_or(STATUS_INVALID_LOCK_RANGE)?;
    Ok((offset, length))
}

/// Every file's locks.
#[derive(Debug, Default)]
pub struct LockTable {
    files: BTreeMap<NodeKey, Vec<ByteLock>>,
}

impl LockTable {
    /// Whether `lock` could be granted now.
    pub fn can_grant(&self, file: NodeKey, lock: &ByteLock) -> bool {
        let Some(held) = self.files.get(&file) else { return true };
        !held.iter().any(|h| h.overlaps(lock.offset, lock.length) && (h.exclusive || lock.exclusive))
    }

    /// Take `lock` if it can be granted (`STATUS_LOCK_NOT_GRANTED` if not).
    pub fn lock(&mut self, file: NodeKey, lock: ByteLock) -> Result<(), NtStatus> {
        if !self.can_grant(file, &lock) {
            return Err(STATUS_LOCK_NOT_GRANTED);
        }
        self.files.entry(file).or_default().push(lock);
        Ok(())
    }

    /// Drop the lock of `owner` with exactly this range
    /// (`STATUS_RANGE_NOT_LOCKED` if there is none). If one exclusive and
    /// one shared lock match, the exclusive one goes first (Learn,
    /// "UnlockFileEx": "if the same range is locked with an exclusive and a
    /// shared lock, two unlock operations are necessary"; which goes first is
    /// to be confirmed by the spec agent).
    pub fn unlock(&mut self, file: NodeKey, owner: LockOwner, offset: u64, length: u64) -> Result<(), NtStatus> {
        let held = self.files.get_mut(&file).ok_or(STATUS_RANGE_NOT_LOCKED)?;
        let same = |l: &ByteLock| l.owner == owner && l.offset == offset && l.length == length;
        let at = held
            .iter()
            .position(|l| same(l) && l.exclusive)
            .or_else(|| held.iter().position(same))
            .ok_or(STATUS_RANGE_NOT_LOCKED)?;
        held.remove(at);
        if held.is_empty() {
            self.files.remove(&file);
        }
        Ok(())
    }

    /// Drop every lock a file object holds (it closed). True if any went.
    pub fn release_file(&mut self, file: NodeKey, object: u64) -> bool {
        let Some(held) = self.files.get_mut(&file) else { return false };
        let before = held.len();
        held.retain(|l| l.owner.file != object);
        let changed = held.len() != before;
        if held.is_empty() {
            self.files.remove(&file);
        }
        changed
    }

    /// May `owner` read `[offset, offset + length)`?
    pub fn may_read(&self, file: NodeKey, owner: LockOwner, offset: u64, length: u64) -> bool {
        let Some(held) = self.files.get(&file) else { return true };
        !held.iter().any(|l| l.exclusive && l.owner != owner && l.overlaps(offset, length))
    }

    /// May `owner` write `[offset, offset + length)`?
    pub fn may_write(&self, file: NodeKey, owner: LockOwner, offset: u64, length: u64) -> bool {
        let Some(held) = self.files.get(&file) else { return true };
        !held.iter().any(|l| l.overlaps(offset, length) && !(l.exclusive && l.owner == owner))
    }

    /// The locks on a file (tests, debugging).
    pub fn locks(&self, file: NodeKey) -> &[ByteLock] {
        self.files.get(&file).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: NodeKey = NodeKey { dev: 1, ino: 2 };
    const A: LockOwner = LockOwner { file: 1, key: 0 };
    const B: LockOwner = LockOwner { file: 2, key: 0 };

    fn lk(owner: LockOwner, offset: u64, length: u64, exclusive: bool) -> ByteLock {
        ByteLock { owner, offset, length, exclusive }
    }

    #[test]
    fn exclusive_and_shared_overlap_rules() {
        let mut t = LockTable::default();
        t.lock(F, lk(A, 0, 10, true)).unwrap();
        assert_eq!(t.lock(F, lk(B, 5, 10, false)), Err(STATUS_LOCK_NOT_GRANTED));
        assert_eq!(t.lock(F, lk(B, 5, 10, true)), Err(STATUS_LOCK_NOT_GRANTED));
        // The same owner cannot overlap its own exclusive lock either.
        assert_eq!(t.lock(F, lk(A, 9, 1, true)), Err(STATUS_LOCK_NOT_GRANTED));
        t.lock(F, lk(B, 10, 10, true)).unwrap();
        // Shared locks overlap shared locks.
        t.lock(F, lk(A, 100, 10, false)).unwrap();
        t.lock(F, lk(B, 105, 10, false)).unwrap();
        assert_eq!(t.lock(F, lk(A, 108, 1, true)), Err(STATUS_LOCK_NOT_GRANTED));
        // Past the end of any file is fine.
        t.lock(F, lk(A, u64::MAX - 10, 10, true)).unwrap();
    }

    #[test]
    fn unlock_needs_the_exact_range_and_counts() {
        let mut t = LockTable::default();
        t.lock(F, lk(A, 0, 10, false)).unwrap();
        t.lock(F, lk(A, 0, 10, false)).unwrap();
        assert_eq!(t.unlock(F, A, 0, 5), Err(STATUS_RANGE_NOT_LOCKED));
        assert_eq!(t.unlock(F, B, 0, 10), Err(STATUS_RANGE_NOT_LOCKED));
        assert_eq!(t.unlock(F, LockOwner { file: 1, key: 9 }, 0, 10), Err(STATUS_RANGE_NOT_LOCKED));
        t.unlock(F, A, 0, 10).unwrap();
        assert_eq!(t.locks(F).len(), 1);
        t.unlock(F, A, 0, 10).unwrap();
        assert!(t.locks(F).is_empty());
        assert_eq!(t.unlock(F, A, 0, 10), Err(STATUS_RANGE_NOT_LOCKED));
    }

    #[test]
    fn reads_and_writes_against_locks() {
        let mut t = LockTable::default();
        t.lock(F, lk(A, 0, 10, true)).unwrap();
        t.lock(F, lk(A, 20, 10, false)).unwrap();
        // Exclusive: the owner reads and writes, others do neither.
        assert!(t.may_read(F, A, 0, 10));
        assert!(t.may_write(F, A, 0, 10));
        assert!(!t.may_read(F, B, 5, 1));
        assert!(!t.may_write(F, B, 9, 5));
        // Another key of the same file object is another owner.
        assert!(!t.may_read(F, LockOwner { file: 1, key: 3 }, 0, 1));
        // Shared: everyone reads, nobody writes, the owner included.
        assert!(t.may_read(F, B, 20, 10));
        assert!(!t.may_write(F, A, 25, 1));
        assert!(!t.may_write(F, B, 25, 1));
        // Outside the locks.
        assert!(t.may_write(F, B, 10, 10));
        assert!(t.may_read(F, B, 30, 100));
        // A zero-length transfer or lock conflicts with nothing.
        assert!(t.may_write(F, B, 5, 0));
        t.lock(F, lk(B, 5, 0, true)).unwrap();
        // Closing the file object drops its locks.
        assert!(t.release_file(F, 1));
        assert!(t.may_write(F, B, 0, 30));
    }

    #[test]
    fn ranges() {
        assert_eq!(check_range(0, 10), Ok((0, 10)));
        assert_eq!(check_range(0, u64::MAX), Ok((0, u64::MAX)));
        assert_eq!(check_range(1, u64::MAX), Err(STATUS_INVALID_LOCK_RANGE));
        assert_eq!(check_range(u64::MAX - 1, 1), Ok((u64::MAX - 1, 1)));
    }
}
