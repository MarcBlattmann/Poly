//! Advisory file locks (fcntl(2) record locks, open file description locks,
//! flock(2); ADR 0057): one `poly_vfs::locks::LockTable` for every file
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
use poly_vfs::locks::{FileKey, LockKind, LockOwner, LockTable, RecordLock};

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

/// flock(2) on `file`: `Some(exclusive)` locks, `None` unlocks. `Err(())`
/// when another description's lock is in the way.
pub fn flock(file: &Arc<OpenFile>, op: Option<bool>) -> Result<(), ()> {
    purge();
    register(file);
    let owner = description(file);
    let result = match op {
        Some(exclusive) => TABLE.lock().flock(key(file), owner, exclusive),
        None => {
            TABLE.lock().unflock(key(file), owner);
            Ok(())
        }
    };
    // Unlocking, and the old lock a conversion gave up.
    changed();
    result
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
