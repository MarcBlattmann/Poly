//! A simulated configuration space for the unit tests.
//!
//! Each function is 4 KiB of bytes plus two per-byte masks: which bits
//! software can write and which are write-one-to-clear. That is enough to
//! model BAR sizing (the address bits below the size are read-only zero),
//! the command / status pair, and bridges. Absent functions read all ones.

extern crate std;

use crate::{Bdf, ConfigSpace, command, reg};
use std::{boxed::Box, cell::RefCell, collections::HashMap, vec::Vec};

pub struct Function {
    bytes: Box<[u8; 4096]>,
    wmask: Box<[u8; 4096]>,
    w1c: Box<[u8; 4096]>,
}

impl Function {
    fn blank(vendor: u16, device: u16, header_type: u8) -> Function {
        let mut f = Function { bytes: Box::new([0; 4096]), wmask: Box::new([0; 4096]), w1c: Box::new([0; 4096]) };
        f.put16(reg::VENDOR_ID, vendor);
        f.put16(reg::DEVICE_ID, device);
        f.put8(reg::HEADER_TYPE, header_type);
        // Command: I/O, memory, bus master and INTx disable are writable.
        f.mask16(reg::COMMAND, command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER | command::INTX_DISABLE);
        // Status: the error bits (15:11, 8) are write-one-to-clear.
        f.w1c[reg::STATUS as usize] = 0x00;
        f.w1c[reg::STATUS as usize + 1] = 0xf9;
        f.put8(reg::INTERRUPT_LINE, 0xff);
        f.wmask[reg::INTERRUPT_LINE as usize] = 0xff;
        f
    }

    /// Type 0 function; `class` is `[class, subclass, prog_if]`.
    pub fn endpoint(vendor: u16, device: u16, class: [u8; 3]) -> Function {
        let mut f = Function::blank(vendor, device, 0);
        f.put8(reg::CLASS, class[0]);
        f.put8(reg::SUBCLASS, class[1]);
        f.put8(reg::PROG_IF, class[2]);
        f
    }

    /// PCI-to-PCI bridge (class 06/04) forwarding `secondary..=subordinate`.
    pub fn bridge(primary: u8, secondary: u8, subordinate: u8) -> Function {
        let mut f = Function::blank(0x8086, 0x0001, 1);
        f.put8(reg::CLASS, 0x06);
        f.put8(reg::SUBCLASS, 0x04);
        f.put8(reg::PRIMARY_BUS, primary);
        f.put8(reg::SECONDARY_BUS, secondary);
        f.put8(reg::SUBORDINATE_BUS, subordinate);
        f
    }

    pub fn multifunction(mut self) -> Function {
        self.bytes[reg::HEADER_TYPE as usize] |= 0x80;
        self
    }

    pub fn with8(mut self, offset: u16, value: u8) -> Function {
        self.put8(offset, value);
        self
    }

    pub fn with16(mut self, offset: u16, value: u16) -> Function {
        self.put16(offset, value);
        self
    }

    pub fn with32(mut self, offset: u16, value: u32) -> Function {
        self.put32(offset, value);
        self
    }

    /// Make `mask` bits of the dword at `offset` writable.
    pub fn writable32(mut self, offset: u16, mask: u32) -> Function {
        self.mask32(offset, mask);
        self
    }

    pub fn status_bits(mut self, bits: u16) -> Function {
        let s = self.get16(reg::STATUS) | bits;
        self.put16(reg::STATUS, s);
        self
    }

    /// 32-bit memory BAR (`io` false) or I/O BAR decoding 16 address bits.
    pub fn bar32(mut self, index: u16, address: u32, size: u32, io: bool, prefetchable: bool) -> Function {
        let off = reg::BAR0 + 4 * index;
        if io {
            self.put32(off, (address & !3) | 1);
            self.mask32(off, !(size - 1) & 0xfffc);
        } else {
            self.put32(off, (address & !0xf) | if prefetchable { 8 } else { 0 });
            self.mask32(off, !(size - 1) & !0xf);
        }
        self
    }

    pub fn bar64(mut self, index: u16, address: u64, size: u64, prefetchable: bool) -> Function {
        let off = reg::BAR0 + 4 * index;
        let mask = !(size - 1);
        self.put32(off, (address as u32 & !0xf) | 0b100 | if prefetchable { 8 } else { 0 });
        self.mask32(off, mask as u32 & !0xf);
        self.put32(off + 4, (address >> 32) as u32);
        self.mask32(off + 4, (mask >> 32) as u32);
        self
    }

