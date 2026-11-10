//! Job objects (Learn's `jobapi2.h` pages: `CreateJobObjectW`,
//! `OpenJobObjectW`, `AssignProcessToJobObject`, `IsProcessInJob`,
//! `QueryInformationJobObject`, `SetInformationJobObject`,
//! `TerminateJobObject`; ADR 0169) over the job services of the NT
//! personality.

use crate::base::*;
use crate::nt::{self, Handle, ObjectAttributes, UnicodeString};
use poly_kernelbase::error::*;
use poly_kernelbase::sync::object_name;

/// `JOB_OBJECT_ALL_ACCESS` (`winnt.h`).
const JOB_OBJECT_ALL_ACCESS: u32 = 0x001F_003F;
/// `STATUS_PROCESS_IN_JOB` (`ntstatus.h`); `STATUS_PROCESS_NOT_IN_JOB` is 0x123.
const STATUS_PROCESS_IN_JOB: u32 = 0x124;

/// `CreateJobObjectW`: a name that is a job's opens it with
/// `ERROR_ALREADY_EXISTS`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateJobObjectW(security: *const u8, name: *const u16) -> Handle {
    // SAFETY: the caller's string.
    let full = unsafe { wide_arg(name) }.and_then(object_name);
    let string = full.as_ref().map(|f| UnicodeString::new(f));
    let mut attrs = nt::OBJ_OPENIF;
    // SAFETY: the caller's structure when non-null.
    if !security.is_null() && unsafe { (security.add(16) as *const i32).read_unaligned() } != 0 {
        attrs |= nt::OBJ_INHERIT;
    }
    let oa = ObjectAttributes::new(string.as_ref(), attrs);
    let mut h: Handle = 0;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, &oa) };
    if !st.is_success() {
        return fail_status(st, 0);
    }
    set_last_error(if st == nt::STATUS_OBJECT_NAME_EXISTS { ERROR_ALREADY_EXISTS } else { ERROR_SUCCESS });
    h
}

/// `OpenJobObjectW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenJobObjectW(access: u32, inherit: i32, name: *const u16) -> Handle {
    // SAFETY: the caller's string.
    let Some(full) = (unsafe { wide_arg(name) }).and_then(object_name) else {
        return fail(ERROR_INVALID_PARAMETER, 0);
    };
    let us = UnicodeString::new(&full);
    let flags = (if inherit != 0 { nt::OBJ_INHERIT } else { 0 }) | nt::OBJ_CASE_INSENSITIVE;
    let oa = ObjectAttributes::new(Some(&us), flags);
    let mut h: Handle = 0;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenJobObject(&mut h, access, &oa) };
    if st.is_success() { h } else { fail_status(st, 0) }
}

/// `AssignProcessToJobObject`.
#[unsafe(no_mangle)]
pub extern "system" fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32 {
    // SAFETY: handle values.
    let st = unsafe { nt::NtAssignProcessToJobObject(job, process) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `IsProcessInJob`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsProcessInJob(process: Handle, job: Handle, result: *mut i32) -> i32 {
    // SAFETY: handle values.
    let st = unsafe { nt::NtIsProcessInJob(process, job) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    if !result.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { result.write(i32::from(st.0 == STATUS_PROCESS_IN_JOB)) };
    }
    TRUE
}

/// `QueryInformationJobObject`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryInformationJobObject(
    job: Handle,
    class: u32,
    info: *mut u8,
    length: u32,
    returned: *mut u32,
) -> i32 {
    let mut ret = 0u32;
    // SAFETY: the caller's buffer of `length` bytes.
    let st = unsafe { nt::NtQueryInformationJobObject(job, class, info, length, &mut ret) };
    if !st.is_success() {
        // A failed query leaves the caller's returned length as it was.
        return fail_status(st, FALSE);
    }
    if !returned.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { returned.write(ret) };
    }
    TRUE
}

/// `SetInformationJobObject`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetInformationJobObject(job: Handle, class: u32, info: *const u8, length: u32) -> i32 {
    // SAFETY: the caller's buffer of `length` bytes.
    let st = unsafe { nt::NtSetInformationJobObject(job, class, info, length) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `TerminateJobObject`.
#[unsafe(no_mangle)]
pub extern "system" fn TerminateJobObject(job: Handle, exit_code: u32) -> i32 {
    // SAFETY: a handle value.
    let st = unsafe { nt::NtTerminateJobObject(job, poly_ntobj::NtStatus(exit_code)) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}
