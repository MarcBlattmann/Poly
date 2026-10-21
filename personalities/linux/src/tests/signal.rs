//! Signals end to end against the fake kernel: generation (kill, tgkill,
//! tkill, rt_sigqueueinfo), delivery on the way back to user mode
//! (order, masks, default actions, frames, alternate stacks), rt_sigreturn,
//! rt_sigpending, rt_sigsuspend, rt_sigtimedwait, and restarts.

extern crate std;

use super::fake::*;
use crate::abi::signal::*;
use crate::abi::{SigAction, SigInfo, StackT, Timespec, le_u64};
use crate::errno::Errno;
use crate::kernel::{SigSet, UserContext};
use crate::signal::frame::{self, FRAME_SIGINFO, FRAME_UCONTEXT, Layout, RED_ZONE, SavedContext, UCONTEXT_SIZE};
use crate::signal::{AltStack, ExitToUser};
use crate::{Linux, nr};

const HANDLER: u64 = 0x40_5000;
const RESTORER: u64 = 0x40_6000;
/// Where tests keep a `struct sigaction` and other arguments.
const ARG: u64 = DATA + 0x800;

fn set(sigs: &[u32]) -> SigSet {
    sigs.iter().fold(SigSet::EMPTY, |s, &x| s.union(SigSet::only(x)))
}

/// Install a handler for `sig` with `flags` (plus SA_RESTORER) and `mask`.
fn install(k: &mut FakeKernel, sig: u32, flags: u64, mask: SigSet) {
    let act = SigAction { handler: HANDLER + sig as u64, flags: flags | SA_RESTORER, restorer: RESTORER, mask: mask.0 };
    k.poke(ARG, &act.to_bytes());
    assert_eq!(sys(k, nr::RT_SIGACTION, &[sig as u64, ARG, 0, 8]), Ok(0));
}

fn set_mask(k: &mut FakeKernel, mask: SigSet) -> (ExitToUser, UserContext) {
    k.poke(ARG, &mask.0.to_le_bytes());
    run(k, nr::RT_SIGPROCMASK, &[SIG_SETMASK as u64, ARG, 0, 8])
}

/// The handler returns: `ret` pops the return address and the restorer
/// calls rt_sigreturn.
fn handler_returns(k: &mut FakeKernel) -> (ExitToUser, UserContext) {
    assert_eq!(k.peek_u64(k.ctx.rsp), RESTORER, "the handler returns into sa_restorer");
    k.ctx.rsp += 8;
    k.ctx.rip = RESTORER + 9;
    run(k, nr::RT_SIGRETURN, &[])
}

/// The signal the thread is about to handle (its handler is running).
fn handling(k: &FakeKernel) -> Option<u32> {
    (k.ctx.rip > HANDLER && k.ctx.rip <= HANDLER + 64).then(|| (k.ctx.rip - HANDLER) as u32)
}

fn frame_info(k: &FakeKernel) -> SigInfo {
    SigInfo::from_bytes(&k.peek(k.ctx.rsi, SigInfo::SIZE).try_into().unwrap())
}

fn frame_uc(k: &FakeKernel) -> SavedContext {
    frame::decode_ucontext(&k.peek(k.ctx.rdx, UCONTEXT_SIZE).try_into().unwrap())
}

#[test]
fn a_handled_signal_gets_a_frame_and_returns() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_SIGINFO, SigSet::EMPTY);
    let start = k.ctx;
    let (exit, ctx) = run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert_eq!(exit, ExitToUser::Return);
    // The handler's registers.
    let layout = Layout::below(start.rsp - RED_ZONE, 512, false).unwrap();
    assert_eq!(ctx.rip, HANDLER + SIGUSR1 as u64);
    assert_eq!(ctx.rsp, layout.frame);
    assert_eq!((ctx.rsp + 8) % 16, 0);
    assert_eq!((ctx.rdi, ctx.rsi, ctx.rdx, ctx.rax), (10, layout.frame + 312, layout.frame + 8, 0));
    assert_eq!(ctx.rsi - ctx.rsp, FRAME_SIGINFO as u64);
    assert_eq!(ctx.rdx - ctx.rsp, FRAME_UCONTEXT as u64);
    assert_eq!(k.fpu_resets, 1, "a clean FPU for the handler");
    // The siginfo kill(2) sends.
    let info = frame_info(&k);
    assert_eq!((info.signo, info.code, info.pid(), info.uid()), (10, SI_USER, 100, 1000));
    // The saved context: the state after kill returned 0.
    let uc = frame_uc(&k);
    let interrupted =
        UserContext { rax: 0, rdi: 100, rsi: SIGUSR1 as u64, rcx: SYSCALL_RIP, r11: start.rflags, ..start };
    assert_eq!(uc.regs, interrupted);
    assert_eq!(uc.mask, SigSet::EMPTY);
    assert_eq!(uc.fpstate, layout.fpstate);
    assert_eq!(k.peek(layout.fpstate, 464), k.fpu[..464], "FPU state saved");
    // While the handler runs, its signal is blocked.
    assert_eq!(k.mask(), SigSet::only(SIGUSR1));

    let (exit, ctx) = handler_returns(&mut k);
    assert_eq!(exit, ExitToUser::Return);
    assert_eq!(ctx, interrupted, "every register comes back");
    assert_eq!(k.mask(), SigSet::EMPTY);
    assert_eq!(k.fpu_loads.last().map(|i| i.len()), Some(512));
}

