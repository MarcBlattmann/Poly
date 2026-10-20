//! Processes and threads (ADR 0041): clone(2) and its relatives
//! ([`clone`]), execve(2) ([`exec`]), the process tree with parents,
//! zombies, process groups and sessions ([`tree`]), wait(2) ([`wait`]),
//! and what happens to the parent when a child ends, stops or continues
//! (this module).
//!
//! The system calls are in [`crate::sys::task`]; the kernel creates
//! threads and processes ([`LinuxKernel::clone_task`]), replaces programs
//! ([`LinuxKernel::execve`]), keeps the [`tree::ProcessTree`]
//! ([`LinuxKernel::with_tree`]) and calls [`process_exited`],
//! [`process_stopped`] and [`process_continued`] at those events.

pub mod clone;
pub mod exec;
pub mod tree;
pub mod wait;

use crate::abi::SigInfo;
use crate::abi::signal::{SA_NOCLDSTOP, SA_NOCLDWAIT, SIG_IGN, SIGCHLD};
use crate::errno::Errno;
use crate::kernel::{LinuxKernel, Target};
use crate::signal::deliver::{complete, with_target};
use crate::signal::state::Dest;
use tree::{ChildEvent, ProcessTree, Termination, Waited};

/// Run `f` with the process tree locked; `ENOSYS` from a kernel without
/// one.
pub fn with_tree<R>(k: &mut dyn LinuxKernel, f: impl FnOnce(&mut ProcessTree) -> R) -> Result<R, Errno> {
    let mut f = Some(f);
    let mut out = None;
    k.with_tree(&mut |t| {
        if let Some(f) = f.take() {
            out = Some(f(t));
        }
    })?;
    out.ok_or(Errno::ENOSYS)
}

/// The flags of `pid`'s `SIGCHLD` disposition, and whether it is ignored.
fn sigchld_of(k: &mut dyn LinuxKernel, pid: i32) -> Option<(bool, u64)> {
    with_target(k, Target::Process(pid), |p, _| {
        let a = p.actions.get(SIGCHLD);
        (a.handler == SIG_IGN, a.flags)
    })
    .ok()
}

/// Queue `info` for process `pid` from the kernel (no permission check, no
/// protection of init: init still only acts on what it handles, since the
/// default of `SIGCHLD` is to ignore it).
pub fn signal_process(k: &mut dyn LinuxKernel, pid: i32, info: SigInfo) {
    if let Ok(Ok(sent)) = with_target(k, Target::Process(pid), |p, _| p.send(info, Dest::Process, false)) {
        complete(k, pid, &sent);
    }
}

/// The calling thread was the last of its process, which ended `how`:
/// update the tree (its children go to init or the kernel; it becomes a
/// zombie, or is reaped at once if its parent ignores `SIGCHLD` or has
/// `SA_NOCLDWAIT`, as sigaction(2) describes) and send the parent the
/// process's exit signal, with `CLD_EXITED`, `CLD_KILLED` or
/// `CLD_DUMPED`. Ended children that init adopts are announced to init
/// the same way. Called by the kernel after the process's last thread
/// left it.
pub fn process_exited(k: &mut dyn LinuxKernel, how: Termination) {
    let pid = k.pid();
    let uid = k.credentials().uid;
    let Ok(Some(parent)) = with_tree(k, |t| t.parent(pid)) else { return };
    let ignores = parent != 0 && sigchld_of(k, parent).is_some_and(|(ign, flags)| ign || flags & SA_NOCLDWAIT != 0);
    let Ok(report) = with_tree(k, |t| t.exit(pid, how, ignores)) else { return };
    if report.parent != 0 && report.exit_signal != 0 {
        let waited = Waited { pid, uid, event: ChildEvent::Ended(how) };
        signal_process(k, report.parent, wait::child_info(report.exit_signal, &waited));
    }
    for &(child, uid, how, signal) in &report.adopted_zombies {
        if signal != 0 {
            let waited = Waited { pid: child, uid, event: ChildEvent::Ended(how) };
            signal_process(k, report.adopter, wait::child_info(signal, &waited));
        }
    }
}

/// Tell the parent of `pid` (by `SIGCHLD`, unless it set `SA_NOCLDSTOP`)
/// and its waiters (`WUNTRACED`, `WCONTINUED`) that `pid` stopped or
/// continued.
fn report_to_parent(k: &mut dyn LinuxKernel, pid: i32, event: ChildEvent) {
    let found = with_tree(k, |t| {
        let parent = match event {
            ChildEvent::Stopped(signal) => t.stopped(pid, signal),
            _ => t.continued(pid),
        };
        parent.map(|p| (p, t.uid(pid).unwrap_or(0)))
    });
    let Ok(Some((parent, uid))) = found else { return };
    if parent == 0 || sigchld_of(k, parent).is_none_or(|(_, flags)| flags & SA_NOCLDSTOP != 0) {
        return;
    }
    signal_process(k, parent, wait::sigchld(&Waited { pid, uid, event }));
}

/// The calling process was stopped by `signal` (sigaction(2):
/// `SA_NOCLDSTOP` "do not receive notification when child processes
/// stop"). Called by the kernel as it stops the process.
pub fn process_stopped(k: &mut dyn LinuxKernel, signal: u32) {
    let pid = k.pid();
    report_to_parent(k, pid, ChildEvent::Stopped(signal));
}

/// Stopped process `pid` was continued (`SIGCONT`). Called by the kernel
/// in the thread that continued it.
pub fn process_continued(k: &mut dyn LinuxKernel, pid: i32) {
    report_to_parent(k, pid, ChildEvent::Continued);
}
