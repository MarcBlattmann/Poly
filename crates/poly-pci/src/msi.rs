//! Message signalled interrupts (PCI 3.0 §6.8).
//!
//! With MSI the function writes `data` to `address` to raise an interrupt.
//! The message itself is architecture-specific (on x86 the address selects a
//! local APIC and the data a vector) and is built by the kernel; this module
//! only puts it where the function will find it. MSI keeps one message in the
//! capability; MSI-X keeps a table of them in a memory BAR. A function must
//! have at most one of INTx, MSI and MSI-X enabled at a time.

use crate::{Bar, Bdf, Capability, ConfigSpace, Mmio, cap_id};

/// The address / data pair a function writes to signal an interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Message {
    pub address: u64,
    pub data: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsiError {
    /// The capability is not MSI (or not MSI-X).
    WrongCapability,
    /// The structure would run past the end of the 256-byte header.
    Truncated,
    /// MSI-X table or PBA in BAR 6 or 7, which do not exist.
    ReservedBir,
    /// MSI-X table or PBA not inside a memory BAR of the function.
    OutsideBar,
    /// The message address is not dword-aligned.
    MisalignedAddress,
    /// A 64-bit message address on a function that only has 32 bits.
    AddressTooWide,
    /// MSI data is 16 bits wide.
    DataTooWide,
    /// The MSI vector count is not a power of two the function supports.
    BadVectorCount,
    /// With several MSI vectors the function fills in the low data bits, so
    /// they must start out zero.
    MisalignedData,
    /// The function cannot mask individual MSI vectors.
    NoMasking,
    NoSuchVector,
}

/// The MSI capability (PCI 3.0 §6.8.1).
///
/// ```text
/// +0x00  ID, next, message control (u16)
/// +0x04  message address [31:2]
/// +0x08  message upper address    (64-bit capable only)
/// next   message data (u16)
/// next   mask bits, pending bits  (per-vector masking only, u32 each, 32-bit aligned)
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msi {
    pub offset: u8,
    /// Message control as read when the capability was parsed.
    pub control: u16,
}

mod msi_control {
    pub const ENABLE: u16 = 1 << 0;
    pub const MULTIPLE_MESSAGE_CAPABLE_SHIFT: u16 = 1;
    pub const MULTIPLE_MESSAGE_ENABLE_SHIFT: u16 = 4;
    pub const MULTIPLE_MESSAGE_ENABLE: u16 = 0b111 << MULTIPLE_MESSAGE_ENABLE_SHIFT;
    pub const ADDRESS_64: u16 = 1 << 7;
    pub const PER_VECTOR_MASKING: u16 = 1 << 8;
}

