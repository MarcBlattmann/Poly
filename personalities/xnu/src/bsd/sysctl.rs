//! `sysctl` and `sysctlbyname`: the values a program and libSystem's start
//! ask for (the sysctl and sys_sysctlbyname sheets; numbers from the macOS
//! SDK `<sys/sysctl.h>`; the name-to-numbers node {0, 3} from the sheet).

use super::errno::*;
use alloc::string::String;
use alloc::vec::Vec;

/// `CTL_MAXNAME`.
pub const CTL_MAXNAME: usize = 12;
/// The longest dotted name accepted.
pub const MAX_NAME: usize = 1024;

const CTL_SYSCTL: i32 = 0;
const CTL_KERN: i32 = 1;
const CTL_HW: i32 = 6;

/// Facts about the machine the values come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct System {
    pub ncpu: u32,
    pub memsize: u64,
    pub page_size: u64,
    pub hostname: String,
    /// Boot time: seconds and microseconds since the epoch.
    pub boottime: (i64, i64),
    /// `hw.machine` (`x86_64`, `arm64`).
    pub machine: &'static str,
    /// Whether the program runs through the arm64 translator.
    pub translated: bool,
}

/// The Darwin release Poly answers as (the oracle's: macOS 27.0, Darwin
/// 27.0.0, build 26A428; ADR 0157 asks for 13 or later).
pub const OSRELEASE: &str = "27.0.0";
pub const OSPRODUCTVERSION: &str = "27.0";
pub const OSVERSION: &str = "26A428";
pub const VERSION: &str = "Darwin Kernel Version 27.0.0: Poly";

/// One value: its numbers (if it has any) and its dotted name.
struct Entry {
    mib: &'static [i32],
    name: &'static str,
    value: fn(&System) -> Vec<u8>,
}

