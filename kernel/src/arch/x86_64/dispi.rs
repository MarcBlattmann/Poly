//! The Bochs VBE ("DISPI") display registers through their I/O ports, for
//! VGA-class devices without the memory-mapped register block (QEMU's
//! standard VGA with `mmio=off`). Everything else about the interface is in
//! `poly-bochs-vbe`; `display.rs` prefers the MMIO BAR and only falls back
//! to this.

use super::port::{inw, outw};
use poly_bochs_vbe::{PORT_DATA, PORT_INDEX, Registers};

/// The index port selects a register, the data port reads or writes it.
/// The pair is one piece of shared state, so only the display driver uses
/// it, under its lock.
#[derive(Clone, Copy, Debug)]
pub struct DispiPorts;

impl Registers for DispiPorts {
    fn read(&self, index: u16) -> u16 {
        // SAFETY: 0x1ce/0x1cf belong to the VGA device the display driver
        // found; selecting and reading a DISPI register has no other effect.
        unsafe {
            outw(PORT_INDEX, index);
            inw(PORT_DATA)
        }
    }

    fn write(&self, index: u16, value: u16) {
        // SAFETY: as for `read`; what the write does to the display is the
        // caller's business.
        unsafe {
            outw(PORT_INDEX, index);
            outw(PORT_DATA, value);
        }
    }
}