impl Msi {
    pub fn new<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf, cap: Capability) -> Result<Msi, MsiError> {
        if cap.id != cap_id::MSI {
            return Err(MsiError::WrongCapability);
        }
        let control = cfg.read16(bdf, cap.offset as u16 + 2);
        let msi = Msi { offset: cap.offset, control };
        if !cap.fits(msi.size()) {
            return Err(MsiError::Truncated);
        }
        Ok(msi)
    }

    pub const fn is_64bit(&self) -> bool {
        self.control & msi_control::ADDRESS_64 != 0
    }

    pub const fn per_vector_masking(&self) -> bool {
        self.control & msi_control::PER_VECTOR_MASKING != 0
    }

    /// Vectors the function can use: 1, 2, 4, 8, 16 or 32. Encodings above
    /// 32 are reserved and read as 32.
    pub const fn max_vectors(&self) -> u8 {
        let log2 = (self.control >> msi_control::MULTIPLE_MESSAGE_CAPABLE_SHIFT) & 0b111;
        1 << if log2 > 5 { 5 } else { log2 }
    }

    /// Bytes the capability occupies.
    pub const fn size(&self) -> usize {
        match (self.is_64bit(), self.per_vector_masking()) {
            (false, false) => 0x0a,
            (true, false) => 0x0e,
            (false, true) => 0x14,
            (true, true) => 0x18,
        }
    }

    const fn data_offset(&self) -> u16 {
        self.offset as u16 + if self.is_64bit() { 0x0c } else { 0x08 }
    }

    const fn mask_offset(&self) -> u16 {
        self.offset as u16 + if self.is_64bit() { 0x10 } else { 0x0c }
    }

    const fn control_offset(&self) -> u16 {
        self.offset as u16 + 2
    }

    /// Program the message and the number of vectors (a power of two up to
    /// [`Msi::max_vectors`]); vector `n` signals with `msg.data + n`.
    /// Leaves MSI disabled; call [`Msi::set_enabled`] afterwards.
    pub fn configure<C: ConfigSpace + ?Sized>(
        &self,
        cfg: &C,
        bdf: Bdf,
        msg: Message,
        vectors: u8,
    ) -> Result<(), MsiError> {
        if !vectors.is_power_of_two() || vectors > self.max_vectors() {
            return Err(MsiError::BadVectorCount);
        }
        if msg.address & 3 != 0 {
            return Err(MsiError::MisalignedAddress);
        }
        if !self.is_64bit() && msg.address > u32::MAX as u64 {
            return Err(MsiError::AddressTooWide);
        }
        let Ok(data) = u16::try_from(msg.data) else {
            return Err(MsiError::DataTooWide);
        };
        if data & (vectors as u16 - 1) != 0 {
            return Err(MsiError::MisalignedData);
        }
        let base = self.offset as u16;
        cfg.write32(bdf, base + 4, msg.address as u32);
        if self.is_64bit() {
            cfg.write32(bdf, base + 8, (msg.address >> 32) as u32);
        }
        cfg.write16(bdf, self.data_offset(), data);
        let mme = (vectors.trailing_zeros() as u16) << msi_control::MULTIPLE_MESSAGE_ENABLE_SHIFT;
        let control = cfg.read16(bdf, self.control_offset());
        cfg.write16(bdf, self.control_offset(), (control & !msi_control::MULTIPLE_MESSAGE_ENABLE) | mme);
        Ok(())
    }

    pub fn set_enabled<C: ConfigSpace + ?Sized>(&self, cfg: &C, bdf: Bdf, enabled: bool) {
        let control = cfg.read16(bdf, self.control_offset());
        let control = if enabled { control | msi_control::ENABLE } else { control & !msi_control::ENABLE };
        cfg.write16(bdf, self.control_offset(), control);
    }

    /// Mask or unmask one vector. Needs per-vector masking.
    pub fn set_masked<C: ConfigSpace + ?Sized>(
        &self,
        cfg: &C,
        bdf: Bdf,
        vector: u8,
        masked: bool,
    ) -> Result<(), MsiError> {
        let bit = self.vector_bit(vector)?;
        let mask = cfg.read32(bdf, self.mask_offset());
        cfg.write32(bdf, self.mask_offset(), if masked { mask | bit } else { mask & !bit });
        Ok(())
    }

    /// Has a masked vector got an interrupt waiting? Needs per-vector
    /// masking.
    pub fn is_pending<C: ConfigSpace + ?Sized>(&self, cfg: &C, bdf: Bdf, vector: u8) -> Result<bool, MsiError> {
        let bit = self.vector_bit(vector)?;
        Ok(cfg.read32(bdf, self.mask_offset() + 4) & bit != 0)
    }

    fn vector_bit(&self, vector: u8) -> Result<u32, MsiError> {
        if !self.per_vector_masking() {
            return Err(MsiError::NoMasking);
        }
        if vector >= self.max_vectors() {
            return Err(MsiError::NoSuchVector);
        }
        Ok(1 << vector)
    }
}

/// The MSI-X capability (PCI 3.0 §6.8.2).
///
/// ```text
/// +0x00  ID, next, message control (u16)
/// +0x04  table offset [31:3] | table BIR [2:0]
/// +0x08  PBA offset [31:3]   | PBA BIR [2:0]
/// ```
///
/// The table has one 16-byte entry per vector (address low, address high,
/// data, vector control); the pending bit array (PBA) one bit per vector, in
/// qwords. Both live in memory BARs ("BIR" names the BAR). To bring MSI-X up
/// without spurious interrupts: set the function mask and enable bits,
/// program the entries, then clear the function mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsiX {
    pub offset: u8,
    /// Number of table entries, 1 to 2048.
    pub table_size: u16,
    pub table_bar: u8,
    pub table_offset: u32,
    pub pba_bar: u8,
    pub pba_offset: u32,
}

