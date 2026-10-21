//! Acting on signals: sending them, delivering them on the way back to
//! user mode, and returning from handlers.

use super::action::{DefaultAction, Disposition, disposition};
use super::altstack::AltStack;
use super::frame::{self, FXSAVE_SIZE, Frame, Layout, RED_ZONE, UCONTEXT_SIZE};
use super::restart::{self, Decision};
use super::state::{Dest, LinuxProcess, Sent};
use crate::abi::signal::*;
use crate::abi::{SigAction, SigInfo};
use crate::cred::{Creds, cap};
use crate::errno::Errno;
use crate::kernel::{FpuState, LinuxKernel, ProcessInfo, SigSet, Target, UserContext, UserPtr};
use crate::ns::pid;
use alloc::vec;
use alloc::vec::Vec;

/// Run `f` on the calling thread's process. The kernel never fails this
/// (the caller's own process exists while it runs).
pub fn with_own<R>(k: &mut dyn LinuxKernel, f: impl FnOnce(&mut LinuxProcess) -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    k.with_process(Target::Current, &mut |p, _| {
        if let Some(f) = f.take() {
            out = Some(f(p));
        }
    })
    .expect("the calling thread's process exists");
    out.expect("with_process ran the closure")
}

/// Run `f` on the process `target` names; `ESRCH` if there is none.
pub fn with_target<R>(
    k: &mut dyn LinuxKernel,
    target: Target,
    f: impl FnOnce(&mut LinuxProcess, &ProcessInfo) -> R,
) -> Result<R, Errno> {
    let mut f = Some(f);
    let mut out = None;
    k.with_process(target, &mut |p, info| {
        if let Some(f) = f.take() {
            out = Some(f(p, info));
        }
    })?;
    out.ok_or(Errno::ESRCH)
}

/// kill(2): "For a process to have permission to send a signal, it must
/// either be privileged (under Linux: have the CAP_KILL capability in the
/// user namespace of the target process), or the real or effective user ID
/// of the sending process must equal the real or saved set-user-ID of the
/// target process. In the case of SIGCONT, it suffices when the sending and
/// receiving processes belong to the same session." The sender's and the
/// target's credentials are their threads' own (ADR 0045): the calling
/// thread's, and the target thread's or the target process's
/// ([`LinuxProcess::process_creds`]).
pub fn may_signal(sender: &Creds, sender_sid: i32, target: &Creds, target_sid: i32, signal: u32) -> bool {
    crate::ns::user::ns_capable(sender, &target.user_ns, cap::KILL)
        || [sender.uid, sender.euid].iter().any(|&id| id == target.uid || id == target.suid)
        || (signal == SIGCONT && sender_sid == target_sid)
}

/// Act on what [`LinuxProcess::send`] asked for.
pub fn complete(k: &mut dyn LinuxKernel, pid: i32, sent: &Sent) {
    if sent.queued {
        k.signal_queued(pid);
    }
    if sent.continue_process {
        k.continue_process(pid);
    }
    for &tid in &sent.notify {
        k.notify_signal(tid);
    }
}

