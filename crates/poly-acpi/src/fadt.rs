//! Fixed ACPI Description Table (§5.2.9), signature "FACP".
//!
//! The FADT has grown with every ACPI revision; a table is only as long as
//! the revision its firmware implements. Fields every revision has (the
//! first 116 bytes, ACPI 1.0) are plain values; later fields are `Option`s
//! that are `None` when the table ends before them.

use crate::{AddressSpace, Error, GenericAddress, Sdt, Table, u8_at, u16_at, u32_at, u64_at};

/// Field offsets (§5.2.9, Table 5.9).
pub(crate) const FIRMWARE_CTRL: usize = 36;
pub(crate) const DSDT: usize = 40;
pub(crate) const PREFERRED_PM_PROFILE: usize = 45;
pub(crate) const SCI_INT: usize = 46;
pub(crate) const SMI_CMD: usize = 48;
pub(crate) const ACPI_ENABLE: usize = 52;
pub(crate) const PM1A_CNT_BLK: usize = 64;
pub(crate) const PM1B_CNT_BLK: usize = 68;
pub(crate) const PM1_CNT_LEN: usize = 89;
pub(crate) const PM_TMR_BLK: usize = 76;
pub(crate) const PM_TMR_LEN: usize = 91;
pub(crate) const CENTURY: usize = 108;
pub(crate) const IAPC_BOOT_ARCH: usize = 109;
pub(crate) const FLAGS: usize = 112;
pub(crate) const RESET_REG: usize = 116;
pub(crate) const RESET_VALUE: usize = 128;
pub(crate) const FADT_MINOR_VERSION: usize = 131;
pub(crate) const X_FIRMWARE_CTRL: usize = 132;
pub(crate) const X_DSDT: usize = 140;
pub(crate) const X_PM1A_CNT_BLK: usize = 172;
pub(crate) const X_PM1B_CNT_BLK: usize = 184;
pub(crate) const X_PM_TMR_BLK: usize = 208;

/// Length of the ACPI 1.0 FADT, the shortest one accepted.
const V1_LEN: usize = 116;

#[derive(Clone, Copy, Debug)]
pub struct Fadt<'a> {
    sdt: Sdt<'a>,
}

impl<'a> Table<'a> for Fadt<'a> {
    const SIGNATURE: [u8; 4] = *b"FACP";

    fn parse(sdt: Sdt<'a>) -> Result<Self, Error> {
        Ok(Fadt { sdt: sdt.expect(Self::SIGNATURE, V1_LEN)? })
    }
}

