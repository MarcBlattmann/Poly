//! The Windows version Poly reports through `RtlGetVersion`: Windows 10,
//! build 19045, the same version the default registry reports
//! (`poly_registry::defaults::BUILD`, ADR 0066).
//!
//! `RTL_OSVERSIONINFOW` and `RTL_OSVERSIONINFOEXW` are `wdm.h`'s layouts
//! (`dwOSVersionInfoSize`, `dwMajorVersion`, `dwMinorVersion`,
//! `dwBuildNumber`, `dwPlatformId`, `szCSDVersion[128]`, then for the `EX`
//! form `wServicePackMajor`, `wServicePackMinor`, `wSuiteMask`,
//! `wProductType`, `wReserved`). Learn's `RtlGetVersion` page: the caller
//! sets `dwOSVersionInfoSize` to the size of the structure it passes, and
//! the function "returns STATUS_SUCCESS".

use poly_ntobj::NtStatus;

pub const MAJOR: u32 = 10;
pub const MINOR: u32 = 0;
pub const BUILD: u32 = 19045;
/// `VER_PLATFORM_WIN32_NT` (`winnt.h`).
pub const PLATFORM_WIN32_NT: u32 = 2;
/// `VER_SUITE_SINGLEUSERTS` (`winnt.h`): one interactive session.
pub const SUITE_SINGLEUSERTS: u16 = 0x0100;
/// `VER_NT_WORKSTATION` (`winnt.h`).
pub const NT_WORKSTATION: u8 = 1;

/// `sizeof(RTL_OSVERSIONINFOW)`.
pub const INFO_SIZE: u32 = 276;
/// `sizeof(RTL_OSVERSIONINFOEXW)`.
pub const INFO_EX_SIZE: u32 = 284;

/// `STATUS_INVALID_PARAMETER`: a size that is neither structure's (to be
/// confirmed by the spec agent, ADR 0071).
pub const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);

/// `RtlGetVersion`: fill in `info`, whose first four bytes give its size.
///
/// # Safety
/// `info` is valid for reading 4 bytes and for writing the size they give,
/// if that is one of the two structures' sizes.
pub unsafe fn get_version(info: *mut u8) -> NtStatus {
    // SAFETY: the caller's contract: the size field is readable.
    let size = unsafe { (info as *const u32).read_unaligned() };
    if size != INFO_SIZE && size != INFO_EX_SIZE {
        return STATUS_INVALID_PARAMETER;
    }
    let mut out = [0u8; INFO_EX_SIZE as usize];
    out[0..4].copy_from_slice(&size.to_le_bytes());
    out[4..8].copy_from_slice(&MAJOR.to_le_bytes());
    out[8..12].copy_from_slice(&MINOR.to_le_bytes());
    out[12..16].copy_from_slice(&BUILD.to_le_bytes());
    out[16..20].copy_from_slice(&PLATFORM_WIN32_NT.to_le_bytes());
    // szCSDVersion[128] at 20: empty (no service pack).
    // wServicePackMajor (276) and wServicePackMinor (278) stay 0.
    out[280..282].copy_from_slice(&SUITE_SINGLEUSERTS.to_le_bytes());
    out[282] = NT_WORKSTATION;
    // SAFETY: the caller's contract: `size` bytes are writable.
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), info, size as usize) };
    NtStatus(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_forms() {
        let mut ex = [0xEEu8; 300];
        ex[0..4].copy_from_slice(&INFO_EX_SIZE.to_le_bytes());
        // SAFETY: the buffer is larger than the structure.
        assert_eq!(unsafe { get_version(ex.as_mut_ptr()) }, NtStatus(0));
        assert_eq!(u32::from_le_bytes(ex[4..8].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(ex[12..16].try_into().unwrap()), 19045);
        assert_eq!(u32::from_le_bytes(ex[16..20].try_into().unwrap()), 2);
        assert_eq!(&ex[20..22], &[0, 0]);
        assert_eq!(&ex[280..284], &[0x00, 0x01, 1, 0]);
        assert_eq!(ex[284], 0xEE, "nothing past the structure");

        let mut plain = [0xEEu8; 284];
        plain[0..4].copy_from_slice(&INFO_SIZE.to_le_bytes());
        // SAFETY: as above.
        assert_eq!(unsafe { get_version(plain.as_mut_ptr()) }, NtStatus(0));
        assert_eq!(plain[276], 0xEE);

        let mut bad = [0u8; 8];
        bad[0] = 8;
        // SAFETY: as above.
        assert_eq!(unsafe { get_version(bad.as_mut_ptr()) }, STATUS_INVALID_PARAMETER);
    }
}
