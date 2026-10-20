//! Signals the kernel sends on its own (a terminal: `SIGINT` for a typed
//! `^C`, `SIGTTIN`, `SIGWINCH`, hangups): queued with `SI_KERNEL`, no
//! permission check, as the personality's own `task::signal_process`
//! does, but from any thread, also a kernel thread with no Linux process
//! of its own (the terminals' input threads).

use crate::linux::Thread;
use crate::process;
use poly_personality_linux::abi::SigInfo;
use poly_personality_linux::abi::signal::SI_KERNEL;
use poly_personality_linux::signal::Dest;
use poly_personality_linux::task::jobs;

/// Send `signal` to process `pid`. A process it continues (`SIGCONT`) is
/// reported to its parent through `k` when the caller has a Linux thread
/// to do it with; from a kernel thread the parent is not told (no
/// `CLD_CONTINUED`; `SIGCONT` is only sent from process exits, which have
/// one).
pub fn send(pid: u32, signal: u32, k: Option<&mut Thread<'_>>) {
    let Some(p) = process::find(pid) else { return };
    let Some(linux) = p.linux.as_ref() else { return };
    let info = SigInfo::new(signal, SI_KERNEL);
    // kill(2), NOTES: init only gets the signals it handles (a ^C typed
    // on its terminal must not end it).
    let Ok(sent) = linux.with(|s| s.send(info, Dest::Process, pid == 1)) else { return };
    if sent.queued {
        crate::linux::signalfd::signal_queued(pid as i32);
    }
    if sent.continue_process && p.cont() {
        crate::kprintln!("linux: pid {pid} continued");
        if let Some(k) = k {
            poly_personality_linux::task::process_continued(k, pid as i32);
        }
    }
    for tid in sent.notify {
        if let Some((_, flags)) = u32::try_from(tid).ok().and_then(process::find_thread) {
            process::notify(&flags);
        }
    }
}

/// Send `signal` to every live process of group `pgid`.
pub fn send_group(pgid: i32, signal: u32) {
    let members = crate::linux::task::with_tree(|t| jobs::members(t, pgid));
    for pid in members {
        if let Ok(pid) = u32::try_from(pid) {
            send(pid, signal, None);
        }
    }
}