fn int(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
fn quad(v: u64) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
fn string(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

const ENTRIES: &[Entry] = &[
    Entry { mib: &[CTL_KERN, 1], name: "kern.ostype", value: |_| string("Darwin") },
    Entry { mib: &[CTL_KERN, 2], name: "kern.osrelease", value: |_| string(OSRELEASE) },
    Entry { mib: &[CTL_KERN, 3], name: "kern.osrevision", value: |_| int(199_506) },
    Entry { mib: &[CTL_KERN, 4], name: "kern.version", value: |_| string(VERSION) },
    Entry { mib: &[CTL_KERN, 6], name: "kern.maxproc", value: |_| int(4000) },
    Entry { mib: &[CTL_KERN, 7], name: "kern.maxfiles", value: |_| int(122_880) },
    Entry { mib: &[CTL_KERN, 8], name: "kern.argmax", value: |_| int(1 << 20) },
    Entry { mib: &[CTL_KERN, 10], name: "kern.hostname", value: |s| string(&s.hostname) },
    Entry {
        mib: &[CTL_KERN, 21],
        name: "kern.boottime",
        value: |s| {
            let mut v = quad(s.boottime.0 as u64);
            v.extend_from_slice(&quad(s.boottime.1 as u64));
            v
        },
    },
    Entry { mib: &[CTL_KERN, 29], name: "kern.maxfilesperproc", value: |_| int(61_440) },
    Entry { mib: &[CTL_KERN, 65], name: "kern.osversion", value: |_| string(OSVERSION) },
    Entry { mib: &[], name: "kern.osproductversion", value: |_| string(OSPRODUCTVERSION) },
    Entry { mib: &[CTL_HW, 1], name: "hw.machine", value: |s| string(s.machine) },
    Entry { mib: &[CTL_HW, 3], name: "hw.ncpu", value: |s| int(s.ncpu) },
    Entry { mib: &[CTL_HW, 4], name: "hw.byteorder", value: |_| int(1234) },
    // By number a 4-byte integer, by name 8 bytes (the sheets).
    Entry { mib: &[CTL_HW, 7], name: "", value: |s| int(s.page_size as u32) },
    Entry { mib: &[], name: "hw.pagesize", value: |s| quad(s.page_size) },
    Entry { mib: &[], name: "hw.pagesize32", value: |s| int(s.page_size as u32) },
    Entry { mib: &[CTL_HW, 24], name: "hw.memsize", value: |s| quad(s.memsize) },
    Entry { mib: &[], name: "hw.logicalcpu", value: |s| int(s.ncpu) },
    Entry { mib: &[], name: "hw.physicalcpu", value: |s| int(s.ncpu) },
    Entry { mib: &[], name: "hw.activecpu", value: |s| int(s.ncpu) },
    Entry { mib: &[], name: "hw.logicalcpu_max", value: |s| int(s.ncpu) },
    Entry { mib: &[], name: "hw.physicalcpu_max", value: |s| int(s.ncpu) },
    Entry { mib: &[], name: "hw.cachelinesize", value: |_| quad(64) },
    // The mach_absolute_time unit: nanoseconds on x86-64, the 24 MHz
    // counter on arm64 (ADR 0238's timebase).
    Entry {
        mib: &[],
        name: "hw.tbfrequency",
        value: |s| quad(if s.machine == "arm64" { 24_000_000 } else { 1_000_000_000 }),
    },
    Entry { mib: &[], name: "vm.loadavg", value: |_| alloc::vec![0; 24] },
    Entry { mib: &[], name: "sysctl.proc_translated", value: |s| int(u32::from(s.translated)) },
];

/// The value for `mib`; `ENOENT` if there is none.
pub fn by_mib(sys: &System, mib: &[i32]) -> Result<Vec<u8>, u32> {
    if mib.len() < 2 || mib.len() > CTL_MAXNAME {
        return Err(EINVAL);
    }
    ENTRIES.iter().find(|e| !e.mib.is_empty() && e.mib == mib).map(|e| (e.value)(sys)).ok_or(ENOENT)
}

/// The value for the dotted `name`; `ENOENT` if there is none (a node such
/// as `kern` is not a value).
pub fn by_name(sys: &System, name: &[u8]) -> Result<Vec<u8>, u32> {
    ENTRIES.iter().find(|e| !e.name.is_empty() && e.name.as_bytes() == name).map(|e| (e.value)(sys)).ok_or(ENOENT)
}

/// `{CTL_SYSCTL, 3}`: the numbers for a dotted name, as 4-byte integers.
pub fn name_to_mib(name: &[u8]) -> Result<Vec<u8>, u32> {
    let e = ENTRIES.iter().find(|e| !e.mib.is_empty() && e.name.as_bytes() == name).ok_or(ENOENT)?;
    Ok(e.mib.iter().flat_map(|n| n.to_le_bytes()).collect())
}

/// Whether `mib` is the name-to-numbers node.
pub fn is_name_to_mib(mib: &[i32]) -> bool {
    mib == [CTL_SYSCTL, 3]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> System {
        System {
            ncpu: 4,
            memsize: 2 << 30,
            page_size: 4096,
            hostname: "poly".into(),
            machine: "x86_64",
            ..System::default()
        }
    }

    #[test]
    fn values_by_number() {
        assert_eq!(by_mib(&sys(), &[1, 1]).unwrap(), b"Darwin\0");
        assert_eq!(by_mib(&sys(), &[6, 7]).unwrap(), 4096u32.to_le_bytes());
        assert_eq!(by_mib(&sys(), &[6, 3]).unwrap(), 4u32.to_le_bytes());
        assert_eq!(by_mib(&sys(), &[6, 24]).unwrap().len(), 8);
        assert_eq!(by_mib(&sys(), &[1, 99999]), Err(ENOENT));
        assert_eq!(by_mib(&sys(), &[99, 1]), Err(ENOENT));
        assert_eq!(by_mib(&sys(), &[1]), Err(EINVAL));
        assert_eq!(by_mib(&sys(), &[0; 13]), Err(EINVAL));
    }

    #[test]
    fn values_by_name() {
        assert_eq!(by_name(&sys(), b"kern.ostype").unwrap(), b"Darwin\0");
        assert_eq!(by_name(&sys(), b"hw.pagesize").unwrap(), 4096u64.to_le_bytes());
        assert_eq!(by_name(&sys(), b"hw.logicalcpu").unwrap(), by_mib(&sys(), &[6, 3]).unwrap());
        assert_eq!(by_name(&sys(), b"vm.loadavg").unwrap().len(), 24);
        assert_eq!(by_name(&sys(), b"sysctl.proc_translated").unwrap(), 0u32.to_le_bytes());
        assert_eq!(by_name(&sys(), b"kern"), Err(ENOENT));
        assert_eq!(by_name(&sys(), b"kern.o"), Err(ENOENT));
    }

    #[test]
    fn names_to_numbers() {
        assert_eq!(name_to_mib(b"kern.ostype").unwrap(), [1, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(name_to_mib(b"hw.memsize").unwrap(), [6, 0, 0, 0, 24, 0, 0, 0]);
        assert_eq!(name_to_mib(b"kern.nosuch"), Err(ENOENT));
    }
}
