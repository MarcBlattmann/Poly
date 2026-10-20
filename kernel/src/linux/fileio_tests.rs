//! In-kernel tests of pipes, `poll` / `select`, `epoll` and the event
//! files (ADR 0042): real Linux processes whose threads make real system
//! calls from ring 3, two threads at a time where one has to wake the
//! other.
//!
//! As in `tests.rs`, most checks run in *callbacks*: a thread's program
//! makes the system call [`CALLBACK`] with an index in `rdi`; the test hook
//! runs that callback on the thread, in that system call, where it makes
//! Linux system calls through `Linux::dispatch_with` with real user memory
//! and real blocking. The program then ends the thread with the
//! callback's result (0: every check passed, else the first failed
//! check's number), which the hook also records. A thread that has to
//! wait for another one to block looks at the wait queue the other
//! registers on, never at the clock, so the tests hold on a loaded host.
//! Two programs run entirely in ring 3: a write that gets `SIGPIPE`, and
//! a blocked read that a signal handler interrupts (`EINTR`, and with
//! `SA_RESTART` a restart).

use super::Thread;
use crate::fs::{self, FileType, OpenFile, OpenFlags};
use crate::kernel_test;
use crate::process::{self, ExitStatus, Process};
use crate::sync::SpinLock;
use crate::user::memory::{Layout, Placement, Prot};
use crate::user::{self, Entry, Memory};
use crate::{thread, time};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use core::sync::atomic::{AtomicI64, Ordering};
use core::time::Duration;
use poly_abi::SyscallRegs;
use poly_binfmt::{Arch, Bits, Format};
use poly_personality_linux::abi::signal::{SA_RESTART, SA_RESTORER, SIGPIPE, SIGUSR1, SIGUSR2};
use poly_personality_linux::{Errno, Linux, nr};
use poly_vfs::pipe::Pipe;

type TestResult<T = ()> = Result<T, &'static str>;

/// The test hook's own system call: run callback `rdi`.
const CALLBACK: u64 = 0x7e58;
const CODE: u64 = 0x40_0000;
/// Four read-write pages, zeroed.
const DATA: u64 = 0x60_0000;
const DATA_LEN: u64 = 0x4000;
const DEADLINE_NS: u64 = 60 * time::NANOS_PER_SEC;

const O_NONBLOCK: u64 = 0o4000;
const O_CLOEXEC: u64 = 0o2000000;
const POLLIN: u16 = 0x1;
const EPOLLIN: u32 = 0x1;
const EPOLLET: u32 = 1 << 31;
const FIONREAD: u64 = 0x541B;
const MS: u64 = time::NANOS_PER_MILLI;

global_asm!(
    r#"
    .section .rodata.linux_fileio_test_programs, "a", @progbits
    .balign 16
    .global fio_test_start
fio_test_start:

/* Callback rdi, then exit (this thread only) with its result. */
    .global fio_callback
fio_callback:
    mov eax, {callback}
    syscall
    mov rdi, rax
    mov eax, 60
    syscall
    ud2

/* write(rdi, rsi, 1); exit_group(-rax) if it returns. */
    .global fio_sigpipe
fio_sigpipe:
    mov eax, 1
    mov edx, 1
    syscall
    neg rax
    mov rdi, rax
    mov eax, 231
    syscall
    ud2

/* rt_sigaction(SIGUSR1, rdi, NULL, 8), then read(rsi, rdx, 1): exit_group
   with 100 + the count, or with the errno if it failed (99: the
   sigaction failed). The handler stores 1 at the buffer + 8 (r14 still
   holds the buffer), then returns through the restorer. */
    .global fio_read_signal
fio_read_signal:
    mov r12, rdi
    mov r13, rsi
    mov r14, rdx
    mov eax, 13
    mov edi, 10
    mov rsi, r12
    xor edx, edx
    mov r10d, 8
    syscall
    test rax, rax
    jnz 9f
    xor eax, eax
    mov rdi, r13
    mov rsi, r14
    mov edx, 1
    syscall
    test rax, rax
    js 8f
    lea rdi, [rax + 100]
    mov eax, 231
    syscall
8:
    mov rdi, rax
    neg rdi
    mov eax, 231
    syscall
9:
    mov edi, 99
    mov eax, 231
    syscall
    ud2
    .global fio_handler
fio_handler:
    mov qword ptr [r14 + 8], 1
    ret
    .global fio_restorer
fio_restorer:
    mov eax, 15
    syscall
    ud2

    .balign 16
    .global fio_test_end
fio_test_end:
"#,
    callback = const CALLBACK,
);

