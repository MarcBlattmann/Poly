//! The version `GetVersion` / `GetVersionExW` report, and `SYSTEM_INFO`.
//!
//! The real version is [`poly_rtl::version`]'s (Windows 10, build 19045,
//! as `RtlGetVersion` and the default registry report). Learn's
//! `GetVersionExW` page ("Operating System Version", "Targeting your
//! application for Windows"): from Windows 8.1 on, an application that is
//! not *manifested* for Windows 8.1 or Windows 10 gets the Windows 8 value,
//! 6.2. Compatibility is declared by `supportedOS` GUIDs in the
//! executable's manifest (resource type `RT_MANIFEST`, 24, id 1); the GUIDs
//! are the ones that page lists. `RtlGetVersion` is not affected.
//!
//! Build numbers for the older versions Poly reports to unmanifested
//! programs (9200 for 6.2, 9600 for 6.3) are the released builds of those
//! versions (to be confirmed by the spec agent, ADR 0079).

use crate::error::ERROR_INSUFFICIENT_BUFFER;

/// What an executable's manifest declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    /// No manifest, or no `supportedOS` for 8.1 or 10.
    Legacy,
    /// Windows 8.1.
    Win81,
    /// Windows 10 (and 11).
    Win10,
}

/// `supportedOS Id` of Windows 10 / 11.
pub const GUID_WIN10: &str = "{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}";
/// `supportedOS Id` of Windows 8.1.
pub const GUID_WIN81: &str = "{1f676c76-80e1-4239-95bb-83d0f6d0da78}";

/// `(major, minor, build)` reported to a program with `compat`.
pub fn reported(compat: Compat) -> (u32, u32, u32) {
    use poly_rtl::version::{BUILD, MAJOR, MINOR};
    match compat {
        Compat::Win10 => (MAJOR, MINOR, BUILD),
        Compat::Win81 => (6, 3, 9600),
        Compat::Legacy => (6, 2, 9200),
    }
}

/// `GetVersion`'s value: Learn: the low word holds the major version (low
/// byte) and the minor version (high byte); the high word the build number
/// (bit 31 clear on NT).
pub fn get_version(v: (u32, u32, u32)) -> u32 {
    ((v.2 & 0x7FFF) << 16) | ((v.1 & 0xFF) << 8) | (v.0 & 0xFF)
}