#[test]
fn xsave_state_round_trips_through_the_frame() {
    let mut k = FakeKernel::new();
    k.fpu = (0..832u32).map(|i| (i * 7) as u8).collect();
    k.xfeatures = Some(0x7);
    install(&mut k, SIGUSR2, 0, SigSet::EMPTY);
    run(&mut k, nr::KILL, &[100, SIGUSR2 as u64]);
    let uc = frame_uc(&k);
    assert_eq!(uc.flags, UC_FP_XSTATE | UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS);
    assert_eq!(uc.fpstate % 64, 0);
    assert_eq!(frame_info(&k), SigInfo::from_bytes(&[0; 128]), "no SA_SIGINFO: no siginfo copied");
    handler_returns(&mut k);
    let loaded = k.fpu_loads.last().unwrap();
    assert_eq!(loaded.len(), 832);
    assert_eq!(loaded[..464], k.fpu[..464]);
    assert_eq!(loaded[512..], k.fpu[512..]);
}

#[test]
fn delivery_order_follows_signal_7() {
    let mut k = FakeKernel::new();
    let all = SigSet::FULL.without(SigSet::UNBLOCKABLE);
    for sig in [SIGUSR1, SIGUSR2, SIGSEGV, SIGRTMIN, SIGRTMIN + 1] {
        install(&mut k, sig, SA_SIGINFO, all);
    }
    set_mask(&mut k, all);
    let queue = |k: &mut FakeKernel, sig: u32, value: u64| {
        let mut info = SigInfo::from_process(sig, SI_QUEUE, 100, 1000);
        info.fields[8..16].copy_from_slice(&value.to_le_bytes());
        k.poke(ARG, &info.to_bytes());
        assert_eq!(sys(k, nr::RT_SIGQUEUEINFO, &[100, sig as u64, ARG]), Ok(0));
    };
    queue(&mut k, SIGRTMIN + 1, 1);
    queue(&mut k, SIGRTMIN + 1, 2);
    queue(&mut k, SIGRTMIN, 3);
    queue(&mut k, SIGUSR2, 4);
    queue(&mut k, SIGUSR1, 5);
    queue(&mut k, SIGUSR1, 6);
    queue(&mut k, SIGSEGV, 7);
    k.poke(ARG, &[0; 8]);
    assert_eq!(sys(&mut k, nr::RT_SIGPENDING, &[ARG, 8]), Ok(0));
    assert_eq!(k.peek_u64(ARG), set(&[SIGUSR1, SIGUSR2, SIGSEGV, SIGRTMIN, SIGRTMIN + 1]).0);

    // Unblock everything; each handler blocks all signals, so they run one
    // after the other.
    set_mask(&mut k, SigSet::EMPTY);
    let mut seen = std::vec::Vec::new();
    while let Some(sig) = handling(&k) {
        seen.push((sig, frame_info(&k).value()));
        assert_eq!(k.mask(), all, "sa_mask applies in the handler");
        handler_returns(&mut k);
    }
    assert_eq!(
        seen,
        [(SIGSEGV, 7), (SIGUSR1, 5), (SIGUSR2, 4), (SIGRTMIN, 3), (SIGRTMIN + 1, 1), (SIGRTMIN + 1, 2)],
        "synchronous first, standard (merged, first siginfo) before real-time, RT lowest first and in order"
    );
    assert_eq!(k.mask(), SigSet::EMPTY);
}

#[test]
fn handlers_nest_when_their_mask_allows() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    install(&mut k, SIGUSR2, 0, SigSet::EMPTY);
    set_mask(&mut k, set(&[SIGUSR1, SIGUSR2]));
    sys(&mut k, nr::KILL, &[100, SIGUSR2 as u64]).unwrap();
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    set_mask(&mut k, SigSet::EMPTY);
    // SIGUSR1 is delivered first; SIGUSR2's frame goes on top of it, so its
    // handler runs first and returns into SIGUSR1's.
    assert_eq!(handling(&k), Some(SIGUSR2));
    assert_eq!(k.mask(), set(&[SIGUSR1, SIGUSR2]));
    assert_eq!(frame_uc(&k).regs.rip, HANDLER + SIGUSR1 as u64);
    handler_returns(&mut k);
    assert_eq!(handling(&k), Some(SIGUSR1));
    assert_eq!(k.mask(), SigSet::only(SIGUSR1));
    let (_, ctx) = handler_returns(&mut k);
    assert_eq!(handling(&k), None);
    assert_eq!(ctx.rip, SYSCALL_RIP);
    assert_eq!(k.mask(), SigSet::EMPTY);
}