/// Send `info` from the calling process to `target`: to the whole process,
/// or with `thread` to that thread only (then `tgid`, if given, must be its
/// process). Signal 0 checks existence and permission only. `ESRCH` if
/// the target does not exist, `EPERM` without permission, `EAGAIN` if a
/// real-time signal cannot be queued.
pub fn send(
    k: &mut dyn LinuxKernel,
    target: Target,
    thread: Option<i32>,
    tgid: Option<i32>,
    info: SigInfo,
) -> Result<(), Errno> {
    let creds = crate::cred::current(k);
    let sid = k.sid();
    let signal = info.signal();
    let sender_ns = k.pid_ns();
    let mut info = info;
    let (pid, sent) = with_target(k, target, |p, t| {
        if tgid.is_some_and(|g| g != t.pid) || thread.is_some_and(|tid| !p.has_thread(tid)) {
            return Err(Errno::ESRCH);
        }
        let root = Creds::root();
        let target_creds = match thread {
            Some(tid) => &p.thread(tid).creds,
            None => p.process_creds(t.pid).unwrap_or(&root),
        };
        if !may_signal(&creds, sid, target_creds, t.sid, signal) {
            return Err(Errno::EPERM);
        }
        if signal == 0 || t.zombie {
            return Ok((t.pid, Sent::default()));
        }
        let dest = thread.map_or(Dest::Process, Dest::Thread);
        // The sender as the receiver's pid namespace sees it (0 from
        // outside it), for the signals a process sends itself (ADR 0054).
        if matches!(info.code, SI_USER | SI_TKILL) {
            let sender = info.pid();
            info.fields[0..4].copy_from_slice(&pid::local_or_zero(&p.ns.pid, sender).to_le_bytes());
        }
        // pid_namespaces(7): an init only gets the signals it handles,
        // from its own namespace and from ancestors, but SIGKILL and
        // SIGSTOP "are forcibly delivered when sent from an ancestor PID
        // namespace".
        let init = pid::local(&p.ns.pid, t.pid) == Some(crate::task::tree::INIT_PID);
        let from_ancestor = !pid::same(&sender_ns, &p.ns.pid) && pid::is_ancestor(&sender_ns, &p.ns.pid);
        let protect = init && !(from_ancestor && matches!(signal, SIGKILL | SIGSTOP));
        p.send(info, dest, protect).map(|s| (t.pid, s))
    })??;
    complete(k, pid, &sent);
    Ok(())
}

/// Send `info` to a list of processes (a process group, or everyone):
/// success if at least one got it; otherwise `EPERM` if one existed but
/// refused, else `ESRCH` (kill(2): "EPERM The calling process does not
/// have permission to send the signal to any of the target processes";
/// "ESRCH The target process or process group does not exist").
pub fn send_to_all(k: &mut dyn LinuxKernel, pids: &[i32], info: SigInfo) -> Result<(), Errno> {
    let mut result = Err(Errno::ESRCH);
    for &pid in pids {
        match send(k, Target::Process(pid), None, None, info) {
            Ok(()) => result = Ok(()),
            Err(Errno::ESRCH) => {}
            Err(e) if result.is_err() => result = Err(e),
            Err(_) => {}
        }
    }
    result
}

/// Raise `info` for the calling thread from the kernel: a fault
/// (`SIGSEGV` for a page fault, `SIGFPE`, `SIGILL`, `SIGBUS`, `SIGTRAP`).
/// It cannot be blocked or ignored ([`LinuxProcess::force`]); it is
/// delivered by the next [`exit_to_user`].
pub fn force_signal(k: &mut dyn LinuxKernel, info: SigInfo) {
    let tid = k.tid();
    let sent = with_own(k, |p| p.force(tid, info, false));
    let pid = k.pid();
    complete(k, pid, &sent);
}

/// Record that the current system call (number `number`) was interrupted
/// with the internal restart code `errno`.
pub fn note_restart(k: &mut dyn LinuxKernel, number: u64, errno: Errno) {
    let Some((policy, with_block)) = restart::policy_of(errno) else { return };
    let tid = k.tid();
    with_own(k, |p| p.thread_mut(tid).restart = Some(restart::PendingRestart { policy, number, with_block }));
}

/// What [`exit_to_user`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitToUser {
    /// Return to user mode (maybe into a handler).
    Return,
    /// The process was killed ([`LinuxKernel::kill_process`]); the thread
    /// must not return to user mode.
    Killed,
}

/// The next thing a thread does about its pending signals.
enum Step {
    None,
    Stop(u32),
    Kill(u32, bool),
    Handle {
        info: SigInfo,
        action: SigAction,
        /// The mask the frame saves: the one before rt_sigsuspend, if that
        /// replaced it, else the current one.
        saved_mask: SigSet,
        /// The current mask, which the handler's mask is built on (POSIX,
        /// sigaction: "the union of the current signal mask and the value of
        /// the sa_mask").
        current_mask: SigSet,
        altstack: AltStack,
    },
}

