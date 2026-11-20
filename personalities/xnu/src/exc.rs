//! Mach exceptions (ADR 0238, addendum): what happens when a thread of an
//! XNU process faults, and the thread state requests handlers make.
//!
//! The kernel turns the CPU fault into an exception type and codes
//! (`poly_mach::exc::x86_exception`) and calls [`deliver`]. The thread
//! stops: its state is kept in [`MachState::stopped`] for thread_get_state
//! and thread_set_state, and each handler registered for the type (the
//! thread's, the task's, the host's) gets the `exception_raise*` message in
//! turn until one replies `KERN_SUCCESS`. Then the thread goes on with the
//! state the reply or thread_set_state gave it. Without such a reply the
//! default action follows: the BSD signal.

use crate::kernel::{MachState, Stopped, Wakeup, XnuKernel, unblock_all, with_mach};
use crate::wait::block_for;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_mach::consts::kern::*;
use poly_mach::exc::{self, ExceptionState64, ThreadState64};
use poly_mach::{Exception, RcvKey, ThreadId, ThreadServer, WakeReason};

/// What becomes of a faulted thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A handler took the exception: go on with these registers.
    Resume(ThreadState64),
    /// Nobody did: the default action (the BSD signal).
    Default,
}

/// A CPU fault of the calling thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub vector: u8,
    pub error: u32,
    /// The faulting address of a page fault.
    pub address: Option<u64>,
    /// A page fault on a mapped page (the protection refused the access).
    pub mapped: bool,
}

/// Raise the exception `fault` causes in the calling thread, whose
/// registers are `regs`, and wait for the handlers.
pub fn deliver(k: &mut dyn XnuKernel, fault: Fault, regs: ThreadState64) -> Outcome {
    let Some((ty, codes)) = exc::x86_exception(fault.vector, fault.address, fault.mapped) else {
        return Outcome::Default;
    };
    let (task, thread) = (k.task(), k.thread());
    let e = Exception { task, thread, ty, codes };
    let exc_state = ExceptionState64 {
        trapno: u16::from(fault.vector),
        cpu: 0,
        err: fault.error,
        faultvaddr: fault.address.unwrap_or(0),
    };
    let handlers = with_mach(k, |m, _| {
        m.stopped.insert(thread, Stopped { regs, exc: exc_state, changed: false });
        m.ipc.exception_handlers(task, thread, ty)
    });
    let mut outcome = Outcome::Default;
    for h in handlers {
        match ask(k, &h, &e) {
            Asked::Handled(new) => {
                outcome = Outcome::Resume(new);
                break;
            }
            Asked::Declined => continue,
            Asked::Interrupted => break,
        }
    }
    with_mach(k, |m, _| {
        m.stopped.remove(&thread);
    });
    outcome
}

enum Asked {
    Handled(ThreadState64),
    Declined,
    Interrupted,
}

/// Send one handler the exception and wait for its reply.
fn ask(k: &mut dyn XnuKernel, h: &exc::Handler, e: &Exception) -> Asked {
    let me = k.thread();
    let sent = with_mach(k, |m, k| {
        let s = m.stopped.get(&me).copied().unwrap_or_default();
        let state = exc::state_words(h.flavor, &s.regs, &s.exc);
        let reply = m.ipc.kernel_port();
        match m.ipc.raise(h, e, state.as_deref(), reply) {
            Ok(woken) => {
                m.ipc.wait_receive(RcvKey::Port(reply), k.thread());
                Some((reply, woken))
            }
            Err(_) => {
                let _ = m.ipc.free_kernel_port(reply);
                None
            }
        }
    });
    let Some((reply, woken)) = sent else { return Asked::Declined };
    if let Some(t) = woken {
        k.unblock(t);
    }
    let waited = block_for(k, None, |m, t| m.ipc.wait_result(t), |m, t| m.ipc.cancel_waits(t));
    let answer = with_mach(k, |m, _| {
        let got = m.ipc.kernel_receive(reply);
        let woken = m.ipc.free_kernel_port(reply);
        let changed = m.stopped.get(&me).filter(|s| s.changed).map(|s| s.regs);
        (got, woken, changed, m.stopped.get(&me).map(|s| s.regs))
    });
    let (got, woken, changed, regs) = answer;
    unblock_all(k, woken);
    match waited {
        Err(Wakeup::Signal) => return Asked::Interrupted,
        Ok(WakeReason::Died) | Err(_) => return Asked::Declined,
        Ok(WakeReason::Ready) => {}
    }
    let Some((_, body)) = got else { return Asked::Declined };
    let rd = |at: usize| body.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")));
    if rd(8) != Some(KERN_SUCCESS) {
        return Asked::Declined;
    }
    let base = h.behavior & !exc::MACH_EXCEPTION_MASK;
    let mut new = changed.or(regs).unwrap_or_default();
    if matches!(base, exc::EXCEPTION_STATE | exc::EXCEPTION_STATE_IDENTITY)
        && let (Some(flavor), Some(n)) = (rd(12), rd(16))
    {
        let words: Option<Vec<u32>> = (0..n as usize).map(|i| rd(20 + 4 * i)).collect();
        if let Some(r) = words.and_then(|w| exc::regs_from_words(flavor, &w)) {
            new = r;
        }
    }
    Asked::Handled(new)
}

/// The kernel's thread services for MIG requests: the state of threads
/// stopped in an exception, and of the caller itself.
#[derive(Debug)]
pub struct Threads<'a> {
    pub stopped: &'a mut BTreeMap<ThreadId, Stopped>,
    pub caller: ThreadId,
    pub current: Option<ThreadState64>,
}

impl ThreadServer for Threads<'_> {
    fn get_state(&mut self, thread: ThreadId, flavor: u32, max: u32) -> Result<Vec<u32>, u32> {
        let (regs, exc_state) = match self.stopped.get(&thread) {
            Some(s) => (s.regs, s.exc),
            None if thread == self.caller => (self.current.ok_or(KERN_FAILURE)?, ExceptionState64::default()),
            None => return Err(KERN_FAILURE),
        };
        let words = exc::state_words(flavor, &regs, &exc_state).ok_or(KERN_INVALID_ARGUMENT)?;
        if words.len() > max as usize {
            return Err(KERN_INVALID_ARGUMENT);
        }
        Ok(words)
    }

    fn set_state(&mut self, thread: ThreadId, flavor: u32, state: &[u32]) -> u32 {
        let Some(s) = self.stopped.get_mut(&thread) else { return KERN_FAILURE };
        match exc::regs_from_words(flavor, state) {
            Some(r) => {
                s.regs = r;
                s.changed = true;
                KERN_SUCCESS
            }
            None => KERN_INVALID_ARGUMENT,
        }
    }
}

/// Run `f` with the thread services of `m` for the calling thread.
pub fn with_threads<R>(
    m: &mut MachState,
    k: &dyn XnuKernel,
    f: impl FnOnce(&mut poly_mach::Ipc, &mut Threads<'_>) -> R,
) -> R {
    let MachState { ipc, stopped, .. } = m;
    let mut t = Threads { stopped, caller: k.thread(), current: k.current_state() };
    f(ipc, &mut t)
}
