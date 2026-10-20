//! ACPI static table parsing.
//!
//! Firmware describes the machine through tables in physical memory. The
//! RSDP points to a root table (RSDT or XSDT) that lists every other table.
//! This crate finds and validates those tables and decodes the ones the
//! kernel needs early: MADT (interrupt controllers and CPUs), HPET, FADT
//! (fixed hardware) and MCFG (PCI Express configuration space). It does not
//! interpret AML, the bytecode inside the DSDT and SSDTs.
//!
//! Layouts come from the ACPI Specification 6.5 (UEFI Forum); section numbers
//! in comments refer to it. Two tables are defined by companion documents the
//! ACPI specification points to: HPET by the IA-PC HPET Specification 1.0a
//! (Intel) and MCFG by the PCI Firmware Specification 3.2 (PCI-SIG).
//!
//! The crate never dereferences a physical address itself: every byte comes
//! through [`PhysMemory`], so the kernel decides how memory is mapped and the
//! tests use plain byte buffers. The parsers never panic and never read out
//! of bounds, whatever the firmware hands them. Nothing allocates; lists such
//! as MADT entries are iterators over the table bytes.

#![no_std]

mod fadt;
mod hpet;
mod madt;
mod mcfg;

pub use fadt::{Fadt, FadtFlags, IapcBootArch, PmTimer, ResetRegister};
pub use hpet::Hpet;
pub use madt::{
    InterruptOverride, IoApic, LocalApic, LocalApicNmi, LocalX2Apic, LocalX2ApicNmi, Madt, MadtEntries, MadtEntry,
    MpsIntiFlags, NmiSource, Polarity, Processor, ProcessorFlags, TriggerMode,
};
pub use mcfg::{EcamRegion, Mcfg};

/// Read access to physical memory.
///
/// The kernel implements it on top of its page tables; tests implement it
/// with byte buffers keyed by address.
pub trait PhysMemory {
    /// The `len` bytes starting at physical address `phys`, or `None` if
    /// that range cannot be read. A returned slice shorter or longer than
    /// `len` is treated as `None`.
    fn bytes(&self, phys: u64, len: usize) -> Option<&[u8]>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// [`PhysMemory`] could not provide these bytes, for example a table
    /// beyond what the kernel has mapped or a length field that is far too big.
    Unmapped { phys: u64, len: usize },
    /// The structure does not start with the expected signature.
    BadSignature,
    /// The bytes covered by a checksum do not sum to zero.
    BadChecksum,
    /// A length field smaller than the structure it describes.
    BadLength,
    /// The bytes end before the structure (or a required field) does.
    Truncated,
    /// The root table lists no table with this signature.
    NotFound,
}

/// Fetch exactly `len` bytes, turning a missing mapping into an error.
fn read<M: PhysMemory + ?Sized>(mem: &M, phys: u64, len: usize) -> Result<&[u8], Error> {
    mem.bytes(phys, len).filter(|b| b.len() == len).ok_or(Error::Unmapped { phys, len })
}

fn array<const N: usize>(b: &[u8], off: usize) -> Option<[u8; N]> {
    b.get(off..off.checked_add(N)?)?.try_into().ok()
}

pub(crate) fn u8_at(b: &[u8], off: usize) -> Option<u8> {
    b.get(off).copied()
}

pub(crate) fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    array(b, off).map(u16::from_le_bytes)
}

pub(crate) fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    array(b, off).map(u32::from_le_bytes)
}

pub(crate) fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    array(b, off).map(u64::from_le_bytes)
}

/// ACPI checksums: all bytes of the structure, checksum byte included, sum to
/// zero modulo 256.
fn checksum_ok(b: &[u8]) -> bool {
    b.iter().fold(0u8, |sum, &x| sum.wrapping_add(x)) == 0
}

/// Root System Description Pointer (§5.2.5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rsdp {
    pub oem_id: [u8; 6],
    /// 0 for ACPI 1.0; 2 for ACPI 2.0 and later.
    pub revision: u8,
    pub rsdt_address: u32,
    /// Only ACPI 2.0+ RSDPs have it; `None` when absent or zero.
    pub xsdt_address: Option<u64>,
}