/// Dequeue signals for `tid` until one needs action. Ignored ones are
/// dropped. A handled one resets to `SIG_DFL` first if it has
/// `SA_RESETHAND` (sigaction(2): "Restore the signal action to the default
/// upon entry to the signal handler"; POSIX also clears `SA_SIGINFO`).
/// Every signal taken goes into `taken`, for
/// [`LinuxKernel::signal_delivered`].
fn next_step(p: &mut LinuxProcess, tid: i32, taken: &mut Vec<u32>) -> Step {
    while let Some(info) = p.dequeue(tid) {
        let signal = info.signal();
        taken.push(signal);
        let action = p.actions.get(signal);
        match disposition(signal, &action) {
            Disposition::Ignore | Disposition::Default(DefaultAction::Ignore | DefaultAction::Continue) => {}
            Disposition::Default(DefaultAction::Stop) => return Step::Stop(signal),
            Disposition::Default(DefaultAction::Terminate) => return Step::Kill(signal, false),
            Disposition::Default(DefaultAction::Core) => return Step::Kill(signal, true),
            Disposition::Handler => {
                if action.flags & SA_RESETHAND != 0 {
                    let reset = SigAction { handler: SIG_DFL, flags: action.flags & !SA_SIGINFO, ..action };
                    p.actions.set(signal, reset);
                }
                let thread = p.thread_mut(tid);
                let saved_mask = thread.saved_mask.take().unwrap_or(thread.mask);
                return Step::Handle { info, action, saved_mask, current_mask: thread.mask, altstack: thread.altstack };
            }
        }
    }
    Step::None
}

/// Called by the kernel each time a thread of a Linux process is about to
/// return to user mode (after a system call, an interrupt or an
/// exception). Acts on the thread's pending signals:
///
/// - ignored signals are dropped; a stop signal stops the process
///   ([`LinuxKernel::stop_process`]); a fatal one kills it
///   ([`LinuxKernel::kill_process`], and this returns
///   [`ExitToUser::Killed`]);
/// - each handled signal gets a frame on the user stack and the thread is
///   pointed at its handler (a second signal's frame goes on top of the
///   first, so its handler runs first);
/// - an interrupted system call is restarted or fails with `EINTR`
///   ([`super::restart`]), decided by the first handler (or by none);
/// - the mask rt_sigsuspend replaced is restored if no handler took it.
///
/// A frame that cannot be written (bad stack, alternate stack too small, no
/// `SA_RESTORER`) raises `SIGSEGV` instead.
pub fn exit_to_user(k: &mut dyn LinuxKernel) -> ExitToUser {
    k.clear_signal_notification();
    let tid = k.tid();
    let mut pending_restart = with_own(k, |p| p.thread_mut(tid).restart.take());
    let mut taken = Vec::new();
    loop {
        let step = with_own(k, |p| next_step(p, tid, &mut taken));
        // ptrace(2): a signal-delivery-stop for "any signal except SIGKILL".
        for signal in taken.drain(..).filter(|&s| s != SIGKILL) {
            k.signal_delivered(signal);
        }
        match step {
            Step::None => {
                if let Some(r) = pending_restart.take() {
                    let mut ctx = k.user_context();
                    if restart::apply(&mut ctx, &r, None) == Decision::Restart {
                        k.set_user_context(&ctx);
                    }
                }
                with_own(k, |p| {
                    let t = p.thread_mut(tid);
                    if let Some(mask) = t.saved_mask.take() {
                        t.mask = mask;
                    }
                });
                return ExitToUser::Return;
            }
            Step::Stop(signal) => k.stop_process(signal),
            Step::Kill(signal, core) => {
                k.kill_process(signal, core);
                return ExitToUser::Killed;
            }
            Step::Handle { info, action, saved_mask, current_mask, altstack } => {
                let mut ctx = k.user_context();
                if let Some(r) = pending_restart.take() {
                    let decision = restart::apply(&mut ctx, &r, Some(action.flags));
                    if decision == Decision::Eintr {
                        with_own(k, |p| p.thread_mut(tid).restart_block = None);
                    }
                }
                let signal = info.signal();
                match setup_frame(k, &mut ctx, &info, &action, saved_mask, altstack) {
                    Some(switched) => {
                        k.set_user_context(&ctx);
                        k.reset_fpu_state();
                        with_own(k, |p| {
                            let t = p.thread_mut(tid);
                            let mut mask = current_mask.union(SigSet(action.mask));
                            if action.flags & SA_NODEFER == 0 {
                                mask.insert(signal);
                            }
                            t.mask = mask.without(SigSet::UNBLOCKABLE);
                            if switched && altstack.autodisarm() {
                                t.altstack = AltStack::DISABLED;
                            }
                        });
                    }
                    None => {
                        let segv = SigInfo::new(SIGSEGV, SI_KERNEL);
                        with_own(k, |p| {
                            p.thread_mut(tid).mask = saved_mask;
                            p.force(tid, segv, signal == SIGSEGV);
                        });
                    }
                }
            }
        }
    }
}

