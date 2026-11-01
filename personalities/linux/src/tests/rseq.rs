//! rseq(2) against the fake kernel (ADR 0111): registration and its
//! checks, what the kernel writes into the area, fork and exec, and the
//! abort of a critical section on a signal or after a switch.

extern crate std;

use super::fake::*;
use crate::abi::SigAction;
use crate::abi::signal::*;
use crate::errno::Errno;
use crate::kernel::{SigSet, UserPtr};
use crate::signal::ExitToUser;
use crate::signal::frame::{self, UCONTEXT_SIZE};
use crate::sys::rseq::Registration;
use crate::{Linux, nr};

const SIG: u64 = 0x5305_3053;
/// The `struct rseq` (32-byte aligned).
const AREA: u64 = DATA + 0x1000;
/// A `struct rseq_cs`.
const CS: u64 = DATA + 0x1100;
/// The code of the critical section and its abort handler (only their
/// addresses and the signature word matter).
const START: u64 = DATA + 0x2000;
const ABORT: u64 = DATA + 0x2100;
const HANDLER: u64 = 0x40_5000;
const RESTORER: u64 = 0x40_6000;
const ARG: u64 = DATA + 0x800;

fn kernel() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.on_cpu = 3;
    k.poke(AREA, &[0xaa; 32]);
    k
}

fn rseq(k: &mut FakeKernel, area: u64, len: u64, flags: u64, sig: u64) -> Result<u64, Errno> {
    sys(k, nr::RSEQ, &[area, len, flags, sig])
}

/// A critical section `[START, START + 16)` aborting to `ABORT`, whose
/// preceding word is `sig`; `rseq_cs` of the area points to it.
fn arm(k: &mut FakeKernel, sig: u32) {
    let mut cs = [0u8; 32];
    cs[8..16].copy_from_slice(&START.to_le_bytes());
    cs[16..24].copy_from_slice(&16u64.to_le_bytes());
    cs[24..32].copy_from_slice(&ABORT.to_le_bytes());
    k.poke(CS, &cs);
    k.poke(ABORT - 4, &sig.to_le_bytes());
    k.poke(AREA + 8, &CS.to_le_bytes());
}

#[test]
fn register_writes_the_ids_and_keeps_the_rest() {
    let mut k = kernel();
    assert_eq!(rseq(&mut k, AREA, 32, 0, SIG), Ok(0));
    assert_eq!((k.peek_u32(AREA), k.peek_u32(AREA + 4)), (3, 3), "cpu_id_start, cpu_id");
    assert_eq!(k.peek_u64(AREA + 8), 0xaaaa_aaaa_aaaa_aaaa, "rseq_cs untouched");
    assert_eq!(k.peek_u32(AREA + 16), 0xaaaa_aaaa, "flags untouched");
    assert_eq!((k.peek_u32(AREA + 20), k.peek_u32(AREA + 24)), (0, 0), "node_id, mm_cid");
    assert_eq!(k.thread_state().rseq, Some(Registration { area: UserPtr(AREA), len: 32, sig: SIG as u32 }));

    assert_eq!(rseq(&mut k, AREA, 32, 0, SIG), Err(Errno::EBUSY));
    assert_eq!(rseq(&mut k, AREA, 32, 0, 7), Err(Errno::EPERM));
    assert_eq!(rseq(&mut k, AREA + 32, 32, 0, SIG), Err(Errno::EINVAL));
    assert_eq!(rseq(&mut k, AREA, 32, 1, 7), Err(Errno::EPERM));
    assert_eq!(rseq(&mut k, AREA, 64, 1, SIG), Err(Errno::EINVAL));

    assert_eq!(rseq(&mut k, AREA, 32, 1, SIG), Ok(0));
    assert_eq!((k.peek_u32(AREA), k.peek_u32(AREA + 4)), (0, 0xffff_ffff), "unregistered");
    assert_eq!(k.thread_state().rseq, None);
    assert_eq!(rseq(&mut k, AREA, 32, 1, SIG), Err(Errno::EINVAL));
    // Afterwards any area and signature.
    assert_eq!(rseq(&mut k, AREA + 32, 64, 0, 7), Ok(0));
}

#[test]
fn checks_and_their_order() {
    let mut k = kernel();
    assert_eq!(rseq(&mut k, AREA, 16, 0, SIG), Err(Errno::EINVAL));
    assert_eq!(rseq(&mut k, AREA + 8, 32, 0, SIG), Err(Errno::EINVAL));
    assert_eq!(rseq(&mut k, AREA, 32, 2, SIG), Err(Errno::EINVAL));
    assert_eq!(rseq(&mut k, 0, 32, 0, SIG), Err(Errno::EFAULT));
    assert_eq!(rseq(&mut k, 0x8000_0000_0000, 32, 0, SIG), Err(Errno::EFAULT));
    assert_eq!(rseq(&mut k, 8, 32, 1, SIG), Err(Errno::EINVAL), "unregistering nothing");
    // Upper halves ignored.
    assert_eq!(rseq(&mut k, AREA, (1 << 32) | 32, 0, (1 << 32) | SIG), Ok(0));
    assert_eq!(rseq(&mut k, AREA, 32, (1 << 32) | 1, SIG), Ok(0));
    assert_eq!(k.thread_state().rseq, None);
}

