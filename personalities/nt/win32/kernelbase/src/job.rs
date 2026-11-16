//! Job objects (Learn's `jobapi2.h` pages: `CreateJobObjectW`,
//! `OpenJobObjectW`, `AssignProcessToJobObject`, `IsProcessInJob`,
//! `QueryInformationJobObject`, `SetInformationJobObject`,
//! `TerminateJobObject`; ADR 0169) over the job services of the NT
//! personality.

use crate::base::*;
use crate::nt::{self, Handle, ObjectAttributes, UnicodeString};
use poly_kernelbase::error::*;
use poly_kernelbase::priority::{job_priority_to_nt, job_priority_to_win32};
use poly_kernelbase::sync::object_name;

/// `JOB_OBJECT_ALL_ACCESS` (`winnt.h`).
const JOB_OBJECT_ALL_ACCESS: u32 = 0x001F_003F;
/// `JOBOBJECTINFOCLASS` values (`winnt.h`) and `JOB_OBJECT_LIMIT_PRIORITY_CLASS`.
const JOB_OBJECT_BASIC_LIMIT_INFORMATION: u32 = 2;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
const JOB_OBJECT_GROUP_INFORMATION_EX: u32 = 14;
const JOB_OBJECT_LIMIT_PRIORITY_CLASS: u32 = 0x20;
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
    // SAFETY: the caller's SECURITY_ATTRIBUTES or null.
    if unsafe { crate::base::sa_inherits(security) } {
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
    let wide = unsafe { wide_arg(name) };
    // An empty name is a bad handle, not a bad parameter (the sheet: `empty_name`).
    if wide.as_ref().is_some_and(|w| w.is_empty()) {
        return fail(ERROR_INVALID_HANDLE, 0);
    }
    let Some(full) = wide.and_then(object_name) else {
        return fail(ERROR_INVALID_PARAMETER, 0);
    };
    let us = UnicodeString::new(&full);
    // The name's case matters (the sheet: `spelling`).
    let flags = if inherit != 0 { nt::OBJ_INHERIT } else { 0 };
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
    // The group information of the jobs asks for no more than the kernel's two entries
    // (the sheet: a 1024-byte buffer succeeds with 16 bytes).
    let ask = if class == JOB_OBJECT_GROUP_INFORMATION_EX { length.min(32) } else { length };
    // SAFETY: the caller's buffer of `length` bytes.
    let st = unsafe { nt::NtQueryInformationJobObject(job, class, info, ask, &mut ret) };
    if !st.is_success() {
        // A failed query leaves the caller's returned length as it was.
        return fail_status(st, FALSE);
    }
    // A job without a priority limit reads NORMAL_PRIORITY_CLASS here (the kernel holds 0).
    if (class == JOB_OBJECT_BASIC_LIMIT_INFORMATION || class == JOB_OBJECT_EXTENDED_LIMIT_INFORMATION)
        && ret >= 64
        && !info.is_null()
    {
        // SAFETY: the kernel wrote `ret` >= 64 bytes of the limit structure at `info`.
        unsafe {
            let flags = info.add(16).cast::<u32>().read_unaligned();
            let prio = info.add(56).cast::<u32>().read_unaligned();
            info.add(56).cast::<u32>().write_unaligned(job_priority_to_win32(flags, prio));
        }
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
    // A priority limit is given as a winbase.h class (NORMAL_PRIORITY_CLASS, ...): the kernel
    // holds 1 to 6.
    let mut local = [0u8; 152];
    let mut pointer = info;
    if (class == JOB_OBJECT_BASIC_LIMIT_INFORMATION || class == JOB_OBJECT_EXTENDED_LIMIT_INFORMATION)
        && (64..=152).contains(&length)
        && !info.is_null()
    {
        // SAFETY: the caller's buffer of `length` bytes.
        unsafe { core::ptr::copy_nonoverlapping(info, local.as_mut_ptr(), length as usize) };
        let flags = u32::from_le_bytes([local[16], local[17], local[18], local[19]]);
        if flags & JOB_OBJECT_LIMIT_PRIORITY_CLASS != 0 {
            let p = u32::from_le_bytes([local[56], local[57], local[58], local[59]]);
            local[56..60].copy_from_slice(&job_priority_to_nt(p).to_le_bytes());
        }
        pointer = local.as_ptr();
    }
    // SAFETY: the caller's buffer of `length` bytes, or the copy of it.
    let st = unsafe { nt::NtSetInformationJobObject(job, class, pointer, length) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `TerminateJobObject`.
#[unsafe(no_mangle)]
pub extern "system" fn TerminateJobObject(job: Handle, exit_code: u32) -> i32 {
    // SAFETY: a handle value.
    let st = unsafe { nt::NtTerminateJobObject(job, poly_ntobj::NtStatus(exit_code)) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}
