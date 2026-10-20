//! The legacy 8259A interrupt controller pair (Intel 8259A Programmable
//! Interrupt Controller datasheet).
//!
//! With the APICs in charge, the 8259s are only remapped and masked: after
//! reset their vectors overlap the CPU exceptions (IRQ 0 to 7 on vectors 8
//! to 15), so a stray interrupt from them would look like a double fault.
//! Even fully masked, the master can still raise a spurious IRQ 7 (and the
//! slave IRQ 15), which [`Pic8259::is_spurious`] recognises.
//!
//! Initialisation is a fixed sequence of four initialisation command words
//! per chip ("Initialization Command Words"): ICW1 on the command port starts
//! it, then ICW2 (vector base), ICW3 (cascade wiring) and ICW4 (8086 mode) on
//! the data port. ICW1 clears the interrupt mask, so masking comes after.

use crate::PortIo;

pub mod port {
    pub const MASTER_COMMAND: u16 = 0x20;
    pub const MASTER_DATA: u16 = 0x21;
    pub const SLAVE_COMMAND: u16 = 0xa0;
    pub const SLAVE_DATA: u16 = 0xa1;
    /// An unused port (the POST code port); a write to it takes about a
    /// microsecond, the settling time old 8259s needed between commands.
    pub const DELAY: u16 = 0x80;
}

pub mod command {
    /// ICW1: bit 4 marks an initialisation word, bit 0 says ICW4 follows.
    /// Bits 1 (single) and 3 (level-triggered) stay 0: cascaded, edge.
    pub const ICW1_INIT: u8 = 0x10;
    pub const ICW1_ICW4: u8 = 0x01;
    /// ICW4 bit 0: 8086/8088 mode (ICW2 bits 7:3 give the vector base).
    pub const ICW4_8086: u8 = 0x01;
    /// OCW2: non-specific end of interrupt.
    pub const OCW2_EOI: u8 = 0x20;
    /// OCW3: next command-port read returns the IRR / the ISR.
    pub const OCW3_READ_IRR: u8 = 0x0a;
    pub const OCW3_READ_ISR: u8 = 0x0b;
}

/// The master input the slave is wired to.
pub const CASCADE_IRQ: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicError {
    /// A vector base must be a multiple of 8 (ICW2 bits 2:0 are ignored in
    /// 8086 mode) and above the 32 exception vectors, and the two bases must
    /// differ.
    InvalidBase,
}

/// The master / slave 8259 pair of a PC.
#[derive(Debug)]
pub struct Pic8259<P> {
    ports: P,
}

impl<P: PortIo> Pic8259<P> {
    pub fn new(ports: P) -> Pic8259<P> {
        Pic8259 { ports }
    }

    pub fn ports(&self) -> &P {
        &self.ports
    }

    fn out(&self, port: u16, value: u8) {
        self.ports.write8(port, value);
        self.ports.write8(port::DELAY, 0);
    }

    /// Reinitialises both chips with IRQ 0 to 7 at `master_base` and 8 to 15
    /// at `slave_base`, and leaves every IRQ masked.
    pub fn init(&self, master_base: u8, slave_base: u8) -> Result<(), PicError> {
        let valid = |base: u8| base.is_multiple_of(8) && base >= 32;
        if !valid(master_base) || !valid(slave_base) || master_base == slave_base {
            return Err(PicError::InvalidBase);
        }
        use command::*;
        self.out(port::MASTER_COMMAND, ICW1_INIT | ICW1_ICW4);
        self.out(port::SLAVE_COMMAND, ICW1_INIT | ICW1_ICW4);
        self.out(port::MASTER_DATA, master_base);
        self.out(port::SLAVE_DATA, slave_base);
        // ICW3: on the master a bit mask of inputs with a slave, on the
        // slave the master input it is wired to.
        self.out(port::MASTER_DATA, 1 << CASCADE_IRQ);
        self.out(port::SLAVE_DATA, CASCADE_IRQ);
        self.out(port::MASTER_DATA, ICW4_8086);
        self.out(port::SLAVE_DATA, ICW4_8086);
        self.mask_all();
        Ok(())
    }

    /// The interrupt mask registers (OCW1): bit n set masks IRQ n.
    pub fn masks(&self) -> u16 {
        self.ports.read8(port::MASTER_DATA) as u16 | (self.ports.read8(port::SLAVE_DATA) as u16) << 8
    }

    pub fn set_masks(&self, masks: u16) {
        self.ports.write8(port::MASTER_DATA, masks as u8);
        self.ports.write8(port::SLAVE_DATA, (masks >> 8) as u8);
    }

    pub fn mask_all(&self) {
        self.set_masks(u16::MAX);
    }

    /// Ends IRQ `irq`: an IRQ from the slave needs an EOI on both chips.
    pub fn eoi(&self, irq: u8) {
        if irq >= 8 {
            self.ports.write8(port::SLAVE_COMMAND, command::OCW2_EOI);
        }
        self.ports.write8(port::MASTER_COMMAND, command::OCW2_EOI);
    }