#[test]
fn a_read_only_area_is_accepted_then_fatal() {
    let mut k = kernel();
    let ro = read_only_page(&mut k);
    let (exit, _) = run(&mut k, nr::RSEQ, &[ro, 32, 0, SIG]);
    assert_eq!(exit, ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSEGV, true)));
}

#[test]
fn fork_keeps_it_exec_drops_it() {
    let mut k = kernel();
    assert_eq!(rseq(&mut k, AREA, 32, 0, SIG), Ok(0));
    let tid = k.tid;
    let child = k.me().fork(tid, 77);
    assert_eq!(child.thread(77).rseq, k.thread_state().rseq);
    k.me().add_thread(tid, 78, true);
    assert_eq!(k.me().thread(78).rseq, None, "a new thread starts without one");
    k.me().exec(tid);
    assert_eq!(k.thread_state().rseq, None);
}

#[test]
fn a_switch_inside_the_section_aborts() {
    let mut k = kernel();
    assert_eq!(rseq(&mut k, AREA, 32, 0, SIG), Ok(0));
    arm(&mut k, SIG as u32);
    k.on_cpu = 5;
    k.ctx.rip = START + 8;
    Linux::rseq_resume(&mut k);
    assert_eq!(k.ctx.rip, ABORT);
    assert_eq!(k.peek_u64(AREA + 8), 0, "rseq_cs cleared");
    assert_eq!((k.peek_u32(AREA), k.peek_u32(AREA + 4)), (5, 5), "the new CPU");

    // Outside the section: only cleared.
    arm(&mut k, SIG as u32);
    k.ctx.rip = START + 16;
    Linux::rseq_resume(&mut k);
    assert_eq!(k.ctx.rip, START + 16);
    assert_eq!(k.peek_u64(AREA + 8), 0);
}

#[test]
fn a_wrong_signature_is_fatal() {
    let mut k = kernel();
    assert_eq!(rseq(&mut k, AREA, 32, 0, SIG), Ok(0));
    arm(&mut k, 0x2222_2222);
    k.ctx.rip = START;
    Linux::rseq_resume(&mut k);
    assert_eq!(k.ctx.rip, START, "not moved");
    assert_eq!(Linux::exit_to_user(&mut k), ExitToUser::Killed);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSEGV, true)));
}

#[test]
fn a_signal_inside_the_section_returns_to_the_abort_handler() {
    let mut k = kernel();
    let act = SigAction { handler: HANDLER, flags: SA_SIGINFO | SA_RESTORER, restorer: RESTORER, mask: 0 };
    k.poke(ARG, &act.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[SIGUSR1 as u64, ARG, 0, 8]), Ok(0));
    assert_eq!(rseq(&mut k, AREA, 32, 0, SIG), Ok(0));
    // The thread makes a call from inside the section (the fake's syscall
    // address) and a signal is delivered on the way back.
    let start = k.ctx.rip;
    let mut cs = [0u8; 32];
    cs[8..16].copy_from_slice(&(start - 4).to_le_bytes());
    cs[16..24].copy_from_slice(&16u64.to_le_bytes());
    cs[24..32].copy_from_slice(&ABORT.to_le_bytes());
    k.poke(CS, &cs);
    k.poke(ABORT - 4, &(SIG as u32).to_le_bytes());
    k.poke(AREA + 8, &CS.to_le_bytes());
    let (exit, ctx) = run(&mut k, nr::KILL, &[100, SIGUSR1 as u64]);
    assert_eq!(exit, ExitToUser::Return);
    assert_eq!(ctx.rip, HANDLER);
    let uc = frame::decode_ucontext(&k.peek(ctx.rdx, UCONTEXT_SIZE).try_into().unwrap());
    assert_eq!(uc.regs.rip, ABORT, "the handler returns to the abort handler");
    assert_eq!(k.peek_u64(AREA + 8), 0);
    assert_eq!(k.mask(), SigSet::only(SIGUSR1));
}

#[test]
fn nothing_happens_without_a_registration() {
    let mut k = kernel();
    k.ctx.rip = START;
    Linux::rseq_resume(&mut k);
    assert_eq!(k.ctx.rip, START);
    assert_eq!(k.peek_u32(AREA), 0xaaaa_aaaa);
}