unsafe extern "C" {
    static fio_test_start: u8;
    static fio_test_end: u8;
    static fio_callback: u8;
    static fio_sigpipe: u8;
    static fio_read_signal: u8;
    static fio_handler: u8;
    static fio_restorer: u8;
}

/// A program's address once the blob is mapped at [`CODE`].
fn at(label: *const u8) -> u64 {
    CODE + (label as u64 - (&raw const fio_test_start) as u64)
}

fn blob() -> &'static [u8] {
    let start = &raw const fio_test_start;
    let len = (&raw const fio_test_end) as usize - start as usize;
    // SAFETY: the assembly above puts every program between the two
    // symbols, in one read-only section of the kernel image.
    unsafe { core::slice::from_raw_parts(start, len) }
}

type Callback = fn(&Process) -> u64;

static CALLBACKS: SpinLock<[Option<Callback>; 2]> = SpinLock::new([None; 2]);
static RESULTS: SpinLock<[Option<u64>; 2]> = SpinLock::new([None; 2]);
/// Descriptors one callback hands the other (-1: not yet).
static SHARED: [AtomicI64; 3] = [AtomicI64::new(-1), AtomicI64::new(-1), AtomicI64::new(-1)];

fn hook(process: &Process, regs: &mut SyscallRegs) {
    if regs.rax == CALLBACK {
        let i = (regs.rdi as usize).min(1);
        let f = CALLBACKS.lock()[i].expect("a callback is set");
        let result = f(process);
        RESULTS.lock()[i] = Some(result);
        regs.rax = result;
    } else {
        user::DEFAULT_HOOK(process, regs);
    }
}

/// A Linux process and a memory with the programs, the data pages and a
/// stack.
fn setup() -> TestResult<(Arc<Process>, Arc<Memory>)> {
    let format = Format::Elf { bits: Bits::B64, arch: Arch::X86_64 };
    let process = Arc::new(Process::new_linux(process::alloc_pid(), 0, format, false));
    let memory = Memory::new(Layout::default()).map_err(|_| "no memory for a memory map")?;
    let code = blob();
    let len = poly_vm::page_up(code.len() as u64).expect("small");
    let ok = memory.map_anonymous(Placement::FixedNoReplace(CODE), len, Prot::RW).is_ok()
        && memory.write(CODE, code).is_ok()
        && memory.protect(CODE, len, Prot::RX).is_ok()
        && memory.map_anonymous(Placement::FixedNoReplace(DATA), DATA_LEN, Prot::RW).is_ok()
        && memory.setup_stack(64 << 10, Prot::RW).is_ok();
    if !ok {
        return Err("could not lay out the test memory");
    }
    for s in &SHARED {
        s.store(-1, Ordering::SeqCst);
    }
    *RESULTS.lock() = [None; 2];
    Ok((process, Arc::new(memory)))
}

/// Start a thread at `program` with `args`, on its own part of the stack.
fn start(process: &Arc<Process>, memory: &Arc<Memory>, program: *const u8, args: [u64; 3], slot: u64) -> TestResult {
    let rsp = memory.layout().stack_top - 64 - slot * 0x4000;
    let entry = Entry { rip: at(program), rsp, args };
    let tid = user::spawn_in(Arc::clone(process), Arc::clone(memory), entry, hook).map_err(|_| "spawn failed")?;
    let _ = thread::detach(tid);
    Ok(())
}

fn wait(process: &Process) -> TestResult<ExitStatus> {
    process.wait_until(time::now_ns() + DEADLINE_NS).ok_or("the test process did not end in time")
}

