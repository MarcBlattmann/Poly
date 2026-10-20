//! exit, exit_group, ids, set_tid_address, arch_prctl, rt_sigaction,
//! rt_sigprocmask, uname, clock_gettime, getrandom.

use super::fake::*;
use crate::abi::{SigAction, Timespec, UTS_FIELD_LEN, arch_prctl as op, clock, random, signal};
use crate::kernel::SigSet;
use crate::sys::system::{UNAME_RELEASE, clock_for};
use crate::{Errno, nr};

#[test]
fn exit_ends_the_thread_and_exit_group_the_process() {
    let mut k = FakeKernel::new();
    assert!(sys(&mut k, nr::EXIT, &[3]).is_ok());
    assert_eq!(k.exit, Some(Exit::Thread(3)));
    assert!(sys(&mut k, nr::EXIT_GROUP, &[0x1_0000_0101]).is_ok());
    assert_eq!(k.exit, Some(Exit::Group(1)), "status & 0377");
    assert!(sys(&mut k, nr::EXIT_GROUP, &[u64::MAX]).is_ok());
    assert_eq!(k.exit, Some(Exit::Group(255)), "exit(-1) is status 255");
}

#[test]
fn id_getters() {
    let mut k = FakeKernel::new();
    k.tid = 101;
    assert_eq!(sys(&mut k, nr::GETPID, &[]), Ok(100));
    assert_eq!(sys(&mut k, nr::GETTID, &[]), Ok(101));
    assert_eq!(sys(&mut k, nr::GETPPID, &[]), Ok(1));
    assert_eq!(sys(&mut k, nr::GETUID, &[]), Ok(1000));
    assert_eq!(sys(&mut k, nr::GETEUID, &[]), Ok(1001));
    assert_eq!(sys(&mut k, nr::GETGID, &[]), Ok(2000));
    assert_eq!(sys(&mut k, nr::GETEGID, &[]), Ok(2001));
    k.ppid = 0;
    assert_eq!(sys(&mut k, nr::GETPPID, &[]), Ok(0));
}

#[test]
fn set_tid_address_records_the_pointer_and_returns_the_tid() {
    let mut k = FakeKernel::new();
    k.tid = 123;
    assert_eq!(sys(&mut k, nr::SET_TID_ADDRESS, &[0x4005_0650]), Ok(123));
    assert_eq!(k.clear_child_tid.0, 0x4005_0650);
    assert_eq!(sys(&mut k, nr::SET_TID_ADDRESS, &[UNMAPPED]), Ok(123), "never fails, not even on a bad pointer");
    assert_eq!(sys(&mut k, nr::SET_TID_ADDRESS, &[0]), Ok(123));
    assert!(k.clear_child_tid.is_null());
}

#[test]
fn arch_prctl_sets_and_gets_fs_and_gs() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_SET_FS as u64, 0x4c_c380]), Ok(0));
    assert_eq!(k.fs_base, 0x4c_c380);
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_SET_GS as u64, 0x7fff_ffff_ffff]), Ok(0));
    assert_eq!(k.gs_base, 0x7fff_ffff_ffff);
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_GET_FS as u64, scratch(0)]), Ok(0));
    assert_eq!(k.peek_u64(scratch(0)), 0x4c_c380);
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_GET_GS as u64, scratch(8)]), Ok(0));
    assert_eq!(k.peek_u64(scratch(8)), 0x7fff_ffff_ffff);
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_SET_FS as u64, 0]), Ok(0), "0 is a valid base");
}

#[test]
fn arch_prctl_errors() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_SET_FS as u64, 0x8000_0000_0000]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_SET_GS as u64, 0xffff_8000_0000_0000]), Err(Errno::EPERM));
    assert_eq!(k.fs_base, 0, "unchanged");
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_GET_FS as u64, UNMAPPED]), Err(Errno::EFAULT));
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[op::ARCH_GET_GS as u64, ro]), Err(Errno::EFAULT));
    for bad in [0, 0x1000, 0x1005, 0x1011, 0x1012, 0x2001, 0x3001, u64::MAX] {
        assert_eq!(sys(&mut k, nr::ARCH_PRCTL, &[bad, scratch(0)]), Err(Errno::EINVAL), "op {bad:#x}");
    }
}

fn put_action(k: &mut FakeKernel, at: u64, a: SigAction) -> u64 {
    k.poke(at, &a.to_bytes());
    at
}

