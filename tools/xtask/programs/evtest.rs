//! A static Linux program without any C library that reads input devices
//! through the event interface, `/dev/input/event*` (roadmap Phase 6, ADR
//! 0103), for the kernel's boot tests: `cargo xtask` boots it as the first
//! process (`poly.init=/bin/evtest`) and sends keys, motion and clicks
//! through QEMU's QMP `input-send-event` (`tools/xtask/src/input.rs`).
//!
//! 1. Opens every `/dev/input/event<n>` there is and describes each with
//!    the ioctls a compositor or libinput starts with: `EVIOCGVERSION`,
//!    `EVIOCGNAME`, `EVIOCGID`, `EVIOCGBIT` (types, keys, relative and
//!    absolute axes), `EVIOCGABS` for an absolute X axis, and switches
//!    each to the monotonic clock (`EVIOCSCLOCKID`).
//! 2. `EVIOCGRAB` on the first device: a second open cannot grab it too
//!    (`EBUSY`); then lets go.
//! 3. On a keyboard: writes `EV_LED` Caps Lock on and `SYN_REPORT` to it,
//!    reads the LEDs back (`EVIOCGLED`) and turns Caps Lock off again.
//! 4. Prints `evtest: ready`, then `poll`s every device and prints each
//!    event it reads (`evtest: event<n> type <t> code <c> value <v>`; the
//!    `SYN_REPORT`s are left out) until a Q key press arrives, or 40
//!    seconds pass.
//!
//! It prints `evtest: done` and exits 0, or what failed and exits 1. Every
//! call is a raw `syscall` (numbers from `asm/unistd_64.h`); the ioctl
//! numbers are computed as `asm-generic/ioctl.h` defines `_IOC`, with the
//! `EVIOC*` numbers, `struct input_event`, `struct input_id` and `struct
//! input_absinfo` of `linux/input.h`, and the event codes of
//! `linux/input-event-codes.h`. Written for Poly from those headers and
//! the man pages.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};

const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_CLOSE: u64 = 3;
const SYS_POLL: u64 = 7;
const SYS_IOCTL: u64 = 16;
const SYS_OPENAT: u64 = 257;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_EXIT_GROUP: u64 = 231;

const AT_FDCWD: u64 = -100i64 as u64;
const O_RDWR: u64 = 0o2;
const O_NONBLOCK: u64 = 0o4000;
const ENOENT: u64 = 2;
const EBUSY: u64 = 16;
const POLLIN: i16 = 1;
const CLOCK_MONOTONIC: u64 = 1;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const EV_LED: u16 = 0x11;
const KEY_Q: u16 = 16;
const KEY_A: u16 = 30;
const LED_CAPSL: u16 = 1;

/// The most devices looked at.
const MAX_DEVICES: usize = 16;

const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;

/// `_IOC(dir, 'E', nr, size)`.
const fn eioc(dir: u64, nr: u64, size: u64) -> u64 {
    dir << 30 | size << 16 | (b'E' as u64) << 8 | nr
}

const EVIOCGVERSION: u64 = eioc(IOC_READ, 0x01, 4);
const EVIOCGID: u64 = eioc(IOC_READ, 0x02, 8);
const EVIOCGRAB: u64 = eioc(IOC_WRITE, 0x90, 4);
const EVIOCSCLOCKID: u64 = eioc(IOC_WRITE, 0xa0, 4);

const fn eviocgname(len: u64) -> u64 {
    eioc(IOC_READ, 0x06, len)
}

const fn eviocgled(len: u64) -> u64 {
    eioc(IOC_READ, 0x19, len)
}

const fn eviocgbit(ev: u16, len: u64) -> u64 {
    eioc(IOC_READ, 0x20 + ev as u64, len)
}

const fn eviocgabs(abs: u64) -> u64 {
    eioc(IOC_READ, 0x40 + abs, 24)
}

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

fn is_err(ret: u64) -> bool {
    ret > u64::MAX - 4095
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
    write(b"evtest: ");
    write(what);
    write(b" failed\n");
    exit(1)
}

/// A line of output, built up in a buffer.
struct Line {
    buf: [u8; 256],
    len: usize,
}

impl Line {
    fn new() -> Line {
        let mut l = Line { buf: [0; 256], len: 0 };
        l.s(b"evtest: ");
        l
    }

