//! `IsProcessorFeaturePresent` (ADR 0087): which `PF_*` features the
//! processor has, from the `CPUID` leaves and `XCR0` (Intel SDM, Vol. 2A,
//! "CPUID"; Vol. 1, "Detection of XSAVE / AVX support").
//!
//! The feature numbers are the `PF_*` values Learn's page lists (and
//! `winnt.h`). Only the x86 ones the page lists can be present; the ARM
//! ones, and the values Learn does not list, are absent. Decisions:
//!
//! - `PF_PAE_ENABLED`: every x64 processor runs with PAE paging.
//! - `PF_NX_ENABLED`: the processor's no-execute bit (Poly turns it on when
//!   the processor has it).
//! - `PF_FASTFAIL_AVAILABLE`: Poly ends a process on `int 0x29` (ADR 0079's
//!   fail-fast).
//! - `PF_XSAVE_ENABLED`, `PF_AVX*`: the processor has them *and* the OS has
//!   enabled the state (`OSXSAVE`, `XCR0` bits 1-2 for AVX, 5-7 too for
//!   AVX-512), as the SDM says software must check.
//! - `PF_SECOND_LEVEL_ADDRESS_TRANSLATION`, `PF_VIRT_FIRMWARE_ENABLED`,
//!   `PF_RDWRFSGSBASE_AVAILABLE` (Poly does not set `CR4.FSGSBASE`),
//!   `PF_CHANNELS_ENABLED`, `PF_FLOATING_POINT_*`: absent.

/// `PF_*` (`winnt.h`; the values Learn's page lists).
pub const PF_FLOATING_POINT_PRECISION_ERRATA: u32 = 0;
pub const PF_FLOATING_POINT_EMULATED: u32 = 1;
pub const PF_COMPARE_EXCHANGE_DOUBLE: u32 = 2;
pub const PF_MMX_INSTRUCTIONS_AVAILABLE: u32 = 3;
pub const PF_XMMI_INSTRUCTIONS_AVAILABLE: u32 = 6;
pub const PF_3DNOW_INSTRUCTIONS_AVAILABLE: u32 = 7;
pub const PF_RDTSC_INSTRUCTION_AVAILABLE: u32 = 8;
pub const PF_PAE_ENABLED: u32 = 9;
pub const PF_XMMI64_INSTRUCTIONS_AVAILABLE: u32 = 10;
pub const PF_NX_ENABLED: u32 = 12;
pub const PF_SSE3_INSTRUCTIONS_AVAILABLE: u32 = 13;
pub const PF_COMPARE_EXCHANGE128: u32 = 14;
pub const PF_COMPARE64_EXCHANGE128: u32 = 15;
pub const PF_CHANNELS_ENABLED: u32 = 16;
pub const PF_XSAVE_ENABLED: u32 = 17;
pub const PF_SECOND_LEVEL_ADDRESS_TRANSLATION: u32 = 20;
pub const PF_VIRT_FIRMWARE_ENABLED: u32 = 21;
pub const PF_RDWRFSGSBASE_AVAILABLE: u32 = 22;
pub const PF_FASTFAIL_AVAILABLE: u32 = 23;
pub const PF_SSSE3_INSTRUCTIONS_AVAILABLE: u32 = 36;
pub const PF_SSE4_1_INSTRUCTIONS_AVAILABLE: u32 = 37;
pub const PF_SSE4_2_INSTRUCTIONS_AVAILABLE: u32 = 38;
pub const PF_AVX_INSTRUCTIONS_AVAILABLE: u32 = 39;
pub const PF_AVX2_INSTRUCTIONS_AVAILABLE: u32 = 40;
pub const PF_AVX512F_INSTRUCTIONS_AVAILABLE: u32 = 41;
pub const PF_BMI2_INSTRUCTIONS_AVAILABLE: u32 = 60;
pub const PF_MOVDIR64B_INSTRUCTION_AVAILABLE: u32 = 61;
pub const PF_UMONITOR_INSTRUCTION_AVAILABLE: u32 = 89;

/// What the processor reports: `CPUID` leaf 1 (`ECX`, `EDX`), leaf 7
/// sub-leaf 0 (`EBX`, `ECX`) when the maximum leaf reaches it, leaf
/// 0x8000_0001 (`ECX`, `EDX`) when the maximum extended leaf reaches it,
/// and `XCR0` (`XGETBV(0)`) when `OSXSAVE` is set (else 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuId {
    pub leaf1_ecx: u32,
    pub leaf1_edx: u32,
    pub leaf7_ebx: u32,
    pub leaf7_ecx: u32,
    pub ext1_ecx: u32,
    pub ext1_edx: u32,
    pub xcr0: u64,
}

fn bit(v: u32, n: u32) -> bool {
    v >> n & 1 != 0
}

impl CpuId {
    /// `OSXSAVE` and the OS saves the SSE and AVX state.
    fn avx_state(&self) -> bool {
        bit(self.leaf1_ecx, 27) && self.xcr0 & 0b110 == 0b110
    }