/// Fill an `OSVERSIONINFOW` (276 bytes) or `OSVERSIONINFOEXW` (284 bytes,
/// `winnt.h`) whose size field says which it is. Another size fails
/// (Learn: "The function fails if you specify an invalid value for the
/// dwOSVersionInfoSize member"; the error is `ERROR_INSUFFICIENT_BUFFER`,
/// to be confirmed).
pub fn fill(buf: &mut [u8], v: (u32, u32, u32)) -> Result<(), u32> {
    let size = u32::from_le_bytes(buf.get(..4).ok_or(ERROR_INSUFFICIENT_BUFFER)?.try_into().unwrap());
    if size != poly_rtl::version::INFO_SIZE && size != poly_rtl::version::INFO_EX_SIZE {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    let size = size as usize;
    if buf.len() < size {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: `buf` holds `size` bytes, one of the two structures' sizes.
    unsafe { poly_rtl::version::get_version(buf.as_mut_ptr()) };
    buf[4..8].copy_from_slice(&v.0.to_le_bytes());
    buf[8..12].copy_from_slice(&v.1.to_le_bytes());
    buf[12..16].copy_from_slice(&v.2.to_le_bytes());
    Ok(())
}

/// What a manifest's text declares (UTF-8, or UTF-16 LE as some resource
/// compilers write it). GUIDs are compared without case.
pub fn manifest_compat(text: &[u8]) -> Compat {
    let ascii: alloc::vec::Vec<u8> = if text.len() >= 2 && (text[1] == 0 || text.starts_with(&[0xFF, 0xFE])) {
        text.chunks(2).map(|c| c[0].to_ascii_lowercase()).collect()
    } else {
        text.iter().map(|c| c.to_ascii_lowercase()).collect()
    };
    let has = |g: &str| ascii.windows(g.len()).any(|w| w == g.as_bytes());
    if has(GUID_WIN10) {
        Compat::Win10
    } else if has(GUID_WIN81) {
        Compat::Win81
    } else {
        Compat::Legacy
    }
}

/// `RT_MANIFEST`.
pub const RT_MANIFEST: u32 = 24;

/// Find the first manifest in a resource section (PE format: "The .rsrc
/// Section": three levels of `IMAGE_RESOURCE_DIRECTORY`, type, name,
/// language, then an `IMAGE_RESOURCE_DATA_ENTRY`). `rsrc` is the resource
/// directory's bytes, starting at its RVA. Returns the data's RVA and size.
pub fn find_manifest(rsrc: &[u8]) -> Option<(u32, u32)> {
    fn u16_at(b: &[u8], at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
    }
    fn u32_at(b: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
    }
    /// The entries of the directory at `at`: `(name or id, offset, is
    /// directory)`.
    fn entries(b: &[u8], at: usize) -> Option<impl Iterator<Item = (u32, u32, bool)> + '_> {
        let named = usize::from(u16_at(b, at + 12)?);
        let ids = usize::from(u16_at(b, at + 14)?);
        Some((0..named + ids).filter_map(move |i| {
            let e = at + 16 + i * 8;
            let name = u32_at(b, e)?;
            let off = u32_at(b, e + 4)?;
            Some((name, off & 0x7FFF_FFFF, off & 0x8000_0000 != 0))
        }))
    }
    let (_, types, dir) = entries(rsrc, 0)?.find(|&(n, _, _)| n == RT_MANIFEST)?;
    if !dir {
        return None;
    }
    let (_, names, dir) = entries(rsrc, types as usize)?.find(|&(n, _, _)| n & 0x8000_0000 == 0)?;
    if !dir {
        return None;
    }
    let (_, data, dir) = entries(rsrc, names as usize)?.next()?;
    if dir {
        return None;
    }
    Some((u32_at(rsrc, data as usize)?, u32_at(rsrc, data as usize + 4)?))
}

/// `SYSTEM_INFO` (`sysinfoapi.h`, x64, 48 bytes) for `cpus` processors:
/// `PROCESSOR_ARCHITECTURE_AMD64`, 4 KiB pages, 64 KiB allocation
/// granularity, Poly's user address range (ADR 0063), one bit per
/// processor, `PROCESSOR_AMD_X8664`. The processor level and revision are
/// Poly's fixed 6 and 0 until they come from CPUID (to be confirmed).
pub fn system_info(cpus: u32) -> [u8; 48] {
    let mut b = [0u8; 48];
    b[0..2].copy_from_slice(&9u16.to_le_bytes());
    b[4..8].copy_from_slice(&4096u32.to_le_bytes());
    b[8..16].copy_from_slice(&0x1_0000u64.to_le_bytes());
    b[16..24].copy_from_slice(&0x7FFF_FFFE_FFFFu64.to_le_bytes());
    let cpus = cpus.clamp(1, 64);
    let mask = if cpus == 64 { u64::MAX } else { (1u64 << cpus) - 1 };
    b[24..32].copy_from_slice(&mask.to_le_bytes());
    b[32..36].copy_from_slice(&cpus.to_le_bytes());
    b[36..40].copy_from_slice(&8664u32.to_le_bytes());
    b[40..44].copy_from_slice(&0x1_0000u32.to_le_bytes());
    b[44..46].copy_from_slice(&6u16.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    #[test]
    fn the_manifest_rule() {
        assert_eq!(reported(Compat::Win10), (10, 0, 19045));
        assert_eq!(reported(Compat::Legacy), (6, 2, 9200));
        assert_eq!(get_version((10, 0, 19045)), (19045 << 16) | 10);
        assert_eq!(get_version((6, 2, 9200)), (9200 << 16) | (2 << 8) | 6);
        let m = br#"<compatibility><application><supportedOS Id="{8E0F7A12-BFB3-4FE8-B9A5-48FD50A15A9A}"/></application></compatibility>"#;
        assert_eq!(manifest_compat(m), Compat::Win10);
        let m81: Vec<u8> = GUID_WIN81.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        assert_eq!(manifest_compat(&m81), Compat::Win81);
        assert_eq!(manifest_compat(b"<assembly/>"), Compat::Legacy);
    }

    #[test]
    fn version_structures() {
        let mut b = [0u8; 284];
        b[..4].copy_from_slice(&284u32.to_le_bytes());
        fill(&mut b, (6, 2, 9200)).unwrap();
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 6);
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 9200);
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 2);
        assert_eq!(b[282], 1);
        let mut s = [0u8; 276];
        s[..4].copy_from_slice(&276u32.to_le_bytes());
        assert!(fill(&mut s, (10, 0, 19045)).is_ok());
        let mut bad = [0u8; 300];
        bad[..4].copy_from_slice(&300u32.to_le_bytes());
        assert_eq!(fill(&mut bad, (10, 0, 1)), Err(ERROR_INSUFFICIENT_BUFFER));
    }

    fn dir(entries: &[(u32, u32)]) -> Vec<u8> {
        let mut d = std::vec![0u8; 16];
        d[14..16].copy_from_slice(&(entries.len() as u16).to_le_bytes());
        for &(n, o) in entries {
            d.extend_from_slice(&n.to_le_bytes());
            d.extend_from_slice(&o.to_le_bytes());
        }
        d
    }

    #[test]
    fn manifests_are_found_in_resources() {
        // Root (type 3: icon, 24: manifest) -> names (1) -> languages
        // (0x409) -> data entry.
        let mut r = dir(&[(3, 0x8000_0000 | 0x100), (24, 0x8000_0000 | 0x40)]);
        r.resize(0x40, 0);
        r.extend(dir(&[(1, 0x8000_0000 | 0x60)]));
        r.resize(0x60, 0);
        r.extend(dir(&[(0x409, 0x80)]));
        r.resize(0x80, 0);
        r.extend_from_slice(&0x5000u32.to_le_bytes());
        r.extend_from_slice(&0x1234u32.to_le_bytes());
        r.resize(0x100, 0);
        r.extend(dir(&[]));
        assert_eq!(find_manifest(&r), Some((0x5000, 0x1234)));
        assert_eq!(find_manifest(&r[..0x30]), None);
        assert_eq!(find_manifest(&dir(&[(3, 0x10)])), None);
        assert_eq!(find_manifest(&[]), None);
    }

    #[test]
    fn system_info_layout() {
        let b = system_info(4);
        assert_eq!(u16::from_le_bytes([b[0], b[1]]), 9);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 4096);
        assert_eq!(u64::from_le_bytes(b[24..32].try_into().unwrap()), 0xF);
        assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 0x1_0000);
        assert_eq!(u64::from_le_bytes(system_info(64)[24..32].try_into().unwrap()), u64::MAX);
        assert_eq!(u32::from_le_bytes(system_info(0)[32..36].try_into().unwrap()), 1);
    }
}

