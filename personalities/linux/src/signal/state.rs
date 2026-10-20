//! The Linux state of a process and its threads: dispositions, pending
//! signals, masks, alternate stacks, and the per-thread addresses the
//! kernel acts on at thread exit (robust list, `clear_child_tid`).
//!
//! The kernel stores one [`LinuxProcess`] per process and lends it to the
//! personality under the process's lock
//! ([`crate::kernel::LinuxKernel::with_process`]). Everything here is
//! plain data and decisions; waking threads and stopping or killing the
//! process are left to the caller, which [`Sent`] tells what to do.

use super::action::{DefaultAction, Disposition, STOP_SIGNALS, default_action, disposition, is_ignored, is_realtime};
use super::altstack::AltStack;
use super::pending::{SigQueue, next_signal};
use super::restart::{PendingRestart, RestartBlock};
use crate::abi::signal::{SIG_DFL, SIG_IGN, SIGCONT, SIGKILL, SIGSTOP};
use crate::abi::{SigAction, SigInfo};
use crate::errno::Errno;
use crate::kernel::{SigSet, SignalActions, UserPtr};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// How many real-time signals one queue holds before `kill`,
/// `rt_sigqueueinfo` and friends fail with `EAGAIN` (Linux counts them per
/// user against `RLIMIT_SIGPENDING`; Poly per queue for now). The kernel
/// may change [`LinuxProcess::queue_limit`].
pub const DEFAULT_QUEUE_LIMIT: usize = 4096;

/// One thread's signal and futex state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinuxThread {
    /// The blocked signals; never contains `SIGKILL` or `SIGSTOP`.
    pub mask: SigSet,
    /// Signals sent to this thread (tkill, tgkill, faults).
    pub pending: SigQueue,
    pub altstack: AltStack,
    /// The mask to restore once the signal that ended an
    /// rt_sigsuspend(2) is handled (the handler's frame saves this one).
    pub saved_mask: Option<SigSet>,
    /// The interrupted system call, until signal delivery decides.
    pub restart: Option<PendingRestart>,
    /// What restart_syscall(2) continues.
    pub restart_block: Option<RestartBlock>,
    /// set_robust_list(2): the head of the thread's robust futex list.
    pub robust_list: UserPtr,
    /// set_tid_address(2) / `CLONE_CHILD_CLEARTID`: cleared and woken at
    /// exit.
    pub clear_child_tid: UserPtr,
}

impl LinuxThread {
    /// A thread with `mask` blocked and nothing else set.
    pub fn new(mask: SigSet) -> Self {
        LinuxThread { mask: mask.without(SigSet::UNBLOCKABLE), ..Default::default() }
    }
}

/// Where a signal goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    /// To the process: any thread that does not block it may take it.
    Process,
    /// To one thread (tkill, tgkill, faults).
    Thread(i32),
}

/// What sending a signal requires of the kernel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sent {
    /// Threads to notify ([`crate::kernel::LinuxKernel::notify_signal`]).
    pub notify: Vec<i32>,
    /// Continue the process if it is stopped (`SIGCONT`, `SIGKILL`).
    pub continue_process: bool,
    /// The signal was queued (false: ignored, or merged with a pending
    /// instance of the same standard signal).
    pub queued: bool,
}

/// One process's Linux state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxProcess {
    pub actions: SignalActions,
    /// Signals sent to the process as a whole.
    pub shared: SigQueue,
    threads: BTreeMap<i32, LinuxThread>,
    /// See [`DEFAULT_QUEUE_LIMIT`].
    pub queue_limit: usize,
    /// Resource limits (getrlimit(2)): inherited by fork, kept by exec.
    pub limits: crate::rlimit::ResourceLimits,
}

impl LinuxProcess {
    /// A new process with one thread, `main_tid`: default actions, nothing
    /// blocked or pending (what the first process starts with).
    pub fn new(main_tid: i32) -> Self {
        let mut threads = BTreeMap::new();
        threads.insert(main_tid, LinuxThread::default());
        LinuxProcess {
            actions: SignalActions::new(),
            shared: SigQueue::new(),
            threads,
            queue_limit: DEFAULT_QUEUE_LIMIT,
            limits: crate::rlimit::ResourceLimits::new(),
        }
    }