#[test]
fn blocked_signals_wait_until_unblocked() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGINT, 0, SigSet::EMPTY);
    set_mask(&mut k, SigSet::only(SIGINT));
    let (_, ctx) = run(&mut k, nr::KILL, &[100, SIGINT as u64]);
    assert_eq!(ctx.rip, SYSCALL_RIP, "blocked: no handler yet");
    assert!(k.me().pending(100).contains(SIGINT));
    set_mask(&mut k, SigSet::EMPTY);
    assert_eq!(handling(&k), Some(SIGINT));
    assert_eq!(frame_uc(&k).mask, SigSet::EMPTY, "the frame saves the mask after rt_sigprocmask");
}

#[test]
fn default_actions() {
    let mut k = FakeKernel::new();
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGCHLD as u64]).0, ExitToUser::Return, "ignored by default");
    assert!(k.me().pending(100).is_empty());
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGTSTP as u64]).0, ExitToUser::Return);
    assert_eq!(k.stops, [SIGTSTP], "stopped, then continued");
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGTERM as u64]).0, ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGTERM, false)));
    let mut k = FakeKernel::new();
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGQUIT as u64]).0, ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGQUIT, true)), "Core");
    let mut k = FakeKernel::new();
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGRTMIN as u64 + 5]).0, ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGRTMIN + 5, false)), "RT signals terminate");
}

#[test]
fn sigkill_and_sigstop_cannot_be_blocked_or_caught() {
    let mut k = FakeKernel::new();
    set_mask(&mut k, SigSet::FULL);
    assert_eq!(k.mask(), SigSet::FULL.without(SigSet::UNBLOCKABLE));
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGSTOP as u64]).0, ExitToUser::Return);
    assert_eq!(k.stops, [SIGSTOP]);
    k.poke(ARG, &SigAction { handler: SIG_IGN, ..Default::default() }.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[SIGKILL as u64, ARG, 0, 8]), Err(Errno::EINVAL));
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGKILL as u64]).0, ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGKILL, false)));
}

#[test]
fn sa_resethand_and_sa_nodefer() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESETHAND | SA_NODEFER | SA_SIGINFO, SigSet::only(SIGHUP));
    run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert_eq!(handling(&k), Some(SIGUSR1));
    assert_eq!(k.mask(), SigSet::only(SIGHUP), "SA_NODEFER: the signal itself stays unblocked");
    let now = k.actions().get(SIGUSR1);
    assert_eq!(now.handler, SIG_DFL, "SA_RESETHAND");
    assert_eq!(now.flags & SA_SIGINFO, 0);
    // A second SIGUSR1 inside the handler now kills.
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).0, ExitToUser::Killed);
}

#[test]
fn sigaltstack_and_sa_onstack() {
    let mut k = FakeKernel::new();
    let alt = DATA + 0x4000;
    let size = 0x3000u64;
    k.poke(ARG, &StackT { sp: alt, flags: 0, size }.to_bytes());
    assert_eq!(sys(&mut k, nr::SIGALTSTACK, &[ARG, 0]), Ok(0));
    install(&mut k, SIGSEGV, SA_ONSTACK | SA_SIGINFO, SigSet::EMPTY);
    install(&mut k, SIGUSR1, SA_ONSTACK, SigSet::EMPTY);
    let before = k.ctx;
    run(&mut k, nr::KILL, &[100, SIGSEGV as u64]);
    assert!(k.ctx.rsp > alt && k.ctx.rsp < alt + size, "the frame is on the alternate stack");
    let layout = Layout::below(alt + size, 512, false).unwrap();
    assert_eq!(k.ctx.rsp, layout.frame, "no red zone when switching stacks");
    assert_eq!(frame_uc(&k).stack, StackT { sp: alt, flags: 0, size }, "uc_stack: not on it when interrupted");
    // On the alternate stack: it cannot be changed, and old_ss says so.
    k.poke(ARG, &StackT { sp: DATA, flags: 0, size: 0x4000 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SIGALTSTACK, &[ARG, 0]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::SIGALTSTACK, &[0, ARG + 0x40]), Ok(0));
    assert_eq!(StackT::from_bytes(&k.peek(ARG + 0x40, 24).try_into().unwrap()).flags, SS_ONSTACK);
    // A nested signal stays on the alternate stack, below the first frame.
    let first = k.ctx.rsp;
    run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert!(k.ctx.rsp < first - RED_ZONE && k.ctx.rsp > alt);
    assert_eq!(frame_uc(&k).stack.flags, SS_ONSTACK);
    handler_returns(&mut k);
    let (_, ctx) = handler_returns(&mut k);
    assert_eq!(ctx.rsp, before.rsp, "back on the normal stack");
    assert_eq!(k.thread_state().altstack, AltStack { sp: alt, size, flags: 0 });
}

