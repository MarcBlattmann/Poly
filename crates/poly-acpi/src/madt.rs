//! Multiple APIC Description Table (§5.2.12): the interrupt controllers and
//! the processors they belong to.

use crate::{Error, Sdt, Table, u8_at, u16_at, u32_at, u64_at};

/// Offsets in the table (§5.2.12, Table 5.20).
const LOCAL_APIC_ADDRESS: usize = 36;
const FLAGS: usize = 40;
const ENTRIES: usize = 44;

/// `Flags` bit 0: the system also has dual 8259 PICs, which must be masked
/// before the APICs are used.
const PCAT_COMPAT: u32 = 1 << 0;

#[derive(Clone, Copy, Debug)]
pub struct Madt<'a> {
    sdt: Sdt<'a>,
}

impl<'a> Table<'a> for Madt<'a> {
    const SIGNATURE: [u8; 4] = *b"APIC";

    fn parse(sdt: Sdt<'a>) -> Result<Self, Error> {
        Ok(Madt { sdt: sdt.expect(Self::SIGNATURE, ENTRIES)? })
    }
}

impl<'a> Madt<'a> {
    pub fn sdt(&self) -> Sdt<'a> {
        self.sdt
    }

    /// The 32-bit local APIC address in the fixed part of the table.
    pub fn local_apic_address_field(&self) -> u32 {
        u32_at(self.sdt.bytes(), LOCAL_APIC_ADDRESS).unwrap_or(0)
    }

    pub fn flags(&self) -> u32 {
        u32_at(self.sdt.bytes(), FLAGS).unwrap_or(0)
    }

    /// True if legacy 8259 PICs are present as well (`PCAT_COMPAT`).
    pub fn has_8259(&self) -> bool {
        self.flags() & PCAT_COMPAT != 0
    }

    /// Physical address of every CPU's local APIC. A Local APIC Address
    /// Override entry (§5.2.12.8) replaces the 32-bit field.
    pub fn local_apic_address(&self) -> u64 {
        self.entries()
            .find_map(|e| match e {
                MadtEntry::LocalApicAddressOverride(a) => Some(a),
                _ => None,
            })
            .unwrap_or(self.local_apic_address_field().into())
    }

    /// The interrupt controller structures that follow the fixed fields.
    pub fn entries(&self) -> MadtEntries<'a> {
        MadtEntries { rest: self.sdt.bytes().get(ENTRIES..).unwrap_or(&[]) }
    }

    /// Every processor, from both Local APIC and Local x2APIC entries. The
    /// firmware decides which form each CPU gets (§5.2.12.12): IDs below 255
    /// normally use the xAPIC form.
    pub fn processors(&self) -> impl Iterator<Item = Processor> + use<'a> {
        self.entries().filter_map(|e| match e {
            MadtEntry::LocalApic(l) => Some(Processor {
                uid: l.processor_uid.into(),
                apic_id: l.apic_id.into(),
                flags: l.flags,
                x2apic: false,
            }),
            MadtEntry::LocalX2Apic(x) => {
                Some(Processor { uid: x.processor_uid, apic_id: x.x2apic_id, flags: x.flags, x2apic: true })
            }
            _ => None,
        })
    }

    pub fn io_apics(&self) -> impl Iterator<Item = IoApic> + use<'a> {
        self.entries().filter_map(|e| match e {
            MadtEntry::IoApic(i) => Some(i),
            _ => None,
        })
    }

    pub fn interrupt_overrides(&self) -> impl Iterator<Item = InterruptOverride> + use<'a> {
        self.entries().filter_map(|e| match e {
            MadtEntry::InterruptOverride(o) => Some(o),
            _ => None,
        })
    }
}

/// Iterator over the MADT's interrupt controller structures.
///
/// Each structure starts with a type byte and a length byte that covers the
/// whole structure. Unknown types are skipped by their length. A length
/// below 2 or past the end of the table ends the iteration, since nothing
/// after it can be located reliably.
#[derive(Clone, Debug)]
pub struct MadtEntries<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for MadtEntries<'a> {
    type Item = MadtEntry<'a>;

    fn next(&mut self) -> Option<MadtEntry<'a>> {
        let len = u8_at(self.rest, 1)? as usize;
        let Some(entry) = self.rest.get(..len).filter(|_| len >= 2) else {
            self.rest = &[];
            return None;
        };
        self.rest = self.rest.get(len..).unwrap_or(&[]);
        Some(MadtEntry::decode(entry))
    }
}

