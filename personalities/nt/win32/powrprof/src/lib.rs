//! Our `powrprof.dll` (ADR 0155, step 9), which also hosts the API sets `api-ms-win-power-base`
//! and `api-ms-win-power-setting` (`poly_ldr::apiset`). Chrome's browser delay-loads
//! `CallNtPowerInformation` through the first contract and calls it about once a second; with
//! the contract absent the browser hangs (the ablation of the gap map).
//!
//! The machine Poly shows is a desktop on mains power without a battery and without power
//! schemes. Learn: "CallNtPowerInformation function (powerbase.h)",
//! "PowerDeterminePlatformRoleEx", "PowerGetActiveScheme", "PowerReadACValue",
//! "PowerReadDCValue"; the answers are `poly_kernelbase::session`'s. Not observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_kernelbase::session;

poly_win32_rt::process_heap_allocator!();

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemInfo(info: *mut u8);
}

/// `ERROR_FILE_NOT_FOUND` (`winerror.h`): there is no power scheme to read.
const ERROR_FILE_NOT_FOUND: u32 = 2;

/// The number of processors (`SYSTEM_INFO.dwNumberOfProcessors`, offset 32, `sysinfoapi.h`).
fn processors() -> u32 {
    let mut info = [0u8; 48];
    // SAFETY: a SYSTEM_INFO is 48 bytes on x64.
    unsafe { GetSystemInfo(info.as_mut_ptr()) };
    u32::from_le_bytes([info[32], info[33], info[34], info[35]])
}

/// `CallNtPowerInformation(InformationLevel, InputBuffer, InputBufferLength, OutputBuffer,
/// OutputBufferLength)`: an `NTSTATUS`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CallNtPowerInformation(
    level: u32,
    input: *const u8,
    input_len: u32,
    output: *mut u8,
    output_len: u32,
) -> u32 {
    let size = if output.is_null() { 0 } else { output_len as usize };
    match session::power_information(level, processors(), !input.is_null() && input_len > 0, size) {
        Ok(bytes) => {
            // SAFETY: the caller's buffer holds `output_len` bytes, at least `bytes.len()`.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
            session::STATUS_SUCCESS
        }
        Err(status) => status,
    }
}

/// `PowerDeterminePlatformRoleEx(Version)`: a desktop.
#[unsafe(no_mangle)]
pub extern "system" fn PowerDeterminePlatformRoleEx(_version: u32) -> u32 {
    session::PLATFORM_ROLE_DESKTOP
}

/// `PowerDeterminePlatformRole()`.
#[unsafe(no_mangle)]
pub extern "system" fn PowerDeterminePlatformRole() -> u32 {
    session::PLATFORM_ROLE_DESKTOP
}

/// `PowerGetActiveScheme(UserRootPowerKey, ActivePolicyGuid)`: "Returns ERROR_SUCCESS (zero)
/// if the call was successful, and a nonzero value if the call failed" (Learn). Poly has no
/// power schemes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PowerGetActiveScheme(_root: usize, scheme: *mut *mut u8) -> u32 {
    if !scheme.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { scheme.write(core::ptr::null_mut()) };
    }
    ERROR_FILE_NOT_FOUND
}

/// `PowerReadACValue(RootPowerKey, SchemeGuid, SubGroupOfPowerSettingsGuid, PowerSettingGuid,
/// Type, Buffer, BufferSize)`: no scheme, no value.
#[unsafe(no_mangle)]
pub extern "system" fn PowerReadACValue(
    _root: usize,
    _scheme: *const u8,
    _subgroup: *const u8,
    _setting: *const u8,
    _ty: *mut u32,
    _buffer: *mut u8,
    _size: *mut u32,
) -> u32 {
    ERROR_FILE_NOT_FOUND
}

/// `PowerReadDCValue`: as `PowerReadACValue`.
#[unsafe(no_mangle)]
pub extern "system" fn PowerReadDCValue(
    _root: usize,
    _scheme: *const u8,
    _subgroup: *const u8,
    _setting: *const u8,
    _ty: *mut u32,
    _buffer: *mut u8,
    _size: *mut u32,
) -> u32 {
    ERROR_FILE_NOT_FOUND
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn powrprof_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
