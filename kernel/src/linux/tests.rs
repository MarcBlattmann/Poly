//! In-kernel tests of the kernel's services for the Linux personality:
//! real Linux processes (with [`LinuxState`](super::LinuxState)) whose
//! threads run small programs in ring 3 and make real Linux system calls.
//!
//! Most tests run their checks from a *callback*: the program makes one
//! system call with the number [`CALLBACK`], which the test hook answers by
//! running a Rust function on that thread, in that system call, where it
//! makes Linux system calls through `Linux::dispatch_with` and the kernel's
//! [`Thread`] like the dispatcher does, with real user memory. The program
//! then exits with what the function returned (0: every check passed, else
//! the number of the first failed check).

use super::Thread;
use crate::fs::{self, Files, OpenFlags};
use crate::kernel_test;
use crate::process::{self, ExitStatus, Process};
use crate::program::{self, ExecPathError};
use crate::sync::SpinLock;
use crate::time;
use crate::user::memory::{Layout, Placement, Prot};
use crate::user::{self, Entry, Memory};
use alloc::sync::Arc;
use core::arch::global_asm;
use poly_abi::SyscallRegs;
use poly_binfmt::{Arch, Bits, Format};
use poly_personality_linux::abi::signal::{SA_RESTORER, SIGSEGV};
use poly_personality_linux::{Errno, Linux, nr};

type TestResult<T = ()> = Result<T, &'static str>;

/// The test hook's own system call: run the test's callback.
const CALLBACK: u64 = 0x7e57;
/// Where the programs are mapped (read, execute).
const CODE: u64 = 0x40_0000;
/// One read-write page of data, zeroed.
const DATA: u64 = 0x60_0000;
/// Where the program break starts.
const BRK: u64 = 0x80_0000;
/// How long a test process may take, generously: a loaded host makes the
/// emulator slow.
const DEADLINE_NS: u64 = 60 * time::NANOS_PER_SEC;

global_asm!(
    r#"
    .section .rodata.linux_test_programs, "a", @progbits
    .balign 16
    .global linux_test_start
linux_test_start:

/* The test's callback, then exit_group with its result. */
    .global linux_test_callback
linux_test_callback:
    mov eax, {callback}
    syscall
    mov rdi, rax
    mov eax, 231
    syscall
    ud2

/* Spin in user mode forever. */
    .global linux_test_spin
linux_test_spin:
    jmp linux_test_spin

/* futex(rdi, FUTEX_WAIT, 0, NULL) for as long as the word is 0. */
    .global linux_test_futex_wait
linux_test_futex_wait:
    mov rbx, rdi
1:
    mov rdi, rbx
    mov eax, 202
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    syscall
    jmp 1b

/* exit_group(rdi). */
    .global linux_test_exit_group
linux_test_exit_group:
    mov eax, 231
    syscall
    ud2

/* Store to the address in rdi. */
    .global linux_test_store
linux_test_store:
    mov qword ptr [rdi], 1
    ud2

/* arch_prctl(ARCH_SET_FS, rdi), then exit_group with the word at fs:0. */
    .global linux_test_fs
linux_test_fs:
    mov rsi, rdi
    mov edi, 0x1002
    mov eax, 158
    syscall
    test rax, rax
    jnz 2f
    mov rdi, qword ptr fs:[0]
    mov eax, 231
    syscall
2:
    mov edi, 99
    mov eax, 231
    syscall
    ud2

/* Install the kernel sigaction at rdi for SIGUSR1 (rt_sigaction), send
   SIGUSR1 to itself (getpid, kill), and exit_group with the word the
   handler stored at rdi + 64 (or 98 / 99 if a call failed). The handler
   stores 5 there with the callee-saved r12 it finds (the interrupted
   value), then returns through the restorer, which calls rt_sigreturn. */
    .global linux_test_signal
linux_test_signal:
    mov r12, rdi
    mov eax, 13
    mov edi, 10
    mov rsi, r12
    xor edx, edx
    mov r10d, 8
    syscall
    test rax, rax
    jnz 3f
    mov eax, 39
    syscall
    mov edi, eax
    mov esi, 10
    mov eax, 62
    syscall
    test rax, rax
    jnz 4f
    mov rdi, qword ptr [r12 + 64]
    mov eax, 231
    syscall
3:
    mov edi, 98
    mov eax, 231
    syscall
4:
    mov edi, 99
    mov eax, 231
    syscall
    ud2
    .global linux_test_handler
linux_test_handler:
    mov qword ptr [r12 + 64], 5
    ret
    .global linux_test_restorer
linux_test_restorer:
    mov eax, 15
    syscall
    ud2

    .balign 16
    .global linux_test_end
linux_test_end:
"#,
    callback = const CALLBACK,
);

