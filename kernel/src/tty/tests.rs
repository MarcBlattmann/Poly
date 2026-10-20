//! In-kernel tests of terminals (ADR 0046): Linux processes whose threads
//! make real system calls from ring 3 on pseudo-terminals and the
//! console.
//!
//! As in `linux/fileio_tests.rs`, most checks run in *callbacks*: a
//! thread's program makes the system call [`CALLBACK`] with an index in
//! `rdi` (and an argument in `rsi`); the test hook runs that callback in
//! that system call, where it makes Linux system calls through
//! `Linux::dispatch_with` with real user memory and real blocking. Files
//! are opened with the process's `fs::Files::open`, the kernel side of
//! `openat`. What signals must reach a handler or kill a process happens
//! in ring 3 proper: a blocked read the process dies in, and a write that
//! a `SIGTTOU` handler interrupts. A thread that has to wait for another
//! to block looks at the wait queue it registers on, never at the clock.

use crate::fs::OpenFlags;
use crate::kernel_test;
use crate::linux::Thread;
use crate::process::{self, ExitStatus, Process};
use crate::sync::SpinLock;
use crate::user::memory::{Layout, Placement, Prot};
use crate::user::{self, Entry, Memory};
use crate::{thread, time};
use alloc::format;
use alloc::sync::Arc;
use core::arch::global_asm;
use core::sync::atomic::{AtomicI64, Ordering};
use core::time::Duration;
use poly_abi::SyscallRegs;
use poly_binfmt::{Arch, Bits, Format};
use poly_personality_linux::abi::signal::{SA_RESTORER, SIGINT, SIGTTOU, SIGWINCH};
use poly_personality_linux::{Errno, Linux, nr};
use poly_tty::ioctl::*;
use poly_tty::termios::{Termios, Winsize, cc, lflag};

type TestResult<T = ()> = Result<T, &'static str>;

/// The test hook's own system call: run callback `rdi` with `rsi`.
const CALLBACK: u64 = 0x7e59;
const CODE: u64 = 0x40_0000;
/// Four read-write pages, zeroed.
const DATA: u64 = 0x60_0000;
const DATA_LEN: u64 = 0x4000;
/// Where `tty_t_read` reads to, `tty_t_fork`'s child writes from, and the
/// signal handler stores the signal number.
const READ_BUF: u64 = DATA + 0x700;
const WRITE_BUF: u64 = DATA + 0x800;
const HANDLED: u64 = DATA + 0x900;
const DEADLINE_NS: u64 = 60 * time::NANOS_PER_SEC;
const SIG_BLOCK: u64 = 0;

global_asm!(
    r#"
    .section .rodata.tty_test_programs, "a", @progbits
    .balign 16
    .global tty_t_start
tty_t_start:

/* Callback rdi, then exit (this thread only) with its result. */
    .global tty_t_callback
tty_t_callback:
    mov eax, {callback}
    syscall
    mov rdi, rax
    mov eax, 60
    syscall
    ud2

/* Callback rdi returns a descriptor; read 16 bytes from it in ring 3;
   exit_group with 100 + the count, or with the errno. */
    .global tty_t_read
tty_t_read:
    mov eax, {callback}
    syscall
    mov rdi, rax
    xor eax, eax
    mov rsi, {read_buf}
    mov edx, 16
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
    ud2

/* Callback 3 sets up (0 to go on, else exit_group with it), then fork.
   The parent runs callback 0 with the child's pid and exits with its
   result. The child runs callback 1, which returns a descriptor, writes
   one byte to it in ring 3, runs callback 2 with the write's result and
   exits with that callback's result. */
    .global tty_t_fork
tty_t_fork:
    mov edi, 3
    mov eax, {callback}
    syscall
    test rax, rax
    jnz 9f
    mov eax, 57
    syscall
    test rax, rax
    jz 1f
    mov rsi, rax
    xor edi, edi
    mov eax, {callback}
    syscall
    mov rdi, rax
    mov eax, 231
    syscall
1:
    mov edi, 1
    mov eax, {callback}
    syscall
    mov rdi, rax
    mov rsi, {write_buf}
    mov edx, 1
    mov eax, 1
    syscall
    mov rsi, rax
    mov edi, 2
    mov eax, {callback}
    syscall
9:
    mov rdi, rax
    mov eax, 231
    syscall
    ud2

/* A signal handler: stores the signal number. */
    .global tty_t_handler
tty_t_handler:
    mov rax, {handled}
    mov [rax], rdi
    ret
    .global tty_t_restorer
tty_t_restorer:
    mov eax, 15
    syscall
    ud2

    .balign 16
    .global tty_t_end
tty_t_end:
"#,
    callback = const CALLBACK,
    read_buf = const READ_BUF,
    write_buf = const WRITE_BUF,
    handled = const HANDLED,
);

