//! Kernel console output (serial for now).

use crate::arch::serial::{COM1, SerialPort};
use crate::sync::SpinLock;
use core::fmt::{self, Write};

// SAFETY: COM1 is only driven through this lock (and the panic path).
static CONSOLE: SpinLock<SerialPort> = SpinLock::new(unsafe { SerialPort::new(COM1) });

pub fn init() {
    CONSOLE.lock().init();
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let _ = CONSOLE.lock().write_fmt(args);
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
