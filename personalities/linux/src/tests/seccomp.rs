//! seccomp(2) and `PR_SET_SECCOMP` (ADR 0054): installing filters and
//! strict mode, the check before each call, the actions, stacking, TSYNC
//! and inheritance.

use super::fake::{Exit, FakeKernel, SYSCALL_RIP, call, scratch, sys};
use crate::abi::signal::{SIGKILL, SIGSYS};
use crate::cred::cap;
use crate::errno::Errno;
use crate::nr;
use crate::seccomp::abi::*;
use crate::seccomp::{Action, Seccomp, SeccompData, precedence};
use crate::signal::LinuxProcess;
use alloc::vec;
use alloc::vec::Vec;
use poly_abi::Outcome;
use poly_bpf::Insn;
use poly_bpf::code::*;

const PR_SET_NO_NEW_PRIVS: u64 = 38;

fn ret(k: u32) -> Insn {
    Insn::stmt(RET | K, k)
}

/// Return `action` for system call `number`, allow the rest.
fn for_call(number: u64, action: u32) -> Vec<Insn> {
    vec![
        Insn::stmt(LD | W | ABS, 4),
        Insn::jump(JMP | JEQ | K, AUDIT_ARCH_X86_64, 1, 0),
        ret(SECCOMP_RET_KILL_PROCESS),
        Insn::stmt(LD | W | ABS, 0),
        Insn::jump(JMP | JEQ | K, number as u32, 0, 1),
        ret(action),
        ret(SECCOMP_RET_ALLOW),
    ]
}

/// Put `insns` and a `struct sock_fprog` for them in the fake's memory;
/// returns the fprog's address.
fn put_program(k: &mut FakeKernel, insns: &[Insn]) -> u64 {
    let at = scratch(0x1000);
    let mut bytes = Vec::new();
    for i in insns {
        bytes.extend_from_slice(&i.code.to_le_bytes());
        bytes.push(i.jt);
        bytes.push(i.jf);
        bytes.extend_from_slice(&i.k.to_le_bytes());
    }
    k.poke(at, &bytes);
    let fprog = scratch(0x800);
    let mut header = [0u8; 16];
    header[0..2].copy_from_slice(&(insns.len() as u16).to_le_bytes());
    header[8..16].copy_from_slice(&at.to_le_bytes());
    k.poke(fprog, &header);
    fprog
}

fn no_new_privs(k: &mut FakeKernel) {
    assert_eq!(sys(k, nr::PRCTL, &[PR_SET_NO_NEW_PRIVS, 1]), Ok(0));
}

fn install(k: &mut FakeKernel, insns: &[Insn], flags: u32) -> Result<u64, Errno> {
    let fprog = put_program(k, insns);
    sys(k, nr::SECCOMP, &[u64::from(SECCOMP_SET_MODE_FILTER), u64::from(flags), fprog])
}

#[test]
fn without_a_filter_calls_are_not_checked() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(100));
    assert!(!k.seccomp_on);
}

#[test]
fn a_filter_returns_an_errno_for_one_call() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    assert_eq!(install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), 0), Ok(0));
    assert!(k.seccomp_on, "the kernel was told");
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::GETTID, &[]), Ok(100));
    // The data is the errno, at most 4095; 0 is a success without the call.
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 0xffff), 0).unwrap();
    assert_eq!(call(&mut k, nr::GETPID, &[]).1 as i64, -4095);
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO), 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(0));
}

#[test]
fn a_filter_needs_no_new_privs_or_cap_sys_admin() {
    let mut k = FakeKernel::new();
    assert_eq!(install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), 0), Err(Errno::EACCES));
    assert!(!k.seccomp_on);
    k.creds_mut().caps.effective = crate::cred::CapSet::only(cap::SYS_ADMIN);
    assert_eq!(install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), 0), Ok(0));
}