impl Rsdp {
    pub const SIGNATURE: [u8; 8] = *b"RSD PTR ";
    /// Size of the ACPI 1.0 structure, which the first checksum covers.
    pub const V1_LEN: usize = 20;
    /// Size of the ACPI 2.0+ structure, which the extended checksum covers.
    pub const V2_LEN: usize = 36;

    /// Validate an RSDP from bytes: signature, the ACPI 1.0 checksum and,
    /// for revision 2 and later, the length and extended checksum.
    pub fn parse(b: &[u8]) -> Result<Rsdp, Error> {
        const OEM_ID: usize = 9;
        const REVISION: usize = 15;
        const RSDT_ADDRESS: usize = 16;
        const LENGTH: usize = 20;
        const XSDT_ADDRESS: usize = 24;

        if array::<8>(b, 0).ok_or(Error::Truncated)? != Self::SIGNATURE {
            return Err(Error::BadSignature);
        }
        let v1 = b.get(..Self::V1_LEN).ok_or(Error::Truncated)?;
        if !checksum_ok(v1) {
            return Err(Error::BadChecksum);
        }
        let mut rsdp = Rsdp {
            oem_id: array(v1, OEM_ID).ok_or(Error::Truncated)?,
            revision: u8_at(v1, REVISION).ok_or(Error::Truncated)?,
            rsdt_address: u32_at(v1, RSDT_ADDRESS).ok_or(Error::Truncated)?,
            xsdt_address: None,
        };
        // Revision 1 was never assigned; treat it like the 1.0 layout.
        if rsdp.revision < 2 {
            return Ok(rsdp);
        }
        let len = u32_at(b, LENGTH).ok_or(Error::Truncated)? as usize;
        if len < Self::V2_LEN {
            return Err(Error::BadLength);
        }
        let v2 = b.get(..len).ok_or(Error::Truncated)?;
        if !checksum_ok(v2) {
            return Err(Error::BadChecksum);
        }
        rsdp.xsdt_address = u64_at(v2, XSDT_ADDRESS).filter(|&a| a != 0);
        Ok(rsdp)
    }

    /// Read and validate the RSDP at physical address `phys`.
    pub fn read<M: PhysMemory + ?Sized>(mem: &M, phys: u64) -> Result<Rsdp, Error> {
        let v1 = read(mem, phys, Self::V1_LEN)?;
        if v1.get(..8) != Some(&Self::SIGNATURE[..]) {
            return Err(Error::BadSignature);
        }
        if u8_at(v1, 15).unwrap_or(0) < 2 {
            return Rsdp::parse(v1);
        }
        let head = read(mem, phys, 24)?;
        let len = u32_at(head, 20).ok_or(Error::Truncated)? as usize;
        if len < Self::V2_LEN {
            // Still check the 1.0 checksum first so a corrupt structure
            // reports that rather than its (meaningless) length.
            Rsdp::parse(v1)?;
            return Err(Error::BadLength);
        }
        Rsdp::parse(read(mem, phys, len)?)
    }

    /// Search `area` for an RSDP on a 16-byte boundary, as firmware places
    /// it in the EBDA or the 0xE0000–0xFFFFF BIOS area (§5.2.5.1). Returns
    /// the offset of the first valid one.
    pub fn search(area: &[u8]) -> Option<usize> {
        (0..area.len()).step_by(16).find(|&off| area.get(off..).is_some_and(|b| Rsdp::parse(b).is_ok()))
    }
}

/// The header every system description table starts with (§5.2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    /// Length of the whole table in bytes, header included.
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: [u8; 4],
    pub creator_revision: u32,
}

impl SdtHeader {
    pub const LEN: usize = 36;

