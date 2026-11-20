//! `CreateRemoteThread` and `CreateRemoteThreadEx` (Learn's `processthreadsapi.h` pages;
//! specs/windows/kernel32.dll/CreateRemoteThread.yaml and CreateRemoteThreadEx.yaml): a thread in
//! another process, through `NtCreateThreadEx` with that process's handle.
//!
//! The process handle's rights are kernel32's own rule, not `NtCreateThreadEx`'s (which needs
//! exactly `PROCESS_CREATE_THREAD`): a handle with `PROCESS_CREATE_THREAD` and either
//! `PROCESS_QUERY_INFORMATION` or both `PROCESS_VM_OPERATION` and `PROCESS_VM_WRITE` is used as it
//! is; any other handle is first duplicated with `PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION`,
//! which the process's security descriptor allows or refuses (`rights_normal`, `rights_widening`,
//! `rights_on_the_handle`). A refused duplication is `ERROR_ACCESS_DENIED` with
//! `STATUS_ACCESS_DENIED`.

use crate::attrlist;
use crate::base::*;
use crate::handles::{NtDuplicateObject, NtQueryObject};
use crate::nt::{self, Handle, ObjectAttributes};
use poly_ntobj::NtStatus;

/// `CREATE_SUSPENDED`, `STACK_SIZE_PARAM_IS_A_RESERVATION` (`winbase.h`): the only creation flags
/// that do anything here; every other bit is ignored (`other_flag_bits`).
const CREATE_SUSPENDED: u32 = 0x4;
const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x1_0000;
/// `THREAD_ALL_ACCESS` (`winnt.h`).
const THREAD_ALL_ACCESS: u32 = 0x1F_FFFF;
/// Process rights (`winnt.h`).
const PROCESS_CREATE_THREAD: u32 = 0x0002;
const PROCESS_VM_OPERATION: u32 = 0x0008;
const PROCESS_VM_WRITE: u32 = 0x0020;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
/// `ntstatus.h`.
const STATUS_ACCESS_DENIED: NtStatus = NtStatus(0xC000_0022);
const STATUS_OBJECT_TYPE_MISMATCH: NtStatus = NtStatus(0xC000_0024);
const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);
/// `ObjectBasicInformation`, `ObjectTypeInformation` (`winternl.h`'s `OBJECT_INFORMATION_CLASS`).
const OBJECT_BASIC_INFORMATION: u32 = 0;
const OBJECT_TYPE_INFORMATION: u32 = 2;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RtlSetLastWin32ErrorAndNtStatusFromNtStatus(status: NtStatus);
}

/// Fail with `status`: the last error and the thread's last NT status (`RtlGetLastNtStatus`,
/// which the sheets read after a failed call).
fn fail_nt(status: NtStatus) -> Handle {
    // SAFETY: a plain ntdll call.
    unsafe { RtlSetLastWin32ErrorAndNtStatusFromNtStatus(status) };
    0
}

/// Whether kernel32 uses a handle with `granted` rights as it is.
fn enough(granted: u32) -> bool {
    let vm = PROCESS_VM_OPERATION | PROCESS_VM_WRITE;
    granted & PROCESS_CREATE_THREAD != 0 && (granted & PROCESS_QUERY_INFORMATION != 0 || granted & vm == vm)
}

/// The process handle to give `NtCreateThreadEx` for `process`, and whether it is a copy to close
/// afterwards; or the status to fail with.
fn process_handle(process: Handle) -> Result<(Handle, bool), NtStatus> {
    if process == nt::CURRENT_PROCESS {
        return Ok((process, false));
    }
    if process == nt::CURRENT_THREAD {
        return Err(STATUS_OBJECT_TYPE_MISMATCH);
    }
    let mut basic = [0u8; 56];
    // SAFETY: a buffer of the size given.
    let st = unsafe { NtQueryObject(process, OBJECT_BASIC_INFORMATION, basic.as_mut_ptr(), 56, core::ptr::null_mut()) };
    if !st.is_success() {
        return Err(st);
    }
    if !is_process(process) {
        return Err(STATUS_OBJECT_TYPE_MISMATCH);
    }
    let granted = u32::from_le_bytes([basic[4], basic[5], basic[6], basic[7]]);
    if enough(granted) {
        return Ok((process, false));
    }
    let mut copy: Handle = 0;
    let wanted = PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION;
    // SAFETY: valid pointers; the kernel checks the handles and the descriptor.
    let st = unsafe { NtDuplicateObject(nt::CURRENT_PROCESS, process, nt::CURRENT_PROCESS, &mut copy, wanted, 0, 0) };
    if !st.is_success() {
        return Err(STATUS_ACCESS_DENIED);
    }
    Ok((copy, true))
}

