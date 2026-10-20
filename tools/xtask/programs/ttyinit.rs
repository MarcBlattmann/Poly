//! A static Linux program without any C library that uses its terminal
//! interactively (roadmap Phase 4, ADR 0046), for the kernel's boot tests:
//! `cargo xtask` boots it as the first process (`poly.init=/bin/ttyinit`)
//! on the serial console and types into QEMU's standard input, which is
//! the serial port (`tools/xtask/src/programs.rs`).
//!
//! 1. Takes the console as its controlling terminal (`TIOCSCTTY`: init
//!    leads its session) and checks it is in the foreground (`TIOCGPGRP`).
//! 2. Prints `ttyinit: name? ` and reads a line in canonical mode; the
//!    test types `pola`, DEL, `y`, CR, so the line is `poly`.
//! 3. Installs a `SIGINT` handler, prints `ttyinit: press ^C> ` and reads;
//!    the test types Ctrl-C: the read fails with `EINTR` and the handler
//!    ran (the terminal sent `SIGINT` to the foreground group).
//! 4. Switches to raw mode (`TCSETS`: no `ICANON`, no `ECHO`, `MIN` 1),
//!    prints `ttyinit: raw> ` and reads three single keys, `xyz`; then
//!    puts the attributes back.
//! 5. A pseudo-terminal as posix_openpt(3), grantpt(3), unlockpt(3) and
//!    ptsname(3) make one: `openat("/dev/ptmx")`, `TIOCGPTN`,
//!    `TIOCSPTLCK`, `openat("/dev/pts/<n>")`; a line typed on the master
//!    is read on the slave, and its echo on the master.
//!
//! It prints what it got (`ttyinit: line [poly]`, `ttyinit: interrupted
//! by SIGINT`, `ttyinit: raw [xyz]`, `ttyinit: pty [pty]`), then
//! `ttyinit: ok`, and exits 0;
//! any failure prints what failed and exits 1. Every call is a raw
//! `syscall` (numbers from `asm/unistd_64.h`, ioctls from
//! `asm-generic/ioctls.h`, `struct termios` from `asm-generic/termbits.h`).
//! Written for Poly from the man pages.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering};

const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPENAT: u64 = 257;
const SYS_RT_SIGACTION: u64 = 13;
const SYS_IOCTL: u64 = 16;
const SYS_GETPID: u64 = 39;
const SYS_EXIT_GROUP: u64 = 231;

const TCGETS: u64 = 0x5401;
const TCSETS: u64 = 0x5402;
const TIOCSCTTY: u64 = 0x540E;
const TIOCGPGRP: u64 = 0x540F;
const TIOCGPTN: u64 = 0x8004_5430;
const TIOCSPTLCK: u64 = 0x4004_5431;
const AT_FDCWD: u64 = -100i64 as u64;
const O_RDWR: u64 = 0o2;
const O_NOCTTY: u64 = 0o400;
const SIGINT: u64 = 2;
const SA_RESTORER: u64 = 0x0400_0000;
const EINTR: u64 = 4;
const ICANON: u32 = 0x2;
const ECHO: u32 = 0x8;
const VTIME: usize = 5;
const VMIN: usize = 6;