#[test]
fn bad_programs_and_arguments_are_refused() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    // A backward-looking load, an unaligned load, no return.
    assert_eq!(install(&mut k, &[Insn::stmt(LD | W | ABS, 64), ret(0)], 0), Err(Errno::EINVAL));
    assert_eq!(install(&mut k, &[Insn::stmt(LD | W | ABS, 1), ret(0)], 0), Err(Errno::EINVAL));
    assert_eq!(install(&mut k, &[Insn::stmt(LD | W | IMM, 1)], 0), Err(Errno::EINVAL));
    assert_eq!(install(&mut k, &[Insn::stmt(JMP | JA, 1), ret(0)], 0), Err(Errno::EINVAL));
    // Length 0 and 4097.
    let fprog = put_program(&mut k, &[ret(0)]);
    k.poke(fprog, &0u16.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SECCOMP, &[1, 0, fprog]), Err(Errno::EINVAL));
    k.poke(fprog, &4097u16.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SECCOMP, &[1, 0, fprog]), Err(Errno::EINVAL));
    // Unreadable structures.
    assert_eq!(sys(&mut k, nr::SECCOMP, &[1, 0, 0]), Err(Errno::EFAULT));
    let fprog = put_program(&mut k, &[ret(0)]);
    k.poke(fprog + 8, &0x3000_0000u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SECCOMP, &[1, 0, fprog]), Err(Errno::EFAULT));
    // Unknown and unsupported flags, operations.
    let allow = [ret(SECCOMP_RET_ALLOW)];
    assert_eq!(install(&mut k, &allow, 1 << 6), Err(Errno::EINVAL));
    assert_eq!(install(&mut k, &allow, SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV), Err(Errno::EINVAL), "needs a listener");
    assert_eq!(
        install(&mut k, &allow, SECCOMP_FILTER_FLAG_TSYNC | SECCOMP_FILTER_FLAG_NEW_LISTENER),
        Err(Errno::EINVAL),
        "TSYNC with a listener needs TSYNC_ESRCH"
    );
    assert_eq!(sys(&mut k, nr::SECCOMP, &[9, 0, 0]), Err(Errno::EINVAL));
    assert!(!k.seccomp_on, "nothing was installed");
    // Accepted flags.
    assert_eq!(install(&mut k, &allow, SECCOMP_FILTER_FLAG_SPEC_ALLOW | SECCOMP_FILTER_FLAG_LOG), Ok(0));
    // A listener: the kernel's descriptor is the result; one per chain.
    assert_eq!(install(&mut k, &allow, SECCOMP_FILTER_FLAG_NEW_LISTENER), Ok(3));
    assert_eq!(install(&mut k, &allow, SECCOMP_FILTER_FLAG_NEW_LISTENER), Err(Errno::EBUSY));
    assert_eq!(k.thread_state().seccomp.filters(), 2);
}

#[test]
fn program_checks_come_before_the_privilege() {
    let mut k = FakeKernel::new();
    k.creds_mut().caps = crate::cred::Caps::NONE;
    // Without no_new_privs: a bad program is still EFAULT / EINVAL.
    assert_eq!(sys(&mut k, nr::SECCOMP, &[1, 0, 0]), Err(Errno::EFAULT));
    assert_eq!(install(&mut k, &[], 0), Err(Errno::EINVAL));
    assert_eq!(install(&mut k, &[ret(SECCOMP_RET_ALLOW)], 0), Err(Errno::EACCES));
}

#[test]
fn trap_sends_sigsys_with_the_call_in_its_siginfo() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_TRAP | 0x42), 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(nr::GETPID), "registers as at entry");
    let info = *k.thread_state().pending.iter().next().expect("SIGSYS queued");
    assert_eq!(info.signal(), SIGSYS);
    assert_eq!(info.code, SYS_SECCOMP);
    assert_eq!(info.errno, 0x42);
    let b = info.to_bytes();
    assert_eq!(u64::from_le_bytes(b[16..24].try_into().unwrap()), SYSCALL_RIP - 2, "si_call_addr");
    assert_eq!(i32::from_le_bytes(b[24..28].try_into().unwrap()), nr::GETPID as i32, "si_syscall");
    assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), AUDIT_ARCH_X86_64, "si_arch");
    assert!(k.notified.contains(&100));
}