fn get_action(k: &FakeKernel, at: u64) -> SigAction {
    SigAction::from_bytes(&k.peek(at, SigAction::SIZE).try_into().unwrap())
}

#[test]
fn rt_sigaction_records_and_reports_actions() {
    let mut k = FakeKernel::new();
    let ign = SigAction {
        handler: signal::SIG_IGN,
        flags: signal::SA_RESTORER | signal::SA_RESTART,
        restorer: 0x40_1234,
        mask: SigSet::only(signal::SIGPIPE).0,
    };
    let act = put_action(&mut k, scratch(0), ign);
    k.poke(scratch(0x40), &[0xee; 32]);
    let sigpipe = signal::SIGPIPE as u64;
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[sigpipe, act, scratch(0x40), 8]), Ok(0));
    assert_eq!(get_action(&k, scratch(0x40)), SigAction::default(), "old action: SIG_DFL");
    assert_eq!(k.actions.get(signal::SIGPIPE), ign);
    // Query only.
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[sigpipe, 0, scratch(0x80), 8]), Ok(0));
    assert_eq!(get_action(&k, scratch(0x80)), ign);
    // Both null is allowed.
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[signal::SIGSEGV as u64, 0, 0, 8]), Ok(0));
    // Signal 64 is the last one.
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[64, act, 0, 8]), Ok(0));
}

#[test]
fn rt_sigaction_errors() {
    let mut k = FakeKernel::new();
    let act = put_action(&mut k, scratch(0), SigAction { handler: 0x40_0000, ..Default::default() });
    for sig in [0, 65, u64::MAX] {
        assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[sig, act, 0, 8]), Err(Errno::EINVAL), "signal {sig}");
    }
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[2, act, 0, 4]), Err(Errno::EINVAL), "sigsetsize");
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[2, act, 0, 16]), Err(Errno::EINVAL), "sigsetsize");
    for sig in [signal::SIGKILL, signal::SIGSTOP] {
        assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[sig as u64, act, 0, 8]), Err(Errno::EINVAL));
        assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[sig as u64, 0, scratch(0x40), 8]), Ok(0), "query is fine");
    }
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[2, UNMAPPED, 0, 8]), Err(Errno::EFAULT));
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::RT_SIGACTION, &[2, act, ro, 8]), Err(Errno::EFAULT));
    assert_eq!(k.actions.get(2), SigAction::default(), "an EFAULT changes nothing");
}

#[test]
fn rt_sigprocmask_block_unblock_setmask() {
    let mut k = FakeKernel::new();
    let set = |k: &mut FakeKernel, bits: u64| {
        k.poke(scratch(0), &bits.to_le_bytes());
        scratch(0)
    };
    let s = set(&mut k, 0b1010);
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[signal::SIG_BLOCK as u64, s, scratch(8), 8]), Ok(0));
    assert_eq!(k.peek_u64(scratch(8)), 0);
    assert_eq!(k.mask, SigSet(0b1010));
    let s = set(&mut k, 0b0010);
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[signal::SIG_UNBLOCK as u64, s, scratch(8), 8]), Ok(0));
    assert_eq!(k.peek_u64(scratch(8)), 0b1010);
    assert_eq!(k.mask, SigSet(0b1000));
    let s = set(&mut k, u64::MAX);
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[signal::SIG_SETMASK as u64, s, 0, 8]), Ok(0));
    assert_eq!(k.mask, SigSet(!SigSet::UNBLOCKABLE.0), "SIGKILL and SIGSTOP silently stay unblocked");
    assert!(!k.mask.contains(signal::SIGKILL) && !k.mask.contains(signal::SIGSTOP) && k.mask.contains(64));
    // set == NULL: how is ignored, the mask is only reported.
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[99, 0, scratch(8), 8]), Ok(0));
    assert_eq!(k.peek_u64(scratch(8)), !SigSet::UNBLOCKABLE.0);
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[99, 0, 0, 8]), Ok(0));
}

#[test]
fn rt_sigprocmask_errors() {
    let mut k = FakeKernel::new();
    k.poke(scratch(0), &1u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[3, scratch(0), 0, 8]), Err(Errno::EINVAL), "how");
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[0, scratch(0), 0, 4]), Err(Errno::EINVAL), "size");
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[0, 0, 0, 0]), Err(Errno::EINVAL), "size without sets");
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[0, UNMAPPED, 0, 8]), Err(Errno::EFAULT));
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::RT_SIGPROCMASK, &[0, scratch(0), ro, 8]), Err(Errno::EFAULT));
    assert_eq!(k.mask, SigSet::EMPTY, "nothing changed");
}