/// A system call with up to four arguments; returns `rax` (`-errno` on
/// failure).
///
/// # Safety
/// The call must not break the program's memory.
unsafe fn syscall(nr: u64, a: [u64; 4]) -> u64 {
    let ret;
    // SAFETY: the x86-64 Linux convention: rcx and r11 are clobbered,
    // memory may be read and written by the kernel.
    unsafe {
        asm!("syscall", inlateout("rax") nr => ret, in("rdi") a[0], in("rsi") a[1], in("rdx") a[2],
             in("r10") a[3], lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

fn sys(nr: u64, a: [u64; 4]) -> u64 {
    // SAFETY: only calls that write into this program's own buffers go
    // through here.
    unsafe { syscall(nr, a) }
}

fn write(s: &[u8]) {
    sys(SYS_WRITE, [1, s.as_ptr() as u64, s.len() as u64, 0]);
}

fn exit(code: u64) -> ! {
    loop {
        sys(SYS_EXIT_GROUP, [code, 0, 0, 0]);
    }
}

fn fail(what: &[u8]) -> ! {
    write(b"ttyinit: ");
    write(what);
    write(b" failed\n");
    exit(1)
}

fn is_err(ret: u64) -> bool {
    ret > u64::MAX - 4095
}

fn read(buf: &mut [u8]) -> u64 {
    read_fd(0, buf)
}

fn read_fd(fd: u64, buf: &mut [u8]) -> u64 {
    sys(SYS_READ, [fd, buf.as_mut_ptr() as u64, buf.len() as u64, 0])
}

/// `openat(AT_FDCWD, path, O_RDWR | O_NOCTTY)`; `path` ends in a NUL.
fn open_rw(path: &[u8]) -> u64 {
    let fd = sys(SYS_OPENAT, [AT_FDCWD, path.as_ptr() as u64, O_RDWR | O_NOCTTY, 0]);
    if is_err(fd) {
        fail(b"openat");
    }
    fd
}

/// The kernel's `struct termios` (36 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [u8; 19],
}

/// The signal the handler saw.
static HANDLED: AtomicU64 = AtomicU64::new(0);

extern "C" fn on_signal(signal: u64) {
    HANDLED.store(signal, Ordering::SeqCst);
}

global_asm!(
    r#"
    .global ttyinit_restorer
ttyinit_restorer:
    mov eax, 15
    syscall
    ud2
"#
);

unsafe extern "C" {
    fn ttyinit_restorer();
}

/// `struct sigaction` for `rt_sigaction`: handler, flags, restorer, mask.
#[repr(C)]
struct SigAction {
    handler: u64,
    flags: u64,
    restorer: u64,
    mask: u64,
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. The console becomes the controlling terminal.
    if sys(SYS_IOCTL, [0, TIOCSCTTY, 0, 0]) != 0 {
        fail(b"TIOCSCTTY");
    }
    let mut pgrp = 0u32;
    if sys(SYS_IOCTL, [0, TIOCGPGRP, &raw mut pgrp as u64, 0]) != 0 || u64::from(pgrp) != sys(SYS_GETPID, [0; 4]) {
        fail(b"TIOCGPGRP");
    }

    // 2. A line, edited.
    write(b"ttyinit: name? ");
    let mut line = [0u8; 64];
    let n = read(&mut line);
    if is_err(n) || n == 0 || line[n as usize - 1] != b'\n' {
        fail(b"reading a line");
    }
    write(b"ttyinit: line [");
    write(&line[..n as usize - 1]);
    write(b"]\n");

    // 3. ^C: SIGINT to the foreground group ends the read.
    let act = SigAction {
        handler: on_signal as *const () as u64,
        flags: SA_RESTORER,
        restorer: ttyinit_restorer as *const () as u64,
        mask: 0,
    };
    if sys(SYS_RT_SIGACTION, [SIGINT, &raw const act as u64, 0, 8]) != 0 {
        fail(b"rt_sigaction");
    }
    write(b"ttyinit: press ^C> ");
    let r = read(&mut line);
    if r != EINTR.wrapping_neg() || HANDLED.load(Ordering::SeqCst) != SIGINT {
        fail(b"^C");
    }
    write(b"\nttyinit: interrupted by SIGINT\n");

    // 4. Raw mode: single keys, no echo.
    let mut saved = Termios { iflag: 0, oflag: 0, cflag: 0, lflag: 0, line: 0, cc: [0; 19] };
    if sys(SYS_IOCTL, [0, TCGETS, &raw mut saved as u64, 0]) != 0 {
        fail(b"TCGETS");
    }
    let mut raw = saved;
    raw.lflag &= !(ICANON | ECHO);
    raw.cc[VMIN] = 1;
    raw.cc[VTIME] = 0;
    if sys(SYS_IOCTL, [0, TCSETS, &raw const raw as u64, 0]) != 0 {
        fail(b"TCSETS");
    }
    write(b"ttyinit: raw> ");
    let mut keys = [0u8; 3];
    let mut got = 0;
    while got < keys.len() {
        let r = read(&mut keys[got..got + 1]);
        if r != 1 {
            fail(b"reading a key");
        }
        got += 1;
    }
    if sys(SYS_IOCTL, [0, TCSETS, &raw const saved as u64, 0]) != 0 {
        fail(b"restoring the attributes");
    }
    write(b"\nttyinit: raw [");
    write(&keys);
    write(b"]\n");

    // 5. A pseudo-terminal.
    let master = open_rw(b"/dev/ptmx\0");
    let mut n = 0u32;
    let unlock = 0i32;
    if sys(SYS_IOCTL, [master, TIOCGPTN, &raw mut n as u64, 0]) != 0
        || sys(SYS_IOCTL, [master, TIOCSPTLCK, &raw const unlock as u64, 0]) != 0
    {
        fail(b"TIOCGPTN / TIOCSPTLCK");
    }
    let mut path = *b"/dev/pts/0000000000\0";
    let mut digits = [0u8; 10];
    let mut len = 0;
    let mut v = n;
    loop {
        digits[len] = b'0' + (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for i in 0..len {
        path[9 + i] = digits[len - 1 - i];
    }
    path[9 + len] = 0;
    let slave = open_rw(&path);
    sys(SYS_WRITE, [master, b"pty\r".as_ptr() as u64, 4, 0]);
    let mut got = [0u8; 16];
    if read_fd(slave, &mut got) != 4 || &got[..4] != b"pty\n" {
        fail(b"reading the slave");
    }
    if read_fd(master, &mut got) != 5 || &got[..5] != b"pty\r\n" {
        fail(b"the echo on the master");
    }
    write(b"ttyinit: pty [");
    write(&got[..3]);
    write(b"]\nttyinit: ok\n");
    exit(0)
}

// The entry point: the kernel starts the program with `rsp` at `argc`
// (16-byte aligned, psABI §3.4.1); `main` gets a normal call frame.
global_asm!(".globl _start", "_start:", "xor ebp, ebp", "and rsp, -16", "call main", "ud2");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail(b"a panic")
}