unsafe extern "C" {
    static tty_t_start: u8;
    static tty_t_end: u8;
    static tty_t_callback: u8;
    static tty_t_read: u8;
    static tty_t_fork: u8;
    static tty_t_handler: u8;
    static tty_t_restorer: u8;
}

/// A program's address once the blob is mapped at [`CODE`].
fn at(label: *const u8) -> u64 {
    CODE + (label as u64 - (&raw const tty_t_start) as u64)
}

fn blob() -> &'static [u8] {
    let start = &raw const tty_t_start;
    let len = (&raw const tty_t_end) as usize - start as usize;
    // SAFETY: the assembly above puts every program between the two
    // symbols, in one read-only section of the kernel image.
    unsafe { core::slice::from_raw_parts(start, len) }
}

type Callback = fn(&Process, u64) -> u64;

static CALLBACKS: SpinLock<[Option<Callback>; 4]> = SpinLock::new([None; 4]);
static RESULTS: SpinLock<[Option<u64>; 4]> = SpinLock::new([None; 4]);
/// Descriptors one callback hands the other (-1: not yet).
static SHARED: [AtomicI64; 2] = [AtomicI64::new(-1), AtomicI64::new(-1)];

fn hook(process: &Process, regs: &mut SyscallRegs) {
    if regs.rax == CALLBACK {
        let i = (regs.rdi as usize).min(3);
        let f = CALLBACKS.lock()[i].expect("a callback is set");
        let result = f(process, regs.rsi);
        RESULTS.lock()[i] = Some(result);
        regs.rax = result;
    } else {
        user::DEFAULT_HOOK(process, regs);
    }
}

/// A Linux process (the root of its own session) and a memory with the
/// programs, the data pages and a stack; `callbacks` installed.
fn setup(callbacks: [Option<Callback>; 4]) -> TestResult<(Arc<Process>, Arc<Memory>)> {
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
    *CALLBACKS.lock() = callbacks;
    *RESULTS.lock() = [None; 4];
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

/// Check that callback `i` returned 0.
fn passed(what: &'static str, i: usize) -> TestResult {
    match RESULTS.lock()[i] {
        Some(0) => Ok(()),
        r => {
            crate::kprintln!("tty test: {what}: callback {i} returned {r:?}");
            Err(what)
        }
    }
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
            crate::kprintln!("tty test: check {} failed: {}", $n, stringify!($cond));
            return $n;
        }
    };
}

fn poke(addr: u64, bytes: &[u8]) -> bool {
    user::copy_to_user(addr, bytes).is_ok()
}

fn peek(addr: u64, len: usize) -> alloc::vec::Vec<u8> {
    let mut b = alloc::vec![0u8; len];
    let _ = user::copy_from_user(&mut b, addr);
    b
}