    /// `IsProcessorFeaturePresent(feature)`.
    pub fn has(&self, feature: u32) -> bool {
        let (c1, d1, b7, c7, de) = (self.leaf1_ecx, self.leaf1_edx, self.leaf7_ebx, self.leaf7_ecx, self.ext1_edx);
        match feature {
            PF_COMPARE_EXCHANGE_DOUBLE => bit(d1, 8),
            PF_MMX_INSTRUCTIONS_AVAILABLE => bit(d1, 23),
            PF_XMMI_INSTRUCTIONS_AVAILABLE => bit(d1, 25),
            PF_3DNOW_INSTRUCTIONS_AVAILABLE => bit(de, 31),
            PF_RDTSC_INSTRUCTION_AVAILABLE => bit(d1, 4),
            PF_PAE_ENABLED => true,
            PF_XMMI64_INSTRUCTIONS_AVAILABLE => bit(d1, 26),
            PF_NX_ENABLED => bit(de, 20),
            PF_SSE3_INSTRUCTIONS_AVAILABLE => bit(c1, 0),
            PF_COMPARE_EXCHANGE128 => bit(c1, 13),
            PF_XSAVE_ENABLED => bit(c1, 26) && bit(c1, 27),
            PF_FASTFAIL_AVAILABLE => true,
            PF_SSSE3_INSTRUCTIONS_AVAILABLE => bit(c1, 9),
            PF_SSE4_1_INSTRUCTIONS_AVAILABLE => bit(c1, 19),
            PF_SSE4_2_INSTRUCTIONS_AVAILABLE => bit(c1, 20),
            PF_AVX_INSTRUCTIONS_AVAILABLE => bit(c1, 28) && self.avx_state(),
            PF_AVX2_INSTRUCTIONS_AVAILABLE => bit(b7, 5) && bit(c1, 28) && self.avx_state(),
            PF_AVX512F_INSTRUCTIONS_AVAILABLE => bit(b7, 16) && self.avx_state() && self.xcr0 & 0xE0 == 0xE0,
            PF_BMI2_INSTRUCTIONS_AVAILABLE => bit(b7, 8),
            PF_MOVDIR64B_INSTRUCTION_AVAILABLE => bit(c7, 28),
            PF_UMONITOR_INSTRUCTION_AVAILABLE => bit(c7, 5),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A processor with SSE to SSE4.2, AVX and AVX2, BMI2, no AVX-512, with
    /// the OS saving the AVX state.
    fn modern() -> CpuId {
        CpuId {
            leaf1_ecx: 1 | 1 << 9 | 1 << 13 | 1 << 19 | 1 << 20 | 1 << 26 | 1 << 27 | 1 << 28,
            leaf1_edx: 1 << 4 | 1 << 8 | 1 << 23 | 1 << 25 | 1 << 26,
            leaf7_ebx: 1 << 5 | 1 << 8,
            leaf7_ecx: 0,
            ext1_ecx: 0,
            ext1_edx: 1 << 20,
            xcr0: 0b111,
        }
    }

    #[test]
    fn features_follow_cpuid() {
        let c = modern();
        for f in [
            PF_COMPARE_EXCHANGE_DOUBLE,
            PF_MMX_INSTRUCTIONS_AVAILABLE,
            PF_XMMI_INSTRUCTIONS_AVAILABLE,
            PF_RDTSC_INSTRUCTION_AVAILABLE,
            PF_PAE_ENABLED,
            PF_XMMI64_INSTRUCTIONS_AVAILABLE,
            PF_NX_ENABLED,
            PF_SSE3_INSTRUCTIONS_AVAILABLE,
            PF_COMPARE_EXCHANGE128,
            PF_XSAVE_ENABLED,
            PF_FASTFAIL_AVAILABLE,
            PF_SSSE3_INSTRUCTIONS_AVAILABLE,
            PF_SSE4_1_INSTRUCTIONS_AVAILABLE,
            PF_SSE4_2_INSTRUCTIONS_AVAILABLE,
            PF_AVX_INSTRUCTIONS_AVAILABLE,
            PF_AVX2_INSTRUCTIONS_AVAILABLE,
            PF_BMI2_INSTRUCTIONS_AVAILABLE,
        ] {
            assert!(c.has(f), "PF {f}");
        }
        for f in [
            PF_FLOATING_POINT_PRECISION_ERRATA,
            PF_FLOATING_POINT_EMULATED,
            PF_3DNOW_INSTRUCTIONS_AVAILABLE,
            PF_COMPARE64_EXCHANGE128,
            PF_CHANNELS_ENABLED,
            PF_SECOND_LEVEL_ADDRESS_TRANSLATION,
            PF_VIRT_FIRMWARE_ENABLED,
            PF_RDWRFSGSBASE_AVAILABLE,
            PF_AVX512F_INSTRUCTIONS_AVAILABLE,
            PF_MOVDIR64B_INSTRUCTION_AVAILABLE,
            PF_UMONITOR_INSTRUCTION_AVAILABLE,
            29, // an ARM feature
            1000,
        ] {
            assert!(!c.has(f), "PF {f}");
        }
    }

    #[test]
    fn avx_needs_the_os_to_save_its_state() {
        let mut c = modern();
        c.xcr0 = 0b011;
        assert!(!c.has(PF_AVX_INSTRUCTIONS_AVAILABLE) && !c.has(PF_AVX2_INSTRUCTIONS_AVAILABLE));
        c.xcr0 = 0b111;
        c.leaf1_ecx &= !(1 << 27);
        assert!(!c.has(PF_AVX_INSTRUCTIONS_AVAILABLE) && !c.has(PF_XSAVE_ENABLED));
        let mut c = modern();
        c.leaf7_ebx |= 1 << 16;
        assert!(!c.has(PF_AVX512F_INSTRUCTIONS_AVAILABLE));
        c.xcr0 = 0xE7;
        assert!(c.has(PF_AVX512F_INSTRUCTIONS_AVAILABLE));
        // A bare processor: only what every x64 one has by Poly's rules.
        let none = CpuId::default();
        assert!(none.has(PF_PAE_ENABLED) && none.has(PF_FASTFAIL_AVAILABLE));
        assert!(!none.has(PF_XMMI64_INSTRUCTIONS_AVAILABLE));
    }
}
