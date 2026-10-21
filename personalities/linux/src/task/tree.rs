//! The process tree: every Linux process's parent, children, process group
//! and session, and the states a parent waits for with wait(2).
//!
//! The kernel keeps one [`ProcessTree`] for the system behind a lock and
//! lends it out ([`crate::kernel::LinuxKernel::with_tree`]), like the futex
//! table. It is plain data; signalling the parent and blocking are the
//! callers' ([`super::notify`], [`crate::sys::task`]).
//!
//! - **Parents.** A process the kernel starts (the first program, the
//!   shell's `run`) has parent 0, "the kernel": nobody waits for it in
//!   user space, so it is reaped as soon as it ends. A child of fork(2) or
//!   clone(2) has the caller as parent (or the caller's parent with
//!   `CLONE_PARENT`).
//! - **Zombies and reaping.** A process that ends stays as a zombie until
//!   its parent waits for it (wait(2): "A child that terminates, but has
//!   not been waited for becomes a "zombie""), unless the parent set
//!   `SIGCHLD` to `SIG_IGN` or `SA_NOCLDWAIT` (then "children that
//!   terminate do not become zombies"), or its parent is the kernel.
//!   Reaped pids are collected for the kernel ([`ProcessTree::take_reaped`]),
//!   which then lets go of the process record.
//! - **Orphans.** When a process ends, its children are adopted by init
//!   (pid 1) if init is alive, else by the kernel (and zombies among them
//!   are reaped at once). A child in another pid namespace goes to that
//!   namespace's init instead (pid_namespaces(7): orphans "are reparented
//!   to the init process of their PID namespace"), and an init whose
//!   parent ends to the reaper of its parent's namespace: each process
//!   has a reaper ([`ProcessTree::set_reaper`]), and when that one has
//!   ended too, its own reaper counts (ADR 0054). A "child subreaper"
//!   (prctl(2) `PR_SET_CHILD_SUBREAPER`) takes init's place for the
//!   orphans among its descendants in its namespace: the nearest one still
//!   living adopts them ([`ProcessTree::set_subreaper`]).
//! - **Process groups and sessions.** A child starts in its creator's
//!   group and session (fork(2)); setpgid(2) and setsid(2) change them
//!   by the man pages' rules.
//! - **Waiting.** Each state change a parent may wait for (an exit, a
//!   stop, a continue) bumps a generation counter, so a waiter can check,
//!   unlock, and block until the generation moves without missing one.

use crate::errno::Errno;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

/// The init process: orphans' adoptive parent.
pub const INIT_PID: i32 = 1;

/// How a process ended, as its parent learns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Termination {
    /// `_exit(status)`: the low 8 bits.
    Exited(u8),
    /// Killed by `signal`; `core` if a core dump was written.
    Killed { signal: u32, core: bool },
}

/// What a parent can wait for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildEvent {
    /// The child ended (it is a zombie until reaped).
    Ended(Termination),
    /// The child was stopped by this signal (`WUNTRACED`).
    Stopped(u32),
    /// The stopped child was continued (`WCONTINUED`).
    Continued,
}

/// A stop or continue not yet reported to the parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Report {
    Stopped(u32),
    Continued,
}

#[derive(Clone, Debug)]
struct Node {
    /// 0: the kernel.
    parent: i32,
    pgid: i32,
    sid: i32,
    /// The real user id, for `si_uid` of the parent's `SIGCHLD`.
    uid: u32,
    /// The signal the parent gets when this process ends (0: none).
    exit_signal: u32,
    children: BTreeSet<i32>,
    zombie: Option<Termination>,
    /// A zombie whose parent has not been sent its exit signal yet
    /// ([`ProcessTree::exit_announced_later`]): not waitable until then.
    unannounced: bool,
    report: Option<Report>,
    /// It has called execve(2) since it was created (setpgid(2): `EACCES`).
    execed: bool,
    /// Who adopts it when its parent ends (the init of its pid namespace;
    /// [`INIT_PID`] by default).
    reaper: i32,
    /// It is a "child subreaper" (prctl(2) `PR_SET_CHILD_SUBREAPER`): it
    /// adopts the orphans among its descendants.
    subreaper: bool,
}

/// Which children a wait looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    /// Any child (`pid == -1`, `P_ALL`).
    Any,
    /// The child with this pid.
    Pid(i32),
    /// Any child in this process group.
    Group(i32),
}