/// One interrupt controller structure (§5.2.12, Table 5.21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadtEntry<'a> {
    LocalApic(LocalApic),
    IoApic(IoApic),
    InterruptOverride(InterruptOverride),
    NmiSource(NmiSource),
    LocalApicNmi(LocalApicNmi),
    /// 64-bit local APIC address that supersedes the table's 32-bit field.
    LocalApicAddressOverride(u64),
    LocalX2Apic(LocalX2Apic),
    LocalX2ApicNmi(LocalX2ApicNmi),
    /// A type this crate does not decode. `bytes` is the whole structure.
    Other {
        kind: u8,
        bytes: &'a [u8],
    },
    /// A known type whose length is too short for its fields.
    Malformed {
        kind: u8,
        bytes: &'a [u8],
    },
}

impl<'a> MadtEntry<'a> {
    const LOCAL_APIC: u8 = 0;
    const IO_APIC: u8 = 1;
    const INTERRUPT_OVERRIDE: u8 = 2;
    const NMI_SOURCE: u8 = 3;
    const LOCAL_APIC_NMI: u8 = 4;
    const LOCAL_APIC_ADDRESS_OVERRIDE: u8 = 5;
    const LOCAL_X2APIC: u8 = 9;
    const LOCAL_X2APIC_NMI: u8 = 0xa;

    /// Minimum length of each decoded type; later revisions may append fields.
    fn min_len(kind: u8) -> Option<usize> {
        Some(match kind {
            Self::LOCAL_APIC => 8,
            Self::IO_APIC => 12,
            Self::INTERRUPT_OVERRIDE => 10,
            Self::NMI_SOURCE => 8,
            Self::LOCAL_APIC_NMI => 6,
            Self::LOCAL_APIC_ADDRESS_OVERRIDE => 12,
            Self::LOCAL_X2APIC => 16,
            Self::LOCAL_X2APIC_NMI => 12,
            _ => return None,
        })
    }

    /// `b` is one whole structure, at least 2 bytes long.
    fn decode(b: &'a [u8]) -> MadtEntry<'a> {
        let kind = u8_at(b, 0).unwrap_or(0xff);
        match Self::min_len(kind) {
            None => MadtEntry::Other { kind, bytes: b },
            Some(min) if b.len() < min => MadtEntry::Malformed { kind, bytes: b },
            Some(_) => Self::decode_known(kind, b).unwrap_or(MadtEntry::Malformed { kind, bytes: b }),
        }
    }

    fn decode_known(kind: u8, b: &[u8]) -> Option<MadtEntry<'a>> {
        Some(match kind {
            // §5.2.12.2
            Self::LOCAL_APIC => MadtEntry::LocalApic(LocalApic {
                processor_uid: u8_at(b, 2)?,
                apic_id: u8_at(b, 3)?,
                flags: ProcessorFlags(u32_at(b, 4)?),
            }),
            // §5.2.12.3
            Self::IO_APIC => {
                MadtEntry::IoApic(IoApic { id: u8_at(b, 2)?, address: u32_at(b, 4)?, gsi_base: u32_at(b, 8)? })
            }
            // §5.2.12.5
            Self::INTERRUPT_OVERRIDE => MadtEntry::InterruptOverride(InterruptOverride {
                bus: u8_at(b, 2)?,
                source: u8_at(b, 3)?,
                gsi: u32_at(b, 4)?,
                flags: MpsIntiFlags(u16_at(b, 8)?),
            }),
            // §5.2.12.6
            Self::NMI_SOURCE => {
                MadtEntry::NmiSource(NmiSource { flags: MpsIntiFlags(u16_at(b, 2)?), gsi: u32_at(b, 4)? })
            }
            // §5.2.12.7
            Self::LOCAL_APIC_NMI => MadtEntry::LocalApicNmi(LocalApicNmi {
                processor_uid: u8_at(b, 2)?,
                flags: MpsIntiFlags(u16_at(b, 3)?),
                lint: u8_at(b, 5)?,
            }),
            // §5.2.12.8
            Self::LOCAL_APIC_ADDRESS_OVERRIDE => MadtEntry::LocalApicAddressOverride(u64_at(b, 4)?),
            // §5.2.12.12
            Self::LOCAL_X2APIC => MadtEntry::LocalX2Apic(LocalX2Apic {
                x2apic_id: u32_at(b, 4)?,
                flags: ProcessorFlags(u32_at(b, 8)?),
                processor_uid: u32_at(b, 12)?,
            }),
            // §5.2.12.13
            Self::LOCAL_X2APIC_NMI => MadtEntry::LocalX2ApicNmi(LocalX2ApicNmi {
                flags: MpsIntiFlags(u16_at(b, 2)?),
                processor_uid: u32_at(b, 4)?,
                lint: u8_at(b, 8)?,
            }),
            _ => return None,
        })
    }
}