/// Run `callbacks` on one thread each of a new process (the first is the
/// main thread, whose id is the pid); every one must return 0.
fn run_threads(what: &'static str, callbacks: &[Callback]) -> TestResult {
    let (process, memory) = setup()?;
    {
        let mut c = CALLBACKS.lock();
        for (i, f) in callbacks.iter().enumerate() {
            c[i] = Some(*f);
        }
    }
    for i in 0..callbacks.len() {
        start(&process, &memory, &raw const fio_callback, [i as u64, 0, 0], i as u64)?;
    }
    wait(&process)?;
    let results = *RESULTS.lock();
    for (i, r) in results.iter().take(callbacks.len()).enumerate() {
        if *r != Some(0) {
            crate::kprintln!("fileio test: {what}: callback {i} returned {r:?}");
            return Err(what);
        }
    }
    Ok(())
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

macro_rules! check {
    ($n:expr, $cond:expr) => {
        if !$cond {
            crate::kprintln!("fileio test: check {} failed: {}", $n, stringify!($cond));
            return $n;
        }
    };
    ($n:expr, $cond:expr, $why:expr) => {
        if !$cond {
            crate::kprintln!("fileio test: check {} failed: {} ({})", $n, stringify!($cond), $why);
            return $n;
        }
    };
}

fn poke(addr: u64, bytes: &[u8]) -> bool {
    user::copy_to_user(addr, bytes).is_ok()
}

fn peek_u64(addr: u64) -> u64 {
    let mut b = [0u8; 8];
    let _ = user::copy_from_user(&mut b, addr);
    u64::from_le_bytes(b)
}

fn peek_u32(addr: u64) -> u32 {
    peek_u64(addr) as u32
}

/// Wait (polling, generously bounded) until `cond` holds.
fn until(mut cond: impl FnMut() -> bool) -> bool {
    let end = time::now_ns() + DEADLINE_NS;
    while !cond() {
        if time::now_ns() > end {
            return false;
        }
        thread::sleep(Duration::from_millis(1));
    }
    true
}

/// Whether some thread waits on the file open as `fd`.
fn has_waiter(p: &Process, fd: i64) -> bool {
    p.files.file(fd as i32).ok().and_then(|f| f.wait_queue().map(|q| !q.is_empty())).unwrap_or(false)
}

fn shared(i: usize) -> i64 {
    SHARED[i].load(Ordering::SeqCst)
}

/// A pipe made from kernel code, as pipe2(2) makes one: `(read, write)`.
fn kernel_pipe(p: &Process) -> TestResult<(i32, i32)> {
    let (r, w) = Pipe::pair(false);
    let node = fs::vfs().anon_node(FileType::Fifo, 0o600, &p.files.cred()).map_err(|_| "anon_node")?;
    let read = Arc::new(OpenFile::on_object(node.clone(), Arc::new(r), OpenFlags::RDONLY));
    let write = Arc::new(OpenFile::on_object(node, Arc::new(w), OpenFlags::WRONLY));
    let rfd = p.files.install(read, false).map_err(|_| "install")?;
    let wfd = p.files.install(write, false).map_err(|_| "install")?;
    Ok((rfd, wfd))
}

fn pipe_fds(p: &Process, at: u64, flags: u64) -> Option<(u64, u64)> {
    sys(p, nr::PIPE2, &[at, flags]).ok()?;
    Some((u64::from(peek_u32(at)), u64::from(peek_u32(at + 4))))
}

fn anon_map(p: &Process, len: u64) -> u64 {
    sys(p, nr::MMAP, &[0, len, 0x3, 0x22, u64::MAX, 0]).unwrap_or(0)
}

// ---- pipes -------------------------------------------------------------------

const BIG: u64 = 200_000;

kernel_test! {
    fn pipe_between_two_threads_blocks_and_wakes() -> TestResult {
        run_threads(
            "pipe between two threads",
            &[
                // The reader: blocks on the empty pipe, then drains a write
                // three times the pipe's size, then sees end of file.
                |p| {
                    let Some((r, w)) = pipe_fds(p, DATA, 0) else { return 1 };
                    SHARED[1].store(w as i64, Ordering::SeqCst);
                    SHARED[0].store(r as i64, Ordering::SeqCst);
                    let got = sys(p, nr::READ, &[r, DATA + 0x100, 100]);
                    // The writer goes on once this read is done.
                    SHARED[2].store(1, Ordering::SeqCst);
                    if got != Ok(5) {
                        // Let the writer finish (EPIPE) instead of blocking.
                        let _ = sys(p, nr::CLOSE, &[r]);
                    }
                    check!(2, got == Ok(5));
                    let mut hello = [0u8; 5];
                    check!(3, user::copy_from_user(&mut hello, DATA + 0x100).is_ok() && &hello == b"hello");
                    let buf = anon_map(p, 8192);
                    check!(4, buf != 0);
                    let mut got = 0u64;
                    let mut chunk = vec![0u8; 7000];
                    loop {
                        match sys(p, nr::READ, &[r, buf, 7000]) {
                            Ok(0) => break,
                            Ok(n) => {
                                check!(5, user::copy_from_user(&mut chunk[..n as usize], buf).is_ok());
                                let ok = chunk[..n as usize].iter().enumerate().all(|(i, &b)| b == ((got + i as u64) % 251) as u8);
                                check!(6, ok);
                                got += n;
                            }
                            Err(_) => return 7,
                        }
                    }
                    check!(8, got == BIG);
                    0
                },
                // The writer: waits for the reader to block, writes.
                |p| {
                    check!(11, until(|| shared(0) >= 0));
                    let (r, w) = (shared(0), shared(1) as u64);
                    check!(12, until(|| has_waiter(p, r)));
                    check!(13, poke(DATA + 0x800, b"hello"));
                    check!(14, sys(p, nr::WRITE, &[w, DATA + 0x800, 5]) == Ok(5));
                    check!(19, until(|| shared(2) >= 0), "the reader took the first write alone");
                    let buf = anon_map(p, BIG.next_multiple_of(4096));
                    check!(15, buf != 0);
                    let data: Vec<u8> = (0..BIG).map(|i| (i % 251) as u8).collect();
                    check!(16, poke(buf, &data));
                    // Blocks while the pipe is full, until all of it is in.
                    check!(17, sys(p, nr::WRITE, &[w, buf, BIG]) == Ok(BIG));
                    check!(18, sys(p, nr::CLOSE, &[w]) == Ok(0));
                    0
                },
            ],
        )
    }
}

kernel_test! {
    fn nonblocking_pipe_fionread_and_epipe() -> TestResult {
        run_threads("non-blocking pipe", &[|p| {
            let Some((r, w)) = pipe_fds(p, DATA, O_NONBLOCK | O_CLOEXEC) else { return 1 };
            check!(2, (r, w) == (3, 4));
            check!(3, sys(p, nr::READ, &[r, DATA + 0x100, 10]) == Err(Errno::EAGAIN));
            check!(4, sys(p, nr::WRITE, &[w, DATA + 0x200, 10]) == Ok(10));
            check!(5, sys(p, nr::IOCTL, &[r, FIONREAD, DATA + 0x300]) == Ok(0) && peek_u32(DATA + 0x300) == 10);
            // st_mode at offset 24 of struct stat: a FIFO.
            check!(6, sys(p, nr::FSTAT, &[r, DATA + 0x400]) == Ok(0));
            check!(7, peek_u32(DATA + 0x400 + 24) & 0o170000 == 0o010000);
            // Fill it: 64 KiB, then EAGAIN, also for one byte.
            // Writes above PIPE_BUF take what fits.
            let mut total = 10;
            while let Ok(n) = sys(p, nr::WRITE, &[w, DATA + 0x1000, 4097]) {
                total += n;
            }
            check!(8, total == 65536);
            check!(9, sys(p, nr::WRITE, &[w, DATA + 0x1000, 1]) == Err(Errno::EAGAIN));
            check!(10, sys(p, nr::PIPE2, &[DATA, 0o200]) == Err(Errno::EINVAL));
            check!(11, sys(p, nr::MMAP, &[0, 4096, 1, 2, r, 0]) == Err(Errno::ENODEV));
            // SIGPIPE blocked: EPIPE, and SIGPIPE pending.
            check!(12, poke(DATA + 0x500, &(1u64 << (SIGPIPE - 1)).to_le_bytes()));
            check!(13, sys(p, nr::RT_SIGPROCMASK, &[0, DATA + 0x500, 0, 8]) == Ok(0));
            check!(14, sys(p, nr::CLOSE, &[r]) == Ok(0));
            check!(15, sys(p, nr::WRITE, &[w, DATA + 0x1000, 1]) == Err(Errno::EPIPE));
            check!(16, sys(p, nr::RT_SIGPENDING, &[DATA + 0x600, 8]) == Ok(0));
            check!(17, peek_u64(DATA + 0x600) & (1 << (SIGPIPE - 1)) != 0);
            0
        }])?;
        // Not blocked: SIGPIPE's default action ends the writer.
        let (process, memory) = setup()?;
        let (r, w) = kernel_pipe(&process)?;
        process.files.close(r).map_err(|_| "close")?;
        start(&process, &memory, &raw const fio_sigpipe, [w as u64, DATA, 0], 0)?;
        match wait(&process)? {
            ExitStatus::Signaled { signal: SIGPIPE, .. } => Ok(()),
            status => {
                crate::kprintln!("fileio test: write without readers: {status}");
                Err("a write to a pipe without readers did not end the process with SIGPIPE")
            }
        }
    }
}

// ---- poll, select, epoll -------------------------------------------------------

/// Store a `struct epoll_event` (12 bytes, packed) at `at`.
fn epoll_event(at: u64, events: u32, data: u64) -> bool {
    let mut b = [0u8; 12];
    b[..4].copy_from_slice(&events.to_le_bytes());
    b[4..].copy_from_slice(&data.to_le_bytes());
    poke(at, &b)
}

fn pollfd(at: u64, fd: u64, events: u16) -> bool {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&(fd as i32).to_le_bytes());
    b[4..6].copy_from_slice(&events.to_le_bytes());
    poke(at, &b)
}