impl<'a> Fadt<'a> {
    pub fn sdt(&self) -> Sdt<'a> {
        self.sdt
    }

    fn b(&self) -> &'a [u8] {
        self.sdt.bytes()
    }

    fn gas(&self, off: usize) -> Option<GenericAddress> {
        self.b().get(off..).and_then(GenericAddress::parse)
    }

    /// A 64-bit `X_` address if the table has it and it is nonzero,
    /// otherwise the 32-bit legacy one if nonzero. §5.2.9 says the `X_`
    /// field wins when set.
    fn address(&self, x_off: usize, legacy_off: usize) -> Option<u64> {
        u64_at(self.b(), x_off)
            .filter(|&a| a != 0)
            .or_else(|| u32_at(self.b(), legacy_off).filter(|&a| a != 0).map(u64::from))
    }

    /// Physical address of the DSDT.
    pub fn dsdt_address(&self) -> Option<u64> {
        self.address(X_DSDT, DSDT)
    }

    /// Physical address of the FACS (firmware control structure).
    pub fn facs_address(&self) -> Option<u64> {
        self.address(X_FIRMWARE_CTRL, FIRMWARE_CTRL)
    }

    /// Preferred power management profile (0 unspecified, 1 desktop,
    /// 2 mobile, ...; Table 5.9).
    pub fn preferred_pm_profile(&self) -> u8 {
        u8_at(self.b(), PREFERRED_PM_PROFILE).unwrap_or(0)
    }

    /// The system vector the SCI interrupt is wired to, in 8259 mode; with
    /// APICs it is a GSI, possibly remapped by an interrupt source override.
    pub fn sci_interrupt(&self) -> u16 {
        u16_at(self.b(), SCI_INT).unwrap_or(0)
    }

    /// I/O port of the SMI command register; 0 if the machine has no SMM
    /// mode transition (already in ACPI mode, or hardware-reduced).
    pub fn smi_command_port(&self) -> u32 {
        u32_at(self.b(), SMI_CMD).unwrap_or(0)
    }

    /// The value to write to [`smi_command_port`](Self::smi_command_port)
    /// to switch the machine into ACPI mode (§4.8.2.1); 0 if it needs none.
    pub fn acpi_enable(&self) -> u8 {
        u8_at(self.b(), ACPI_ENABLE).unwrap_or(0)
    }

    /// The PM1a control register block (`SLP_TYP`, `SLP_EN`, `SCI_EN`;
    /// §4.8.3.2.1), if the platform has one.
    pub fn pm1a_control(&self) -> Option<GenericAddress> {
        self.pm1_control(X_PM1A_CNT_BLK, PM1A_CNT_BLK)
    }

    /// The optional PM1b control register block, written together with
    /// PM1a's.
    pub fn pm1b_control(&self) -> Option<GenericAddress> {
        self.pm1_control(X_PM1B_CNT_BLK, PM1B_CNT_BLK)
    }

    fn pm1_control(&self, x_off: usize, legacy_off: usize) -> Option<GenericAddress> {
        if let Some(block) = self.gas(x_off).filter(|g| g.address != 0) {
            return Some(block);
        }
        // The 32-bit field is an I/O port; PM1_CNT_LEN is at least 2.
        let port = u32_at(self.b(), legacy_off).filter(|&p| p != 0)?;
        if u8_at(self.b(), PM1_CNT_LEN).is_none_or(|len| len < 2) {
            return None;
        }
        Some(GenericAddress {
            space: AddressSpace::SystemIo,
            bit_width: 16,
            bit_offset: 0,
            access_size: 2,
            address: port.into(),
        })
    }

    /// The ACPI PM timer, a 3.579545 MHz counter, if the platform has one.
    pub fn pm_timer(&self) -> Option<PmTimer> {
        let bits = if self.flags().contains(FadtFlags::TMR_VAL_EXT) { 32 } else { 24 };
        if let Some(block) = self.gas(X_PM_TMR_BLK).filter(|g| g.address != 0) {
            return Some(PmTimer { block, bits });
        }
        // The 32-bit field is an I/O port; PM_TMR_LEN is 4 when it exists.
        let port = u32_at(self.b(), PM_TMR_BLK).filter(|&p| p != 0)?;
        if u8_at(self.b(), PM_TMR_LEN) != Some(4) {
            return None;
        }
        let block = GenericAddress {
            space: AddressSpace::SystemIo,
            bit_width: 32,
            bit_offset: 0,
            access_size: 3,
            address: port.into(),
        };
        Some(PmTimer { block, bits })
    }

    /// Index of the century byte in CMOS RAM; 0 if the RTC has none.
    pub fn century(&self) -> u8 {
        u8_at(self.b(), CENTURY).unwrap_or(0)
    }

    /// IA-PC boot architecture flags. The field is reserved, and reads as
    /// zero, in ACPI 1.0 tables (revision 1).
    pub fn boot_arch(&self) -> IapcBootArch {
        IapcBootArch(u16_at(self.b(), IAPC_BOOT_ARCH).unwrap_or(0))
    }

    pub fn flags(&self) -> FadtFlags {
        FadtFlags(u32_at(self.b(), FLAGS).unwrap_or(0))
    }

    /// The reset register and the value to write to it, if the table has
    /// one and `RESET_REG_SUP` says it works.
    pub fn reset(&self) -> Option<ResetRegister> {
        if !self.flags().contains(FadtFlags::RESET_REG_SUP) {
            return None;
        }
        let register = self.gas(RESET_REG).filter(|g| g.address != 0)?;
        Some(ResetRegister { register, value: u8_at(self.b(), RESET_VALUE)? })
    }

    /// FADT minor version (ACPI 5.1 and later; `None` before).
    pub fn minor_version(&self) -> Option<u8> {
        u8_at(self.b(), FADT_MINOR_VERSION).map(|v| v & 0xf)
    }

    /// Hardware-reduced ACPI: no fixed hardware (PM timer, SCI, PM1 blocks).
    pub fn hardware_reduced(&self) -> bool {
        self.flags().contains(FadtFlags::HW_REDUCED_ACPI)
    }
}