    /// The child of fork(2), made by thread `parent_tid`, with main thread
    /// `child_tid`. fork(2): "The child inherits copies of the parent's
    /// set of open ... signal dispositions"; "The child's set of pending
    /// signals is initially empty"; the signal mask and the alternate stack
    /// (sigaltstack(2)) are inherited from the calling thread. The robust
    /// list is not (set_robust_list(2) registers it per thread; to be
    /// confirmed by the diff runner).
    pub fn fork(&self, parent_tid: i32, child_tid: i32) -> LinuxProcess {
        let parent = self.thread(parent_tid);
        let child = LinuxThread { mask: parent.mask, altstack: parent.altstack, ..LinuxThread::default() };
        let mut threads = BTreeMap::new();
        threads.insert(child_tid, child);
        LinuxProcess {
            actions: self.actions,
            shared: SigQueue::new(),
            threads,
            queue_limit: self.queue_limit,
            limits: self.limits,
        }
    }

    /// execve(2) by thread `tid`, which becomes the only thread. execve(2):
    /// "The dispositions of any signals that are being caught are reset to
    /// the default", ignored signals stay ignored, "the process's set of
    /// pending signals" and "signal mask" are preserved, and "any
    /// alternate signal stack is not preserved". The robust list and
    /// `clear_child_tid` belong to the old program and are dropped.
    pub fn exec(&mut self, tid: i32) {
        for signal in 1..=crate::abi::signal::NSIG {
            let action = self.actions.get(signal);
            if action.handler != SIG_DFL && action.handler != SIG_IGN {
                self.actions.set(signal, SigAction::default());
            }
        }
        let mut thread = self.threads.remove(&tid).unwrap_or_default();
        thread.altstack = AltStack::DISABLED;
        thread.robust_list = UserPtr::NULL;
        thread.clear_child_tid = UserPtr::NULL;
        thread.saved_mask = None;
        thread.restart = None;
        thread.restart_block = None;
        self.threads.clear();
        self.threads.insert(tid, thread);
    }

    /// A new thread `tid` created by `parent_tid` with clone(2). It
    /// inherits the mask; sigaltstack(2): the alternate stack is inherited
    /// "unless the clone flags include CLONE_VM and do not include
    /// CLONE_VFORK, in which case any alternate signal stack that was
    /// established in the parent is disabled".
    pub fn add_thread(&mut self, parent_tid: i32, tid: i32, clone_vm_without_vfork: bool) {
        let parent = self.thread(parent_tid);
        let altstack = if clone_vm_without_vfork { AltStack::DISABLED } else { parent.altstack };
        let thread = LinuxThread { mask: parent.mask, altstack, ..LinuxThread::default() };
        self.threads.insert(tid, thread);
    }

    /// Forget thread `tid` (it has exited). Signals pending for it alone
    /// go with it; the process's shared ones stay.
    pub fn remove_thread(&mut self, tid: i32) -> Option<LinuxThread> {
        self.threads.remove(&tid)
    }

    /// Whether the process has a thread `tid`.
    pub fn has_thread(&self, tid: i32) -> bool {
        self.threads.contains_key(&tid)
    }

    /// Thread `tid`'s state. Panics if the process has no such thread: the
    /// kernel registers every thread before it runs.
    pub fn thread(&self, tid: i32) -> &LinuxThread {
        self.threads.get(&tid).expect("thread registered with its LinuxProcess")
    }

    /// Thread `tid`'s state, for changing. Panics like [`Self::thread`].
    pub fn thread_mut(&mut self, tid: i32) -> &mut LinuxThread {
        self.threads.get_mut(&tid).expect("thread registered with its LinuxProcess")
    }

