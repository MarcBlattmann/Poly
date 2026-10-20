//! PCI Express memory-mapped configuration space table, defined in the PCI
//! Firmware Specification 3.2, §4.1.2 "MCFG Table Description".

use crate::{Error, Sdt, Table, u8_at, u16_at, u64_at};

/// Eight reserved bytes follow the header; allocation structures follow them.
const ALLOCATIONS: usize = 44;
const ALLOCATION_LEN: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct Mcfg<'a> {
    sdt: Sdt<'a>,
}

impl<'a> Table<'a> for Mcfg<'a> {
    const SIGNATURE: [u8; 4] = *b"MCFG";

    fn parse(sdt: Sdt<'a>) -> Result<Self, Error> {
        Ok(Mcfg { sdt: sdt.expect(Self::SIGNATURE, ALLOCATIONS)? })
    }
}

impl<'a> Mcfg<'a> {
    pub fn sdt(&self) -> Sdt<'a> {
        self.sdt
    }

    /// The ECAM windows, one per PCI segment group and bus range. A trailing
    /// partial structure is ignored.
    pub fn regions(&self) -> impl Iterator<Item = EcamRegion> + use<'a> {
        self.sdt.bytes().get(ALLOCATIONS..).unwrap_or(&[]).chunks_exact(ALLOCATION_LEN).filter_map(|e| {
            Some(EcamRegion {
                base: u64_at(e, 0)?,
                segment: u16_at(e, 8)?,
                start_bus: u8_at(e, 10)?,
                end_bus: u8_at(e, 11)?,
            })
        })
    }
}

/// One memory-mapped configuration space base address allocation structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcamRegion {
    /// Base address of the enhanced configuration access mechanism window.
    pub base: u64,
    /// PCI segment group number.
    pub segment: u16,
    /// First PCI bus decoded by this window.
    pub start_bus: u8,
    /// Last PCI bus decoded by this window.
    pub end_bus: u8,
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::testutil::*;
    use std::vec::Vec;

    #[test]
    fn regions() {
        let t = table(b"MCFG", 1, &mcfg_body(&[(0xb000_0000, 0, 0, 255), (0x8_0000_0000, 1, 0x10, 0x1f)]));
        let m = Mcfg::parse(Sdt::parse(&t).unwrap()).unwrap();
        assert_eq!(
            m.regions().collect::<Vec<_>>(),
            [
                EcamRegion { base: 0xb000_0000, segment: 0, start_bus: 0, end_bus: 255 },
                EcamRegion { base: 0x8_0000_0000, segment: 1, start_bus: 0x10, end_bus: 0x1f },
            ]
        );
    }

    #[test]
    fn empty_and_partial() {
        let t = table(b"MCFG", 1, &mcfg_body(&[]));
        assert_eq!(Mcfg::parse(Sdt::parse(&t).unwrap()).unwrap().regions().count(), 0);

        let mut body = mcfg_body(&[(0xb000_0000, 0, 0, 255)]);
        body.extend_from_slice(&[0xff; 15]);
        let t = table(b"MCFG", 1, &body);
        assert_eq!(Mcfg::parse(Sdt::parse(&t).unwrap()).unwrap().regions().count(), 1);

        let t = table(b"MCFG", 1, &[0; 7]);
        assert_eq!(Mcfg::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::Truncated));
    }

    #[test]
    fn prefixes_never_panic() {
        for_each_prefix(&table(b"MCFG", 1, &mcfg_body(&[(0xb000_0000, 0, 0, 255), (0xc000_0000, 1, 0, 7)])), |sdt| {
            if let Ok(m) = Mcfg::parse(sdt) {
                walk_mcfg(&m);
            }
        });
    }
}
