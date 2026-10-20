//! What `/proc/cpuinfo` reports about the CPU, read with CPUID (Intel SDM
//! vol. 2A, CPUID: leaves 0, 01H, 07H, 80000001H-80000004H, 80000008H).

use super::cpu;
use alloc::string::String;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use poly_procfs::{CpuFeatures, CpuInfo};

/// The boot CPU, as CPU number 0, running at `khz`.
pub fn cpu_info(khz: u64) -> CpuInfo {
    let id = cpu::identify();
    let max_basic = __cpuid(0).eax;
    let max_extended = __cpuid(0x8000_0000).eax;
    let leaf1 = __cpuid(1);
    let leaf7 = if max_basic >= 7 { Some(__cpuid_count(7, 0)) } else { None };
    let ext1 = if max_extended >= 0x8000_0001 { Some(__cpuid(0x8000_0001)) } else { None };
    // Leaf 80000008H EAX: physical address bits 7:0, linear 15:8.
    let (phys_bits, virt_bits) = if max_extended >= 0x8000_0008 {
        let a = __cpuid(0x8000_0008).eax;
        (a & 0xff, (a >> 8) & 0xff)
    } else {
        (36, 48)
    };
    CpuInfo {
        index: 0,
        vendor: String::from(id.vendor()),
        model_name: String::from(id.brand()),
        family: id.family,
        model: id.model,
        stepping: id.stepping,
        cpuid_level: max_basic,
        khz,
        // Leaf 01H EBX: initial APIC id in bits 31:24, CLFLUSH line size in
        // 8-byte units in bits 15:8.
        apic_id: leaf1.ebx >> 24,
        clflush: ((leaf1.ebx >> 8) & 0xff) * 8,
        phys_bits,
        virt_bits,
        features: CpuFeatures {
            leaf1_edx: leaf1.edx,
            leaf1_ecx: leaf1.ecx,
            leaf7_ebx: leaf7.map_or(0, |r| r.ebx),
            leaf7_ecx: leaf7.map_or(0, |r| r.ecx),
            leaf7_edx: leaf7.map_or(0, |r| r.edx),
            ext1_edx: ext1.map_or(0, |r| r.edx),
            ext1_ecx: ext1.map_or(0, |r| r.ecx),
        },
    }
}
