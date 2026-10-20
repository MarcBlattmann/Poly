//! In-kernel tests of user mode: real ring-3 code (the hand-assembled
//! programs in `programs.rs`), real system calls, real faults.
//!
//! Every test builds an address space with the programs at [`CODE`]
//! (read and execute), a zeroed data page at [`DATA`] and a stack below
//! [`STACK_TOP`], starts user threads in it with the test [`hook`] (or the
//! default one) and joins them: a program's exit code, or [`killed`] for a
//! thread that faulted, says what happened.

use super::programs::{self, BAD, Call, Program, RECORD_RESULT};
use super::*;
use crate::arch::user::LOWER_HALF_END;
use crate::kernel_test;
use crate::sync::SpinLock;
use crate::thread::State;
use crate::time;
use core::time::Duration;
use poly_abi::Personality;
use poly_binfmt::{Arch, Bits, Format};
use poly_mem::{PAGE_SIZE, align_up};

/// Where the programs are mapped (read, execute).
const CODE: u64 = 0x40_0000;
/// One zeroed read-write page.
const DATA: u64 = 0x60_0000;
/// The stack: [`STACK_PAGES`] read-write pages below this.
const STACK_TOP: u64 = 0x7fff_0000_0000;
const STACK_PAGES: u64 = 4;
/// Never mapped.
const UNMAPPED: u64 = 0x5000_0000;
/// The process id the test threads run under.
const PID: u32 = 4242;

/// Vectors of the exceptions the tests provoke.
const UD: u8 = 6;
const GP: u8 = 13;
const PF: u8 = 14;

/// How a thread that touches a non-canonical address ends: #GP on real
/// CPUs and under KVM; QEMU's TCG does not check canonicality and treats
/// the address as if sign-extended, so it is a page fault there.
fn non_canonical(code: usize) -> bool {
    code == killed(GP) || code == killed(PF)
}

fn process() -> Arc<Process> {
    Arc::new(Process::new(PID, Personality::Linux, Format::Elf { bits: Bits::B64, arch: Arch::X86_64 }))
}

/// A fresh address space with the programs, the data page and a stack.
fn new_space() -> Result<Arc<AddressSpace>, &'static str> {
    let space = AddressSpace::new().map_err(|_| "no memory for an address space")?;
    let code = programs::blob();
    let len = align_up(code.len() as u64, PAGE_SIZE);
    let stack = STACK_PAGES * PAGE_SIZE;
    let mapped = space
        .map_zeroed(CODE, len, Prot::RW)
        .and_then(|()| space.write(CODE, code))
        .and_then(|()| space.protect(CODE, len, Prot::RX))
        .and_then(|()| space.map_zeroed(DATA, PAGE_SIZE, Prot::RW))
        .and_then(|()| space.map_zeroed(STACK_TOP - stack, stack, Prot::RW));
    mapped.map_err(|_| "could not set up the test address space")?;
    Ok(Arc::new(space))
}

fn start_with(space: &Arc<AddressSpace>, program: Program, args: [u64; 3], hook: SyscallHook) -> TestResult<ThreadId> {
    let entry = Entry { rip: CODE + program.offset(), rsp: STACK_TOP, args };
    spawn(process(), Arc::clone(space), entry, hook).map_err(|_| "could not spawn a user thread")
}

fn start(space: &Arc<AddressSpace>, program: Program, args: [u64; 3]) -> TestResult<ThreadId> {
    start_with(space, program, args, hook)
}

/// Run `program` to its end and return its exit code.
fn run(space: &Arc<AddressSpace>, program: Program, args: [u64; 3]) -> TestResult<usize> {
    let t = start(space, program, args)?;
    thread::join(t).map_err(|_| "join failed")
}

type TestResult<T = ()> = Result<T, &'static str>;

fn read_u64(space: &AddressSpace, virt: u64) -> TestResult<u64> {
    let mut b = [0u8; 8];
    space.read(virt, &mut b).map_err(|_| "user page not mapped")?;
    Ok(u64::from_le_bytes(b))
}