#[test]
fn uname_fills_new_utsname() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::UNAME, &[scratch(0)]), Ok(0));
    let b = k.peek(scratch(0), 6 * UTS_FIELD_LEN);
    let field = |i: usize| {
        let f = &b[i * UTS_FIELD_LEN..(i + 1) * UTS_FIELD_LEN];
        let end = f.iter().position(|&c| c == 0).expect("NUL-terminated");
        core::str::from_utf8(&f[..end]).unwrap()
    };
    assert_eq!(field(0), "Linux");
    assert_eq!(field(1), "testhost");
    assert_eq!(field(2), UNAME_RELEASE);
    assert_eq!(field(3), "#1 Poly");
    assert_eq!(field(4), "x86_64");
    assert_eq!(field(5), "(none)");
    let mut parts = UNAME_RELEASE.split(['.', '-']).map(|p| p.parse::<u32>());
    assert!(parts.next().unwrap().unwrap() >= 3, "a release C libraries accept");
    assert_eq!(sys(&mut k, nr::UNAME, &[UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn clock_gettime_reads_the_kernel_clocks() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[clock::CLOCK_MONOTONIC as u64, scratch(0)]), Ok(0));
    assert_eq!(k.peek(scratch(0), 16), Timespec { sec: 42, nsec: 999_999_999 }.to_bytes());
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[clock::CLOCK_REALTIME as u64, scratch(0)]), Ok(0));
    assert_eq!(k.peek(scratch(0), 16), Timespec { sec: 1_700_000_000, nsec: 123 }.to_bytes());
    for id in [clock::CLOCK_MONOTONIC_RAW, clock::CLOCK_MONOTONIC_COARSE, clock::CLOCK_BOOTTIME] {
        assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[id as u64, scratch(0)]), Ok(0));
        assert_eq!(le_sec(&k), 42);
    }
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[clock::CLOCK_REALTIME_COARSE as u64, scratch(0)]), Ok(0));
    assert_eq!(le_sec(&k), 1_700_000_000);
}

fn le_sec(k: &FakeKernel) -> u64 {
    k.peek_u64(scratch(0))
}

#[test]
fn clock_gettime_errors() {
    let mut k = FakeKernel::new();
    for id in [8u64, 9, 10, 11, 12, u64::MAX, (-6i64) as u64] {
        assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[id, scratch(0)]), Err(Errno::EINVAL), "clock {id}");
    }
    assert_eq!(clock_for(clock::CLOCK_PROCESS_CPUTIME_ID), Some(crate::kernel::Clock::ProcessCpuTime));
    let cpu = clock::CLOCK_THREAD_CPUTIME_ID as u64;
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[cpu, scratch(0)]), Err(Errno::EINVAL), "the fake has no CPU clocks");
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[1, UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn getrandom_fills_the_buffer() {
    let mut k = FakeKernel::new();
    let nonblock = random::GRND_NONBLOCK as u64;
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[scratch(0), 8, nonblock]), Ok(8));
    assert_eq!(k.peek(scratch(0), 8), [0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[scratch(0), 1000, 0]), Ok(1000), "several 256-byte blocks");
    assert_eq!(k.peek(scratch(999), 1), [((8 + 999) % 256) as u8]);
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[UNMAPPED, 0, 0]), Ok(0));
    let all = (random::GRND_NONBLOCK | random::GRND_RANDOM | random::GRND_INSECURE) as u64;
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[scratch(0), 4, all]), Ok(4));
}

#[test]
fn getrandom_errors() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[scratch(0), 8, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[UNMAPPED, 8, 0]), Err(Errno::EFAULT));
    k.random_ready = false;
    let nonblock = random::GRND_NONBLOCK as u64;
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[scratch(0), 8, nonblock]), Err(Errno::EAGAIN));
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[scratch(0), 8, random::GRND_INSECURE as u64]), Ok(8), "never blocks");
    // A fault after the first block returns what was copied.
    k.map_range(0x100_0000, 4096, RW);
    assert_eq!(sys(&mut k, nr::GETRANDOM, &[0x100_0000 + 4096 - 256, 1000, 0]), Ok(256));
}