#[test]
fn kill_process_ends_the_process_as_if_by_sigsys() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_KILL_PROCESS), 0).unwrap();
    call(&mut k, nr::GETPID, &[]);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSYS, true)));
    assert_eq!(k.seccomp_logged, vec![(nr::GETPID, SECCOMP_RET_KILL_PROCESS)]);
    // Unknown actions count as KILL_PROCESS.
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, 0x0001_0000), 0).unwrap();
    call(&mut k, nr::GETPID, &[]);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSYS, true)));
}

#[test]
fn kill_thread_ends_the_thread_or_a_single_threaded_process() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_KILL_THREAD), 0).unwrap();
    call(&mut k, nr::GETPID, &[]);
    assert_eq!(k.exit, Some(Exit::Signal(SIGSYS, true)));
    let mut k = FakeKernel::new();
    k.add_thread(101);
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_KILL_THREAD), 0).unwrap();
    call(&mut k, nr::GETPID, &[]);
    assert_eq!(k.exit, Some(Exit::Thread(0)));
}

#[test]
fn trace_and_user_notif_without_a_tracer_fail_with_enosys() {
    for action in [SECCOMP_RET_TRACE | 7, SECCOMP_RET_USER_NOTIF] {
        let mut k = FakeKernel::new();
        no_new_privs(&mut k);
        install(&mut k, &for_call(nr::GETPID, action), 0).unwrap();
        assert_eq!(sys(&mut k, nr::GETPID, &[]), Err(Errno::ENOSYS));
        assert_eq!(k.exit, None);
    }
}

#[test]
fn log_runs_the_call_and_logs_it() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_LOG), 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(100));
    assert_eq!(k.seccomp_logged, vec![(nr::GETPID, SECCOMP_RET_LOG)]);
    // With the LOG flag every action but ALLOW is logged.
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), SECCOMP_FILTER_FLAG_LOG).unwrap();
    assert_eq!(sys(&mut k, nr::GETTID, &[]), Ok(100));
    assert!(k.seccomp_logged.is_empty());
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Err(Errno::EPERM));
    assert_eq!(k.seccomp_logged, vec![(nr::GETPID, SECCOMP_RET_ERRNO | 1)]);
}

#[test]
fn the_filter_sees_the_arguments_and_the_instruction_pointer() {
    // Allow getpid only when its first argument is 7 and the instruction
    // pointer's low word is SYSCALL_RIP's.
    let insns = [
        Insn::stmt(LD | W | ABS, 16),
        Insn::jump(JMP | JEQ | K, 7, 0, 3),
        Insn::stmt(LD | W | ABS, 8),
        Insn::jump(JMP | JEQ | K, SYSCALL_RIP as u32, 0, 1),
        ret(SECCOMP_RET_ALLOW),
        ret(SECCOMP_RET_ERRNO | 13),
    ];
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &insns, 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[7]), Ok(100));
    assert_eq!(sys(&mut k, nr::GETPID, &[8]), Err(Errno::EACCES));
}

#[test]
fn stacked_filters_take_the_most_restrictive_action() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), 0).unwrap();
    // A newer filter that allows everything does not lift the older one's
    // ERRNO, and a newer TRAP beats it.
    install(&mut k, &[ret(SECCOMP_RET_ALLOW)], 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Err(Errno::EPERM));
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_TRAP), 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(nr::GETPID));
    assert_eq!(k.thread_state().pending.len(), 1);
    assert_eq!(k.thread_state().seccomp.filters(), 3);
    // Among equal actions the newest filter's data counts.
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), 0).unwrap();
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 2), 0).unwrap();
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Err(Errno::ENOENT));
}

