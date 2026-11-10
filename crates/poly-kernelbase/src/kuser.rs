//! `KUSER_SHARED_DATA` (ADR 0155): the page every Windows process has read-only at
//! 0x7FFE0000 (`MM_SHARED_USER_DATA_VA`, `ntddk.h`), where programs read the OS version, the
//! processor features and the clock without calling anything. Chrome's modules (and the C runtime)
//! read it directly; with the page unmapped such a read faults.
//!
//! Layout: x64 `KUSER_SHARED_DATA` of `ntddk.h`/`wdm.h` (the offsets below are the ones those
//! headers' fields have; Learn's "KUSER_SHARED_DATA" documents the structure's purpose):
//!
//! | Field | Offset |
//! |---|---|
//! | `TickCountMultiplier` | 0x004 |
//! | `InterruptTime`, `SystemTime`, `TimeZoneBias` (`KSYSTEM_TIME`: low, high1, high2) | 0x008, 0x014, 0x020 |
//! | `ImageNumberLow`, `ImageNumberHigh` | 0x02C, 0x02E |
//! | `NtSystemRoot` (`WCHAR[260]`) | 0x030 |
//! | `NtBuildNumber`, `NtProductType`, `ProductTypeIsValid` | 0x260, 0x264, 0x268 |
//! | `NativeProcessorArchitecture` | 0x26A |
//! | `NtMajorVersion`, `NtMinorVersion` | 0x26C, 0x270 |
//! | `ProcessorFeatures` (`BOOLEAN[64]`, indexed by `PF_*`) | 0x274 |
//! | `SuiteMask`, `KdDebuggerEnabled` | 0x2D0, 0x2D4 |
//! | `ActiveConsoleId`, `NumberOfPhysicalPages`, `SafeBootMode` | 0x2D8, 0x2E8, 0x2EC |
//! | `QpcFrequency` | 0x300 |
//! | `TickCount` (`KSYSTEM_TIME`) | 0x320 |
//! | `ActiveProcessorCount`, `ActiveGroupCount` | 0x3C0, 0x3C4 |
//!
//! Poly fills the page when a process starts (a snapshot: the clock fields do not advance).

use crate::cpu::CpuId;
use poly_rtl::version;

/// Where the page is mapped in every process.
pub const ADDRESS: u64 = 0x7FFE_0000;
/// The page's size.
pub const SIZE: usize = 4096;

pub const TICK_COUNT_MULTIPLIER: usize = 0x004;
pub const INTERRUPT_TIME: usize = 0x008;
pub const SYSTEM_TIME: usize = 0x014;
pub const TIME_ZONE_BIAS: usize = 0x020;
pub const IMAGE_NUMBER_LOW: usize = 0x02C;
pub const IMAGE_NUMBER_HIGH: usize = 0x02E;
pub const NT_SYSTEM_ROOT: usize = 0x030;
pub const NT_BUILD_NUMBER: usize = 0x260;
pub const NT_PRODUCT_TYPE: usize = 0x264;
pub const PRODUCT_TYPE_IS_VALID: usize = 0x268;
pub const NATIVE_PROCESSOR_ARCHITECTURE: usize = 0x26A;
pub const NT_MAJOR_VERSION: usize = 0x26C;
pub const NT_MINOR_VERSION: usize = 0x270;
pub const PROCESSOR_FEATURES: usize = 0x274;
pub const SUITE_MASK: usize = 0x2D0;
pub const KD_DEBUGGER_ENABLED: usize = 0x2D4;
pub const ACTIVE_CONSOLE_ID: usize = 0x2D8;
pub const NUMBER_OF_PHYSICAL_PAGES: usize = 0x2E8;
pub const QPC_FREQUENCY: usize = 0x300;
pub const TICK_COUNT: usize = 0x320;
pub const ACTIVE_PROCESSOR_COUNT: usize = 0x3C0;
pub const ACTIVE_GROUP_COUNT: usize = 0x3C4;