fn peek_u64(addr: u64) -> u64 {
    u64::from_le_bytes(peek(addr, 8).try_into().expect("8 bytes"))
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

/// posix_openpt + grantpt + unlockpt + open(ptsname): a master and its
/// slave, opened by the calling process (`slave_flags` decide whether the
/// slave may become its controlling terminal). `(master, slave)`.
fn open_pair(p: &Process, slave_flags: OpenFlags) -> Option<(u64, u64)> {
    let m = p.files.open(b"/dev/ptmx", OpenFlags::RDWR | OpenFlags::NOCTTY, 0).ok()? as u64;
    sys(p, nr::IOCTL, &[m, u64::from(TIOCGPTN), DATA]).ok()?;
    let n = peek_u32(DATA);
    poke(DATA, &0u32.to_le_bytes()).then_some(())?;
    sys(p, nr::IOCTL, &[m, u64::from(TIOCSPTLCK), DATA]).ok()?;
    let s = p.files.open(format!("/dev/pts/{n}").as_bytes(), OpenFlags::RDWR | slave_flags, 0).ok()? as u64;
    Some((m, s))
}

fn ioctl(p: &Process, fd: u64, request: u32, arg: u64) -> Result<u64, Errno> {
    sys(p, nr::IOCTL, &[fd, u64::from(request), arg])
}

fn termios_at(addr: u64) -> Termios {
    Termios::sane(38400).from_bytes(&peek(addr, Termios::SIZE).try_into().expect("36 bytes"))
}

/// A signal set with `signal`, at `addr`.
fn sigset(addr: u64, signal: u32) -> bool {
    poke(addr, &(1u64 << (signal - 1)).to_le_bytes())
}

// ---- a line through a pty ------------------------------------------------------------

kernel_test! {
    fn pty_slave_reads_a_line_typed_on_the_master() -> TestResult {
        let reader: Callback = |p, _| {
            let Some((m, s)) = open_pair(p, OpenFlags::NOCTTY) else { return 1 };
            SHARED[1].store(s as i64, Ordering::SeqCst);
            SHARED[0].store(m as i64, Ordering::SeqCst);
            // Blocks until the master types a whole line.
            let got = sys(p, nr::READ, &[s, DATA + 0x100, 100]);
            check!(2, got == Ok(6));
            check!(3, peek(DATA + 0x100, 6) == b"hello\n");
            0
        };
        let typist: Callback = |p, _| {
            check!(11, until(|| shared(0) >= 0));
            let (m, s) = (shared(0) as u64, shared(1));
            check!(12, until(|| has_waiter(p, s)));
            check!(13, poke(DATA + 0x200, b"hello\r"));
            check!(14, sys(p, nr::WRITE, &[m, DATA + 0x200, 6]) == Ok(6));
            // The echo: CR became NL (ICRNL), sent as CR NL (ONLCR).
            check!(15, sys(p, nr::READ, &[m, DATA + 0x300, 100]) == Ok(7));
            check!(16, peek(DATA + 0x300, 7) == b"hello\r\n");
            0
        };
        let (process, memory) = setup([Some(reader), Some(typist), None, None])?;
        start(&process, &memory, &raw const tty_t_callback, [0, 0, 0], 0)?;
        start(&process, &memory, &raw const tty_t_callback, [1, 0, 0], 1)?;
        wait(&process)?;
        passed("a line through a pty: the reader", 0)?;
        passed("a line through a pty: the typist", 1)
    }
}

// ---- attributes ------------------------------------------------------------------------

kernel_test! {
    fn termios_round_trip_raw_mode_and_poll() -> TestResult {
        let checks: Callback = |p, _| {
            let Some((m, s)) = open_pair(p, OpenFlags::NOCTTY) else { return 1 };
            // isatty: TCGETS works on both ends, not on a pipe.
            check!(2, ioctl(p, s, TCGETS, DATA) == Ok(0));
            let t = termios_at(DATA);
            check!(3, t.lflag & (lflag::ICANON | lflag::ECHO | lflag::ISIG) == lflag::ICANON | lflag::ECHO | lflag::ISIG);
            check!(4, t.cc[cc::VINTR] == 3 && t.cc[cc::VMIN] == 1);
            check!(5, ioctl(p, m, TCGETS, DATA + 0x40) == Ok(0));
            check!(6, sys(p, nr::PIPE2, &[DATA + 0x80, 0]) == Ok(0));
            check!(7, ioctl(p, u64::from(peek_u32(DATA + 0x80)), TCGETS, DATA + 0x40) == Err(Errno::ENOTTY));
            // Raw mode: no line editing, no echo, no signals.
            let mut raw = t;
            raw.lflag &= !(lflag::ICANON | lflag::ECHO | lflag::ISIG);
            raw.cc[cc::VMIN] = 1;
            raw.cc[cc::VTIME] = 0;
            check!(8, poke(DATA, &raw.to_bytes()));
            check!(9, ioctl(p, s, TCSETS, DATA) == Ok(0));
            check!(10, ioctl(p, s, TCGETS, DATA + 0x40) == Ok(0));
            check!(11, peek(DATA + 0x40, Termios::SIZE) == raw.to_bytes());
            // poll: nothing to read yet; then three bytes, ^C among them.
            let pollfd = |fd: u64| {
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&(fd as i32).to_le_bytes());
                b[4..6].copy_from_slice(&1u16.to_le_bytes());
                poke(DATA + 0x100, &b)
            };
            check!(12, pollfd(s) && sys(p, nr::POLL, &[DATA + 0x100, 1, 0]) == Ok(0));
            // epoll on the slave: EPOLLIN, data 7 (a packed epoll_event).
            let Ok(ep) = sys(p, nr::EPOLL_CREATE1, &[0]) else { return 20 };
            let mut event = [0u8; 12];
            event[..4].copy_from_slice(&1u32.to_le_bytes());
            event[4..].copy_from_slice(&7u64.to_le_bytes());
            check!(21, poke(DATA + 0x180, &event));
            check!(22, sys(p, nr::EPOLL_CTL, &[ep, 1, s, DATA + 0x180]) == Ok(0));
            check!(23, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x1c0, 4, 0]) == Ok(0));
            check!(13, poke(DATA + 0x200, b"a\x03b"));
            check!(14, sys(p, nr::WRITE, &[m, DATA + 0x200, 3]) == Ok(3));
            check!(15, pollfd(s) && sys(p, nr::POLL, &[DATA + 0x100, 1, 0]) == Ok(1));
            check!(24, sys(p, nr::EPOLL_WAIT, &[ep, DATA + 0x1c0, 4, 0]) == Ok(1));
            check!(25, peek_u64(DATA + 0x1c4) == 7);
            check!(16, ioctl(p, s, FIONREAD, DATA + 0x300) == Ok(0) && peek_u32(DATA + 0x300) == 3);
            check!(17, sys(p, nr::READ, &[s, DATA + 0x400, 100]) == Ok(3));
            check!(18, peek(DATA + 0x400, 3) == b"a\x03b");
            // No echo reached the master.
            check!(19, pollfd(m) && sys(p, nr::POLL, &[DATA + 0x100, 1, 0]) == Ok(0));
            0
        };
        let (process, memory) = setup([Some(checks), None, None, None])?;
        start(&process, &memory, &raw const tty_t_callback, [0, 0, 0], 0)?;
        wait(&process)?;
        passed("termios round trip and raw mode", 0)
    }
}

