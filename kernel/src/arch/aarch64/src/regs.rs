//! Decoding of system register values, from the Arm Architecture Reference
//! Manual for A-profile (DDI 0487): `CurrentEL`, `MIDR_EL1`, `ESR_EL1` and the
//! exception vector table's 16 entries.

/// The exception level in a `CurrentEL` value (bits 3:2).
pub const fn el_from_current_el(current_el: u64) -> u64 {
    (current_el >> 2) & 0b11
}

/// The fields of `MIDR_EL1`, the main ID register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Midr {
    pub implementer: u8,
    pub variant: u8,
    pub architecture: u8,
    pub part: u16,
    pub revision: u8,
}

impl Midr {
    pub const fn decode(midr: u64) -> Midr {
        Midr {
            implementer: (midr >> 24) as u8,
            variant: ((midr >> 20) & 0xf) as u8,
            architecture: ((midr >> 16) & 0xf) as u8,
            part: ((midr >> 4) & 0xfff) as u16,
            revision: (midr & 0xf) as u8,
        }
    }

    /// The implementer's name, for the few that matter here.
    pub const fn implementer_name(&self) -> &'static str {
        match self.implementer {
            0x41 => "Arm",
            0x61 => "Apple",
            0x51 => "Qualcomm",
            _ => "unknown implementer",
        }
    }

    /// The Arm core's name, for the cores QEMU models and the common ones.
    pub const fn part_name(&self) -> &'static str {
        match (self.implementer, self.part) {
            (0x41, 0xd03) => "Cortex-A53",
            (0x41, 0xd07) => "Cortex-A57",
            (0x41, 0xd08) => "Cortex-A72",
            (0x41, 0xd0b) => "Cortex-A76",
            (0x41, 0xd0c) => "Neoverse-N1",
            _ => "unknown part",
        }
    }
}

/// The exception class of an `ESR_ELx` value (bits 31:26).
pub const fn esr_class(esr: u64) -> u64 {
    (esr >> 26) & 0x3f
}

/// The instruction specific syndrome of an `ESR_ELx` value (bits 24:0).
pub const fn esr_iss(esr: u64) -> u64 {
    esr & 0x1ff_ffff
}

/// Exception classes (ESR_ELx.EC), the ones the stub meets.
pub mod ec {
    pub const UNKNOWN: u64 = 0x00;
    pub const SVC_AARCH64: u64 = 0x15;
    pub const HVC_AARCH64: u64 = 0x16;
    pub const SMC_AARCH64: u64 = 0x17;
    pub const INSTRUCTION_ABORT_LOWER: u64 = 0x20;
    pub const INSTRUCTION_ABORT_SAME: u64 = 0x21;
    pub const PC_ALIGNMENT: u64 = 0x22;
    pub const DATA_ABORT_LOWER: u64 = 0x24;
    pub const DATA_ABORT_SAME: u64 = 0x25;
    pub const SP_ALIGNMENT: u64 = 0x26;
    pub const SERROR: u64 = 0x2f;
    pub const BRK_AARCH64: u64 = 0x3c;
}

/// A name for an exception class.
pub const fn class_name(class: u64) -> &'static str {
    match class {
        ec::UNKNOWN => "unknown reason",
        0x01 => "trapped WFI/WFE",
        0x07 => "trapped FP/SIMD access",
        ec::SVC_AARCH64 => "SVC (AArch64)",
        ec::HVC_AARCH64 => "HVC (AArch64)",
        ec::SMC_AARCH64 => "SMC (AArch64)",
        0x18 => "trapped system register access",
        ec::INSTRUCTION_ABORT_LOWER => "instruction abort from a lower EL",
        ec::INSTRUCTION_ABORT_SAME => "instruction abort",
        ec::PC_ALIGNMENT => "PC alignment fault",
        ec::DATA_ABORT_LOWER => "data abort from a lower EL",
        ec::DATA_ABORT_SAME => "data abort",
        ec::SP_ALIGNMENT => "SP alignment fault",
        0x2c => "floating-point exception",
        ec::SERROR => "SError",
        0x30..=0x35 => "debug exception",
        ec::BRK_AARCH64 => "BRK (AArch64)",
        _ => "other",
    }
}