kernel_test! {
    fn poll_select_and_epoll_on_pipes_and_eventfd() -> TestResult {
        run_threads(
            "poll and epoll",
            &[
                |p| {
                    // An event counter under poll.
                    let Ok(efd) = sys(p, nr::EVENTFD2, &[0, O_NONBLOCK]) else { return 1 };
                    check!(2, pollfd(DATA, efd, POLLIN));
                    check!(3, sys(p, nr::POLL, &[DATA, 1, 0]) == Ok(0));
                    check!(4, poke(DATA + 0x100, &3u64.to_le_bytes()));
                    check!(5, sys(p, nr::WRITE, &[efd, DATA + 0x100, 8]) == Ok(8));
                    check!(6, sys(p, nr::POLL, &[DATA, 1, 0]) == Ok(1));
                    check!(7, peek_u64(DATA) >> 48 == u64::from(POLLIN), "revents");
                    check!(8, sys(p, nr::READ, &[efd, DATA + 0x108, 8]) == Ok(8) && peek_u64(DATA + 0x108) == 3);
                    check!(9, sys(p, nr::READ, &[efd, DATA + 0x108, 8]) == Err(Errno::EAGAIN));
                    // A poll that times out waits at least its timeout.
                    let Some((r, w)) = pipe_fds(p, DATA + 0x200, 0) else { return 10 };
                    check!(11, pollfd(DATA, r, POLLIN));
                    let before = time::now_ns();
                    check!(12, sys(p, nr::POLL, &[DATA, 1, 30]) == Ok(0));
                    check!(13, time::now_ns() - before >= 30 * MS);
                    // An epoll set: the pipe level-triggered, the counter
                    // edge-triggered.
                    let Ok(ep) = sys(p, nr::EPOLL_CREATE1, &[O_CLOEXEC]) else { return 14 };
                    check!(15, epoll_event(DATA + 0x300, EPOLLIN, 0x1234));
                    check!(16, sys(p, nr::EPOLL_CTL, &[ep, 1, r, DATA + 0x300]) == Ok(0));
                    check!(17, epoll_event(DATA + 0x300, EPOLLIN | EPOLLET, 0x5678));
                    check!(18, sys(p, nr::EPOLL_CTL, &[ep, 1, efd, DATA + 0x300]) == Ok(0));
                    check!(19, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x400, 4, 0]) == Ok(0));
                    SHARED[1].store(w as i64, Ordering::SeqCst);
                    SHARED[2].store(ep as i64, Ordering::SeqCst);
                    SHARED[0].store(r as i64, Ordering::SeqCst);
                    // Blocks until the other thread writes to the pipe.
                    check!(20, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x400, 4, u64::MAX]) == Ok(1));
                    check!(21, peek_u32(DATA + 0x400) == EPOLLIN && peek_u64(DATA + 0x404) == 0x1234);
                    check!(22, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x400, 4, 0]) == Ok(1), "level-triggered");
                    check!(23, sys(p, nr::READ, &[r, DATA + 0x500, 16]) == Ok(1));
                    check!(24, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x400, 4, 0]) == Ok(0));
                    check!(25, sys(p, nr::WRITE, &[efd, DATA + 0x100, 8]) == Ok(8));
                    check!(26, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x400, 4, 0]) == Ok(1));
                    check!(27, peek_u64(DATA + 0x404) == 0x5678);
                    check!(28, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x400, 4, 0]) == Ok(0), "edge-triggered");
                    // select on the pipe: not readable, then readable.
                    check!(29, poke(DATA + 0x600, &(1u64 << r).to_le_bytes()));
                    check!(30, poke(DATA + 0x700, &[0u8; 16]));
                    check!(31, sys(p, nr::SELECT, &[r + 1, DATA + 0x600, 0, 0, DATA + 0x700]) == Ok(0));
                    check!(32, peek_u64(DATA + 0x600) == 0);
                    check!(33, sys(p, nr::WRITE, &[w, DATA + 0x500, 1]) == Ok(1));
                    check!(34, poke(DATA + 0x600, &(1u64 << r).to_le_bytes()));
                    check!(35, sys(p, nr::SELECT, &[r + 1, DATA + 0x600, 0, 0, DATA + 0x700]) == Ok(1));
                    check!(36, peek_u64(DATA + 0x600) == 1 << r);
                    0
                },
                |p| {
                    check!(41, until(|| shared(0) >= 0));
                    let (w, ep) = (shared(1) as u64, shared(2));
                    check!(42, until(|| has_waiter(p, ep)), "the other thread waits in epoll_wait");
                    check!(43, poke(DATA + 0x900, b"!"));
                    check!(44, sys(p, nr::WRITE, &[w, DATA + 0x900, 1]) == Ok(1));
                    0
                },
            ],
        )
    }
}

