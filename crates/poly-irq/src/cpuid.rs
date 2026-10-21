//! The CPUID feature bits interrupt and timer setup depends on (SDM Vol. 2A,
//! `CPUID`, tables "Feature Information Returned in the ECX Register" and
//! "... in the EDX Register", leaf 06H and leaf 80000007H).
//!
//! The kernel executes `cpuid` and passes the raw registers in, so the
//! decoding is testable on the host. For a leaf above the maximum the CPU
//! reports (CPUID.0:EAX for basic leaves, CPUID.80000000H:EAX for extended
//! ones), pass 0: every feature it would have reported then reads as absent.

/// Feature flags decoded from CPUID.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuFeatures {
    /// The `rdtsc` instruction exists (CPUID.01H:EDX[4]).
    pub tsc: bool,
    /// An on-chip local APIC exists (CPUID.01H:EDX[9]).
    pub apic: bool,
    /// The local APIC supports x2APIC mode (CPUID.01H:ECX[21]).
    pub x2apic: bool,
    /// The APIC timer supports TSC-deadline mode (CPUID.01H:ECX[24]).
    pub tsc_deadline: bool,
    /// The APIC timer keeps running in deep C-states (ARAT,
    /// CPUID.06H:EAX[2]). Without it, the APIC timer may stop while the CPU
    /// idles.
    pub arat: bool,
    /// The TSC runs at a constant rate in all ACPI P-, C- and T-states
    /// (CPUID.80000007H:EDX[8]; SDM Vol. 3B §18.17.1 "Invariant TSC"), so it
    /// can serve as a wall-clock time base.
    pub invariant_tsc: bool,
    /// The CPU is a virtual one: a hypervisor runs the system
    /// (CPUID.01H:ECX[31], reserved for this use; it is 0 on real
    /// processors).
    pub hypervisor: bool,
}

impl CpuFeatures {
    pub const fn from_cpuid(leaf1_ecx: u32, leaf1_edx: u32, leaf6_eax: u32, leaf_8000_0007_edx: u32) -> CpuFeatures {
        CpuFeatures {
            tsc: leaf1_edx & (1 << 4) != 0,
            apic: leaf1_edx & (1 << 9) != 0,
            x2apic: leaf1_ecx & (1 << 21) != 0,
            tsc_deadline: leaf1_ecx & (1 << 24) != 0,
            arat: leaf6_eax & (1 << 2) != 0,
            invariant_tsc: leaf_8000_0007_edx & (1 << 8) != 0,
            hypervisor: leaf1_ecx & (1 << 31) != 0,
        }
    }

    /// Whether a calibrated TSC may serve as the clock: it is invariant,
    /// or the CPU is virtual. A hypervisor gives its guests a TSC that runs
    /// at one rate whatever the host's power states do, even where it does
    /// not report the invariant bit, as QEMU's CPU models do not (ADR
    /// 0072). On real processors without the bit the TSC may change rate or
    /// stop.
    pub const fn constant_rate_tsc(&self) -> bool {
        self.invariant_tsc || self.hypervisor
    }
}

/// The nominal TSC frequency from CPUID leaf 15H ("Time Stamp Counter and
/// Nominal Core Crystal Clock Information"): `ecx * ebx / eax`, where EAX and
/// EBX are the TSC / crystal clock ratio and ECX the crystal frequency in Hz.
/// `None` when any of them is 0, which means "not enumerated". Even when
/// present, the value is nominal; [`crate::time::calibrate`] measures it.
pub const fn tsc_hz_from_leaf15(eax: u32, ebx: u32, ecx: u32) -> Option<u64> {
    if eax == 0 || ebx == 0 || ecx == 0 {
        return None;
    }
    Some(ecx as u64 * ebx as u64 / eax as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_each_bit() {
        assert_eq!(CpuFeatures::from_cpuid(0, 0, 0, 0), CpuFeatures::default());
        let all = CpuFeatures::from_cpuid(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        assert!(
            all.tsc && all.apic && all.x2apic && all.tsc_deadline && all.arat && all.invariant_tsc && all.hypervisor
        );
        assert!(CpuFeatures::from_cpuid(1 << 21, 0, 0, 0).x2apic);
        assert!(CpuFeatures::from_cpuid(1 << 24, 0, 0, 0).tsc_deadline);
        assert!(CpuFeatures::from_cpuid(0, 1 << 4, 0, 0).tsc);
        assert!(CpuFeatures::from_cpuid(0, 1 << 9, 0, 0).apic);
        assert!(CpuFeatures::from_cpuid(0, 0, 1 << 2, 0).arat);
        assert!(CpuFeatures::from_cpuid(0, 0, 0, 1 << 8).invariant_tsc);
        assert!(CpuFeatures::from_cpuid(1 << 31, 0, 0, 0).hypervisor);
        // Neighbouring bits do not leak in.
        assert_eq!(
            CpuFeatures::from_cpuid(!(1 << 21 | 1 << 24 | 1 << 31), !(1 << 4 | 1 << 9), !4, !(1 << 8)),
            CpuFeatures::default()
        );
    }

    #[test]
    fn a_tsc_is_constant_rate_when_invariant_or_virtual() {
        let real_old = CpuFeatures::from_cpuid(0, 1 << 4, 0, 0);
        assert!(!real_old.constant_rate_tsc(), "a real CPU without the invariant bit");
        assert!(CpuFeatures::from_cpuid(0, 1 << 4, 0, 1 << 8).constant_rate_tsc());
        // QEMU's CPU models: the hypervisor bit, no invariant bit.
        let qemu = CpuFeatures::from_cpuid(1 << 31, 1 << 4, 0, 0);
        assert!(!qemu.invariant_tsc && qemu.constant_rate_tsc());
    }

    #[test]
    fn leaf15_frequency() {
        // 24 MHz crystal, ratio 188/2: 2.256 GHz.
        assert_eq!(tsc_hz_from_leaf15(2, 188, 24_000_000), Some(2_256_000_000));
        assert_eq!(tsc_hz_from_leaf15(2, 188, 0), None);
        assert_eq!(tsc_hz_from_leaf15(0, 188, 24_000_000), None);
        assert_eq!(tsc_hz_from_leaf15(1, u32::MAX, u32::MAX), Some(u32::MAX as u64 * u32::MAX as u64));
    }
}
