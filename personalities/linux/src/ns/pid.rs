//! PID namespaces (pid_namespaces(7)): the numbers a process and its
//! threads have in each namespace they belong to.
//!
//! The kernel and the process tree know every thread and process by one
//! *global* id (what the initial namespace sees). A process belongs to its
//! own pid namespace and to every ancestor of it, and has a number in each
//! ("a process ... has one process ID in each of the layers of the PID
//! namespace hierarchy"): [`attach`] gives a new thread or process the
//! next free number in every namespace from its own outwards, the first
//! process of a namespace getting 1 ("the first process created in a new
//! namespace ... has the PID 1, and is the "init" process"). System calls
//! translate at the boundary: ids they report are the caller's view
//! ([`local`], 0 for a process the caller's namespace cannot see, as
//! getppid(2) reports a parent outside the namespace), ids they are given
//! are looked up in it ([`global`], none: `ESRCH`).
//!
//! When the init of a namespace ends, the namespace is dead: no process
//! can be added any more (fork(2) "fails with ENOMEM") and the kernel ends
//! every other process in it with `SIGKILL` ([`init_ended`]).

use super::lock::Mutex;
use crate::errno::Errno;
use crate::task::tree::INIT_PID;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// pid_namespaces(7): "PID namespaces can be nested ... up to 32 levels"
/// (Poly's error past that: `ENOSPC`, as for too many namespaces).
pub const MAX_DEPTH: u32 = 32;
/// The largest number in a namespace (`/proc/sys/kernel/pid_max`).
pub const PID_MAX: i32 = 32768;

#[derive(Debug, Default)]
struct Table {
    /// Global id to the namespace's number, and back.
    to_local: BTreeMap<i32, i32>,
    to_global: BTreeMap<i32, i32>,
    /// The number the next thread or process tries first.
    next: i32,
    /// The global pid of the namespace's init, once there is one.
    init: Option<i32>,
    /// Its init ended.
    dead: bool,
}

/// One pid namespace other than the initial one.
#[derive(Debug)]
pub struct PidNs {
    inum: u32,
    parent: PidNsRef,
    /// 1 for a child of the initial namespace.
    level: u32,
    /// The user namespace that owns it (the creator's).
    owner: super::user::UserNsRef,
    table: Mutex<Table>,
}

impl PartialEq for PidNs {
    fn eq(&self, other: &PidNs) -> bool {
        core::ptr::eq(self, other)
    }
}

impl Eq for PidNs {}

/// A pid namespace; `None` is the initial one.
pub type PidNsRef = Option<Arc<PidNs>>;

impl PidNs {
    /// A child of `parent`, owned by user namespace `owner`. `ENOSPC` past
    /// [`MAX_DEPTH`].
    pub fn new(parent: &PidNsRef, owner: super::user::UserNsRef) -> Result<Arc<PidNs>, Errno> {
        let level = parent.as_ref().map_or(0, |p| p.level) + 1;
        if level > MAX_DEPTH {
            return Err(Errno::ENOSPC);
        }
        let table = Table { next: 1, ..Table::default() };
        Ok(Arc::new(PidNs { inum: super::new_inum(), parent: parent.clone(), level, owner, table: Mutex::new(table) }))
    }

    pub fn inum(&self) -> u32 {
        self.inum
    }

