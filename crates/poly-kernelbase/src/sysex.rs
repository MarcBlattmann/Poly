//! System information and small memory/loader rules for Chrome's batch 2
//! (ADR 0155), as host-tested code: logical processor information, computer
//! names, geographic locations, power status, locale names, heap
//! information, `GetModuleHandleExW`'s flags and `VirtualAlloc2`'s extended
//! parameters.
//!
//! Sources (clean room): Learn's pages for `GetLogicalProcessorInformation`,
//! `GetLogicalProcessorInformationEx` and the structures they fill
//! (`SYSTEM_LOGICAL_PROCESSOR_INFORMATION(_EX)`, `PROCESSOR_RELATIONSHIP`,
//! `NUMA_NODE_RELATIONSHIP`, `GROUP_RELATIONSHIP`, `GROUP_AFFINITY`),
//! `GetComputerNameExW`, `COMPUTER_NAME_FORMAT`, `GetUserGeoID`,
//! `GetGeoInfoW`, `GEOCLASS`, `SYSTEM_POWER_STATUS`, `HeapSetInformation`,
//! `GetModuleHandleExW`, `VirtualAlloc2`, `MEM_EXTENDED_PARAMETER`,
//! `MEM_ADDRESS_REQUIREMENTS` and `IsWow64Process2`; layouts and constants
//! from `winnt.h`, `winbase.h` and `memoryapi.h`.

use crate::error::*;
use alloc::string::String;
use alloc::vec::Vec;

// ----- logical processors -------------------------------------------------------------------

/// `LOGICAL_PROCESSOR_RELATIONSHIP` (`winnt.h`).
pub const RELATION_PROCESSOR_CORE: u32 = 0;
pub const RELATION_NUMA_NODE: u32 = 1;
pub const RELATION_CACHE: u32 = 2;
pub const RELATION_PROCESSOR_PACKAGE: u32 = 3;
pub const RELATION_GROUP: u32 = 4;
pub const RELATION_ALL: u32 = 0xFFFF;

/// `sizeof(SYSTEM_LOGICAL_PROCESSOR_INFORMATION)` on x64.
pub const LPI_SIZE: usize = 32;

/// What the processor topology is: Poly knows its logical processors only.
/// Each is its own core (no SMT is reported), all in one package and one
/// NUMA node; cache sizes are not known, so no cache relation is reported.
/// Processors are numbered into groups of 64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Topology {
    pub cpus: u32,
}

impl Topology {
    fn groups(&self) -> u32 {
        self.cpus.max(1).div_ceil(64)
    }

    fn in_group(&self, g: u32) -> u32 {
        (self.cpus.max(1) - g * 64).min(64)
    }

    fn mask(&self, g: u32) -> u64 {
        let n = self.in_group(g);
        if n == 64 { u64::MAX } else { (1u64 << n) - 1 }
    }

    /// `GetLogicalProcessorInformation`: entries for the first group
    /// (Learn: the processors of the caller's group): one core per
    /// processor, then the NUMA node and the package.
    pub fn legacy(&self) -> Vec<u8> {
        let n = self.in_group(0);
        let mut out = Vec::new();
        let mut entry = |mask: u64, relationship: u32, first: u64| {
            let mut e = [0u8; LPI_SIZE];
            e[0..8].copy_from_slice(&mask.to_le_bytes());
            e[8..12].copy_from_slice(&relationship.to_le_bytes());
            e[16..24].copy_from_slice(&first.to_le_bytes());
            out.extend_from_slice(&e);
        };
        for i in 0..n {
            entry(1u64 << i, RELATION_PROCESSOR_CORE, 0);
        }
        entry(self.mask(0), RELATION_NUMA_NODE, 0);
        entry(self.mask(0), RELATION_PROCESSOR_PACKAGE, 0);
        out
    }

    fn group_affinity(&self, g: u32) -> [u8; 16] {
        let mut a = [0u8; 16];
        a[0..8].copy_from_slice(&self.mask(g).to_le_bytes());
        a[8..10].copy_from_slice(&(g as u16).to_le_bytes());
        a
    }