#[test]
fn ss_autodisarm_clears_the_stack_while_the_handler_runs() {
    let mut k = FakeKernel::new();
    let alt = DATA + 0x4000;
    k.poke(ARG, &StackT { sp: alt, flags: SS_AUTODISARM, size: 0x3000 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SIGALTSTACK, &[ARG, 0]), Ok(0));
    install(&mut k, SIGUSR1, SA_ONSTACK, SigSet::EMPTY);
    run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert!(k.ctx.rsp > alt);
    assert!(!k.thread_state().altstack.is_enabled(), "disarmed");
    assert_eq!(frame_uc(&k).stack.flags, SS_AUTODISARM);
    // The handler may install another stack now (e.g. before swapcontext).
    assert_eq!(sys(&mut k, nr::SIGALTSTACK, &[0, ARG + 0x40]), Ok(0));
    assert_eq!(StackT::from_bytes(&k.peek(ARG + 0x40, 24).try_into().unwrap()).flags, SS_DISABLE);
    handler_returns(&mut k);
    assert_eq!(k.thread_state().altstack, AltStack { sp: alt, size: 0x3000, flags: SS_AUTODISARM }, "restored");
}

#[test]
fn a_frame_that_does_not_fit_raises_sigsegv() {
    let mut k = FakeKernel::new();
    k.fpu = std::vec![0; 2696];
    k.xfeatures = Some(0x2e7);
    let alt = DATA + 0x4000;
    k.poke(ARG, &StackT { sp: alt, flags: 0, size: MINSIGSTKSZ }.to_bytes());
    assert_eq!(sys(&mut k, nr::SIGALTSTACK, &[ARG, 0]), Ok(0));
    install(&mut k, SIGUSR1, SA_ONSTACK, SigSet::EMPTY);
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).0, ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSEGV, true)));
    // An unwritable stack, and a handler without SA_RESTORER, too.
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    k.ctx.rsp = UNMAPPED;
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).0, ExitToUser::Killed);
    let mut k = FakeKernel::new();
    k.poke(ARG, &SigAction { handler: HANDLER, ..Default::default() }.to_bytes());
    sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, ARG, 0, 8]).unwrap();
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).0, ExitToUser::Killed);
    // A SIGSEGV handler still runs for the SIGSEGV that follows another
    // signal's failed frame.
    let mut k = FakeKernel::new();
    install(&mut k, SIGSEGV, SA_SIGINFO, SigSet::EMPTY);
    k.poke(ARG, &SigAction { handler: HANDLER, ..Default::default() }.to_bytes());
    sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, ARG, 0, 8]).unwrap();
    run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert_eq!(handling(&k), Some(SIGSEGV));
    assert_eq!(frame_info(&k).code, SI_KERNEL);
}