    pub fn parent(&self) -> &PidNsRef {
        &self.parent
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    pub fn owner(&self) -> &super::user::UserNsRef {
        &self.owner
    }

    /// The global pid of the namespace's init, while it has one.
    pub fn init(&self) -> Option<i32> {
        self.table.lock().init
    }

    /// Whether its init has ended.
    pub fn is_dead(&self) -> bool {
        self.table.lock().dead
    }

    /// The global ids of every thread and process in the namespace
    /// (descendant namespaces included).
    pub fn members(&self) -> Vec<i32> {
        self.table.lock().to_local.keys().copied().collect()
    }
}

/// Whether `a` and `b` are the same namespace.
pub fn same(a: &PidNsRef, b: &PidNsRef) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// Whether `ancestor` is `ns` or one of its ancestors.
pub fn is_ancestor(ancestor: &PidNsRef, ns: &PidNsRef) -> bool {
    let mut n = ns.clone();
    loop {
        if same(ancestor, &n) {
            return true;
        }
        match n {
            None => return false,
            Some(x) => n = x.parent.clone(),
        }
    }
}

/// The namespaces a member of `ns` belongs to, innermost first, without
/// the initial one.
fn chain(ns: &PidNsRef) -> Vec<Arc<PidNs>> {
    let mut out = Vec::new();
    let mut n = ns.clone();
    while let Some(x) = n {
        n = x.parent.clone();
        out.push(x);
    }
    out
}

/// Give thread or process `global` a number in `ns` and every ancestor;
/// `process` says whether it is a new process (the first one of a
/// namespace becomes its init). `ENOMEM` if a namespace is dead (its init
/// ended) or full; nothing changes then.
pub fn attach(ns: &PidNsRef, global: i32, process: bool) -> Result<(), Errno> {
    let namespaces = chain(ns);
    let mut done: Vec<&Arc<PidNs>> = Vec::new();
    for n in &namespaces {
        let mut t = n.table.lock();
        let free = if t.dead { None } else { free_number(&t) };
        let Some(local) = free else {
            drop(t);
            for d in done {
                forget(d, global);
            }
            return Err(Errno::ENOMEM);
        };
        t.to_local.insert(global, local);
        t.to_global.insert(local, global);
        t.next = if local >= PID_MAX { 2 } else { local + 1 };
        if process && local == INIT_PID && t.init.is_none() {
            t.init = Some(global);
        }
        drop(t);
        done.push(n);
    }
    Ok(())
}

/// The first free number from the table's next one, wrapping around past
/// [`PID_MAX`] (to 2: 1 is the init's, never reused).
fn free_number(t: &Table) -> Option<i32> {
    let start = t.next.clamp(1, PID_MAX);
    (start..=PID_MAX).chain(2..start).find(|n| !t.to_global.contains_key(n))
}

fn forget(n: &PidNs, global: i32) {
    let mut t = n.table.lock();
    if let Some(local) = t.to_local.remove(&global) {
        t.to_global.remove(&local);
    }
}

/// Thread or process `global` is gone (a thread ended, a process was
/// reaped): its numbers are free again.
pub fn detach(ns: &PidNsRef, global: i32) {
    for n in chain(ns) {
        forget(&n, global);
    }
}

/// `global` as `ns` sees it; `None` if it is not in `ns`.
pub fn local(ns: &PidNsRef, global: i32) -> Option<i32> {
    match ns {
        None => Some(global),
        Some(n) => n.table.lock().to_local.get(&global).copied(),
    }
}

/// [`local`], with 0 for an id `ns` cannot see (getppid(2) of a process
/// whose parent is outside its namespace, `si_pid` of a sender outside).
pub fn local_or_zero(ns: &PidNsRef, global: i32) -> i32 {
    if global <= 0 {
        return global;
    }
    local(ns, global).unwrap_or(0)
}

/// The global id of `ns`'s number `local`; `None` if there is none.
pub fn global(ns: &PidNsRef, local: i32) -> Option<i32> {
    match ns {
        None => Some(local),
        Some(n) => n.table.lock().to_global.get(&local).copied(),
    }
}

/// Global id `global` as the calling process sees it (0 if it cannot):
/// what every id a system call reports goes through.
pub fn view(k: &dyn crate::kernel::LinuxKernel, global: i32) -> i32 {
    match k.pid_ns() {
        None => global,
        ns => local_or_zero(&ns, global),
    }
}

/// The global id of the calling process's number `local` (`ESRCH` if its
/// namespace has none): what every id a system call is given goes
/// through.
pub fn find(k: &dyn crate::kernel::LinuxKernel, local: i32) -> Result<i32, Errno> {
    match k.pid_ns() {
        None => Ok(local),
        ns => global(&ns, local).ok_or(Errno::ESRCH),
    }
}

/// [`find`] for an id that may also name nothing (a process group, a wait
/// selector): an id the namespace does not have becomes 0, which no
/// process has.
pub fn find_or_none(k: &dyn crate::kernel::LinuxKernel, local: i32) -> i32 {
    find(k, local).unwrap_or(0)
}

/// The reaper of a new process `global` in `ns` (attached already): the
/// namespace's init, or for that init itself the reaper of the parent
/// namespace ([`crate::task::tree::ProcessTree::set_reaper`]).
pub fn reaper_of_new(ns: &PidNsRef, global: i32) -> i32 {
    match ns {
        Some(n) if n.init() == Some(global) => reaper(&n.parent),
        _ => reaper(ns),
    }
}

/// The process that adopts the orphans of `ns` (its init; the initial
/// namespace's is [`INIT_PID`]), or of the nearest living ancestor's when
/// its init has ended.
pub fn reaper(ns: &PidNsRef) -> i32 {
    let mut n = ns.clone();
    while let Some(x) = n {
        let t = x.table.lock();
        if let (Some(init), false) = (t.init, t.dead) {
            return init;
        }
        drop(t);
        n = x.parent.clone();
    }
    INIT_PID
}

/// Process `global` ended. If it was the init of `ns`, the namespace is
/// dead and the global ids of every other member are returned, for the
/// kernel to end with `SIGKILL` (pid_namespaces(7): "If the "init"
/// process of a PID namespace terminates, the kernel terminates all of the
/// processes in the namespace via a SIGKILL signal").
pub fn init_ended(ns: &PidNsRef, global: i32) -> Vec<i32> {
    let Some(n) = ns else { return Vec::new() };
    let mut t = n.table.lock();
    if t.init != Some(global) || t.dead {
        return Vec::new();
    }
    t.dead = true;
    t.to_local.keys().copied().filter(|&g| g != global).collect()
}