unsafe extern "C" {
    static linux_test_start: u8;
    static linux_test_end: u8;
    static linux_test_callback: u8;
    static linux_test_spin: u8;
    static linux_test_futex_wait: u8;
    static linux_test_exit_group: u8;
    static linux_test_store: u8;
    static linux_test_fs: u8;
    static linux_test_signal: u8;
    static linux_test_handler: u8;
    static linux_test_restorer: u8;
}

/// A program's (or label's) address once the blob is mapped at [`CODE`].
fn at(label: *const u8) -> u64 {
    CODE + (label as u64 - (&raw const linux_test_start) as u64)
}

fn blob() -> &'static [u8] {
    let start = &raw const linux_test_start;
    let len = (&raw const linux_test_end) as usize - start as usize;
    // SAFETY: the assembly above puts every program between the two
    // symbols, in one read-only section of the kernel image.
    unsafe { core::slice::from_raw_parts(start, len) }
}

/// What the next [`CALLBACK`] call runs.
/// A callback: what the program exits with.
type Callback = fn(&Process) -> u64;

static RUN: SpinLock<Option<Callback>> = SpinLock::new(None);

/// The test threads' system calls: [`CALLBACK`], else the default (the
/// Linux personality with the kernel's services).
fn hook(process: &Process, regs: &mut SyscallRegs) {
    if regs.rax == CALLBACK {
        let f = RUN.lock().expect("a callback is set");
        regs.rax = f(process);
    } else {
        user::DEFAULT_HOOK(process, regs);
    }
}

/// A Linux process and a memory with the programs, a data page, a break
/// and a stack.
fn setup() -> TestResult<(Arc<Process>, Arc<Memory>)> {
    let format = Format::Elf { bits: Bits::B64, arch: Arch::X86_64 };
    let process = Arc::new(Process::new_linux(process::alloc_pid(), 0, format, false));
    let memory = Memory::new(Layout::default()).map_err(|_| "no memory for a memory map")?;
    let code = blob();
    let len = poly_vm::page_up(code.len() as u64).expect("small");
    let ok = memory.map_anonymous(Placement::FixedNoReplace(CODE), len, Prot::RW).is_ok()
        && memory.write(CODE, code).is_ok()
        && memory.protect(CODE, len, Prot::RX).is_ok()
        && memory.map_anonymous(Placement::FixedNoReplace(DATA), 4096, Prot::RW).is_ok()
        && memory.init_brk(BRK).is_ok()
        && memory.setup_stack(64 << 10, Prot::RW).is_ok();
    if !ok {
        return Err("could not lay out the test memory");
    }
    Ok((process, Arc::new(memory)))
}

/// Start a thread of `process` in `memory` at `program` with `args`.
fn start(process: &Arc<Process>, memory: &Arc<Memory>, program: *const u8, args: [u64; 3]) -> TestResult {
    let rsp = memory.layout().stack_top - 64;
    let entry = Entry { rip: at(program), rsp, args };
    let tid = user::spawn_in(Arc::clone(process), Arc::clone(memory), entry, hook).map_err(|_| "spawn failed")?;
    let _ = crate::thread::detach(tid);
    Ok(())
}

fn wait(process: &Process) -> TestResult<ExitStatus> {
    process.wait_until(time::now_ns() + DEADLINE_NS).ok_or("the test process did not end in time")
}