/// Which children count by how they signal their parent (wait(2):
/// `__WCLONE`, `__WALL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clones {
    /// Only children that send `SIGCHLD` when they end (the default).
    Exclude,
    /// Only "clone" children: another signal, or none (`__WCLONE`).
    Only,
    /// All children (`__WALL`).
    Include,
}

/// What a wait looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitFilter {
    pub selector: Selector,
    /// Ended children (`WEXITED`; always for wait4).
    pub ended: bool,
    /// Stopped children (`WUNTRACED` / `WSTOPPED`).
    pub stopped: bool,
    /// Continued children (`WCONTINUED`).
    pub continued: bool,
    /// Take the event (reap an ended child); false with `WNOWAIT`.
    pub consume: bool,
    pub clones: Clones,
}

/// A child's event a wait found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waited {
    pub pid: i32,
    pub uid: u32,
    pub event: ChildEvent,
}

/// The result of [`ProcessTree::poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    /// A child had an event.
    Ready(Waited),
    /// There are children the wait looks at, but none had an event.
    Pending,
    /// None had an event yet, but one has ended and its exit signal is on
    /// its way to the parent ([`ProcessTree::announce`] comes next): even
    /// a `WNOHANG` wait waits for that.
    Exiting,
    /// No child matches (`ECHILD`).
    NoChildren,
}

/// What [`ProcessTree::exit`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitReport {
    /// The parent (0: the kernel) and the signal it is to get (0: none).
    pub parent: i32,
    pub exit_signal: u32,
    /// The process stays as a zombie for its parent to reap.
    pub zombie: bool,
    /// Who adopted its children whose reaper is init (0: the kernel).
    pub adopter: i32,
    /// Children that had already ended and that their adopter now has to
    /// reap: `(pid, uid, how it ended, its exit signal, the adopter)`.
    pub adopted_zombies: Vec<(i32, u32, Termination, u32, i32)>,
}

/// See the module docs.
#[derive(Clone, Debug, Default)]
pub struct ProcessTree {
    nodes: BTreeMap<i32, Node>,
    generation: u64,
    reaped: Vec<i32>,
}

impl ProcessTree {
    pub const fn new() -> ProcessTree {
        ProcessTree { nodes: BTreeMap::new(), generation: 0, reaped: Vec::new() }
    }

    /// Changes a waiter may care about so far.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// A process the kernel started (parent 0), the leader of its own
    /// process group and session.
    pub fn insert_root(&mut self, pid: i32, uid: u32) {
        self.nodes.insert(pid, Node::new(0, pid, pid, uid, 0));
    }

