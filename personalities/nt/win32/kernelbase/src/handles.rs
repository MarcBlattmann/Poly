//! A handle's flags and copies (Learn's `handleapi.h` pages:
//! `GetHandleInformation`, `SetHandleInformation`, `DuplicateHandle`), ADR
//! 0089: what `CreateProcessW`'s `bInheritHandles` passes on is decided
//! per handle by `HANDLE_FLAG_INHERIT`.
//!
//! The flags are the object manager's (`poly_ntobj::HandleFlags`), read
//! and set through `NtQueryObject` / `NtSetInformationObject` with Poly's
//! `ObjectHandleFlagInformation` class (two `BOOLEAN`s, `Inherit` and
//! `ProtectFromClose`: Poly's own, no public header defines them);
//! `DuplicateHandle` is `NtDuplicateObject` with `OBJ_INHERIT` for
//! `bInheritHandle`.

use crate::base::*;
use crate::nt::{self, Handle};
use poly_kernelbase::process as p;
use poly_ntobj::NtStatus;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn NtDuplicateObject(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        access: u32,
        attributes: u32,
        options: u32,
    ) -> NtStatus;
    pub fn NtQueryObject(handle: Handle, class: u32, info: *mut u8, length: u32, returned: *mut u32) -> NtStatus;
    pub fn NtSetInformationObject(handle: Handle, class: u32, info: *const u8, length: u32) -> NtStatus;
}

/// Poly's `ObjectHandleFlagInformation` (ADR 0089,
/// `poly_personality_nt::consts::OBJECT_HANDLE_FLAG_INFORMATION`).
pub const OBJECT_HANDLE_FLAG_INFORMATION: u32 = 4;

/// `(inherit, protect_from_close)` of `h`.
fn flags_of(h: Handle) -> Result<(bool, bool), NtStatus> {
    let mut info = [0u8; 2];
    // SAFETY: a 2-byte buffer for the 2-byte class.
    let st = unsafe { NtQueryObject(h, OBJECT_HANDLE_FLAG_INFORMATION, info.as_mut_ptr(), 2, core::ptr::null_mut()) };
    if st.is_success() { Ok((info[0] != 0, info[1] != 0)) } else { Err(st) }
}

/// `GetHandleInformation`: `HANDLE_FLAG_INHERIT` and
/// `HANDLE_FLAG_PROTECT_FROM_CLOSE`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetHandleInformation(h: Handle, flags: *mut u32) -> i32 {
    match flags_of(h) {
        Ok((inherit, protect)) => {
            if flags.is_null() {
                return fail(poly_kernelbase::error::ERROR_INVALID_PARAMETER, FALSE);
            }
            // SAFETY: the caller's out-pointer.
            unsafe { flags.write_unaligned(p::handle_flag_bits(inherit, protect)) };
            TRUE
        }
        Err(st) => fail_status(st, FALSE),
    }
}

/// `SetHandleInformation`: `dwMask` says which flags change, `dwFlags`
/// their new values (`poly_kernelbase::process::handle_flags`).
#[unsafe(no_mangle)]
pub extern "system" fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32 {
    let current = match flags_of(h) {
        Ok(c) => c,
        Err(st) => return fail_status(st, FALSE),
    };
    let (inherit, protect) = match p::handle_flags(current, mask, flags) {
        Ok(f) => f,
        Err(e) => return fail(e, FALSE),
    };
    let info = [u8::from(inherit), u8::from(protect)];
    // SAFETY: a 2-byte buffer for the 2-byte class.
    let st = unsafe { NtSetInformationObject(h, OBJECT_HANDLE_FLAG_INFORMATION, info.as_ptr(), 2) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `DuplicateHandle`: `bInheritHandle` makes the copy inheritable;
/// `DUPLICATE_CLOSE_SOURCE` and `DUPLICATE_SAME_ACCESS` are
/// `NtDuplicateObject`'s options of the same values (`winnt.h`).
/// `lpTargetHandle` may be null (the copy is closed again: Learn allows
/// null with `DUPLICATE_CLOSE_SOURCE`, to close a handle in another
/// process).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DuplicateHandle(
    source_process: Handle,
    source: Handle,
    target_process: Handle,
    target: *mut Handle,
    access: u32,
    inherit: i32,
    options: u32,
) -> i32 {
    let attributes = if inherit != 0 { nt::OBJ_INHERIT } else { 0 };
    let mut h: Handle = 0;
    // SAFETY: valid pointers; the kernel checks the handles.
    let st = unsafe { NtDuplicateObject(source_process, source, target_process, &mut h, access, attributes, options) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    if target.is_null() {
        if h != 0 && target_process == nt::CURRENT_PROCESS {
            close(h);
        }
    } else {
        // SAFETY: the caller's out-pointer.
        unsafe { target.write_unaligned(h) };
    }
    TRUE
}