/// Run `f` as the callback of a one-thread process; its result is the
/// process's exit status.
fn run_callback(f: Callback) -> TestResult<ExitStatus> {
    let (process, memory) = setup()?;
    *RUN.lock() = Some(f);
    start(&process, &memory, &raw const linux_test_callback, [0; 3])?;
    wait(&process)
}

/// A Linux system call from a callback, as the dispatcher makes it.
fn sys(process: &Process, number: u64, args: &[u64]) -> Result<u64, Errno> {
    let mut k = Thread::current(process).expect("a thread of a Linux process");
    let mut a = [0u64; 6];
    a[..args.len()].copy_from_slice(args);
    let mut regs = SyscallRegs {
        rax: number,
        rdi: a[0],
        rsi: a[1],
        rdx: a[2],
        r10: a[3],
        r8: a[4],
        r9: a[5],
        ..Default::default()
    };
    Linux::dispatch_with(&mut k, &mut regs);
    match Errno::from_rax(regs.rax) {
        Some(e) => Err(e),
        None => Ok(regs.rax),
    }
}

/// Checks in a callback: the first failure returns its number.
macro_rules! check {
    ($n:expr, $cond:expr) => {
        if !$cond {
            crate::kprintln!("linux test: check {} failed: {}", $n, stringify!($cond));
            return $n;
        }
    };
}

fn expect_exit(status: ExitStatus, code: u8, what: &'static str) -> TestResult {
    if status == ExitStatus::Exited(code) {
        Ok(())
    } else {
        crate::kprintln!("linux test: {what}: {status}");
        Err(what)
    }
}

const PROT_READ: u64 = 0x1;
const PROT_RW: u64 = 0x3;
const MAP_PRIVATE_ANON: u64 = 0x22;
const MAP_FIXED: u64 = 0x10;

kernel_test! {
    fn write_goes_to_the_process_files() -> TestResult {
        const OUT: &[u8] = b"/tmp/linux-test-write";
        let (process, memory) = setup()?;
        // fd 1 is a tmpfs file instead of the console.
        let flags = OpenFlags::CREAT | OpenFlags::RDWR | OpenFlags::TRUNC;
        let fd = process.files.open(OUT, flags, 0o600).map_err(|e| e.name())?;
        process.files.dup2(fd, 1).map_err(|e| e.name())?;
        process.files.close(fd).map_err(|e| e.name())?;
        *RUN.lock() = Some(|p| {
            let page = match sys(p, nr::MMAP, &[0, 4096, PROT_RW, MAP_PRIVATE_ANON, u64::MAX, 0]) {
                Ok(a) => a,
                Err(_) => return 1,
            };
            check!(2, user::copy_to_user(page, b"hello, file").is_ok());
            check!(3, sys(p, nr::WRITE, &[1, page, 11]) == Ok(11));
            check!(4, sys(p, nr::WRITE, &[1, page, 0]) == Ok(0));
            check!(5, sys(p, nr::WRITE, &[9, page, 1]) == Err(Errno::EBADF));
            0
        });
        start(&process, &memory, &raw const linux_test_callback, [0; 3])?;
        let status = wait(&process)?;
        let files = Files::new();
        let written = files.read_to_end(OUT, 100).map(|(d, _)| d);
        let _ = files.unlinkat(fs::AT_FDCWD, OUT, 0);
        expect_exit(status, 0, "write checks failed")?;
        if written.as_deref() != Ok(b"hello, file".as_slice()) {
            return Err("write did not reach the file on fd 1");
        }
        Ok(())
    }
}

kernel_test! {
    fn brk_grows_and_shrinks() -> TestResult {
        let status = run_callback(|p| {
            let start = sys(p, nr::BRK, &[0]).unwrap_or(0);
            check!(1, start == BRK);
            check!(2, sys(p, nr::BRK, &[start + 0x10000]) == Ok(start + 0x10000));
            check!(3, user::copy_to_user(start + 0xfff8, &[7; 8]).is_ok());
            let mut b = [0u8; 8];
            check!(4, user::copy_from_user(&mut b, start + 0x8000).is_ok() && b == [0; 8]);
            check!(5, sys(p, nr::BRK, &[start]) == Ok(start));
            check!(6, user::copy_to_user(start + 8, &[1]).is_err());
            // Below the start, or absurdly high: the break stays.
            check!(7, sys(p, nr::BRK, &[4096]) == Ok(start));
            check!(8, sys(p, nr::BRK, &[0x7fff_ffff_f000]) == Ok(start));
            0
        })?;
        expect_exit(status, 0, "brk checks failed")
    }
}

