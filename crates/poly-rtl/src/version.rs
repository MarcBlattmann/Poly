//! The Windows version Poly reports through `RtlGetVersion`: Windows 11,
//! build 26200, the same version the default registry reports
//! (`poly_registry::defaults::BUILD`, ADR 0066, ADR 0189).
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
pub const BUILD: u32 = 26200;
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

/// `STATUS_NO_MORE_ENTRIES`: what `RtlGetVersion` answers for a size field of 300 (after writing
/// the extended layout; `specs/windows/ntdll.dll/RtlGetVersion.yaml`).
pub const STATUS_NO_MORE_ENTRIES: NtStatus = NtStatus(0x8000_001A);

/// `RtlGetVersion`: fill in `info`, whose first four bytes give its size.
///
/// Unlike `GetVersionEx`, the size is not checked (observed on Windows,
/// `specs/windows/ntdll.dll/RtlGetVersion.yaml`): the size 284 gets the extended layout, the size
/// 300 the extended layout and `STATUS_NO_MORE_ENTRIES`, every other size, also 0, the basic
/// layout. The basic layout is the version, the platform and the first unit of an empty service
/// pack string; the extended one adds the service pack numbers (0.0), the suite mask (0) and the
/// product type (workstation), but not `wReserved`.
///
/// # Safety
/// `info` is valid for reading 4 bytes and for writing 22 bytes, and for 283 bytes when the size
/// field says 284 or 300.
pub unsafe fn get_version(info: *mut u8) -> NtStatus {
    // SAFETY: the caller's contract: the size field is readable.
    let size = unsafe { (info as *const u32).read_unaligned() };
    let mut out = [0u8; INFO_EX_SIZE as usize];
    out[4..8].copy_from_slice(&MAJOR.to_le_bytes());
    out[8..12].copy_from_slice(&MINOR.to_le_bytes());
    out[12..16].copy_from_slice(&BUILD.to_le_bytes());
    out[16..20].copy_from_slice(&PLATFORM_WIN32_NT.to_le_bytes());
    // szCSDVersion[128] at 20: empty (no service pack); only its first unit is written.
    let extended = size == INFO_EX_SIZE || size == EXTENDED_STATUS_SIZE;
    // The extended fields: wServicePackMajor (276), wServicePackMinor (278) and wSuiteMask (280)
    // are 0, wProductType (282) is VER_NT_WORKSTATION.
    out[282] = NT_WORKSTATION;
    // SAFETY: the caller's contract: 22 bytes (283 for the extended layout) are writable.
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr().add(4), info.add(4), 18);
        if extended {
            core::ptr::copy_nonoverlapping(out.as_ptr().add(276), info.add(276), 7);
        }
    }
    if size == EXTENDED_STATUS_SIZE { STATUS_NO_MORE_ENTRIES } else { NtStatus(0) }
}

/// The size field for which `RtlGetVersion` writes the extended layout and answers
/// [`STATUS_NO_MORE_ENTRIES`].
const EXTENDED_STATUS_SIZE: u32 = 300;

/// `RtlGetNtVersionNumbers`: the major and minor version and the build number with the high
/// nibble 0xF (a "free build" marker), each through its pointer (null ones are skipped).
///
/// # Safety
/// Each pointer is null or valid for writing 4 bytes.
pub unsafe fn get_nt_version_numbers(major: *mut u32, minor: *mut u32, build: *mut u32) {
    // SAFETY: the caller's contract.
    unsafe {
        if !major.is_null() {
            major.write_unaligned(MAJOR);
        }
        if !minor.is_null() {
            minor.write_unaligned(MINOR);
        }
        if !build.is_null() {
            build.write_unaligned(BUILD | 0xF000_0000);
        }
    }
}

/// The update build revision (`UBR`) Poly reports with [`BUILD`]: the fourth part of
/// `10.0.26200.9974` (`poly_registry::defaults`, ADR 0189).
pub const UPDATE: u32 = 9974;

/// `DEVICEFAMILYINFOENUM_*` (`winnt.h`): the device family. Poly is a desktop.
pub const DEVICEFAMILYINFOENUM_DESKTOP: u32 = 3;
/// `DEVICEFAMILYDEVICEFORM_*` (`winnt.h`): the form of the device; Poly does not know it
/// (`DEVICEFAMILYDEVICEFORM_UNKNOWN`, which Learn documents as "not set").
pub const DEVICEFAMILYDEVICEFORM_UNKNOWN: u32 = 0;