fn write_u64(space: &AddressSpace, virt: u64, value: u64) -> TestResult {
    space.write(virt, &value.to_le_bytes()).map_err(|_| "user page not mapped")
}

/// The registers of the last [`Call::Record`].
static RECORDED: SpinLock<Option<SyscallRegs>> = SpinLock::new(None);
/// What [`Call::Callback`] runs.
static CALLBACK: SpinLock<Option<fn() -> usize>> = SpinLock::new(None);

/// The test programs' system calls (see [`Call`]).
fn hook(_: &Process, regs: &mut SyscallRegs) {
    let call = regs.rax;
    let is = |c: Call| call == c as u64;
    if is(Call::Record) {
        *RECORDED.lock() = Some(*regs);
        regs.rax = RECORD_RESULT;
    } else if is(Call::Exit) {
        exit(regs.rdi as usize);
    } else if is(Call::Yield) {
        thread::yield_now();
        regs.rax = 0;
    } else if is(Call::SetFs) {
        regs.rax = u64::from(set_fs_base(regs.rdi).is_err());
    } else if is(Call::SetGs) {
        regs.rax = u64::from(set_gs_base(regs.rdi).is_err());
    } else if is(Call::ReturnTo) {
        set_return_address(regs.rdi).expect("a user thread may set its return address");
    } else if is(Call::FailReturn) {
        crate::arch::user::make_next_return_fail();
    } else if is(Call::Callback) {
        let f = CALLBACK.lock().expect("no callback set");
        exit(f());
    } else {
        regs.rax = u64::MAX;
    }
}

/// Wait (briefly) until the space's only reference is the test's: the
/// threads that used it have been reaped.
fn released(space: &Arc<AddressSpace>) -> TestResult {
    for _ in 0..100 {
        if Arc::strong_count(space) == 1 {
            return Ok(());
        }
        thread::yield_now();
    }
    Err("a user thread kept its address space after it exited")
}

kernel_test! {
    fn syscall_passes_registers_and_returns() -> TestResult {
        let space = new_space()?;
        *RECORDED.lock() = None;
        match run(&space, Program::Registers, [0; 3])? {
            BAD => return Err("callee-saved registers, RSP or the return address changed across a system call"),
            code if code as u64 != RECORD_RESULT => return Err("the system call's result did not reach user mode"),
            _ => {}
        }
        let r = RECORDED.lock().ok_or("the system call never reached the hook")?;
        let args = [r.rax, r.rdi, r.rsi, r.rdx, r.r10, r.r8, r.r9];
        if args != [Call::Record as u64, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60] {
            kprintln!("user: recorded {r:x?}");
            return Err("the system call number or arguments arrived wrong");
        }
        if r.rsp != STACK_TOP || r.rflags & (1 << 9) == 0 {
            return Err("the user RSP or RFLAGS arrived wrong");
        }
        released(&space)
    }
}

kernel_test! {
    fn default_hook_routes_to_the_personality() -> TestResult {
        // A number no Linux system call has: the personality answers
        // -ENOSYS (and logs it), the program stores that and traps.
        let space = new_space()?;
        let t = start_with(&space, Program::CallStoreTrap, [999, DATA, 0], DEFAULT_HOOK)?;
        if thread::join(t) != Ok(killed(UD)) {
            return Err("the program did not end with its ud2");
        }
        let enosys = poly_personality_linux::err(poly_personality_linux::ENOSYS);
        if read_u64(&space, DATA)? != enosys {
            return Err("the Linux personality's -ENOSYS did not reach user mode");
        }
        Ok(())
    }
}

/// Where the spinning bystander in `user_faults_kill_only_the_thread`
/// waits for its exit code.
const FLAG: u64 = DATA + 0x800;

