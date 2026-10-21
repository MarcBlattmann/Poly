//! Memory status, drives, disk space and affinity masks (ADR 0087):
//! `GlobalMemoryStatusEx`, `GetLogicalDrives`, `GetLogicalDriveStringsW`,
//! `GetDiskFreeSpaceW`, `GetDiskFreeSpaceExW`, `GetProcessAffinityMask`,
//! `SetProcessAffinityMask`, `SetThreadAffinityMask` (Learn's pages;
//! layouts from `sysinfoapi.h` and `ntifs.h`/`wdm.h`).

use alloc::vec::Vec;

/// `sizeof(MEMORYSTATUSEX)` (`sysinfoapi.h`): `dwLength`, `dwMemoryLoad`,
/// then seven `DWORDLONG`s.
pub const MEMORYSTATUSEX_SIZE: usize = 64;

/// The user-mode address range's size: `SYSTEM_INFO`'s minimum (64 KiB)
/// to maximum application address ([`crate::version::system_info`]).
pub const USER_VIRTUAL: u64 = 0x7FFF_FFFE_FFFF + 1 - 0x1_0000;

/// Physical memory in bytes: total and available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysMemory {
    pub total: u64,
    pub available: u64,
}

/// Physical memory from the text of Poly's `/proc/meminfo` (the Linux
/// format, `proc(5)`: `MemTotal:` and `MemAvailable:` in kB, `MemFree:` if
/// there is no `MemAvailable:`). `kernelbase` reads it through `Z:`, the
/// whole VFS (ADR 0067): Poly's NT personality has no documented system
/// information class for memory sizes.
pub fn parse_meminfo(text: &str) -> Option<PhysMemory> {
    let field = |name: &str| -> Option<u64> {
        text.lines().find_map(|l| {
            let rest = l.strip_prefix(name)?.strip_prefix(':')?;
            let kb = rest.trim().trim_end_matches("kB").trim();
            kb.parse::<u64>().ok().map(|k| k * 1024)
        })
    };
    let total = field("MemTotal")?;
    let available = field("MemAvailable").or_else(|| field("MemFree"))?.min(total);
    Some(PhysMemory { total, available })
}

/// `MEMORYSTATUSEX` for `phys`. Poly has no paging file, so the commit
/// limit (`ullTotalPageFile`, Learn: "the current committed memory limit")
/// is physical memory; the virtual sizes are the user address range, all
/// of it reported available (Poly does not count reservations here; to be
/// confirmed). `dwMemoryLoad` is the percentage of physical memory in use.
pub fn memory_status(phys: PhysMemory) -> [u8; MEMORYSTATUSEX_SIZE] {
    let mut b = [0u8; MEMORYSTATUSEX_SIZE];
    let load = ((phys.total - phys.available) * 100).checked_div(phys.total).unwrap_or(0);
    b[0..4].copy_from_slice(&(MEMORYSTATUSEX_SIZE as u32).to_le_bytes());
    b[4..8].copy_from_slice(&(load as u32).to_le_bytes());
    for (i, v) in
        [phys.total, phys.available, phys.total, phys.available, USER_VIRTUAL, USER_VIRTUAL, 0].into_iter().enumerate()
    {
        b[8 + i * 8..16 + i * 8].copy_from_slice(&v.to_le_bytes());
    }
    b
}

/// `GetLogicalDriveStringsW`'s list for a `GetLogicalDrives` mask: `A:\`,
/// NUL, ... and a final NUL.
pub fn drive_strings(mask: u32) -> Vec<u16> {
    let mut out = Vec::new();
    for i in 0..26u16 {
        if mask & (1 << i) != 0 {
            out.extend_from_slice(&[u16::from(b'A') + i, u16::from(b':'), u16::from(b'\\'), 0]);
        }
    }
    out.push(0);
    out
}

/// What `GetDiskFreeSpaceW` / `GetDiskFreeSpaceExW` report, from a
/// volume's `FILE_FS_FULL_SIZE_INFORMATION` (`ntifs.h`: total, caller
/// available and actually available allocation units as `LARGE_INTEGER`s,
/// then sectors per unit and bytes per sector; 32 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskSpace {
    pub total_units: u64,
    pub caller_available_units: u64,
    pub available_units: u64,
    pub sectors_per_unit: u32,
    pub bytes_per_sector: u32,
}