    /// One `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` entry: `Relationship`
    /// 0, `Size` 4, then the union at 8.
    fn ex_entry(&self, relationship: u32, cpu: u32) -> Vec<u8> {
        let mut e = Vec::new();
        e.extend_from_slice(&relationship.to_le_bytes());
        e.extend_from_slice(&[0; 4]); // Size, filled below
        match relationship {
            RELATION_PROCESSOR_CORE => {
                // PROCESSOR_RELATIONSHIP: Flags 8, EfficiencyClass 9,
                // Reserved[20], GroupCount 30, GroupMask[] 32.
                e.resize(32, 0);
                e[30..32].copy_from_slice(&1u16.to_le_bytes());
                let g = cpu / 64;
                let mut a = [0u8; 16];
                a[0..8].copy_from_slice(&(1u64 << (cpu % 64)).to_le_bytes());
                a[8..10].copy_from_slice(&(g as u16).to_le_bytes());
                e.extend_from_slice(&a);
            }
            RELATION_PROCESSOR_PACKAGE => {
                e.resize(32, 0);
                e[30..32].copy_from_slice(&(self.groups() as u16).to_le_bytes());
                for g in 0..self.groups() {
                    e.extend_from_slice(&self.group_affinity(g));
                }
            }
            RELATION_NUMA_NODE => {
                // NUMA_NODE_RELATIONSHIP: NodeNumber 8, Reserved[18],
                // GroupCount 30, GroupMasks[] 32.
                e.resize(32, 0);
                e[30..32].copy_from_slice(&(self.groups() as u16).to_le_bytes());
                for g in 0..self.groups() {
                    e.extend_from_slice(&self.group_affinity(g));
                }
            }
            RELATION_GROUP => {
                // GROUP_RELATIONSHIP: MaximumGroupCount 8, ActiveGroupCount
                // 10, Reserved[20], GroupInfo[] 32; PROCESSOR_GROUP_INFO is
                // 48 bytes: maximum and active processor counts, Reserved[38],
                // ActiveProcessorMask.
                e.resize(32, 0);
                e[8..10].copy_from_slice(&(self.groups() as u16).to_le_bytes());
                e[10..12].copy_from_slice(&(self.groups() as u16).to_le_bytes());
                for g in 0..self.groups() {
                    let mut info = [0u8; 48];
                    info[0] = self.in_group(g) as u8;
                    info[1] = self.in_group(g) as u8;
                    info[40..48].copy_from_slice(&self.mask(g).to_le_bytes());
                    e.extend_from_slice(&info);
                }
            }
            _ => {}
        }
        let size = e.len() as u32;
        e[4..8].copy_from_slice(&size.to_le_bytes());
        e
    }

    /// `GetLogicalProcessorInformationEx(relationship)`: the entries in
    /// order (cores, NUMA node, package, group for `RelationAll`).
    /// `ERROR_INVALID_PARAMETER` for a value that is not a relationship;
    /// the newer die and module relationships have no entries here.
    pub fn ex(&self, relationship: u32) -> Result<Vec<u8>, u32> {
        let mut out = Vec::new();
        let want = |r: u32| relationship == RELATION_ALL || relationship == r;
        match relationship {
            RELATION_PROCESSOR_CORE
            | RELATION_NUMA_NODE
            | RELATION_CACHE
            | RELATION_PROCESSOR_PACKAGE
            | RELATION_GROUP
            | RELATION_ALL => {}
            5..=7 => return Ok(out),
            _ => return Err(ERROR_INVALID_PARAMETER),
        }
        if want(RELATION_PROCESSOR_CORE) {
            for cpu in 0..self.cpus.max(1) {
                out.extend(self.ex_entry(RELATION_PROCESSOR_CORE, cpu));
            }
        }
        for r in [RELATION_NUMA_NODE, RELATION_PROCESSOR_PACKAGE, RELATION_GROUP] {
            if want(r) {
                out.extend(self.ex_entry(r, 0));
            }
        }
        Ok(out)
    }
}