/// `RtlGetDeviceFamilyInfoEnum(pullUAPInfo, pulDeviceFamily, pulDeviceForm)`: the universal
/// application platform version, the device family and the device form, each through its
/// pointer (null ones are skipped). The UAP version is the version Poly reports as one 64-bit
/// number, `major << 48 | minor << 32 | build << 16 | revision`.
///
/// # Safety
/// Each pointer is null or valid for writing its type.
pub unsafe fn get_device_family_info_enum(uap: *mut u64, family: *mut u32, form: *mut u32) {
    // SAFETY: the caller's contract.
    unsafe {
        if !uap.is_null() {
            uap.write_unaligned(uap_info());
        }
        if !family.is_null() {
            family.write_unaligned(DEVICEFAMILYINFOENUM_DESKTOP);
        }
        if !form.is_null() {
            form.write_unaligned(DEVICEFAMILYDEVICEFORM_UNKNOWN);
        }
    }
}

/// The UAP version of [`get_device_family_info_enum`].
pub const fn uap_info() -> u64 {
    ((MAJOR as u64) << 48) | ((MINOR as u64) << 32) | ((BUILD as u64) << 16) | UPDATE as u64
}

/// `ERROR_INSUFFICIENT_BUFFER` (`winerror.h`).
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// The family and form names `RtlConvertDeviceFamilyInfoToString` gives for the values this
/// module reports: `Windows.Desktop` and `Unknown`.
pub const DEVICE_FAMILY_NAME: &str = "Windows.Desktop";
pub const DEVICE_FORM_NAME: &str = "Unknown";

/// `RtlConvertDeviceFamilyInfoToString(pulDeviceFamilyBufferSize, pulDeviceFormBufferSize,
/// DeviceFamily, DeviceForm)`: the names of the family and the form of this system as NUL-terminated
/// UTF-16 strings. The sizes are in bytes, with the NUL: on input what the buffers hold, on output
/// what the strings need. 0 on success; `ERROR_INSUFFICIENT_BUFFER` when a buffer is too small
/// (the sizes then say what is needed and nothing is written to that buffer).
///
/// # Safety
/// The size pointers are valid for reading and writing a `u32`; each buffer is null or valid for
/// writing the number of bytes its size says.
pub unsafe fn convert_device_family_info_to_string(
    family_size: *mut u32,
    form_size: *mut u32,
    family: *mut u16,
    form: *mut u16,
) -> u32 {
    if family_size.is_null() || form_size.is_null() {
        return ERROR_INSUFFICIENT_BUFFER;
    }
    let mut result = 0;
    // SAFETY: the caller's contract.
    unsafe {
        for (size, buf, text) in [(family_size, family, DEVICE_FAMILY_NAME), (form_size, form, DEVICE_FORM_NAME)] {
            let need = ((text.len() + 1) * 2) as u32;
            let have = size.read_unaligned();
            size.write_unaligned(need);
            if buf.is_null() || have < need {
                result = ERROR_INSUFFICIENT_BUFFER;
                continue;
            }
            for (i, c) in text.encode_utf16().enumerate() {
                buf.add(i).write_unaligned(c);
            }
            buf.add(text.len()).write_unaligned(0);
        }
    }
    result
}

/// Bit 63 of every condition mask `VerSetConditionMask` returns: the call-site trace of Chrome on
/// Windows 11 shows `0x8000000000000018` for `VerSetConditionMask(0, VER_MAJORVERSION,
/// VER_GREATER_EQUAL)` (the three bits of the condition at bit 3, and this one).
pub const CONDITION_MASK_FLAG: u64 = 1 << 63;