/// `rflags` bits for a signal handler's entry: the trap flag, the
/// direction flag (the psABI wants it clear at function entry) and the
/// resume flag are cleared.
const HANDLER_CLEARS: u64 = (1 << 8) | (1 << 10) | (1 << 16);

/// Write the frame for one handled signal to the user stack and point
/// `ctx` at the handler. Returns whether it switched to the alternate
/// stack, or `None` if the frame cannot be set up.
fn setup_frame(
    k: &mut dyn LinuxKernel,
    ctx: &mut UserContext,
    info: &SigInfo,
    action: &SigAction,
    saved_mask: SigSet,
    altstack: AltStack,
) -> Option<bool> {
    // x86-64 has no default return path: without sa_restorer the handler
    // could not return (to be confirmed by the diff runner).
    if action.flags & SA_RESTORER == 0 {
        return None;
    }
    let rsp = ctx.rsp;
    let switch = action.flags & SA_ONSTACK != 0 && altstack.is_enabled() && !altstack.on_stack(rsp);
    let on_alt = switch || altstack.on_stack(rsp);
    let top = if switch { altstack.sp.checked_add(altstack.size)? } else { rsp.checked_sub(RED_ZONE)? };
    let fpu = k.fpu_state();
    let image: Vec<u8> = fpu.image.to_vec();
    let xfeatures = fpu.xfeatures;
    let layout = Layout::below(top, image.len(), xfeatures.is_some())?;
    if on_alt && layout.frame <= altstack.sp {
        return None; // the alternate stack overflows
    }
    let frame = Frame {
        ctx,
        info,
        with_siginfo: action.flags & SA_SIGINFO != 0,
        mask: saved_mask,
        // Without an alternate stack uc_stack is all zero (rt_sigaction's
        // spec sheet), not sigaltstack(2)'s SS_DISABLE.
        stack: if altstack.is_enabled() {
            altstack.report(rsp)
        } else {
            crate::abi::StackT { sp: 0, flags: 0, size: 0 }
        },
        restorer: action.restorer,
        fpu: FpuState { image: &image, xfeatures },
    };
    let mut bytes = vec![0; layout.len()];
    frame.write(&layout, &mut bytes);
    k.copy_to_user(UserPtr(layout.frame), &bytes).ok()?;
    // The handler: handler(signo, &info, &uc) with rax = 0 (the number of
    // vector registers used by a variadic call, in case the handler was
    // declared without a prototype). rsi and rdx are set even without
    // SA_SIGINFO (to be confirmed by the diff runner).
    ctx.rip = action.handler;
    ctx.rsp = layout.frame;
    ctx.rdi = info.signal() as u64;
    ctx.rsi = layout.siginfo;
    ctx.rdx = layout.ucontext;
    ctx.rax = 0;
    ctx.rflags &= !HANDLER_CLEARS;
    Some(switch)
}