    /// A process `pid` made by `creator` (fork(2), clone(2)) whose parent
    /// is `parent` (`creator`, or its parent with `CLONE_PARENT`). It
    /// starts in the creator's process group and session. A creator the
    /// tree does not know gets the child as a group and session of its own.
    pub fn insert_child(&mut self, creator: i32, parent: i32, pid: i32, exit_signal: u32, uid: u32) {
        let (pgid, sid) = self.nodes.get(&creator).map_or((pid, pid), |c| (c.pgid, c.sid));
        let parent = if self.nodes.get(&parent).is_some_and(|p| p.zombie.is_none()) { parent } else { 0 };
        self.nodes.insert(pid, Node::new(parent, pgid, sid, uid, exit_signal));
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.insert(pid);
        }
    }

    /// Forget `pid` without a trace (a clone that failed after it was
    /// inserted).
    pub fn remove(&mut self, pid: i32) {
        if let Some(node) = self.nodes.remove(&pid)
            && let Some(p) = self.nodes.get_mut(&node.parent)
        {
            p.children.remove(&pid);
        }
    }

    pub fn contains(&self, pid: i32) -> bool {
        self.nodes.contains_key(&pid)
    }

    /// The parent of `pid` (0: the kernel).
    pub fn parent(&self, pid: i32) -> Option<i32> {
        self.nodes.get(&pid).map(|n| n.parent)
    }

    /// The process group of `pid`.
    pub fn pgid(&self, pid: i32) -> Option<i32> {
        self.nodes.get(&pid).map(|n| n.pgid)
    }

    /// The session of `pid`.
    pub fn sid(&self, pid: i32) -> Option<i32> {
        self.nodes.get(&pid).map(|n| n.sid)
    }

    /// The real user id `pid` had when it was created.
    pub fn uid(&self, pid: i32) -> Option<u32> {
        self.nodes.get(&pid).map(|n| n.uid)
    }

    /// Whether `pid` has ended and waits to be reaped.
    pub fn is_zombie(&self, pid: i32) -> bool {
        self.nodes.get(&pid).is_some_and(|n| n.zombie.is_some())
    }

    /// The children of `pid`, live and zombie.
    pub fn children(&self, pid: i32) -> Vec<i32> {
        self.nodes.get(&pid).map(|n| n.children.iter().copied().collect()).unwrap_or_default()
    }

    /// The members of process group `pgid`, zombies included.
    pub fn group(&self, pgid: i32) -> Vec<i32> {
        self.nodes.iter().filter(|(_, n)| n.pgid == pgid).map(|(&pid, _)| pid).collect()
    }

    /// The pids reaped since the last call: their records can go.
    pub fn take_reaped(&mut self) -> Vec<i32> {
        core::mem::take(&mut self.reaped)
    }

    /// Process `pid` is adopted by `reaper` when its parent ends (and by
    /// `reaper`'s own reaper if that has ended too).
    pub fn set_reaper(&mut self, pid: i32, reaper: i32) {
        if let Some(n) = self.nodes.get_mut(&pid) {
            n.reaper = reaper;
        }
    }

    /// Mark `pid` as a child subreaper or not (prctl(2)
    /// `PR_SET_CHILD_SUBREAPER`: "not inherited by children created by
    /// fork(2) and clone(2)", "preserved across execve(2)").
    pub fn set_subreaper(&mut self, pid: i32, on: bool) {
        if let Some(n) = self.nodes.get_mut(&pid) {
            n.subreaper = on;
        }
    }

    /// Whether `pid` is a child subreaper (`PR_GET_CHILD_SUBREAPER`).
    pub fn is_subreaper(&self, pid: i32) -> bool {
        self.nodes.get(&pid).is_some_and(|n| n.subreaper)
    }

    /// The nearest still living ancestor of `exiting` that is a child
    /// subreaper, below `child`'s reaper (the init of its pid namespace,
    /// which a subreaper outside the namespace cannot take the place of):
    /// prctl(2), "reparented to the nearest still living ancestor
    /// subreaper".
    fn subreaper_for(&self, child: i32, exiting: i32) -> Option<i32> {
        let reaper = self.nodes.get(&child).map_or(INIT_PID, |n| n.reaper);
        let mut a = self.nodes.get(&exiting)?.parent;
        for _ in 0..=self.nodes.len() {
            if a <= 0 || a == reaper || a == INIT_PID {
                return None;
            }
            let n = self.nodes.get(&a)?;
            if n.subreaper && n.zombie.is_none() {
                return Some(a);
            }
            a = n.parent;
        }
        None
    }

    /// `pid` called execve(2).
    pub fn exec(&mut self, pid: i32) {
        if let Some(n) = self.nodes.get_mut(&pid) {
            n.execed = true;
        }
    }

    /// Who adopts the children of `exiting`: init while it lives (and is
    /// not the one exiting), else the kernel.
    fn adopter(&self, exiting: i32) -> i32 {
        match self.nodes.get(&INIT_PID) {
            Some(init) if exiting != INIT_PID && init.zombie.is_none() => INIT_PID,
            _ => 0,
        }
    }

    /// Who adopts `child` of `exiting`: its reaper if alive (and not the
    /// one exiting), else that one's reaper, and so on; init's, if it is
    /// gone, the kernel.
    fn adopter_of(&self, child: i32, exiting: i32) -> i32 {
        let mut r = self.nodes.get(&child).map_or(INIT_PID, |n| n.reaper);
        // Reapers form a chain towards init; bounded anyway.
        for _ in 0..=self.nodes.len() {
            if r == INIT_PID {
                return self.adopter(exiting);
            }
            match self.nodes.get(&r) {
                Some(n) if r != exiting && n.zombie.is_none() => return r,
                Some(n) if n.reaper != r => r = n.reaper,
                _ => r = INIT_PID,
            }
        }
        self.adopter(exiting)
    }

    fn reap(&mut self, pid: i32) {
        if let Some(node) = self.nodes.remove(&pid) {
            if let Some(p) = self.nodes.get_mut(&node.parent) {
                p.children.remove(&pid);
            }
            self.reaped.push(pid);
        }
    }

    /// Process `pid` has ended with `how`. Its children go to init (or the
    /// kernel); it becomes a zombie unless its parent is the kernel or
    /// `parent_ignores` (the parent's `SIGCHLD` is `SIG_IGN` or has
    /// `SA_NOCLDWAIT`), in which case it is reaped now.
    pub fn exit(&mut self, pid: i32, how: Termination, parent_ignores: bool) -> ExitReport {
        self.exit_with(pid, how, parent_ignores, false)
    }

    /// [`exit`](Self::exit), but a zombie that is to send its parent an
    /// exit signal is not waitable until [`announce`](Self::announce): a
    /// parent on another CPU must not reap the child, and go on, before the
    /// signal is pending (on Linux, wait(2) never returns a child whose
    /// `SIGCHLD` is still to come).
    pub fn exit_announced_later(&mut self, pid: i32, how: Termination, parent_ignores: bool) -> ExitReport {
        self.exit_with(pid, how, parent_ignores, true)
    }

    /// The exit signal of `pid`, which [`exit_announced_later`]
    /// (Self::exit_announced_later) left a zombie, is pending: its parent may
    /// reap it now.
    pub fn announce(&mut self, pid: i32) {
        if let Some(n) = self.nodes.get_mut(&pid).filter(|n| n.unannounced) {
            n.unannounced = false;
            self.generation += 1;
        }
    }

    fn exit_with(&mut self, pid: i32, how: Termination, parent_ignores: bool, later: bool) -> ExitReport {
        let mut report = ExitReport::default();
        let Some(node) = self.nodes.get(&pid) else { return report };
        let children: Vec<i32> = node.children.iter().copied().collect();
        report.parent = node.parent;
        report.exit_signal = node.exit_signal;
        report.adopter = self.adopter(pid);
        for child in children {
            let adopter = self.subreaper_for(child, pid).unwrap_or_else(|| self.adopter_of(child, pid));
            let Some(c) = self.nodes.get_mut(&child) else { continue };
            c.parent = adopter;
            if let Some(how) = c.zombie {
                if adopter == 0 {
                    self.reap(child);
                    continue;
                }
                report.adopted_zombies.push((child, c.uid, how, c.exit_signal, adopter));
            }
            if let Some(a) = self.nodes.get_mut(&adopter) {
                a.children.insert(child);
            }
        }
        if let Some(n) = self.nodes.get_mut(&pid) {
            n.children.clear();
        }
        if report.parent == 0 || parent_ignores {
            self.reap(pid);
        } else if let Some(n) = self.nodes.get_mut(&pid) {
            n.zombie = Some(how);
            n.unannounced = later && n.exit_signal != 0;
            n.report = None;
            report.zombie = true;
        }
        self.generation += 1;
        report
    }

    /// Process `pid` was stopped by `signal`. Returns its parent.
    pub fn stopped(&mut self, pid: i32, signal: u32) -> Option<i32> {
        let n = self.nodes.get_mut(&pid)?;
        n.report = Some(Report::Stopped(signal));
        self.generation += 1;
        Some(n.parent)
    }

    /// Stopped process `pid` was continued. Returns its parent.
    pub fn continued(&mut self, pid: i32) -> Option<i32> {
        let n = self.nodes.get_mut(&pid)?;
        n.report = Some(Report::Continued);
        self.generation += 1;
        Some(n.parent)
    }

    /// The exit signal of `pid`.
    pub fn exit_signal(&self, pid: i32) -> Option<u32> {
        self.nodes.get(&pid).map(|n| n.exit_signal)
    }

    /// Look for an event of a child of `waiter` that `filter` wants, lowest
    /// pid first; take it (reap an ended child) unless the filter says not
    /// to.
    pub fn poll(&mut self, waiter: i32, filter: &WaitFilter) -> Poll {
        let Some(w) = self.nodes.get(&waiter) else { return Poll::NoChildren };
        let mut any = false;
        let mut exiting = false;
        let mut found = None;
        for &child in &w.children {
            let Some(c) = self.nodes.get(&child) else { continue };
            let selected = match filter.selector {
                Selector::Any => true,
                Selector::Pid(pid) => child == pid,
                Selector::Group(g) => c.pgid == g,
            };
            let clone_child = c.exit_signal != crate::abi::signal::SIGCHLD;
            let kind_ok = match filter.clones {
                Clones::Exclude => !clone_child,
                Clones::Only => clone_child,
                Clones::Include => true,
            };
            if !selected || !kind_ok {
                continue;
            }
            any = true;
            if c.zombie.is_some() && c.unannounced {
                exiting |= filter.ended;
                continue;
            }
            let event = match (c.zombie, c.report) {
                (Some(how), _) if filter.ended => Some(ChildEvent::Ended(how)),
                (None, Some(Report::Stopped(sig))) if filter.stopped => Some(ChildEvent::Stopped(sig)),
                (None, Some(Report::Continued)) if filter.continued => Some(ChildEvent::Continued),
                _ => None,
            };
            if let Some(event) = event {
                found = Some(Waited { pid: child, uid: c.uid, event });
                break;
            }
        }
        let Some(waited) = found else {
            return match (exiting, any) {
                (true, _) => Poll::Exiting,
                (false, true) => Poll::Pending,
                (false, false) => Poll::NoChildren,
            };
        };
        if filter.consume {
            match waited.event {
                ChildEvent::Ended(_) => {
                    self.reap(waited.pid);
                    self.generation += 1;
                }
                _ => {
                    if let Some(c) = self.nodes.get_mut(&waited.pid) {
                        c.report = None;
                    }
                }
            }
        }
        Poll::Ready(waited)
    }

    /// setpgid(2) by `caller`: move `pid` (0: the caller) into group
    /// `pgid` (0: `pid`'s own). Errors from the man page, in this order:
    /// `EINVAL` for a negative `pgid`; `ESRCH` if `pid` is neither the
    /// caller nor its child; `EACCES` for a child that has called
    /// execve(2); `EPERM` for a child in another session, a session
    /// leader, or a group that does not exist in the caller's session.
    pub fn setpgid(&mut self, caller: i32, pid: i32, pgid: i32) -> Result<(), Errno> {
        if pgid < 0 {
            return Err(Errno::EINVAL);
        }
        let pid = if pid == 0 { caller } else { pid };
        let pgid = if pgid == 0 { pid } else { pgid };
        let me = self.nodes.get(&caller).ok_or(Errno::ESRCH)?;
        let my_sid = me.sid;
        let is_child = me.children.contains(&pid);
        let target = match self.nodes.get(&pid) {
            Some(t) if pid == caller || (is_child && t.zombie.is_none()) => t,
            _ => return Err(Errno::ESRCH),
        };
        if pid != caller && target.execed {
            return Err(Errno::EACCES);
        }
        if target.sid != my_sid || target.sid == pid {
            return Err(Errno::EPERM);
        }
        if pgid != pid && !self.nodes.values().any(|n| n.pgid == pgid && n.sid == my_sid) {
            return Err(Errno::EPERM);
        }
        self.nodes.get_mut(&pid).expect("found above").pgid = pgid;
        Ok(())
    }

    /// setsid(2) by `caller`: a new session and process group, both with
    /// the caller's pid, which is returned. `EPERM` if any process's group
    /// id is the caller's pid ("The process group ID of any process equals
    /// the PID of the calling process").
    pub fn setsid(&mut self, caller: i32) -> Result<i32, Errno> {
        if !self.nodes.contains_key(&caller) || self.nodes.values().any(|n| n.pgid == caller) {
            return Err(Errno::EPERM);
        }
        let n = self.nodes.get_mut(&caller).expect("checked above");
        n.pgid = caller;
        n.sid = caller;
        Ok(caller)
    }

    /// getpgid(2) of `pid` (0: the caller). `ESRCH` if there is none.
    pub fn getpgid(&self, caller: i32, pid: i32) -> Result<i32, Errno> {
        let pid = if pid == 0 { caller } else { pid };
        self.pgid(pid).ok_or(Errno::ESRCH)
    }

    /// getsid(2) of `pid` (0: the caller). `ESRCH` if there is none.
    pub fn getsid(&self, caller: i32, pid: i32) -> Result<i32, Errno> {
        let pid = if pid == 0 { caller } else { pid };
        self.sid(pid).ok_or(Errno::ESRCH)
    }
}

