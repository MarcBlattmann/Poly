//! A static Linux program without any C library that exercises processes
//! and threads (roadmap Phase 4, ADR 0041), for the kernel's boot tests:
//! `cargo xtask` builds it like the nolibc hello world
//! (`tools/xtask/src/programs.rs`) and boots it as the first process
//! (`poly.init=/bin/forkexec`), so it is pid 1, the init process.
//!
//! 1. `fork`, and the child `execve`s `/bin/hello` (which prints its
//!    greeting); `wait4` reports exit status 0.
//! 2. `fork`; the child changes a variable and exits with it; the parent
//!    still sees its own value (copy on write) and gets the status.
//! 3. `clone` a thread the way a thread library does: its own stack, TLS
//!    through `%fs`, `CLONE_PARENT_SETTID` / `CLONE_CHILD_CLEARTID`; the
//!    main thread waits on the tid word with `futex` until the exiting
//!    thread clears it.
//! 4. Several CPUs (when `sched_getaffinity` offers two or more): the
//!    main thread pins itself to one CPU and a thread to another
//!    (`sched_setaffinity`, checked with `getcpu`); each wakes the other
//!    with `futex` across the CPUs. The thread then stores to a counter
//!    without pause while the main thread forks: the child, on the
//!    thread's CPU, must see its copy of the counter stand still (a CPU
//!    that kept a writable translation from before the fork would write
//!    into the page the child shares), and its own store must stay out of
//!    the parent's memory; the parent's counter goes on rising.
//! 5. An orphan: a child forks a grandchild and ends; the grandchild is
//!    adopted by init (this program: `getppid` turns 1) and init reaps it
//!    with `wait4(-1)`. Only when the program is pid 1.
//!
//! Then it prints `forkexec: ok` and exits 0; any failure prints what
//! failed on stderr and exits 1. Every call is a raw `syscall` (numbers
//! from `asm/unistd_64.h`, flags from `linux/sched.h`, `linux/futex.h`).
//! Written for Poly from the man pages and the x86-64 psABI.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const SYS_WRITE: u64 = 1;
const SYS_SCHED_YIELD: u64 = 24;
const SYS_SCHED_SETAFFINITY: u64 = 203;
const SYS_SCHED_GETAFFINITY: u64 = 204;
const SYS_GETCPU: u64 = 309;
const SYS_FORK: u64 = 57;
const SYS_EXECVE: u64 = 59;
const SYS_WAIT4: u64 = 61;
const SYS_GETPID: u64 = 39;
const SYS_GETPPID: u64 = 110;
const SYS_GETTID: u64 = 186;
const SYS_FUTEX: u64 = 202;
const SYS_EXIT_GROUP: u64 = 231;

const CLONE_VM: u64 = 0x100;
const CLONE_FS: u64 = 0x200;
const CLONE_FILES: u64 = 0x400;
const CLONE_SIGHAND: u64 = 0x800;
const CLONE_THREAD: u64 = 0x1_0000;
const CLONE_SYSVSEM: u64 = 0x4_0000;
const CLONE_SETTLS: u64 = 0x8_0000;
const CLONE_PARENT_SETTID: u64 = 0x10_0000;
const CLONE_CHILD_CLEARTID: u64 = 0x20_0000;
const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;
const ECHILD: u64 = 10;

/// A system call with up to six arguments; returns `rax` (`-errno` on
/// failure).
///
/// # Safety
/// The call must not break the program's memory.
unsafe fn syscall(nr: u64, a: [u64; 6]) -> u64 {
    let ret;
    // SAFETY: the x86-64 Linux convention: rcx and r11 are clobbered,
    // memory may be read and written by the kernel.
    unsafe {
        asm!("syscall", inlateout("rax") nr => ret, in("rdi") a[0], in("rsi") a[1], in("rdx") a[2],
             in("r10") a[3], in("r8") a[4], in("r9") a[5], lateout("rcx") _, lateout("r11") _,
             options(nostack));
    }
    ret
}

fn sys(nr: u64, a: [u64; 6]) -> u64 {
    // SAFETY: only calls that leave this program's memory alone go
    // through here (fork copies it, wait4 writes the status word).
    unsafe { syscall(nr, a) }
}

fn write(fd: u64, s: &[u8]) {
    sys(SYS_WRITE, [fd, s.as_ptr() as u64, s.len() as u64, 0, 0, 0]);
}

fn exit(code: u64) -> ! {
    loop {
        sys(SYS_EXIT_GROUP, [code, 0, 0, 0, 0, 0]);
    }
}

