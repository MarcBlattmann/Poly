//! In-kernel tests of time (ADR 0044): sleeps, clocks and timers through
//! real system calls of Linux processes, timer signals delivered to
//! threads running in ring 3, CPU-time accounting, and `/bin/time-test`
//! (the vDSO from ring 3), once as the clock decides and once with the
//! vDSO forced onto the TSC.

use super::{CODE, DATA, expect_exit, run_callback, setup, sys, wait};
use crate::fs;
use crate::linux::timers;
use crate::process::ExitStatus;
use crate::program;
use crate::time::{self, NANOS_PER_MILLI, NANOS_PER_SEC};
use crate::user::{self, Entry, Memory};
use crate::{kernel_test, kprintln, vdso};
use alloc::sync::Arc;
use core::arch::global_asm;
use poly_personality_linux::abi::signal::{SA_RESTORER, SIGALRM, SIGUSR1, SIGVTALRM};
use poly_personality_linux::{Errno, nr};

type TestResult<T = ()> = Result<T, &'static str>;

/// Where these programs are mapped (after the other tests' programs).
const PROGRAMS: u64 = CODE + 0x10_0000;

global_asm!(
    r#"
    .section .rodata.linux_time_test_programs, "a", @progbits
    .balign 16
    .global linux_time_test_start
linux_time_test_start:

/* setitimer(rsi = which, rdi = &itimerval, NULL), then spin in ring 3 until
   a signal ends the process; exit_group(99) if setitimer failed. */
    .global linux_time_test_itimer_spin
linux_time_test_itimer_spin:
    xchg rdi, rsi
    xor edx, edx
    mov eax, 38
    syscall
    test rax, rax
    jnz 2f
1:
    jmp 1b
2:
    mov edi, 99
    mov eax, 231
    syscall
    ud2

    .balign 16
    .global linux_time_test_end
linux_time_test_end:
"#
);

unsafe extern "C" {
    static linux_time_test_start: u8;
    static linux_time_test_end: u8;
    static linux_time_test_itimer_spin: u8;
}

/// Map this file's programs into `memory` at [`PROGRAMS`].
fn map_programs(memory: &Memory) -> TestResult {
    use crate::user::memory::{Placement, Prot};
    let start = &raw const linux_time_test_start;
    let len = (&raw const linux_time_test_end) as usize - start as usize;
    // SAFETY: the assembly above puts the programs between the two
    // symbols, in one read-only section of the kernel image.
    let code = unsafe { core::slice::from_raw_parts(start, len) };
    let pages = poly_vm::page_up(len as u64).expect("small");
    let ok = memory.map_anonymous(Placement::FixedNoReplace(PROGRAMS), pages, Prot::RW).is_ok()
        && memory.write(PROGRAMS, code).is_ok()
        && memory.protect(PROGRAMS, pages, Prot::RX).is_ok();
    if ok { Ok(()) } else { Err("could not map the time test programs") }
}

fn program(label: *const u8) -> u64 {
    PROGRAMS + (label as u64 - (&raw const linux_time_test_start) as u64)
}

/// `struct itimerval` with a value of `value_us` and no interval.
fn itimerval(value_us: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[16..24].copy_from_slice(&(value_us / 1_000_000).to_le_bytes());
    b[24..32].copy_from_slice(&(value_us % 1_000_000).to_le_bytes());
    b
}

/// A process whose one thread arms interval timer `which` for `value_us`
/// and spins in ring 3: how it ended.
fn spin_with_itimer(which: u64, value_us: u64) -> TestResult<ExitStatus> {
    let (mut process, mut memory) = setup()?;
    if process.pid == 1 {
        // Process 1 gets only handled signals (kill(2)); this test wants the
        // default action.
        (process, memory) = setup()?;
    }
    map_programs(&memory)?;
    memory.write(DATA, &itimerval(value_us)).map_err(|_| "writing the data page failed")?;
    let entry = Entry {
        rip: program(&raw const linux_time_test_itimer_spin),
        rsp: memory.layout().stack_top - 64,
        args: [DATA, which, 0],
    };
    let tid = user::spawn_in(Arc::clone(&process), Arc::clone(&memory), entry, user::DEFAULT_HOOK)
        .map_err(|_| "spawn failed")?;
    let _ = crate::thread::detach(tid);
    wait(&process)
}

kernel_test! {
    fn itimer_real_kills_a_spinning_process_with_sigalrm() -> TestResult {
        let before = timers::fired();
        let status = spin_with_itimer(0, 20_000)?;
        if status != (ExitStatus::Signaled { signal: SIGALRM, core: false }) {
            kprintln!("linux time test: ITIMER_REAL: {status}");
            return Err("SIGALRM from ITIMER_REAL did not end the process");
        }
        if timers::fired() == before {
            return Err("the timer thread fired nothing");
        }
        Ok(())
    }
}