impl Node {
    fn new(parent: i32, pgid: i32, sid: i32, uid: u32, exit_signal: u32) -> Node {
        Node {
            parent,
            pgid,
            sid,
            uid,
            exit_signal,
            children: BTreeSet::new(),
            zombie: None,
            unannounced: false,
            report: None,
            execed: false,
            reaper: INIT_PID,
            subreaper: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::{SIGCHLD, SIGSTOP, SIGUSR1};

    fn any(ended: bool, stopped: bool, continued: bool) -> WaitFilter {
        WaitFilter { selector: Selector::Any, ended, stopped, continued, consume: true, clones: Clones::Exclude }
    }

    /// init (1) with a shell (10) that has two children (11, 12).
    fn family() -> ProcessTree {
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 1000);
        t.insert_child(10, 10, 12, SIGCHLD, 1000);
        t
    }

    #[test]
    fn children_inherit_group_and_session() {
        let t = family();
        assert_eq!((t.parent(11), t.pgid(11), t.sid(11)), (Some(10), Some(1), Some(1)));
        assert_eq!(t.children(10), [11, 12]);
        assert_eq!(t.parent(1), Some(0));
    }

    #[test]
    fn a_zombie_is_waitable_once_its_exit_signal_is_announced() {
        let mut t = family();
        let r = t.exit_announced_later(11, Termination::Exited(3), false);
        assert!(r.zombie && t.is_zombie(11));
        // Not reaped before the parent's SIGCHLD is pending (the exiting
        // thread may still be sending it on another CPU).
        assert_eq!(t.poll(10, &any(true, false, false)), Poll::Exiting);
        assert_eq!(t.poll(10, &any(false, true, false)), Poll::Pending, "only waits for ends wait for it");
        let g = t.generation();
        t.announce(11);
        assert!(t.generation() > g, "waiters wake");
        assert!(matches!(t.poll(10, &any(true, false, false)), Poll::Ready(Waited { pid: 11, .. })));
        t.exit_announced_later(12, Termination::Exited(0), false);
        t.announce(12);
        assert!(matches!(t.poll(10, &any(true, false, false)), Poll::Ready(Waited { pid: 12, .. })));
    }

    #[test]
    fn a_child_is_a_zombie_until_reaped() {
        let mut t = family();
        assert_eq!(t.poll(10, &any(true, false, false)), Poll::Pending);
        let g = t.generation();
        let r = t.exit(11, Termination::Exited(3), false);
        assert_eq!((r.parent, r.exit_signal, r.zombie), (10, SIGCHLD, true));
        assert!(t.generation() > g && t.is_zombie(11));
        let peek = WaitFilter { consume: false, ..any(true, false, false) };
        let want = Poll::Ready(Waited { pid: 11, uid: 1000, event: ChildEvent::Ended(Termination::Exited(3)) });
        assert_eq!(t.poll(10, &peek), want, "WNOWAIT leaves it");
        assert_eq!(t.poll(10, &any(true, false, false)), want);
        assert!(!t.contains(11) && t.take_reaped() == [11] && t.take_reaped().is_empty());
        assert_eq!(t.poll(10, &any(true, false, false)), Poll::Pending, "12 still runs");
        t.exit(12, Termination::Killed { signal: 9, core: false }, false);
        assert!(matches!(t.poll(10, &any(true, false, false)), Poll::Ready(Waited { pid: 12, .. })));
        assert_eq!(t.poll(10, &any(true, false, false)), Poll::NoChildren);
    }

    #[test]
    fn selectors_and_clone_children() {
        let mut t = family();
        t.insert_child(10, 10, 13, 0, 0);
        t.setpgid(10, 12, 12).unwrap();
        let f = |selector, clones| WaitFilter { selector, clones, ..any(true, false, false) };
        assert_eq!(t.poll(10, &f(Selector::Pid(99), Clones::Exclude)), Poll::NoChildren);
        assert_eq!(t.poll(10, &f(Selector::Group(12), Clones::Exclude)), Poll::Pending);
        assert_eq!(t.poll(10, &f(Selector::Group(77), Clones::Exclude)), Poll::NoChildren);
        assert_eq!(t.poll(10, &f(Selector::Pid(13), Clones::Exclude)), Poll::NoChildren, "13 is a clone child");
        assert_eq!(t.poll(10, &f(Selector::Pid(13), Clones::Only)), Poll::Pending);
        assert_eq!(t.poll(10, &f(Selector::Pid(11), Clones::Only)), Poll::NoChildren);
        assert_eq!(t.poll(10, &f(Selector::Pid(13), Clones::Include)), Poll::Pending);
        let r = t.exit(13, Termination::Exited(0), false);
        assert_eq!(r.exit_signal, 0);
        assert!(matches!(t.poll(10, &f(Selector::Any, Clones::Include)), Poll::Ready(Waited { pid: 13, .. })));
    }

    #[test]
    fn stops_and_continues_are_reported_once() {
        let mut t = family();
        assert_eq!(t.stopped(11, SIGSTOP), Some(10));
        assert_eq!(t.poll(10, &any(true, false, false)), Poll::Pending, "without WUNTRACED");
        let stop = Poll::Ready(Waited { pid: 11, uid: 1000, event: ChildEvent::Stopped(SIGSTOP) });
        assert_eq!(t.poll(10, &any(true, true, false)), stop);
        assert_eq!(t.poll(10, &any(true, true, false)), Poll::Pending, "reported once");
        assert_eq!(t.continued(11), Some(10));
        let cont = Poll::Ready(Waited { pid: 11, uid: 1000, event: ChildEvent::Continued });
        assert_eq!(t.poll(10, &any(false, false, true)), cont);
        t.stopped(11, SIGSTOP);
        t.exit(11, Termination::Killed { signal: 9, core: false }, false);
        assert!(matches!(t.poll(10, &any(true, true, true)), Poll::Ready(Waited { event: ChildEvent::Ended(_), .. })));
    }

    #[test]
    fn orphans_go_to_init_or_the_kernel() {
        let mut t = family();
        t.exit(11, Termination::Exited(1), false);
        let r = t.exit(10, Termination::Exited(0), false);
        assert_eq!(r.adopter, INIT_PID);
        assert_eq!(r.adopted_zombies, [(11, 1000, Termination::Exited(1), SIGCHLD, INIT_PID)]);
        assert_eq!((t.parent(11), t.parent(12)), (Some(1), Some(1)));
        assert_eq!(t.children(1), [10, 11, 12]);
        assert!(matches!(t.poll(1, &any(true, false, false)), Poll::Ready(Waited { pid: 10, .. })));
        // Without init the kernel adopts, and reaps ended orphans at once.
        let mut t = family();
        t.exit(1, Termination::Exited(0), false);
        assert!(t.take_reaped().contains(&1), "init's parent is the kernel");
        t.exit(11, Termination::Exited(0), false);
        let r = t.exit(10, Termination::Exited(0), false);
        assert_eq!(r.adopter, 0);
        assert!(r.adopted_zombies.is_empty());
        assert_eq!(t.take_reaped(), [11, 10]);
        assert_eq!(t.parent(12), Some(0));
        t.exit(12, Termination::Exited(0), false);
        assert_eq!(t.take_reaped(), [12], "the kernel reaps at once");
    }

    #[test]
    fn orphans_go_to_the_nearest_living_subreaper() {
        // init (1) - 10 (subreaper) - 11 (subreaper) - 12 - 13.
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        t.insert_child(11, 11, 12, SIGCHLD, 0);
        t.insert_child(12, 12, 13, SIGCHLD, 0);
        t.set_subreaper(10, true);
        t.set_subreaper(11, true);
        assert!(t.is_subreaper(11) && !t.is_subreaper(12));
        t.exit(12, Termination::Exited(0), false);
        assert_eq!(t.parent(13), Some(11), "the nearest one");
        assert_eq!(t.children(11), [12, 13]);
        // 11 ends: its children go to 10, the next subreaper up.
        t.exit(13, Termination::Exited(5), false);
        let r = t.exit(11, Termination::Exited(0), false);
        assert_eq!(
            r.adopted_zombies,
            [(12, 0, Termination::Exited(0), SIGCHLD, 10), (13, 0, Termination::Exited(5), SIGCHLD, 10)]
        );
        assert_eq!((t.parent(12), t.parent(13)), (Some(10), Some(10)));
        // A subreaper that is a zombie itself, or unmarked, adopts nothing.
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        t.insert_child(11, 11, 12, SIGCHLD, 0);
        t.set_subreaper(10, true);
        t.set_subreaper(10, false);
        t.exit(11, Termination::Exited(0), false);
        assert_eq!(t.parent(12), Some(1));
        // The mark is not inherited.
        t.set_subreaper(12, true);
        t.insert_child(12, 12, 14, SIGCHLD, 0);
        assert!(!t.is_subreaper(14));
    }

    #[test]
    fn a_subreaper_outside_a_pid_namespace_does_not_adopt_from_it() {
        // 10 (subreaper) - 11 (init of a namespace) - 12 - 13.
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        t.insert_child(11, 11, 12, SIGCHLD, 0);
        t.insert_child(12, 12, 13, SIGCHLD, 0);
        t.set_subreaper(10, true);
        t.set_reaper(12, 11);
        t.set_reaper(13, 11);
        t.exit(12, Termination::Exited(0), false);
        assert_eq!(t.parent(13), Some(11), "the namespace's init");
    }

    #[test]
    fn orphans_in_a_pid_namespace_go_to_its_init() {
        // init (1); 10 is the init of a pid namespace, 11 and 12 in it.
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        t.insert_child(11, 11, 12, SIGCHLD, 0);
        t.set_reaper(11, 10);
        t.set_reaper(12, 10);
        t.exit(12, Termination::Exited(3), false);
        let r = t.exit(11, Termination::Exited(0), false);
        assert_eq!(r.adopted_zombies, [(12, 0, Termination::Exited(3), SIGCHLD, 10)], "to the namespace's init");
        assert_eq!(t.parent(12), Some(10));
        // With its init gone, the next reaper out adopts.
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        t.insert_child(11, 11, 12, SIGCHLD, 0);
        t.set_reaper(11, 10);
        t.set_reaper(12, 10);
        t.exit(10, Termination::Killed { signal: 9, core: false }, false);
        t.exit(11, Termination::Exited(0), false);
        assert_eq!(t.parent(12), Some(1));
    }

    #[test]
    fn ignoring_sigchld_reaps_at_once() {
        let mut t = family();
        let r = t.exit(11, Termination::Exited(0), true);
        assert!(!r.zombie && t.take_reaped() == [11]);
        assert_eq!(r.parent, 10, "the parent may still get the signal");
    }

    #[test]
    fn setpgid_rules() {
        let mut t = family();
        assert_eq!(t.setpgid(10, 0, -1), Err(Errno::EINVAL));
        assert_eq!(t.setpgid(10, 1, 0), Err(Errno::ESRCH), "not a child");
        assert_eq!(t.setpgid(10, 0, 0), Ok(()), "the shell leads a group of its own");
        assert_eq!(t.pgid(10), Some(10));
        assert_eq!(t.setpgid(10, 11, 10), Ok(()));
        assert_eq!(t.setpgid(10, 12, 55), Err(Errno::EPERM), "no such group in the session");
        t.exec(12);
        assert_eq!(t.setpgid(10, 12, 10), Err(Errno::EACCES));
        assert_eq!(t.setpgid(1, 1, 0), Err(Errno::EPERM), "init leads its session");
        // A child in another session.
        assert_eq!(t.setsid(11), Ok(11));
        assert_eq!(t.setpgid(10, 11, 10), Err(Errno::EPERM));
        assert_eq!(t.getsid(10, 11), Ok(11));
        assert_eq!(t.getpgid(10, 0), Ok(10));
        assert_eq!(t.getpgid(10, 999), Err(Errno::ESRCH));
    }

    #[test]
    fn setsid_refuses_group_leaders() {
        let mut t = family();
        assert_eq!(t.setsid(1), Err(Errno::EPERM));
        assert_eq!(t.setsid(10), Ok(10));
        assert_eq!((t.pgid(10), t.sid(10)), (Some(10), Some(10)));
        assert_eq!(t.setsid(10), Err(Errno::EPERM), "now it leads a group");
        assert_eq!(t.group(10), [10]);
        // A child of a session leader is in its session and group.
        t.insert_child(10, 10, 20, SIGUSR1, 0);
        assert_eq!((t.pgid(20), t.sid(20)), (Some(10), Some(10)));
    }

    #[test]
    fn failed_clones_leave_nothing() {
        let mut t = family();
        t.insert_child(10, 10, 30, SIGCHLD, 0);
        t.remove(30);
        assert_eq!(t.children(10), [11, 12]);
        assert!(!t.contains(30) && t.take_reaped().is_empty());
        // A creator the tree does not know (a bare kernel record).
        t.insert_child(77, 77, 31, SIGCHLD, 0);
        assert_eq!((t.parent(31), t.pgid(31)), (Some(0), Some(31)));
    }
}