// ---- timers --------------------------------------------------------------------

/// Store a `struct itimerspec` (interval, value) at `at`.
fn itimerspec(at: u64, interval_ns: u64, value_ns: u64) -> bool {
    let mut b = [0u8; 32];
    let ts = |ns: u64| {
        let mut t = [0u8; 16];
        t[..8].copy_from_slice(&(ns / time::NANOS_PER_SEC).to_le_bytes());
        t[8..].copy_from_slice(&(ns % time::NANOS_PER_SEC).to_le_bytes());
        t
    };
    b[..16].copy_from_slice(&ts(interval_ns));
    b[16..].copy_from_slice(&ts(value_ns));
    poke(at, &b)
}

fn timespec_ns(at: u64) -> u64 {
    peek_u64(at) * time::NANOS_PER_SEC + peek_u64(at + 8)
}

kernel_test! {
    fn timerfd_expirations() -> TestResult {
        run_threads("timer files", &[|p| {
            let Ok(t) = sys(p, nr::TIMERFD_CREATE, &[1, 0]) else { return 1 };
            check!(2, sys(p, nr::TIMERFD_GETTIME, &[t, DATA]) == Ok(0) && timespec_ns(DATA + 16) == 0);
            // Every 20 ms from 20 ms on; a blocking read waits for the first.
            let before = time::now_ns();
            check!(3, itimerspec(DATA, 20 * MS, 20 * MS));
            check!(4, sys(p, nr::TIMERFD_SETTIME, &[t, 0, DATA, DATA + 0x100]) == Ok(0));
            check!(5, sys(p, nr::READ, &[t, DATA + 0x200, 8]) == Ok(8));
            check!(6, peek_u64(DATA + 0x200) >= 1);
            check!(7, time::now_ns() - before >= 20 * MS);
            check!(8, sys(p, nr::READ, &[t, DATA + 0x200, 4]) == Err(Errno::EINVAL));
            check!(9, sys(p, nr::TIMERFD_GETTIME, &[t, DATA + 0x300]) == Ok(0));
            check!(10, timespec_ns(DATA + 0x300) == 20 * MS);
            let left = timespec_ns(DATA + 0x310);
            check!(11, left > 0 && left <= 20 * MS);
            // Under epoll (edge-triggered), the timer thread's callback
            // wakes the waiter.
            let Ok(ep) = sys(p, nr::EPOLL_CREATE1, &[0]) else { return 12 };
            check!(13, epoll_event(DATA + 0x400, EPOLLIN | EPOLLET, 7));
            check!(14, sys(p, nr::EPOLL_CTL, &[ep, 1, t, DATA + 0x400]) == Ok(0));
            check!(15, sys(p, nr::READ, &[t, DATA + 0x200, 8]) == Ok(8));
            check!(16, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x500, 4, u64::MAX]) == Ok(1));
            check!(17, peek_u64(DATA + 0x504) == 7);
            // Disarmed: the old setting comes back, the callback is gone.
            check!(18, itimerspec(DATA, 0, 0));
            check!(19, sys(p, nr::TIMERFD_SETTIME, &[t, 0, DATA, DATA + 0x100]) == Ok(0));
            check!(20, timespec_ns(DATA + 0x100) == 20 * MS);
            check!(21, sys(p, nr::TIMERFD_GETTIME, &[t, DATA + 0x300]) == Ok(0) && timespec_ns(DATA + 0x310) == 0);
            check!(22, fs::poll::pending_timers() == 0);
            // Absolute times, in the past (expires at once) and on the
            // wall clock.
            check!(23, sys(p, nr::CLOCK_GETTIME, &[1, DATA + 0x600]) == Ok(0));
            check!(24, itimerspec(DATA, 0, timespec_ns(DATA + 0x600) - MS));
            check!(25, sys(p, nr::TIMERFD_SETTIME, &[t, 1, DATA, 0]) == Ok(0));
            check!(26, sys(p, nr::READ, &[t, DATA + 0x200, 8]) == Ok(8) && peek_u64(DATA + 0x200) == 1);
            let Ok(rt) = sys(p, nr::TIMERFD_CREATE, &[0, O_NONBLOCK]) else { return 27 };
            check!(28, sys(p, nr::CLOCK_GETTIME, &[0, DATA + 0x600]) == Ok(0));
            check!(29, itimerspec(DATA, 0, timespec_ns(DATA + 0x600) + 10 * MS));
            check!(30, sys(p, nr::TIMERFD_SETTIME, &[rt, 1, DATA, 0]) == Ok(0));
            check!(31, sys(p, nr::READ, &[rt, DATA + 0x200, 8]) == Err(Errno::EAGAIN));
            check!(32, pollfd(DATA + 0x700, rt, POLLIN));
            check!(33, sys(p, nr::POLL, &[DATA + 0x700, 1, u64::MAX]) == Ok(1));
            check!(34, sys(p, nr::READ, &[rt, DATA + 0x200, 8]) == Ok(8));
            check!(35, sys(p, nr::CLOSE, &[rt]) == Ok(0) && sys(p, nr::CLOSE, &[t]) == Ok(0));
            0
        }])
    }
}