#[test]
fn precedence_follows_the_man_page() {
    let order = [
        SECCOMP_RET_KILL_PROCESS,
        SECCOMP_RET_KILL_THREAD,
        SECCOMP_RET_TRAP,
        SECCOMP_RET_ERRNO,
        SECCOMP_RET_USER_NOTIF,
        SECCOMP_RET_TRACE,
        SECCOMP_RET_LOG,
        SECCOMP_RET_ALLOW,
    ];
    for w in order.windows(2) {
        assert!(precedence(w[0]) < precedence(w[1]), "{:#x} before {:#x}", w[0], w[1]);
    }
    assert_eq!(Action::of(SECCOMP_RET_ERRNO | 5), Action::Errno(5));
    assert_eq!(Action::of(0x7fff_0001), Action::Allow);
    let data = SeccompData { nr: 1, arch: 2, instruction_pointer: 0x1_0000_0003, args: [0x5_0000_0004, 0, 0, 0, 0, 9] };
    let w = data.words();
    assert_eq!((w[0], w[1], w[2], w[3], w[4], w[5], w[14]), (1, 2, 3, 1, 4, 5, 9));
}

#[test]
fn the_instruction_count_of_a_thread_is_limited() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    let big = vec![ret(SECCOMP_RET_ALLOW); 4096];
    // 7 filters: 7 * 4096 + 6 * 4 = 28696; the 8th would make 32796.
    for _ in 0..7 {
        assert_eq!(install(&mut k, &big, 0), Ok(0));
    }
    assert_eq!(install(&mut k, &big, 0), Err(Errno::ENOMEM));
    assert_eq!(install(&mut k, &big[..4000], 0), Ok(0));
}

#[test]
fn tsync_gives_every_thread_the_filter_unless_one_diverged() {
    let mut k = FakeKernel::new();
    k.add_thread(101);
    k.add_thread(102);
    no_new_privs(&mut k);
    assert_eq!(install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), SECCOMP_FILTER_FLAG_TSYNC), Ok(0));
    let mine = k.thread_state().seccomp.clone();
    for t in [101, 102] {
        let s = &k.me().thread(t).seccomp;
        assert_eq!(*s, mine, "thread {t} synchronised");
        assert!(k.me().thread(t).creds.no_new_privs, "no_new_privs went with the filter");
    }
    // Thread 102 adds a filter of its own: it no longer descends from the
    // caller's tree, and TSYNC names it (or fails with ESRCH).
    k.as_thread(102, |k| install(k, &[ret(SECCOMP_RET_ALLOW)], 0).unwrap());
    let before = k.thread_state().seccomp.clone();
    let insns = for_call(nr::GETTID, SECCOMP_RET_ERRNO | 2);
    assert_eq!(install(&mut k, &insns, SECCOMP_FILTER_FLAG_TSYNC), Ok(102));
    assert_eq!(k.thread_state().seccomp, before, "nothing installed");
    assert_eq!(install(&mut k, &insns, SECCOMP_FILTER_FLAG_TSYNC | SECCOMP_FILTER_FLAG_TSYNC_ESRCH), Err(Errno::ESRCH));
    // Thread 102 itself can synchronise the others to its tree: the
    // caller's filters are ancestors of it.
    assert_eq!(k.as_thread(102, |k| install(k, &insns, SECCOMP_FILTER_FLAG_TSYNC)), Ok(0));
    let me = k.me();
    assert_eq!(me.thread(100).seccomp, me.thread(102).seccomp);
}

