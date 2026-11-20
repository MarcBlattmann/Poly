//! Blocking until a waker leaves a result.

use crate::kernel::{MachState, Wakeup, XnuKernel, with_mach};

/// Block until `take` finds a result for the calling thread (a waker left
/// one), the deadline passes or a signal arrives. The thread must already be
/// queued. On a timeout or a signal `cancel` takes it off its queue; a wake
/// that won the race is still taken. `Err` says why the wait ended without a
/// result.
pub fn block_for<R>(
    k: &mut dyn XnuKernel,
    deadline: Option<u64>,
    mut take: impl FnMut(&mut MachState, u64) -> Option<R>,
    mut cancel: impl FnMut(&mut MachState, u64),
) -> Result<R, Wakeup> {
    let me = k.thread();
    loop {
        let why = k.block(deadline);
        let got = with_mach(k, |m, _| {
            if let Some(r) = take(m, me) {
                return Some(Ok(r));
            }
            match why {
                Wakeup::Woken => None,
                other => {
                    cancel(m, me);
                    Some(Err(other))
                }
            }
        });
        if let Some(r) = got {
            return r;
        }
        // An old unblock: still queued, wait on.
        if deadline.is_some_and(|d| k.now_ns() >= d) {
            return with_mach(k, |m, _| match take(m, me) {
                Some(r) => Ok(r),
                None => {
                    cancel(m, me);
                    Err(Wakeup::TimedOut)
                }
            });
        }
    }
}

/// A deadline `ns` nanoseconds from now.
pub fn after(k: &dyn XnuKernel, ns: u64) -> u64 {
    k.now_ns().saturating_add(ns)
}
