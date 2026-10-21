//! Signal system calls: rt_sigaction(2), rt_sigprocmask(2), sigaltstack(2),
//! rt_sigreturn(2), kill(2), tkill / tgkill(2), rt_sigqueueinfo /
//! rt_tgsigqueueinfo(2), rt_sigpending(2), rt_sigtimedwait(2),
//! rt_sigsuspend(2), pause(2), restart_syscall(2).
//!
//! The state lives in the kernel's [`crate::signal::LinuxProcess`]; the
//! logic in [`crate::signal`]. Where the man pages list errors without an
//! order, the order here is Poly's and is to be confirmed by the diff
//! runner.

use crate::abi::signal::{self, *};
use crate::abi::{SigAction, SigInfo, StackT, Timespec};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Clock, Deadline, LinuxKernel, ProcessSelector, SigSet, Target, UserPtr, Wakeup};
use crate::ns::pid;
use crate::signal::deliver::{self, send, send_to_all, with_own};
use crate::signal::restart::RestartBlock;

/// The highest valid signal number; 0 is valid where a call allows a
/// check without sending.
fn signal_number(raw: i32, allow_zero: bool) -> Result<u32, Errno> {
    match raw {
        0 if allow_zero => Ok(0),
        1..=64 => Ok(raw as u32),
        _ => Err(Errno::EINVAL),
    }
}

