//! Advisory locks: record locks (split, merge, convert, conflicts between
//! owners and kinds), open file description owners, flock(2) locks, and
//! owners going away.

use crate::locks::*;
use alloc::vec;
use alloc::vec::Vec;

const F: FileKey = (7, 42);
const G: FileKey = (7, 43);
const A: LockOwner = LockOwner::Process(1);
const B: LockOwner = LockOwner::Process(2);
const D: LockOwner = LockOwner::Description(0x1000);
const END: u64 = u64::MAX;

fn spans(t: &LockTable, owner: LockOwner) -> Vec<(LockKind, u64, u64)> {
    t.records_of(F, owner).iter().map(|l| (l.kind, l.start, l.end)).collect()
}

#[test]
fn locks_of_one_owner_split_merge_and_convert() {
    use LockKind::*;
    let mut t = LockTable::new();
    t.set(F, A, 10, Some(Write), 0, 100).unwrap();
    // A read lock in the middle converts that part.
    t.set(F, A, 10, Some(Read), 40, 60).unwrap();
    assert_eq!(spans(&t, A), vec![(Write, 0, 40), (Read, 40, 60), (Write, 60, 100)]);
    // Unlocking across a boundary.
    t.set(F, A, 10, None, 50, 70).unwrap();
    assert_eq!(spans(&t, A), vec![(Write, 0, 40), (Read, 40, 50), (Write, 70, 100)]);
    // Write again over the gap: neighbours of the same kind merge.
    t.set(F, A, 10, Some(Write), 40, 70).unwrap();
    assert_eq!(spans(&t, A), vec![(Write, 0, 100)]);
    // To the end of the file, then all of it unlocked.
    t.set(F, A, 10, Some(Read), 100, END).unwrap();
    assert_eq!(spans(&t, A), vec![(Write, 0, 100), (Read, 100, END)]);
    t.set(F, A, 10, None, 0, END).unwrap();
    assert!(t.is_empty());
}

#[test]
fn owners_conflict_on_overlapping_ranges_with_a_writer() {
    use LockKind::*;
    let mut t = LockTable::new();
    t.set(F, A, 10, Some(Read), 0, 10).unwrap();
    t.set(F, B, 20, Some(Read), 5, 15).unwrap();
    let conflict = t.set(F, B, 20, Some(Write), 5, 15).unwrap_err();
    assert_eq!((conflict.owner, conflict.pid, conflict.start, conflict.end), (A, 10, 0, 10));
    assert_eq!(spans(&t, B), vec![(Read, 5, 15)], "a refused lock changes nothing");
    assert_eq!(t.test(F, B, Write, 10, 20), None, "past A's range");
    assert_eq!(t.test(F, B, Write, 9, 20).map(|l| l.owner), Some(A));
    assert_eq!(t.test(G, B, Write, 0, END), None, "another file");
    // An open file description lock conflicts with a process's lock.
    assert!(t.set(F, D, -1, Some(Write), 12, 13).is_err());
    t.set(F, D, -1, Some(Write), 15, 16).unwrap();
    assert_eq!(t.test(F, A, Read, 0, END).map(|l| (l.owner, l.pid)), Some((D, -1)), "B only reads");
    assert_eq!(t.test(F, A, Write, 0, END).map(|l| (l.owner, l.pid)), Some((B, 20)));
    assert_eq!(t.descriptions(), vec![0x1000]);
    // A process closing a descriptor of the file loses its locks there.
    assert!(t.release(F, B));
    assert!(!t.release(F, B));
    assert_eq!(t.test(F, A, Write, 0, END).map(|l| l.owner), Some(D));
    // Descriptions that are gone lose theirs.
    assert!(t.retain_owners(|o| o != D));
    assert_eq!(t.test(F, B, Write, 0, END).map(|l| l.owner), Some(A));
}

#[test]
fn flock_locks_are_whole_file_and_separate_from_record_locks() {
    let (d1, d2) = (LockOwner::Description(1), LockOwner::Description(2));
    let mut t = LockTable::new();
    t.set(F, A, 10, Some(LockKind::Write), 0, END).unwrap();
    t.flock(F, d1, true).unwrap();
    assert!(t.flock_would_block(F, d2, false));
    assert_eq!(t.flock(F, d2, false), Err(WouldBlock));
    t.flock(F, d1, false).unwrap(); // converted to shared
    t.flock(F, d2, false).unwrap();
    assert!(t.flock_would_block(F, d1, true), "another shared lock");
    t.unflock(F, d2);
    t.flock(F, d1, true).unwrap();
    assert!(t.release(F, d1));
    assert!(!t.flock_would_block(F, d2, true));
    assert_eq!(spans(&t, A).len(), 1, "the record lock stayed");
}

#[test]
fn waiting_flock_requests_get_the_lock_when_it_goes() {
    let (d1, d2, d3) = (LockOwner::Description(1), LockOwner::Description(2), LockOwner::Description(3));
    let mut t = LockTable::new();
    t.flock(F, d1, true).unwrap();
    // Two waiters: the first wants it exclusive, the second shared.
    assert_eq!(t.flock_wait(F, d2, true), Err(WouldBlock));
    assert_eq!(t.flock_wait(F, d3, false), Err(WouldBlock));
    assert!(!t.flock_granted(F, d2));
    t.unflock(F, d1);
    assert!(t.flock_granted(F, d2), "the first waiter got it");
    assert!(!t.flock_granted(F, d3), "the second still waits");
    // A closed holder (a released description) hands it on too.
    assert!(t.retain_owners(|o| o != d2));
    assert!(t.flock_granted(F, d3));
    t.unflock(F, d3);
    assert!(t.is_empty());
}

#[test]
fn a_conversion_lets_a_waiter_in_first() {
    // The spec of flock: "a conversion to LOCK_EX that fails with EAGAIN
    // leaves the description with no lock at all".
    let (d1, d2) = (LockOwner::Description(1), LockOwner::Description(2));
    let mut t = LockTable::new();
    t.flock(F, d1, false).unwrap();
    t.flock(F, d2, false).unwrap();
    // d1 waits to convert to exclusive (its shared lock is gone).
    assert_eq!(t.flock_wait(F, d1, true), Err(WouldBlock));
    assert_eq!(t.flock_of(F, d1), None);
    // d2 converts too: its shared lock goes first, d1 gets the file.
    assert_eq!(t.flock(F, d2, true), Err(WouldBlock));
    assert!(t.flock_granted(F, d1));
    assert_eq!(t.flock_of(F, d2), None);
    // A waiter that gives up leaves nothing behind.
    assert_eq!(t.flock_wait(F, d2, false), Err(WouldBlock));
    t.flock_cancel(F, d2);
    t.unflock(F, d1);
    assert!(t.is_empty());
}