/// The fault status code (DFSC/IFSC, ISS bits 5:0) of a data or instruction
/// abort, named.
pub const fn fault_status_name(fsc: u64) -> &'static str {
    match fsc & 0x3f {
        0x00..=0x03 => "address size fault",
        0x04..=0x07 => "translation fault",
        0x09..=0x0b => "access flag fault",
        0x0d..=0x0f => "permission fault",
        0x10 => "synchronous external abort",
        0x21 => "alignment fault",
        _ => "other fault",
    }
}

/// Whether a fault status code is a translation fault (no valid table entry).
pub const fn is_translation_fault(fsc: u64) -> bool {
    matches!(fsc & 0x3f, 0x04..=0x07)
}

/// Which of the vector table's four groups entry `id` (0..16) is in.
pub const fn vector_source(id: u64) -> &'static str {
    match id / 4 {
        0 => "current EL, SP_EL0",
        1 => "current EL, SP_ELx",
        2 => "lower EL, AArch64",
        _ => "lower EL, AArch32",
    }
}

/// The kind of exception of vector table entry `id` (0..16).
pub const fn vector_kind(id: u64) -> &'static str {
    match id % 4 {
        0 => "synchronous",
        1 => "IRQ",
        2 => "FIQ",
        _ => "SError",
    }
}

/// The vector table's `id`: entry 4 * group + kind.
pub const fn vector_is_synchronous(id: u64) -> bool {
    id.is_multiple_of(4)
}

/// `ID_AA64MMFR0_EL1.PARange` (bits 3:0): the physical address size, as the
/// `TCR_EL1.IPS` encoding limited to what 4 KiB pages use (48 bits).
pub const fn ips_for(id_aa64mmfr0: u64) -> u64 {
    let parange = id_aa64mmfr0 & 0xf;
    if parange > 0b101 { 0b101 } else { parange }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_el() {
        assert_eq!(el_from_current_el(0b0100), 1);
        assert_eq!(el_from_current_el(0b1000), 2);
        assert_eq!(el_from_current_el(0b1100), 3);
    }

    /// 0x410fd083: Arm, variant 0, architecture "defined by ID registers"
    /// (0xf), Cortex-A72 (0xd08), revision 3: the value QEMU's model reports.
    #[test]
    fn midr_fields() {
        let m = Midr::decode(0x410f_d083);
        assert_eq!(m, Midr { implementer: 0x41, variant: 0, architecture: 0xf, part: 0xd08, revision: 3 });
        assert_eq!((m.implementer_name(), m.part_name()), ("Arm", "Cortex-A72"));
        assert_eq!(Midr::decode(0x611f_0000).implementer_name(), "Apple");
        assert_eq!(Midr::decode(0).part_name(), "unknown part");
    }

    #[test]
    fn esr_fields() {
        // BRK #0x42: EC 0x3c, IL set, ISS = the immediate.
        let esr = (0x3c << 26) | (1 << 25) | 0x42;
        assert_eq!(esr_class(esr), ec::BRK_AARCH64);
        assert_eq!(esr_iss(esr), 0x42);
        assert_eq!(class_name(ec::BRK_AARCH64), "BRK (AArch64)");
        assert_eq!(class_name(0x3e), "other");
    }

    #[test]
    fn fault_codes() {
        assert!(is_translation_fault(0b000101));
        assert!(!is_translation_fault(0b001101));
        assert_eq!(fault_status_name(0b000101), "translation fault");
        assert_eq!(fault_status_name(0b001101), "permission fault");
        assert_eq!(fault_status_name(0x21), "alignment fault");
        // Bits above the 6-bit code (the data abort's other ISS fields) are ignored.
        assert!(is_translation_fault(0x1_0045));
    }

    #[test]
    fn vector_names() {
        assert_eq!((vector_source(4), vector_kind(4)), ("current EL, SP_ELx", "synchronous"));
        assert_eq!((vector_source(11), vector_kind(11)), ("lower EL, AArch64", "SError"));
        assert!(vector_is_synchronous(8) && !vector_is_synchronous(9));
    }

    #[test]
    fn ips_is_parange_capped_at_48_bits() {
        assert_eq!(ips_for(0x0), 0);
        assert_eq!(ips_for(0x2), 2);
        assert_eq!(ips_for(0x6), 5);
        assert_eq!(ips_for(0xf1), 1);
    }
}
