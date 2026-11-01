//! Advisory file locks (fcntl(2) record locks, open file description locks,
//! flock(2); ADR 0059): one `poly_vfs::locks::LockTable` for every file
//! of every file system, keyed by the file system's device and the inode.
//!
//! The table decides; this keeps it, tells waiters when locks go (a
//! generation counter and the process event queue, which signals and
//! exits wake too, so a waiting thread can be interrupted) and drops the
//! locks of owners that are gone: a process's record locks on a file when
//! it closes any descriptor of the file (fcntl(2): "If a process closes
//! any file descriptor referring to a file, then all of the process's
//! locks on that file are released"), and an open file description's locks
//! once the description is released, which the descriptor table's closes
//! notice ([`closed`]).

use crate::process;
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_vfs::OpenFile;
use poly_vfs::locks::{FileKey, LockKind, LockOwner, LockTable, RecordLock, WouldBlock};

static TABLE: SpinLock<LockTable> = SpinLock::new(LockTable::new());
/// The open file descriptions that hold (or held) a lock, by owner id.
static DESCRIPTIONS: SpinLock<BTreeMap<usize, Weak<OpenFile>>> = SpinLock::new(BTreeMap::new());
/// Moves whenever a lock goes.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Whether a lock was ever taken (closes skip all of this until then).
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The table's name for the file `file` is open on.
pub fn key(file: &OpenFile) -> FileKey {
    let node = file.node();
    let dev = node.mount().dev();
    ((u64::from(dev.major) << 32) | u64::from(dev.minor), node.ino())
}

/// The owner id of an open file description.
pub fn description(file: &Arc<OpenFile>) -> LockOwner {
    LockOwner::Description(Arc::as_ptr(file) as usize)
}

/// The generation now: wait until it moves.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

fn changed() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    process::wake_waiters();
}

/// Drop the locks of descriptions that were released.
fn purge() {
    let dead: Vec<usize> = {
        let mut d = DESCRIPTIONS.lock();
        let dead: Vec<usize> = d.iter().filter(|(_, w)| w.strong_count() == 0).map(|(&id, _)| id).collect();
        for id in &dead {
            d.remove(id);
        }
        dead
    };
    if dead.is_empty() {
        return;
    }
    let released = TABLE.lock().retain_owners(|o| !matches!(o, LockOwner::Description(id) if dead.contains(&id)));
    if released {
        changed();
    }
}

fn register(file: &Arc<OpenFile>) {
    ACTIVE.store(true, Ordering::SeqCst);
    DESCRIPTIONS.lock().insert(Arc::as_ptr(file) as usize, Arc::downgrade(file));
}

/// `F_GETLK`: a lock of another owner in the way of `owner`'s `kind` lock
/// on `[start, end)` of `file`.
pub fn test(owner: LockOwner, file: &OpenFile, kind: LockKind, start: u64, end: u64) -> Option<RecordLock> {
    purge();
    TABLE.lock().test(key(file), owner, kind, start, end)
}

/// `F_SETLK`: lock (`Some(kind)`) or unlock `[start, end)` of `file` for
/// `owner` once; `Err` with the lock in the way.
pub fn set(
    owner: LockOwner,
    pid: i32,
    file: &Arc<OpenFile>,
    kind: Option<LockKind>,
    start: u64,
    end: u64,
) -> Result<(), RecordLock> {
    purge();
    if matches!(owner, LockOwner::Description(_)) {
        register(file);
    } else {
        ACTIVE.store(true, Ordering::SeqCst);
    }
    let result = TABLE.lock().set(key(file), owner, pid, kind, start, end);
    if result.is_ok() && kind != Some(LockKind::Write) {
        // An unlock, or a write lock turned into a read lock: waiters may
        // get theirs now.
        changed();
    }
    result
}

/// flock(2) on `file`: `Some(exclusive)` locks, `None` unlocks. `Err`
/// when another description's lock is in the way; with `wait` the request
/// then waits in the lock table, which gives it the lock as soon as the
/// conflicting ones go ([`flock_granted`], [`flock_cancel`]).
pub fn flock(file: &Arc<OpenFile>, op: Option<bool>, wait: bool) -> Result<(), WouldBlock> {
    purge();
    register(file);
    let owner = description(file);
    let (held, result) = {
        let mut table = TABLE.lock();
        let held = table.flock_of(key(file), owner);
        let result = match op {
            Some(exclusive) if wait => table.flock_wait(key(file), owner, exclusive),
            Some(exclusive) => table.flock(key(file), owner, exclusive),
            None => {
                table.unflock(key(file), owner);
                Ok(())
            }
        };
        (held, result)
    };
    // Waiters look again only when a lock went: an unlock, a lock a
    // conversion gave up (also one that then failed), an exclusive lock
    // turned shared. A waiter's own failed attempt, holding nothing, wakes
    // nobody: otherwise two waiters (or one and its own attempt) would
    // wake each other for ever and keep the holder from running.
    let released = match (held, op, &result) {
        (None, _, _) => false,
        (Some(_), None, _) | (Some(_), Some(_), Err(_)) => true,
        (Some(was_exclusive), Some(exclusive), Ok(())) => was_exclusive && !exclusive,
    };
    if released {
        changed();
    }
    result
}