/// `PROCESSOR_ARCHITECTURE_AMD64` (`winnt.h`).
const PROCESSOR_ARCHITECTURE_AMD64: u16 = 9;
/// `IMAGE_FILE_MACHINE_AMD64` (`winnt.h`).
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
/// The clock tick of `GetTickCount` (15.625 ms) in 100 ns units.
const TICK_100NS: u64 = 156_250;
/// `TickCountMultiplier`: the tick length in milliseconds as a 8.24 fixed-point number
/// (`GetTickCount` is `TickCount * TickCountMultiplier >> 24`): 15.625 * 2^24.
const TICK_MULTIPLIER: u32 = 0x0FA0_0000;
/// The frequency `QueryPerformanceFrequency` reports (the 100 ns clock).
const QPC_FREQUENCY_HZ: u64 = 10_000_000;
/// `NtSystemRoot`.
const SYSTEM_ROOT: &str = "C:\\Windows";

/// What the kernel knows when it starts a process.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    /// The wall clock as a `FILETIME`.
    pub system_time: u64,
    /// The time since boot in 100 ns units.
    pub uptime: u64,
    /// The processors online.
    pub processors: u32,
    /// The usable memory, in 4 KiB pages.
    pub physical_pages: u64,
    /// The processor's `CPUID` leaves (for `ProcessorFeatures`).
    pub cpu: CpuId,
}

fn put(page: &mut [u8; SIZE], at: usize, bytes: &[u8]) {
    page[at..at + bytes.len()].copy_from_slice(bytes);
}

/// A `KSYSTEM_TIME`: the low part, then the high part twice (a reader retries until
/// `High1Time == High2Time`).
fn put_time(page: &mut [u8; SIZE], at: usize, value: u64) {
    put(page, at, &(value as u32).to_le_bytes());
    put(page, at + 4, &((value >> 32) as u32).to_le_bytes());
    put(page, at + 8, &((value >> 32) as u32).to_le_bytes());
}