/// Read a `sigset_t` of `sigsetsize` bytes (only 8 is accepted).
fn read_sigset(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<SigSet, Errno> {
    let mut raw = [0; 8];
    k.copy_from_user(ptr, &mut raw)?;
    Ok(SigSet(u64::from_le_bytes(raw)))
}

/// rt_sigaction(signum, act, oldact, sigsetsize)
///
/// sigaction(2): `EINVAL` if `sigsetsize` is not `sizeof(sigset_t)` (8),
/// if the signal is not 1-64, or if `act` tries to change `SIGKILL` or
/// `SIGSTOP`; `EFAULT` if `act` cannot be read or `oldact` written. With
/// `act` null the action is only reported. `oldact` is written before the
/// new action is stored, so an `EFAULT` changes nothing. Flags Poly does
/// not know are dropped when the action is stored, so `oldact` never
/// reports them ("Dynamically probing for flag bit support"), and
/// installing an ignoring action discards pending instances
/// ([`crate::signal::LinuxProcess::set_action`]).
pub fn rt_sigaction(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let act = a.ptr(1);
    let oldact = a.ptr(2);
    if a.ulong(3) != signal::SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let signum = signal_number(a.int(0), false)?;
    let new = if act.is_null() {
        None
    } else {
        if signum == SIGKILL || signum == SIGSTOP {
            return Err(Errno::EINVAL);
        }
        let mut raw = [0; SigAction::SIZE];
        k.copy_from_user(act, &mut raw)?;
        let mut new = SigAction::from_bytes(&raw);
        // SIGKILL and SIGSTOP are silently left out of sa_mask
        // (rt_sigaction's spec sheet: it reads back without them).
        new.mask = SigSet(new.mask).without(SigSet::UNBLOCKABLE).0;
        Some(new)
    };
    let old = with_own(k, |p| p.actions.get(signum));
    if !oldact.is_null() {
        k.copy_to_user(oldact, &old.to_bytes())?;
    }
    if let Some(new) = new {
        with_own(k, |p| p.set_action(signum, new));
    }
    Ok(0)
}

/// rt_sigprocmask(how, set, oldset, sigsetsize)
///
/// sigprocmask(2): `EINVAL` if `sigsetsize` is not 8 or, when `set` is not
/// null, `how` is not `SIG_BLOCK`, `SIG_UNBLOCK` or `SIG_SETMASK` ("if set
/// is NULL ... how is ignored"); `EFAULT` for unreadable `set` or
/// unwritable `oldset`. "It is not possible to block SIGKILL or SIGSTOP.
/// Attempts to do so are silently ignored." Signals the new mask unblocks
/// are delivered on the way back to user mode.
pub fn rt_sigprocmask(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let how = a.uint(0);
    let set = a.ptr(1);
    let oldset = a.ptr(2);
    if a.ulong(3) != signal::SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let change = if set.is_null() {
        None
    } else {
        if !matches!(how, SIG_BLOCK | SIG_UNBLOCK | SIG_SETMASK) {
            return Err(Errno::EINVAL);
        }
        Some(read_sigset(k, set)?)
    };
    let tid = k.tid();
    let old = with_own(k, |p| p.thread(tid).mask);
    if !oldset.is_null() {
        k.copy_to_user(oldset, &old.0.to_le_bytes())?;
    }
    if let Some(bits) = change {
        let new = match how {
            SIG_BLOCK => old.union(bits),
            SIG_UNBLOCK => old.without(bits),
            _ => bits,
        };
        with_own(k, |p| p.thread_mut(tid).mask = new.without(SigSet::UNBLOCKABLE));
    }
    Ok(0)
}

/// sigaltstack(ss, old_ss)
///
/// sigaltstack(2): `old_ss` (if not null) gets the current stack, with
/// `SS_ONSTACK` while the thread runs on it and `SS_DISABLE` when there is
/// none; `ss` (if not null) installs a new one, with the rules of
/// [`crate::signal::AltStack::change`] (`EPERM`, `EINVAL`, `ENOMEM`);
/// `EFAULT` if either cannot be accessed. The new stack is validated
/// before `old_ss` is written, and stored only after that write succeeded.
pub fn sigaltstack(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let ss = a.ptr(0);
    let old_ss = a.ptr(1);
    let rsp = k.user_context().rsp;
    let tid = k.tid();
    let current = with_own(k, |p| p.thread(tid).altstack);
    let new = if ss.is_null() {
        None
    } else {
        let mut raw = [0; StackT::SIZE];
        k.copy_from_user(ss, &mut raw)?;
        Some(current.change(StackT::from_bytes(&raw), rsp)?)
    };
    if !old_ss.is_null() {
        k.copy_to_user(old_ss, &current.report(rsp).to_bytes())?;
    }
    if let Some(stack) = new {
        with_own(k, |p| p.thread_mut(tid).altstack = stack);
    }
    Ok(0)
}

/// rt_sigreturn(): see [`deliver::sigreturn`].
pub fn rt_sigreturn(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    deliver::sigreturn(k)
}

/// The siginfo of a signal sent by the calling process.
fn sender_info(k: &dyn LinuxKernel, signal: u32, code: i32) -> SigInfo {
    SigInfo::from_process(signal, code, k.pid(), k.credentials().uid)
}

/// kill(pid, sig)
///
/// kill(2): `pid > 0` signals that process; `0` every process in the
/// caller's process group; `-1` "every process for which the calling
/// process has permission to send signals, except for process 1 (init)",
/// and on Linux not the caller itself; `< -1` every process in group
/// `-pid`. `sig` 0 checks existence and permission only. `EINVAL` for an
/// invalid signal, `EPERM` / `ESRCH` as [`deliver::send_to_all`] decides
/// for groups. `pid` = `INT_MIN` names no group and gets `ESRCH` (to be
/// confirmed by the diff runner). The siginfo has `SI_USER`, the
/// caller's pid and real uid.
pub fn kill(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let pid = a.int(0);
    // The target is looked up before the signal number is checked: a bad
    // number for a missing target is ESRCH (kill's visible test). Ids are
    // the caller's pid namespace's (ADR 0054); -1 means every process it
    // can see but its own init and itself.
    let pids = match pid {
        p if p > 0 => {
            let pid = pid::find(k, p)?;
            deliver::with_target(k, Target::Process(pid), |_, _| ())?;
            alloc::vec![pid]
        }
        0 => k.process_ids(ProcessSelector::Group(k.pgid())),
        -1 => {
            let me = k.pid();
            let mut pids = k.process_ids(ProcessSelector::All);
            pids.retain(|&p| p != me && !matches!(pid::view(k, p), 0 | 1));
            pids
        }
        i32::MIN => return Err(Errno::ESRCH),
        p => k.process_ids(ProcessSelector::Group(pid::find(k, -p)?)),
    };
    if pids.is_empty() {
        return Err(Errno::ESRCH);
    }
    let sig = signal_number(a.int(1), true)?;
    let info = sender_info(k, sig, SI_USER);
    match pid {
        p if p > 0 => send(k, Target::Process(pids[0]), None, None, info)?,
        _ => send_to_all(k, &pids, info)?,
    }
    Ok(0)
}

/// tgkill(tgid, tid, sig): signal thread `tid` of process `tgid`.
/// tgkill(2): `EINVAL` "An invalid thread ID, thread group ID, or signal
/// was specified" (ids must be positive), `ESRCH` "No process with the
/// specified thread ID (and thread group ID) exists", `EPERM` as for
/// kill(2). The siginfo has `SI_TKILL`.
pub fn tgkill(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (tgid, tid) = (a.int(0), a.int(1));
    let sig = signal_number(a.int(2), true)?;
    if tgid <= 0 || tid <= 0 {
        return Err(Errno::EINVAL);
    }
    let info = sender_info(k, sig, SI_TKILL);
    let (tgid, tid) = (pid::find(k, tgid)?, pid::find(k, tid)?);
    send(k, Target::Thread(tid), Some(tid), Some(tgid), info)?;
    Ok(0)
}

/// tkill(tid, sig): tgkill without the process check (tgkill(2): "tkill()
/// is an obsolete predecessor to tgkill()").
pub fn tkill(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tid = a.int(0);
    let sig = signal_number(a.int(1), true)?;
    if tid <= 0 {
        return Err(Errno::EINVAL);
    }
    let info = sender_info(k, sig, SI_TKILL);
    let tid = pid::find(k, tid)?;
    send(k, Target::Thread(tid), Some(tid), None, info)?;
    Ok(0)
}

/// Read the caller's siginfo for rt_sigqueueinfo / rt_tgsigqueueinfo.
/// rt_sigqueueinfo(2): the caller may not claim to be the kernel or tkill
/// when signalling another process: `EPERM` if `si_code` is `SI_TKILL` or
/// not negative and the target is not the caller's own process.
/// `si_signo` is set to `sig` (to be confirmed by the diff runner).
/// The bytes of a caller's `siginfo_t` rt_sigqueueinfo(2) takes; the rest
/// must be zero (`E2BIG`).
const QUEUED_INFO_BYTES: usize = 48;

fn queued_info(k: &mut dyn LinuxKernel, ptr: UserPtr, sig: u32, tgid: i32) -> Result<SigInfo, Errno> {
    let mut raw = [0; SigInfo::SIZE];
    k.copy_from_user(ptr, &mut raw)?;
    // Only the first 48 bytes may carry anything; nothing is sent
    // otherwise (rt_sigqueueinfo's spec sheet).
    if raw[QUEUED_INFO_BYTES..].iter().any(|&b| b != 0) {
        return Err(Errno::E2BIG);
    }
    let mut info = SigInfo::from_bytes(&raw);
    if (info.code >= 0 || info.code == SI_TKILL) && tgid != pid::view(k, k.pid()) {
        return Err(Errno::EPERM);
    }
    info.signo = sig as i32;
    Ok(info)
}

/// rt_sigqueueinfo(tgid, sig, info): send `sig` with the caller's siginfo
/// to process `tgid`. Errors as for kill(2) with `pid > 0`, `EFAULT` for
/// `info`, `EAGAIN` when a real-time signal cannot be queued.
pub fn rt_sigqueueinfo(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tgid = a.int(0);
    let sig = signal_number(a.int(1), true)?;
    let info = queued_info(k, a.ptr(2), sig, tgid)?;
    let target = Target::Process(pid::find(k, tgid)?);
    send(k, target, None, None, info)?;
    Ok(0)
}

/// rt_tgsigqueueinfo(tgid, tid, sig, info): the same for one thread, with
/// tgkill(2)'s rules for the ids.
pub fn rt_tgsigqueueinfo(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (tgid, tid) = (a.int(0), a.int(1));
    let sig = signal_number(a.int(2), true)?;
    if tgid <= 0 || tid <= 0 {
        return Err(Errno::EINVAL);
    }
    let info = queued_info(k, a.ptr(3), sig, tgid)?;
    let (tgid, tid) = (pid::find(k, tgid)?, pid::find(k, tid)?);
    send(k, Target::Thread(tid), Some(tid), Some(tgid), info)?;
    Ok(0)
}

/// rt_sigpending(set, sigsetsize): sigpending(2): "returns the set of
/// signals that are pending for delivery to the calling thread (i.e., the
/// signals which have been raised while blocked)": the thread's and the
/// process's pending signals that the thread blocks. `EINVAL` unless
/// `sigsetsize` is 8 (to be confirmed by the diff runner), `EFAULT`.
pub fn rt_sigpending(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    // rt_sigpending's spec sheet: a smaller size writes only that many
    // bytes (0 writes nothing); above 8 is EINVAL.
    let size = a.ulong(1);
    if size > signal::SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let tid = k.tid();
    let set = with_own(k, |p| p.pending(tid).intersection(p.thread(tid).mask));
    if size > 0 {
        k.copy_to_user(a.ptr(0), &set.0.to_le_bytes()[..size as usize])?;
    }
    Ok(0)
}

/// rt_sigsuspend(mask, sigsetsize): sigsuspend(2) "temporarily replaces
/// the signal mask of the calling thread with the mask given by mask and
/// then suspends the thread until delivery of a signal whose action is to
/// invoke a signal handler or to terminate a process"; it "always returns
/// -1, with errno set to indicate the error (normally, EINTR)". `SIGKILL`
/// and `SIGSTOP` cannot be blocked. The old mask comes back when the
/// handler returns (it is the one its frame saves). If the signal turns
/// out not to need a handler (a stop and `SIGCONT`), the call starts over.
pub fn rt_sigsuspend(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ulong(1) != signal::SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let mask = read_sigset(k, a.ptr(0))?.without(SigSet::UNBLOCKABLE);
    let tid = k.tid();
    with_own(k, |p| {
        let t = p.thread_mut(tid);
        t.saved_mask.get_or_insert(t.mask);
        t.mask = mask;
    });
    loop {
        k.clear_signal_notification();
        if with_own(k, |p| p.has_deliverable(tid)) {
            return Err(Errno::RESTART_IF_NO_HANDLER);
        }
        k.block(None);
    }
}

/// pause(): pause(2) "causes the calling process (or thread) to sleep
/// until a signal is delivered that either terminates the process or
/// causes the invocation of a signal-catching function"; it "returns only
/// when a signal was caught and the signal-catching function returned",
/// with `EINTR`. No arguments are read and the mask is left as it is, so
/// blocked signals stay pending without waking it. A signal that needs no
/// handler (a stop and `SIGCONT`) starts the call over, and `SA_RESTART`
/// does not restart it once a handler ran.
pub fn pause(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    let tid = k.tid();
    loop {
        k.clear_signal_notification();
        if with_own(k, |p| p.has_deliverable(tid)) {
            return Err(Errno::RESTART_IF_NO_HANDLER);
        }
        k.block(None);
    }
}

/// rt_sigtimedwait(set, info, timeout, sigsetsize)
///
/// sigwaitinfo(2): "suspends execution of the calling thread until one of
/// the signals in set is pending ... (If one of the signals in set is
/// already pending for the calling thread, sigwaitinfo() will return
/// immediately.)" It takes that signal off the pending set, writes its
/// siginfo to `info` (if not null) and returns its number. `timeout` (if
/// not null) bounds the wait, measured on `CLOCK_MONOTONIC`; zero polls.
/// Errors: `EINVAL` for a `sigsetsize` other than 8 or an invalid
/// `timeout`; `EFAULT`; `EAGAIN` "No signal in set became pending within
/// the timeout period"; `EINTR` "The wait was interrupted by a signal
/// handler" (signal(7) lists these calls as never restarted, also after a
/// stop). `SIGKILL` and `SIGSTOP` in `set` are ignored. While it waits the
/// signals of `set` are unblocked for the thread (so their arrival wakes
/// it); the mask is the old one again before it returns.
pub fn rt_sigtimedwait(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if a.ulong(3) != signal::SIGSET_SIZE {
        return Err(Errno::EINVAL);
    }
    let set = read_sigset(k, a.ptr(0))?.without(SigSet::UNBLOCKABLE);
    let timeout = if a.ptr(2).is_null() {
        None
    } else {
        let mut raw = [0; Timespec::SIZE];
        k.copy_from_user(a.ptr(2), &mut raw)?;
        let t = Timespec::from_bytes(&raw);
        if !t.is_valid() {
            return Err(Errno::EINVAL);
        }
        Some(t)
    };
    let tid = k.tid();
    let got = match with_own(k, |p| take_from(p, tid, set)) {
        Some(info) => Ok(info),
        None if timeout.is_some_and(Timespec::is_zero) => Err(Errno::EAGAIN),
        None => {
            let deadline = match timeout {
                Some(t) => Some(Deadline { clock: Clock::Monotonic, at: k.clock(Clock::Monotonic)?.saturating_add(t) }),
                None => None,
            };
            let real_mask = with_own(k, |p| {
                let t = p.thread_mut(tid);
                let real = t.mask;
                t.mask = real.without(set);
                t.sigwait = set;
                real
            });
            let got = wait_for_signal(k, tid, set, deadline);
            with_own(k, |p| {
                let t = p.thread_mut(tid);
                t.mask = real_mask;
                t.sigwait = SigSet::default();
            });
            got
        }
    }?;
    let (mut got, timer) = got;
    if let Some(t) = timer {
        crate::sys::time::count_taken(k, t, Some(&mut got));
    }
    if !a.ptr(1).is_null() {
        k.copy_to_user(a.ptr(1), &got.to_bytes())?;
    }
    Ok(got.signal() as u64)
}

/// Take the next pending signal of `set` for `tid`, and whether it was a
/// POSIX timer's (whose overrun count is fixed next:
/// [`crate::sys::time::count_taken`]).
fn take_from(
    p: &mut crate::signal::LinuxProcess,
    tid: i32,
    set: SigSet,
) -> Option<(SigInfo, Option<crate::time::timers::TakenTimer>)> {
    let info = p.dequeue_from(tid, set)?;
    let timer = crate::time::timers::signal_taken(p, &info);
    Some((info, timer))
}

fn wait_for_signal(
    k: &mut dyn LinuxKernel,
    tid: i32,
    set: SigSet,
    deadline: Option<Deadline>,
) -> Result<(SigInfo, Option<crate::time::timers::TakenTimer>), Errno> {
    loop {
        k.clear_signal_notification();
        let state = with_own(k, |p| match take_from(p, tid, set) {
            Some(info) => Some(Ok(info)),
            None if p.has_deliverable(tid) => Some(Err(Errno::EINTR)),
            None => None,
        });
        if let Some(result) = state {
            return result;
        }
        if k.block(deadline) == Wakeup::TimedOut {
            return Err(Errno::EAGAIN);
        }
    }
}

/// restart_syscall(): continue the call a signal interrupted, from the
/// state it saved (restart_syscall(2)). Without saved state: `EINTR`.
pub fn restart_syscall(k: &mut dyn LinuxKernel, _: &Args) -> SysResult {
    let tid = k.tid();
    match with_own(k, |p| p.thread_mut(tid).restart_block.take()) {
        None => Err(Errno::EINTR),
        Some(RestartBlock::FutexWait { addr, private, val, bitset, deadline }) => {
            crate::sys::futex::wait_with_restart(k, UserPtr(addr), private, val, bitset, Some(deadline), true)
        }
        Some(RestartBlock::Sleep { deadline, rem }) => crate::sys::time::sleep(k, deadline, Some(UserPtr(rem))),
    }
}