    /// Decode the header fields. Does not check the length or checksum;
    /// [`Sdt::parse`] does.
    pub fn parse(b: &[u8]) -> Result<SdtHeader, Error> {
        let h = b.get(..Self::LEN).ok_or(Error::Truncated)?;
        let field = || Error::Truncated;
        Ok(SdtHeader {
            signature: array(h, 0).ok_or_else(field)?,
            length: u32_at(h, 4).ok_or_else(field)?,
            revision: u8_at(h, 8).ok_or_else(field)?,
            checksum: u8_at(h, 9).ok_or_else(field)?,
            oem_id: array(h, 10).ok_or_else(field)?,
            oem_table_id: array(h, 16).ok_or_else(field)?,
            oem_revision: u32_at(h, 24).ok_or_else(field)?,
            creator_id: array(h, 28).ok_or_else(field)?,
            creator_revision: u32_at(h, 32).ok_or_else(field)?,
        })
    }
}

/// A system description table whose length and checksum have been checked.
/// Its bytes are exactly `header.length` long.
#[derive(Clone, Copy, Debug)]
pub struct Sdt<'a> {
    header: SdtHeader,
    bytes: &'a [u8],
}

impl<'a> Sdt<'a> {
    /// Validate a table from bytes. `b` may be longer than the table.
    pub fn parse(b: &'a [u8]) -> Result<Sdt<'a>, Error> {
        let header = SdtHeader::parse(b)?;
        let len = header.length as usize;
        if len < SdtHeader::LEN {
            return Err(Error::BadLength);
        }
        let bytes = b.get(..len).ok_or(Error::Truncated)?;
        if !checksum_ok(bytes) {
            return Err(Error::BadChecksum);
        }
        Ok(Sdt { header, bytes })
    }

    /// Read and validate the table at physical address `phys`.
    pub fn read<M: PhysMemory + ?Sized>(mem: &'a M, phys: u64) -> Result<Sdt<'a>, Error> {
        let header = SdtHeader::parse(read(mem, phys, SdtHeader::LEN)?)?;
        if (header.length as usize) < SdtHeader::LEN {
            return Err(Error::BadLength);
        }
        Sdt::parse(read(mem, phys, header.length as usize)?)
    }

    pub fn header(&self) -> &SdtHeader {
        &self.header
    }

    pub fn signature(&self) -> [u8; 4] {
        self.header.signature
    }

    /// The whole table, header included.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Everything after the common header.
    pub fn body(&self) -> &'a [u8] {
        self.bytes.get(SdtHeader::LEN..).unwrap_or(&[])
    }

    /// Check the signature and that the table is at least `min_len` bytes:
    /// the common entry check for the typed tables.
    pub(crate) fn expect(self, signature: [u8; 4], min_len: usize) -> Result<Sdt<'a>, Error> {
        if self.header.signature != signature {
            return Err(Error::BadSignature);
        }
        if self.bytes.len() < min_len {
            return Err(Error::Truncated);
        }
        Ok(self)
    }
}

/// A table type this crate decodes.
pub trait Table<'a>: Sized {
    const SIGNATURE: [u8; 4];

    /// Decode a validated table. Fails if the signature is wrong or the
    /// table is too short for its fixed fields.
    fn parse(sdt: Sdt<'a>) -> Result<Self, Error>;
}

/// Generic Address Structure (§5.2.3.2): where a register lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericAddress {
    pub space: AddressSpace,
    pub bit_width: u8,
    pub bit_offset: u8,
    /// 0 undefined, 1 byte, 2 word, 3 dword, 4 qword access.
    pub access_size: u8,
    pub address: u64,
}

impl GenericAddress {
    pub const LEN: usize = 12;

    pub fn parse(b: &[u8]) -> Option<GenericAddress> {
        Some(GenericAddress {
            space: AddressSpace::from(u8_at(b, 0)?),
            bit_width: u8_at(b, 1)?,
            bit_offset: u8_at(b, 2)?,
            access_size: u8_at(b, 3)?,
            address: u64_at(b, 4)?,
        })
    }
}

/// Address space IDs (§5.2.3.2, Table 5.1) the kernel cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressSpace {
    SystemMemory,
    SystemIo,
    PciConfig,
    FunctionalFixedHardware,
    Other(u8),
}

impl From<u8> for AddressSpace {
    fn from(id: u8) -> Self {
        match id {
            0 => AddressSpace::SystemMemory,
            1 => AddressSpace::SystemIo,
            2 => AddressSpace::PciConfig,
            0x7f => AddressSpace::FunctionalFixedHardware,
            other => AddressSpace::Other(other),
        }
    }
}