mod msix_control {
    pub const TABLE_SIZE: u16 = 0x7ff;
    pub const FUNCTION_MASK: u16 = 1 << 14;
    pub const ENABLE: u16 = 1 << 15;
}

/// Size of one MSI-X table entry and the offsets inside it.
const ENTRY_SIZE: usize = 16;
const ENTRY_ADDRESS_LOW: usize = 0;
const ENTRY_ADDRESS_HIGH: usize = 4;
const ENTRY_DATA: usize = 8;
const ENTRY_VECTOR_CONTROL: usize = 12;
const VECTOR_MASKED: u32 = 1 << 0;

impl MsiX {
    const LEN: usize = 12;

    pub fn new<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf, cap: Capability) -> Result<MsiX, MsiError> {
        if cap.id != cap_id::MSI_X {
            return Err(MsiError::WrongCapability);
        }
        if !cap.fits(Self::LEN) {
            return Err(MsiError::Truncated);
        }
        let base = cap.offset as u16;
        let control = cfg.read16(bdf, base + 2);
        let table = cfg.read32(bdf, base + 4);
        let pba = cfg.read32(bdf, base + 8);
        let msix = MsiX {
            offset: cap.offset,
            table_size: (control & msix_control::TABLE_SIZE) + 1,
            table_bar: (table & 7) as u8,
            table_offset: table & !7,
            pba_bar: (pba & 7) as u8,
            pba_offset: pba & !7,
        };
        if msix.table_bar > 5 || msix.pba_bar > 5 {
            return Err(MsiError::ReservedBir);
        }
        Ok(msix)
    }

    /// Bytes the table occupies.
    pub const fn table_len(&self) -> u64 {
        self.table_size as u64 * ENTRY_SIZE as u64
    }

    /// Bytes the pending bit array occupies.
    pub const fn pba_len(&self) -> u64 {
        (self.table_size as u64).div_ceil(64) * 8
    }

    /// Check that the table and the PBA lie inside memory BARs of the
    /// function (as returned by [`crate::probe_bars`]), before mapping them.
    pub fn check_bars(&self, bars: &[Option<Bar>; 6]) -> Result<(), MsiError> {
        let inside = |bar: u8, offset: u32, len: u64| {
            bars.get(bar as usize).copied().flatten().is_some_and(|b| b.is_memory() && b.contains(offset as u64, len))
        };
        if inside(self.table_bar, self.table_offset, self.table_len())
            && inside(self.pba_bar, self.pba_offset, self.pba_len())
        {
            Ok(())
        } else {
            Err(MsiError::OutsideBar)
        }
    }

    pub fn set_enabled<C: ConfigSpace + ?Sized>(&self, cfg: &C, bdf: Bdf, enabled: bool) {
        self.update_control(cfg, bdf, msix_control::ENABLE, enabled);
    }

    /// Mask (or unmask) every vector at once, whatever the per-entry masks
    /// say.
    pub fn set_function_mask<C: ConfigSpace + ?Sized>(&self, cfg: &C, bdf: Bdf, masked: bool) {
        self.update_control(cfg, bdf, msix_control::FUNCTION_MASK, masked);
    }

    fn update_control<C: ConfigSpace + ?Sized>(&self, cfg: &C, bdf: Bdf, bit: u16, set: bool) {
        let off = self.offset as u16 + 2;
        let control = cfg.read16(bdf, off);
        cfg.write16(bdf, off, if set { control | bit } else { control & !bit });
    }

    /// Program table entry `index`. `table` maps the table (offset 0 is
    /// entry 0). The entry is masked while its address and data change,
    /// since changing them on an unmasked entry is undefined (PCI 3.0
    /// §6.8.2.9), and is left masked or unmasked as asked.
    pub fn write_entry<M: Mmio + ?Sized>(
        &self,
        table: &M,
        index: u16,
        msg: Message,
        masked: bool,
    ) -> Result<(), MsiError> {
        if msg.address & 3 != 0 {
            return Err(MsiError::MisalignedAddress);
        }
        let entry = self.entry(index)?;
        let control = table.read32(entry + ENTRY_VECTOR_CONTROL);
        table.write32(entry + ENTRY_VECTOR_CONTROL, control | VECTOR_MASKED);
        table.write32(entry + ENTRY_ADDRESS_LOW, msg.address as u32);
        table.write32(entry + ENTRY_ADDRESS_HIGH, (msg.address >> 32) as u32);
        table.write32(entry + ENTRY_DATA, msg.data);
        if !masked {
            table.write32(entry + ENTRY_VECTOR_CONTROL, control & !VECTOR_MASKED);
        }
        Ok(())
    }

    /// Mask or unmask one vector. The other vector control bits are
    /// reserved and kept as read.
    pub fn set_vector_masked<M: Mmio + ?Sized>(&self, table: &M, index: u16, masked: bool) -> Result<(), MsiError> {
        let off = self.entry(index)? + ENTRY_VECTOR_CONTROL;
        let control = table.read32(off);
        table.write32(off, if masked { control | VECTOR_MASKED } else { control & !VECTOR_MASKED });
        Ok(())
    }

    /// Pending bit of vector `index`. `pba` maps the PBA.
    pub fn is_pending<M: Mmio + ?Sized>(&self, pba: &M, index: u16) -> Result<bool, MsiError> {
        if index >= self.table_size {
            return Err(MsiError::NoSuchVector);
        }
        let dword = pba.read32(index as usize / 32 * 4);
        Ok(dword & (1 << (index % 32)) != 0)
    }

    fn entry(&self, index: u16) -> Result<usize, MsiError> {
        if index >= self.table_size {
            return Err(MsiError::NoSuchVector);
        }
        Ok(index as usize * ENTRY_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Function, Sim};
    use crate::{BarKind, Capabilities, Header, reg};
    extern crate std;
    use std::{cell::RefCell, vec, vec::Vec};

    const B: Bdf = Bdf::new(0, 6, 0);

    fn setup(f: Function) -> (Sim, Header) {
        let s = Sim::new();
        s.add(B, f.with8(reg::CAPABILITIES_POINTER, 0x50));
        let h = Header::read(&s, B).unwrap();
        (s, h)
    }

    /// An MSI capability at 0x50 with the given message control, every
    /// software-writable field writable.
    fn msi_dev(control: u16) -> Function {
        Function::endpoint(1, 2, [2, 0, 0])
            .cap(0x50, cap_id::MSI, 0, &control.to_le_bytes())
            .writable32(0x50, 0x0071_0000)
            .writable32(0x54, 0xffff_fffc)
            .writable32(0x58, 0xffff_ffff)
            .writable32(0x5c, 0xffff_ffff)
            .writable32(0x60, 0xffff_ffff)
    }

    fn msi(s: &Sim, h: &Header) -> Msi {
        Msi::new(s, B, Capabilities::new(s, h).find_id(cap_id::MSI).unwrap()).unwrap()
    }

    #[test]
    fn msi_32bit() {
        let (s, h) = setup(msi_dev(0b010 << 1));
        let m = msi(&s, &h);
        assert!(!m.is_64bit() && !m.per_vector_masking());
        assert_eq!(m.max_vectors(), 4);
        assert_eq!(m.size(), 0x0a);
        let msg = Message { address: 0xfee0_0000, data: 0x40 };
        m.configure(&s, B, msg, 4).unwrap();
        m.set_enabled(&s, B, true);
        assert_eq!(s.get32(B, 0x54), 0xfee0_0000);
        assert_eq!(s.get16(B, 0x58), 0x40);
        let control = s.get16(B, 0x52);
        assert_eq!(control & 1, 1);
        assert_eq!((control >> 4) & 7, 2);
        m.set_enabled(&s, B, false);
        assert_eq!(s.get16(B, 0x52) & 1, 0);

        assert_eq!(m.configure(&s, B, Message { address: 1 << 32, data: 0 }, 1), Err(MsiError::AddressTooWide));
        assert_eq!(m.configure(&s, B, Message { address: 0xfee0_0002, data: 0 }, 1), Err(MsiError::MisalignedAddress));
        assert_eq!(m.configure(&s, B, Message { address: 0xfee0_0000, data: 0x1_0000 }, 1), Err(MsiError::DataTooWide));
        assert_eq!(m.configure(&s, B, msg, 3), Err(MsiError::BadVectorCount));
        assert_eq!(m.configure(&s, B, msg, 8), Err(MsiError::BadVectorCount));
        assert_eq!(m.configure(&s, B, Message { address: 0xfee0_0000, data: 0x41 }, 2), Err(MsiError::MisalignedData));
        assert_eq!(m.set_masked(&s, B, 0, true), Err(MsiError::NoMasking));
    }

    #[test]
    fn msi_64bit_with_masking() {
        let (s, h) = setup(msi_dev(msi_control::ADDRESS_64 | msi_control::PER_VECTOR_MASKING | (0b101 << 1)));
        let m = msi(&s, &h);
        assert!(m.is_64bit() && m.per_vector_masking());
        assert_eq!(m.max_vectors(), 32);
        assert_eq!(m.size(), 0x18);
        m.configure(&s, B, Message { address: 0x1_2345_6780, data: 0x20 }, 32).unwrap();
        assert_eq!((s.get32(B, 0x54), s.get32(B, 0x58)), (0x2345_6780, 1));
        assert_eq!(s.get16(B, 0x5c), 0x20);
        m.set_masked(&s, B, 3, true).unwrap();
        m.set_masked(&s, B, 31, true).unwrap();
        m.set_masked(&s, B, 3, false).unwrap();
        assert_eq!(s.get32(B, 0x60), 1 << 31);
        assert_eq!(m.set_masked(&s, B, 32, true), Err(MsiError::NoSuchVector));
        s.set16(B, 0x64, 1 << 5);
        assert_eq!(m.is_pending(&s, B, 5), Ok(true));
        assert_eq!(m.is_pending(&s, B, 4), Ok(false));
    }

    #[test]
    fn msi_reserved_capable_count_and_truncation() {
        let (s, h) = setup(msi_dev(0b111 << 1));
        assert_eq!(msi(&s, &h).max_vectors(), 32);
        let s = Sim::new();
        let ctl = msi_control::ADDRESS_64 | msi_control::PER_VECTOR_MASKING;
        s.add(
            B,
            Function::endpoint(1, 2, [0; 3]).with8(reg::CAPABILITIES_POINTER, 0xf0).cap(
                0xf0,
                cap_id::MSI,
                0,
                &ctl.to_le_bytes(),
            ),
        );
        let h = Header::read(&s, B).unwrap();
        let cap = Capabilities::new(&s, &h).find_id(cap_id::MSI).unwrap();
        assert_eq!(Msi::new(&s, B, cap), Err(MsiError::Truncated));
        assert_eq!(MsiX::new(&s, B, cap), Err(MsiError::WrongCapability));
    }

    fn msix_dev(control: u16, table: u32, pba: u32) -> Function {
        let mut body = vec![];
        body.extend_from_slice(&control.to_le_bytes());
        body.extend_from_slice(&table.to_le_bytes());
        body.extend_from_slice(&pba.to_le_bytes());
        Function::endpoint(1, 2, [2, 0, 0]).cap(0x50, cap_id::MSI_X, 0, &body).writable32(0x50, 0xc000_0000)
    }

    fn msix(s: &Sim, h: &Header) -> Result<MsiX, MsiError> {
        MsiX::new(s, B, Capabilities::new(s, h).find_id(cap_id::MSI_X).unwrap())
    }

    #[test]
    fn msix_parse_and_control() {
        let (s, h) = setup(msix_dev(63, 0x2000 | 4, 0x3000 | 4));
        let m = msix(&s, &h).unwrap();
        assert_eq!(
            m,
            MsiX { offset: 0x50, table_size: 64, table_bar: 4, table_offset: 0x2000, pba_bar: 4, pba_offset: 0x3000 }
        );
        assert_eq!((m.table_len(), m.pba_len()), (1024, 8));
        m.set_function_mask(&s, B, true);
        m.set_enabled(&s, B, true);
        assert_eq!(s.get16(B, 0x52), 0xc000 | 63);
        m.set_function_mask(&s, B, false);
        assert_eq!(s.get16(B, 0x52), 0x8000 | 63);

        let bar = |size| Some(Bar { kind: BarKind::Memory64, address: 0xfe00_0000, size, prefetchable: false });
        let mut bars = [None; 6];
        bars[4] = bar(0x4000);
        assert_eq!(m.check_bars(&bars), Ok(()));
        bars[4] = bar(0x3004);
        assert_eq!(m.check_bars(&bars), Err(MsiError::OutsideBar));
        bars[4] = Some(Bar { kind: BarKind::Io, address: 0xc000, size: 0x4000, prefetchable: false });
        assert_eq!(m.check_bars(&bars), Err(MsiError::OutsideBar));
        assert_eq!(m.check_bars(&[None; 6]), Err(MsiError::OutsideBar));
    }

    #[test]
    fn msix_reserved_bir() {
        let (s, h) = setup(msix_dev(0, 6, 0));
        assert_eq!(msix(&s, &h), Err(MsiError::ReservedBir));
        let (s, h) = setup(msix_dev(0x7ff, 0, 7));
        assert_eq!(msix(&s, &h), Err(MsiError::ReservedBir));
    }

    /// An MSI-X table in memory that logs every write.
    struct Table {
        mem: RefCell<Vec<u32>>,
        log: RefCell<Vec<(usize, u32)>>,
    }

    impl Mmio for Table {
        fn read8(&self, _: usize) -> u8 {
            unimplemented!()
        }
        fn read16(&self, _: usize) -> u16 {
            unimplemented!()
        }
        fn read32(&self, offset: usize) -> u32 {
            assert_eq!(offset % 4, 0);
            self.mem.borrow()[offset / 4]
        }
        fn write8(&self, _: usize, _: u8) {
            unimplemented!()
        }
        fn write16(&self, _: usize, _: u16) {
            unimplemented!()
        }
        fn write32(&self, offset: usize, value: u32) {
            assert_eq!(offset % 4, 0);
            self.mem.borrow_mut()[offset / 4] = value;
            self.log.borrow_mut().push((offset, value));
        }
    }

    #[test]
    fn msix_table_entries() {
        let (s, h) = setup(msix_dev(3, 0, 0x800));
        let m = msix(&s, &h).unwrap();
        // Entries come out of reset masked; bit 31 stands in for a reserved
        // bit that must survive.
        let t = Table { mem: RefCell::new(vec![1 | 1 << 31; 16]), log: RefCell::new(vec![]) };
        m.write_entry(&t, 2, Message { address: 0x1_fee0_1000, data: 0x31 }, false).unwrap();
        assert_eq!(&t.mem.borrow()[8..12], &[0xfee0_1000, 1, 0x31, 1 << 31]);
        // Masked first, unmasked last.
        let log = t.log.borrow().clone();
        assert_eq!(log.first(), Some(&(44, 1 | 1 << 31)));
        assert_eq!(log.last(), Some(&(44, 1 << 31)));

        t.log.borrow_mut().clear();
        m.write_entry(&t, 0, Message { address: 0xfee0_0000, data: 0x30 }, true).unwrap();
        assert_eq!(&t.mem.borrow()[0..4], &[0xfee0_0000, 0, 0x30, 1 | 1 << 31]);
        assert!(t.log.borrow().iter().all(|&(off, v)| off != 12 || v & 1 == 1));

        m.set_vector_masked(&t, 0, false).unwrap();
        assert_eq!(t.mem.borrow()[3], 1 << 31);
        assert_eq!(m.write_entry(&t, 4, Message { address: 0, data: 0 }, false), Err(MsiError::NoSuchVector));
        assert_eq!(m.set_vector_masked(&t, 4, true), Err(MsiError::NoSuchVector));
        assert_eq!(m.write_entry(&t, 0, Message { address: 1, data: 0 }, false), Err(MsiError::MisalignedAddress));

        let pba = Table { mem: RefCell::new(vec![0b100, 0]), log: RefCell::new(vec![]) };
        assert_eq!(m.is_pending(&pba, 2), Ok(true));
        assert_eq!(m.is_pending(&pba, 1), Ok(false));
        assert_eq!(m.is_pending(&pba, 4), Err(MsiError::NoSuchVector));
    }
}