kernel_test! {
    fn user_faults_kill_only_the_thread() -> TestResult {
        let space = new_space()?;
        // A thread of the same process that keeps running in ring 3 while
        // the others die.
        write_u64(&space, FLAG, 0)?;
        let bystander = start(&space, Program::Spin, [FLAG, 0, 0])?;

        static KERNEL_DATA: u64 = 0x1234;
        let kernel_data = (&raw const KERNEL_DATA) as u64;
        let kernel_text = hook as *const () as u64;
        let data_phys = space.translate(DATA).ok_or("data page not mapped")?.phys;
        let direct_alias = crate::arch::phys_to_virt(data_phys).ok_or("data page outside the direct map")?;
        if !non_canonical(run(&space, Program::Load, [LOWER_HALF_END, 0, 0])?) {
            return Err("reading a non-canonical address did not kill the thread");
        }
        let cases = [
            (Program::Load, DATA, 0, "reading its own data"),
            (Program::Load, kernel_data, killed(PF), "reading kernel data"),
            (Program::Load, direct_alias, killed(PF), "reading its own page through the kernel's direct map"),
            (Program::Load, UNMAPPED, killed(PF), "reading unmapped memory"),
            (Program::Load, 0, killed(PF), "reading address 0"),
            (Program::Store, CODE, killed(PF), "writing its own code"),
            (Program::Store, kernel_data, killed(PF), "writing kernel data"),
            (Program::Jump, kernel_text, killed(PF), "executing kernel code"),
            (Program::Jump, DATA, killed(PF), "executing its data page"),
            (Program::Int80, 0, killed(GP), "int 0x80"),
            (Program::IntTimer, 0, killed(GP), "raising the timer vector with int"),
            (Program::Hlt, 0, killed(GP), "hlt"),
            (Program::Cli, 0, killed(GP), "cli"),
            (Program::ReadCr3, 0, killed(GP), "reading CR3"),
            (Program::Ud2, 0, killed(UD), "ud2"),
        ];
        for (program, arg, expected, what) in cases {
            let code = run(&space, program, [arg, 0, 0])?;
            if code != expected {
                kprintln!("user: {what}: exit code {code:#x}, expected {expected:#x}");
                return Err("a user program did not end as expected");
            }
        }
        if thread::state(bystander) != Some(State::Runnable) {
            return Err("a fault in one thread ended another");
        }
        write_u64(&space, FLAG, 0x5eed)?;
        if thread::join(bystander) != Ok(0x5eed) {
            return Err("the bystander did not finish");
        }
        released(&space)
    }
}

kernel_test! {
    fn returns_outside_the_lower_half_go_through_iretq() -> TestResult {
        let space = new_space()?;
        let target = CODE + Program::ReturnTarget.offset();
        let kernel_text = hook as *const () as u64;
        let cases = [
            (target, 0x99, "resuming at another user address"),
            (0xffff_8000_0000_0000, killed(PF), "resuming at the start of the higher half"),
            (kernel_text, killed(PF), "resuming in kernel code"),
        ];
        for (rip, expected, what) in cases {
            let code = run(&space, Program::ReturnTo, [rip, 0, 0])?;
            if code != expected {
                kprintln!("user: {what}: exit code {code:#x}, expected {expected:#x}");
                return Err("a redirected return did not end as expected");
            }
        }
        // IRETQ to a non-canonical address raises #GP in ring 0, which kills
        // the thread (TCG lets it through to a user page fault instead).
        if !non_canonical(run(&space, Program::ReturnTo, [LOWER_HALF_END, 0, 0])?) {
            return Err("resuming at a non-canonical address did not kill the thread");
        }
        // So that TCG runs the ring-0 path too: an IRETQ that faults on its
        // way to ring 3 kills the thread, and the kernel's GS state survives
        // (the system calls after it, here and in later tests, depend on it).
        if run(&space, Program::FailReturn, [0; 3])? != killed(GP) {
            return Err("a return to ring 3 that faulted in IRETQ did not kill the thread");
        }
        if run(&space, Program::ReturnTo, [target, 0, 0])? != 0x99 {
            return Err("system calls broke after a failed return");
        }
        released(&space)
    }
}