    fn read_register(&self, ocw3: u8) -> u16 {
        self.ports.write8(port::MASTER_COMMAND, ocw3);
        self.ports.write8(port::SLAVE_COMMAND, ocw3);
        self.ports.read8(port::MASTER_COMMAND) as u16 | (self.ports.read8(port::SLAVE_COMMAND) as u16) << 8
    }

    /// In-service register of both chips (bit n = IRQ n).
    pub fn in_service(&self) -> u16 {
        self.read_register(command::OCW3_READ_ISR)
    }

    /// Interrupt request register of both chips.
    pub fn requested(&self) -> u16 {
        self.read_register(command::OCW3_READ_IRR)
    }

    /// Call from the IRQ 7 and IRQ 15 handlers before anything else. If the
    /// interrupt was spurious (the line dropped before it was acknowledged,
    /// so it is not in service), returns true and the handler must not send
    /// an EOI; a spurious IRQ 15 still was a real interrupt on the master's
    /// cascade input, so this sends the master its EOI.
    pub fn is_spurious(&self, irq: u8) -> bool {
        if irq != 7 && irq != 15 {
            return false;
        }
        if self.in_service() & (1 << irq) != 0 {
            return false;
        }
        if irq == 15 {
            self.ports.write8(port::MASTER_COMMAND, command::OCW2_EOI);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::sim::SimPic;
    use std::vec::Vec;

    #[test]
    fn init_sequence() {
        let sim = SimPic::new();
        // Firmware left things unmasked and at the reset vectors.
        sim.set_masks(0x0000);
        let pic = Pic8259::new(&sim);
        pic.init(0x20, 0x28).unwrap();
        let log = sim.log();
        // A delay write follows every initialisation word.
        let words: Vec<(u16, u8)> = log.iter().copied().filter(|&(p, _)| p != port::DELAY).collect();
        assert_eq!(log.len(), 8 * 2 + 2);
        for (i, &(p, _)) in log[..16].iter().enumerate() {
            assert_eq!(p == port::DELAY, i % 2 == 1, "{log:x?}");
        }
        assert_eq!(
            words,
            [
                (0x20, 0x11),
                (0xa0, 0x11),
                (0x21, 0x20),
                (0xa1, 0x28),
                (0x21, 0x04),
                (0xa1, 0x02),
                (0x21, 0x01),
                (0xa1, 0x01),
                (0x21, 0xff),
                (0xa1, 0xff)
            ]
        );
        assert_eq!(sim.state(), [(0x20, 0x04, 0x01, true), (0x28, 0x02, 0x01, true)]);
        assert_eq!(pic.masks(), 0xffff);
    }

    #[test]
    fn rejects_bad_bases() {
        let sim = SimPic::new();
        let pic = Pic8259::new(&sim);
        assert_eq!(pic.init(0x21, 0x28), Err(PicError::InvalidBase));
        assert_eq!(pic.init(0x08, 0x70), Err(PicError::InvalidBase));
        assert_eq!(pic.init(0x30, 0x30), Err(PicError::InvalidBase));
        assert!(sim.log().is_empty());
        pic.init(0xf0, 0xf8).unwrap();
    }

    #[test]
    fn masks_round_trip() {
        let sim = SimPic::new();
        let pic = Pic8259::new(&sim);
        pic.init(0x20, 0x28).unwrap();
        pic.set_masks(0b1111_1110_1111_1011);
        assert_eq!(pic.masks(), 0b1111_1110_1111_1011);
        pic.mask_all();
        assert_eq!(pic.masks(), 0xffff);
    }

    #[test]
    fn eoi_goes_to_the_right_chips() {
        let sim = SimPic::new();
        let pic = Pic8259::new(&sim);
        pic.init(0x20, 0x28).unwrap();
        sim.raise(3);
        pic.eoi(3);
        assert_eq!(sim.eois(), (1, 0));
        sim.raise(10);
        pic.eoi(10);
        assert_eq!(sim.eois(), (2, 1));
        assert_eq!(pic.in_service(), 0);
    }

    #[test]
    fn spurious_interrupts() {
        let sim = SimPic::new();
        let pic = Pic8259::new(&sim);
        pic.init(0x20, 0x28).unwrap();
        assert!(!pic.is_spurious(3));
        // Nothing in service: both are spurious; only IRQ 15 costs the
        // master an EOI.
        assert!(pic.is_spurious(7));
        assert_eq!(sim.eois(), (0, 0));
        assert!(pic.is_spurious(15));
        assert_eq!(sim.eois(), (1, 0));
        // Real ones.
        sim.raise(7);
        assert!(!pic.is_spurious(7));
        sim.raise(15);
        assert!(!pic.is_spurious(15));
        assert_eq!(pic.in_service(), 1 << 7 | 1 << 15 | 1 << CASCADE_IRQ);
        sim.request(4);
        assert_eq!(pic.requested(), 1 << 4);
    }
}
