//! The NT processes, each behind a lock of its own (ADR 0262).
//!
//! The NT state's lock (`STATE`) guards the system: objects, handles, the dispatcher, sections.
//! Each process's state ([`NtProcess`]: its memory model, threads, mitigations) sits behind its
//! own [`Mutex`], so a process's memory calls on its own private memory take that lock alone
//! and wait for no other process ([`own`]).
//!
//! The lock order is the NT state, then a process, never the reverse: code that holds the NT
//! state locks a process through the [`ProcTable`]; code that holds only a process's lock (the
//! own-memory paths) never takes the NT state's lock, nor another process's. A thread holding two
//! processes at once (a parent and its child) holds the NT state too, which keeps those apart.

use crate::sync::{Mutex, MutexGuard};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use poly_ntobj::ProcessId;
use poly_personality_nt::system::NtProcess;

/// One process's state and its lock.
pub type ProcCell = Arc<Mutex<NtProcess>>;

/// Every process's cell, for the own-memory paths that do not take the NT state's lock: kept
/// equal to the [`ProcTable`]'s (changed only by it, under the NT state's lock). A leaf lock:
/// nothing else is taken while it is held.
static CELLS: Mutex<BTreeMap<ProcessId, ProcCell>> = Mutex::new(BTreeMap::new());

/// Process `pid`'s cell, without the NT state's lock (`None`: not an NT process, or it ended).
pub fn own(pid: ProcessId) -> Option<ProcCell> {
    CELLS.lock().get(&pid).cloned()
}

/// The NT state's table of processes: each one locked while a guard [`get`](Self::get) or
/// [`get_mut`](Self::get_mut) gave is alive.
#[derive(Default)]
pub struct ProcTable {
    map: BTreeMap<ProcessId, ProcCell>,
}

impl ProcTable {
    /// Process `pid`, locked.
    pub fn get(&self, pid: &ProcessId) -> Option<MutexGuard<'_, NtProcess>> {
        self.map.get(pid).map(|c| c.lock())
    }

    /// Process `pid`, locked; `&mut self` keeps a second guard from the table out of the same
    /// scope (locking a process twice would never return).
    pub fn get_mut(&mut self, pid: &ProcessId) -> Option<MutexGuard<'_, NtProcess>> {
        self.map.get(pid).map(|c| c.lock())
    }

    pub fn contains_key(&self, pid: &ProcessId) -> bool {
        self.map.contains_key(pid)
    }

    pub fn keys(&self) -> impl Iterator<Item = &ProcessId> {
        self.map.keys()
    }

    /// Every process in turn, each locked while its item is alive.
    pub fn iter(&self) -> impl Iterator<Item = (&ProcessId, MutexGuard<'_, NtProcess>)> {
        self.map.iter().map(|(pid, c)| (pid, c.lock()))
    }

    /// Every process in turn, locked (see [`iter`](Self::iter)).
    pub fn values(&self) -> impl Iterator<Item = MutexGuard<'_, NtProcess>> {
        self.map.values().map(|c| c.lock())
    }

    /// Add `proc` as process `pid` (`pid` is not in the table).
    pub fn insert(&mut self, pid: ProcessId, proc: NtProcess) {
        let cell = Arc::new(Mutex::new(proc));
        CELLS.lock().insert(pid, Arc::clone(&cell));
        self.map.insert(pid, cell);
    }

    /// Take process `pid` out of the table. Its state is moved out under its lock; a thread that
    /// took the cell before ([`own`]) and locks it after finds an empty process in its place.
    pub fn remove(&mut self, pid: &ProcessId) -> Option<NtProcess> {
        let cell = self.map.remove(pid)?;
        CELLS.lock().remove(pid);
        let mut proc = cell.lock();
        Some(core::mem::replace(&mut *proc, NtProcess::new(*pid)))
    }
}