kernel_test! {
    fn mmap_and_munmap_anonymous_and_file_memory() -> TestResult {
        const FILE: &[u8] = b"/tmp/linux-test-mmap";
        let files = Files::new();
        let fd = files.open(FILE, OpenFlags::CREAT | OpenFlags::RDWR | OpenFlags::TRUNC, 0o644).map_err(|e| e.name())?;
        let written = files.write(fd, b"mapped from tmpfs");
        let _ = files.close(fd);
        written.map_err(|e| e.name())?;
        let status = run_callback(|p| {
            let a = sys(p, nr::MMAP, &[0, 8192, PROT_RW, MAP_PRIVATE_ANON, u64::MAX, 0]).unwrap_or(1);
            check!(1, a.is_multiple_of(4096));
            let mut b = [0xffu8; 8];
            check!(2, user::copy_from_user(&mut b, a + 4096).is_ok() && b == [0; 8]);
            check!(3, user::copy_to_user(a, &[1, 2, 3]).is_ok());
            check!(4, sys(p, nr::MPROTECT, &[a, 4096, PROT_READ]) == Ok(0));
            check!(5, user::copy_to_user(a, &[9]).is_err());
            check!(6, user::copy_from_user(&mut b[..3], a).is_ok() && b[..3] == [1, 2, 3]);
            check!(7, sys(p, nr::MUNMAP, &[a, 8192]) == Ok(0));
            check!(8, user::copy_from_user(&mut b, a).is_err());
            let fixed = MAP_PRIVATE_ANON | MAP_FIXED;
            check!(9, sys(p, nr::MMAP, &[a + 4096, 4096, PROT_RW, fixed, u64::MAX, 0]) == Ok(a + 4096));
            // A private mapping of a tmpfs file, read through the VFS.
            let Ok(fd) = p.files.open(b"/tmp/linux-test-mmap", OpenFlags::RDONLY, 0) else { return 10 };
            let m = sys(p, nr::MMAP, &[0, 4096, PROT_READ, 0x2, fd as u64, 0]).unwrap_or(1);
            let mut text = [0u8; 18];
            check!(11, user::copy_from_user(&mut text, m).is_ok());
            check!(12, &text == b"mapped from tmpfs\0");
            check!(13, sys(p, nr::MMAP, &[0, 4096, PROT_READ, 0x2, 1, 0]) == Err(Errno::ENODEV));
            0
        });
        let _ = files.unlinkat(fs::AT_FDCWD, FILE, 0);
        expect_exit(status?, 0, "mmap checks failed")
    }
}

kernel_test! {
    fn arch_prctl_sets_the_fs_base() -> TestResult {
        let status = run_callback(|p| {
            check!(1, sys(p, nr::ARCH_PRCTL, &[0x1002, 0x1234_5000]) == Ok(0));
            check!(2, user::fs_base() == 0x1234_5000);
            let Ok(page) = sys(p, nr::MMAP, &[0, 4096, PROT_RW, MAP_PRIVATE_ANON, u64::MAX, 0]) else { return 3 };
            check!(4, sys(p, nr::ARCH_PRCTL, &[0x1003, page]) == Ok(0));
            let mut b = [0u8; 8];
            check!(5, user::copy_from_user(&mut b, page).is_ok() && u64::from_le_bytes(b) == 0x1234_5000);
            check!(6, sys(p, nr::ARCH_PRCTL, &[0x1002, 1 << 47]) == Err(Errno::EPERM));
            0
        })?;
        expect_exit(status, 0, "arch_prctl checks failed")?;
        // In ring 3: the program reads fs:0 after setting FS to DATA.
        let (process, memory) = setup()?;
        memory.write(DATA, &42u64.to_le_bytes()).map_err(|_| "writing the data page failed")?;
        start(&process, &memory, &raw const linux_test_fs, [DATA, 0, 0])?;
        expect_exit(wait(&process)?, 42, "the FS base did not reach user mode")
    }
}