kernel_test! {
    fn itimer_virtual_counts_the_cpu_time_of_a_spinning_process() -> TestResult {
        let status = spin_with_itimer(1, 30_000)?;
        if status != (ExitStatus::Signaled { signal: SIGVTALRM, core: false }) {
            kprintln!("linux time test: ITIMER_VIRTUAL: {status}");
            return Err("SIGVTALRM from ITIMER_VIRTUAL did not end the process");
        }
        Ok(())
    }
}

/// Write a timespec at user address `at`.
fn put_timespec(at: u64, ns: u64) -> bool {
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&(ns / NANOS_PER_SEC).to_le_bytes());
    b[8..16].copy_from_slice(&(ns % NANOS_PER_SEC).to_le_bytes());
    user::copy_to_user(at, &b).is_ok()
}

fn get_timespec(at: u64) -> u64 {
    let mut b = [0u8; 16];
    let _ = user::copy_from_user(&mut b, at);
    let sec = u64::from_le_bytes(b[0..8].try_into().expect("8 bytes"));
    let nsec = u64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
    sec * NANOS_PER_SEC + nsec
}

kernel_test! {
    fn sleeps_last_at_least_as_long_as_asked() -> TestResult {
        let status = run_callback(|p| {
            let ts = DATA + 0x100;
            // nanosleep
            check!(1, put_timespec(ts, 30 * NANOS_PER_MILLI));
            let t0 = time::now_ns();
            check!(2, sys(p, nr::NANOSLEEP, &[ts, 0]) == Ok(0));
            check!(3, time::now_ns() - t0 >= 30 * NANOS_PER_MILLI);
            // clock_nanosleep, absolute, on the monotonic and the wall clock
            let clocks: [(u64, fn() -> u64); 2] = [(1, time::now_ns), (0, time::realtime_ns)];
            for (clock, now) in clocks {
                let deadline = now() + 20 * NANOS_PER_MILLI;
                check!(4, put_timespec(ts, deadline));
                check!(5, sys(p, nr::CLOCK_NANOSLEEP, &[clock, 1, ts, 0]) == Ok(0));
                check!(6, now() >= deadline);
            }
            // clock_gettime through the system call agrees with the kernel.
            let before = time::now_ns();
            check!(7, sys(p, nr::CLOCK_GETTIME, &[1, ts]) == Ok(0));
            let got = get_timespec(ts);
            check!(8, got >= before && got <= time::now_ns());
            0
        })?;
        expect_exit(status, 0, "sleep checks failed")
    }
}

kernel_test! {
    fn a_posix_timer_signal_arrives_through_the_timer_thread() -> TestResult {
        let status = run_callback(|p| {
            let (set, ev, id, spec, info, ts) = (DATA, DATA + 0x40, DATA + 0x80, DATA + 0x100, DATA + 0x200, DATA + 0x300);
            // A handler (the process may be process 1, which only gets
            // handled signals) that never runs: SIGUSR1 is blocked, so it
            // stays pending for rt_sigtimedwait.
            let mut act = [0u8; 32];
            act[0..8].copy_from_slice(&CODE.to_le_bytes());
            act[8..16].copy_from_slice(&SA_RESTORER.to_le_bytes());
            act[16..24].copy_from_slice(&CODE.to_le_bytes());
            check!(1, user::copy_to_user(set, &act).is_ok());
            check!(1, sys(p, nr::RT_SIGACTION, &[u64::from(SIGUSR1), set, 0, 8]) == Ok(0));
            check!(1, user::copy_to_user(set, &(1u64 << (SIGUSR1 - 1)).to_le_bytes()).is_ok());
            check!(2, sys(p, nr::RT_SIGPROCMASK, &[0, set, 0, 8]) == Ok(0));
            // SIGEV_SIGNAL with SIGUSR1 and a value.
            let mut sigev = [0u8; 64];
            sigev[0..8].copy_from_slice(&0x1234u64.to_le_bytes());
            sigev[8..12].copy_from_slice(&(SIGUSR1 as i32).to_le_bytes());
            check!(3, user::copy_to_user(ev, &sigev).is_ok());
            check!(4, sys(p, nr::TIMER_CREATE, &[1, ev, id]) == Ok(0));
            let mut timer = [0u8; 4];
            check!(5, user::copy_from_user(&mut timer, id).is_ok());
            let timer = u32::from_le_bytes(timer) as u64;
            // In 10 ms.
            check!(6, user::copy_to_user(spec, &[0u8; 32]).is_ok() && put_timespec(spec + 16, 10 * NANOS_PER_MILLI));
            check!(7, sys(p, nr::TIMER_SETTIME, &[timer, 0, spec, 0]) == Ok(0));
            check!(8, put_timespec(ts, 10 * NANOS_PER_SEC));
            check!(9, sys(p, nr::RT_SIGTIMEDWAIT, &[set, info, ts, 8]) == Ok(u64::from(SIGUSR1)));
            let mut si = [0u8; 32];
            check!(10, user::copy_from_user(&mut si, info).is_ok());
            let code = i32::from_le_bytes(si[8..12].try_into().unwrap());
            let tid = u32::from_le_bytes(si[16..20].try_into().unwrap()) as u64;
            let value = u64::from_le_bytes(si[24..32].try_into().unwrap());
            check!(11, code == -2 && tid == timer && value == 0x1234);
            check!(12, sys(p, nr::TIMER_DELETE, &[timer]) == Ok(0));
            check!(13, sys(p, nr::TIMER_DELETE, &[timer]) == Err(Errno::EINVAL));
            0
        })?;
        expect_exit(status, 0, "POSIX timer checks failed")
    }
}

