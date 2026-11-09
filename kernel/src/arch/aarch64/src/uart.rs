//! The serial console: QEMU `virt`'s PL011 (Arm DDI 0183), polled, no
//! interrupts. [`Console`] is stateless (the base address is a static), so it
//! needs no lock and the panic and exception paths can print whatever the
//! interrupted code was doing.

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_aarch64_stub::pl011::*;

/// Where QEMU's `virt` board puts the first PL011 (QEMU documentation, "Arm
/// Virt": device locations may change between versions, so a guest should
/// read them from the devicetree, which [`crate::boot`] does; this is the
/// fallback when the blob has none).
pub const QEMU_VIRT_PL011: usize = 0x0900_0000;

/// The UART clock of QEMU's `virt` (its `apb-pclk`, 24 MHz in the
/// devicetree); QEMU ignores the divisors, a real PL011 would not.
const UART_CLOCK_HZ: u32 = 24_000_000;
const BAUD: u32 = 115_200;

static BASE: AtomicUsize = AtomicUsize::new(QEMU_VIRT_PL011);

fn read(offset: usize) -> u32 {
    // SAFETY: `BASE` is a PL011's register block (set by `init`), which is
    // mapped (MMU off: all of physical memory; MMU on: the device gigabyte),
    // and registers are 32 bits wide and aligned.
    unsafe { core::ptr::read_volatile((BASE.load(Ordering::Relaxed) + offset) as *const u32) }
}

fn write(offset: usize, value: u32) {
    // SAFETY: as for `read`.
    unsafe { core::ptr::write_volatile((BASE.load(Ordering::Relaxed) + offset) as *mut u32, value) }
}

/// Program the PL011 at `base`: 115200 8N1, FIFOs on, interrupts off, in the
/// order the TRM gives (disable, flush, baud rate, line control, enable).
pub fn init(base: usize) {
    BASE.store(base, Ordering::Relaxed);
    write(UARTCR, 0);
    while read(UARTFR) & FR_BUSY != 0 {}
    write(UARTLCR_H, 0); // FIFOs off flushes them
    if let Some((integer, fraction)) = baud_divisors(UART_CLOCK_HZ, BAUD) {
        write(UARTIBRD, u32::from(integer));
        write(UARTFBRD, u32::from(fraction));
    }
    write(UARTLCR_H, LCR_H_WLEN_8 | LCR_H_FEN);
    write(UARTIMSC, 0);
    write(UARTICR, ICR_ALL);
    write(UARTCR, CR_UARTEN | CR_TXE | CR_RXE);
}

/// The base address in use.
pub fn base() -> usize {
    BASE.load(Ordering::Relaxed)
}

fn put_byte(byte: u8) {
    while read(UARTFR) & FR_TXFF != 0 {}
    write(UARTDR, u32::from(byte));
}

/// The console as a `fmt::Write`.
#[derive(Debug)]
pub struct Console;

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        s.bytes().for_each(put_byte);
        Ok(())
    }
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = write!($crate::uart::Console, $($arg)*);
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = writeln!($crate::uart::Console, $($arg)*);
    }};
}
