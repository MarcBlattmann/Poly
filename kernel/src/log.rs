//! Kernel console output: the serial port, mirrored onto the screen
//! terminal (`terminal::mirror`) when there is one.
//!
//! The kernel log ([`kprint!`](crate::kprint), [`kprintln!`](crate::kprintln))
//! and the shell share COM1. While the shell waits for input with its prompt
//! on the screen, a log line would land in the middle of the line being
//! typed. So the shell's serial console marks the prompt as shown
//! ([`prompt_shown`]); the next log write first erases that row
//! (`CR`, `ESC [ K`) and marks the prompt as disturbed, and the shell draws
//! the prompt and its line again below the log output
//! ([`take_prompt_disturbed`]).
//!
//! Whole lines (ADR 0228): the boot tests read their results from this
//! port, so a line must reach it whole. Kernel text and the programs'
//! terminal output (`/dev/ttyS0`, [`write_tty`]) go through one
//! [`poly_logline::Gate`] under the port's lock: kernel text is collected
//! per writer (a thread, or a CPU's interrupt handlers) and goes out as whole
//! lines, and while a program's terminal row is open, kernel lines wait for
//! its row boundary (at most [`WINDOW_NS`] after its last write).
//! [`poll`] (from the boot CPU's timer tick) lets out what waited long
//! enough; [`flush`] lets out everything before the machine stops.

use crate::arch::serial::{COM1, SerialPort};
use crate::sync::SpinLock;
use alloc::vec::Vec;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use poly_logline::{Gate, Port, WINDOW_NS};

/// Writers that can have an unfinished line at a time (a slot each).
const SLOTS: usize = 64;
/// Bytes of an unfinished kernel line one writer can have.
const SLOT_CAP: usize = 512;
/// Bytes of whole kernel lines that can wait for a program's terminal row.
const PENDING: usize = 32 * 1024;

type LogGate = Gate<SLOTS, SLOT_CAP, PENDING>;

/// A serial console: the port (none for a self-test's console), the gate
/// in front of it, and a copy of what went out while a test captures.
struct Console {
    port: Option<SerialPort>,
    gate: LogGate,
    capture: Option<Vec<u8>>,
}

impl Console {
    const fn new(port: Option<SerialPort>) -> Console {
        Console { port, gate: Gate::new(WINDOW_NS), capture: None }
    }
}

/// Where the gate's output goes: the port, and the capture if there is one.
struct Wire<'a> {
    port: &'a mut Option<SerialPort>,
    capture: &'a mut Option<Vec<u8>>,
}

impl Wire<'_> {
    fn record(&mut self, bytes: &[u8]) {
        if let Some(c) = self.capture.as_mut() {
            // Never more than reserved: no allocation under the lock.
            let n = bytes.len().min(c.capacity() - c.len());
            c.extend_from_slice(&bytes[..n]);
        }
    }

    /// A kernel line or a program's output erases a shown shell prompt first.
    fn erase_prompt(port: &mut SerialPort) {
        if PROMPT.compare_exchange(PROMPT_SHOWN, PROMPT_ERASED, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            let _ = port.write_str("\r\x1b[K");
        }
    }
}

impl Port for Wire<'_> {
    fn kernel(&mut self, bytes: &[u8]) {
        if let Some(port) = self.port.as_mut() {
            Wire::erase_prompt(port);
            // `\r\n` line ends, as `SerialPort`'s `fmt::Write` writes them.
            for (n, piece) in bytes.split(|&b| b == b'\n').enumerate() {
                if n > 0 {
                    port.write_raw(b"\r\n");
                }
                port.write_raw(piece);
            }
        }
        self.record(bytes);
    }

    fn tty(&mut self, bytes: &[u8]) {
        if let Some(port) = self.port.as_mut() {
            Wire::erase_prompt(port);
            port.write_raw(bytes);
        }
        self.record(bytes);
    }
}

/// Formatted kernel text into a console's gate.
struct Feed<'a> {
    gate: &'a mut LogGate,
    wire: Wire<'a>,
    writer: u64,
    now: u64,
}

impl Write for Feed<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.gate.kernel(self.writer, s.as_bytes(), self.now, &mut self.wire);
        Ok(())
    }
}

// SAFETY: COM1 is only driven through this lock (and the panic path).
static CONSOLE: SpinLock<Console> = SpinLock::new(Console::new(Some(unsafe { SerialPort::new(COM1) })));

/// The gate of [`CONSOLE`] holds text: [`poll`] has work.
static WAITING: AtomicBool = AtomicBool::new(false);

/// No prompt on the serial console.
const PROMPT_NONE: u8 = 0;
/// The shell's prompt and line are on the current row.
const PROMPT_SHOWN: u8 = 1;
/// A log write erased them; the shell must draw them again.
const PROMPT_ERASED: u8 = 2;

static PROMPT: AtomicU8 = AtomicU8::new(PROMPT_NONE);

pub fn init() {
    if let Some(port) = CONSOLE.lock().port.as_mut() {
        port.init();
    }
}