// ---- signal files --------------------------------------------------------------

fn sigset(at: u64, signals: &[u32]) -> bool {
    poke(at, &signals.iter().fold(0u64, |s, &n| s | 1 << (n - 1)).to_le_bytes())
}

kernel_test! {
    fn signalfd_reads_signals() -> TestResult {
        run_threads(
            "signal files",
            &[
                |p| {
                    let pid = u64::from(p.pid);
                    check!(1, sigset(DATA, &[SIGUSR1, SIGUSR2]));
                    check!(2, sys(p, nr::RT_SIGPROCMASK, &[0, DATA, 0, 8]) == Ok(0));
                    // Both threads block them, so no handler or default
                    // action takes what the signal files are to read.
                    check!(25, until(|| shared(2) >= 0));
                    check!(3, sigset(DATA, &[SIGUSR1]));
                    let Ok(nb) = sys(p, nr::SIGNALFD4, &[u64::MAX, DATA, 8, O_NONBLOCK]) else { return 4 };
                    check!(5, sys(p, nr::READ, &[nb, DATA + 0x100, 128]) == Err(Errno::EAGAIN));
                    check!(6, sys(p, nr::READ, &[nb, DATA + 0x100, 64]) == Err(Errno::EINVAL));
                    check!(7, sys(p, nr::KILL, &[pid, u64::from(SIGUSR1)]) == Ok(0));
                    check!(8, sys(p, nr::READ, &[nb, DATA + 0x100, 256]) == Ok(128), "one signal pending");
                    check!(9, peek_u32(DATA + 0x100) == SIGUSR1, "ssi_signo");
                    check!(10, peek_u32(DATA + 0x108) == 0, "ssi_code SI_USER");
                    check!(11, peek_u32(DATA + 0x10c) == p.pid, "ssi_pid");
                    // Readiness follows the pending signals.
                    check!(12, pollfd(DATA + 0x200, nb, POLLIN));
                    check!(13, sys(p, nr::POLL, &[DATA + 0x200, 1, 0]) == Ok(0));
                    check!(14, sys(p, nr::KILL, &[pid, u64::from(SIGUSR1)]) == Ok(0));
                    check!(15, sys(p, nr::POLL, &[DATA + 0x200, 1, 0]) == Ok(1));
                    check!(16, sys(p, nr::READ, &[nb, DATA + 0x100, 128]) == Ok(128));
                    // A signal outside the mask stays pending.
                    check!(17, sys(p, nr::KILL, &[pid, u64::from(SIGUSR2)]) == Ok(0));
                    check!(18, sys(p, nr::READ, &[nb, DATA + 0x100, 128]) == Err(Errno::EAGAIN));
                    check!(19, sys(p, nr::RT_SIGPENDING, &[DATA + 0x300, 8]) == Ok(0));
                    check!(20, peek_u64(DATA + 0x300) == 1 << (SIGUSR2 - 1));
                    // A blocking read waits for the other thread's tgkill.
                    let Ok(sfd) = sys(p, nr::SIGNALFD4, &[u64::MAX, DATA, 8, 0]) else { return 21 };
                    SHARED[0].store(sfd as i64, Ordering::SeqCst);
                    check!(22, sys(p, nr::READ, &[sfd, DATA + 0x400, 128]) == Ok(128));
                    check!(23, peek_u32(DATA + 0x400) == SIGUSR1);
                    check!(24, peek_u32(DATA + 0x408) as i32 == -6, "ssi_code SI_TKILL");
                    0
                },
                |p| {
                    check!(34, sigset(DATA + 0x800, &[SIGUSR1, SIGUSR2]));
                    check!(35, sys(p, nr::RT_SIGPROCMASK, &[0, DATA + 0x800, 0, 8]) == Ok(0));
                    SHARED[2].store(1, Ordering::SeqCst);
                    check!(31, until(|| shared(0) >= 0));
                    check!(32, until(|| has_waiter(p, shared(0))));
                    let pid = u64::from(p.pid);
                    check!(33, sys(p, nr::TGKILL, &[pid, pid, u64::from(SIGUSR1)]) == Ok(0));
                    0
                },
            ],
        )
    }
}