fn fail(what: &[u8]) -> ! {
    write(2, b"forkexec: ");
    write(2, what);
    write(2, b" failed\n");
    exit(1)
}

fn is_err(ret: u64) -> bool {
    ret > u64::MAX - 4095
}

fn fork() -> u64 {
    let pid = sys(SYS_FORK, [0; 6]);
    if is_err(pid) {
        fail(b"fork");
    }
    pid
}

/// wait4(pid) for the status word; fails unless it reports `pid` (or, for
/// -1, any child).
fn wait(pid: u64) -> (u64, u32) {
    let mut status = 0u32;
    let got = sys(SYS_WAIT4, [pid, &raw mut status as u64, 0, 0, 0, 0]);
    if is_err(got) || (pid != u64::MAX && got != pid) {
        fail(b"wait4");
    }
    (got, status)
}

/// A variable the child of step 2 changes in its copy.
static mut SHARED_UNTIL_WRITTEN: u64 = 1;

/// What the thread of step 3 found, and its tid word.
static THREAD_TLS: AtomicU64 = AtomicU64::new(0);
static THREAD_TID: AtomicU64 = AtomicU64::new(0);
static TID_WORD: AtomicU32 = AtomicU32::new(0);
/// The thread's TLS block: `%fs:0` points at its first word.
static TLS_BLOCK: [AtomicU64; 2] = [AtomicU64::new(0x7e57_7155), AtomicU64::new(0)];
/// The thread's stack, 16-byte aligned as the psABI wants at a call.
#[repr(C, align(16))]
struct Stack([u64; 1024]);
static mut THREAD_STACK: Stack = Stack([0; 1024]);

unsafe extern "C" {
    /// `clone(flags, stack, parent_tid, child_tid, tls)`; the child calls
    /// `f()` on its new stack and exits with its result. Returns the
    /// child's tid (or `-errno`) in the parent.
    fn clone_thread(flags: u64, stack: u64, parent_tid: u64, child_tid: u64, tls: u64, f: extern "C" fn() -> u64) -> u64;
}

// The child of `clone` resumes with the parent's registers on a stack of
// its own, so it must not return into Rust code: it calls `f` from here.
global_asm!(
    ".globl clone_thread",
    "clone_thread:",
    "push r12",
    "mov r12, r9",
    "mov r10, rcx",
    "mov eax, 56",
    "syscall",
    "test rax, rax",
    "jz 2f",
    "pop r12",
    "ret",
    "2:",
    "xor ebp, ebp",
    "call r12",
    "mov rdi, rax",
    "mov eax, 60",
    "syscall",
    "ud2",
);

/// The CPUs this thread may run on (`sched_getaffinity`; 64 at most).
fn cpus() -> u64 {
    let mut mask = 0u64;
    if is_err(sys(SYS_SCHED_GETAFFINITY, [0, 8, &raw mut mask as u64, 0, 0, 0])) {
        fail(b"sched_getaffinity");
    }
    mask
}

/// The CPU this thread runs on (`getcpu`).
fn getcpu() -> u64 {
    let mut cpu = u32::MAX;
    if is_err(sys(SYS_GETCPU, [&raw mut cpu as u64, 0, 0, 0, 0, 0])) {
        fail(b"getcpu");
    }
    u64::from(cpu)
}

/// Run only on CPU `cpu` from now on.
fn pin(cpu: u64) {
    let mask = 1u64 << cpu;
    if is_err(sys(SYS_SCHED_SETAFFINITY, [0, 8, &raw const mask as u64, 0, 0, 0])) {
        fail(b"sched_setaffinity");
    }
    if getcpu() != cpu {
        fail(b"getcpu after sched_setaffinity");
    }
}

/// Wait until `word` is not 0, sleeping in `futex`.
fn wait_for(word: &AtomicU32) {
    while word.load(Ordering::SeqCst) == 0 {
        // SAFETY: waits on our own word.
        unsafe { syscall(SYS_FUTEX, [word.as_ptr() as u64, FUTEX_WAIT, 0, 0, 0, 0]) };
    }
}

/// Set `word` to 1 and wake whoever waits on it.
fn set_and_wake(word: &AtomicU32) {
    word.store(1, Ordering::SeqCst);
    sys(SYS_FUTEX, [word.as_ptr() as u64, FUTEX_WAKE, 1, 0, 0, 0]);
}

