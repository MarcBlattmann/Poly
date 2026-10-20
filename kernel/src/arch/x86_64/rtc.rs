//! The PC's CMOS real-time clock (MC146818-compatible): index port 0x70,
//! data port 0x71. Only read, once, for the wall clock (`time::realtime_ns`);
//! `poly-rtc` decodes the registers.

use super::cpu::InterruptsOff;
use super::port::{inb, outb};
use poly_rtc::{Registers, STATUS_A_UPDATE_IN_PROGRESS, reg};

const INDEX: u16 = 0x70;
const DATA: u16 = 0x71;

fn read_reg(index: u8) -> u8 {
    // SAFETY: selecting a CMOS register (bit 7 clear: NMIs stay enabled)
    // and reading it has no other effect; callers keep interrupts off so
    // nothing selects another register in between.
    unsafe {
        outb(INDEX, index & 0x7f);
        inb(DATA)
    }
}

fn snapshot(century: Option<u8>) -> Registers {
    Registers {
        seconds: read_reg(reg::SECONDS),
        minutes: read_reg(reg::MINUTES),
        hours: read_reg(reg::HOURS),
        day: read_reg(reg::DAY),
        month: read_reg(reg::MONTH),
        year: read_reg(reg::YEAR),
        century: century.map(read_reg),
        status_b: read_reg(reg::STATUS_B),
    }
}

/// The time registers, read outside an update and twice the same (the
/// chip updates them once a second; a read across an update could mix two
/// seconds). `century` is the century register's index, if the platform
/// has one. `None` if the registers never held still.
pub fn read(century: Option<u8>) -> Option<Registers> {
    let _irq = InterruptsOff::new();
    for _ in 0..8 {
        // The update-in-progress flag stays up for under 2 ms.
        let mut waited = 0;
        while read_reg(reg::STATUS_A) & STATUS_A_UPDATE_IN_PROGRESS != 0 && waited < 100_000 {
            core::hint::spin_loop();
            waited += 1;
        }
        let first = snapshot(century);
        if snapshot(century) == first {
            return Some(first);
        }
    }
    None
}