/// The page's contents.
pub fn page(f: &Facts) -> [u8; SIZE] {
    let mut p = [0u8; SIZE];
    put(&mut p, TICK_COUNT_MULTIPLIER, &TICK_MULTIPLIER.to_le_bytes());
    put_time(&mut p, INTERRUPT_TIME, f.uptime);
    put_time(&mut p, SYSTEM_TIME, f.system_time);
    put_time(&mut p, TIME_ZONE_BIAS, 0);
    put(&mut p, IMAGE_NUMBER_LOW, &IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
    put(&mut p, IMAGE_NUMBER_HIGH, &IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
    for (i, unit) in SYSTEM_ROOT.encode_utf16().enumerate() {
        put(&mut p, NT_SYSTEM_ROOT + 2 * i, &unit.to_le_bytes());
    }
    put(&mut p, NT_BUILD_NUMBER, &version::BUILD.to_le_bytes());
    put(&mut p, NT_PRODUCT_TYPE, &u32::from(version::NT_WORKSTATION).to_le_bytes());
    p[PRODUCT_TYPE_IS_VALID] = 1;
    put(&mut p, NATIVE_PROCESSOR_ARCHITECTURE, &PROCESSOR_ARCHITECTURE_AMD64.to_le_bytes());
    put(&mut p, NT_MAJOR_VERSION, &version::MAJOR.to_le_bytes());
    put(&mut p, NT_MINOR_VERSION, &version::MINOR.to_le_bytes());
    for feature in 0..64u32 {
        p[PROCESSOR_FEATURES + feature as usize] = u8::from(f.cpu.has(feature));
    }
    put(&mut p, SUITE_MASK, &u32::from(version::SUITE_SINGLEUSERTS).to_le_bytes());
    put(&mut p, NUMBER_OF_PHYSICAL_PAGES, &(f.physical_pages.min(u64::from(u32::MAX)) as u32).to_le_bytes());
    put(&mut p, QPC_FREQUENCY, &QPC_FREQUENCY_HZ.to_le_bytes());
    put_time(&mut p, TICK_COUNT, f.uptime / TICK_100NS);
    put(&mut p, ACTIVE_PROCESSOR_COUNT, &f.processors.max(1).to_le_bytes());
    p[ACTIVE_GROUP_COUNT] = 1;
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn facts() -> Facts {
        Facts {
            system_time: 0x01DC_0000_1234_5678,
            uptime: 5 * 10_000_000 + 77,
            processors: 4,
            physical_pages: 262_144,
            cpu: CpuId { leaf1_edx: 1 << 4 | 1 << 25 | 1 << 26, leaf1_ecx: 1, ..CpuId::default() },
        }
    }

    fn u32_at(p: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(p[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn the_version_fields_are_the_reported_version() {
        let p = page(&facts());
        assert_eq!((u32_at(&p, NT_MAJOR_VERSION), u32_at(&p, NT_MINOR_VERSION)), (10, 0));
        assert_eq!(u32_at(&p, NT_BUILD_NUMBER), 19045);
        assert_eq!((u32_at(&p, NT_PRODUCT_TYPE), p[PRODUCT_TYPE_IS_VALID]), (1, 1));
        assert_eq!(u16::from_le_bytes([p[NATIVE_PROCESSOR_ARCHITECTURE], p[NATIVE_PROCESSOR_ARCHITECTURE + 1]]), 9);
        assert_eq!(u32_at(&p, SUITE_MASK), 0x100);
    }

    #[test]
    fn the_system_root_and_machine_are_set() {
        let p = page(&facts());
        let units: Vec<u16> =
            p[NT_SYSTEM_ROOT..NT_SYSTEM_ROOT + 22].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "C:\\Windows\0");
        assert_eq!(u16::from_le_bytes([p[IMAGE_NUMBER_LOW], p[IMAGE_NUMBER_LOW + 1]]), 0x8664);
    }

    #[test]
    fn the_times_are_consistent_ksystem_times() {
        let f = facts();
        let p = page(&f);
        let time = |at: usize| u64::from(u32_at(&p, at)) | u64::from(u32_at(&p, at + 4)) << 32;
        assert_eq!(time(SYSTEM_TIME), f.system_time);
        assert_eq!(u32_at(&p, SYSTEM_TIME + 4), u32_at(&p, SYSTEM_TIME + 8), "High1Time == High2Time");
        assert_eq!(time(INTERRUPT_TIME), f.uptime);
        // 5.0000077 s is 320 ticks of 15.625 ms.
        assert_eq!(time(TICK_COUNT), 320);
        // GetTickCount: TickCount * multiplier >> 24 milliseconds.
        let ms = (time(TICK_COUNT) * u64::from(u32_at(&p, TICK_COUNT_MULTIPLIER))) >> 24;
        assert_eq!(ms, 5000);
    }

    #[test]
    fn the_processor_features_follow_the_cpu() {
        let p = page(&facts());
        let feature = |n: u32| p[PROCESSOR_FEATURES + n as usize];
        assert_eq!(feature(crate::cpu::PF_RDTSC_INSTRUCTION_AVAILABLE), 1);
        assert_eq!(feature(crate::cpu::PF_XMMI_INSTRUCTIONS_AVAILABLE), 1);
        assert_eq!(feature(crate::cpu::PF_XMMI64_INSTRUCTIONS_AVAILABLE), 1);
        assert_eq!(feature(crate::cpu::PF_SSE3_INSTRUCTIONS_AVAILABLE), 1);
        assert_eq!(feature(crate::cpu::PF_AVX_INSTRUCTIONS_AVAILABLE), 0);
        assert_eq!(feature(crate::cpu::PF_FASTFAIL_AVAILABLE), 1);
        assert_eq!(u32_at(&p, ACTIVE_PROCESSOR_COUNT), 4);
        assert_eq!(p[ACTIVE_GROUP_COUNT], 1);
        assert_eq!(u32_at(&p, NUMBER_OF_PHYSICAL_PAGES), 262_144);
        assert_eq!(u64::from_le_bytes(p[QPC_FREQUENCY..QPC_FREQUENCY + 8].try_into().unwrap()), 10_000_000);
    }

    #[test]
    fn the_fields_stay_inside_the_page_and_the_features_do_not_overlap_others() {
        const {
            assert!(PROCESSOR_FEATURES + 64 <= SUITE_MASK);
            assert!(ACTIVE_GROUP_COUNT < SIZE);
            assert!(NT_SYSTEM_ROOT + 520 <= 0x238);
        }
    }
}