/// The "buffer and length" rule of both functions: the buffer holds
/// `have` bytes; `need` are required. `Err(ERROR_INSUFFICIENT_BUFFER)` when
/// it is smaller (the caller returns `need` in `*ReturnedLength`).
pub fn fits(have: usize, need: usize) -> Result<(), u32> {
    if have < need { Err(ERROR_INSUFFICIENT_BUFFER) } else { Ok(()) }
}

// ----- computer names ---------------------------------------------------------------------------

/// `COMPUTER_NAME_FORMAT` (`sysinfoapi.h`).
pub const COMPUTER_NAME_NETBIOS: u32 = 0;
pub const COMPUTER_NAME_DNS_HOSTNAME: u32 = 1;
pub const COMPUTER_NAME_DNS_DOMAIN: u32 = 2;
pub const COMPUTER_NAME_DNS_FULLY_QUALIFIED: u32 = 3;
pub const COMPUTER_NAME_PHYSICAL_NETBIOS: u32 = 4;
pub const COMPUTER_NAME_PHYSICAL_DNS_HOSTNAME: u32 = 5;
pub const COMPUTER_NAME_PHYSICAL_DNS_DOMAIN: u32 = 6;
pub const COMPUTER_NAME_PHYSICAL_DNS_FULLY_QUALIFIED: u32 = 7;
/// `MAX_COMPUTERNAME_LENGTH` (`winbase.h`).
pub const MAX_COMPUTERNAME_LENGTH: usize = 15;

/// The name of the computer in format `kind`, from `host` (its configured
/// name). The NetBIOS name is upper case and at most 15 characters; the DNS
/// host name is the name as configured; Poly's computers are in no DNS
/// domain, so the domain is empty and the fully qualified name is the host
/// name.
pub fn computer_name(kind: u32, host: &str) -> Result<String, u32> {
    Ok(match kind {
        COMPUTER_NAME_NETBIOS | COMPUTER_NAME_PHYSICAL_NETBIOS => host
            .chars()
            .take(MAX_COMPUTERNAME_LENGTH)
            .flat_map(char::to_uppercase)
            .take(MAX_COMPUTERNAME_LENGTH)
            .collect(),
        COMPUTER_NAME_DNS_HOSTNAME | COMPUTER_NAME_PHYSICAL_DNS_HOSTNAME => host.into(),
        COMPUTER_NAME_DNS_DOMAIN | COMPUTER_NAME_PHYSICAL_DNS_DOMAIN => String::new(),
        COMPUTER_NAME_DNS_FULLY_QUALIFIED | COMPUTER_NAME_PHYSICAL_DNS_FULLY_QUALIFIED => host.into(),
        _ => return Err(ERROR_INVALID_PARAMETER),
    })
}

/// The result of the in/out size convention both functions share: `size`
/// is the buffer's capacity in characters *including the NUL*. `Ok(n)`: the
/// name (`n` characters) fits and `*size` becomes `n`; `Err(needed)`: it
/// does not and `*size` becomes `needed` (characters with the NUL). The
/// error to report is `ERROR_MORE_DATA` for `GetComputerNameExW` and
/// `ERROR_BUFFER_OVERFLOW` (111) for `GetComputerNameW`.
pub fn name_fits(name_len: usize, size: u32) -> Result<u32, u32> {
    if (size as usize) > name_len { Ok(name_len as u32) } else { Err(name_len as u32 + 1) }
}

// ----- geographic locations ----------------------------------------------------------------------------

/// `GEOCLASS_NATION`, `GEOCLASS_REGION` (`winnls.h`), `GEOID_NOT_AVAILABLE`.
pub const GEOCLASS_NATION: u32 = 16;
pub const GEOCLASS_REGION: u32 = 14;
pub const GEOID_NOT_AVAILABLE: i32 = -1;
/// The `GEOID` of the United States, and of its region, Northern America
/// (Learn, "Table of Geographical Locations").
pub const GEOID_UNITED_STATES: i32 = 244;
pub const GEOID_NORTHERN_AMERICA: i32 = 21;

/// `GetUserGeoID`: the user's location, the United States.
pub fn user_geo_id(class: u32) -> Option<i32> {
    match class {
        GEOCLASS_NATION => Some(GEOID_UNITED_STATES),
        GEOCLASS_REGION => Some(GEOID_NORTHERN_AMERICA),
        _ => None,
    }
}

