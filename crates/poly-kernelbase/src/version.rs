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
//! versions (to be confirmed by the spec agent, ADR 0076).

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