/// One check of `copy_checks`, given a scratch buffer.
type Check<'a> = dyn Fn(&mut [u8; 32]) -> bool + 'a;

/// The callback of `copies_recover_from_faults`: 1 if every check passed,
/// else 100 + the number of the first that failed.
fn copy_checks() -> usize {
    let kernel = (&raw const RECORDED) as u64;
    let page_end = DATA + PAGE_SIZE;
    let mut buf = [0u8; 32];
    let checks: [&Check<'_>; 20] = [
        &|b| copy_from_user(&mut b[..16], DATA).is_ok() && &b[..6] == b"hello\0",
        &|b| copy_from_user(&mut b[..16], UNMAPPED) == Err(Efault),
        &|b| copy_from_user(&mut b[..16], kernel) == Err(Efault),
        &|b| copy_from_user(&mut b[..8], page_end - 4) == Err(Efault),
        &|b| copy_from_user(&mut b[..0], DATA).is_ok() && copy_from_user(&mut b[..0], kernel) == Err(Efault),
        &|b| {
            copy_to_user(DATA + 64, b"written").is_ok()
                && copy_from_user(&mut b[..7], DATA + 64).is_ok()
                && &b[..7] == b"written"
        },
        &|_| copy_to_user(CODE, b"x") == Err(Efault),
        &|_| copy_to_user(UNMAPPED, b"x") == Err(Efault),
        &|_| copy_to_user(page_end - 2, b"four") == Err(Efault),
        &|_| copy_to_user(kernel, b"x") == Err(Efault),
        &|b| strncpy_from_user(b, DATA) == Ok(5) && &b[..6] == b"hello\0",
        &|b| strncpy_from_user(&mut b[..3], DATA) == Ok(3) && &b[..3] == b"hel",
        &|b| strncpy_from_user(b, page_end - 4) == Err(Efault),
        &|b| strncpy_from_user(b, kernel) == Err(Efault),
        &|b| strncpy_from_user(b, UNMAPPED) == Err(Efault),
        &|b| strncpy_from_user(b, USER_END - 1) == Err(Efault),
        &|_| is_user_range(USER_END - 8, 8) && !is_user_range(USER_END - 8, 9) && !is_user_range(u64::MAX, 2),
        &|_| current_process().is_some_and(|p| p.pid == PID),
        &|_| current_address_space().is_some_and(|s| s.is_active() && s.translate(DATA).is_some()),
        &|_| {
            let ok = set_fs_base(0x1234_5000).is_ok()
                && fs_base() == 0x1234_5000
                && set_gs_base(0x5678_9000).is_ok()
                && gs_base() == 0x5678_9000
                && set_fs_base(LOWER_HALF_END) == Err(Efault)
                && set_gs_base(LOWER_HALF_END) == Err(Efault);
            ok && set_fs_base(0).is_ok() && set_gs_base(0).is_ok()
        },
    ];
    for (i, check) in checks.iter().enumerate() {
        buf.fill(0xee);
        if !check(&mut buf) {
            return 100 + i;
        }
    }
    1
}

kernel_test! {
    fn copies_recover_from_faults() -> TestResult {
        let space = new_space()?;
        space.write(DATA, b"hello\0").map_err(|_| "data page not mapped")?;
        // A string that runs into the unmapped page after the data page.
        space.write(DATA + PAGE_SIZE - 4, b"abcd").map_err(|_| "data page not mapped")?;
        if !matches!(space.translate(DATA), Some(Mapping { prot: Prot::RW, owned: true, .. }))
            || space.map_zeroed(USER_START - PAGE_SIZE, PAGE_SIZE, Prot::RW) != Err(MapError::OutsideUserSpace)
        {
            return Err("the test address space is not laid out as expected");
        }
        *CALLBACK.lock() = Some(copy_checks);
        let code = run(&space, Program::Callback, [0; 3])?;
        if code != 1 {
            kprintln!("user: copy check {} failed", code.wrapping_sub(100));
            return Err("a user-copy check failed");
        }
        Ok(())
    }
}