/// The `rflags` bits a signal frame may change: CF, PF, AF, ZF, SF, TF,
/// DF, OF, RF, AC, ID (what user code can set itself; IOPL, IF, NT, VM and
/// the virtual-interrupt bits stay the kernel's). To be confirmed by the
/// diff runner.
pub const USER_RFLAGS: u64 = (1 << 0)
    | (1 << 2)
    | (1 << 4)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 10)
    | (1 << 11)
    | (1 << 16)
    | (1 << 18)
    | (1 << 21);

/// An address the CPU accepts in `rip` / `rsp`: bits 63-47 all equal (48-bit
/// virtual addresses, Intel SDM vol. 1, 3.3.7.1).
pub const fn is_canonical(addr: u64) -> bool {
    ((addr as i64) << 16 >> 16) as u64 == addr
}

/// The registers rt_sigreturn may restore from `saved`, given the thread's
/// current ones; `None` if the frame is unusable:
///
/// - `rip` and `rsp` must be canonical (`iret` would fault in the kernel);
///   a canonical kernel address is let through, since the thread then
///   faults in user mode, as on Linux.
/// - `cs` must be the current (64-bit) user code segment: Poly runs no
///   32-bit code.
/// - `ss`, following x86 `asm/ucontext.h`: the saved one if it is valid
///   (the current user data segment), else, unless
///   `UC_STRICT_RESTORE_SS` is set, the flat user data segment; with the
///   flag an invalid one fails ("will fail IRET and signal").
/// - `rflags`: only [`USER_RFLAGS`] come from the frame.
pub fn restorable(saved: &frame::SavedContext, current: &UserContext) -> Option<UserContext> {
    let r = &saved.regs;
    if !is_canonical(r.rip) || !is_canonical(r.rsp) || r.cs != current.cs {
        return None;
    }
    if r.ss != current.ss && saved.flags & UC_STRICT_RESTORE_SS != 0 {
        return None;
    }
    Some(UserContext {
        rflags: (current.rflags & !USER_RFLAGS) | (r.rflags & USER_RFLAGS),
        cs: current.cs,
        ss: current.ss,
        trapno: 0,
        err: 0,
        cr2: 0,
        ..*r
    })
}

/// The largest XSAVE area rt_sigreturn reads (the kernel's own image
/// length bounds it further).
const XSTATE_MAX: usize = 64 * 1024;

/// Read the FPU state saved at `fpstate` (0: none) and load it, or reset
/// the FPU. `Err` for a bad area: misaligned (16 bytes for `FXRSTOR`, 64
/// for `XRSTOR`), unreadable, or refused by the kernel.
fn restore_fpu(k: &mut dyn LinuxKernel, fpstate: u64) -> Result<(), Errno> {
    if fpstate == 0 {
        k.reset_fpu_state();
        return Ok(());
    }
    if !fpstate.is_multiple_of(16) {
        return Err(Errno::EFAULT);
    }
    let mut legacy = [0; FXSAVE_SIZE];
    k.copy_from_user(UserPtr(fpstate), &mut legacy)?;
    let max = XSTATE_MAX.min(k.fpu_state().image.len().max(FXSAVE_SIZE));
    let xsave = k.fpu_state().xfeatures.is_some();
    match frame::xstate_size(&legacy, max).filter(|_| xsave) {
        Some(size) => {
            if !fpstate.is_multiple_of(64) {
                return Err(Errno::EFAULT);
            }
            let mut image = vec![0; size + 4];
            k.copy_from_user(UserPtr(fpstate), &mut image)?;
            if image[size..] != FP_XSTATE_MAGIC2.to_le_bytes() {
                // No end marker: only the legacy part is trustworthy.
                return k.set_fpu_state(&legacy);
            }
            image.truncate(size);
            k.set_fpu_state(&image)
        }
        None => k.set_fpu_state(&legacy),
    }
}