// ----- VerifyVersionInfoW (ADR 0155) ------------------------------------------

// `VER_*` type masks and comparison values (`winnt.h`; Learn:
// "VerSetConditionMask function", "VerifyVersionInfoW function").
pub const VER_MINORVERSION: u32 = 0x1;
pub const VER_MAJORVERSION: u32 = 0x2;
pub const VER_BUILDNUMBER: u32 = 0x4;
pub const VER_PLATFORMID: u32 = 0x8;
pub const VER_SERVICEPACKMINOR: u32 = 0x10;
pub const VER_SERVICEPACKMAJOR: u32 = 0x20;
pub const VER_SUITENAME: u32 = 0x40;
pub const VER_PRODUCT_TYPE: u32 = 0x80;
pub const VER_EQUAL: u8 = 1;
pub const VER_GREATER: u8 = 2;
pub const VER_GREATER_EQUAL: u8 = 3;
pub const VER_LESS: u8 = 4;
pub const VER_LESS_EQUAL: u8 = 5;
pub const VER_AND: u8 = 6;
pub const VER_OR: u8 = 7;
/// `VER_PLATFORM_WIN32_NT` and `VER_NT_WORKSTATION`.
pub const PLATFORM_WIN32_NT: u32 = 2;
pub const NT_WORKSTATION: u8 = 1;

/// `VerSetConditionMask`: three bits per version field, at the position of
/// the field's type bit (bit 0 for the minor version ... bit 7 for the
/// product type).
pub fn set_condition(mask: u64, type_mask: u32, condition: u8) -> u64 {
    let mut m = mask;
    for bit in 0..8 {
        if type_mask & (1 << bit) != 0 {
            m = (m & !(7u64 << (3 * bit))) | (u64::from(condition & 7) << (3 * bit));
        }
    }
    m
}