/// Whether `h` is a handle to a process object (`ObjectTypeInformation`'s name).
fn is_process(h: Handle) -> bool {
    // `PUBLIC_OBJECT_TYPE_INFORMATION`: a `UNICODE_STRING` with its characters after it.
    #[repr(C, align(8))]
    struct Buf([u8; 512]);
    let mut b = Buf([0; 512]);
    // SAFETY: a buffer of the size given.
    let st = unsafe { NtQueryObject(h, OBJECT_TYPE_INFORMATION, b.0.as_mut_ptr(), 512, core::ptr::null_mut()) };
    if !st.is_success() {
        // A type the kernel cannot name: `NtCreateThreadEx` decides.
        return true;
    }
    // SAFETY: the kernel wrote a `UNICODE_STRING` at the start of the aligned buffer, whose
    // characters lie in it.
    let name = unsafe {
        let s = (b.0.as_ptr() as *const nt::UnicodeString).read();
        if s.buffer.is_null() {
            return true;
        }
        core::slice::from_raw_parts(s.buffer, usize::from(s.length) / 2)
    };
    name.iter().copied().eq("Process".encode_utf16())
}

/// What both calls do; `attributes` is the parsed attribute list of `CreateRemoteThreadEx`.
///
/// # Safety
/// The caller's pointers are valid or null, as the functions' contracts say.
#[allow(clippy::too_many_arguments)]
unsafe fn create_remote(
    process: Handle,
    security: *const u8,
    stack: usize,
    start: *const u8,
    parameter: *mut u8,
    flags: u32,
    attributes: &attrlist::ThreadParsed,
    thread_id: *mut u32,
) -> Handle {
    // The attributes name processors the system has (specs/windows/kernel32.dll/
    // GetThreadIdealProcessorEx.yaml and GetThreadGroupAffinity.yaml,
    // `attribute_of_createremotethreadex`), else no thread is made.
    let mask = match attributes.affinity {
        Some(g) => match crate::affinity::valid_affinity(&g) {
            Some(m) => Some(m),
            None => return fail_nt(STATUS_INVALID_PARAMETER),
        },
        None => None,
    };
    if attributes.ideal.is_some_and(|p| !crate::affinity::valid_processor(p)) {
        return fail_nt(STATUS_INVALID_PARAMETER);
    }
    let (target, copied) = match process_handle(process) {
        Ok(t) => t,
        Err(st) => return fail_nt(st),
    };
    // `lpThreadAttributes`: the thread object's descriptor and an inheritable handle.
    // SAFETY: the caller's structure when non-null.
    let inherit = unsafe { sa_inherits(security) };
    let mut oa = ObjectAttributes::new(None, if inherit { nt::OBJ_INHERIT } else { 0 });
    // SAFETY: as above.
    oa.security_descriptor = unsafe { sa_descriptor(security) };
    let (commit, reserve) = if flags & STACK_SIZE_PARAM_IS_A_RESERVATION != 0 { (0, stack) } else { (stack, 0) };
    let mut h: Handle = 0;
    // SAFETY: valid pointers; the kernel checks the rest.
    let st = unsafe {
        nt::NtCreateThreadEx(
            &mut h,
            THREAD_ALL_ACCESS,
            &oa,
            target,
            start,
            parameter,
            u32::from(flags & CREATE_SUSPENDED != 0),
            0,
            commit,
            reserve,
            core::ptr::null(),
        )
    };
    if copied {
        // SAFETY: our own copy.
        unsafe { nt::NtClose(target) };
    }
    if !st.is_success() {
        return fail_nt(st);
    }
    if let Some(mask) = mask {
        crate::affinity::set_mask(h, mask);
    }
    if let Some(p) = attributes.ideal {
        crate::affinity::set_ideal(h, p, None);
    }
    if !thread_id.is_null() {
        let mut info = [0u8; 48];
        // SAFETY: valid pointers.
        let q = unsafe { nt::NtQueryInformationThread(h, 0, info.as_mut_ptr(), 48, core::ptr::null_mut()) };
        let id = if q.is_success() { id_in(&info, TBI_UNIQUE_THREAD) } else { 0 };
        // SAFETY: the caller's out-pointer.
        unsafe { thread_id.write(id) };
    }
    h
}

/// `CreateRemoteThread`: a thread in `process` at `start(parameter)` (the address is the
/// target's and is not checked).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateRemoteThread(
    process: Handle,
    security: *const u8,
    stack: usize,
    start: *const u8,
    parameter: *mut u8,
    flags: u32,
    thread_id: *mut u32,
) -> Handle {
    let none = attrlist::ThreadParsed::default();
    // SAFETY: the caller's pointers, passed on.
    unsafe { create_remote(process, security, stack, start, parameter, flags, &none, thread_id) }
}

/// `CreateRemoteThreadEx`: `CreateRemoteThread` with a list of thread attributes (none, the
/// group affinity, the ideal processor); a process attribute in it is `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateRemoteThreadEx(
    process: Handle,
    security: *const u8,
    stack: usize,
    start: *const u8,
    parameter: *mut u8,
    flags: u32,
    list: *const u8,
    thread_id: *mut u32,
) -> Handle {
    // SAFETY: the caller's list, as Learn's contract says.
    let attributes = match unsafe { attrlist::parse_thread(list) } {
        Ok(a) => a,
        Err(_) => return fail_nt(STATUS_INVALID_PARAMETER),
    };
    // SAFETY: the caller's pointers, passed on.
    unsafe { create_remote(process, security, stack, start, parameter, flags, &attributes, thread_id) }
}