kernel_test! {
    fn cpu_time_clocks_count_a_busy_thread() -> TestResult {
        let status = run_callback(|p| {
            let ts = DATA + 0x100;
            // The clock time brackets both CPU-clock readings: the thread
            // runs, and its CPU clock counts, between this and the first
            // reading too (a busy host may hold the emulator off for many
            // milliseconds just there).
            let wall0 = time::now_ns();
            check!(1, sys(p, nr::CLOCK_GETTIME, &[3, ts]) == Ok(0));
            let thread0 = get_timespec(ts);
            // Busy until the thread's own CPU clock says it ran 20 ms. How
            // much clock time that takes depends on the other threads and on
            // the host (one that holds the emulator off while another thread
            // is switched in charges that time to it), so the clock time is
            // only bounded generously: a clock that does not count, or counts
            // a fraction of the time, misses the deadline. It can never count
            // more than the time that passed.
            const RUN: u64 = 20 * NANOS_PER_MILLI;
            const BUDGET: u64 = 10 * NANOS_PER_SEC;
            let ran = loop {
                check!(2, sys(p, nr::CLOCK_GETTIME, &[3, ts]) == Ok(0));
                let ran = get_timespec(ts) - thread0;
                if ran >= RUN || time::now_ns() - wall0 > BUDGET {
                    break ran;
                }
                for _ in 0..1000 {
                    core::hint::spin_loop();
                }
            };
            let wall = time::now_ns() - wall0;
            if ran < RUN || ran > wall {
                kprintln!("linux test: the thread CPU clock counted {ran} ns in {wall} ns");
            }
            check!(3, ran >= RUN && ran <= wall);
            check!(4, sys(p, nr::CLOCK_GETTIME, &[2, ts]) == Ok(0));
            check!(5, get_timespec(ts) >= ran);
            // getrusage: busy in the kernel (this callback runs inside a
            // system call), so it is system time (the ring-3 code around
            // the call is a few instructions).
            let ru = DATA + 0x200;
            check!(6, sys(p, nr::GETRUSAGE, &[0, ru]) == Ok(0));
            let mut b = [0u8; 32];
            check!(7, user::copy_from_user(&mut b, ru).is_ok());
            let us = |at: usize| {
                u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) * 1_000_000
                    + u64::from_le_bytes(b[at + 8..at + 16].try_into().unwrap())
            };
            let (utime_ns, stime_ns) = (us(0) * 1000, us(16) * 1000);
            if stime_ns < ran / 2 {
                kprintln!("linux test: {ran} ns busy in the kernel: user {utime_ns} ns, system {stime_ns} ns");
            }
            check!(8, stime_ns >= ran / 2);
            0
        })?;
        expect_exit(status, 0, "CPU time checks failed")
    }
}

kernel_test! {
    fn time_test_runs_from_ring_3(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(fs::TEST_INITRD_FLAG) {
            return Ok(());
        }
        let path: &[u8] = b"/bin/time-test";
        let run = || -> TestResult {
            let process = program::exec_path(path, &[path], &[], false).map_err(|_| "exec_path /bin/time-test failed")?;
            expect_exit(wait(&process)?, 0, "/bin/time-test reported failures")
        };
        run()?;
        // Again with the vDSO computing the time from the TSC itself (which
        // the clock does not use under QEMU's TCG).
        let forced = vdso::force_tsc_for_tests(true);
        let result = run();
        vdso::force_tsc_for_tests(false);
        if !forced {
            kprintln!("linux time test: no calibrated TSC, the vDSO's TSC path was not run");
        }
        result
    }
}
