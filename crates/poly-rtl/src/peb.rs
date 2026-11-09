//! The fixed fields of the PEB a process start fills in (ADR 0155): programs read them
//! straight from `gs:[0x60]` instead of calling a function (the OS version, the number of
//! processors, the executable's subsystem, the process heap). Offsets are x64 `PEB`'s
//! (the WDK's `ntpebteb.h`; `winternl.h` documents `BeingDebugged`, `Ldr` and
//! `ProcessParameters`).
//!
//! Chrome's `chrome_elf.dll` reads `PEB.ImageBaseAddress` that way and faulted at address 0x10
//! when it was 0 (found with the Poly-side call tracer, `cargo xtask chrome-win run --kit`).

use crate::version;

pub const PROCESS_HEAP: usize = 0x30;
pub const NUMBER_OF_PROCESSORS: usize = 0xB8;
pub const NT_GLOBAL_FLAG: usize = 0xBC;
pub const OS_MAJOR_VERSION: usize = 0x118;
pub const OS_MINOR_VERSION: usize = 0x11C;
pub const OS_BUILD_NUMBER: usize = 0x120;
pub const OS_CSD_VERSION: usize = 0x122;
pub const OS_PLATFORM_ID: usize = 0x124;
pub const IMAGE_SUBSYSTEM: usize = 0x128;
pub const IMAGE_SUBSYSTEM_MAJOR_VERSION: usize = 0x12C;
pub const IMAGE_SUBSYSTEM_MINOR_VERSION: usize = 0x130;
pub const SESSION_ID: usize = 0x2C0;

/// What the process start knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    /// The processors the system reports (1..=64).
    pub processors: u32,
    /// The executable's `Subsystem`, `MajorSubsystemVersion` and `MinorSubsystemVersion`.
    pub subsystem: (u16, u16, u16),
}

/// One write: `width` (2 or 4) little-endian bytes of `value` at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub at: usize,
    pub value: u32,
    pub width: usize,
}

/// The writes that fill in the fields, in offset order.
pub fn fields(f: Facts) -> [Field; 10] {
    let w4 = |at, value| Field { at, value, width: 4 };
    let (sub, major, minor) = f.subsystem;
    [
        w4(NUMBER_OF_PROCESSORS, f.processors.clamp(1, 64)),
        w4(NT_GLOBAL_FLAG, 0),
        w4(OS_MAJOR_VERSION, version::MAJOR),
        w4(OS_MINOR_VERSION, version::MINOR),
        Field { at: OS_BUILD_NUMBER, value: version::BUILD & 0xFFFF, width: 2 },
        Field { at: OS_CSD_VERSION, value: 0, width: 2 },
        w4(OS_PLATFORM_ID, version::PLATFORM_WIN32_NT),
        w4(IMAGE_SUBSYSTEM, u32::from(sub)),
        w4(IMAGE_SUBSYSTEM_MAJOR_VERSION, u32::from(major)),
        w4(IMAGE_SUBSYSTEM_MINOR_VERSION, u32::from(minor)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_are_the_reported_version_and_the_executables_subsystem() {
        let mut peb = [0u8; 0x200];
        for f in fields(Facts { processors: 4, subsystem: (3, 6, 0) }) {
            peb[f.at..f.at + f.width].copy_from_slice(&f.value.to_le_bytes()[..f.width]);
        }
        let u32_at = |o: usize| u32::from_le_bytes(peb[o..o + 4].try_into().unwrap());
        assert_eq!((u32_at(OS_MAJOR_VERSION), u32_at(OS_MINOR_VERSION)), (10, 0));
        assert_eq!(u16::from_le_bytes([peb[OS_BUILD_NUMBER], peb[OS_BUILD_NUMBER + 1]]), 19045);
        assert_eq!(u32_at(OS_PLATFORM_ID), 2);
        assert_eq!(u32_at(NUMBER_OF_PROCESSORS), 4);
        assert_eq!(
            (u32_at(IMAGE_SUBSYSTEM), u32_at(IMAGE_SUBSYSTEM_MAJOR_VERSION), u32_at(IMAGE_SUBSYSTEM_MINOR_VERSION)),
            (3, 6, 0)
        );
    }

    #[test]
    fn the_processor_count_stays_in_range_and_the_writes_do_not_overlap() {
        let f = |p| fields(Facts { processors: p, subsystem: (2, 0, 0) });
        assert_eq!(f(0)[0].value, 1);
        assert_eq!(f(500)[0].value, 64);
        let w = f(2);
        for pair in w.windows(2) {
            assert!(pair[0].at + pair[0].width <= pair[1].at, "{pair:?}");
        }
    }
}