impl DiskSpace {
    pub fn from_full_size(b: &[u8; 32]) -> DiskSpace {
        let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let d = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        DiskSpace {
            total_units: q(0),
            caller_available_units: q(8),
            available_units: q(16),
            sectors_per_unit: d(24),
            bytes_per_sector: d(28),
        }
    }

    fn unit_bytes(&self) -> u64 {
        u64::from(self.sectors_per_unit) * u64::from(self.bytes_per_sector)
    }

    /// `GetDiskFreeSpaceExW`: (free bytes available to the caller, total
    /// bytes, total free bytes).
    pub fn bytes(&self) -> (u64, u64, u64) {
        let u = self.unit_bytes();
        (
            self.caller_available_units.saturating_mul(u),
            self.total_units.saturating_mul(u),
            self.available_units.saturating_mul(u),
        )
    }

    /// `GetDiskFreeSpaceW`: (sectors per cluster, bytes per sector, free
    /// clusters available to the caller, total clusters); the counts are
    /// `DWORD`s, so larger volumes report `0xFFFFFFFF` (Learn: the function
    /// "returns incorrect values for volumes that are larger than 2
    /// gigabytes"; Poly's choice of incorrect value).
    pub fn clusters(&self) -> (u32, u32, u32, u32) {
        let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        (self.sectors_per_unit, self.bytes_per_sector, clamp(self.caller_available_units), clamp(self.total_units))
    }
}

/// The system's affinity mask for `cpus` processors (one bit each, as
/// `SYSTEM_INFO.dwActiveProcessorMask`).
pub fn system_affinity(cpus: u32) -> u64 {
    let cpus = cpus.clamp(1, 64);
    if cpus == 64 { u64::MAX } else { (1u64 << cpus) - 1 }
}

/// Whether `mask` is a valid affinity for a process or thread on a system
/// with `system` as its mask: Learn: not zero and a subset of the system's
/// (else `ERROR_INVALID_PARAMETER`).
pub fn valid_affinity(mask: u64, system: u64) -> bool {
    mask != 0 && mask & !system == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    #[test]
    fn meminfo_and_status() {
        let text = "MemTotal:         262144 kB\nMemFree:          100000 kB\nMemAvailable:     131072 kB\n";
        let m = parse_meminfo(text).unwrap();
        assert_eq!(m, PhysMemory { total: 262_144 * 1024, available: 131_072 * 1024 });
        assert_eq!(parse_meminfo("MemTotal: 1000 kB\nMemFree: 250 kB\n").unwrap().available, 250 * 1024);
        assert_eq!(parse_meminfo("MemFree: 250 kB\n"), None);
        let b = memory_status(m);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 64);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 50);
        assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), m.total);
        assert_eq!(u64::from_le_bytes(b[16..24].try_into().unwrap()), m.available);
        assert_eq!(u64::from_le_bytes(b[40..48].try_into().unwrap()), USER_VIRTUAL);
        assert_eq!(u64::from_le_bytes(b[56..64].try_into().unwrap()), 0);
    }

    #[test]
    fn drives_and_space() {
        let s = drive_strings(1 << 2 | 1 << 25);
        let want: std::vec::Vec<u16> = "C:\\\0Z:\\\0\0".encode_utf16().collect();
        assert_eq!(s, want);
        assert_eq!(drive_strings(0), [0]);
        let mut b = [0u8; 32];
        b[0..8].copy_from_slice(&(1u64 << 33).to_le_bytes());
        b[8..16].copy_from_slice(&1000u64.to_le_bytes());
        b[16..24].copy_from_slice(&2000u64.to_le_bytes());
        b[24..28].copy_from_slice(&8u32.to_le_bytes());
        b[28..32].copy_from_slice(&512u32.to_le_bytes());
        let d = DiskSpace::from_full_size(&b);
        assert_eq!(d.bytes(), (1000 * 4096, (1u64 << 33) * 4096, 2000 * 4096));
        assert_eq!(d.clusters(), (8, 512, 1000, u32::MAX));
    }

    #[test]
    fn affinity() {
        assert_eq!(system_affinity(2), 0b11);
        assert_eq!(system_affinity(64), u64::MAX);
        assert_eq!(system_affinity(0), 1);
        assert!(valid_affinity(0b10, 0b11));
        assert!(!valid_affinity(0, 0b11));
        assert!(!valid_affinity(0b100, 0b11));
    }
}