/// Local APIC flags (§5.2.12.2, Table 5.23), shared by the x2APIC entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessorFlags(pub u32);

impl ProcessorFlags {
    /// The processor is ready to use.
    pub fn enabled(self) -> bool {
        self.0 & 1 != 0
    }

    /// Disabled now but can be brought online later. Only meaningful when
    /// `enabled` is clear.
    pub fn online_capable(self) -> bool {
        self.0 & 2 != 0
    }
}

/// Processor Local APIC structure (§5.2.12.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalApic {
    /// Matches a processor object's `_UID` in the namespace.
    pub processor_uid: u8,
    pub apic_id: u8,
    pub flags: ProcessorFlags,
}

/// Processor Local x2APIC structure (§5.2.12.12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalX2Apic {
    pub x2apic_id: u32,
    pub flags: ProcessorFlags,
    pub processor_uid: u32,
}

/// A processor from either kind of local APIC entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Processor {
    pub uid: u32,
    pub apic_id: u32,
    pub flags: ProcessorFlags,
    /// Came from a Local x2APIC entry.
    pub x2apic: bool,
}

impl Processor {
    pub fn enabled(&self) -> bool {
        self.flags.enabled()
    }

    pub fn online_capable(&self) -> bool {
        self.flags.online_capable()
    }
}

/// I/O APIC structure (§5.2.12.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApic {
    pub id: u8,
    /// Physical address of its register window.
    pub address: u32,
    /// First global system interrupt this I/O APIC handles.
    pub gsi_base: u32,
}

/// Interrupt Source Override structure (§5.2.12.5): an ISA IRQ that is not
/// identity-mapped to a GSI, or has non-ISA polarity/trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptOverride {
    /// 0 = ISA, the only value defined.
    pub bus: u8,
    /// Bus-relative IRQ.
    pub source: u8,
    pub gsi: u32,
    pub flags: MpsIntiFlags,
}

/// Non-Maskable Interrupt Source structure (§5.2.12.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NmiSource {
    pub flags: MpsIntiFlags,
    pub gsi: u32,
}

/// Local APIC NMI structure (§5.2.12.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalApicNmi {
    /// 0xff means all processors.
    pub processor_uid: u8,
    pub flags: MpsIntiFlags,
    /// LINT input (0 or 1) the NMI is wired to.
    pub lint: u8,
}

/// Local x2APIC NMI structure (§5.2.12.13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalX2ApicNmi {
    pub flags: MpsIntiFlags,
    /// 0xffff_ffff means all processors.
    pub processor_uid: u32,
    pub lint: u8,
}

/// MPS INTI flags (§5.2.12.5, Table 5.26): polarity and trigger mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MpsIntiFlags(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    /// Whatever the bus specifies (active high for ISA).
    BusDefault,
    ActiveHigh,
    ActiveLow,
    Reserved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerMode {
    /// Whatever the bus specifies (edge for ISA).
    BusDefault,
    Edge,
    Level,
    Reserved,
}

impl MpsIntiFlags {
    pub fn polarity(self) -> Polarity {
        match self.0 & 0b11 {
            0b00 => Polarity::BusDefault,
            0b01 => Polarity::ActiveHigh,
            0b11 => Polarity::ActiveLow,
            _ => Polarity::Reserved,
        }
    }

