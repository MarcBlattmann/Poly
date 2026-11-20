//! A thread's ideal processor and group affinity (Learn's `processthreadsapi.h` and `winbase.h`
//! pages; specs/windows/kernel32.dll/GetThreadIdealProcessorEx.yaml and
//! GetThreadGroupAffinity.yaml): `ThreadIdealProcessorEx` (class 33, a `PROCESSOR_NUMBER`) and
//! the affinity mask of `THREAD_BASIC_INFORMATION` / `ThreadAffinityMask` (class 4). Poly's
//! machine has one processor group, group 0.

use crate::base::*;
use crate::nt::{self, Handle};
use poly_kernelbase::error::*;
use poly_ntobj::NtStatus;

/// `ThreadAffinityMask`, `ThreadIdealProcessorEx` (`ntddk.h`'s `THREADINFOCLASS`).
const THREAD_AFFINITY_MASK: u32 = 4;
const THREAD_IDEAL_PROCESSOR_EX: u32 = 33;
const P: usize = core::mem::size_of::<usize>();
/// `GROUP_AFFINITY` (`winnt.h`): a `KAFFINITY` mask, a `WORD` group, three reserved `WORD`s.
const GROUP_AFFINITY_SIZE: usize = P + 8;

/// The mask of the processors a process may use: every one the system has.
pub(crate) fn process_mask() -> u64 {
    let n = crate::misc::processors().clamp(1, 64);
    if n == 64 { u64::MAX } else { (1u64 << n) - 1 }
}

/// A `PROCESSOR_NUMBER` that names a processor the system has (group 0, reserved 0).
pub(crate) fn valid_processor(p: [u8; 4]) -> bool {
    u16::from_le_bytes([p[0], p[1]]) == 0 && p[3] == 0 && u32::from(p[2]) < crate::misc::processors().min(64)
}

/// A `GROUP_AFFINITY`'s mask if it is one the process may use (group 0, reserved zero, only the
/// process's processors); mask 0 means the process mask.
pub(crate) fn valid_affinity(g: &[u8]) -> Option<u64> {
    let mut m = [0u8; 8];
    m[..P].copy_from_slice(&g[..P]);
    let mask = u64::from_le_bytes(m);
    let group = u16::from_le_bytes([g[P], g[P + 1]]);
    if group != 0 || g[P + 2..GROUP_AFFINITY_SIZE].iter().any(|&b| b != 0) || mask & !process_mask() != 0 {
        return None;
    }
    Some(if mask == 0 { process_mask() } else { mask })
}

/// Set a thread's affinity mask (`ThreadAffinityMask`).
pub(crate) fn set_mask(thread: Handle, mask: u64) -> NtStatus {
    let m = mask as usize;
    // SAFETY: a pointer-sized value, as the class takes.
    unsafe { nt::NtSetInformationThread(thread, THREAD_AFFINITY_MASK, (&raw const m).cast(), P as u32) }
}

/// Set a thread's ideal processor; `previous` gets the one before.
pub(crate) fn set_ideal(thread: Handle, p: [u8; 4], previous: Option<&mut [u8; 4]>) -> NtStatus {
    let mut b = p;
    // SAFETY: four bytes, in and out (the previous value comes back in them).
    let st = unsafe { nt::NtSetInformationThread(thread, THREAD_IDEAL_PROCESSOR_EX, b.as_mut_ptr(), 4) };
    if st.is_success()
        && let Some(prev) = previous
    {
        *prev = b;
    }
    st
}

/// `GetThreadIdealProcessorEx`: needs a query right on the handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadIdealProcessorEx(thread: Handle, out: *mut u8) -> i32 {
    let mut b = [0u8; 4];
    // SAFETY: a buffer of the class's size.
    let st = unsafe {
        nt::NtQueryInformationThread(thread, THREAD_IDEAL_PROCESSOR_EX, b.as_mut_ptr(), 4, core::ptr::null_mut())
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's PROCESSOR_NUMBER.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, 4) };
    TRUE
}

/// `SetThreadIdealProcessorEx`: needs `THREAD_SET_INFORMATION`; the previous ideal processor goes
/// to `previous` when it is not NULL; a processor the system lacks is `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadIdealProcessorEx(thread: Handle, ideal: *const u8, previous: *mut u8) -> i32 {
    if ideal.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let mut p = [0u8; 4];
    // SAFETY: the caller's PROCESSOR_NUMBER.
    unsafe { core::ptr::copy_nonoverlapping(ideal, p.as_mut_ptr(), 4) };
    let mut prev = [0u8; 4];
    let st = set_ideal(thread, p, Some(&mut prev));
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    if !previous.is_null() {
        // SAFETY: the caller's PROCESSOR_NUMBER.
        unsafe { core::ptr::copy_nonoverlapping(prev.as_ptr(), previous, 4) };
    }
    TRUE
}

/// The affinity mask of `thread` (`THREAD_BASIC_INFORMATION.AffinityMask`; a query right).
fn mask_of(thread: Handle) -> Result<u64, NtStatus> {
    let mut info = [0u8; 48];
    let size = 6 * P as u32;
    // SAFETY: a buffer of the class's size.
    let st = unsafe { nt::NtQueryInformationThread(thread, 0, info.as_mut_ptr(), size, core::ptr::null_mut()) };
    if !st.is_success() {
        return Err(st);
    }
    let mut m = [0u8; 8];
    m[..P].copy_from_slice(&info[4 * P..5 * P]);
    Ok(u64::from_le_bytes(m))
}

/// Write a `GROUP_AFFINITY` of group 0 with `mask`.
///
/// # Safety
/// `out` is the caller's `GROUP_AFFINITY`.
unsafe fn put_affinity(out: *mut u8, mask: u64) {
    let mut g = [0u8; GROUP_AFFINITY_SIZE];
    g[..P].copy_from_slice(&mask.to_le_bytes()[..P]);
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(g.as_ptr(), out, GROUP_AFFINITY_SIZE) };
}

/// `GetThreadGroupAffinity`: group 0 and the thread's mask; needs a query right.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadGroupAffinity(thread: Handle, out: *mut u8) -> i32 {
    match mask_of(thread) {
        Ok(mask) => {
            // SAFETY: the caller's GROUP_AFFINITY.
            unsafe { put_affinity(out, mask) };
            TRUE
        }
        Err(st) => fail_status(st, FALSE),
    }
}

/// `SetThreadGroupAffinity`: needs a query right (the previous mask is read) and
/// `THREAD_SET_INFORMATION`; mask 0 means the process mask; a mask outside it, group 1 or a
/// non-zero reserved field is `ERROR_INVALID_PARAMETER` and nothing changes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadGroupAffinity(thread: Handle, affinity: *const u8, previous: *mut u8) -> i32 {
    let before = match mask_of(thread) {
        Ok(m) => m,
        Err(st) => return fail_status(st, FALSE),
    };
    if affinity.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let mut g = [0u8; GROUP_AFFINITY_SIZE];
    // SAFETY: the caller's GROUP_AFFINITY.
    unsafe { core::ptr::copy_nonoverlapping(affinity, g.as_mut_ptr(), GROUP_AFFINITY_SIZE) };
    let Some(mask) = valid_affinity(&g) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let st = set_mask(thread, mask);
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    if !previous.is_null() {
        // SAFETY: the caller's GROUP_AFFINITY.
        unsafe { put_affinity(previous, before) };
    }
    TRUE
}