/// Deliver SIGUSR1 to a handler and return the frame's ucontext address.
fn in_handler(k: &mut FakeKernel) -> u64 {
    install(k, SIGUSR1, SA_SIGINFO, SigSet::EMPTY);
    run(k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert_eq!(handling(k), Some(SIGUSR1));
    k.ctx.rdx
}

/// Patch the saved sigcontext at `offset`.
fn patch(k: &mut FakeKernel, uc: u64, offset: usize, bytes: &[u8]) {
    k.poke(uc + frame::UC_MCONTEXT as u64 + offset as u64, bytes);
}

#[test]
fn rt_sigreturn_validates_the_frame() {
    use frame::sc;
    // A non-canonical rip or rsp, or a foreign code segment: SIGSEGV.
    for (offset, value) in [(sc::RIP, 0x8000_0000_0000u64), (sc::RSP, 0x00ff_0000_0000_0000)] {
        let mut k = FakeKernel::new();
        let uc = in_handler(&mut k);
        patch(&mut k, uc, offset, &value.to_le_bytes());
        assert_eq!(handler_returns(&mut k).0, ExitToUser::Killed, "offset {offset}");
        assert_eq!(k.exit, Some(Exit::Signal(SIGSEGV, true)));
    }
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    patch(&mut k, uc, sc::CS, &0x23u16.to_le_bytes());
    assert_eq!(handler_returns(&mut k).0, ExitToUser::Killed, "32-bit code segment");
    // A canonical kernel address is let through: the thread faults later.
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    patch(&mut k, uc, sc::RIP, &0xffff_8000_0000_0000u64.to_le_bytes());
    assert_eq!(handler_returns(&mut k).1.rip, 0xffff_8000_0000_0000);
    // An unreadable frame.
    let mut k = FakeKernel::new();
    in_handler(&mut k);
    k.ctx.rsp = UNMAPPED - 8;
    assert_eq!(run(&mut k, nr::RT_SIGRETURN, &[]).0, ExitToUser::Killed);
}

#[test]
fn rt_sigreturn_segment_and_flag_rules() {
    use frame::sc;
    // A bad saved SS is replaced by the user data segment...
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    patch(&mut k, uc, sc::SS, &0u16.to_le_bytes());
    k.poke(uc + frame::UC_FLAGS as u64, &UC_SIGCONTEXT_SS.to_le_bytes());
    let (exit, ctx) = handler_returns(&mut k);
    assert_eq!((exit, ctx.ss), (ExitToUser::Return, USER_SS));
    // ...unless UC_STRICT_RESTORE_SS asks for it as saved.
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    patch(&mut k, uc, sc::SS, &0u16.to_le_bytes());
    assert_eq!(handler_returns(&mut k).0, ExitToUser::Killed);
    // Only the user-changeable flags come from the frame: not IF (9),
    // IOPL (12-13) or NT (14).
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    let flags: u64 = 0x1 | 0x400 | 0x4_0000 | 0x3000 | 0x4000;
    patch(&mut k, uc, sc::EFLAGS, &flags.to_le_bytes());
    let (_, ctx) = handler_returns(&mut k);
    assert_eq!(ctx.rflags, 0x202 | 0x1 | 0x400 | 0x4_0000);
    // The mask comes back without SIGKILL and SIGSTOP.
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    k.poke(uc + frame::UC_SIGMASK as u64, &u64::MAX.to_le_bytes());
    handler_returns(&mut k);
    assert_eq!(k.mask(), SigSet::FULL.without(SigSet::UNBLOCKABLE));
}

#[test]
fn rt_sigreturn_fpu_rules() {
    use frame::sc;
    // fpstate 0: a clean FPU.
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    patch(&mut k, uc, sc::FPSTATE, &0u64.to_le_bytes());
    let resets = k.fpu_resets;
    assert_eq!(handler_returns(&mut k).0, ExitToUser::Return);
    assert_eq!(k.fpu_resets, resets + 1);
    // Misaligned: SIGSEGV.
    let mut k = FakeKernel::new();
    let uc = in_handler(&mut k);
    let fp = le_u64(&k.peek(uc + frame::UC_MCONTEXT as u64 + sc::FPSTATE as u64, 8));
    patch(&mut k, uc, sc::FPSTATE, &(fp + 8).to_le_bytes());
    assert_eq!(handler_returns(&mut k).0, ExitToUser::Killed);
    // Refused by the kernel (bad MXCSR bits, say): SIGSEGV.
    let mut k = FakeKernel::new();
    in_handler(&mut k);
    k.refuse_fpu = true;
    assert_eq!(handler_returns(&mut k).0, ExitToUser::Killed);
    // An XSAVE frame whose end marker is gone: only the legacy part.
    let mut k = FakeKernel::new();
    k.fpu = std::vec![3; 832];
    k.xfeatures = Some(7);
    let uc = in_handler(&mut k);
    let fp = le_u64(&k.peek(uc + frame::UC_MCONTEXT as u64 + sc::FPSTATE as u64, 8));
    k.poke(fp + 832, &[0; 4]);
    handler_returns(&mut k);
    assert_eq!(k.fpu_loads.last().map(|i| i.len()), Some(512));
}

#[test]
fn kill_permissions_and_targets() {
    let mut k = FakeKernel::new();
    k.add_process(200, 3000, 100, 100); // another user, our group and session
    k.add_process(300, 1000, 300, 300); // our real uid
    k.add_process(1, 0, 1, 1);
    let sigterm = SIGTERM as u64;
    assert_eq!(sys(&mut k, nr::KILL, &[200, sigterm]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::KILL, &[200, 0]), Err(Errno::EPERM), "signal 0 checks permission");
    assert_eq!(sys(&mut k, nr::KILL, &[200, SIGCONT as u64]), Ok(0), "SIGCONT within the session");
    assert_eq!(k.continued, [200]);
    assert_eq!(sys(&mut k, nr::KILL, &[300, sigterm]), Ok(0));
    assert!(k.proc_state(300).pending(300).contains(SIGTERM));
    assert_eq!(sys(&mut k, nr::KILL, &[300, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::KILL, &[999, sigterm]), Err(Errno::ESRCH));
    for bad in [65u64, u64::MAX, 1 << 32 | 65] {
        assert_eq!(sys(&mut k, nr::KILL, &[300, bad]), Err(Errno::EINVAL), "signal {bad:#x}");
    }
    // Process groups: 0 is ours (100 and 200); one refused, one accepted.
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    assert_eq!(sys(&mut k, nr::KILL, &[0, SIGUSR1 as u64]), Ok(0));
    assert!(k.me().pending(100).contains(SIGUSR1));
    assert!(!k.proc_state(200).pending(200).contains(SIGUSR1));
    assert_eq!(sys(&mut k, nr::KILL, &[(-300i64) as u64, SIGUSR2 as u64]), Ok(0));
    assert_eq!(sys(&mut k, nr::KILL, &[(-200i64) as u64, sigterm]), Err(Errno::ESRCH), "no group 200");
    assert_eq!(sys(&mut k, nr::KILL, &[(-100i64) as u64, SIGHUP as u64]), Ok(0));
    k.add_process(400, 4000, 400, 400);
    assert_eq!(sys(&mut k, nr::KILL, &[(-400i64) as u64, sigterm]), Err(Errno::EPERM), "only refusals");
    assert_eq!(sys(&mut k, nr::KILL, &[i32::MIN as u32 as u64, sigterm]), Err(Errno::ESRCH));
    // -1: everyone we may signal except init and ourselves.
    k.me().thread_mut(100).mask = SigSet::FULL.without(SigSet::UNBLOCKABLE);
    install(&mut k, SIGWINCH, 0, SigSet::EMPTY);
    assert_eq!(sys(&mut k, nr::KILL, &[u64::MAX, SIGWINCH as u64]), Ok(0));
    assert!(!k.me().pending(100).contains(SIGWINCH), "not the caller");
    // CAP_KILL may signal anyone; init only gets signals it handles.
    k.creds_mut().caps.effective = crate::cred::CapSet::only(crate::cred::cap::KILL);
    assert_eq!(sys(&mut k, nr::KILL, &[400, sigterm]), Ok(0));
    assert_eq!(sys(&mut k, nr::KILL, &[1, SIGKILL as u64]), Ok(0));
    assert!(k.proc_state(1).pending(1).is_empty(), "init ignores what it does not handle");
    k.proc_state(1).set_action(SIGTERM, SigAction { handler: 0x1000, ..Default::default() });
    assert_eq!(sys(&mut k, nr::KILL, &[1, sigterm]), Ok(0));
    assert!(k.proc_state(1).pending(1).contains(SIGTERM));
}

#[test]
fn zombies_accept_signals_silently() {
    let mut k = FakeKernel::new();
    k.add_process(300, 1000, 300, 300).info.zombie = true;
    assert_eq!(sys(&mut k, nr::KILL, &[300, SIGTERM as u64]), Ok(0));
    assert!(k.proc_state(300).pending(300).is_empty());
}

#[test]
fn tgkill_and_tkill_target_threads() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    let usr1 = SIGUSR1 as u64;
    assert_eq!(sys(&mut k, nr::TGKILL, &[100, 101, usr1]), Ok(0));
    assert!(k.me().thread(101).pending.contains(SIGUSR1));
    assert!(!k.me().thread(100).pending.contains(SIGUSR1));
    assert!(k.notified.contains(&101));
    let info = k.me().dequeue(101).unwrap();
    assert_eq!((info.code, info.pid()), (SI_TKILL, 100));
    assert_eq!(sys(&mut k, nr::TGKILL, &[999, 101, usr1]), Err(Errno::ESRCH), "wrong process");
    assert_eq!(sys(&mut k, nr::TGKILL, &[100, 555, usr1]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::TGKILL, &[0, 101, usr1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::TGKILL, &[100, u32::MAX as u64, usr1]), Err(Errno::EINVAL), "tid -1");
    assert_eq!(sys(&mut k, nr::TGKILL, &[100, 101, 99]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::TGKILL, &[100, 101, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::TKILL, &[101, usr1]), Ok(0));
    assert!(k.me().thread(101).pending.contains(SIGUSR1));
    assert_eq!(sys(&mut k, nr::TKILL, &[0, usr1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::TKILL, &[555, usr1]), Err(Errno::ESRCH));
    k.add_process(200, 3000, 200, 200);
    assert_eq!(sys(&mut k, nr::TKILL, &[200, usr1]), Err(Errno::EPERM));
}

#[test]
fn rt_sigqueueinfo_rules() {
    let mut k = FakeKernel::new();
    k.add_process(300, 1000, 300, 300);
    let mut info = SigInfo::from_process(SIGRTMIN, SI_USER, 4242, 0);
    k.poke(ARG, &info.to_bytes());
    let rt = SIGRTMIN as u64;
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, rt, ARG]), Err(Errno::EPERM), "SI_USER to another");
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[100, rt, ARG]), Ok(0), "to itself anything goes");
    info.code = SI_TKILL;
    k.poke(ARG, &info.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, rt, ARG]), Err(Errno::EPERM));
    info.code = SI_QUEUE;
    info.signo = 99;
    info.fields[8..16].copy_from_slice(&0xabcdu64.to_le_bytes());
    k.poke(ARG, &info.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, rt, ARG]), Ok(0));
    let got = k.proc_state(300).dequeue(300).unwrap();
    assert_eq!((got.signo, got.pid(), got.value()), (SIGRTMIN as i32, 4242, 0xabcd), "si_signo is sig");
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, rt, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, 65, ARG]), Err(Errno::EINVAL));
    k.proc_state(300).queue_limit = 1;
    k.proc_state(300).thread_mut(300).mask = SigSet::only(SIGRTMIN);
    k.proc_state(300).set_action(SIGRTMIN, SigAction { handler: 0x1000, ..Default::default() });
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, rt, ARG]), Ok(0));
    assert_eq!(sys(&mut k, nr::RT_SIGQUEUEINFO, &[300, rt, ARG]), Err(Errno::EAGAIN));
    // The thread-directed variant.
    k.add_thread(101);
    k.poke(ARG, &info.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 101, rt, ARG]), Ok(0));
    assert!(k.me().thread(101).pending.contains(SIGRTMIN));
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[300, 101, rt, ARG]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 0, rt, ARG]), Err(Errno::EINVAL));
}