    /// A capability at `offset`: id, next pointer, then `body`. Also sets
    /// the capability-list status bit; the head pointer is set separately
    /// with `with8(reg::CAPABILITIES_POINTER, ..)`.
    pub fn cap(mut self, offset: u8, id: u8, next: u8, body: &[u8]) -> Function {
        let o = offset as usize;
        self.bytes[o] = id;
        self.bytes[o + 1] = next;
        self.bytes[o + 2..o + 2 + body.len()].copy_from_slice(body);
        self.status_bits(crate::status::CAPABILITIES_LIST)
    }

    pub fn put8(&mut self, offset: u16, v: u8) {
        self.bytes[offset as usize] = v;
    }

    pub fn put16(&mut self, offset: u16, v: u16) {
        let o = offset as usize;
        self.bytes[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    pub fn put32(&mut self, offset: u16, v: u32) {
        let o = offset as usize;
        self.bytes[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn get16(&self, offset: u16) -> u16 {
        let o = offset as usize;
        u16::from_le_bytes([self.bytes[o], self.bytes[o + 1]])
    }

    pub fn get32(&self, offset: u16) -> u32 {
        let o = offset as usize;
        u32::from_le_bytes(self.bytes[o..o + 4].try_into().unwrap())
    }

    fn mask16(&mut self, offset: u16, m: u16) {
        let o = offset as usize;
        self.wmask[o..o + 2].copy_from_slice(&m.to_le_bytes());
    }

    fn mask32(&mut self, offset: u16, m: u32) {
        let o = offset as usize;
        self.wmask[o..o + 4].copy_from_slice(&m.to_le_bytes());
    }
}

/// One recorded configuration write: function, offset, width, value.
pub type Write = (Bdf, u16, u8, u32);

#[derive(Default)]
pub struct Sim {
    functions: RefCell<HashMap<Bdf, Function>>,
    pub writes: RefCell<Vec<Write>>,
}

impl Sim {
    pub fn new() -> Sim {
        Sim::default()
    }

    pub fn add(&self, bdf: Bdf, f: Function) {
        self.functions.borrow_mut().insert(bdf, f);
    }

    pub fn get16(&self, bdf: Bdf, offset: u16) -> u16 {
        self.functions.borrow()[&bdf].get16(offset)
    }

    pub fn get32(&self, bdf: Bdf, offset: u16) -> u32 {
        self.functions.borrow()[&bdf].get32(offset)
    }

    pub fn set16(&self, bdf: Bdf, offset: u16, v: u16) {
        self.functions.borrow_mut().get_mut(&bdf).unwrap().put16(offset, v);
    }

    fn write(&self, bdf: Bdf, offset: u16, width: u8, value: u32) {
        assert_eq!(offset % width as u16, 0, "unaligned config write");
        self.writes.borrow_mut().push((bdf, offset, width, value));
        let mut fns = self.functions.borrow_mut();
        let Some(f) = fns.get_mut(&bdf) else { return };
        // A BAR must never be sized while the function decodes it: the
        // all-ones address would claim most of the address space.
        let in_bars = (reg::BAR0..reg::BAR0 + 24).contains(&offset);
        if in_bars && width == 4 && value == u32::MAX {
            let cmd = f.get16(reg::COMMAND);
            assert_eq!(cmd & (command::IO_SPACE | command::MEMORY_SPACE), 0, "BAR sized with decode enabled");
        }
        for i in 0..width as usize {
            let o = offset as usize + i;
            let v = (value >> (8 * i)) as u8;
            f.bytes[o] = (f.bytes[o] & !f.wmask[o]) | (v & f.wmask[o]);
            f.bytes[o] &= !(v & f.w1c[o]);
        }
    }
}

impl ConfigSpace for Sim {
    fn read32(&self, bdf: Bdf, offset: u16) -> u32 {
        assert_eq!(offset % 4, 0, "unaligned config read");
        self.functions.borrow().get(&bdf).map_or(u32::MAX, |f| f.get32(offset))
    }

    fn write32(&self, bdf: Bdf, offset: u16, value: u32) {
        self.write(bdf, offset, 4, value);
    }

    fn write16(&self, bdf: Bdf, offset: u16, value: u16) {
        self.write(bdf, offset, 2, value as u32);
    }

    fn write8(&self, bdf: Bdf, offset: u16, value: u8) {
        self.write(bdf, offset, 1, value as u32);
    }
}