/// The writer the running code is: this CPU's interrupt handlers (from the
/// top of the number range down), or the running thread (its id's bits).
fn writer() -> u64 {
    if crate::arch::irq::in_interrupt() {
        u64::MAX - crate::arch::percpu::cpu_index() as u64
    } else {
        crate::arch::percpu::this().current()
    }
}

/// Run `f` with a console's gate and wire, under its lock.
fn with_console<R>(console: &SpinLock<Console>, f: impl FnOnce(&mut LogGate, &mut Wire) -> R) -> R {
    let mut c = console.lock();
    let Console { port, gate, capture } = &mut *c;
    let r = f(gate, &mut Wire { port, capture });
    if core::ptr::eq(console, &CONSOLE) {
        WAITING.store(!gate.is_idle(), Ordering::Relaxed);
    }
    r
}

fn print_to(console: &SpinLock<Console>, args: fmt::Arguments, mirror: bool) {
    let writer = writer();
    let now = crate::time::now_ns();
    let mut c = console.lock();
    let Console { port, gate, capture } = &mut *c;
    let _ = Feed { gate: &mut *gate, wire: Wire { port, capture }, writer, now }.write_fmt(args);
    if core::ptr::eq(console, &CONSOLE) {
        WAITING.store(!gate.is_idle(), Ordering::Relaxed);
    }
    if mirror {
        // The screen terminal shows the log too: into a ring its render thread
        // drains, never onto the screen from here (see `terminal`).
        crate::terminal::mirror(args);
    }
}

fn tty_to(console: &SpinLock<Console>, bytes: &[u8]) {
    let now = crate::time::now_ns();
    with_console(console, |gate, wire| gate.tty(bytes, now, wire));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    print_to(&CONSOLE, args, true);
}

/// The kernel log as a [`fmt::Write`], for code that writes to any console.
/// A `write!` or `writeln!` is one log write (whole, like a `kprintln!`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Kprint;

impl fmt::Write for Kprint {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        _print(format_args!("{s}"));
        Ok(())
    }

    fn write_fmt(&mut self, args: fmt::Arguments) -> fmt::Result {
        _print(args);
        Ok(())
    }
}

/// Write the shell's own output (text and escape sequences) to the serial
/// console: like the log, but it never counts as disturbing the prompt,
/// and it goes out at once (the shell's prompt has no line end).
pub fn write_console(s: &str) {
    with_console(&CONSOLE, |_, wire| {
        if let Some(port) = wire.port.as_mut() {
            let _ = port.write_str(s);
        }
    });
}

/// Write whole lines of records (the test runner's `POLY-*` lines, the
/// system call trace) to the serial console: as kernel lines, but not
/// mirrored on the screen.
pub fn write_lines(s: &str) {
    print_to(&CONSOLE, format_args!("{s}"), false);
}

/// Write a program's terminal output (`/dev/ttyS0`, ADR 0046) to the
/// serial port as it is: like a log line, it erases a shown shell prompt
/// first (which the shell then draws again), but it is not mirrored on the
/// screen, which has a terminal of its own.
pub fn write_tty(bytes: &[u8]) {
    tty_to(&CONSOLE, bytes);
}

/// Let out kernel text that waited long enough (see the module
/// documentation). Cheap when nothing waits; from the boot CPU's timer
/// tick, so it never spins for the lock.
pub fn poll() {
    if !WAITING.load(Ordering::Relaxed) {
        return;
    }
    if let Some(mut c) = CONSOLE.try_lock() {
        let now = crate::time::now_ns();
        let Console { port, gate, capture } = &mut *c;
        gate.poll(now, &mut Wire { port, capture });
        WAITING.store(!gate.is_idle(), Ordering::Relaxed);
    }
}

/// Let out all kernel text the gate holds (before the machine stops).
pub fn flush() {
    with_console(&CONSOLE, |gate, wire| gate.flush(wire));
}

/// The shell's prompt and line are on the serial console's current row
/// (true), or not any more (false: the line was submitted). Marking it
/// shown keeps a pending "erased" state, so a redraw is not lost.
pub fn prompt_shown(shown: bool) {
    if shown {
        let _ = PROMPT.compare_exchange(PROMPT_NONE, PROMPT_SHOWN, Ordering::Relaxed, Ordering::Relaxed);
    } else {
        PROMPT.store(PROMPT_NONE, Ordering::Relaxed);
    }
}

/// Whether log output erased the prompt since it was last drawn. Reading it
/// marks the prompt as shown again: the caller draws it.
pub fn take_prompt_disturbed() -> bool {
    PROMPT.compare_exchange(PROMPT_ERASED, PROMPT_SHOWN, Ordering::Relaxed, Ordering::Relaxed).is_ok()
}

/// Whether log output erased the prompt (without changing anything).
pub fn prompt_disturbed() -> bool {
    PROMPT.load(Ordering::Relaxed) == PROMPT_ERASED
}