/// The ACPI PM timer register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PmTimer {
    pub block: GenericAddress,
    /// Counter width: 24, or 32 with `TMR_VAL_EXT`.
    pub bits: u8,
}

/// Reset register (§4.8.3.6) and the value that resets the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetRegister {
    pub register: GenericAddress,
    pub value: u8,
}

/// FADT fixed feature flags (§5.2.9, Table 5.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FadtFlags(pub u32);

impl FadtFlags {
    pub const WBINVD: u32 = 1 << 0;
    pub const PWR_BUTTON: u32 = 1 << 4;
    pub const SLP_BUTTON: u32 = 1 << 5;
    pub const FIX_RTC: u32 = 1 << 6;
    pub const TMR_VAL_EXT: u32 = 1 << 8;
    pub const RESET_REG_SUP: u32 = 1 << 10;
    pub const HEADLESS: u32 = 1 << 12;
    pub const FORCE_APIC_CLUSTER_MODEL: u32 = 1 << 18;
    pub const FORCE_APIC_PHYSICAL_DESTINATION_MODE: u32 = 1 << 19;
    pub const HW_REDUCED_ACPI: u32 = 1 << 20;
    pub const LOW_POWER_S0_IDLE_CAPABLE: u32 = 1 << 21;

    pub fn contains(self, flag: u32) -> bool {
        self.0 & flag == flag
    }
}

/// IA-PC boot architecture flags (§5.2.9.3, Table 5.11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IapcBootArch(pub u16);

impl IapcBootArch {
    /// The motherboard has user-visible devices on the LPC or ISA bus.
    /// Clear means "no legacy devices".
    pub fn legacy_devices(self) -> bool {
        self.0 & (1 << 0) != 0
    }

