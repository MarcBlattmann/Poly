//! `GetProcessMitigationPolicy` / `SetProcessMitigationPolicy` (ADR 0171,
//! `processthreadsapi.h`, `winnt.h` `PROCESS_MITIGATION_POLICY`): each
//! policy is a 4-byte flags word (DEP: 8 bytes), the calling process's
//! state kept here, with the rules the spec sheets observed: most policies
//! can only be switched on, a permanent strict-handle policy cannot be
//! weakened, `ChildProcess` stops `CreateProcessW`
//! (`ERROR_CHILD_PROCESS_BLOCKED`), and the dynamic-code and win32k policies
//! also go to the kernel (`NtSetInformationProcess`, class 52), which
//! enforces them.

use crate::base::*;
use crate::nt::Handle;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::error::ERROR_INVALID_PARAMETER;
use poly_ntobj::NtStatus;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSetInformationProcess(process: Handle, class: u32, info: *const u8, length: u32) -> NtStatus;
}

const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_ACCESS_DENIED: u32 = 5;

const DEP: u32 = 0;
const ASLR: u32 = 1;
const DYNAMIC_CODE: u32 = 2;
const STRICT_HANDLE: u32 = 3;
const SYSTEM_CALL_DISABLE: u32 = 4;
const OPTIONS_MASK: u32 = 5;
const CONTROL_FLOW_GUARD: u32 = 7;
const SIGNATURE: u32 = 8;
const PAYLOAD_RESTRICTION: u32 = 12;
const CHILD_PROCESS: u32 = 13;
const USER_SHADOW_STACK: u32 = 15;
const USER_POINTER_AUTH: u32 = 17;
const MAX_POLICY: u32 = 18;

/// The state of policies 0 to 18: a flags word each (DEP's is the flags of
/// its 8-byte structure). ASLR starts with bottom-up and high-entropy on, DEP
/// enabled and permanent (x64).
static POLICY: [AtomicU32; 19] = {
    const ZERO: AtomicU32 = AtomicU32::new(0);
    let mut a = [ZERO; 19];
    a[DEP as usize] = AtomicU32::new(3);
    a[ASLR as usize] = AtomicU32::new(5);
    a
};

/// Whether `NoChildProcessCreation` is on.
pub fn child_processes_blocked() -> bool {
    POLICY[CHILD_PROCESS as usize].load(Ordering::Relaxed) & 1 != 0
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
    if process != -1 && crate::spawn::GetProcessId(process) == 0 {
        return FALSE;
    }
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
            let flags = POLICY[policy as usize].load(Ordering::Relaxed).to_le_bytes();
            core::ptr::copy_nonoverlapping(flags.as_ptr(), buffer, 4);
        }
    }
    TRUE
}

/// `SetProcessMitigationPolicy`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetProcessMitigationPolicy(policy: u32, buffer: *const u8, length: usize) -> i32 {
    let Some(want) = size_of(policy) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if length != want {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's structure (a NULL one faults, as on Windows).
    let flags = unsafe { (buffer as *const u32).read_unaligned() };
    let slot = &POLICY[policy as usize];
    match policy {
        DEP => return fail(ERROR_NOT_SUPPORTED, FALSE),
        CONTROL_FLOW_GUARD | USER_SHADOW_STACK => return fail(ERROR_ACCESS_DENIED, FALSE),
        PAYLOAD_RESTRICTION | OPTIONS_MASK => return fail(ERROR_INVALID_PARAMETER, FALSE),
        STRICT_HANDLE => {
            // Raise-on-invalid and permanent go together; once permanent it stays.
            if slot.load(Ordering::Relaxed) & 2 != 0 && flags & 2 == 0 {
                return fail(ERROR_ACCESS_DENIED, FALSE);
            }
            if flags & 3 != 0 && flags & 3 != 3 {
                return fail(ERROR_INVALID_PARAMETER, FALSE);
            }
            slot.fetch_or(flags, Ordering::Relaxed);
        }
        ASLR => {
            if flags & 1 != 0 {
                slot.fetch_or(5, Ordering::Relaxed);
            }
        }
        SIGNATURE => {
            // Microsoft-signed-only turns on the opt-in bit as well.
            slot.fetch_or(if flags & 1 != 0 { flags | 4 } else { flags }, Ordering::Relaxed);
        }
        _ => {
            slot.fetch_or(flags, Ordering::Relaxed);
        }
    }
    if matches!(policy, DYNAMIC_CODE | SYSTEM_CALL_DISABLE) && flags & 1 != 0 {
        let mut info = [0u8; 8];
        info[0..4].copy_from_slice(&policy.to_le_bytes());
        info[4..8].copy_from_slice(&flags.to_le_bytes());
        // SAFETY: a local 8-byte structure; -1 is the current process.
        unsafe { NtSetInformationProcess(-1, 52, info.as_ptr(), 8) };
    }
    TRUE
}