// ---- signals ---------------------------------------------------------------------------

kernel_test! {
    fn ctrl_c_on_the_master_kills_the_foreground_group() -> TestResult {
        // The process leads its session: opening the slave without
        // O_NOCTTY makes it its controlling terminal, with its group in
        // the foreground. Its thread blocks reading it in ring 3.
        let reader: Callback = |p, _| {
            let Some((m, s)) = open_pair(p, OpenFlags::RDONLY) else { return u64::MAX };
            if ioctl(p, s, TIOCGPGRP, DATA) != Ok(0) || peek_u32(DATA) != p.pid {
                return u64::MAX;
            }
            SHARED[1].store(s as i64, Ordering::SeqCst);
            SHARED[0].store(m as i64, Ordering::SeqCst);
            s
        };
        let typist: Callback = |p, _| {
            check!(11, until(|| shared(0) >= 0));
            let (m, s) = (shared(0) as u64, shared(1));
            check!(12, until(|| has_waiter(p, s)));
            check!(13, poke(DATA + 0x200, b"\x03"));
            check!(14, sys(p, nr::WRITE, &[m, DATA + 0x200, 1]) == Ok(1));
            0
        };
        let (process, memory) = setup([Some(reader), Some(typist), None, None])?;
        start(&process, &memory, &raw const tty_t_read, [0, 0, 0], 0)?;
        start(&process, &memory, &raw const tty_t_callback, [1, 0, 0], 1)?;
        let status = wait(&process)?;
        passed("^C: the typist", 1)?;
        match status {
            ExitStatus::Signaled { signal: SIGINT, .. } => Ok(()),
            status => {
                crate::kprintln!("tty test: ^C: the reader's process ended with {status}");
                Err("^C typed on the master did not kill the foreground group with SIGINT")
            }
        }
    }
}