#[test]
fn rt_sigpending_reports_blocked_pending_signals() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    install(&mut k, SIGUSR2, 0, SigSet::EMPTY);
    set_mask(&mut k, set(&[SIGUSR1, SIGUSR2, SIGHUP]));
    k.me().thread_mut(101).mask = SigSet::FULL.without(SigSet::UNBLOCKABLE);
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    sys(&mut k, nr::TGKILL, &[100, 101, SIGUSR2 as u64]).unwrap();
    assert_eq!(sys(&mut k, nr::RT_SIGPENDING, &[ARG, 8]), Ok(0));
    assert_eq!(k.peek_u64(ARG), SigSet::only(SIGUSR1).0, "the process's, not another thread's");
    // A smaller size writes that many bytes; above 8 is EINVAL.
    k.poke(ARG, &[0xee; 8]);
    assert_eq!(sys(&mut k, nr::RT_SIGPENDING, &[ARG, 4]), Ok(0));
    assert_eq!(k.peek(ARG, 8), [&SigSet::only(SIGUSR1).0.to_le_bytes()[..4], &[0xee; 4]].concat());
    assert_eq!(sys(&mut k, nr::RT_SIGPENDING, &[UNMAPPED, 0]), Ok(0), "0 writes nothing");
    assert_eq!(sys(&mut k, nr::RT_SIGPENDING, &[ARG, 9]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::RT_SIGPENDING, &[UNMAPPED, 8]), Err(Errno::EFAULT));
}

