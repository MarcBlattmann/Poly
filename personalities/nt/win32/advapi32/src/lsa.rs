//! The local security authority's policy functions (Learn: `LsaOpenPolicy`, `LsaQueryInformationPolicy`,
//! `LsaFreeMemory`, `LsaClose`, `LsaNtStatusToWinError`; `ntsecapi.h`), answered from Poly's single machine: the
//! local policy opens for any access and its domain information is that of the machine (ADR 0171). The
//! MSYS2 runtime opens the policy with a NULL system name for the domain information.

use core::sync::atomic::{AtomicU64, Ordering};

/// `STATUS_SUCCESS`, `STATUS_INVALID_PARAMETER`, `STATUS_INVALID_HANDLE`, `STATUS_ACCESS_DENIED`.
const STATUS_SUCCESS: u32 = 0;
const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_ACCESS_DENIED: u32 = 0xC000_0022;
/// `POLICY_VIEW_AUDIT_INFORMATION`, `POLICY_GET_PRIVATE_INFORMATION` (`ntsecapi.h`).
const POLICY_VIEW_AUDIT_INFORMATION: u32 = 0x2;
const POLICY_GET_PRIVATE_INFORMATION: u32 = 0x4;
/// `MAXIMUM_ALLOWED`, `GENERIC_ALL` (`winnt.h`): every right.
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
/// What a policy handle starts with, to tell one from a stray pointer.
const MAGIC: u64 = 0x504F_4C59_4C53_4131;

/// An open policy: the access it was opened with.
#[repr(C)]
struct Policy {
    magic: AtomicU64,
    access: u32,
}

unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn LocalFree(mem: *mut u8) -> *mut u8;
}

/// `LsaOpenPolicy(SystemName, ObjectAttributes, DesiredAccess, PolicyHandle)`: the local policy opens
/// for any access mask (the oracle's elevated token gets it all); `ObjectAttributes` is required.
///
/// # Safety
/// `handle` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaOpenPolicy(
    _system: *const u8,
    attributes: *const u8,
    access: u32,
    handle: *mut *mut u8,
) -> u32 {
    if attributes.is_null() || handle.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: LocalAlloc returns a zeroed, 16-byte aligned block.
    let p = unsafe { LocalAlloc(0x40, core::mem::size_of::<Policy>()) } as *mut Policy;
    if p.is_null() {
        return 0xC000_0017; // STATUS_NO_MEMORY
    }
    // SAFETY: the block is ours and large enough.
    unsafe {
        (*p).magic.store(MAGIC, Ordering::Relaxed);
        (*p).access = if access & (MAXIMUM_ALLOWED | GENERIC_ALL) != 0 { u32::MAX } else { access };
        handle.write(p.cast());
    }
    STATUS_SUCCESS
}

/// `LsaClose(ObjectHandle)`.
///
/// # Safety
/// `handle` is null or a handle `LsaOpenPolicy` gave (closed once).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaClose(handle: *mut u8) -> u32 {
    let p = handle as *mut Policy;
    // SAFETY: a policy handle, which starts with the tag.
    if p.is_null() || unsafe { (*p).magic.swap(0, Ordering::AcqRel) } != MAGIC {
        return STATUS_INVALID_HANDLE;
    }
    // SAFETY: made by `LocalAlloc`.
    unsafe { LocalFree(p.cast()) };
    STATUS_SUCCESS
}

/// `LsaFreeMemory(Buffer)`: a buffer of `LsaQueryInformationPolicy`.
///
/// # Safety
/// `buffer` is null or one of those.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaFreeMemory(buffer: *mut u8) -> u32 {
    if !buffer.is_null() {
        // SAFETY: made by `LocalAlloc`.
        unsafe { LocalFree(buffer) };
    }
    STATUS_SUCCESS
}

/// The SID of the machine's account domain, `S-1-5-21-a-b-c`: revision 1, four sub-authorities.
fn machine_sid() -> [u8; 24] {
    let mut s = [0u8; 24];
    s[0] = 1;
    s[1] = 4;
    s[7] = 5;
    for (i, part) in poly_security::MACHINE_DOMAIN.iter().enumerate() {
        s[8 + 4 * i..12 + 4 * i].copy_from_slice(&part.to_le_bytes());
    }
    s
}