/// The version fields of the system, in `OSVERSIONINFOEXW`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub platform: u32,
    pub sp_major: u16,
    pub sp_minor: u16,
    pub suite: u16,
    pub product: u8,
}

impl Fields {
    /// What Poly is: `(major, minor, build)` as reported, NT, no service
    /// pack, a workstation.
    pub fn of(v: (u32, u32, u32)) -> Fields {
        Fields {
            major: v.0,
            minor: v.1,
            build: v.2,
            platform: PLATFORM_WIN32_NT,
            sp_major: 0,
            sp_minor: 0,
            suite: 0,
            product: NT_WORKSTATION,
        }
    }

    /// Read the wanted values from an `OSVERSIONINFOEXW` (284 bytes).
    pub fn from_bytes(b: &[u8]) -> Option<Fields> {
        let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
        let u16_at = |o: usize| Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?));
        Some(Fields {
            major: u32_at(4)?,
            minor: u32_at(8)?,
            build: u32_at(12)?,
            platform: u32_at(16)?,
            sp_major: u16_at(276)?,
            sp_minor: u16_at(278)?,
            suite: u16_at(280)?,
            product: *b.get(282)?,
        })
    }
}

fn holds(condition: u8, actual: i64, wanted: i64) -> bool {
    match condition {
        VER_EQUAL => actual == wanted,
        VER_GREATER => actual > wanted,
        VER_GREATER_EQUAL => actual >= wanted,
        VER_LESS => actual < wanted,
        VER_LESS_EQUAL => actual <= wanted,
        _ => false,
    }
}

/// `VerifyVersionInfoW`: does the system (`actual`) meet `wanted` under the
/// conditions in `condition_mask` for the fields `type_mask` names? The
/// major, minor, build and service pack fields are compared in that order
/// and the first that differs decides; when all of them are equal the
/// condition of the last one decides (equal passes `VER_EQUAL`,
/// `VER_GREATER_EQUAL` and `VER_LESS_EQUAL`). The platform and product type
/// are compared on their own, the suite mask with `VER_AND` (all bits) or
/// `VER_OR` (any). `None`: a condition not allowed for the field
/// (`ERROR_BAD_ARGUMENTS`). The order and the tie rule are to be confirmed
/// by the spec agent.
pub fn verify(actual: &Fields, wanted: &Fields, type_mask: u32, condition_mask: u64) -> Option<bool> {
    let cond = |bit: u32| ((condition_mask >> (3 * bit)) & 7) as u8;
    let ordered = [
        (VER_MAJORVERSION, 1, i64::from(actual.major), i64::from(wanted.major)),
        (VER_MINORVERSION, 0, i64::from(actual.minor), i64::from(wanted.minor)),
        (VER_BUILDNUMBER, 2, i64::from(actual.build), i64::from(wanted.build)),
        (VER_SERVICEPACKMAJOR, 5, i64::from(actual.sp_major), i64::from(wanted.sp_major)),
        (VER_SERVICEPACKMINOR, 4, i64::from(actual.sp_minor), i64::from(wanted.sp_minor)),
    ];
    let mut decided = None;
    let mut last = None;
    for (type_bit, index, a, w) in ordered {
        if type_mask & type_bit == 0 {
            continue;
        }
        let c = cond(index);
        if !(VER_EQUAL..=VER_LESS_EQUAL).contains(&c) {
            return None;
        }
        last = Some(c);
        if a != w && decided.is_none() {
            decided = Some(holds(c, a, w));
        }
    }
    let mut ok = match (decided, last) {
        (Some(d), _) => d,
        (None, Some(c)) => holds(c, 0, 0),
        (None, None) => true,
    };
    if type_mask & VER_PLATFORMID != 0 {
        let c = cond(3);
        if !(VER_EQUAL..=VER_LESS_EQUAL).contains(&c) {
            return None;
        }
        ok &= holds(c, i64::from(actual.platform), i64::from(wanted.platform));
    }
    if type_mask & VER_PRODUCT_TYPE != 0 {
        let c = cond(7);
        if !(VER_EQUAL..=VER_LESS_EQUAL).contains(&c) {
            return None;
        }
        ok &= holds(c, i64::from(actual.product), i64::from(wanted.product));
    }
    if type_mask & VER_SUITENAME != 0 {
        ok &= match cond(6) {
            VER_AND => actual.suite & wanted.suite == wanted.suite,
            VER_OR => wanted.suite == 0 || actual.suite & wanted.suite != 0,
            _ => return None,
        };
    }
    Some(ok)
}