#[test]
fn rt_sigaction_drops_unknown_flags_and_discards_ignored_pending() {
    let mut k = FakeKernel::new();
    let act = SigAction { handler: HANDLER, flags: SA_RESTORER | SA_UNSUPPORTED | 0x200, restorer: RESTORER, mask: 0 };
    k.poke(ARG, &act.to_bytes());
    sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, ARG, 0, 8]).unwrap();
    sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, 0, ARG + 0x40, 8]).unwrap();
    let old = SigAction::from_bytes(&k.peek(ARG + 0x40, 32).try_into().unwrap());
    assert_eq!(old.flags, SA_RESTORER, "SA_UNSUPPORTED reads back as unsupported");
    set_mask(&mut k, SigSet::only(SIGUSR1));
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    k.poke(ARG, &SigAction { handler: SIG_IGN, ..Default::default() }.to_bytes());
    sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, ARG, 0, 8]).unwrap();
    assert!(k.me().pending(100).is_empty());
}

#[test]
fn rt_sigsuspend_waits_with_a_temporary_mask() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    let original = set(&[SIGUSR1, SIGHUP, SIGUSR2]);
    set_mask(&mut k, original);
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGHUP as u64]).unwrap());
    });
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGUSR1 as u64]).unwrap());
    });
    // Wait with only SIGHUP blocked: SIGHUP stays pending, SIGUSR1 ends it.
    k.poke(ARG, &SigSet::only(SIGHUP).0.to_le_bytes());
    let (exit, _) = run(&mut k, nr::RT_SIGSUSPEND, &[ARG, 8]);
    assert_eq!(exit, ExitToUser::Return);
    assert_eq!(k.blocks, 1, "the SIGHUP did not wake it");
    assert_eq!(handling(&k), Some(SIGUSR1));
    let uc = frame_uc(&k);
    assert_eq!(uc.mask, original, "the frame saves the mask from before rt_sigsuspend");
    assert_eq!(Errno::from_rax(uc.regs.rax), Some(Errno::EINTR));
    assert_eq!(uc.regs.rip, SYSCALL_RIP, "not restarted after a handler");
    assert_eq!(k.mask(), set(&[SIGUSR1, SIGHUP]), "the handler runs with the temporary mask plus its own");
    handler_returns(&mut k);
    assert_eq!(k.mask(), original);
    assert!(k.me().pending(100).contains(SIGHUP));
    assert_eq!(sys(&mut k, nr::RT_SIGSUSPEND, &[ARG, 4]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::RT_SIGSUSPEND, &[UNMAPPED, 8]), Err(Errno::EFAULT));
}

#[test]
fn rt_sigsuspend_restarts_after_a_stop() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGTSTP as u64]).unwrap());
    });
    set_mask(&mut k, SigSet::only(SIGINT));
    k.poke(ARG, &0u64.to_le_bytes());
    let (_, ctx) = run(&mut k, nr::RT_SIGSUSPEND, &[ARG, 8]);
    assert_eq!(k.stops, [SIGTSTP]);
    assert_eq!((ctx.rip, ctx.rax), (SYSCALL_RIP - 2, nr::RT_SIGSUSPEND), "no handler ran: called again");
    assert_eq!(k.mask(), SigSet::only(SIGINT), "the old mask is back meanwhile");
}

