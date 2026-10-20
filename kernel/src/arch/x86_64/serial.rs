//! 16550 UART driver (COM1). The first and most reliable debug channel: it
//! works in QEMU, on most real PCs and in CI.

use super::port::{inb, outb};
use core::fmt;

#[derive(Debug)]
pub struct SerialPort {
    base: u16,
}

pub const COM1: u16 = 0x3f8;

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

    fn write_byte(&mut self, byte: u8) {
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
