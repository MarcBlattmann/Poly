//! Darwin signals: numbers, default actions, a process's actions, mask and
//! pending set (the sigaction, sigprocmask and kill sheets).

/// Darwin signal numbers (the sigaction sheet; `<sys/signal.h>`).
pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
pub const SIGPIPE: u32 = 13;
pub const SIGTERM: u32 = 15;
pub const SIGURG: u32 = 16;
pub const SIGSTOP: u32 = 17;
pub const SIGTSTP: u32 = 18;
pub const SIGCONT: u32 = 19;
pub const SIGCHLD: u32 = 20;
pub const SIGTTIN: u32 = 21;
pub const SIGTTOU: u32 = 22;
pub const SIGIO: u32 = 23;
pub const SIGWINCH: u32 = 28;
pub const SIGINFO: u32 = 29;
pub const SIGUSR1: u32 = 30;
pub const SIGUSR2: u32 = 31;
/// Signals are 1 to 31.
pub const NSIG: u32 = 32;

pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

/// `sigprocmask` how values.
pub const SIG_BLOCK: u32 = 1;
pub const SIG_UNBLOCK: u32 = 2;
pub const SIG_SETMASK: u32 = 3;

/// `SA_RESETHAND`.
pub const SA_RESETHAND: u32 = 4;

/// The bit of signal `n` in a set.
pub const fn bit(n: u32) -> u32 {
    1 << (n - 1)
}

/// The bits a mask can never hold.
pub const UNBLOCKABLE: u32 = bit(SIGKILL) | bit(SIGSTOP);

/// What a signal does by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Default {
    Terminate,
    Ignore,
    Stop,
    Continue,
}

/// The default action of `sig` (`signal(3)`'s table).
pub const fn default_action(sig: u32) -> Default {
    match sig {
        SIGURG | SIGCHLD | SIGIO | SIGWINCH | SIGINFO => Default::Ignore,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => Default::Stop,
        SIGCONT => Default::Continue,
        _ => Default::Terminate,
    }
}

/// A signal's action as `sigaction` stores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub handler: u64,
    pub trampoline: u64,
    pub mask: u32,
    pub flags: u32,
}

/// What happens to a signal sent to a process now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Dropped (ignored, or its default is to ignore).
    Discard,
    /// The process ends, killed by it.
    Terminate,
    /// It stops or continues the process (not done yet: dropped).
    StopOrContinue,
    /// It waits in the pending set (blocked, or caught: handlers are not
    /// called yet).
    Pending,
}

/// A process's signal state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signals {
    pub actions: [Action; NSIG as usize],
    pub mask: u32,
    pub pending: u32,
}

impl core::default::Default for Signals {
    fn default() -> Self {
        Signals { actions: [Action::default(); NSIG as usize], mask: 0, pending: 0 }
    }
}

impl Signals {
    /// Whether `sig` is a signal number (1 to 31).
    pub const fn valid(sig: u32) -> bool {
        sig >= 1 && sig < NSIG
    }

    /// `sigaction`: the old action, after setting `new` if given.
    pub fn set_action(&mut self, sig: u32, new: Option<Action>) -> Result<Action, u32> {
        if !Self::valid(sig) || sig == SIGKILL || sig == SIGSTOP {
            return Err(super::errno::EINVAL);
        }
        let slot = &mut self.actions[sig as usize];
        let old = *slot;
        if let Some(a) = new {
            *slot = a;
            if a.handler == SIG_IGN {
                self.pending &= !bit(sig);
            }
        }
        Ok(old)
    }

    /// `sigprocmask`: the old mask, after applying `set` with `how`.
    pub fn set_mask(&mut self, how: u32, set: Option<u32>) -> Result<u32, u32> {
        let old = self.mask;
        if let Some(s) = set {
            self.mask = match how {
                SIG_BLOCK => old | s,
                SIG_UNBLOCK => old & !s,
                SIG_SETMASK => s,
                _ => return Err(super::errno::EINVAL),
            } & !UNBLOCKABLE;
        } else if !(SIG_BLOCK..=SIG_SETMASK).contains(&how) && how != 0 {
            return Err(super::errno::EINVAL);
        }
        Ok(old)
    }