/// Whether `file`'s waiting flock(2) request got its lock.
pub fn flock_granted(file: &Arc<OpenFile>) -> bool {
    purge();
    TABLE.lock().flock_granted(key(file), description(file))
}

/// `file`'s waiting flock(2) request gives up (a signal).
pub fn flock_cancel(file: &Arc<OpenFile>) {
    TABLE.lock().flock_cancel(key(file), description(file));
}

/// The keys of `files`, for [`closed`] (nothing before any lock was taken).
pub fn keys_of<'a>(files: impl IntoIterator<Item = &'a Arc<OpenFile>>) -> Vec<FileKey> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return Vec::new();
    }
    files.into_iter().map(|f| key(f)).collect()
}

/// The descriptor table `table` (an id: the process's owner id) closed
/// descriptors of the files `keys`, and has dropped them: its record locks
/// on them go, and so do the locks of descriptions that are now released.
pub fn closed(table: usize, keys: &[FileKey]) {
    if keys.is_empty() {
        return;
    }
    let mut released = false;
    {
        let mut t = TABLE.lock();
        for &k in keys {
            released |= t.release(k, LockOwner::Process(table));
        }
    }
    if released {
        changed();
    }
    purge();
}

crate::kernel_test! {
    fn locks_go_with_a_close_and_with_the_last_description() -> TestResult {
        use super::Files;
        use poly_vfs::OpenFlags;
        let path = b"/tmp/locks-test";
        let (a, b) = (Files::new(), Files::new());
        let fa = a.open(path, OpenFlags::RDWR | OpenFlags::CREAT, 0o644).map_err(|_| "open failed")?;
        let fb = b.open(path, OpenFlags::RDWR, 0).map_err(|_| "open failed")?;
        let (file_a, file_b) = (a.file(fa).map_err(|_| "no file")?, b.file(fb).map_err(|_| "no file")?);
        let (pa, pb) = (LockOwner::Process(a.lock_owner()), LockOwner::Process(b.lock_owner()));
        set(pa, 1, &file_a, Some(LockKind::Write), 0, 10).map_err(|_| "the first lock was refused")?;
        if set(pb, 2, &file_b, Some(LockKind::Read), 5, 6).is_ok() {
            return Err("two processes hold conflicting locks");
        }
        if test(pb, &file_b, LockKind::Read, 0, u64::MAX).map(|l| (l.pid, l.start, l.end)) != Some((1, 0, 10)) {
            return Err("F_GETLK does not report the lock in the way");
        }
        // Any close of the file by the owner lets go of its locks, even of a
        // descriptor the lock was not taken through.
        let again = a.open(path, OpenFlags::RDONLY, 0).map_err(|_| "open failed")?;
        a.close(again).map_err(|_| "close failed")?;
        set(pb, 2, &file_b, Some(LockKind::Read), 5, 6).map_err(|_| "a close did not release the lock")?;
        // flock: the description's lock goes with the description.
        let file_key = key(&file_b);
        drop(file_a);
        flock(&file_b, Some(true), false).map_err(|WouldBlock| "flock was refused")?;
        drop(file_b);
        let fa2 = a.open(path, OpenFlags::RDONLY, 0).map_err(|_| "open failed")?;
        let file_a2 = a.file(fa2).map_err(|_| "no file")?;
        if flock(&file_a2, Some(false), false).is_ok() {
            return Err("flock ignored another description's lock");
        }
        b.close_all();
        flock(&file_a2, Some(false), false).map_err(|WouldBlock| "the closed description kept its flock")?;
        drop(file_a2);
        a.close_all();
        let _ = Files::new().unlinkat(poly_vfs::AT_FDCWD, path, 0);
        if TABLE.lock().records_of(file_key, pb).is_empty() { Ok(()) } else { Err("locks outlived their owner") }
    }
}