#[test]
fn rt_sigtimedwait_takes_pending_signals() {
    let mut k = FakeKernel::new();
    k.poke(ARG, &set(&[SIGUSR1, SIGRTMIN, SIGKILL]).0.to_le_bytes());
    let info = ARG + 0x100;
    let zero = ARG + 0x40;
    k.poke(zero, &Timespec { sec: 0, nsec: 0 }.to_bytes());
    // Nothing pending: a zero timeout polls.
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, info, zero, 8]), Err(Errno::EAGAIN));
    assert_eq!(k.blocks, 0);
    // Pending (and blocked, as it should be): taken at once.
    set_mask(&mut k, set(&[SIGUSR1, SIGRTMIN]));
    sys(&mut k, nr::KILL, &[100, SIGRTMIN as u64]).unwrap();
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, info, 0, 8]), Ok(SIGUSR1 as u64));
    let got = SigInfo::from_bytes(&k.peek(info, 128).try_into().unwrap());
    assert_eq!((got.signo, got.code, got.pid()), (SIGUSR1 as i32, SI_USER, 100));
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, 0, zero, 8]), Ok(SIGRTMIN as u64));
    // A timeout that expires.
    k.poke(ARG + 0x50, &Timespec { sec: 1, nsec: 0 }.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, info, ARG + 0x50, 8]), Err(Errno::EAGAIN));
    assert_eq!(k.mask(), set(&[SIGUSR1, SIGRTMIN]), "the mask is restored");
    // Errors.
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, info, zero, 16]), Err(Errno::EINVAL));
    k.poke(ARG + 0x60, &Timespec { sec: 0, nsec: 1_000_000_000 }.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, info, ARG + 0x60, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[UNMAPPED, info, zero, 8]), Err(Errno::EFAULT));
}

#[test]
fn rt_sigtimedwait_wakes_for_its_signals_and_is_interrupted_by_others() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    install(&mut k, SIGINT, 0, SigSet::EMPTY);
    set_mask(&mut k, SigSet::only(SIGUSR2));
    k.poke(ARG, &SigSet::only(SIGUSR2).0.to_le_bytes());
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::KILL, &[100, SIGUSR2 as u64]).unwrap());
    });
    assert_eq!(sys(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, 0, 0, 8]), Ok(SIGUSR2 as u64));
    assert_eq!(k.blocks, 1);
    assert_eq!(k.mask(), SigSet::only(SIGUSR2));
    // A handled signal outside the set interrupts it: EINTR, never restarted.
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGINT as u64]).unwrap());
    });
    install(&mut k, SIGINT, SA_RESTART, SigSet::EMPTY);
    k.poke(ARG, &SigSet::only(SIGUSR2).0.to_le_bytes());
    run(&mut k, nr::RT_SIGTIMEDWAIT, &[ARG, 0, 0, 8]);
    assert_eq!(handling(&k), Some(SIGINT));
    let uc = frame_uc(&k);
    assert_eq!(Errno::from_rax(uc.regs.rax), Some(Errno::EINTR));
    assert_eq!(uc.regs.rip, SYSCALL_RIP);
}

#[test]
fn fault_signals_cannot_be_blocked() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGSEGV, SA_SIGINFO, SigSet::EMPTY);
    k.ctx.trapno = 14;
    k.ctx.err = 6;
    k.ctx.cr2 = 0xbad0;
    Linux::fault_signal(&mut k, SigInfo::fault(SIGSEGV, SEGV_MAPERR, 0xbad0));
    assert_eq!(Linux::exit_to_user(&mut k), ExitToUser::Return);
    assert_eq!(handling(&k), Some(SIGSEGV));
    assert_eq!(frame_info(&k).addr(), 0xbad0);
    let uc = frame_uc(&k);
    assert_eq!((uc.regs.trapno, uc.regs.err, uc.regs.cr2), (14, 6, 0xbad0));
    // Blocked: unblocked, and the default action applies (sigprocmask(2):
    // the result is undefined; Poly kills, like an ignored fault).
    let mut k = FakeKernel::new();
    install(&mut k, SIGSEGV, SA_SIGINFO, SigSet::EMPTY);
    set_mask(&mut k, SigSet::only(SIGSEGV));
    Linux::fault_signal(&mut k, SigInfo::fault(SIGSEGV, SEGV_MAPERR, 0xbad0));
    assert_eq!(Linux::exit_to_user(&mut k), ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSEGV, true)));
    // Ignored: the default action applies.
    let mut k = FakeKernel::new();
    k.poke(ARG, &SigAction { handler: SIG_IGN, ..Default::default() }.to_bytes());
    sys(&mut k, nr::RT_SIGACTION, &[SIGBUS as u64, ARG, 0, 8]).unwrap();
    Linux::fault_signal(&mut k, SigInfo::fault(SIGBUS, 2, 0));
    assert_eq!(Linux::exit_to_user(&mut k), ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGBUS, true)));
}

#[test]
fn signals_wake_the_right_thread() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    k.add_thread(102);
    install(&mut k, SIGUSR1, 0, SigSet::EMPTY);
    k.me().thread_mut(100).mask = SigSet::only(SIGUSR1);
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    assert_eq!(k.notified.iter().copied().collect::<std::vec::Vec<_>>(), [101]);
    k.notified.clear();
    sys(&mut k, nr::KILL, &[100, SIGTERM as u64]).unwrap();
    assert_eq!(k.notified.len(), 3, "a fatal signal reaches every thread");
}
