//! `GetProcessMitigationPolicy` / `SetProcessMitigationPolicy` (ADR 0171, ADR 0192,
//! `processthreadsapi.h`, `winnt.h` `PROCESS_MITIGATION_POLICY`): each policy is a 4-byte flags
//! word (DEP: 8 bytes). The state is the kernel's, per process
//! ([`poly_kernelbase::mitigation::State`]): the creation attributes of `CreateProcessW` put the
//! child's policies there before it runs, `SetProcessMitigationPolicy` adds to them, and these
//! functions read and write it through `NtQueryInformationProcess` and `NtSetInformationProcess`
//! (class 52). `ChildProcess` stops `CreateProcessW` (`ERROR_CHILD_PROCESS_BLOCKED`).

use crate::base::*;
use crate::nt::Handle;
use poly_kernelbase::error::ERROR_INVALID_PARAMETER;
use poly_kernelbase::mitigation::{CHILD_PROCESS, DEP, MAX_POLICY, OPTIONS_MASK, USER_POINTER_AUTH};
use poly_ntobj::NtStatus;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSetInformationProcess(process: Handle, class: u32, info: *const u8, length: u32) -> NtStatus;
    fn NtQueryInformationProcess(process: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
}

const ERROR_NOT_SUPPORTED: u32 = 50;

/// The flags word of `policy` of `process` (the kernel's `ProcessMitigationPolicy` query).
fn query(process: Handle, policy: u32) -> Result<u32, NtStatus> {
    let mut info = [0u8; 8];
    info[0..4].copy_from_slice(&policy.to_le_bytes());
    // SAFETY: a local 8-byte structure.
    let st = unsafe { NtQueryInformationProcess(process, 52, info.as_mut_ptr(), 8, core::ptr::null_mut()) };
    if st.is_success() { Ok(u32::from_le_bytes(info[4..8].try_into().unwrap())) } else { Err(st) }
}

/// Whether `NoChildProcessCreation` is on for this process.
pub fn child_processes_blocked() -> bool {
    query(-1, CHILD_PROCESS).is_ok_and(|f| f & 1 != 0)
}

/// The length a policy's structure has (`None` for an unknown policy).
fn size_of(policy: u32) -> Option<usize> {
    match policy {
        DEP => Some(8),
        0..=MAX_POLICY => Some(4),
        _ => None,
    }
}

/// `GetProcessMitigationPolicy`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessMitigationPolicy(
    process: Handle,
    policy: u32,
    buffer: *mut u8,
    length: usize,
) -> i32 {
    if policy == USER_POINTER_AUTH {
        return fail(if length == 4 { ERROR_NOT_SUPPORTED } else { ERROR_INVALID_PARAMETER }, FALSE);
    }
    let Some(want) = size_of(policy) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    // The options mask takes every length from 8 up; the others exactly theirs.
    let ok_length = if policy == OPTIONS_MASK { length >= 8 } else { length == want };
    if !ok_length || buffer.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // The handle: the pseudo-handle or one that may query the process.
    let flags = match query(process, policy) {
        Ok(f) => f,
        Err(st) => return fail_status(st, FALSE),
    };
    // SAFETY: `length` bytes at `buffer`, the caller's.
    unsafe {
        if policy == OPTIONS_MASK {
            // A row of 64-bit masks of the policies the system knows, then
            // zeros: exactly `length` bytes, cut inside a mask if need be
            // (the sheet's bytes).
            let masks: [u8; 16] = poly_kernelbase::process::mitigation_options_mask();
            core::ptr::write_bytes(buffer, 0, length);
            core::ptr::copy_nonoverlapping(masks.as_ptr(), buffer, length.min(masks.len()));
        } else {
            core::ptr::write_bytes(buffer, 0, want);
            let flags = flags.to_le_bytes();
            core::ptr::copy_nonoverlapping(flags.as_ptr(), buffer, 4);
        }
    }
    TRUE
}

/// `SetProcessMitigationPolicy`: the kernel's state takes the change (rules in
/// [`poly_kernelbase::mitigation::State::set`]).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetProcessMitigationPolicy(policy: u32, buffer: *const u8, length: usize) -> i32 {
    let Some(want) = size_of(policy) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if length != want {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's structure (a NULL one faults, as on Windows).
    let flags = unsafe { (buffer as *const u32).read_unaligned() };
    let mut info = [0u8; 8];
    info[0..4].copy_from_slice(&policy.to_le_bytes());
    info[4..8].copy_from_slice(&flags.to_le_bytes());
    // SAFETY: a local 8-byte structure; -1 is the current process.
    let st = unsafe { NtSetInformationProcess(-1, 52, info.as_ptr(), 8) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}