    /// What `sig` does to this process now; a pending one is recorded.
    pub fn deliver(&mut self, sig: u32) -> Disposition {
        let a = self.actions[sig as usize];
        if sig == SIGKILL {
            return Disposition::Terminate;
        }
        if a.handler == SIG_IGN {
            return Disposition::Discard;
        }
        if self.mask & bit(sig) != 0 || a.handler != SIG_DFL {
            self.pending |= bit(sig);
            return Disposition::Pending;
        }
        match default_action(sig) {
            Default::Terminate => Disposition::Terminate,
            Default::Ignore => Disposition::Discard,
            Default::Stop | Default::Continue => Disposition::StopOrContinue,
        }
    }

    /// The state of a child started by `posix_spawn` or `execve`: the
    /// mask and ignored signals are kept, caught signals go back to the
    /// default, nothing is pending.
    pub fn for_exec(&self) -> Signals {
        let mut s = *self;
        for a in &mut s.actions {
            if a.handler != SIG_IGN {
                *a = Action::default();
            }
        }
        s.pending = 0;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bsd::errno::EINVAL;

    #[test]
    fn actions_read_back() {
        let mut s = Signals::default();
        let a = Action { handler: 0x1000, trampoline: 0x2000, mask: 4, flags: 0x42 };
        assert_eq!(s.set_action(SIGUSR1, Some(a)).unwrap(), Action::default());
        assert_eq!(s.set_action(SIGUSR1, None).unwrap(), a);
        assert_eq!(s.set_action(SIGKILL, None), Err(EINVAL));
        assert_eq!(s.set_action(SIGSTOP, Some(a)), Err(EINVAL));
        assert_eq!(s.set_action(0, None), Err(EINVAL));
        assert_eq!(s.set_action(32, None), Err(EINVAL));
    }

    #[test]
    fn mask_drops_kill_and_stop() {
        let mut s = Signals::default();
        assert_eq!(s.set_mask(SIG_SETMASK, Some(u32::MAX)).unwrap(), 0);
        assert_eq!(s.mask, 0xfffe_feff);
        s.set_mask(SIG_SETMASK, Some(0)).unwrap();
        s.set_mask(SIG_BLOCK, Some(UNBLOCKABLE)).unwrap();
        assert_eq!(s.mask, 0);
        s.set_mask(SIG_BLOCK, Some(bit(SIGUSR1))).unwrap();
        assert_eq!(s.set_mask(SIG_UNBLOCK, Some(bit(SIGUSR1))).unwrap(), bit(SIGUSR1));
        assert_eq!(s.set_mask(9, Some(0)), Err(EINVAL));
        assert_eq!(s.set_mask(0, Some(0)), Err(EINVAL));
    }

    #[test]
    fn dispositions() {
        let mut s = Signals::default();
        assert_eq!(s.deliver(SIGTERM), Disposition::Terminate);
        assert_eq!(s.deliver(SIGCHLD), Disposition::Discard);
        assert_eq!(s.deliver(SIGWINCH), Disposition::Discard);
        s.set_action(SIGTERM, Some(Action { handler: SIG_IGN, ..Action::default() })).unwrap();
        assert_eq!(s.deliver(SIGTERM), Disposition::Discard);
        assert_eq!(s.deliver(SIGKILL), Disposition::Terminate);
        s.set_mask(SIG_BLOCK, Some(bit(SIGUSR1))).unwrap();
        assert_eq!(s.deliver(SIGUSR1), Disposition::Pending);
        assert_eq!(s.pending, bit(SIGUSR1));
        let child = s.for_exec();
        assert_eq!(child.pending, 0);
        assert_eq!(child.mask, bit(SIGUSR1));
        assert_eq!(child.actions[SIGTERM as usize].handler, SIG_IGN);
    }
}