/// The root table and the memory the tables are read from: the entry point
/// for everything else.
#[derive(Debug)]
pub struct Acpi<'a, M: PhysMemory + ?Sized> {
    mem: &'a M,
    rsdp: Rsdp,
    root: Sdt<'a>,
    /// 8 for the XSDT's 64-bit entries, 4 for the RSDT's 32-bit ones.
    entry_size: usize,
}

impl<'a, M: PhysMemory + ?Sized> Acpi<'a, M> {
    /// Read the RSDP at `rsdp_phys` and the root table it points to.
    pub fn new(mem: &'a M, rsdp_phys: u64) -> Result<Self, Error> {
        Self::from_rsdp(mem, Rsdp::read(mem, rsdp_phys)?)
    }

    /// Load the root table of an already validated RSDP. The XSDT is used
    /// when present (§5.2.5.3 says ACPI 2.0+ OSes must); if it is broken and
    /// an RSDT exists, the RSDT is used instead.
    pub fn from_rsdp(mem: &'a M, rsdp: Rsdp) -> Result<Self, Error> {
        if let Some(xsdt) = rsdp.xsdt_address {
            match Sdt::read(mem, xsdt).and_then(|t| t.expect(*b"XSDT", SdtHeader::LEN)) {
                Ok(root) => return Ok(Acpi { mem, rsdp, root, entry_size: 8 }),
                Err(e) if rsdp.rsdt_address == 0 => return Err(e),
                Err(_) => {}
            }
        }
        let root = Sdt::read(mem, rsdp.rsdt_address.into())?.expect(*b"RSDT", SdtHeader::LEN)?;
        Ok(Acpi { mem, rsdp, root, entry_size: 4 })
    }

    pub fn rsdp(&self) -> &Rsdp {
        &self.rsdp
    }

    /// The RSDT or XSDT in use.
    pub fn root(&self) -> Sdt<'a> {
        self.root
    }

    pub fn uses_xsdt(&self) -> bool {
        self.entry_size == 8
    }

    /// Physical addresses the root table lists, in order (§5.2.7, §5.2.8).
    /// A trailing partial entry is ignored.
    pub fn table_addresses(&self) -> impl Iterator<Item = u64> + use<'a, M> {
        let wide = self.entry_size == 8;
        self.root
            .body()
            .chunks_exact(self.entry_size)
            .filter_map(move |e| if wide { u64_at(e, 0) } else { u32_at(e, 0).map(u64::from) })
    }

    /// Every table the root lists, read and validated. Tables that cannot
    /// be read or fail validation come through as errors.
    pub fn tables(&self) -> impl Iterator<Item = Result<Sdt<'a>, Error>> + use<'a, M> {
        let mem = self.mem;
        self.table_addresses().map(move |phys| Sdt::read(mem, phys))
    }

    /// The first valid table with this signature. If only broken copies
    /// exist, the first one's error is returned; [`Error::NotFound`] if none.
    pub fn find(&self, signature: [u8; 4]) -> Result<Sdt<'a>, Error> {
        let mut err = Error::NotFound;
        for phys in self.table_addresses() {
            let Ok(header) = read(self.mem, phys, SdtHeader::LEN).and_then(SdtHeader::parse) else {
                continue;
            };
            if header.signature != signature {
                continue;
            }
            match Sdt::read(self.mem, phys) {
                Ok(t) => return Ok(t),
                Err(e) if err == Error::NotFound => err = e,
                Err(_) => {}
            }
        }
        Err(err)
    }

    /// Find and decode a table type, e.g. `acpi.table::<Madt>()`.
    pub fn table<T: Table<'a>>(&self) -> Result<T, Error> {
        T::parse(self.find(T::SIGNATURE)?)
    }

    /// The DSDT, which the FADT points to rather than the root table.
    pub fn dsdt(&self) -> Result<Sdt<'a>, Error> {
        let phys = self.table::<Fadt>()?.dsdt_address().ok_or(Error::NotFound)?;
        Sdt::read(self.mem, phys)?.expect(*b"DSDT", SdtHeader::LEN)
    }
}

