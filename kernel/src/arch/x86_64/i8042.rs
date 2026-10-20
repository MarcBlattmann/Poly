//! The i8042 PS/2 controller's I/O ports, for `poly_input::i8042`.

use super::port::{inb, outb};
use poly_input::i8042::{DATA_PORT, Io, STATUS_PORT};

/// The controller at its fixed ISA ports (0x60 data, 0x64 status /
/// command). Only the keyboard driver uses these ports.
#[derive(Clone, Copy, Debug)]
pub struct I8042Ports;

impl Io for I8042Ports {
    fn read_status(&self) -> u8 {
        // SAFETY: reading the status register has no side effects.
        unsafe { inb(STATUS_PORT) }
    }

    fn read_data(&self) -> u8 {
        // SAFETY: takes the byte out of the controller's output buffer;
        // only the keyboard driver reads this port.
        unsafe { inb(DATA_PORT) }
    }

    fn write_command(&self, command: u8) {
        // SAFETY: a controller command; the keyboard driver is the only
        // writer and `poly_input::i8042` sends only documented commands.
        unsafe { outb(STATUS_PORT, command) }
    }

    fn write_data(&self, byte: u8) {
        // SAFETY: a command parameter or a byte for the keyboard, from the
        // keyboard driver only.
        unsafe { outb(DATA_PORT, byte) }
    }

    fn now_ns(&self) -> u64 {
        crate::time::now_ns()
    }
}
