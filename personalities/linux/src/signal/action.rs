//! What a signal does when it is delivered: the default actions of
//! signal(7) and the disposition a [`SigAction`] gives.

use crate::abi::SigAction;
use crate::abi::signal::*;
use crate::kernel::SigSet;

/// The default action of a signal (signal(7), "Signal dispositions").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultAction {
    /// "Term": terminate the process.
    Terminate,
    /// "Core": terminate the process and dump core.
    Core,
    /// "Ign": ignore the signal.
    Ignore,
    /// "Stop": stop the process.
    Stop,
    /// "Cont": continue the process if it is stopped.
    Continue,
}

/// The default action of `signal` (1-64), from the table in signal(7),
/// "Standard signals". "The default action for an unhandled real-time
/// signal is to terminate the receiving process" (signal(7), "Real-time
/// signals").
pub const fn default_action(signal: u32) -> DefaultAction {
    use DefaultAction::*;
    match signal {
        SIGABRT | SIGBUS | SIGFPE | SIGILL | SIGQUIT | SIGSEGV | SIGSYS | SIGTRAP | SIGXCPU | SIGXFSZ => Core,
        SIGCHLD | SIGURG | SIGWINCH => Ignore,
        SIGCONT => Continue,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => Stop,
        // SIGALRM SIGHUP SIGINT SIGIO SIGKILL SIGPIPE SIGPROF SIGPWR
        // SIGSTKFLT SIGTERM SIGUSR1 SIGUSR2 SIGVTALRM, and SIGRTMIN-SIGRTMAX.
        _ => Terminate,
    }
}

/// A real-time signal (`SIGRTMIN` to `SIGRTMAX`): queued with its siginfo
/// each time it is sent.
pub const fn is_realtime(signal: u32) -> bool {
    signal >= SIGRTMIN && signal <= NSIG
}

/// The stop signals. Generating one discards pending `SIGCONT`, and
/// generating `SIGCONT` discards pending stop signals (POSIX, "Signal
/// Generation and Delivery").
pub const STOP_SIGNALS: SigSet =
    SigSet(SigSet::only(SIGSTOP).0 | SigSet::only(SIGTSTP).0 | SigSet::only(SIGTTIN).0 | SigSet::only(SIGTTOU).0);

/// The signals the CPU raises for the instruction that caused them
/// (faults and traps). They are delivered before any other pending
/// signal, so a handler sees the fault of the instruction it interrupted.
/// signal(7) leaves the order of standard signals unspecified; this is
/// Poly's choice, to be confirmed by the diff runner.
pub const SYNCHRONOUS: SigSet = SigSet(
    SigSet::only(SIGSEGV).0
        | SigSet::only(SIGBUS).0
        | SigSet::only(SIGILL).0
        | SigSet::only(SIGTRAP).0
        | SigSet::only(SIGFPE).0
        | SigSet::only(SIGSYS).0,
);

/// What happens to a signal under an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// `SIG_IGN`.
    Ignore,
    /// `SIG_DFL`: the signal's default action.
    Default(DefaultAction),
    /// A handler function.
    Handler,
}

/// The disposition of `signal` under `action`. `SIGKILL` and `SIGSTOP`
/// always get their default (rt_sigaction refuses to change them, and
/// [`crate::signal::LinuxProcess::set_action`] keeps them).
pub const fn disposition(signal: u32, action: &SigAction) -> Disposition {
    if signal == SIGKILL || signal == SIGSTOP {
        return Disposition::Default(default_action(signal));
    }
    match action.handler {
        SIG_DFL => Disposition::Default(default_action(signal)),
        SIG_IGN => Disposition::Ignore,
        _ => Disposition::Handler,
    }
}

/// True if delivering `signal` under `action` would do nothing: `SIG_IGN`,
/// or `SIG_DFL` with the default action "Ign" or "Cont" (continuing
/// happens when `SIGCONT` is generated, not when it is delivered).
pub const fn is_ignored(signal: u32, action: &SigAction) -> bool {
    matches!(
        disposition(signal, action),
        Disposition::Ignore | Disposition::Default(DefaultAction::Ignore | DefaultAction::Continue)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use DefaultAction::*;

    #[test]
    fn default_actions_follow_signal_7() {
        let table = [
            (SIGHUP, Terminate),
            (SIGINT, Terminate),
            (SIGQUIT, Core),
            (SIGILL, Core),
            (SIGTRAP, Core),
            (SIGABRT, Core),
            (SIGBUS, Core),
            (SIGFPE, Core),
            (SIGKILL, Terminate),
            (SIGUSR1, Terminate),
            (SIGSEGV, Core),
            (SIGUSR2, Terminate),
            (SIGPIPE, Terminate),
            (SIGALRM, Terminate),
            (SIGTERM, Terminate),
            (SIGSTKFLT, Terminate),
            (SIGCHLD, Ignore),
            (SIGCONT, Continue),
            (SIGSTOP, Stop),
            (SIGTSTP, Stop),
            (SIGTTIN, Stop),
            (SIGTTOU, Stop),
            (SIGURG, Ignore),
            (SIGXCPU, Core),
            (SIGXFSZ, Core),
            (SIGVTALRM, Terminate),
            (SIGPROF, Terminate),
            (SIGWINCH, Ignore),
            (SIGIO, Terminate),
            (SIGPWR, Terminate),
            (SIGSYS, Core),
        ];
        assert_eq!(table.len(), 31);
        for (sig, action) in table {
            assert_eq!(default_action(sig), action, "signal {sig}");
            assert!(!is_realtime(sig));
        }
        for sig in SIGRTMIN..=NSIG {
            assert_eq!(default_action(sig), Terminate);
            assert!(is_realtime(sig));
        }
    }

    #[test]
    fn dispositions() {
        let dfl = SigAction::default();
        let ign = SigAction { handler: SIG_IGN, ..dfl };
        let handler = SigAction { handler: 0x40_1000, ..dfl };
        assert_eq!(disposition(SIGINT, &dfl), Disposition::Default(Terminate));
        assert_eq!(disposition(SIGINT, &ign), Disposition::Ignore);
        assert_eq!(disposition(SIGINT, &handler), Disposition::Handler);
        assert_eq!(disposition(SIGKILL, &handler), Disposition::Default(Terminate), "never caught");
        assert_eq!(disposition(SIGSTOP, &ign), Disposition::Default(Stop), "never ignored");
        assert!(is_ignored(SIGCHLD, &dfl) && is_ignored(SIGCONT, &dfl) && is_ignored(SIGTERM, &ign));
        assert!(!is_ignored(SIGTERM, &dfl) && !is_ignored(SIGCHLD, &handler) && !is_ignored(SIGKILL, &ign));
    }
}
