//! The security functions 7-Zip's command line imports beyond ADR 0083's:
//! privilege adjustment (ADR 0114: the kernel's `NtAdjustPrivilegesToken`,
//! the logic `poly_security::token::adjust_privileges`) and file security
//! descriptors (ADR 0087: stubs in the coverage map, each reporting what
//! Learn documents for that outcome).

use crate::base::*;
use crate::nt::Handle;
use poly_kernelbase::error::*;
use poly_ntobj::NtStatus;
use poly_security::token::STATUS_NOT_ALL_ASSIGNED;

/// `ERROR_NOT_ALL_ASSIGNED` (`winerror.h`).
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtAdjustPrivilegesToken(
        token: Handle,
        disable_all: u8,
        new_state: *const u8,
        buffer_length: u32,
        previous: *mut u8,
        return_length: *mut u32,
    ) -> NtStatus;
}

/// `AdjustTokenPrivileges` (Learn): enables, disables or removes the
/// privileges `NewState` names that the token holds, or with
/// `DisableAllPrivileges` disables them all. A success sets the last error
/// to `ERROR_SUCCESS`, or to `ERROR_NOT_ALL_ASSIGNED` when the token lacks
/// one of them ("The function may succeed with this error value even if no
/// privileges were adjusted"); `PreviousState` gets the privileges that
/// changed with their former attributes, and a buffer too small for them
/// is `ERROR_INSUFFICIENT_BUFFER` with nothing adjusted.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustTokenPrivileges(
    token: Handle,
    disable_all: i32,
    new_state: *const u8,
    buffer_length: u32,
    previous: *mut u8,
    return_length: *mut u32,
) -> i32 {
    // Learn: ReturnLength "can be NULL if PreviousState is NULL".
    if !previous.is_null() && return_length.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's structures, checked by the kernel.
    let st = unsafe {
        NtAdjustPrivilegesToken(token, u8::from(disable_all != 0), new_state, buffer_length, previous, return_length)
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    set_last_error(if st == STATUS_NOT_ALL_ASSIGNED { ERROR_NOT_ALL_ASSIGNED } else { ERROR_SUCCESS });
    TRUE
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQuerySecurityObject(handle: Handle, info: u32, sd: *mut u8, length: u32, needed: *mut u32) -> NtStatus;
    fn NtSetSecurityObject(handle: Handle, info: u32, sd: *const u8) -> NtStatus;
}

/// `ERROR_INVALID_NAME`, `ERROR_NOACCESS`, `ERROR_UNKNOWN_REVISION` (`winerror.h`).
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_NOACCESS: u32 = 998;
const ERROR_UNKNOWN_REVISION: u32 = 1305;
/// The `SECURITY_INFORMATION` parts a file's descriptor has: owner, group, DACL, SACL, label
/// (`winnt.h`). Any other bit alone asks for nothing (specs/windows/advapi32.dll/
/// SetFileSecurityW.yaml and GetFileSecurityW.yaml, `bad_information`: `ERROR_ACCESS_DENIED`).
const FILE_SECURITY_PARTS: u32 = 0x1F;
/// `SACL_SECURITY_INFORMATION`, `ACCESS_SYSTEM_SECURITY`, `READ_CONTROL`, `WRITE_DAC`,
/// `WRITE_OWNER`, `FILE_OPEN_FOR_BACKUP_INTENT` (`winnt.h`, `winternl.h`).
const SACL_SECURITY_INFORMATION: u32 = 8;
const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x4000;

/// A file name argument: NULL or empty is `ERROR_INVALID_NAME` (the sheets).
fn file_name(name: Option<&[u16]>) -> Result<&[u16], u32> {
    match name {
        Some(n) if !n.is_empty() => Ok(n),
        _ => Err(ERROR_INVALID_NAME),
    }
}

/// `GetFileSecurityW(lpFileName, RequestedInformation, pSecurityDescriptor, nLength,
/// lpnLengthNeeded)`: the file's or directory's descriptor, as `NtQuerySecurityObject` gives it on
/// a handle opened for it (Learn: `READ_CONTROL`, and `ACCESS_SYSTEM_SECURITY` for the SACL; the
/// open asks for backup intent so that a directory opens too). A file the caller cannot open is
/// that open's error (the sheet of `SystemTemp`: `ERROR_ACCESS_DENIED` for the filtered token); a
/// buffer that is too small is `ERROR_INSUFFICIENT_BUFFER` with the size in `*lpnLengthNeeded`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileSecurityW(
    name: *const u16,
    requested: u32,
    descriptor: *mut u8,
    length: u32,
    needed: *mut u32,
) -> i32 {
    // SAFETY: the caller's string.
    let name = match file_name(unsafe { wide_arg(name) }) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    // The sheet, `null_pointers`: no place for the length, or no buffer with a length, is
    // ERROR_NOACCESS; nothing asked for is ERROR_ACCESS_DENIED.
    if needed.is_null() || (descriptor.is_null() && length != 0) {
        return fail(ERROR_NOACCESS, FALSE);
    }
    if requested & FILE_SECURITY_PARTS == 0 {
        return fail(ERROR_ACCESS_DENIED, FALSE);
    }
    let access = READ_CONTROL | if requested & SACL_SECURITY_INFORMATION != 0 { ACCESS_SYSTEM_SECURITY } else { 0 };
    let h = match crate::file::open_for_security(name, access, FILE_OPEN_FOR_BACKUP_INTENT) {
        Ok(h) => h,
        Err(e) => return fail(e, FALSE),
    };
    let mut need = 0u32;
    // SAFETY: the caller's buffer of `length` bytes (or NULL with 0); a local count.
    let st = unsafe { NtQuerySecurityObject(h, requested, descriptor, length, &mut need) };
    close(h);
    // SAFETY: the caller's out-pointer.
    unsafe { needed.write_unaligned(need) };
    if st.is_success() {
        TRUE
    } else if st.0 == 0xC000_0023 {
        // STATUS_BUFFER_TOO_SMALL: ERROR_INSUFFICIENT_BUFFER (122).
        fail(122, FALSE)
    } else {
        fail_status(st, FALSE)
    }
}