    /// An 8042-compatible keyboard controller is present.
    pub fn has_8042(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    /// VGA hardware must not be probed.
    pub fn vga_not_present(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    /// Message signalled interrupts must not be enabled.
    pub fn msi_not_supported(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    /// The OS must not change PCIe ASPM settings.
    pub fn pcie_aspm_controls(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// There is no CMOS RTC at the legacy ports.
    pub fn cmos_rtc_not_present(self) -> bool {
        self.0 & (1 << 5) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn fadt(t: &[u8]) -> Fadt<'_> {
        Fadt::parse(Sdt::parse(t).unwrap()).unwrap()
    }

    #[test]
    fn full_table() {
        let t = fadt_table(0x10_0000, FADT_FULL_LEN);
        let f = fadt(&t);
        assert_eq!(f.dsdt_address(), Some(0x10_0000));
        assert_eq!(f.facs_address(), None);
        assert_eq!(f.sci_interrupt(), 9);
        assert_eq!(f.smi_command_port(), 0xb2);
        assert_eq!(f.preferred_pm_profile(), 0);
        assert_eq!(f.century(), 0x32);
        assert!(f.boot_arch().has_8042());
        assert!(f.boot_arch().legacy_devices());
        assert!(!f.boot_arch().vga_not_present());
        assert!(!f.boot_arch().msi_not_supported());
        assert!(!f.boot_arch().pcie_aspm_controls());
        assert!(!f.boot_arch().cmos_rtc_not_present());
        assert!(!f.hardware_reduced());
        assert_eq!(f.minor_version(), Some(5));

        let tmr = f.pm_timer().unwrap();
        assert_eq!(tmr.block.space, AddressSpace::SystemIo);
        assert_eq!(tmr.block.address, 0x608);
        assert_eq!(tmr.bits, 24);

        let reset = f.reset().unwrap();
        assert_eq!(reset.register.space, AddressSpace::SystemIo);
        assert_eq!(reset.register.address, 0xcf9);
        assert_eq!(reset.value, 0x06);

        let pm1a = f.pm1a_control().unwrap();
        assert_eq!((pm1a.space, pm1a.address, pm1a.bit_width), (AddressSpace::SystemIo, 0x604, 16));
        assert_eq!(f.pm1b_control(), None);
        assert_eq!(f.acpi_enable(), 0xf1);
    }

    #[test]
    fn pm1_control_blocks() {
        let mut t = fadt_table(0x10_0000, FADT_FULL_LEN);
        put(&mut t, X_PM1B_CNT_BLK, &[1, 16, 0, 2]);
        put(&mut t, X_PM1B_CNT_BLK + 4, &0x1604u64.to_le_bytes());
        fix_checksum(&mut t);
        assert_eq!(fadt(&t).pm1b_control().unwrap().address, 0x1604);
        // A legacy block shorter than two bytes is not usable.
        let mut t = fadt_table(0x10_0000, 116);
        put(&mut t, PM1_CNT_LEN, &[1]);
        fix_checksum(&mut t);
        assert_eq!(fadt(&t).pm1a_control(), None);
    }

    #[test]
    fn x_fields_win_over_legacy_ones() {
        let mut t = fadt_table(0x10_0000, FADT_FULL_LEN);
        put(&mut t, X_DSDT, &0x2_0000_0000u64.to_le_bytes());
        put(&mut t, X_FIRMWARE_CTRL, &0x3000u64.to_le_bytes());
        put(&mut t, X_PM_TMR_BLK + 4, &0x1608u64.to_le_bytes());
        put(&mut t, FLAGS, &(FadtFlags::RESET_REG_SUP | FadtFlags::TMR_VAL_EXT).to_le_bytes());
        fix_checksum(&mut t);
        let f = fadt(&t);
        assert_eq!(f.dsdt_address(), Some(0x2_0000_0000));
        assert_eq!(f.facs_address(), Some(0x3000));
        let tmr = f.pm_timer().unwrap();
        assert_eq!((tmr.block.address, tmr.bits), (0x1608, 32));
    }

    #[test]
    fn acpi_1_table() {
        let t = fadt_table(0x10_0000, 116);
        let f = fadt(&t);
        assert_eq!(f.sdt().bytes().len(), 116);
        assert_eq!(f.dsdt_address(), Some(0x10_0000));
        assert_eq!(f.reset(), None); // RESET_REG_SUP is set, but the register is not in the table
        assert_eq!(f.minor_version(), None);
        // The legacy PM timer port still works.
        assert_eq!(f.pm_timer().unwrap().block.address, 0x608);
    }

    #[test]
    fn missing_features() {
        let mut t = fadt_table(0x10_0000, FADT_FULL_LEN);
        put(&mut t, FLAGS, &FadtFlags::HW_REDUCED_ACPI.to_le_bytes());
        put(&mut t, PM_TMR_BLK, &0u32.to_le_bytes());
        put(&mut t, X_PM_TMR_BLK + 4, &0u64.to_le_bytes());
        put(&mut t, DSDT, &0u32.to_le_bytes());
        put(&mut t, X_DSDT, &0u64.to_le_bytes());
        fix_checksum(&mut t);
        let f = fadt(&t);
        assert!(f.hardware_reduced());
        assert_eq!(f.reset(), None);
        assert_eq!(f.pm_timer(), None);
        assert_eq!(f.dsdt_address(), None);

        // A legacy timer port with PM_TMR_LEN != 4 is not a timer.
        let mut t = fadt_table(0x10_0000, 116);
        put(&mut t, PM_TMR_LEN, &[0]);
        fix_checksum(&mut t);
        assert_eq!(fadt(&t).pm_timer(), None);
    }

    #[test]
    fn too_short_or_wrong() {
        let t = fadt_table(0x10_0000, 115);
        assert_eq!(Fadt::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::Truncated));
        let mut t = fadt_table(0x10_0000, FADT_FULL_LEN);
        put(&mut t, 0, b"FACX");
        fix_checksum(&mut t);
        assert_eq!(Fadt::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::BadSignature));
    }

    #[test]
    fn prefixes_never_panic() {
        for_each_prefix(&fadt_table(0x10_0000, FADT_FULL_LEN), |sdt| {
            if let Ok(f) = Fadt::parse(sdt) {
                walk_fadt(&f);
            }
        });
    }
}
