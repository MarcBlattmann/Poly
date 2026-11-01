//! personality(2), pause(2), tkill(2)'s and rt_tgsigqueueinfo(2)'s check
//! order, and restart_syscall(2) from a handler (hand-off spec-linux-d).

use super::fake::*;
use crate::abi::signal::*;
use crate::abi::{SigAction, SigInfo};
use crate::errno::Errno;
use crate::nr;
use crate::signal::ExitToUser;
use crate::sys::personality::{ADDR_NO_RANDOMIZE, PER_LINUX32, READ_IMPLIES_EXEC, UNAME26};

const ARG: u64 = DATA + 0x800;
const HANDLER: u64 = 0x40_5000;

fn install(k: &mut FakeKernel, sig: u32, flags: u64) {
    let act = SigAction { handler: HANDLER, flags: flags | SA_RESTORER, restorer: 0x40_6000, mask: 0 };
    k.poke(DATA + 0xc00, &act.to_bytes());
    sys(k, nr::RT_SIGACTION, &[sig as u64, DATA + 0xc00, 0, 8]).unwrap();
}

/// The saved `rax` in the frame of the handler now running.
fn saved_rax(k: &FakeKernel) -> u64 {
    let uc = k.ctx.rdx + crate::signal::frame::UC_MCONTEXT as u64;
    k.peek_u64(uc + 104)
}

#[test]
fn personality_sets_and_reads_32_bits() {
    let mut k = FakeKernel::new();
    let query = 0xffff_ffffu64;
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[query]), Ok(0));
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[6]), Ok(0));
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[0x8000_0000]), Ok(6));
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[query]), Ok(0x8000_0000), "zero-extended, never an error");
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[(1 << 32) | 5]), Ok(0x8000_0000));
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[u64::MAX]), Ok(5), "low half all ones: a query");
    assert_eq!(sys(&mut k, nr::PERSONALITY, &[0x2_ffff_ffff]), Ok(5));
    assert_eq!(k.thread_state().personality, 5);
}

#[test]
fn personality_is_per_thread_inherited_and_mostly_kept_by_exec() {
    let mut k = FakeKernel::new();
    sys(&mut k, nr::PERSONALITY, &[u64::from(READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE)]).unwrap();
    k.add_thread(101);
    assert_eq!(k.me().thread(101).personality, READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE);
    k.as_thread(101, |k| sys(k, nr::PERSONALITY, &[0]).unwrap());
    assert_eq!(k.thread_state().personality, READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE, "the creator keeps its own");
    let child = k.me().fork(100, 200);
    assert_eq!(child.thread(200).personality, READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE);
    k.me().exec(100);
    assert_eq!(k.thread_state().personality, ADDR_NO_RANDOMIZE, "execve drops READ_IMPLIES_EXEC");
}

#[test]
fn uname_follows_the_persona() {
    let mut k = FakeKernel::new();
    sys(&mut k, nr::PERSONALITY, &[u64::from(PER_LINUX32 | UNAME26)]).unwrap();
    assert_eq!(sys(&mut k, nr::UNAME, &[ARG]), Ok(0));
    let field = |k: &FakeKernel, i: u64| {
        let raw = k.peek(ARG + i * 65, 65);
        raw[..raw.iter().position(|&b| b == 0).unwrap()].to_vec()
    };
    assert_eq!(field(&k, 4), b"i686");
    assert!(field(&k, 2).starts_with(b"2.6."));
    sys(&mut k, nr::PERSONALITY, &[0]).unwrap();
    sys(&mut k, nr::UNAME, &[ARG]).unwrap();
    assert_eq!(field(&k, 4), b"x86_64");
}

#[test]
fn pause_returns_eintr_after_a_handler_even_with_sa_restart() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    k.add_thread(101);
    k.meanwhile(|k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGUSR1 as u64]).unwrap());
    });
    let (exit, ctx) = run(&mut k, nr::PAUSE, &[]);
    assert_eq!(exit, ExitToUser::Return);
    assert_eq!(ctx.rip, HANDLER);
    assert_eq!(saved_rax(&k), Errno::EINTR.to_rax());
    // Nothing to restart from the handler.
    assert_eq!(sys(&mut k, nr::RESTART_SYSCALL, &[]), Err(Errno::EINTR));
}

#[test]
fn tkill_looks_up_the_thread_before_the_signal() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::TKILL, &[0, 999]), Err(Errno::EINVAL), "tid first");
    assert_eq!(sys(&mut k, nr::TKILL, &[0x7fff_ffff, 999]), Err(Errno::ESRCH), "then the thread");
    assert_eq!(sys(&mut k, nr::TKILL, &[100, 999]), Err(Errno::EINVAL), "then the signal");
    assert_eq!(sys(&mut k, nr::TKILL, &[100, 0]), Ok(0));
}

#[test]
fn rt_tgsigqueueinfo_codes_for_other_threads() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    let rt = SIGRTMIN as u64;
    let mut info = SigInfo::from_process(SIGRTMIN, SI_USER, 4242, 0);
    k.poke(ARG, &info.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 101, rt, ARG]), Err(Errno::EPERM), "a sibling is another");
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 0x7fff_ffff, 999, ARG]), Err(Errno::EPERM), "before lookup");
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 100, rt, ARG]), Ok(0), "its own thread: anything");
    info.code = SI_QUEUE;
    k.poke(ARG, &info.to_bytes());
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 0x7fff_ffff, 999, ARG]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 101, 999, ARG]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[100, 101, rt, ARG]), Ok(0));
    assert!(k.me().thread(101).pending.contains(SIGRTMIN));
    // An unknown layout with bytes past the first 48: E2BIG, before the ids.
    let mut raw = info.to_bytes();
    raw[4 * 2..4 * 3].copy_from_slice(&(-42i32).to_le_bytes());
    raw[100] = 1;
    k.poke(ARG, &raw);
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[0, 0, rt, ARG]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::RT_TGSIGQUEUEINFO, &[0, 0, rt, UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn known_siginfo_layouts() {
    use crate::sys::signal::known_info_layout;
    for code in [0, -1, -6, -7, -60, 0x80, 1, 5] {
        assert!(known_info_layout(code), "{code}");
    }
    for code in [-8, -42, 0x81, 1000] {
        assert!(!known_info_layout(code), "{code}");
    }
}

#[test]
fn a_thread_waiting_for_a_signal_sent_to_it_is_woken() {
    use crate::kernel::SigSet;
    use crate::signal::Dest;
    let mut k = FakeKernel::new();
    k.add_thread(101);
    let t = k.me().thread_mut(101);
    // rt_sigtimedwait: blocked in the real mask, unmasked and waited for.
    t.sigwait = SigSet::only(SIGRTMIN);
    t.mask = SigSet::default();
    let sent = k.me().send(SigInfo::new(SIGRTMIN, SI_QUEUE), Dest::Thread(101), false).unwrap();
    assert_eq!(sent.notify, [101]);
    assert!(k.me().thread(101).pending.contains(SIGRTMIN));
}