/// rt_sigreturn(): restore what the handler's frame saved. The `ucontext`
/// is at the user `rsp` (the handler's `ret` popped the return address).
/// Restores the registers, the mask (`SIGKILL` and `SIGSTOP` never
/// blocked), the FPU state and the alternate stack (as sigaltstack(2)
/// would set it; its errors are ignored), and cancels a pending
/// restart_syscall. Returns the restored `rax`. A bad frame raises
/// `SIGSEGV` and returns 0 (to be confirmed by the diff runner).
pub fn sigreturn(k: &mut dyn LinuxKernel) -> Result<u64, Errno> {
    let current = k.user_context();
    let tid = k.tid();
    let mut raw = [0; UCONTEXT_SIZE];
    let restored = k
        .copy_from_user(UserPtr(current.rsp), &mut raw)
        .ok()
        .map(|()| frame::decode_ucontext(&raw))
        .and_then(|saved| restorable(&saved, &current).map(|regs| (saved, regs)));
    let Some((saved, regs)) = restored else { return bad_frame(k) };
    if restore_fpu(k, saved.fpstate).is_err() {
        return bad_frame(k);
    }
    with_own(k, |p| {
        let t = p.thread_mut(tid);
        t.mask = saved.mask.without(SigSet::UNBLOCKABLE);
        t.restart = None;
        t.restart_block = None;
        t.saved_mask = None;
        if let Ok(stack) = t.altstack.change(saved.stack, regs.rsp) {
            t.altstack = stack;
        }
    });
    k.set_user_context(&regs);
    Ok(regs.rax)
}

fn bad_frame(k: &mut dyn LinuxKernel) -> Result<u64, Errno> {
    force_signal(k, SigInfo::new(SIGSEGV, SI_KERNEL));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_rules_of_kill_2() {
        let me = Creds::user(1000, 100);
        let other = Creds::user(2000, 2000);
        let ids = |r, e, s| {
            let mut c = Creds::root();
            c.set_resuid(Some(r), Some(e), Some(s)).unwrap();
            c
        };
        assert!(!may_signal(&me, 1, &other, 9, SIGTERM));
        assert!(may_signal(&me, 9, &other, 9, SIGCONT), "same session, SIGCONT");
        assert!(!may_signal(&me, 9, &other, 9, SIGTERM));
        assert!(may_signal(&me, 1, &ids(1000, 2000, 2000), 9, SIGTERM), "real uid matches");
        assert!(may_signal(&me, 1, &ids(2000, 2000, 1000), 9, SIGTERM), "saved uid matches");
        assert!(!may_signal(&me, 1, &ids(2000, 1000, 2000), 9, SIGTERM), "target euid does not count");
        assert!(may_signal(&ids(3000, 2000, 2000), 1, &other, 9, SIGTERM), "sender euid matches");
        // Privilege is CAP_KILL, not user id 0.
        assert!(may_signal(&ids(1000, 0, 0), 1, &other, 9, SIGKILL));
        let mut no_kill = Creds::root();
        no_kill.set_caps(cap_set_without_kill(), cap_set_without_kill(), Default::default()).unwrap();
        assert!(!may_signal(&no_kill, 1, &other, 9, SIGKILL), "root without CAP_KILL");
        assert!(may_signal(&no_kill, 1, &Creds::root(), 9, SIGKILL), "but same uid");
        let mut killer = Creds::user(1000, 100);
        killer.caps.effective = crate::cred::CapSet::only(cap::KILL);
        assert!(may_signal(&killer, 1, &other, 9, SIGKILL), "CAP_KILL without uid 0");
    }

    fn cap_set_without_kill() -> crate::cred::CapSet {
        crate::cred::CapSet::ALL.without(cap::KILL)
    }

    #[test]
    fn canonical_addresses() {
        assert!(is_canonical(0) && is_canonical(0x7fff_ffff_ffff) && is_canonical(0xffff_8000_0000_0000));
        assert!(!is_canonical(0x8000_0000_0000) && !is_canonical(0x00ff_0000_0000_0000));
    }
}