kernel_test! {
    fn exit_group_ends_every_thread() -> TestResult {
        let (process, memory) = setup()?;
        // One thread spins in ring 3, one waits on a futex, one ends them.
        start(&process, &memory, &raw const linux_test_spin, [0; 3])?;
        start(&process, &memory, &raw const linux_test_futex_wait, [DATA + 0x100, 0, 0])?;
        start(&process, &memory, &raw const linux_test_exit_group, [7, 0, 0])?;
        let status = wait(&process)?;
        if process.thread_count() != 0 {
            return Err("threads are left after the process ended");
        }
        expect_exit(status, 7, "the process did not end with exit_group's status")
    }
}

kernel_test! {
    fn bad_pointers_give_efault_and_faults_give_sigsegv() -> TestResult {
        let status = run_callback(|p| {
            check!(1, sys(p, nr::WRITE, &[1, 0x10, 5]) == Err(Errno::EFAULT));
            check!(2, sys(p, nr::WRITE, &[1, 0xffff_8000_0000_0000, 5]) == Err(Errno::EFAULT));
            check!(3, sys(p, nr::UNAME, &[0x10]) == Err(Errno::EFAULT));
            check!(4, sys(p, nr::CLOCK_GETTIME, &[1, 8]) == Err(Errno::EFAULT));
            check!(5, sys(p, nr::RT_SIGPROCMASK, &[0, 0x10, 0, 8]) == Err(Errno::EFAULT));
            0
        })?;
        expect_exit(status, 0, "EFAULT checks failed")?;
        let (process, memory) = setup()?;
        start(&process, &memory, &raw const linux_test_store, [16, 0, 0])?;
        let status = wait(&process)?;
        if status != (ExitStatus::Signaled { signal: SIGSEGV, core: false }) {
            crate::kprintln!("linux test: store to 16: {status}");
            return Err("a store to an unmapped page did not end the process with SIGSEGV");
        }
        Ok(())
    }
}

kernel_test! {
    fn signal_handler_runs_and_returns() -> TestResult {
        let (process, memory) = setup()?;
        // struct sigaction as the kernel takes it: handler, flags,
        // restorer, mask.
        let mut act = [0u8; 32];
        act[0..8].copy_from_slice(&at(&raw const linux_test_handler).to_le_bytes());
        act[8..16].copy_from_slice(&SA_RESTORER.to_le_bytes());
        act[16..24].copy_from_slice(&at(&raw const linux_test_restorer).to_le_bytes());
        memory.write(DATA, &act).map_err(|_| "writing the data page failed")?;
        start(&process, &memory, &raw const linux_test_signal, [DATA, 0, 0])?;
        expect_exit(wait(&process)?, 5, "the handler did not run, or rt_sigreturn did not bring the thread back")
    }
}

kernel_test! {
    fn exec_path_runs_programs_from_the_initramfs(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(fs::TEST_INITRD_FLAG) {
            return Ok(());
        }
        let argv: [&[u8]; 1] = [b"/bin/hello"];
        let process = program::exec_path(b"/bin/hello", &argv, &[], false).map_err(|_| "exec_path /bin/hello failed")?;
        expect_exit(wait(&process)?, 0, "/bin/hello did not exit with 0")?;
        let refused = |path: &[u8]| program::exec_path(path, &[path], &[], false).err();
        if refused(b"/nope") != Some(ExecPathError::Open(poly_vfs::Errno::ENOENT)) {
            return Err("a missing file was not ENOENT");
        }
        if refused(b"/test/hello.txt") != Some(ExecPathError::Open(poly_vfs::Errno::EACCES)) {
            return Err("a file without execute permission was not EACCES");
        }
        if !matches!(refused(b"/test/script.sh"), Some(ExecPathError::Unsupported(_))) {
            return Err("a #! script was not refused as unsupported");
        }
        Ok(())
    }
}