kernel_test! {
    fn a_background_writer_gets_sigttou_with_tostop() -> TestResult {
        // Before the fork: the slave becomes the controlling terminal
        // (the parent's group in the foreground), with TOSTOP; a SIGTTOU
        // handler is laid out for the child.
        let setup_cb: Callback = |p, _| {
            let Some((_m, s)) = open_pair(p, OpenFlags::RDONLY) else { return 31 };
            check!(32, ioctl(p, s, TCGETS, DATA) == Ok(0));
            let mut t = termios_at(DATA);
            t.lflag |= lflag::TOSTOP;
            check!(33, poke(DATA, &t.to_bytes()));
            check!(34, ioctl(p, s, TCSETS, DATA) == Ok(0));
            let mut act = [0u8; 32];
            act[0..8].copy_from_slice(&at(&raw const tty_t_handler).to_le_bytes());
            act[8..16].copy_from_slice(&SA_RESTORER.to_le_bytes());
            act[16..24].copy_from_slice(&at(&raw const tty_t_restorer).to_le_bytes());
            check!(35, poke(DATA + 0xa00, &act));
            check!(36, poke(WRITE_BUF, b"!"));
            SHARED[1].store(s as i64, Ordering::SeqCst);
            0
        };
        // The child: a process group of its own in the background (not
        // orphaned: its parent is in another group of the session).
        let child: Callback = |p, _| {
            if sys(p, nr::SETPGID, &[0, 0]) != Ok(0) {
                return u64::MAX;
            }
            if sys(p, nr::RT_SIGACTION, &[u64::from(SIGTTOU), DATA + 0xa00, 0, 8]) != Ok(0) {
                return u64::MAX;
            }
            shared(1) as u64
        };
        // After the write in ring 3: EINTR, and the handler ran.
        let child_after: Callback = |p, wrote| {
            check!(41, wrote == Errno::EINTR.to_rax());
            check!(42, peek_u64(HANDLED) == u64::from(SIGTTOU));
            // Blocking SIGTTOU lets the write through.
            check!(43, sigset(DATA + 0x40, SIGTTOU));
            check!(44, sys(p, nr::RT_SIGPROCMASK, &[SIG_BLOCK, DATA + 0x40, 0, 8]) == Ok(0));
            check!(45, sys(p, nr::WRITE, &[shared(1) as u64, WRITE_BUF, 1]) == Ok(1));
            0
        };
        let parent: Callback = |p, child| {
            check!(51, sys(p, nr::WAIT4, &[child, DATA + 0x600, 0, 0]) == Ok(child));
            check!(52, peek_u32(DATA + 0x600) == 0);
            // The parent's group stayed in the foreground.
            check!(53, ioctl(p, shared(1) as u64, TIOCGPGRP, DATA) == Ok(0) && peek_u32(DATA) == p.pid);
            0
        };
        let (process, memory) = setup([Some(parent), Some(child), Some(child_after), Some(setup_cb)])?;
        start(&process, &memory, &raw const tty_t_fork, [0, 0, 0], 0)?;
        let status = wait(&process)?;
        passed("SIGTTOU: the setup", 3)?;
        passed("SIGTTOU: the parent", 0)?;
        match status {
            ExitStatus::Exited(0) => Ok(()),
            status => {
                crate::kprintln!("tty test: SIGTTOU: the parent ended with {status}; child results {:?}", *RESULTS.lock());
                Err("a background write with TOSTOP did not get SIGTTOU")
            }
        }
    }
}