/// `GEOTYPE` (`winnls.h`).
pub const GEO_NATION: u32 = 1;
pub const GEO_ISO2: u32 = 4;
pub const GEO_ISO3: u32 = 5;
pub const GEO_RFC1766: u32 = 6;
pub const GEO_FRIENDLYNAME: u32 = 8;
pub const GEO_OFFICIALNAME: u32 = 9;
pub const GEO_ISO_UN_NUMBER: u32 = 12;

/// `GetGeoInfoW` for the United States: the values Learn's table gives.
/// Other locations and the types not listed (coordinates, time zones,
/// official languages, parent) are not answered (`None`).
pub fn geo_info(geo_id: i32, geo_type: u32) -> Option<&'static str> {
    if geo_id != GEOID_UNITED_STATES {
        return None;
    }
    Some(match geo_type {
        GEO_NATION => "244",
        GEO_ISO2 => "US",
        GEO_ISO3 => "USA",
        GEO_RFC1766 => "en-US",
        GEO_FRIENDLYNAME => "United States",
        GEO_OFFICIALNAME => "United States of America",
        GEO_ISO_UN_NUMBER => "840",
        _ => return None,
    })
}

// ----- power status ----------------------------------------------------------------------------------------

/// `SYSTEM_POWER_STATUS` (`winbase.h`, 12 bytes) of a machine on AC power
/// with no battery: `ACLineStatus` 1 (online), `BatteryFlag` 128 (no system
/// battery), `BatteryLifePercent` 255 (unknown), `SystemStatusFlag` 0,
/// `BatteryLifeTime` and `BatteryFullLifeTime` `0xFFFFFFFF` (unknown).
pub fn power_status() -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0] = 1;
    b[1] = 128;
    b[2] = 255;
    b[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    b[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    b
}

// ----- locale names -------------------------------------------------------------------------------------------

/// `LOCALE_NAME_INVARIANT` is the empty string; `LOCALE_NAME_SYSTEM_DEFAULT`
/// (`winnls.h`).
pub const LOCALE_NAME_SYSTEM_DEFAULT: &str = "!x-sys-default-locale";

/// Is `name` a locale name Poly has (it has one locale, `en-US`)? NULL is
/// the user default (`LOCALE_NAME_USER_DEFAULT`), `""` the invariant
/// locale, `"en"` the neutral English locale; case does not matter.
pub fn locale_name_ok(name: Option<&[u16]>) -> bool {
    let Some(name) = name else { return true };
    let s: String = char::decode_utf16(name.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')).collect();
    s.is_empty()
        || s.eq_ignore_ascii_case("en-US")
        || s.eq_ignore_ascii_case("en")
        || s.eq_ignore_ascii_case(LOCALE_NAME_SYSTEM_DEFAULT)
}

// ----- HeapSetInformation ----------------------------------------------------------------------------------------

/// `HEAP_INFORMATION_CLASS` (`winnt.h`).
pub const HEAP_COMPATIBILITY_INFORMATION: u32 = 0;
pub const HEAP_ENABLE_TERMINATION_ON_CORRUPTION: u32 = 1;
pub const HEAP_OPTIMIZE_RESOURCES_INFORMATION: u32 = 3;

/// Check `HeapSetInformation`'s arguments (Learn): the compatibility value
/// is a `ULONG`, 0 (standard heap) or 2 (low-fragmentation heap);
/// termination on corruption takes no information (`NULL`, 0); the
/// resource optimisation takes `HEAP_OPTIMIZE_RESOURCES_INFORMATION`
/// (`Version` 1, `Flags` 0). The settings change nothing Poly's heap does.
pub fn heap_set_information(class: u32, info: Option<&[u8]>) -> Result<(), u32> {
    match (class, info) {
        (HEAP_ENABLE_TERMINATION_ON_CORRUPTION, None) => Ok(()),
        (HEAP_ENABLE_TERMINATION_ON_CORRUPTION, Some(_)) => Err(ERROR_INVALID_PARAMETER),
        (HEAP_COMPATIBILITY_INFORMATION, Some(b)) if b.len() == 4 => match u32::from_le_bytes(b.try_into().unwrap()) {
            0 | 2 => Ok(()),
            _ => Err(ERROR_INVALID_PARAMETER),
        },
        (HEAP_OPTIMIZE_RESOURCES_INFORMATION, Some(b)) if b.len() == 8 => {
            if u32::from_le_bytes(b[0..4].try_into().unwrap()) == 1
                && u32::from_le_bytes(b[4..8].try_into().unwrap()) == 0
            {
                Ok(())
            } else {
                Err(ERROR_INVALID_PARAMETER)
            }
        }
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

// ----- GetModuleHandleExW ----------------------------------------------------------------------------------------

/// `GET_MODULE_HANDLE_EX_FLAG_PIN`, `..._UNCHANGED_REFCOUNT`,
/// `..._FROM_ADDRESS` (`libloaderapi.h`).
pub const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 1;
pub const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 2;
pub const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 4;

/// What `GetModuleHandleExW`'s flags ask for. Learn: PIN and
/// UNCHANGED_REFCOUNT together are invalid, as are unknown bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleEx {
    pub pin: bool,
    pub unchanged: bool,
    pub from_address: bool,
}

pub fn module_ex_flags(flags: u32) -> Result<ModuleEx, u32> {
    if flags & !7 != 0 || (flags & 1 != 0 && flags & 2 != 0) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(ModuleEx { pin: flags & 1 != 0, unchanged: flags & 2 != 0, from_address: flags & 4 != 0 })
}

// ----- VirtualAlloc2 -------------------------------------------------------------------------------------------------

/// `MEM_EXTENDED_PARAMETER_TYPE` (`winnt.h`).
pub const MEM_EXTENDED_PARAMETER_INVALID_TYPE: u8 = 0;
pub const MEM_EXTENDED_PARAMETER_ADDRESS_REQUIREMENTS: u8 = 1;
pub const MEM_EXTENDED_PARAMETER_NUMA_NODE: u8 = 2;
pub const MEM_EXTENDED_PARAMETER_PARTITION_HANDLE: u8 = 3;
pub const MEM_EXTENDED_PARAMETER_USER_PHYSICAL_HANDLE: u8 = 4;
pub const MEM_EXTENDED_PARAMETER_ATTRIBUTE_FLAGS: u8 = 5;
pub const MEM_EXTENDED_PARAMETER_IMAGE_MACHINE: u8 = 6;
/// `sizeof(MEM_EXTENDED_PARAMETER)`: `Type` (low 8 bits) and `Reserved`
/// in the first 8 bytes, the union in the second.
pub const MEM_EXTENDED_PARAMETER_SIZE: usize = 16;
/// `sizeof(MEM_ADDRESS_REQUIREMENTS)`: `LowestStartingAddress`,
/// `HighestEndingAddress`, `Alignment`.
pub const MEM_ADDRESS_REQUIREMENTS_SIZE: usize = 24;

/// The allocation granularity: Learn: an alignment below it is rounded up.
pub const ALLOCATION_GRANULARITY: usize = 0x1_0000;
/// The page size.
pub const PAGE_SIZE: usize = 0x1000;

/// What the extended parameters of one call ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocRequest {
    /// 0: no lower bound.
    pub lowest: usize,
    /// 0: no upper bound; else the last byte allowed (`HighestEndingAddress`).
    pub highest: usize,
    /// 0: the allocation granularity; else a power of two.
    pub alignment: usize,
}

/// Read the `count` `MEM_EXTENDED_PARAMETER`s in `raw` (which `u64` pointer
/// values at offset 8 read as `read_ptr`: the address requirements are
/// behind a pointer). `ERROR_INVALID_PARAMETER` for an invalid type or
/// address requirements, `ERROR_NOT_SUPPORTED` for partitions and
/// user-physical pages; NUMA nodes, attribute flags and the image machine
/// are accepted and ignored.
pub fn alloc_request(
    raw: &[u8],
    count: usize,
    read_requirements: &dyn Fn(u64) -> Option<[u8; MEM_ADDRESS_REQUIREMENTS_SIZE]>,
) -> Result<AllocRequest, u32> {
    let mut req = AllocRequest::default();
    if count == 0 {
        return Ok(req);
    }
    if raw.len() < count * MEM_EXTENDED_PARAMETER_SIZE {
        return Err(ERROR_INVALID_PARAMETER);
    }
    for i in 0..count {
        let p = &raw[i * MEM_EXTENDED_PARAMETER_SIZE..][..MEM_EXTENDED_PARAMETER_SIZE];
        let value = u64::from_le_bytes(p[8..16].try_into().unwrap());
        match p[0] {
            MEM_EXTENDED_PARAMETER_ADDRESS_REQUIREMENTS => {
                let r = read_requirements(value).ok_or(ERROR_INVALID_PARAMETER)?;
                let get = |at: usize| u64::from_le_bytes(r[at..at + 8].try_into().unwrap()) as usize;
                req.lowest = get(0);
                req.highest = get(8);
                req.alignment = get(16);
                if req.alignment != 0 && !req.alignment.is_power_of_two() {
                    return Err(ERROR_INVALID_PARAMETER);
                }
                if req.highest != 0 && !(req.highest + 1).is_multiple_of(PAGE_SIZE) {
                    return Err(ERROR_INVALID_PARAMETER);
                }
                if !req.lowest.is_multiple_of(PAGE_SIZE) || (req.highest != 0 && req.lowest > req.highest) {
                    return Err(ERROR_INVALID_PARAMETER);
                }
            }
            MEM_EXTENDED_PARAMETER_NUMA_NODE
            | MEM_EXTENDED_PARAMETER_ATTRIBUTE_FLAGS
            | MEM_EXTENDED_PARAMETER_IMAGE_MACHINE => {}
            MEM_EXTENDED_PARAMETER_PARTITION_HANDLE | MEM_EXTENDED_PARAMETER_USER_PHYSICAL_HANDLE => {
                return Err(ERROR_NOT_SUPPORTED);
            }
            _ => return Err(ERROR_INVALID_PARAMETER),
        }
    }
    Ok(req)
}

/// The alignment actually used: at least the allocation granularity.
pub fn effective_alignment(req: &AllocRequest) -> usize {
    req.alignment.max(ALLOCATION_GRANULARITY)
}

/// The lowest address at or above `base` that is a multiple of `alignment`.
pub fn align_up(base: usize, alignment: usize) -> usize {
    base.div_ceil(alignment) * alignment
}

/// Does the range `[at, at + size)` obey `req`?
pub fn range_ok(req: &AllocRequest, at: usize, size: usize) -> bool {
    at.is_multiple_of(effective_alignment(req))
        && at >= req.lowest
        && (req.highest == 0 || at.checked_add(size).is_some_and(|end| end - 1 <= req.highest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn legacy_topology() {
        let t = Topology { cpus: 4 }.legacy();
        assert_eq!(t.len(), (4 + 2) * LPI_SIZE);
        for i in 0..4 {
            let e = &t[i * 32..][..32];
            assert_eq!(u64_at(e, 0), 1 << i);
            assert_eq!(u32::from_le_bytes(e[8..12].try_into().unwrap()), RELATION_PROCESSOR_CORE);
            assert_eq!(e[16], 0, "no SMT");
        }
        assert_eq!(u64_at(&t, 4 * 32), 0b1111);
        assert_eq!(u32::from_le_bytes(t[4 * 32 + 8..][..4].try_into().unwrap()), RELATION_NUMA_NODE);
        assert_eq!(u32::from_le_bytes(t[5 * 32 + 8..][..4].try_into().unwrap()), RELATION_PROCESSOR_PACKAGE);
        // More than 64 processors: the first group only.
        let big = Topology { cpus: 100 }.legacy();
        assert_eq!(big.len(), (64 + 2) * LPI_SIZE);
        assert_eq!(u64_at(&big, 64 * 32), u64::MAX);
    }

    /// Walk an `Ex` buffer: (relationship, size) of each entry.
    fn entries(b: &[u8]) -> Vec<(u32, usize)> {
        let (mut out, mut at) = (Vec::new(), 0);
        while at < b.len() {
            let r = u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
            let s = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
            out.push((r, s));
            at += s;
        }
        assert_eq!(at, b.len());
        out
    }

    #[test]
    fn ex_topology() {
        let t = Topology { cpus: 2 };
        let cores = t.ex(RELATION_PROCESSOR_CORE).unwrap();
        assert_eq!(entries(&cores), [(0, 48), (0, 48)]);
        assert_eq!(u16::from_le_bytes(cores[30..32].try_into().unwrap()), 1, "GroupCount");
        assert_eq!(u64_at(&cores, 32), 1, "GroupMask.Mask of CPU 0");
        assert_eq!(u64_at(&cores, 48 + 32), 2, "of CPU 1");
        let all = t.ex(RELATION_ALL).unwrap();
        assert_eq!(entries(&all), [(0, 48), (0, 48), (1, 48), (3, 48), (4, 80)]);
        let group = &all[all.len() - 80..];
        assert_eq!(u16::from_le_bytes(group[8..10].try_into().unwrap()), 1);
        assert_eq!(group[32], 2, "MaximumProcessorCount");
        assert_eq!(group[33], 2, "ActiveProcessorCount");
        assert_eq!(u64_at(group, 32 + 40), 0b11);
        assert_eq!(entries(&t.ex(RELATION_NUMA_NODE).unwrap()), [(1, 48)]);
        assert_eq!(t.ex(RELATION_CACHE).unwrap(), Vec::<u8>::new());
        assert_eq!(t.ex(7).unwrap(), Vec::<u8>::new());
        assert_eq!(t.ex(9), Err(ERROR_INVALID_PARAMETER));
        // 70 processors: two groups; processor 65 is bit 1 of group 1.
        let t = Topology { cpus: 70 };
        let cores = t.ex(RELATION_PROCESSOR_CORE).unwrap();
        let last = &cores[65 * 48..][..48];
        assert_eq!(u64_at(last, 32), 2);
        assert_eq!(u16::from_le_bytes(last[40..42].try_into().unwrap()), 1, "group");
        let pkg = t.ex(RELATION_PROCESSOR_PACKAGE).unwrap();
        assert_eq!(pkg.len(), 32 + 2 * 16);
        assert_eq!(u64_at(&pkg, 32 + 16), (1 << 6) - 1);
        assert_eq!(fits(10, 20), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(fits(20, 20), Ok(()));
    }

    #[test]
    fn computer_names() {
        assert_eq!(computer_name(0, "Poly-Machine-Long-Name").unwrap(), "POLY-MACHINE-LO");
        assert_eq!(computer_name(1, "Poly").unwrap(), "Poly");
        assert_eq!(computer_name(2, "Poly").unwrap(), "");
        assert_eq!(computer_name(3, "Poly").unwrap(), "Poly");
        assert_eq!(computer_name(4, "poly").unwrap(), "POLY");
        assert_eq!(computer_name(8, "poly"), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(name_fits(4, 5), Ok(4));
        assert_eq!(name_fits(4, 4), Err(5));
        assert_eq!(name_fits(0, 1), Ok(0));
        assert_eq!(name_fits(0, 0), Err(1));
    }

    #[test]
    fn geo_and_power() {
        assert_eq!(user_geo_id(GEOCLASS_NATION), Some(244));
        assert_eq!(user_geo_id(GEOCLASS_REGION), Some(21));
        assert_eq!(user_geo_id(3), None);
        assert_eq!(geo_info(244, GEO_ISO2), Some("US"));
        assert_eq!(geo_info(244, GEO_ISO3), Some("USA"));
        assert_eq!(geo_info(94, GEO_ISO2), None);
        assert_eq!(geo_info(244, 2), None);
        let p = power_status();
        assert_eq!((p[0], p[1], p[2], p[3]), (1, 128, 255, 0));
        assert_eq!(&p[4..], &[0xFF; 8]);
    }

    #[test]
    fn names_and_heap_info() {
        let w = crate::wide;
        assert!(locale_name_ok(None));
        assert!(locale_name_ok(Some(&w(""))));
        assert!(locale_name_ok(Some(&w("EN-us"))));
        assert!(locale_name_ok(Some(&w("!x-sys-default-locale"))));
        assert!(!locale_name_ok(Some(&w("de-DE"))));
        assert_eq!(heap_set_information(1, None), Ok(()));
        assert_eq!(heap_set_information(1, Some(&[0])), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(heap_set_information(0, Some(&2u32.to_le_bytes())), Ok(()));
        assert_eq!(heap_set_information(0, Some(&1u32.to_le_bytes())), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(heap_set_information(0, Some(&[2, 0])), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(heap_set_information(3, Some(&[1, 0, 0, 0, 0, 0, 0, 0])), Ok(()));
        assert_eq!(heap_set_information(3, Some(&[2, 0, 0, 0, 0, 0, 0, 0])), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(heap_set_information(7, None), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(module_ex_flags(0).unwrap(), ModuleEx { pin: false, unchanged: false, from_address: false });
        assert_eq!(module_ex_flags(6).unwrap(), ModuleEx { pin: false, unchanged: true, from_address: true });
        assert_eq!(module_ex_flags(3), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(module_ex_flags(8), Err(ERROR_INVALID_PARAMETER));
    }

    fn param(kind: u8, value: u64) -> [u8; 16] {
        let mut p = [0u8; 16];
        p[0] = kind;
        p[8..16].copy_from_slice(&value.to_le_bytes());
        p
    }

    fn requirements(low: u64, high: u64, align: u64) -> [u8; 24] {
        let mut r = [0u8; 24];
        r[0..8].copy_from_slice(&low.to_le_bytes());
        r[8..16].copy_from_slice(&high.to_le_bytes());
        r[16..24].copy_from_slice(&align.to_le_bytes());
        r
    }

    #[test]
    fn extended_parameters() {
        let none = |_| None;
        assert_eq!(alloc_request(&[], 0, &none), Ok(AllocRequest::default()));
        let r = requirements(0x1000_0000, 0x1FFF_FFFF, 0x20_0000);
        let read = move |p: u64| (p == 77).then_some(r);
        let raw = param(MEM_EXTENDED_PARAMETER_ADDRESS_REQUIREMENTS, 77);
        let req = alloc_request(&raw, 1, &read).unwrap();
        assert_eq!(req, AllocRequest { lowest: 0x1000_0000, highest: 0x1FFF_FFFF, alignment: 0x20_0000 });
        assert!(range_ok(&req, 0x1020_0000, 0x10_0000));
        assert!(!range_ok(&req, 0x1010_0000, 0x10_0000), "alignment");
        assert!(!range_ok(&req, 0x1FE0_0000, 0x40_0000), "past the highest address");
        assert!(!range_ok(&req, 0x0, 0x10_0000), "below the lowest");
        assert_eq!(effective_alignment(&AllocRequest::default()), 0x1_0000);
        assert_eq!(align_up(0x1_2345, 0x1_0000), 0x2_0000);
        assert_eq!(align_up(0x2_0000, 0x1_0000), 0x2_0000);
        // Errors.
        assert_eq!(alloc_request(&raw, 1, &none), Err(ERROR_INVALID_PARAMETER), "unreadable requirements");
        let bad = move |_| Some(requirements(0, 0, 3 << 16));
        assert_eq!(alloc_request(&raw, 1, &bad), Err(ERROR_INVALID_PARAMETER), "alignment not a power of two");
        let bad = move |_| Some(requirements(0, 0xFFE, 0));
        assert_eq!(alloc_request(&raw, 1, &bad), Err(ERROR_INVALID_PARAMETER), "highest not end of page");
        assert_eq!(alloc_request(&param(0, 0), 1, &none), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(alloc_request(&param(3, 0), 1, &none), Err(ERROR_NOT_SUPPORTED));
        assert_eq!(alloc_request(&param(2, 1), 1, &none), Ok(AllocRequest::default()));
        assert_eq!(alloc_request(&param(5, 1), 1, &none), Ok(AllocRequest::default()));
        assert_eq!(alloc_request(&raw, 2, &read), Err(ERROR_INVALID_PARAMETER), "short array");
    }
}