    pub fn trigger_mode(self) -> TriggerMode {
        match (self.0 >> 2) & 0b11 {
            0b00 => TriggerMode::BusDefault,
            0b01 => TriggerMode::Edge,
            0b11 => TriggerMode::Level,
            _ => TriggerMode::Reserved,
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::testutil::*;
    use std::vec::Vec;

    fn madt(body: &[u8]) -> Vec<u8> {
        table(b"APIC", 5, body)
    }

    #[test]
    fn qemu_like_table() {
        let t = madt(&madt_qemu());
        let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
        assert_eq!(m.local_apic_address(), 0xfee0_0000);
        assert!(m.has_8259());

        let cpus: Vec<Processor> = m.processors().collect();
        assert_eq!(cpus.len(), 2);
        assert_eq!(cpus[1], Processor { uid: 1, apic_id: 1, flags: ProcessorFlags(1), x2apic: false });
        assert!(cpus.iter().all(Processor::enabled));

        assert_eq!(m.io_apics().collect::<Vec<_>>(), [IoApic { id: 0, address: 0xfec0_0000, gsi_base: 0 }]);

        let isos: Vec<InterruptOverride> = m.interrupt_overrides().collect();
        assert_eq!(isos.len(), 2);
        assert_eq!((isos[0].source, isos[0].gsi), (0, 2));
        assert_eq!(isos[0].flags.trigger_mode(), TriggerMode::BusDefault);
        assert_eq!((isos[1].source, isos[1].gsi), (9, 9));
        assert_eq!(isos[1].flags.polarity(), Polarity::ActiveHigh);
        assert_eq!(isos[1].flags.trigger_mode(), TriggerMode::Level);

        let nmis: Vec<MadtEntry> = m.entries().filter(|e| matches!(e, MadtEntry::LocalApicNmi(_))).collect();
        assert_eq!(
            nmis,
            [MadtEntry::LocalApicNmi(LocalApicNmi { processor_uid: 0xff, flags: MpsIntiFlags(0), lint: 1 })]
        );
    }

    #[test]
    fn every_entry_type() {
        let t = madt(&madt_full());
        let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
        let e: Vec<MadtEntry> = m.entries().collect();
        assert_eq!(
            e,
            [
                MadtEntry::LocalApic(LocalApic { processor_uid: 0, apic_id: 0, flags: ProcessorFlags(1) }),
                MadtEntry::LocalApic(LocalApic { processor_uid: 1, apic_id: 2, flags: ProcessorFlags(0) }),
                MadtEntry::LocalApic(LocalApic { processor_uid: 2, apic_id: 4, flags: ProcessorFlags(2) }),
                MadtEntry::LocalX2Apic(LocalX2Apic {
                    x2apic_id: 0x100,
                    flags: ProcessorFlags(1),
                    processor_uid: 0x1000
                }),
                MadtEntry::IoApic(IoApic { id: 0, address: 0xfec0_0000, gsi_base: 0 }),
                MadtEntry::IoApic(IoApic { id: 1, address: 0xfec0_1000, gsi_base: 24 }),
                MadtEntry::InterruptOverride(InterruptOverride { bus: 0, source: 0, gsi: 2, flags: MpsIntiFlags(0) }),
                MadtEntry::NmiSource(NmiSource { flags: MpsIntiFlags(0b1111), gsi: 30 }),
                MadtEntry::LocalApicNmi(LocalApicNmi { processor_uid: 0xff, flags: MpsIntiFlags(0b0101), lint: 1 }),
                MadtEntry::LocalX2ApicNmi(LocalX2ApicNmi {
                    flags: MpsIntiFlags(0),
                    processor_uid: 0xffff_ffff,
                    lint: 1
                }),
                MadtEntry::Other { kind: 0x7f, bytes: &[0x7f, 4, 0xaa, 0xbb] },
                MadtEntry::LocalApicAddressOverride(0x1_fee0_0000),
            ]
        );
        assert_eq!(m.local_apic_address(), 0x1_fee0_0000);
        assert_eq!(m.local_apic_address_field(), 0xfee0_0000);
        assert!(!m.has_8259());

        let cpus: Vec<Processor> = m.processors().collect();
        assert_eq!(cpus.len(), 4);
        assert_eq!(cpus.iter().filter(|c| c.enabled()).count(), 2);
        assert_eq!(cpus.iter().filter(|c| !c.enabled() && c.online_capable()).count(), 1);
        assert_eq!(cpus[3], Processor { uid: 0x1000, apic_id: 0x100, flags: ProcessorFlags(1), x2apic: true });
        assert_eq!(m.io_apics().map(|i| i.gsi_base).collect::<Vec<_>>(), [0, 24]);
    }

    #[test]
    fn longer_entries_from_newer_revisions_still_decode() {
        let mut e = lapic(3, 7, 1);
        e.extend_from_slice(&[0, 0, 0, 0]);
        e[1] = 12;
        let t = madt(&madt_body(0xfee0_0000, 0, &[e, ioapic(0, 0xfec0_0000, 0)]));
        let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
        assert_eq!(m.processors().map(|p| p.apic_id).collect::<Vec<_>>(), [7]);
        assert_eq!(m.io_apics().count(), 1);
    }

    #[test]
    fn short_known_entry_is_malformed_and_skipped() {
        let short = std::vec![MadtEntry::IO_APIC, 6, 0, 0, 0, 0];
        let t = madt(&madt_body(0xfee0_0000, 0, &[short, lapic(0, 0, 1)]));
        let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
        let e: Vec<MadtEntry> = m.entries().collect();
        assert!(matches!(e[0], MadtEntry::Malformed { kind: 1, .. }));
        assert_eq!(m.processors().count(), 1);
        assert_eq!(m.io_apics().count(), 0);
    }

    #[test]
    fn zero_or_one_length_entry_stops() {
        for bad_len in [0u8, 1] {
            let t = madt(&madt_body(0xfee0_0000, 0, &[lapic(0, 0, 1), std::vec![0x7f, bad_len], lapic(1, 1, 1)]));
            let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
            assert_eq!(m.entries().count(), 1);
        }
    }

    #[test]
    fn entry_running_past_the_table_stops() {
        let mut overrun = lapic(1, 1, 1);
        overrun[1] = 200;
        let t = madt(&madt_body(0xfee0_0000, 0, &[lapic(0, 0, 1), overrun]));
        let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
        assert_eq!(m.entries().count(), 1);

        // A single trailing byte cannot hold an entry header.
        let t = madt(&madt_body(0xfee0_0000, 0, &[lapic(0, 0, 1), std::vec![0]]));
        let m = Madt::parse(Sdt::parse(&t).unwrap()).unwrap();
        assert_eq!(m.entries().count(), 1);
    }

    #[test]
    fn table_too_short_or_wrong() {
        let t = table(b"APIC", 5, &[0; 7]);
        assert_eq!(Madt::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::Truncated));
        let t = table(b"APIX", 5, &madt_qemu());
        assert_eq!(Madt::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::BadSignature));
        // No entries at all is valid.
        let t = table(b"APIC", 5, &[0; 8]);
        assert_eq!(Madt::parse(Sdt::parse(&t).unwrap()).unwrap().entries().count(), 0);
    }

    #[test]
    fn mps_flags() {
        assert_eq!(MpsIntiFlags(0b1111).polarity(), Polarity::ActiveLow);
        assert_eq!(MpsIntiFlags(0b1111).trigger_mode(), TriggerMode::Level);
        assert_eq!(MpsIntiFlags(0b0110).polarity(), Polarity::Reserved);
        assert_eq!(MpsIntiFlags(0b0110).trigger_mode(), TriggerMode::Edge);
        assert_eq!(MpsIntiFlags(0b1000).trigger_mode(), TriggerMode::Reserved);
    }

    #[test]
    fn prefixes_never_panic() {
        for body in [madt_qemu(), madt_full()] {
            for_each_prefix(&table(b"APIC", 5, &body), |sdt| {
                if let Ok(m) = Madt::parse(sdt) {
                    walk_madt(&m);
                }
            });
        }
    }
}