kernel_test! {
    fn user_copies_from_a_kernel_thread_fail_cleanly() -> TestResult {
        // The kernel PML4 maps nothing in the lower half, so any user
        // address faults, and the fixup turns that into Efault.
        let mut buf = [0u8; 8];
        if current_process().is_some() || current_address_space().is_some() {
            return Err("the boot thread looks like a user thread");
        }
        if copy_from_user(&mut buf, CODE) != Err(Efault)
            || copy_to_user(CODE, &buf) != Err(Efault)
            || strncpy_from_user(&mut buf, CODE) != Err(Efault)
        {
            return Err("a user copy without a user address space did not fail with Efault");
        }
        if set_return_address(CODE).is_ok() {
            return Err("a kernel thread set a user return address");
        }
        Ok(())
    }
}

kernel_test! {
    fn address_spaces_are_isolated() -> TestResult {
        // The same address in two spaces holds different data, while the
        // threads alternate (each yields ten times between its reads).
        let (a, b) = (new_space()?, new_space()?);
        write_u64(&a, DATA, 0xaaaa)?;
        write_u64(&b, DATA, 0xbbbb)?;
        let ta = start(&a, Program::Isolation, [DATA, 0, 0])?;
        let tb = start(&b, Program::Isolation, [DATA, 0, 0])?;
        let (ca, cb) = (thread::join(ta), thread::join(tb));
        if ca != Ok(0xaaaa) || cb != Ok(0xbbbb) {
            return Err("a thread saw another address space's memory");
        }
        released(&a)?;
        released(&b)
    }
}

kernel_test! {
    fn sse_and_segment_bases_survive_context_switches() -> TestResult {
        // Two threads of one process, each with its own XMM values, MXCSR
        // rounding mode and FS/GS bases, yielding to each other twenty
        // times; and a third in another address space for good measure.
        let space = new_space()?;
        let other = new_space()?;
        let bases = [(DATA + 0x100, 0x1111_0000u64), (DATA + 0x200, 0x2222_0000)];
        for (base, value) in bases {
            write_u64(&space, base, value)?;
            write_u64(&space, base + 8, value + 1)?;
        }
        write_u64(&other, DATA, 0x3333_0000)?;
        write_u64(&other, DATA + 8, 0x3333_0001)?;
        let threads = [
            start(&space, Program::Sse, [0x1000, bases[0].0, bases[0].1])?,
            start(&space, Program::Sse, [0x2001, bases[1].0, bases[1].1])?,
            start(&other, Program::Sse, [0x3002, DATA, 0x3333_0000])?,
        ];
        for t in threads {
            match thread::join(t) {
                Ok(1) => {}
                Ok(BAD) => return Err("XMM registers, MXCSR or a segment base changed across context switches"),
                _ => return Err("an SSE test thread failed"),
            }
        }
        Ok(())
    }
}

kernel_test! {
    fn ring3_is_preempted_by_the_timer() -> TestResult {
        // The program never makes a system call until it sees the flag: the
        // timer has to take the CPU from ring 3 for this thread to run on.
        // Both share one CPU, which the spinner must be preempted on.
        let pin = thread::pin_current();
        let space = new_space()?;
        let t = start(&space, Program::Spin, [DATA, 0, 0])?;
        thread::set_affinity(t, thread::CpuSet::one(pin.cpu())).map_err(|_| "could not pin the spinner")?;
        let ticks = time::ticks();
        thread::sleep(Duration::from_millis(30));
        if time::ticks() < ticks + 20 {
            return Err("the tick stopped while a thread ran in ring 3");
        }
        if thread::state(t) != Some(State::Runnable) {
            return Err("the spinning thread is not runnable");
        }
        write_u64(&space, DATA, 0xd0e)?;
        if thread::join(t) != Ok(0xd0e) {
            return Err("the spinning thread did not see the flag");
        }
        released(&space)
    }
}