    fn s(&mut self, s: &[u8]) -> &mut Line {
        for &b in s {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        self
    }

    fn dec(&mut self, v: i64) -> &mut Line {
        if v < 0 {
            self.s(b"-");
        }
        let mut digits = [0u8; 20];
        let mut n = v.unsigned_abs();
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        self.s(&digits[i..])
    }

    fn hex(&mut self, v: u64) -> &mut Line {
        self.s(b"0x");
        let mut started = false;
        for shift in (0..16).rev() {
            let d = ((v >> (shift * 4)) & 0xf) as u8;
            if d != 0 || started || shift == 0 {
                started = true;
                self.s(&[if d < 10 { b'0' + d } else { b'a' + d - 10 }]);
            }
        }
        self
    }

    fn end(&mut self) {
        self.s(b"\n");
        write(&self.buf[..self.len]);
    }
}

fn ioctl(fd: u64, request: u64, arg: u64) -> u64 {
    sys(SYS_IOCTL, [fd, request, arg, 0])
}

/// `path` ends in a NUL.
fn open(path: &[u8]) -> u64 {
    sys(SYS_OPENAT, [AT_FDCWD, path.as_ptr() as u64, O_RDWR | O_NONBLOCK, 0])
}

fn bit(bits: &[u8], n: u16) -> bool {
    bits.get(usize::from(n / 8)).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

fn now_ns() -> u64 {
    let mut ts = [0i64; 2];
    sys(SYS_CLOCK_GETTIME, [CLOCK_MONOTONIC, ts.as_mut_ptr() as u64, 0, 0]);
    ts[0] as u64 * 1_000_000_000 + ts[1] as u64
}

/// `/dev/input/event<n>`, NUL-terminated.
fn device_path(n: usize, buf: &mut [u8; 32]) -> &[u8] {
    let prefix = b"/dev/input/event";
    buf[..prefix.len()].copy_from_slice(prefix);
    let mut len = prefix.len();
    if n >= 10 {
        buf[len] = b'0' + (n / 10) as u8;
        len += 1;
    }
    buf[len] = b'0' + (n % 10) as u8;
    buf[len + 1] = 0;
    &buf[..len + 2]
}

/// Describe the device on `fd`; returns whether it has the letter A (a
/// keyboard).
fn describe(n: usize, fd: u64) -> bool {
    let mut version = 0u32;
    if ioctl(fd, EVIOCGVERSION, &raw mut version as u64) != 0 || version != 0x01_0001 {
        fail(b"EVIOCGVERSION");
    }
    let mut name = [0u8; 256];
    let len = ioctl(fd, eviocgname(256), name.as_mut_ptr() as u64);
    if is_err(len) || len == 0 || name[len as usize - 1] != 0 {
        fail(b"EVIOCGNAME");
    }
    let mut id = [0u16; 4];
    if ioctl(fd, EVIOCGID, id.as_mut_ptr() as u64) != 0 {
        fail(b"EVIOCGID");
    }
    let mut types = [0u8; 8];
    let mut keys = [0u8; 96];
    let mut rel = [0u8; 8];
    let mut abs = [0u8; 8];
    for (ev, buf) in [(0, &mut types[..]), (EV_KEY, &mut keys[..]), (EV_REL, &mut rel[..]), (EV_ABS, &mut abs[..])] {
        if is_err(ioctl(fd, eviocgbit(ev, buf.len() as u64), buf.as_mut_ptr() as u64)) {
            fail(b"EVIOCGBIT");
        }
    }
    let nkeys: u32 = keys.iter().map(|b| b.count_ones()).sum();
    let mut line = Line::new();
    line.s(b"event").dec(n as i64).s(b" \"").s(&name[..len as usize - 1]).s(b"\" bus ").hex(u64::from(id[0]));
    line.s(b" types ").hex(u64::from_le_bytes(types)).s(b" keys ").dec(i64::from(nkeys));
    line.s(b" rel ").hex(u64::from_le_bytes(rel)).s(b" abs ").hex(u64::from_le_bytes(abs));
    if bit(&abs, 0) {
        let mut info = [0i32; 6];
        if ioctl(fd, eviocgabs(0), info.as_mut_ptr() as u64) != 0 {
            fail(b"EVIOCGABS");
        }
        line.s(b" x ").dec(i64::from(info[1])).s(b"..").dec(i64::from(info[2]));
    }
    line.end();
    let monotonic = CLOCK_MONOTONIC as i32;
    if ioctl(fd, EVIOCSCLOCKID, &raw const monotonic as u64) != 0 {
        fail(b"EVIOCSCLOCKID");
    }
    bit(&keys, KEY_A) && bit(&types, EV_LED)
}

/// `struct input_event` bytes for type, code, value (the time is ignored
/// on writes).
fn event(kind: u16, code: u16, value: i32) -> [u8; 24] {
    let mut e = [0u8; 24];
    e[16..18].copy_from_slice(&kind.to_le_bytes());
    e[18..20].copy_from_slice(&code.to_le_bytes());
    e[20..24].copy_from_slice(&value.to_le_bytes());
    e
}

/// Light (or not) Caps Lock by writing to the device, and check it with
/// `EVIOCGLED`.
fn set_caps_led(fd: u64, on: bool) {
    let mut both = [0u8; 48];
    both[..24].copy_from_slice(&event(EV_LED, LED_CAPSL, i32::from(on)));
    both[24..].copy_from_slice(&event(EV_SYN, 0, 0));
    if sys(SYS_WRITE, [fd, both.as_ptr() as u64, 48, 0]) != 48 {
        fail(b"writing EV_LED");
    }
    let mut leds = [0u8; 8];
    if is_err(ioctl(fd, eviocgled(8), leds.as_mut_ptr() as u64)) || bit(&leds, LED_CAPSL) != on {
        fail(b"EVIOCGLED after writing EV_LED");
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let mut fds = [PollFd { fd: -1, events: POLLIN, revents: 0 }; MAX_DEVICES];
    let mut count = 0;
    let mut keyboard = None;
    for (n, slot) in fds.iter_mut().enumerate() {
        let mut path = [0u8; 32];
        let fd = open(device_path(n, &mut path));
        if fd == ENOENT.wrapping_neg() {
            break;
        }
        if is_err(fd) {
            fail(b"opening an event device");
        }
        slot.fd = fd as i32;
        count += 1;
        if describe(n, fd) && keyboard.is_none() {
            keyboard = Some(fd);
        }
    }
    if count == 0 {
        fail(b"finding /dev/input/event0");
    }

    // A grab is exclusive.
    let first = fds[0].fd as u64;
    let mut path = [0u8; 32];
    let second = open(device_path(0, &mut path));
    if is_err(second) || ioctl(first, EVIOCGRAB, 1) != 0 || ioctl(second, EVIOCGRAB, 1) != EBUSY.wrapping_neg() {
        fail(b"EVIOCGRAB");
    }
    if ioctl(first, EVIOCGRAB, 0) != 0 {
        fail(b"releasing the grab");
    }
    sys(SYS_CLOSE, [second, 0, 0, 0]);
    write(b"evtest: grab ok\n");

    if let Some(fd) = keyboard {
        set_caps_led(fd, true);
        set_caps_led(fd, false);
        write(b"evtest: led ok\n");
    }

    // Throw away what came in so far (the LED events).
    let mut buf = [0u8; 24 * 64];
    for p in &fds[..count] {
        while !is_err(sys(SYS_READ, [p.fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, 0])) {}
    }

    write(b"evtest: ready\n");
    let deadline = now_ns() + 40_000_000_000;
    loop {
        if now_ns() > deadline {
            fail(b"waiting for Q (timed out)");
        }
        let ready = sys(SYS_POLL, [fds.as_mut_ptr() as u64, count as u64, 1000, 0]);
        if is_err(ready) {
            fail(b"poll");
        }
        for (n, p) in fds[..count].iter_mut().enumerate() {
            if p.revents & POLLIN == 0 {
                continue;
            }
            p.revents = 0;
            let got = sys(SYS_READ, [p.fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, 0]);
            if is_err(got) || got % 24 != 0 {
                fail(b"reading events");
            }
            let mut quit = false;
            for e in buf[..got as usize].chunks_exact(24) {
                let kind = u16::from_le_bytes([e[16], e[17]]);
                let code = u16::from_le_bytes([e[18], e[19]]);
                let value = i32::from_le_bytes([e[20], e[21], e[22], e[23]]);
                if kind == EV_SYN {
                    continue;
                }
                let mut line = Line::new();
                line.s(b"event").dec(n as i64).s(b" type ").dec(i64::from(kind));
                line.s(b" code ").dec(i64::from(code)).s(b" value ").dec(i64::from(value)).end();
                quit |= kind == EV_KEY && code == KEY_Q && value == 1;
            }
            if quit {
                write(b"evtest: done\n");
                exit(0);
            }
        }
    }
}

global_asm!(".globl _start", "_start:", "xor ebp, ebp", "and rsp, -16", "call main", "ud2");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail(b"a panic")
}
