//! `poly_tty::Host` for the kernel: the calling process from the running
//! user thread and the Linux process tree, signals from the kernel,
//! blocking with park / unpark, user memory through the user-copy
//! routines, the monotonic clock.

use super::signal;
use crate::process::{self, Process, ThreadFlags, Unparked};
use crate::{time, user};
use alloc::boxed::Box;
use alloc::sync::Arc;
use poly_personality_linux::abi::signal::SIG_IGN;
use poly_personality_linux::task::jobs;
use poly_tty::{Caller, Host};
use poly_vfs::{Blocker, Errno, PollEvents, Result, Waker, Woke};

pub struct KernelHost;

/// The running thread's Linux process.
fn linux_process() -> Option<Arc<Process>> {
    user::current_process().filter(|p| p.linux.is_some())
}

impl Host for KernelHost {
    fn caller(&self) -> Option<Caller> {
        let p = linux_process()?;
        let (pgid, sid) = crate::linux::task::group_and_session(p.pid);
        // The calling thread's credentials (ADR 0045): effective ids own a
        // new pty; CAP_SYS_ADMIN is the privilege ioctl_tty(2) asks for.
        let c = crate::linux::cred::current_creds().unwrap_or_default();
        let privileged = c.capable(poly_personality_linux::cred::cap::SYS_ADMIN);
        Some(Caller { pid: p.pid as i32, pgid, sid, uid: c.euid, gid: c.egid, privileged })
    }

    fn blocks_or_ignores(&self, signal: u32) -> bool {
        let (Some(p), Some(tid)) = (linux_process(), user::current_tid()) else { return false };
        let Some(linux) = p.linux.as_ref() else { return false };
        linux.with(|s| {
            let blocked = s.has_thread(tid as i32) && s.thread(tid as i32).mask.contains(signal);
            blocked || s.actions.get(signal).handler == SIG_IGN
        })
    }

    fn signal_group(&self, pgid: i32, signal: u32) {
        signal::send_group(pgid, signal);
    }

    fn signal_process(&self, pid: i32, signal: u32) {
        if let Ok(pid) = u32::try_from(pid) {
            signal::send(pid, signal, None);
        }
    }

    fn session_of_group(&self, pgid: i32) -> Option<i32> {
        crate::linux::task::with_tree(|t| jobs::members(t, pgid).first().and_then(|&m| t.sid(m)))
    }

    fn group_orphaned(&self, pgid: i32) -> bool {
        crate::linux::task::with_tree(|t| jobs::is_orphaned(t, pgid))
    }

    fn now_ns(&self) -> u64 {
        time::now_ns()
    }

    fn blocker(&self) -> Option<Box<dyn Blocker>> {
        let process = user::current_process()?;
        let flags = user::current_flags()?;
        let waker: Arc<dyn Waker> = Arc::new(Unpark(flags.clone()));
        Some(Box::new(Park { process, flags, waker }))
    }

    fn copy_from_user(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        if user::current_process().is_none() {
            return Err(Errno::EFAULT);
        }
        user::copy_from_user(buf, addr).map_err(|_| Errno::EFAULT)
    }

    fn copy_to_user(&self, addr: u64, bytes: &[u8]) -> Result<()> {
        if user::current_process().is_none() {
            return Err(Errno::EFAULT);
        }
        user::copy_to_user(addr, bytes).map_err(|_| Errno::EFAULT)
    }
}

/// Wakes a parked thread.
struct Unpark(Arc<ThreadFlags>);

impl Waker for Unpark {
    fn wake(&self, _: PollEvents) {
        process::unpark(&self.0);
    }
}

/// Blocks the running user thread: its signal notification, or the end
/// of its process, interrupts (the system call layer then restarts the
/// call or fails it with `EINTR`).
struct Park {
    process: Arc<Process>,
    flags: Arc<ThreadFlags>,
    waker: Arc<dyn Waker>,
}

impl Blocker for Park {
    fn waker(&self) -> Arc<dyn Waker> {
        self.waker.clone()
    }

    fn block(&mut self, until: Option<u64>) -> Woke {
        match process::park(&self.process, &self.flags, until) {
            Unparked::Woken => Woke::Woken,
            Unparked::TimedOut => Woke::TimedOut,
            Unparked::Interrupted => Woke::Interrupted,
        }
    }
}