kernel_test! {
    fn window_size_changes_send_sigwinch() -> TestResult {
        let checks: Callback = |p, _| {
            let Some((m, s)) = open_pair(p, OpenFlags::RDONLY) else { return 1 };
            // SIGWINCH is ignored by default: blocked, it stays pending.
            check!(2, sigset(DATA, SIGWINCH));
            check!(3, sys(p, nr::RT_SIGPROCMASK, &[SIG_BLOCK, DATA, 0, 8]) == Ok(0));
            check!(4, ioctl(p, s, TIOCGWINSZ, DATA + 0x40) == Ok(0) && peek(DATA + 0x40, 8) == [0; 8]);
            check!(5, poke(DATA + 0x40, &Winsize::new(40, 120).to_bytes()));
            check!(6, ioctl(p, m, TIOCSWINSZ, DATA + 0x40) == Ok(0));
            check!(7, sys(p, nr::RT_SIGPENDING, &[DATA + 0x80, 8]) == Ok(0));
            check!(8, peek_u64(DATA + 0x80) & (1 << (SIGWINCH - 1)) != 0);
            check!(9, ioctl(p, s, TIOCGWINSZ, DATA + 0xc0) == Ok(0));
            check!(10, peek(DATA + 0xc0, 8) == Winsize::new(40, 120).to_bytes());
            0
        };
        let (process, memory) = setup([Some(checks), None, None, None])?;
        start(&process, &memory, &raw const tty_t_callback, [0, 0, 0], 0)?;
        wait(&process)?;
        passed("TIOCSWINSZ and SIGWINCH", 0)
    }
}

// ---- the console ---------------------------------------------------------------------------

kernel_test! {
    fn the_console_is_a_terminal_and_dev_tty_follows_tiocsctty() -> TestResult {
        let checks: Callback = |p, _| {
            // fds 0-2 are /dev/console: a terminal (isatty), which opening
            // never made the controlling terminal.
            check!(2, ioctl(p, 1, TCGETS, DATA) == Ok(0));
            check!(3, p.files.open(b"/dev/tty", OpenFlags::RDWR, 0).is_err());
            check!(4, ioctl(p, 0, TIOCGPGRP, DATA) == Err(Errno::ENOTTY));
            // A session leader takes it.
            check!(5, ioctl(p, 0, TIOCSCTTY, 0) == Ok(0));
            check!(6, ioctl(p, 0, TIOCGPGRP, DATA) == Ok(0) && peek_u32(DATA) == p.pid);
            check!(7, ioctl(p, 0, TIOCGSID, DATA) == Ok(0) && peek_u32(DATA) == p.pid);
            let Ok(fd) = p.files.open(b"/dev/tty", OpenFlags::RDWR, 0) else { return 8 };
            check!(9, ioctl(p, fd as u64, TCGETS, DATA) == Ok(0));
            0
        };
        let (process, memory) = setup([Some(checks), None, None, None])?;
        start(&process, &memory, &raw const tty_t_callback, [0, 0, 0], 0)?;
        wait(&process)?;
        passed("the console and /dev/tty", 0)?;
        // The session ended: the console is nobody's controlling terminal.
        match super::console() {
            Some(t) if t.session() == (None, None) => Ok(()),
            Some(_) => Err("the console stayed the controlling terminal of an ended session"),
            None => Err("no console terminal"),
        }
    }
}

kernel_test! {
    fn devpts_lists_open_masters() -> TestResult {
        let ptys = super::ptys().ok_or("no ptys")?;
        let before = ptys.indices();
        let f = crate::fs::Files::new();
        let m = f.open(b"/dev/ptmx", OpenFlags::RDWR, 0).map_err(|e| e.name())?;
        let after = ptys.indices();
        let new: alloc::vec::Vec<u32> = after.iter().copied().filter(|i| !before.contains(i)).collect();
        let [n] = new[..] else { return Err("opening /dev/ptmx made no pty") };
        let st = f.stat(format!("/dev/pts/{n}").as_bytes()).map_err(|e| e.name())?;
        if st.rdev != poly_vfs::DeviceId::new(poly_tty::pty::PTY_SLAVE_MAJOR, n) {
            return Err("/dev/pts/<n> is not the slave's device");
        }
        f.close(m).map_err(|e| e.name())?;
        if ptys.indices().contains(&n) || f.stat(format!("/dev/pts/{n}").as_bytes()).is_ok() {
            return Err("closing the master left its pty behind");
        }
        Ok(())
    }
}