// ---- interrupted reads -----------------------------------------------------------

/// The signaller of [`interrupted_read`]: waits for the reader to block on
/// the pipe, sends it `SIGUSR1`, and with `SA_RESTART` waits for the
/// handler and the restarted read, then writes one byte.
fn signal_the_reader(p: &Process, restart: bool) -> u64 {
    let (r, w) = (shared(0), shared(1) as u64);
    check!(51, until(|| has_waiter(p, r)));
    let pid = u64::from(p.pid);
    check!(52, sys(p, nr::TGKILL, &[pid, pid, u64::from(SIGUSR1)]) == Ok(0));
    if restart {
        check!(53, until(|| peek_u64(DATA + 0x108) == 1), "the handler ran");
        check!(54, until(|| has_waiter(p, r)), "the read was restarted");
        check!(55, poke(DATA + 0x200, b"x"));
        check!(56, sys(p, nr::WRITE, &[w, DATA + 0x200, 1]) == Ok(1));
    }
    0
}

/// A ring-3 read blocked on an empty pipe, interrupted by a handled
/// signal: `EINTR` without `SA_RESTART`, restarted (and then reading the
/// byte written) with it. Returns the reader's exit code.
fn interrupted_read(restart: bool) -> TestResult<u8> {
    let (process, memory) = setup()?;
    let (r, w) = kernel_pipe(&process)?;
    SHARED[1].store(i64::from(w), Ordering::SeqCst);
    SHARED[0].store(i64::from(r), Ordering::SeqCst);
    // struct sigaction: handler, flags, restorer, mask.
    let flags = SA_RESTORER | if restart { SA_RESTART } else { 0 };
    let mut act = [0u8; 32];
    act[0..8].copy_from_slice(&at(&raw const fio_handler).to_le_bytes());
    act[8..16].copy_from_slice(&flags.to_le_bytes());
    act[16..24].copy_from_slice(&at(&raw const fio_restorer).to_le_bytes());
    memory.write(DATA, &act).map_err(|_| "writing the data page failed")?;
    let signaller: Callback = if restart { |p| signal_the_reader(p, true) } else { |p| signal_the_reader(p, false) };
    CALLBACKS.lock()[1] = Some(signaller);
    start(&process, &memory, &raw const fio_read_signal, [DATA, r as u64, DATA + 0x100], 0)?;
    start(&process, &memory, &raw const fio_callback, [1, 0, 0], 1)?;
    let status = wait(&process)?;
    if RESULTS.lock()[1] != Some(0) {
        return Err("the signalling thread's checks failed");
    }
    match status {
        ExitStatus::Exited(code) => Ok(code),
        _ => {
            crate::kprintln!("fileio test: interrupted read: {status}");
            Err("the reader did not exit")
        }
    }
}

kernel_test! {
    fn blocked_read_is_interrupted_by_a_signal() -> TestResult {
        // EINTR (4) without SA_RESTART.
        if interrupted_read(false)? != 4 {
            return Err("a handled signal did not end the blocked read with EINTR");
        }
        // Restarted with SA_RESTART: the byte is read (100 + 1).
        if interrupted_read(true)? != 101 {
            return Err("the read interrupted under SA_RESTART was not restarted");
        }
        Ok(())
    }
}