/// Step 4: the CPU the second thread runs on, its signals, the counter it
/// stores to, the word that stops it, what the child stores, and the
/// thread's tid word and stack.
static OTHER_CPU: AtomicU64 = AtomicU64::new(0);
static ON_OTHER_CPU: AtomicU32 = AtomicU32::new(0);
static GO: AtomicU32 = AtomicU32::new(0);
static COUNTER: AtomicU64 = AtomicU64::new(0);
static STOP: AtomicU32 = AtomicU32::new(0);
static CHILD_MARK: AtomicU64 = AtomicU64::new(0);
static WRITER_TID: AtomicU32 = AtomicU32::new(0);
static mut WRITER_STACK: Stack = Stack([0; 1024]);

/// The thread of step 4: move to the other CPU, say so, wait for the go,
/// then count until told to stop.
extern "C" fn writer_main() -> u64 {
    pin(OTHER_CPU.load(Ordering::SeqCst));
    set_and_wake(&ON_OTHER_CPU);
    wait_for(&GO);
    while STOP.load(Ordering::Relaxed) == 0 {
        COUNTER.fetch_add(1, Ordering::Relaxed);
    }
    0
}

/// Step 4; `mask` holds two CPUs or more.
fn smp(mask: u64) {
    let here = u64::from(mask.trailing_zeros());
    let there = u64::from((mask & !(1 << here)).trailing_zeros());
    pin(here);
    OTHER_CPU.store(there, Ordering::SeqCst);
    let flags = CLONE_VM
        | CLONE_FS
        | CLONE_FILES
        | CLONE_SIGHAND
        | CLONE_THREAD
        | CLONE_SYSVSEM
        | CLONE_PARENT_SETTID
        | CLONE_CHILD_CLEARTID;
    let stack = (&raw mut WRITER_STACK) as u64 + 1024 * 8;
    let tid_word = WRITER_TID.as_ptr() as u64;
    // SAFETY: the stack is used by the thread alone, the words through
    // atomics, until it has exited.
    let tid = unsafe { clone_thread(flags, stack, tid_word, tid_word, 0, writer_main) };
    if is_err(tid) {
        fail(b"clone on several CPUs");
    }
    // Futex wakes both ways between the two CPUs.
    wait_for(&ON_OTHER_CPU);
    set_and_wake(&GO);
    while COUNTER.load(Ordering::SeqCst) == 0 {
        core::hint::spin_loop();
    }
    // Fork while the thread stores on the other CPU. The child moves to
    // that CPU and watches its copy of the counter while the thread runs
    // there too (each yield hands it the CPU).
    let child = fork();
    if child == 0 {
        pin(there);
        let before = COUNTER.load(Ordering::SeqCst);
        for _ in 0..5 {
            sys(SYS_SCHED_YIELD, [0; 6]);
        }
        let after = COUNTER.load(Ordering::SeqCst);
        CHILD_MARK.store(0x5eed, Ordering::SeqCst);
        exit(if before == after { 0 } else { 3 });
    }
    let (_, status) = wait(child);
    if status == 3 << 8 {
        fail(b"copy on write with a thread storing on another CPU (the child saw the parent's stores)");
    } else if status != 0 {
        fail(b"the child on the other CPU");
    }
    if CHILD_MARK.load(Ordering::SeqCst) != 0 {
        fail(b"copy on write across CPUs (the child's store reached the parent)");
    }
    let seen = COUNTER.load(Ordering::SeqCst);
    while COUNTER.load(Ordering::SeqCst) == seen {
        sys(SYS_SCHED_YIELD, [0; 6]);
    }
    STOP.store(1, Ordering::SeqCst);
    loop {
        let word = WRITER_TID.load(Ordering::SeqCst);
        if word == 0 {
            break;
        }
        // SAFETY: waits on our own word.
        unsafe { syscall(SYS_FUTEX, [tid_word, FUTEX_WAIT, u64::from(word), 0, 0, 0]) };
    }
    // Back to every CPU for the rest.
    if is_err(sys(SYS_SCHED_SETAFFINITY, [0, 8, &raw const mask as u64, 0, 0, 0])) {
        fail(b"sched_setaffinity back to every CPU");
    }
}