#[cfg(test)]
pub(crate) mod testutil;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::testutil::*;
    use super::*;
    use std::vec::Vec;

    #[test]
    fn rsdp_acpi_1() {
        let r = Rsdp::parse(&rsdp_v1(0x1234)).unwrap();
        assert_eq!(r, Rsdp { oem_id: *b"POLYOS", revision: 0, rsdt_address: 0x1234, xsdt_address: None });
    }

    #[test]
    fn rsdp_acpi_2() {
        let r = Rsdp::parse(&rsdp_v2(0x1234, 0x1_0000_5678)).unwrap();
        assert_eq!(r.revision, 2);
        assert_eq!(r.rsdt_address, 0x1234);
        assert_eq!(r.xsdt_address, Some(0x1_0000_5678));
        // A zero XSDT address means "none".
        assert_eq!(Rsdp::parse(&rsdp_v2(0x1234, 0)).unwrap().xsdt_address, None);
    }

    #[test]
    fn rsdp_errors() {
        let mut bad_sig = rsdp_v2(1, 2);
        bad_sig[0] = b'X';
        assert_eq!(Rsdp::parse(&bad_sig), Err(Error::BadSignature));

        let mut bad_v1 = rsdp_v2(1, 2);
        // Break the 1.0 checksum and compensate in the extended one (which
        // covers both bytes), so only the 1.0 check can fail.
        bad_v1[8] = bad_v1[8].wrapping_add(1);
        bad_v1[32] = bad_v1[32].wrapping_sub(1);
        assert_eq!(Rsdp::parse(&bad_v1), Err(Error::BadChecksum));

        let mut bad_ext = rsdp_v2(1, 2);
        bad_ext[24] ^= 1; // XSDT address, only the extended checksum covers it
        assert_eq!(Rsdp::parse(&bad_ext), Err(Error::BadChecksum));

        let mut short_len = rsdp_v2(1, 2);
        set_rsdp_length(&mut short_len, 20);
        assert_eq!(Rsdp::parse(&short_len), Err(Error::BadLength));

        assert_eq!(Rsdp::parse(&rsdp_v2(1, 2)[..30]), Err(Error::Truncated));
        assert_eq!(Rsdp::parse(b"RSD PTR "), Err(Error::Truncated));
        assert_eq!(Rsdp::parse(b"RSD"), Err(Error::Truncated));
    }

    #[test]
    fn rsdp_read_through_memory() {
        let mut mem = Mem::default();
        mem.add(0xe_0000, rsdp_v2(0x1000, 0x2000));
        mem.add(0xf_0000, rsdp_v1(0x1000));
        assert_eq!(Rsdp::read(&mem, 0xe_0000).unwrap().xsdt_address, Some(0x2000));
        assert_eq!(Rsdp::read(&mem, 0xf_0000).unwrap().xsdt_address, None);
        assert_eq!(Rsdp::read(&mem, 0xd_0000), Err(Error::Unmapped { phys: 0xd_0000, len: 20 }));

        // A 2.0 RSDP whose length points past the mapping.
        let mut long = rsdp_v2(0x1000, 0x2000);
        set_rsdp_length(&mut long, 0x1000);
        mem.add(0xa_0000, long);
        assert_eq!(Rsdp::read(&mem, 0xa_0000), Err(Error::Unmapped { phys: 0xa_0000, len: 0x1000 }));
    }

    #[test]
    fn rsdp_search_on_16_byte_boundaries() {
        let mut area = std::vec![0u8; 0x400];
        let rsdp = rsdp_v2(0x1000, 0x2000);
        // Misaligned copies do not count.
        area[0x108..0x108 + rsdp.len()].copy_from_slice(&rsdp);
        assert_eq!(Rsdp::search(&area), None);
        area[0x240..0x240 + rsdp.len()].copy_from_slice(&rsdp);
        assert_eq!(Rsdp::search(&area), Some(0x240));
        assert_eq!(Rsdp::search(&[]), None);
    }

    #[test]
    fn sdt_validation() {
        let t = table(b"TEST", 1, &[1, 2, 3, 4]);
        let sdt = Sdt::parse(&t).unwrap();
        assert_eq!(sdt.signature(), *b"TEST");
        assert_eq!(sdt.header().length, 40);
        assert_eq!(sdt.header().oem_id, *b"POLYOS");
        assert_eq!(sdt.body(), &[1, 2, 3, 4]);

        // Extra bytes after the table are not part of it.
        let mut longer = t.clone();
        longer.extend_from_slice(&[0xff; 8]);
        assert_eq!(Sdt::parse(&longer).unwrap().bytes().len(), 40);

        let mut bad = t.clone();
        bad[36] ^= 0x80;
        assert_eq!(Sdt::parse(&bad).map(|_| ()), Err(Error::BadChecksum));

        let mut short = t.clone();
        set_length(&mut short, 35);
        assert_eq!(Sdt::parse(&short).map(|_| ()), Err(Error::BadLength));

        let mut long = t.clone();
        set_length(&mut long, 41);
        assert_eq!(Sdt::parse(&long).map(|_| ()), Err(Error::Truncated));

        assert_eq!(Sdt::parse(&t[..20]).map(|_| ()), Err(Error::Truncated));
    }

    #[test]
    fn sdt_length_beyond_mapping() {
        let mut t = table(b"TEST", 1, &[]);
        set_length(&mut t, u32::MAX);
        let mut mem = Mem::default();
        mem.add(0x1000, t);
        assert_eq!(Sdt::read(&mem, 0x1000).map(|_| ()), Err(Error::Unmapped { phys: 0x1000, len: u32::MAX as usize }));
    }

    #[test]
    fn prefers_xsdt() {
        let sys = System::new().with(b"APIC", madt_qemu()).build();
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        assert!(acpi.uses_xsdt());
        assert_eq!(acpi.root().signature(), *b"XSDT");
        assert!(acpi.find(*b"APIC").is_ok());
    }

    #[test]
    fn rsdt_only() {
        let sys = System::new().v1().with(b"APIC", madt_qemu()).build();
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        assert!(!acpi.uses_xsdt());
        assert_eq!(acpi.rsdp().revision, 0);
        assert!(acpi.find(*b"APIC").is_ok());
    }

    #[test]
    fn broken_xsdt_falls_back_to_rsdt() {
        let mut sys = System::new().with(b"APIC", madt_qemu()).build();
        sys.mem.corrupt(sys.xsdt.unwrap() + 40);
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        assert!(!acpi.uses_xsdt());
        assert!(acpi.find(*b"APIC").is_ok());

        // Without an RSDT the XSDT's error is reported.
        let mut sys = System::new().without_rsdt().build();
        sys.mem.corrupt(sys.xsdt.unwrap() + 40);
        assert_eq!(Acpi::new(&sys.mem, sys.rsdp).map(|_| ()), Err(Error::BadChecksum));
    }

    #[test]
    fn root_with_wrong_signature() {
        let mut mem = Mem::default();
        mem.add(0x1000, table(b"FAKE", 1, &[]));
        mem.add(0x100, rsdp_v1(0x1000));
        assert_eq!(Acpi::new(&mem, 0x100).map(|_| ()), Err(Error::BadSignature));
    }

    #[test]
    fn unmapped_root() {
        let mut mem = Mem::default();
        mem.add(0x100, rsdp_v1(0x9000));
        assert_eq!(Acpi::new(&mem, 0x100).map(|_| ()), Err(Error::Unmapped { phys: 0x9000, len: 36 }));
    }

    #[test]
    fn table_listing_and_find() {
        let sys = System::new()
            .with(b"APIC", madt_qemu())
            .with(b"HPET", hpet_body())
            .with(b"MCFG", mcfg_body(&[(0xb000_0000, 0, 0, 255)]))
            .build();
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        let sigs: Vec<[u8; 4]> = acpi.tables().map(|t| t.unwrap().signature()).collect();
        assert_eq!(sigs, [*b"FACP", *b"APIC", *b"HPET", *b"MCFG"]);
        assert_eq!(acpi.find(*b"SSDT").map(|_| ()), Err(Error::NotFound));
        assert!(acpi.table::<Madt>().is_ok());
        assert!(acpi.table::<Hpet>().is_ok());
        assert!(acpi.table::<Mcfg>().is_ok());
        assert!(acpi.table::<Fadt>().is_ok());
    }

    #[test]
    fn find_skips_unmapped_and_broken_entries() {
        let mut sys = System::new().with(b"APIC", madt_qemu()).with(b"APIC", madt_qemu()).dangling(0x7777_0000).build();
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        assert_eq!(acpi.tables().filter(|t| t.is_err()).count(), 1);
        // A broken first copy is skipped in favour of a valid second one.
        let first = sys.tables[1];
        sys.mem.corrupt(first + 40);
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        assert_eq!(acpi.find(*b"APIC").unwrap().bytes().as_ptr(), sys.mem.ptr(sys.tables[2]));

        // Only broken copies: report why.
        let mut sys = System::new().with(b"APIC", madt_qemu()).build();
        sys.mem.corrupt(sys.tables[1] + 40);
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        assert_eq!(acpi.find(*b"APIC").map(|_| ()), Err(Error::BadChecksum));
        assert_eq!(acpi.table::<Madt>().map(|_| ()), Err(Error::BadChecksum));
    }

    #[test]
    fn dsdt_through_fadt() {
        let sys = System::new().build();
        let acpi = Acpi::new(&sys.mem, sys.rsdp).unwrap();
        let dsdt = acpi.dsdt().unwrap();
        assert_eq!(dsdt.signature(), *b"DSDT");
        assert_eq!(dsdt.body(), DSDT_AML);
    }

    #[test]
    fn root_trailing_partial_entry_is_ignored() {
        let mut mem = Mem::default();
        let mut body = 0x2000u32.to_le_bytes().to_vec();
        body.extend_from_slice(&[0x30, 0x00]);
        mem.add(0x1000, table(b"RSDT", 1, &body));
        mem.add(0x2000, table(b"TEST", 1, &[]));
        mem.add(0x100, rsdp_v1(0x1000));
        let acpi = Acpi::new(&mem, 0x100).unwrap();
        assert_eq!(acpi.table_addresses().collect::<Vec<_>>(), [0x2000]);
    }

    #[test]
    fn generic_address() {
        let mut b = [0u8; 12];
        b[0] = 1;
        b[1] = 32;
        b[3] = 3;
        b[4..].copy_from_slice(&0x608u64.to_le_bytes());
        let g = GenericAddress::parse(&b).unwrap();
        assert_eq!(
            g,
            GenericAddress {
                space: AddressSpace::SystemIo,
                bit_width: 32,
                bit_offset: 0,
                access_size: 3,
                address: 0x608
            }
        );
        assert_eq!(GenericAddress::parse(&b[..11]), None);
        assert_eq!(AddressSpace::from(0x7f), AddressSpace::FunctionalFixedHardware);
        assert_eq!(AddressSpace::from(9), AddressSpace::Other(9));
    }

    /// Every prefix of the RSDP and root table must parse or fail cleanly.
    #[test]
    fn rsdp_and_sdt_prefixes_never_panic() {
        for img in [rsdp_v1(1), rsdp_v2(1, 2)] {
            for n in 0..=img.len() {
                let _ = Rsdp::parse(&img[..n]);
                let _ = Rsdp::search(&img[..n]);
            }
        }
        let t = table(b"XSDT", 1, &[0x11; 24]);
        for n in 0..=t.len() {
            assert!(Sdt::parse(&t[..n]).is_err() || n == t.len());
        }
    }

    /// Random corruption of a whole system (with checksums repaired, so the
    /// damage reaches the decoders) must never panic or hang.
    #[test]
    fn random_corruption_never_panics() {
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..2000 {
            let mut sys = full_system().build();
            let targets = sys.all_sdts();
            for _ in 0..1 + next() % 8 {
                let phys = targets[(next() as usize) % targets.len()];
                let len = sys.mem.len_at(phys);
                let off = (next() as usize) % len;
                sys.mem.set(phys + off as u64, next() as u8);
                sys.mem.fix_checksum(phys);
            }
            walk_everything(&sys);
        }
    }
}