/// `SetFileSecurityW(lpFileName, SecurityInformation, pSecurityDescriptor)`
/// (specs/windows/advapi32.dll/SetFileSecurityW.yaml): the parts the information names are taken
/// from the descriptor and set on the file or directory, opened for what they need (`WRITE_OWNER`
/// for the owner, group and label, `WRITE_DAC` for the DACL, `ACCESS_SYSTEM_SECURITY` for the
/// SACL; backup intent, so that a directory opens too; every sharing mode, so that a file open
/// elsewhere without sharing still takes it). `NtSetSecurityObject` decides the rest: the owner
/// a token may set, the label it may give, a descriptor without the part asked for.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFileSecurityW(name: *const u16, information: u32, descriptor: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let name = match file_name(unsafe { wide_arg(name) }) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    if descriptor.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    // SAFETY: the caller's descriptor: at least its revision byte.
    if unsafe { descriptor.read() } != 1 {
        return fail(ERROR_UNKNOWN_REVISION, FALSE);
    }
    if information & FILE_SECURITY_PARTS == 0 {
        return fail(ERROR_ACCESS_DENIED, FALSE);
    }
    let mut access = 0;
    if information & (1 | 2 | 0x10) != 0 {
        access |= WRITE_OWNER;
    }
    if information & 4 != 0 {
        access |= WRITE_DAC;
    }
    if information & SACL_SECURITY_INFORMATION != 0 {
        access |= ACCESS_SYSTEM_SECURITY;
    }
    let h = match crate::file::open_for_security(name, access, FILE_OPEN_FOR_BACKUP_INTENT) {
        Ok(h) => h,
        Err(e) => return fail(e, FALSE),
    };
    // SAFETY: the caller's descriptor; the kernel checks it.
    let st = unsafe { NtSetSecurityObject(h, information, descriptor) };
    close(h);
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `SetFileSecurityA`: `SetFileSecurityW` with an ANSI path.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFileSecurityA(name: *const u8, information: u32, descriptor: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let Some(wide) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_NAME, FALSE) };
    let mut z = wide;
    z.push(0);
    // SAFETY: a NUL-terminated copy; the descriptor passed on.
    unsafe { SetFileSecurityW(z.as_ptr(), information, descriptor) }
}

/// `GetFileSecurityA`: `GetFileSecurityW` with an ANSI path.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileSecurityA(
    name: *const u8,
    requested: u32,
    descriptor: *mut u8,
    length: u32,
    needed: *mut u32,
) -> i32 {
    // SAFETY: the caller's string.
    let Some(wide) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_NAME, FALSE) };
    let mut z = wide;
    z.push(0);
    // SAFETY: a NUL-terminated copy; the caller's buffers passed on.
    unsafe { GetFileSecurityW(z.as_ptr(), requested, descriptor, length, needed) }
}
