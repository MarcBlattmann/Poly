//! 16550 UART driver (COM1). The first and most reliable debug channel: it
//! works in QEMU, on most real PCs and in CI.
//!
//! [`SerialPort`] transmits (the kernel log); [`SerialReceiver`] receives
//! (the shell's input). They use disjoint registers once the port is set up:
//! the transmitter writes THR and polls LSR for THRE, the receiver reads RBR
//! and LSR's data-ready bit and owns IER and FCR. Reading LSR clears only
//! its error bits, which neither side uses, so the two need no common lock.
//! Register offsets and bits from the PC16550D datasheet (National
//! Semiconductor, 1995), table II.

use super::port::{inb, outb};
use core::fmt;

#[derive(Debug)]
pub struct SerialPort {
    base: u16,
}

pub const COM1: u16 = 0x3f8;

/// The ISA IRQ of COM1 on PCs.
pub const COM1_IRQ: u8 = 4;

/// Register offsets from the base port.
const RBR: u16 = 0; // receiver buffer (read, DLAB 0)
const IER: u16 = 1; // interrupt enable (DLAB 0)
const IIR_FCR: u16 = 2; // interrupt identification (read) / FIFO control (write)
const LSR: u16 = 5; // line status

/// IER bit 0: interrupt when received data is available (or the FIFO's
/// character timeout expires).
const IER_RECEIVED_DATA: u8 = 1 << 0;
/// FCR: FIFOs enabled (bit 0), receive trigger level 1 byte (bits 7:6 = 0),
/// without resetting either FIFO (bits 1 and 2 clear).
const FCR_FIFO_TRIGGER_1: u8 = 0x01;
/// LSR bit 0: a received byte is waiting in RBR (or the FIFO).
const LSR_DATA_READY: u8 = 1 << 0;

impl SerialPort {
    /// # Safety
    /// `base` must be a 16550-compatible UART, and only one `SerialPort` may
    /// drive it at a time (except the panic path, which steals it).
    pub const unsafe fn new(base: u16) -> SerialPort {
        SerialPort { base }
    }

    /// 115200 baud, 8N1, FIFOs on, interrupts off.
    pub fn init(&mut self) {
        let b = self.base;
        // SAFETY: standard 16550 programming sequence on a port we own.
        unsafe {
            outb(b + 1, 0x00);
            outb(b + 3, 0x80);
            outb(b, 0x01);
            outb(b + 1, 0x00);
            outb(b + 3, 0x03);
            outb(b + 2, 0xc7);
            outb(b + 4, 0x0b);
        }
    }

    /// Send `bytes` as they are (no CR added before LF): a terminal's
    /// output, already through its line discipline.
    pub fn write_raw(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_byte(b);
        }
    }

    fn write_byte(&mut self, byte: u8) {
        // LSR bit 5: the transmit holding register (or FIFO) is empty.
        // SAFETY: polling the line status register of a UART we own.
        unsafe {
            while inb(self.base + 5) & 0x20 == 0 {
                core::hint::spin_loop();
            }
            outb(self.base, byte);
        }
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }
}

/// The receive side of a 16550 UART (see the module documentation).
#[derive(Debug)]
pub struct SerialReceiver {
    base: u16,
}

impl SerialReceiver {
    /// # Safety
    /// `base` must be a 16550-compatible UART already set up by
    /// [`SerialPort::init`], and only one `SerialReceiver` may exist for it.
    pub const unsafe fn new(base: u16) -> SerialReceiver {
        SerialReceiver { base }
    }

    /// Raise an interrupt as soon as a byte arrives. The UART drives its
    /// IRQ line only with MCR.OUT2 set, which [`SerialPort::init`] does.
    pub fn enable_interrupt(&mut self) {
        // SAFETY: FCR and IER of a UART this receiver owns; neither write
        // touches the transmitter (the FIFOs are not reset).
        unsafe {
            outb(self.base + IIR_FCR, FCR_FIFO_TRIGGER_1);
            outb(self.base + IER, IER_RECEIVED_DATA);
        }
    }

    /// No more receive interrupts (flow control, ADR 0123): what arrives
    /// stays in the receive FIFO, which is kept. Once it is full QEMU's
    /// UART takes no more from its character device, which holds the rest
    /// back; a real one without hardware flow control would overrun.
    pub fn disable_interrupt(&mut self) {
        // SAFETY: IER of a UART this receiver owns; the transmitter does
        // not use its interrupt (it polls LSR).
        unsafe { outb(self.base + IER, 0) };
    }

    /// The next received byte, if one is waiting. Reading the last one
    /// clears the receive interrupt.
    pub fn read(&mut self) -> Option<u8> {
        // SAFETY: LSR and RBR of a UART this receiver owns; reading them
        // only consumes received data (and LSR's error bits).
        unsafe { (inb(self.base + LSR) & LSR_DATA_READY != 0).then(|| inb(self.base + RBR)) }
    }
}