#[test]
fn strict_mode_allows_four_calls_and_kills_for_the_rest() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::SECCOMP, &[0, 1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SECCOMP, &[0, 0, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SECCOMP, &[0, 0, 0]), Ok(0));
    assert_eq!(k.thread_state().seccomp, Seccomp::Strict);
    assert_eq!(sys(&mut k, nr::WRITE, &[1, scratch(0), 3]), Ok(3));
    assert_eq!(k.exit, None);
    call(&mut k, nr::GETPID, &[]);
    assert_eq!(k.exit, Some(Exit::Signal(SIGKILL, false)));
    // exit_group is not exit.
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::PRCTL, &[u64::from(PR_SET_SECCOMP), u64::from(SECCOMP_MODE_STRICT)]), Ok(0));
    call(&mut k, nr::EXIT_GROUP, &[0]);
    assert_eq!(k.exit, Some(Exit::Signal(SIGKILL, false)));
    let mut k = FakeKernel::new();
    sys(&mut k, nr::SECCOMP, &[0, 0, 0]).unwrap();
    assert_eq!(call(&mut k, nr::EXIT, &[3]).0, Outcome::Handled);
    assert_eq!(k.exit, Some(Exit::Thread(3)));
}

#[test]
fn strict_and_filter_modes_exclude_each_other() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &[ret(SECCOMP_RET_ALLOW)], 0).unwrap();
    assert_eq!(sys(&mut k, nr::SECCOMP, &[0, 0, 0]), Err(Errno::EINVAL));
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    k.me().thread_mut(100).seccomp = Seccomp::Strict;
    k.seccomp_on = false;
    assert_eq!(install(&mut k, &[ret(SECCOMP_RET_ALLOW)], 0), Err(Errno::EINVAL));
}

#[test]
fn prctl_sets_and_reports_the_mode() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::PRCTL, &[u64::from(PR_GET_SECCOMP)]), Ok(0));
    no_new_privs(&mut k);
    let fprog = put_program(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1));
    assert_eq!(sys(&mut k, nr::PRCTL, &[u64::from(PR_SET_SECCOMP), u64::from(SECCOMP_MODE_FILTER), fprog]), Ok(0));
    assert_eq!(sys(&mut k, nr::PRCTL, &[u64::from(PR_GET_SECCOMP)]), Ok(2));
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::PRCTL, &[u64::from(PR_SET_SECCOMP), 3]), Err(Errno::EINVAL));
}

#[test]
fn action_availability_is_reported() {
    let mut k = FakeKernel::new();
    let at = scratch(0x40);
    for (action, ok) in [
        (SECCOMP_RET_KILL_PROCESS, true),
        (SECCOMP_RET_KILL_THREAD, true),
        (SECCOMP_RET_TRAP, true),
        (SECCOMP_RET_ERRNO, true),
        (SECCOMP_RET_TRACE, true),
        (SECCOMP_RET_LOG, true),
        (SECCOMP_RET_ALLOW, true),
        (SECCOMP_RET_USER_NOTIF, true),
        (0x1234_0000, false),
    ] {
        k.poke(at, &action.to_le_bytes());
        let got = sys(&mut k, nr::SECCOMP, &[u64::from(SECCOMP_GET_ACTION_AVAIL), 0, at]);
        assert_eq!(got, if ok { Ok(0) } else { Err(Errno::EOPNOTSUPP) }, "{action:#x}");
    }
    assert_eq!(sys(&mut k, nr::SECCOMP, &[2, 1, at]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SECCOMP, &[2, 0, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SECCOMP, &[3, 0, at]), Ok(0));
    assert_eq!(k.peek(at, 6), [80, 0, 24, 0, 64, 0]);
}

#[test]
fn filters_are_inherited_by_fork_and_threads_and_kept_by_exec() {
    let mut k = FakeKernel::new();
    no_new_privs(&mut k);
    install(&mut k, &for_call(nr::GETPID, SECCOMP_RET_ERRNO | 1), 0).unwrap();
    let mine = k.thread_state().seccomp.clone();
    let child: LinuxProcess = k.me().fork(100, 200);
    assert_eq!(child.thread(200).seccomp, mine);
    k.add_thread(101);
    assert_eq!(k.me().thread(101).seccomp, mine);
    k.me().exec(101);
    assert_eq!(k.me().thread(101).seccomp, mine);
}
