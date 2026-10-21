//! System calls interrupted by signals: restart or `EINTR`.
//!
//! signal(7), "Interruption of system calls and library functions by
//! signal handlers": a blocked call interrupted by a handler is
//! "automatically restarted after the signal handler returns if the
//! SA_RESTART flag was used; otherwise the call fails with the error
//! EINTR", except for a list of calls that "are never restarted after
//! being interrupted by a signal handler, regardless of the use of
//! SA_RESTART; they always fail with the error EINTR". When no handler
//! runs (the signal stopped the process and `SIGCONT` resumed it), most
//! calls continue transparently; signal(7), "Interruption of system calls
//! and library functions by stop signals", lists those that fail with
//! `EINTR` even then.
//!
//! A handler that is interrupted returns one of the internal
//! `Errno::RESTART_*` codes (or plain `EINTR` for the calls that always
//! fail). The dispatcher records a [`PendingRestart`] in the thread and
//! puts `-EINTR` in `rax`; on the way back to user mode
//! ([`super::deliver::exit_to_user`]) [`apply`] decides, once it knows
//! whether a handler runs. Restarting rewinds `rip` over the 2-byte
//! `syscall` instruction and puts the call number back in `rax`; the
//! argument registers were never changed. A call with a
//! [`RestartBlock`] restarts as restart_syscall(2) instead, which continues
//! from the saved state (a relative timeout keeps counting from where it
//! was, not from the start). restart_syscall(2) describes this for calls
//! resumed after a stop, so these calls restart only when no handler runs.

use crate::abi::signal::SA_RESTART;
use crate::errno::Errno;
use crate::kernel::{Deadline, UserContext};
use crate::nr;

/// When an interrupted call is restarted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Restart unless a handler without `SA_RESTART` runs.
    IfSaRestart,
    /// Always restart.
    Always,
    /// Restart only if no handler runs.
    IfNoHandler,
}

/// An interrupted call waiting for the restart decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingRestart {
    pub policy: RestartPolicy,
    /// The call's number, put back in `rax` to restart it.
    pub number: u64,
    /// Restart through restart_syscall(2) and the thread's [`RestartBlock`].
    pub with_block: bool,
}

/// What restart_syscall(2) continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartBlock {
    /// A `FUTEX_WAIT` with a relative timeout, now an absolute deadline.
    FutexWait { addr: u64, private: bool, val: u32, bitset: u32, deadline: Deadline },
    /// A relative nanosleep / clock_nanosleep, now an absolute deadline;
    /// `rem` is where to report the time left.
    Sleep { deadline: Deadline, rem: u64 },
}

/// What becomes of the interrupted call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Restart,
    Eintr,
}

/// The number a tracer of a Linux process sees at the exit of a call
/// interrupted this way (it is not a user errno: the program sees the
/// call restarted, or `EINTR`): ERESTARTSYS (512) for
/// [`RestartPolicy::IfSaRestart`], ERESTARTNOINTR (513) for
/// [`RestartPolicy::Always`], ERESTARTNOHAND (514) for
/// [`RestartPolicy::IfNoHandler`], ERESTART_RESTARTBLOCK (516) with a
/// [`RestartBlock`]; the diff runner reads them so
/// (`tools/pipeline/src/trace.rs`).
pub const fn traced_errno(r: &PendingRestart) -> i64 {
    match (r.policy, r.with_block) {
        (_, true) => 516,
        (RestartPolicy::IfSaRestart, false) => 512,
        (RestartPolicy::Always, false) => 513,
        (RestartPolicy::IfNoHandler, false) => 514,
    }
}

/// The policy behind an internal restart code; `None` for every other
/// errno. The flag says whether the call restarts with a [`RestartBlock`].
pub const fn policy_of(errno: Errno) -> Option<(RestartPolicy, bool)> {
    match errno {
        Errno::RESTART_IF_SA_RESTART => Some((RestartPolicy::IfSaRestart, false)),
        Errno::RESTART_ALWAYS => Some((RestartPolicy::Always, false)),
        Errno::RESTART_IF_NO_HANDLER => Some((RestartPolicy::IfNoHandler, false)),
        Errno::RESTART_WITH_BLOCK => Some((RestartPolicy::IfNoHandler, true)),
        _ => None,
    }
}

/// Restart or `EINTR`, given the flags of the handler about to run
/// (`None`: no handler runs).
pub const fn decide(policy: RestartPolicy, handler_flags: Option<u64>) -> Decision {
    match (policy, handler_flags) {
        (_, None) | (RestartPolicy::Always, _) => Decision::Restart,
        (RestartPolicy::IfSaRestart, Some(flags)) if flags & SA_RESTART != 0 => Decision::Restart,
        _ => Decision::Eintr,
    }
}

/// The length of the `syscall` instruction (0F 05).
pub const SYSCALL_INSN_LEN: u64 = 2;

/// Decide and change `ctx` accordingly: rewind it to the `syscall`
/// instruction with the call (or restart_syscall) number in `rax`, or
/// leave `-EINTR` in `rax`.
pub fn apply(ctx: &mut UserContext, restart: &PendingRestart, handler_flags: Option<u64>) -> Decision {
    let decision = decide(restart.policy, handler_flags);
    match decision {
        Decision::Restart => {
            ctx.rip = ctx.rip.wrapping_sub(SYSCALL_INSN_LEN);
            ctx.rax = if restart.with_block { nr::RESTART_SYSCALL } else { restart.number };
        }
        Decision::Eintr => ctx.rax = Errno::EINTR.to_rax(),
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::SA_SIGINFO;

    #[test]
    fn decisions_follow_signal_7() {
        use Decision::*;
        use RestartPolicy::*;
        for policy in [IfSaRestart, Always, IfNoHandler] {
            assert_eq!(decide(policy, None), Restart, "no handler: the call continues ({policy:?})");
        }
        assert_eq!(decide(IfSaRestart, Some(SA_RESTART)), Restart);
        assert_eq!(decide(IfSaRestart, Some(SA_SIGINFO)), Eintr);
        assert_eq!(decide(Always, Some(0)), Restart);
        assert_eq!(decide(IfNoHandler, Some(SA_RESTART)), Eintr, "never restarted after a handler");
    }

    #[test]
    fn apply_rewinds_or_leaves_eintr() {
        let ctx = UserContext { rip: 0x40_1002, rax: Errno::EINTR.to_rax(), ..Default::default() };
        let r = PendingRestart { policy: RestartPolicy::IfSaRestart, number: nr::READ, with_block: false };
        let mut c = ctx;
        assert_eq!(apply(&mut c, &r, None), Decision::Restart);
        assert_eq!((c.rip, c.rax), (0x40_1000, nr::READ));
        let mut c = ctx;
        assert_eq!(apply(&mut c, &r, Some(0)), Decision::Eintr);
        assert_eq!(c, ctx);
        let mut c = ctx;
        let (policy, with_block) = policy_of(Errno::RESTART_WITH_BLOCK).unwrap();
        let r = PendingRestart { policy, with_block, number: nr::FUTEX };
        assert_eq!(apply(&mut c, &r, None), Decision::Restart);
        assert_eq!((c.rip, c.rax), (0x40_1000, nr::RESTART_SYSCALL));
        let mut c = ctx;
        assert_eq!(apply(&mut c, &r, Some(SA_RESTART)), Decision::Eintr, "a handler ran");
        assert_eq!(policy_of(Errno::EINTR), None);
        assert!(Errno::RESTART_ALWAYS.is_restart() && !Errno::EINTR.is_restart());
    }
}