/// `LsaQueryInformationPolicy(PolicyHandle, InformationClass, Buffer**)`: what a view handle may ask
/// (`specs/windows/advapi32.dll/LsaOpenPolicy.yaml`). The buffer is freed with `LsaFreeMemory`.
///
/// # Safety
/// `buffer` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaQueryInformationPolicy(handle: *mut u8, class: u32, buffer: *mut *mut u8) -> u32 {
    let p = handle as *const Policy;
    // SAFETY: a policy handle, which starts with the tag.
    if p.is_null() || unsafe { (*p).magic.load(Ordering::Acquire) } != MAGIC {
        return STATUS_INVALID_HANDLE;
    }
    if buffer.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: as above.
    let access = unsafe { (*p).access };
    // The size of what each class returns (`POLICY_*_INFO`, x64) and the rights it needs.
    let (size, needs) = match class {
        1 | 2 => (16, POLICY_VIEW_AUDIT_INFORMATION),
        4 => (16, POLICY_GET_PRIVATE_INFORMATION),
        // PolicyPrimaryDomainInformation: a name and the SID.
        3 => (24, 0),
        // PolicyAccountDomainInformation, PolicyLocalAccountDomainInformation.
        5 | 14 => (24 + 24, 0),
        // PolicyLsaServerRoleInformation.
        6 => (8, 0),
        // PolicyReplicaSourceInformation: two names.
        7 => (32, 0),
        // PolicyDefaultQuotaInformation.
        8 => (48, 0),
        // PolicyDnsDomainInformation: three names, a GUID, the SID.
        12 => (72 + 24, 0),
        15 => (16, 0),
        _ => return STATUS_INVALID_PARAMETER,
    };
    if access & needs != needs {
        return STATUS_ACCESS_DENIED;
    }
    // The structure, then (after it) the machine name or the workgroup name and the SID, so one
    // `LsaFreeMemory` frees all.
    const MACHINE: &[u16] = &[0x50, 0x4F, 0x4C, 0x59, 0]; // "POLY"
    const WORKGROUP: &[u16] = &[0x57, 0x4F, 0x52, 0x4B, 0x47, 0x52, 0x4F, 0x55, 0x50, 0]; // "WORKGROUP"
    // SAFETY: LocalAlloc returns a zeroed block.
    let out = unsafe { LocalAlloc(0x40, size + 64) };
    if out.is_null() {
        return 0xC000_0017;
    }
    let sid = machine_sid();
    // SAFETY: the block holds `size` bytes and 64 more, enough for what each class writes below.
    unsafe {
        let tail = out.add(size);
        let put_name = |at: *mut u8, text: &[u16], area: *mut u8| {
            core::ptr::copy_nonoverlapping(text.as_ptr().cast::<u8>(), area, text.len() * 2);
            // LSA_UNICODE_STRING: Length, MaximumLength, Buffer (at a pointer's size: 8, or 4 in
            // a 32-bit process).
            at.cast::<u16>().write_unaligned(((text.len() - 1) * 2) as u16);
            at.cast::<u16>().add(1).write_unaligned((text.len() * 2) as u16);
            at.add(core::mem::size_of::<usize>()).cast::<*mut u8>().write_unaligned(area);
        };
        match class {
            // Primary domain of a workgroup machine: the workgroup, no SID.
            3 => put_name(out, WORKGROUP, tail),
            // The account domain is the machine itself: its name and the machine SID.
            5 | 14 => {
                put_name(out, MACHINE, tail);
                // POLICY_ACCOUNT_DOMAIN_INFO: the name (an LSA_UNICODE_STRING, two pointers'
                // size), then the SID pointer.
                let at = tail.add(16);
                core::ptr::copy_nonoverlapping(sid.as_ptr(), at, 24);
                out.add(2 * core::mem::size_of::<usize>()).cast::<*mut u8>().write_unaligned(at);
            }
            6 => out.cast::<u32>().write_unaligned(3),
            // DNS domain information of a workgroup machine: the workgroup name, no DNS names, no SID.
            12 => put_name(out, WORKGROUP, tail),
            _ => {}
        }
        buffer.write(out);
    }
    STATUS_SUCCESS
}

/// `LsaNtStatusToWinError(Status)`: the Win32 error of an `NTSTATUS` the LSA functions return.
#[unsafe(no_mangle)]
pub extern "system" fn LsaNtStatusToWinError(status: u32) -> u32 {
    match status {
        STATUS_SUCCESS => 0,
        STATUS_ACCESS_DENIED => 5,
        STATUS_INVALID_PARAMETER => 87,
        STATUS_INVALID_HANDLE => 6,
        0xC000_0017 => 8,
        0xC000_0034 => 2,
        // ERROR_MR_MID_NOT_FOUND: no mapping.
        _ => 317,
    }
}