/// `VerSetConditionMask`: three bits per version field, at the position of
/// the field's type bit (bit 0 for the minor version ... bit 7 for the
/// product type), and [`CONDITION_MASK_FLAG`].
pub fn set_condition(mask: u64, type_mask: u32, condition: u8) -> u64 {
    let mut m = mask | CONDITION_MASK_FLAG;
    for bit in 0..8 {
        if type_mask & (1 << bit) != 0 {
            m = (m & !(7u64 << (3 * bit))) | (u64::from(condition & 7) << (3 * bit));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    #[test]
    fn both_forms() {
        let mut ex = [0xAAu8; 300];
        ex[0..4].copy_from_slice(&INFO_EX_SIZE.to_le_bytes());
        // SAFETY: the buffer is larger than the structure.
        assert_eq!(unsafe { get_version(ex.as_mut_ptr()) }, NtStatus(0));
        assert_eq!(u32::from_le_bytes(ex[4..8].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(ex[12..16].try_into().unwrap()), 26200);
        assert_eq!(u32::from_le_bytes(ex[16..20].try_into().unwrap()), 2);
        assert_eq!(&ex[20..22], &[0, 0]);
        // Only the first unit of the string is written; the extended fields, but not wReserved.
        assert_eq!(&ex[22..24], &[0xAA, 0xAA]);
        assert_eq!(&ex[276..284], &[0, 0, 0, 0, 0, 0, 1, 0xAA]);
        assert_eq!(ex[284], 0xAA, "nothing past the structure");

        // Any other size gets the basic layout, the size is not checked.
        for size in [0u32, 148, 275, 276, 285] {
            let mut b = [0xAAu8; 300];
            b[0..4].copy_from_slice(&size.to_le_bytes());
            // SAFETY: as above.
            assert_eq!(unsafe { get_version(b.as_mut_ptr()) }, NtStatus(0), "{size}");
            assert_eq!(&b[20..22], &[0, 0], "{size}");
            assert_eq!(&b[22..], &[0xAA; 278][..], "{size}");
        }
        // 300 writes the extended layout and says STATUS_NO_MORE_ENTRIES.
        let mut b = [0xAAu8; 304];
        b[0..4].copy_from_slice(&300u32.to_le_bytes());
        // SAFETY: as above.
        assert_eq!(unsafe { get_version(b.as_mut_ptr()) }, STATUS_NO_MORE_ENTRIES);
        assert_eq!(b[282], 1);
    }

    #[test]
    fn nt_version_numbers_carry_the_free_build_nibble() {
        let (mut a, mut b, mut c, mut d) = (0u32, 0u32, 0u32, 0x1234_5678u32);
        // SAFETY: valid locals.
        unsafe { get_nt_version_numbers(&mut a, &mut b, &mut c) };
        assert_eq!((a, b, c), (10, 0, 26200 | 0xF000_0000));
        assert_eq!(d, 0x1234_5678);
        // SAFETY: null pointers are skipped.
        unsafe { get_nt_version_numbers(core::ptr::null_mut(), &mut d, core::ptr::null_mut()) };
        assert_eq!(d, 0);
    }

    #[test]
    fn device_family_is_a_desktop_of_the_reported_version() {
        let (mut uap, mut family, mut form) = (0u64, 0u32, 99u32);
        // SAFETY: valid locals.
        unsafe { get_device_family_info_enum(&mut uap, &mut family, &mut form) };
        assert_eq!(uap, 0x000A_0000_6658_26F6);
        assert_eq!((family, form), (3, 0));
        // Null outputs are skipped.
        // SAFETY: as above.
        unsafe { get_device_family_info_enum(core::ptr::null_mut(), &mut family, core::ptr::null_mut()) };
    }

    #[test]
    fn device_family_names() {
        let (mut fs, mut os) = (32u32, 16u32);
        let (mut f, mut o) = ([0xAAAAu16; 16], [0xAAAAu16; 8]);
        // SAFETY: buffers as large as their sizes say.
        let r = unsafe { convert_device_family_info_to_string(&mut fs, &mut os, f.as_mut_ptr(), o.as_mut_ptr()) };
        assert_eq!((r, fs, os), (0, 32, 16));
        assert_eq!(f[..16].to_vec(), "Windows.Desktop\0".encode_utf16().collect::<std::vec::Vec<_>>());
        assert_eq!(o[..8].to_vec(), "Unknown\0".encode_utf16().collect::<std::vec::Vec<_>>());
        // Too small: the size says what is needed, the buffer is left alone.
        let (mut fs, mut os) = (4u32, 16u32);
        let (mut f, mut o) = ([0xAAAAu16; 16], [0xAAAAu16; 8]);
        // SAFETY: as above.
        let r = unsafe { convert_device_family_info_to_string(&mut fs, &mut os, f.as_mut_ptr(), o.as_mut_ptr()) };
        assert_eq!((r, fs, os), (ERROR_INSUFFICIENT_BUFFER, 32, 16));
        assert_eq!(f[0], 0xAAAA);
        assert_eq!(o[0], u16::from(b'U'));
    }
}
