//! Signals as a tracer sees them (the test-run protocol's `POLY-SIGNAL`,
//! ADR 0027): every signal a thread takes to act on is reported through
//! `LinuxKernel::signal_delivered`, but `SIGKILL`; a traced process keeps
//! ignored signals until a thread takes them; interrupted calls report
//! the number a tracer sees at their exit.

use super::fake::*;
use crate::abi::signal::*;
use crate::abi::{SigAction, SigInfo};
use crate::errno::Errno;
use crate::kernel::SigSet;
use crate::signal::ExitToUser;
use crate::{Linux, nr};

const HANDLER: u64 = 0x40_5000;
const RESTORER: u64 = 0x40_6000;
const ARG: u64 = DATA + 0x800;

fn action(k: &mut FakeKernel, sig: u32, handler: u64) {
    let act = SigAction { handler, flags: SA_RESTORER, restorer: RESTORER, mask: 0 };
    k.poke(ARG, &act.to_bytes());
    assert_eq!(sys(k, nr::RT_SIGACTION, &[sig as u64, ARG, 0, 8]), Ok(0));
}

fn mask(k: &mut FakeKernel, how: u32, set: SigSet) -> ExitToUser {
    k.poke(ARG, &set.0.to_le_bytes());
    run(k, nr::RT_SIGPROCMASK, &[how as u64, ARG, 0, 8]).0
}

fn pending(k: &mut FakeKernel) -> u64 {
    assert_eq!(sys(k, nr::RT_SIGPENDING, &[ARG, 8]), Ok(0));
    k.peek_u64(ARG)
}

#[test]
fn handled_stopping_and_fatal_signals_are_reported_but_sigkill() {
    let mut k = FakeKernel::new();
    action(&mut k, SIGUSR1, HANDLER);
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).0, ExitToUser::Return);
    assert_eq!(k.delivered, [SIGUSR1]);
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGSTOP as u64]).0, ExitToUser::Return);
    assert_eq!(k.stops, [SIGSTOP]);
    assert_eq!(k.delivered, [SIGUSR1, SIGSTOP]);
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGTERM as u64]).0, ExitToUser::Killed);
    assert_eq!(k.delivered, [SIGUSR1, SIGSTOP, SIGTERM]);

    let mut k = FakeKernel::new();
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGKILL as u64]).0, ExitToUser::Killed);
    assert!(k.delivered.is_empty(), "ptrace(2): no signal-delivery-stop for SIGKILL");
}

#[test]
fn several_pending_signals_are_reported_in_delivery_order() {
    let mut k = FakeKernel::new();
    for s in [SIGUSR1, SIGUSR2, SIGRTMIN] {
        action(&mut k, s, HANDLER);
    }
    let all = SigSet::only(SIGUSR1).union(SigSet::only(SIGUSR2)).union(SigSet::only(SIGRTMIN));
    mask(&mut k, SIG_BLOCK, all);
    for s in [SIGRTMIN, SIGUSR2, SIGUSR1] {
        sys(&mut k, nr::KILL, &[100, s as u64]).unwrap();
    }
    assert!(k.delivered.is_empty(), "blocked signals are not taken");
    mask(&mut k, SIG_UNBLOCK, all);
    assert_eq!(k.delivered, [SIGUSR1, SIGUSR2, SIGRTMIN]);
}

#[test]
fn a_fault_is_reported_once_when_taken() {
    let mut k = FakeKernel::new();
    Linux::fault_signal(&mut k, SigInfo::fault(SIGSEGV, SEGV_MAPERR, 0x10));
    assert!(k.delivered.is_empty());
    assert_eq!(Linux::exit_to_user(&mut k), ExitToUser::Killed);
    assert_eq!(k.delivered, [SIGSEGV]);
}

#[test]
fn a_handler_frame_that_cannot_be_written_reports_both_signals() {
    let mut k = FakeKernel::new();
    // No SA_RESTORER: the frame cannot be set up, SIGSEGV follows.
    let act = SigAction { handler: HANDLER, flags: 0, restorer: 0, mask: 0 };
    k.poke(ARG, &act.to_bytes());
    sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, ARG, 0, 8]).unwrap();
    assert_eq!(run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).0, ExitToUser::Killed);
    assert_eq!(k.delivered, [SIGUSR1, SIGSEGV]);
}

#[test]
fn blocked_ignored_signals_stay_pending_until_unblocked() {
    let mut k = FakeKernel::new();
    action(&mut k, SIGUSR1, SIG_IGN);
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    assert_eq!(pending(&mut k), 0, "unblocked: discarded");
    mask(&mut k, SIG_BLOCK, SigSet::only(SIGUSR1).union(SigSet::only(SIGURG)));
    sys(&mut k, nr::KILL, &[100, SIGUSR1 as u64]).unwrap();
    sys(&mut k, nr::KILL, &[100, SIGURG as u64]).unwrap();
    assert_eq!(pending(&mut k), SigSet::only(SIGUSR1).union(SigSet::only(SIGURG)).0, "SIG_IGN and default ignore");
    assert_eq!(mask(&mut k, SIG_SETMASK, SigSet::EMPTY), ExitToUser::Return);
    assert_eq!(pending(&mut k), 0, "dropped when taken");
    assert_eq!(k.delivered, [SIGUSR1, SIGURG]);
    assert!(k.exit.is_none());
}

#[test]
fn an_untraced_process_discards_ignored_signals_a_traced_one_takes_them() {
    let mut k = FakeKernel::new();
    action(&mut k, SIGUSR2, SIG_IGN);
    for s in [SIGURG, SIGCHLD, SIGWINCH, SIGCONT, SIGUSR2] {
        assert_eq!(run(&mut k, nr::KILL, &[100, s as u64]).0, ExitToUser::Return);
    }
    assert!(k.delivered.is_empty());

    let mut k = FakeKernel::new();
    k.me().traced = true;
    action(&mut k, SIGUSR2, SIG_IGN);
    for s in [SIGURG, SIGCHLD, SIGWINCH, SIGCONT, SIGUSR2] {
        assert_eq!(run(&mut k, nr::KILL, &[100, s as u64]).0, ExitToUser::Return);
        assert_eq!(pending(&mut k), 0);
    }
    assert_eq!(k.delivered, [SIGURG, SIGCHLD, SIGWINCH, SIGCONT, SIGUSR2]);
    assert!(k.exit.is_none());
    assert!(k.me().fork(100, 200).traced, "inherited by fork");
}

#[test]
fn an_interrupted_call_reports_the_number_a_tracer_sees() {
    let eintr = Errno::EINTR.to_rax();
    let mut k = FakeKernel::new();
    assert_eq!(Linux::traced_result(&mut k, eintr), eintr, "a plain EINTR");
    assert_eq!(Linux::traced_result(&mut k, 5), 5);
    for (code, seen) in [
        (Errno::RESTART_IF_SA_RESTART, 512),
        (Errno::RESTART_ALWAYS, 513),
        (Errno::RESTART_IF_NO_HANDLER, 514),
        (Errno::RESTART_WITH_BLOCK, 516),
    ] {
        crate::signal::deliver::note_restart(&mut k, nr::READ, code);
        assert_eq!(Linux::traced_result(&mut k, eintr), (-seen) as u64, "{code:?}");
        k.me().thread_mut(100).restart = None;
    }
}