/// Write to the console without waiting for the lock. Only for the panic
/// path, where the lock may be held by the code that panicked: if the lock
/// is free, what the gate holds goes out first.
pub fn emergency_print(args: fmt::Arguments) {
    if let Some(mut c) = CONSOLE.try_lock() {
        let Console { port, gate, capture } = &mut *c;
        gate.flush(&mut Wire { port, capture });
        if let Some(p) = port.as_mut() {
            let _ = p.write_fmt(args);
        }
        return;
    }
    // SAFETY: we are about to stop the machine; interleaving with a stuck
    // writer is acceptable.
    let mut port = unsafe { SerialPort::new(COM1) };
    let _ = port.write_fmt(args);
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::log::_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => { $crate::log::_print(format_args!("{}\n", format_args!($($arg)*))) };
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use core::sync::atomic::AtomicUsize;

/// A console without a port, for the self-test: the same code as the
/// kernel log's, but what goes out is only captured (the self-test runs on
/// every boot, also while a program draws on COM1).
static TEST_CONSOLE: SpinLock<Console> = SpinLock::new(Console::new(None));

const NOISE_THREADS: usize = 3;
const NOISE_LINES: usize = 64;
const RESULT_ROWS: usize = 32;
static NOISE_DONE: AtomicUsize = AtomicUsize::new(0);

fn test_print(args: fmt::Arguments) {
    print_to(&TEST_CONSOLE, args, false);
}

/// Another CPU's kernel log: whole lines, as fast as it can.
fn noise(id: usize) -> usize {
    for i in 0..NOISE_LINES {
        test_print(format_args!("logline-noise {id} {i} the quick brown fox jumps over the lazy dog\n"));
        if i % 8 == 0 {
            crate::thread::yield_now();
        }
    }
    NOISE_DONE.fetch_add(1, Ordering::SeqCst);
    0
}

fn result_line(i: usize) -> alloc::string::String {
    alloc::format!("logline-result {i} {:08x} end", (i as u32).wrapping_mul(2_654_435_761))
}

fn tty_line(i: usize) -> alloc::string::String {
    alloc::format!("logline-tty {i} {:016x}{:016x} end", (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15), !(i as u64))
}

kernel_test! {
    /// ADR 0228: while other CPUs log, a result line written in pieces
    /// (`kprint!` by `kprint!`, and a program's terminal row two bytes per
    /// write, as the Chrome launcher wrote its digest) reaches the port
    /// whole, and so does every other CPU's line.
    fn result_lines_stay_whole_while_other_cpus_log() -> TestResult {
        let _pin = crate::thread::pin_current();
        let here = crate::thread::this_cpu();
        let others: Vec<usize> = crate::thread::online_cpus().iter().filter(|&c| c != here).collect();
        TEST_CONSOLE.lock().capture = Some(Vec::with_capacity(256 * 1024));
        NOISE_DONE.store(0, Ordering::SeqCst);
        let mut threads = Vec::new();
        for n in 0..NOISE_THREADS {
            let cpu = others.get(n % others.len().max(1)).copied().unwrap_or(here);
            let set = crate::thread::CpuSet::one(cpu);
            match crate::thread::spawn_on("logline-noise", noise, n, set) {
                Ok(t) => threads.push(t),
                Err(_) => return Err("spawn failed"),
            }
        }
        let mut row = 0;
        while row < RESULT_ROWS || NOISE_DONE.load(Ordering::SeqCst) < NOISE_THREADS {
            if row < RESULT_ROWS {
                let line = result_line(row);
                let (a, b) = line.split_at(line.len() / 2);
                test_print(format_args!("{a}"));
                test_print(format_args!("{b}"));
                test_print(format_args!("\n"));
                let tty = alloc::format!("{}\n", tty_line(row));
                for piece in tty.as_bytes().chunks(2) {
                    tty_to(&TEST_CONSOLE, piece);
                }
                row += 1;
            } else {
                crate::thread::yield_now();
            }
        }
        for t in threads {
            let _ = crate::thread::join(t);
        }
        with_console(&TEST_CONSOLE, |gate, wire| gate.flush(wire));
        let out = TEST_CONSOLE.lock().capture.take().unwrap_or_default();
        if out.len() == out.capacity() {
            return Err("the capture filled up");
        }
        let text = alloc::string::String::from_utf8_lossy(&out);
        let (mut results, mut ttys, mut noise) = (0, 0, 0);
        for line in text.split('\n').filter(|l| !l.is_empty()) {
            if line.matches("logline-").count() != 1 || !line.starts_with("logline-") {
                crate::kprintln!("log: torn line in the self-test's capture: {line:?}");
                return Err("a line was torn");
            }
            if line.starts_with("logline-result ") {
                if line != result_line(results) {
                    return Err("a result line is not the one written, or out of order");
                }
                results += 1;
            } else if line.starts_with("logline-tty ") {
                if line != tty_line(ttys) {
                    return Err("a terminal row is not the one written, or out of order");
                }
                ttys += 1;
            } else if line.ends_with(" the quick brown fox jumps over the lazy dog") {
                noise += 1;
            } else {
                return Err("a noise line was torn");
            }
        }
        if results != RESULT_ROWS || ttys != RESULT_ROWS || noise != NOISE_THREADS * NOISE_LINES {
            return Err("lines are missing");
        }
        Ok(())
    }
}