/// The thread of step 3: read the TLS word through `%fs`, note its tid.
extern "C" fn thread_main() -> u64 {
    let word: u64;
    // SAFETY: the thread's FS base points at TLS_BLOCK (CLONE_SETTLS).
    unsafe { asm!("mov {}, qword ptr fs:[0]", out(reg) word, options(nostack, readonly)) };
    THREAD_TLS.store(word, Ordering::SeqCst);
    THREAD_TID.store(sys(SYS_GETTID, [0; 6]), Ordering::SeqCst);
    0
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. fork + execve + wait4.
    let child = fork();
    if child == 0 {
        let path = b"/bin/hello\0";
        let argv = [path.as_ptr() as u64, 0];
        let envp = [b"FROM=forkexec\0".as_ptr() as u64, 0];
        // SAFETY: the strings and arrays live until the call replaces the
        // program (or fails).
        let err = unsafe { syscall(SYS_EXECVE, [path.as_ptr() as u64, argv.as_ptr() as u64, envp.as_ptr() as u64, 0, 0, 0]) };
        let _ = err;
        fail(b"execve /bin/hello");
    }
    let (_, status) = wait(child);
    if status != 0 {
        fail(b"the exit status of /bin/hello");
    }

    // 2. Copy on write: the child's change stays in its copy.
    let child = fork();
    if child == 0 {
        // SAFETY: one thread in this process.
        unsafe {
            if SHARED_UNTIL_WRITTEN != 1 {
                exit(9);
            }
            SHARED_UNTIL_WRITTEN = 5;
            exit(SHARED_UNTIL_WRITTEN)
        }
    }
    let (_, status) = wait(child);
    // SAFETY: one thread in this process.
    let mine = unsafe { SHARED_UNTIL_WRITTEN };
    if status != 5 << 8 || mine != 1 {
        fail(b"copy on write after fork");
    }

    // 3. A thread with TLS, joined through CLONE_CHILD_CLEARTID + futex.
    let flags = CLONE_VM
        | CLONE_FS
        | CLONE_FILES
        | CLONE_SIGHAND
        | CLONE_THREAD
        | CLONE_SYSVSEM
        | CLONE_SETTLS
        | CLONE_PARENT_SETTID
        | CLONE_CHILD_CLEARTID;
    let stack = (&raw mut THREAD_STACK) as u64 + 1024 * 8;
    let tid_word = TID_WORD.as_ptr() as u64;
    let tls = TLS_BLOCK.as_ptr() as u64;
    // SAFETY: the stack and the words are this program's, used by the
    // thread alone (or through atomics) until it has exited.
    let tid = unsafe { clone_thread(flags, stack, tid_word, tid_word, tls, thread_main) };
    if is_err(tid) {
        fail(b"clone");
    }
    loop {
        let word = TID_WORD.load(Ordering::SeqCst);
        if word == 0 {
            break;
        }
        if word as u64 != tid {
            fail(b"CLONE_PARENT_SETTID");
        }
        // SAFETY: waits on our own word.
        unsafe { syscall(SYS_FUTEX, [tid_word, FUTEX_WAIT, u64::from(word), 0, 0, 0]) };
    }
    if THREAD_TLS.load(Ordering::SeqCst) != 0x7e57_7155 || THREAD_TID.load(Ordering::SeqCst) != tid {
        fail(b"the thread's TLS or tid");
    }

    // 4. Two CPUs at once, when there are.
    let mask = cpus();
    if mask & (1 << getcpu()) == 0 {
        fail(b"getcpu outside the affinity mask");
    }
    if mask.count_ones() >= 2 {
        smp(mask);
    }

    // 5. An orphan comes to init (us) and init reaps it; only when this
    // program is init.
    let me = sys(SYS_GETPID, [0; 6]);
    if me != 1 {
        write(1, b"forkexec: ok\n");
        exit(0);
    }
    let child = fork();
    if child == 0 {
        let parent = sys(SYS_GETPID, [0; 6]);
        if fork() == 0 {
            while sys(SYS_GETPPID, [0; 6]) == parent {
                core::hint::spin_loop();
            }
            exit(if sys(SYS_GETPPID, [0; 6]) == me { 7 } else { 8 });
        }
        exit(0);
    }
    let (_, status) = wait(child);
    if status != 0 {
        fail(b"the orphan's parent");
    }
    let (_, status) = wait(u64::MAX);
    if status != 7 << 8 {
        fail(b"adoption of the orphan by init");
    }
    if sys(SYS_WAIT4, [u64::MAX, 0, 0, 0, 0, 0]) != (-(ECHILD as i64)) as u64 {
        fail(b"ECHILD with no children left");
    }
    write(1, b"forkexec: ok\n");
    exit(0)
}

// The entry point: the kernel starts the program with `rsp` at `argc`
// (16-byte aligned, psABI §3.4.1); `main` gets a normal call frame.
global_asm!(".globl _start", "_start:", "xor ebp, ebp", "and rsp, -16", "call main", "ud2");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail(b"a panic")
}