#[cfg(test)]
mod verify_tests {
    use super::*;

    const WIN10: (u32, u32, u32) = (10, 0, 19045);

    fn want(major: u32, minor: u32) -> Fields {
        Fields { major, minor, ..Fields::of(WIN10) }
    }

    #[test]
    fn the_mask_holds_three_bits_per_field() {
        let m = set_condition(0, VER_MAJORVERSION, VER_GREATER_EQUAL);
        assert_eq!(m, 3 << 3);
        let m = set_condition(m, VER_MINORVERSION | VER_SERVICEPACKMAJOR, VER_EQUAL);
        assert_eq!(m, (3 << 3) | 1 | (1 << 15));
        // A later call replaces the condition.
        assert_eq!((set_condition(m, VER_MAJORVERSION, VER_LESS) >> 3) & 7, u64::from(VER_LESS));
    }

    #[test]
    fn at_least_a_version_is_the_usual_use() {
        let actual = Fields::of(WIN10);
        let mask =
            set_condition(set_condition(0, VER_MAJORVERSION, VER_GREATER_EQUAL), VER_MINORVERSION, VER_GREATER_EQUAL);
        let ask = |major, minor| verify(&actual, &want(major, minor), VER_MAJORVERSION | VER_MINORVERSION, mask);
        assert_eq!(ask(6, 3), Some(true), "10.0 is at least 6.3: the major version decides");
        assert_eq!(ask(10, 0), Some(true), "all equal, inclusive");
        assert_eq!(ask(10, 1), Some(false));
        assert_eq!(ask(11, 0), Some(false));
        let equal = set_condition(set_condition(0, VER_MAJORVERSION, VER_EQUAL), VER_MINORVERSION, VER_EQUAL);
        assert_eq!(verify(&actual, &want(10, 0), VER_MAJORVERSION | VER_MINORVERSION, equal), Some(true));
        assert_eq!(verify(&actual, &want(6, 3), VER_MAJORVERSION | VER_MINORVERSION, equal), Some(false));
        let greater = set_condition(0, VER_MAJORVERSION, VER_GREATER);
        assert_eq!(verify(&actual, &want(10, 0), VER_MAJORVERSION, greater), Some(false), "strictly");
    }

    #[test]
    fn platform_product_and_suite_are_tested_on_their_own() {
        let actual = Fields::of(WIN10);
        let mut w = want(10, 0);
        w.platform = 2;
        let eq = set_condition(0, VER_PLATFORMID | VER_PRODUCT_TYPE, VER_EQUAL);
        assert_eq!(verify(&actual, &w, VER_PLATFORMID | VER_PRODUCT_TYPE, eq), Some(true));
        w.product = 3;
        assert_eq!(verify(&actual, &w, VER_PRODUCT_TYPE, eq), Some(false));
        w.suite = 0x100;
        assert_eq!(verify(&actual, &w, VER_SUITENAME, set_condition(0, VER_SUITENAME, VER_AND)), Some(false));
        assert_eq!(verify(&actual, &w, VER_SUITENAME, set_condition(0, VER_SUITENAME, VER_EQUAL)), None);
    }

    #[test]
    fn a_wanted_structure_is_read_at_winnts_offsets() {
        let mut b = [0u8; 284];
        b[4..8].copy_from_slice(&10u32.to_le_bytes());
        b[16..20].copy_from_slice(&2u32.to_le_bytes());
        b[276..278].copy_from_slice(&1u16.to_le_bytes());
        b[282] = 1;
        let f = Fields::from_bytes(&b).unwrap();
        assert_eq!((f.major, f.platform, f.sp_major, f.product), (10, 2, 1, 1));
        assert_eq!(Fields::from_bytes(&b[..100]), None);
    }
}
