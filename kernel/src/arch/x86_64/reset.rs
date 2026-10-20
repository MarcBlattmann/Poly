//! Resetting the machine, in order of preference: the ACPI reset register,
//! the 8042 keyboard controller's reset line, and a triple fault. The
//! caller (`power::reboot`) tries them one after the other.

use super::gdt::DescriptorTablePointer;
use super::mmio::MmioRegion;
use super::port::{inb, outb};
use core::arch::asm;
use poly_acpi::{AddressSpace, ResetRegister};
use poly_irq::Mmio;

/// The 8042's status (read) and command (write) port, and its "pulse output
/// port bit 0" command, which pulses the CPU reset line on PCs (IBM PC AT
/// Technical Reference, keyboard controller commands).
const I8042_STATUS_COMMAND: u16 = 0x64;
const I8042_PULSE_RESET: u8 = 0xfe;
/// Status bit 1: the controller has not read the last command yet.
const I8042_INPUT_FULL: u8 = 1 << 1;

/// Write the FADT's reset value to its reset register (ACPI 6.5
/// §4.8.3.6). False if the register lives somewhere this cannot write
/// (PCI configuration space, an I/O port above 0xffff, unmappable memory);
/// otherwise the machine should reset shortly after this returns.
pub fn acpi(reset: &ResetRegister) -> bool {
    let r = reset.register;
    match r.space {
        AddressSpace::SystemIo => {
            let Ok(port) = u16::try_from(r.address) else { return false };
            // SAFETY: the firmware names this port as the reset register;
            // writing the reset value to it is what it is for.
            unsafe { outb(port, reset.value) };
            true
        }
        AddressSpace::SystemMemory => match MmioRegion::map(r.address, 1) {
            Some(reg) => {
                reg.write8(0, reset.value);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Ask the 8042 keyboard controller to pulse the reset line. Returns if
/// nothing happened (no 8042, or it ignored the command).
pub fn keyboard_controller() {
    // SAFETY: the 8042's status and command port; reading the status has
    // no side effects, and the only command written is the reset pulse.
    unsafe {
        for _ in 0..100_000 {
            if inb(I8042_STATUS_COMMAND) & I8042_INPUT_FULL == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        outb(I8042_STATUS_COMMAND, I8042_PULSE_RESET);
    }
}

/// Reset by triple fault: with an empty IDT, the breakpoint cannot be
/// delivered, neither can the #GP that causes nor the double fault after
/// it, and a CPU that cannot deliver a double fault shuts down, which the
/// chipset turns into a reset (SDM Vol. 3A §6.15, interrupt 8).
pub fn triple_fault() -> ! {
    let empty = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: deliberately unrecoverable: interrupts are masked first so
    // nothing runs between loading the empty IDT and the fault.
    unsafe { asm!("cli", "lidt [{}]", "int3", in(reg) &empty, options(noreturn, nostack)) }
}
