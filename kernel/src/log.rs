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

use crate::arch::serial::{COM1, SerialPort};
use crate::sync::SpinLock;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU8, Ordering};

// SAFETY: COM1 is only driven through this lock (and the panic path).
static CONSOLE: SpinLock<SerialPort> = SpinLock::new(unsafe { SerialPort::new(COM1) });

/// No prompt on the serial console.
const PROMPT_NONE: u8 = 0;
/// The shell's prompt and line are on the current row.
const PROMPT_SHOWN: u8 = 1;
/// A log write erased them; the shell must draw them again.
const PROMPT_ERASED: u8 = 2;

static PROMPT: AtomicU8 = AtomicU8::new(PROMPT_NONE);

pub fn init() {
    CONSOLE.lock().init();
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut port = CONSOLE.lock();
    if PROMPT.compare_exchange(PROMPT_SHOWN, PROMPT_ERASED, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        let _ = port.write_str("\r\x1b[K");
    }
    let _ = port.write_fmt(args);
    // The screen terminal shows the log too: into a ring its render thread
    // drains, never onto the screen from here (see `terminal`).
    crate::terminal::mirror(args);
}

/// The kernel log as a [`fmt::Write`], for code that writes to any console.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kprint;

impl fmt::Write for Kprint {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        _print(format_args!("{s}"));
        Ok(())
    }
}

/// Write the shell's own output (text and escape sequences) to the serial
/// console: like the log, but it never counts as disturbing the prompt.
pub fn write_console(s: &str) {
    let _ = CONSOLE.lock().write_str(s);
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

/// Write to the console without taking the lock. Only for the panic path,
/// where the lock may be held by the code that panicked.
pub fn emergency_print(args: fmt::Arguments) {
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