    /// The process's thread ids, lowest first.
    pub fn thread_ids(&self) -> impl Iterator<Item = i32> + '_ {
        self.threads.keys().copied()
    }

    /// Install `action` for `signal` (not `SIGKILL` or `SIGSTOP`), keeping
    /// only the flags Poly knows ([`crate::abi::signal::SA_KNOWN`]). POSIX
    /// (sigaction): "Setting a signal action to SIG_IGN for a signal that
    /// is pending shall cause the pending signal to be discarded, whether
    /// or not it is blocked"; Poly does the same for `SIG_DFL` when the
    /// default is to ignore.
    pub fn set_action(&mut self, signal: u32, action: SigAction) {
        let action = SigAction { flags: action.flags & crate::abi::signal::SA_KNOWN, ..action };
        self.actions.set(signal, action);
        if is_ignored(signal, &action) {
            self.discard(SigSet::only(signal));
        }
    }

    /// Discard pending instances of `signals`, process-wide and in every
    /// thread.
    pub fn discard(&mut self, signals: SigSet) {
        self.shared.discard(signals);
        for t in self.threads.values_mut() {
            t.pending.discard(signals);
        }
    }

    /// Whether `signal` is blocked for `dest`: in that thread, or for the
    /// process in every thread.
    fn blocked(&self, signal: u32, dest: Dest) -> bool {
        match dest {
            Dest::Thread(tid) => self.threads.get(&tid).is_some_and(|t| t.mask.contains(signal)),
            Dest::Process => self.threads.values().all(|t| t.mask.contains(signal)),
        }
    }

    /// Generate `info` (signal 1-64) for `dest`. `protect_init` is set when
    /// the target is process 1 and the sender is not the kernel: kill(2),
    /// NOTES: "The only signals that can be sent to process ID 1, the init
    /// process, are those for which init has explicitly installed signal
    /// handlers."
    ///
    /// - Stop signals discard pending `SIGCONT`, and `SIGCONT` discards
    ///   pending stop signals and continues the process even if it is
    ///   ignored or blocked (POSIX); `SIGKILL` also continues it, so it
    ///   can die.
    /// - Ignored signals are discarded at once: `SIG_IGN` always (the
    ///   sigpending(2) note: a signal that is "both blocked and has a
    ///   disposition of ignored ... is not added to the mask of pending
    ///   signals"), and a default-ignored one unless it is blocked, so
    ///   sigwaitinfo(2) can still take it (to be confirmed by the diff
    ///   runner).
    /// - Standard signals merge with a pending instance; real-time signals
    ///   queue, `EAGAIN` beyond [`Self::queue_limit`].
    /// - The thread to notify: the destination thread, or the first thread
    ///   (lowest id) that does not block it; every thread when the signal
    ///   will kill the process (`SIGKILL`, or an unblocked signal whose
    ///   default action terminates).
    pub fn send(&mut self, info: SigInfo, dest: Dest, protect_init: bool) -> Result<Sent, Errno> {
        let signal = info.signal();
        let mut sent = Sent::default();
        if STOP_SIGNALS.contains(signal) {
            self.discard(SigSet::only(SIGCONT));
        }
        if signal == SIGCONT {
            self.discard(STOP_SIGNALS);
            sent.continue_process = true;
        }
        if signal == SIGKILL {
            sent.continue_process = true;
        }
        let action = self.actions.get(signal);
        if protect_init && disposition(signal, &action) != Disposition::Handler {
            sent.continue_process = false;
            return Ok(sent);
        }
        let blocked = self.blocked(signal, dest);
        let ignored = match disposition(signal, &action) {
            Disposition::Ignore => true,
            Disposition::Default(DefaultAction::Ignore | DefaultAction::Continue) => !blocked,
            _ => false,
        };
        if ignored {
            return Ok(sent);
        }
        let queue_limit = self.queue_limit;
        let queue = match dest {
            Dest::Process => &mut self.shared,
            Dest::Thread(tid) => &mut self.threads.get_mut(&tid).ok_or(Errno::ESRCH)?.pending,
        };
        if is_realtime(signal) && queue.realtime_len() >= queue_limit {
            return Err(Errno::EAGAIN);
        }
        if !queue.push(info) {
            return Ok(sent);
        }
        sent.queued = true;
        let fatal = signal == SIGKILL
            || (!blocked
                && action.handler == SIG_DFL
                && matches!(default_action(signal), DefaultAction::Terminate | DefaultAction::Core));
        if fatal || signal == SIGSTOP {
            sent.notify = self.threads.keys().copied().collect();
        } else {
            match dest {
                Dest::Thread(tid) if !blocked => sent.notify.push(tid),
                Dest::Thread(_) => {}
                Dest::Process => {
                    sent.notify.extend(self.threads.iter().find(|(_, t)| !t.mask.contains(signal)).map(|(&tid, _)| tid))
                }
            }
        }
        Ok(sent)
    }

    /// Take the next signal thread `tid` should act on: the first of its
    /// own unblocked pending signals in delivery order
    /// ([`next_signal`]), else the first of the process's (Poly looks at
    /// the thread's queue first; to be confirmed by the diff runner).
    pub fn dequeue(&mut self, tid: i32) -> Option<SigInfo> {
        let mask = self.thread(tid).mask;
        self.take(tid, |set| set.without(mask))
    }

    /// Take the next pending signal of `set`, blocked or not (for
    /// rt_sigtimedwait(2)).
    pub fn dequeue_from(&mut self, tid: i32, set: SigSet) -> Option<SigInfo> {
        self.take(tid, |pending| pending.intersection(set))
    }

    fn take(&mut self, tid: i32, select: impl Fn(SigSet) -> SigSet) -> Option<SigInfo> {
        let thread = self.threads.get_mut(&tid).expect("thread registered with its LinuxProcess");
        if let Some(signal) = next_signal(select(thread.pending.set())) {
            return thread.pending.take(signal);
        }
        let signal = next_signal(select(self.shared.set()))?;
        self.shared.take(signal)
    }

    /// The signals pending for thread `tid`: its own and the process's.
    pub fn pending(&self, tid: i32) -> SigSet {
        self.thread(tid).pending.set().union(self.shared.set())
    }

    /// Whether thread `tid` has a pending signal it does not block.
    pub fn has_deliverable(&self, tid: i32) -> bool {
        !self.pending(tid).without(self.thread(tid).mask).is_empty()
    }

    /// Prepare a signal the kernel raises for thread `tid` itself (a fault,
    /// or `SIGSEGV` for a bad signal frame): if the thread blocks it or
    /// ignores it, the action becomes `SIG_DFL` and the signal is unblocked,
    /// so the fault cannot be skipped (Linux kills a process that blocks or
    /// ignores `SIGSEGV` from a fault; to be confirmed by the diff runner).
    /// With `reset_handler` (the `SIGSEGV` for a frame that could not be
    /// written while handling that same signal) the handler is dropped too,
    /// so the process dies instead of looping.
    pub fn force(&mut self, tid: i32, info: SigInfo, reset_handler: bool) -> Sent {
        let signal = info.signal();
        let action = self.actions.get(signal);
        let thread = self.thread_mut(tid);
        if thread.mask.contains(signal) || action.handler == SIG_IGN || reset_handler {
            thread.mask.remove(signal);
            self.actions.set(signal, SigAction { handler: SIG_DFL, ..action });
        }
        self.thread_mut(tid).pending.push(info);
        Sent { notify: alloc::vec![tid], continue_process: false, queued: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::*;

    fn user(sig: u32) -> SigInfo {
        SigInfo::from_process(sig, SI_USER, 7, 1000)
    }

    fn handler() -> SigAction {
        SigAction { handler: 0x40_1000, flags: SA_RESTORER, restorer: 0x40_2000, mask: 0 }
    }

    fn three_threads() -> LinuxProcess {
        let mut p = LinuxProcess::new(100);
        p.add_thread(100, 101, true);
        p.add_thread(100, 102, true);
        p
    }

    #[test]
    fn process_signals_go_to_a_thread_that_does_not_block_them() {
        let mut p = three_threads();
        p.set_action(SIGUSR1, handler());
        p.thread_mut(100).mask.insert(SIGUSR1);
        let sent = p.send(user(SIGUSR1), Dest::Process, false).unwrap();
        assert_eq!(sent.notify, [101]);
        assert!(sent.queued);
        assert_eq!(p.dequeue(100), None, "blocked in 100");
        assert_eq!(p.dequeue(102).map(|i| i.signal()), Some(SIGUSR1), "any unblocked thread may take it");
        assert_eq!(p.dequeue(101), None, "taken once");
    }

    #[test]
    fn blocked_everywhere_stays_pending() {
        let mut p = three_threads();
        p.set_action(SIGUSR1, handler());
        for tid in [100, 101, 102] {
            p.thread_mut(tid).mask.insert(SIGUSR1);
        }
        let sent = p.send(user(SIGUSR1), Dest::Process, false).unwrap();
        assert!(sent.notify.is_empty() && sent.queued);
        assert!(p.pending(101).contains(SIGUSR1) && !p.has_deliverable(101));
        p.thread_mut(101).mask = SigSet::EMPTY;
        assert!(p.has_deliverable(101));
    }

    #[test]
    fn thread_signals_stay_with_their_thread() {
        let mut p = three_threads();
        p.set_action(SIGUSR2, handler());
        let sent = p.send(user(SIGUSR2), Dest::Thread(102), false).unwrap();
        assert_eq!(sent.notify, [102]);
        assert_eq!(p.dequeue(100), None);
        assert!(p.has_deliverable(102));
        assert_eq!(p.dequeue(102).map(|i| i.signal()), Some(SIGUSR2));
        assert_eq!(p.send(user(SIGUSR2), Dest::Thread(999), false), Err(Errno::ESRCH));
    }

    #[test]
    fn ignored_signals_are_discarded_at_generation() {
        let mut p = LinuxProcess::new(1);
        // SIGCHLD defaults to ignore: discarded unless blocked.
        assert!(!p.send(user(SIGCHLD), Dest::Process, false).unwrap().queued);
        p.thread_mut(1).mask.insert(SIGCHLD);
        assert!(p.send(user(SIGCHLD), Dest::Process, false).unwrap().queued, "sigwaitinfo can take it");
        // SIG_IGN: discarded even when blocked, and installing it drops
        // what is pending.
        p.set_action(SIGCHLD, SigAction { handler: SIG_IGN, ..SigAction::default() });
        assert!(p.pending(1).is_empty());
        assert!(!p.send(user(SIGCHLD), Dest::Process, false).unwrap().queued);
        p.thread_mut(1).mask.insert(SIGTERM);
        p.send(user(SIGTERM), Dest::Thread(1), false).unwrap();
        p.set_action(SIGTERM, SigAction { handler: SIG_IGN, ..SigAction::default() });
        assert!(p.pending(1).is_empty(), "blocked and pending, then ignored: discarded");
    }

    #[test]
    fn standard_signals_merge_realtime_queue_with_a_limit() {
        let mut p = LinuxProcess::new(1);
        p.set_action(SIGUSR1, handler());
        p.set_action(SIGRTMIN, handler());
        p.thread_mut(1).mask = SigSet::FULL.without(SigSet::UNBLOCKABLE);
        assert!(p.send(user(SIGUSR1), Dest::Process, false).unwrap().queued);
        assert!(!p.send(user(SIGUSR1), Dest::Process, false).unwrap().queued);
        p.queue_limit = 3;
        for _ in 0..3 {
            assert!(p.send(user(SIGRTMIN), Dest::Process, false).unwrap().queued);
        }
        assert_eq!(p.send(user(SIGRTMIN), Dest::Process, false), Err(Errno::EAGAIN));
        assert_eq!(p.send(user(SIGUSR1), Dest::Process, false).map(|s| s.queued), Ok(false), "no limit");
        let all = SigSet::FULL;
        let order: Vec<u32> = core::iter::from_fn(|| p.dequeue_from(1, all).map(|i| i.signal())).collect();
        assert_eq!(order, [SIGUSR1, SIGRTMIN, SIGRTMIN, SIGRTMIN]);
    }

    #[test]
    fn stop_and_continue_cancel_each_other() {
        let mut p = LinuxProcess::new(1);
        p.thread_mut(1).mask = SigSet::FULL.without(SigSet::UNBLOCKABLE);
        p.set_action(SIGCONT, handler());
        p.send(user(SIGTSTP), Dest::Process, false).unwrap();
        p.send(user(SIGTTIN), Dest::Thread(1), false).unwrap();
        let sent = p.send(user(SIGCONT), Dest::Process, false).unwrap();
        assert!(sent.continue_process && sent.queued);
        assert_eq!(p.pending(1), SigSet::only(SIGCONT), "stop signals discarded");
        p.send(user(SIGSTOP), Dest::Process, false).unwrap();
        assert_eq!(p.pending(1), SigSet::only(SIGSTOP), "SIGCONT discarded");
        // SIGCONT with the default action continues but queues nothing.
        p.set_action(SIGCONT, SigAction::default());
        p.thread_mut(1).mask = SigSet::EMPTY;
        let sent = p.send(user(SIGCONT), Dest::Process, false).unwrap();
        assert!(sent.continue_process && !sent.queued);
    }

    #[test]
    fn fatal_signals_notify_every_thread() {
        let mut p = three_threads();
        let sent = p.send(user(SIGTERM), Dest::Process, false).unwrap();
        assert_eq!(sent.notify, [100, 101, 102]);
        let sent = p.send(user(SIGKILL), Dest::Thread(101), false).unwrap();
        assert_eq!(sent.notify, [100, 101, 102]);
        assert!(sent.continue_process);
        p.set_action(SIGINT, handler());
        assert_eq!(p.send(user(SIGINT), Dest::Process, false).unwrap().notify, [100]);
    }

    #[test]
    fn init_only_gets_handled_signals() {
        let mut p = LinuxProcess::new(1);
        for sig in [SIGKILL, SIGTERM, SIGSTOP, SIGCONT] {
            let sent = p.send(user(sig), Dest::Process, true).unwrap();
            assert_eq!(sent, Sent::default(), "signal {sig}");
        }
        p.set_action(SIGTERM, handler());
        assert!(p.send(user(SIGTERM), Dest::Process, true).unwrap().queued);
    }

    #[test]
    fn set_action_keeps_known_flags_only() {
        let mut p = LinuxProcess::new(1);
        let a = SigAction { flags: SA_SIGINFO | SA_UNSUPPORTED | 0x0100_0000, ..handler() };
        p.set_action(SIGUSR1, a);
        assert_eq!(p.actions.get(SIGUSR1).flags, SA_SIGINFO);
    }

    #[test]
    fn fork_exec_and_threads() {
        let mut p = LinuxProcess::new(10);
        p.set_action(SIGUSR1, handler());
        p.set_action(SIGPIPE, SigAction { handler: SIG_IGN, ..SigAction::default() });
        p.thread_mut(10).mask = SigSet::only(SIGHUP);
        p.thread_mut(10).altstack = AltStack { sp: 0x1000, size: 0x4000, flags: 0 };
        p.thread_mut(10).robust_list = UserPtr(0x5000);
        p.send(user(SIGHUP), Dest::Process, false).unwrap();

        let child = p.fork(10, 20);
        assert_eq!(child.actions, p.actions);
        assert_eq!(child.thread(20).mask, SigSet::only(SIGHUP));
        assert_eq!(child.thread(20).altstack, p.thread(10).altstack);
        assert!(child.pending(20).is_empty(), "pending signals are not inherited");
        assert!(child.thread(20).robust_list.is_null());

        p.add_thread(10, 11, true);
        assert_eq!(p.thread(11).altstack, AltStack::DISABLED, "CLONE_VM without CLONE_VFORK");
        assert_eq!(p.thread(11).mask, SigSet::only(SIGHUP));
        p.add_thread(10, 12, false);
        assert_eq!(p.thread(12).altstack, p.thread(10).altstack);

        p.exec(11);
        assert_eq!(p.thread_ids().collect::<Vec<_>>(), [11]);
        assert_eq!(p.actions.get(SIGUSR1), SigAction::default(), "caught: reset");
        assert_eq!(p.actions.get(SIGPIPE).handler, SIG_IGN, "ignored: kept");
        assert!(p.pending(11).contains(SIGHUP), "pending signals survive execve");
        assert_eq!(p.thread(11).mask, SigSet::only(SIGHUP));
        assert_eq!(p.remove_thread(11).map(|t| t.mask), Some(SigSet::only(SIGHUP)));
        assert!(!p.has_thread(11));
    }

    #[test]
    fn forced_signals_cannot_be_blocked_or_ignored() {
        let mut p = LinuxProcess::new(1);
        p.thread_mut(1).mask.insert(SIGSEGV);
        let sent = p.force(1, SigInfo::fault(SIGSEGV, SEGV_MAPERR, 0), false);
        assert_eq!(sent.notify, [1]);
        assert!(!p.thread(1).mask.contains(SIGSEGV));
        p.set_action(SIGBUS, SigAction { handler: SIG_IGN, ..SigAction::default() });
        p.force(1, SigInfo::fault(SIGBUS, 2, 0), false);
        assert_eq!(p.actions.get(SIGBUS).handler, SIG_DFL);
        p.set_action(SIGILL, handler());
        p.force(1, SigInfo::fault(SIGILL, 1, 0), false);
        assert_eq!(p.actions.get(SIGILL), handler(), "a handler stays");
        p.force(1, SigInfo::fault(SIGILL, 1, 0), true);
        assert_eq!(p.actions.get(SIGILL).handler, SIG_DFL, "unless reset");
    }
}
