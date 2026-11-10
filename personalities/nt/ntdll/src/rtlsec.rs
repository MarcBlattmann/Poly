//! The native security routines of `ntdll`: SIDs, ACLs, security
//! descriptors and access masks (`Rtl*`, `winnt.h`, `ntifs.h`). They report
//! `NTSTATUS`; the Win32 forms in `kernelbase` are the same logic
//! (`poly_security`, host-tested against the spec sheets) with Win32 errors.

use crate::heap::process_heap;
use crate::sys::SysPlatform;
use crate::syscalls::*;
use poly_ntobj::NtStatus;
use poly_rtl::string::UnicodeString;
use poly_security::access::GenericMapping;
use poly_security::acl::{self, AclPtr};
use poly_security::err::{SecErr, SecResult};
use poly_security::sd::{self, OutBuf, SdPtr};
use poly_security::sddl;
use poly_security::sid::{self, Sid};

const SUCCESS: NtStatus = NtStatus(0);
const STATUS_INVALID_INFO_CLASS: NtStatus = NtStatus(0xC000_0003);
const STATUS_BUFFER_OVERFLOW: NtStatus = NtStatus(0x8000_0005);
const STATUS_NO_MEMORY: NtStatus = NtStatus(0xC000_0017);

fn status(r: SecResult<()>) -> NtStatus {
    match r {
        Ok(()) => SUCCESS,
        Err(e) => e.nt(),
    }
}

/// The SID at `p` as bytes: all of it when its header is valid, else the
/// header.
///
/// # Safety
/// `p` points at a SID header and, if valid, the whole SID.
unsafe fn sid_arg<'a>(p: *const u8) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe {
        let h = core::slice::from_raw_parts(p, 2);
        let len = if h[0] == sid::SID_REVISION && usize::from(h[1]) <= sid::SID_MAX_SUB_AUTHORITIES {
            sid::length_required(h[1]) as usize
        } else {
            2
        };
        core::slice::from_raw_parts(p, len)
    }
}

/// Whether `n` bytes at `p` are committed, readable memory.
fn readable(p: *const u8, n: usize) -> bool {
    let mut at = p as usize;
    let end = at.saturating_add(n.max(1));
    while at < end {
        let mut mbi = [0u64; 6];
        let mut got = 0usize;
        // SAFETY: a MEMORY_BASIC_INFORMATION-sized buffer.
        let st = unsafe { NtQueryVirtualMemory(-1, at as *const u8, 0, mbi.as_mut_ptr().cast(), 48, &mut got) };
        if !st.is_success() {
            return false;
        }
        let (state, protect) = ((mbi[4] & 0xFFFF_FFFF) as u32, (mbi[4] >> 32) as u32);
        if state != 0x1000 || protect & 0x101 != 0 || protect == 0 {
            return false;
        }
        let region_end = (mbi[0] as usize).saturating_add(mbi[3] as usize);
        if region_end <= at {
            return false;
        }
        at = region_end;
    }
    true
}

fn heap_alloc(n: usize) -> *mut u8 {
    // SAFETY: the process heap is valid.
    unsafe { process_heap().allocate(&SysPlatform, 0, n.max(1)) }.unwrap_or(core::ptr::null_mut())
}

// ----- SIDs ---------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlLengthRequiredSid(count: u32) -> u32 {
    sid::length_required_native(count)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlLengthSid(p: *const u8) -> u32 {
    // SAFETY: the caller's SID header.
    sid::length_of(unsafe { core::slice::from_raw_parts(p, 2) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlValidSid(p: *const u8) -> u8 {
    // SAFETY: the caller's SID or null.
    u8::from(!p.is_null() && sid::is_valid(unsafe { sid_arg(p) }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlEqualSid(a: *const u8, b: *const u8) -> u8 {
    // SAFETY: the caller's SIDs.
    u8::from(sid::equal(unsafe { sid::raw(a) }, unsafe { sid::raw(b) }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlEqualPrefixSid(a: *const u8, b: *const u8) -> u8 {
    // SAFETY: the caller's SIDs.
    u8::from(sid::equal_prefix(unsafe { sid::raw(a) }, unsafe { sid::raw(b) }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlCopySid(length: u32, dest: *mut u8, src: *const u8) -> NtStatus {
    // SAFETY: the caller's SID.
    let s = unsafe { sid::raw(src) };
    if (length as usize) < s.len() {
        return SecErr::BufferTooSmall.nt();
    }
    // SAFETY: `dest` holds `length` bytes.
    unsafe { core::ptr::copy(s.as_ptr(), dest, s.len()) };
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlInitializeSid(p: *mut u8, authority: *const [u8; 6], count: u8) -> NtStatus {
    if usize::from(count) > sid::SID_MAX_SUB_AUTHORITIES {
        return SecErr::InvalidParameter.nt();
    }
    // SAFETY: `p` holds the 8-byte header; `authority` six bytes.
    unsafe {
        p.write(sid::SID_REVISION);
        p.add(1).write(count);
        core::ptr::copy_nonoverlapping(authority.cast::<u8>(), p.add(2), 6);
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAllocateAndInitializeSid(
    authority: *const [u8; 6],
    count: u8,
    s0: u32,
    s1: u32,
    s2: u32,
    s3: u32,
    s4: u32,
    s5: u32,
    s6: u32,
    s7: u32,
    out: *mut *mut u8,
) -> NtStatus {
    if count > 8 {
        return SecErr::InvalidSid.nt();
    }
    let subs = [s0, s1, s2, s3, s4, s5, s6, s7];
    // SAFETY: six bytes.
    let auth = sid::authority_of(unsafe { &*authority });
    let bytes = Sid::new(auth, &subs[..usize::from(count)]).to_bytes();
    let p = heap_alloc(bytes.len());
    if p.is_null() {
        return STATUS_NO_MEMORY;
    }
    // SAFETY: `p` holds `bytes.len()` bytes; `out` is the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        out.write(p);
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlFreeSid(p: *mut u8) -> *mut u8 {
    // SAFETY: a block from `RtlAllocateAndInitializeSid`.
    unsafe { process_heap().free(&SysPlatform, 0, p) };
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlIdentifierAuthoritySid(p: *mut u8) -> *mut u8 {
    p.wrapping_add(2)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSubAuthorityCountSid(p: *mut u8) -> *mut u8 {
    p.wrapping_add(1)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSubAuthoritySid(p: *mut u8, index: u32) -> *mut u32 {
    p.wrapping_add(sid::SID_HEADER + 4 * index as usize).cast()
}

/// `RtlConvertSidToUnicodeString`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlConvertSidToUnicodeString(
    out: *mut UnicodeString,
    p: *const u8,
    allocate: u8,
) -> NtStatus {
    // SAFETY: the caller's SID.
    let Some(text) = sddl::to_wide(unsafe { sid_arg(p) }) else { return SecErr::InvalidSid.nt() };
    let bytes = text.len() * 2;
    // SAFETY: the caller's UNICODE_STRING.
    unsafe {
        let s = &mut *out;
        if allocate != 0 {
            let buf = heap_alloc(bytes + 2).cast::<u16>();
            if buf.is_null() {
                return STATUS_NO_MEMORY;
            }
            core::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
            buf.add(text.len()).write(0);
            s.length = bytes as u16;
            s.maximum_length = (bytes + 2) as u16;
            s.buffer = buf;
        } else {
            // The caller's buffer has to be larger than the text; the NUL
            // is written only where it has room too.
            let Some(terminated) = sddl::caller_buffer_takes(s.maximum_length, text.len()) else {
                return STATUS_BUFFER_OVERFLOW;
            };
            if s.buffer.is_null() {
                return STATUS_BUFFER_OVERFLOW;
            }
            core::ptr::copy_nonoverlapping(text.as_ptr(), s.buffer, text.len());
            if terminated {
                s.buffer.add(text.len()).write(0);
            }
            s.length = bytes as u16;
        }
    }
    SUCCESS
}

// ----- ACLs ---------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlCreateAcl(p: *mut u8, length: u32, revision: u32) -> NtStatus {
    // SAFETY: the caller's buffer.
    status(unsafe { acl::initialize(AclPtr(p), length, revision) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlValidAcl(p: *mut u8) -> u8 {
    // SAFETY: the caller's ACL.
    u8::from(!p.is_null() && unsafe { acl::is_valid(AclPtr(p)) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlQueryInformationAcl(p: *mut u8, info: *mut u8, length: u32, class: u32) -> NtStatus {
    if class != acl::ACL_REVISION_INFORMATION && class != acl::ACL_SIZE_INFORMATION {
        return STATUS_INVALID_INFO_CLASS;
    }
    // SAFETY: the caller's ACL.
    match unsafe { acl::information(AclPtr(p), class, length, false) } {
        Ok(bytes) => {
            // SAFETY: `info` holds `length` >= `bytes.len()` bytes.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), info, bytes.len()) };
            SUCCESS
        }
        Err(e) => e.nt(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetInformationAcl(p: *mut u8, info: *const u8, length: u32, class: u32) -> NtStatus {
    if class != acl::ACL_REVISION_INFORMATION {
        return STATUS_INVALID_INFO_CLASS;
    }
    // SAFETY: the caller's buffer of `length` bytes.
    let bytes = if length >= 4 && !info.is_null() { unsafe { core::slice::from_raw_parts(info, 4) } } else { &[][..] };
    // SAFETY: the caller's ACL.
    status(unsafe { acl::set_information(AclPtr(p), class, bytes) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAddAce(
    p: *mut u8,
    revision: u32,
    start: u32,
    list: *const u8,
    length: u32,
) -> NtStatus {
    // SAFETY: the caller's list of `length` bytes.
    let l = if length == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(list, length as usize) } };
    // SAFETY: the caller's ACL.
    status(unsafe { acl::insert(AclPtr(p), revision, start, l) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetAce(p: *mut u8, index: u32, out: *mut *mut u8) -> NtStatus {
    // SAFETY: the caller's ACL.
    match unsafe { acl::ace_offset(AclPtr(p), index) } {
        Ok(off) => {
            // SAFETY: the caller's out pointer.
            unsafe { out.write(p.add(off)) };
            SUCCESS
        }
        Err((e, at)) => {
            if let Some(off) = at {
                // SAFETY: the caller's out pointer; the address is only stored.
                unsafe { out.write(p.wrapping_add(off)) };
            }
            e.nt()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlDeleteAce(p: *mut u8, index: u32) -> NtStatus {
    // SAFETY: the caller's ACL.
    status(unsafe { acl::delete(AclPtr(p), index) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlFirstFreeAce(p: *mut u8, out: *mut *mut u8) -> u8 {
    // SAFETY: the caller's ACL and out pointer.
    unsafe {
        match acl::first_free(AclPtr(p)) {
            Ok(at) => {
                out.write(at.map_or(core::ptr::null_mut(), |o| p.add(o)));
                1
            }
            Err(()) => {
                out.write(core::ptr::null_mut());
                0
            }
        }
    }
}

unsafe fn add_ace(
    p: *mut u8,
    revision: u32,
    kind: u8,
    flags: u32,
    allowed: u32,
    mask: u32,
    sid: *const u8,
) -> NtStatus {
    if flags & !allowed != 0 {
        return SecErr::InvalidFlags.nt();
    }
    // SAFETY: the caller's ACL and SID.
    status(unsafe { acl::add(AclPtr(p), revision, kind, flags as u8, mask, sid_arg(sid)) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAddAccessAllowedAce(p: *mut u8, revision: u32, mask: u32, sid: *const u8) -> NtStatus {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::ACCESS_ALLOWED_ACE_TYPE, 0, 0, mask, sid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAddAccessAllowedAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    sid: *const u8,
) -> NtStatus {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::ACCESS_ALLOWED_ACE_TYPE, flags, 0x3F, mask, sid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAddAccessDeniedAce(p: *mut u8, revision: u32, mask: u32, sid: *const u8) -> NtStatus {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::ACCESS_DENIED_ACE_TYPE, 0, 0, mask, sid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAddAccessDeniedAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    sid: *const u8,
) -> NtStatus {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::ACCESS_DENIED_ACE_TYPE, flags, 0x3F, mask, sid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAddMandatoryAce(
    p: *mut u8,
    revision: u32,
    flags: u32,
    label: *const u8,
    kind: u8,
    mask: u32,
) -> NtStatus {
    // A value in the first 64 KiB is no address (nothing is ever mapped
    // there): not a SID, as when the arguments are given in the order of
    // `AddMandatoryAce` and the policy or the type lands here (the sheet).
    if (label as usize) < 0x1_0000 {
        return SecErr::InvalidSid.nt();
    }
    // SAFETY: the caller's SID.
    let s = unsafe { sid_arg(label) };
    if sid::valid_length(s).is_none() {
        return SecErr::InvalidSid.nt();
    }
    if kind != acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE
        || sid::authority_of(&s[2..8]) != sid::SECURITY_MANDATORY_LABEL_AUTHORITY
        || mask > 7
        || flags & !0x1F != 0
    {
        return SecErr::InvalidParameter.nt();
    }
    // SAFETY: the caller's ACL.
    status(unsafe { acl::add(AclPtr(p), revision, kind, flags as u8, mask, s) })
}

// ----- security descriptors -----------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlCreateSecurityDescriptor(p: *mut u8, revision: u32) -> NtStatus {
    // SAFETY: the caller's 40 bytes.
    status(unsafe { sd::create(SdPtr(p), revision) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlValidSecurityDescriptor(p: *mut u8) -> u8 {
    // SAFETY: the caller's descriptor.
    u8::from(!p.is_null() && unsafe { sd::is_valid(SdPtr(p), &readable) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlLengthSecurityDescriptor(p: *mut u8) -> u32 {
    // SAFETY: the caller's descriptor.
    unsafe { sd::length(SdPtr(p)) as u32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetControlSecurityDescriptor(
    p: *mut u8,
    control: *mut u16,
    revision: *mut u32,
) -> NtStatus {
    // SAFETY: the caller's descriptor and variables.
    unsafe {
        let s = SdPtr(p);
        let rev = s.revision();
        revision.write(u32::from(rev));
        if rev != sd::SECURITY_DESCRIPTOR_REVISION {
            return SecErr::UnknownRevision.nt();
        }
        control.write(s.control());
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetControlSecurityDescriptor(p: *mut u8, mask: u16, bits: u16) -> NtStatus {
    // SAFETY: the caller's descriptor.
    status(unsafe { sd::set_control(SdPtr(p), mask, bits) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetOwnerSecurityDescriptor(p: *mut u8, owner: *mut u8, defaulted: u8) -> NtStatus {
    // SAFETY: the caller's descriptor.
    status(unsafe { sd::set_owner_or_group(SdPtr(p), false, owner, defaulted != 0) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetGroupSecurityDescriptor(p: *mut u8, group: *mut u8, defaulted: u8) -> NtStatus {
    // SAFETY: the caller's descriptor.
    status(unsafe { sd::set_owner_or_group(SdPtr(p), true, group, defaulted != 0) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetDaclSecurityDescriptor(
    p: *mut u8,
    present: u8,
    dacl: *mut u8,
    defaulted: u8,
) -> NtStatus {
    // SAFETY: the caller's descriptor.
    status(unsafe { sd::set_acl(SdPtr(p), false, present != 0, dacl, defaulted != 0) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetSaclSecurityDescriptor(
    p: *mut u8,
    present: u8,
    sacl: *mut u8,
    defaulted: u8,
) -> NtStatus {
    // SAFETY: the caller's descriptor.
    status(unsafe { sd::set_acl(SdPtr(p), true, present != 0, sacl, defaulted != 0) })
}

unsafe fn get_owner_or_group(p: *mut u8, out: *mut *mut u8, defaulted: *mut u8, group: bool) -> NtStatus {
    // SAFETY: the caller's descriptor and variables.
    match unsafe { sd::get_owner_or_group(SdPtr(p), group) } {
        Ok((sid, d)) => {
            unsafe {
                out.write(sid);
                defaulted.write(u8::from(d));
            }
            SUCCESS
        }
        Err(e) => e.nt(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetOwnerSecurityDescriptor(
    p: *mut u8,
    owner: *mut *mut u8,
    defaulted: *mut u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { get_owner_or_group(p, owner, defaulted, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetGroupSecurityDescriptor(
    p: *mut u8,
    group: *mut *mut u8,
    defaulted: *mut u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { get_owner_or_group(p, group, defaulted, true) }
}

unsafe fn get_acl(p: *mut u8, present: *mut u8, out: *mut *mut u8, defaulted: *mut u8, sacl: bool) -> NtStatus {
    // SAFETY: the caller's descriptor and variables.
    match unsafe { sd::get_acl(SdPtr(p), sacl) } {
        Ok(i) => {
            unsafe {
                present.write(u8::from(i.present));
                if i.present {
                    out.write(i.acl);
                    defaulted.write(u8::from(i.defaulted));
                }
            }
            SUCCESS
        }
        Err(e) => e.nt(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetDaclSecurityDescriptor(
    p: *mut u8,
    present: *mut u8,
    dacl: *mut *mut u8,
    defaulted: *mut u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { get_acl(p, present, dacl, defaulted, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetSaclSecurityDescriptor(
    p: *mut u8,
    present: *mut u8,
    sacl: *mut *mut u8,
    defaulted: *mut u8,
) -> NtStatus {
    // SAFETY: as above.
    unsafe { get_acl(p, present, sacl, defaulted, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlMakeSelfRelativeSD(abs: *mut u8, rel: *mut u8, length: *mut u32) -> NtStatus {
    // SAFETY: the caller's descriptors and variable.
    status(unsafe { sd::make_self_relative(SdPtr(abs), rel, &mut *length, true) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAbsoluteToSelfRelativeSD(abs: *mut u8, rel: *mut u8, length: *mut u32) -> NtStatus {
    // SAFETY: the caller's descriptors and variable.
    status(unsafe { sd::make_self_relative(SdPtr(abs), rel, &mut *length, false) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSelfRelativeToAbsoluteSD(
    rel: *mut u8,
    abs: *mut u8,
    abs_size: *mut u32,
    dacl: *mut u8,
    dacl_size: *mut u32,
    sacl: *mut u8,
    sacl_size: *mut u32,
    owner: *mut u8,
    owner_size: *mut u32,
    group: *mut u8,
    group_size: *mut u32,
) -> NtStatus {
    // SAFETY: the caller's buffers and variables.
    status(unsafe {
        sd::make_absolute(
            SdPtr(rel),
            OutBuf { ptr: abs, size: &mut *abs_size },
            OutBuf { ptr: dacl, size: &mut *dacl_size },
            OutBuf { ptr: sacl, size: &mut *sacl_size },
            OutBuf { ptr: owner, size: &mut *owner_size },
            OutBuf { ptr: group, size: &mut *group_size },
        )
    })
}

/// `RtlCopySecurityDescriptor`: a self-relative descriptor byte for byte,
/// an absolute one as a new header with the same pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlCopySecurityDescriptor(src: *mut u8, out: *mut *mut u8) -> NtStatus {
    // SAFETY: the caller's descriptor and variable.
    unsafe {
        let s = SdPtr(src);
        let n = if s.is_relative() { sd::length(s) } else { sd::SECURITY_DESCRIPTOR_MIN_LENGTH };
        let p = heap_alloc(n);
        if p.is_null() {
            return STATUS_NO_MEMORY;
        }
        core::ptr::copy_nonoverlapping(src, p, n);
        out.write(p);
    }
    SUCCESS
}

// ----- access masks -------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlMapGenericMask(mask: *mut u32, mapping: *const GenericMapping) {
    // SAFETY: the caller's variable and mapping.
    unsafe { mask.write((*mapping).map(mask.read())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAreAllAccessesGranted(granted: u32, desired: u32) -> u8 {
    u8::from(desired & !granted == 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAreAnyAccessesGranted(granted: u32, desired: u32) -> u8 {
    u8::from(granted & desired != 0)
}

// ----- tokens ------------------------------------------------------------------------------

const STATUS_NO_TOKEN: NtStatus = NtStatus(0xC000_007C);
const STATUS_PRIVILEGE_NOT_HELD: NtStatus = NtStatus(0xC000_0061);
const STATUS_BAD_IMPERSONATION_LEVEL: NtStatus = NtStatus(0xC000_00A5);
const STATUS_CANT_OPEN_ANONYMOUS: NtStatus = NtStatus(0xC000_00A6);
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x20;
const TOKEN_DUPLICATE: u32 = 0x2;
const TOKEN_IMPERSONATE: u32 = 0x4;
const TOKEN_QUERY: u32 = 0x8;

/// `RtlAdjustPrivilege(Privilege, Enable, CurrentThread, WasEnabled*)`:
/// enable or disable one privilege (the low part of its LUID) on the
/// process token, or on the thread's with `CurrentThread` (bit 0). A
/// privilege the token does not hold is `STATUS_PRIVILEGE_NOT_HELD`.
/// `*WasEnabled` is the state before, or the `Enable` argument if the call
/// changed nothing (spec sheet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAdjustPrivilege(
    privilege: u32,
    enable: u8,
    current_thread: u8,
    was_enabled: *mut u8,
) -> NtStatus {
    // The sheets: LUID 100 (no SE_* privilege has it; the last is 36) succeeds
    // on Windows and keeps a state of its own, 0 until enabled. Observed, not
    // understood: a process-wide flag reproduces it.
    if privilege == 100 {
        static STATE_100: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        let was = STATE_100.swap(enable != 0, core::sync::atomic::Ordering::Relaxed);
        // SAFETY: the caller's variable.
        unsafe { was_enabled.write(u8::from(was)) };
        return SUCCESS;
    }
    let mut token: isize = 0;
    // SAFETY: syscall stubs with local out variables.
    let st = unsafe {
        if current_thread & 1 != 0 {
            NtOpenThreadToken(-2, TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, 1, &mut token)
        } else {
            NtOpenProcessToken(-1, TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token)
        }
    };
    if !st.is_success() {
        return st;
    }
    let mut state = [0u8; 16];
    state[0] = 1;
    state[4..8].copy_from_slice(&privilege.to_le_bytes());
    let attrs: u32 = if enable != 0 { 2 } else { 0 };
    state[12..16].copy_from_slice(&attrs.to_le_bytes());
    let mut previous = [0u8; 64];
    let mut ret = 0u32;
    // SAFETY: local buffers of the sizes passed.
    let st = unsafe { NtAdjustPrivilegesToken(token, 0, state.as_ptr(), 64, previous.as_mut_ptr(), &mut ret) };
    // SAFETY: our handle.
    unsafe { NtClose(token) };
    if st == NtStatus(0x106) {
        return STATUS_PRIVILEGE_NOT_HELD;
    }
    if !st.is_success() {
        return st;
    }
    let was = if previous[0] == 1 {
        u8::from(u32::from_le_bytes([previous[12], previous[13], previous[14], previous[15]]) & 2 != 0)
    } else {
        enable
    };
    // SAFETY: the caller's variable.
    unsafe { was_enabled.write(was) };
    SUCCESS
}

/// The impersonation level of the calling thread's token: `None` without
/// one, `Some(0)` for an anonymous one (which cannot be opened).
fn thread_level() -> Result<Option<u32>, NtStatus> {
    let mut token: isize = 0;
    // SAFETY: local out variable.
    let st = unsafe { NtOpenThreadToken(-2, TOKEN_QUERY, 1, &mut token) };
    if st == STATUS_NO_TOKEN {
        return Ok(None);
    }
    if st == STATUS_CANT_OPEN_ANONYMOUS {
        return Ok(Some(0));
    }
    if !st.is_success() {
        return Err(st);
    }
    let mut level = [0u8; 4];
    let mut ret = 0u32;
    // SAFETY: local buffer; our handle.
    let st = unsafe {
        let st = NtQueryInformationToken(token, 9, level.as_mut_ptr(), 4, &mut ret);
        NtClose(token);
        st
    };
    if !st.is_success() {
        return Err(st);
    }
    Ok(Some(u32::from_le_bytes(level)))
}

/// `RtlImpersonateSelf(Level)`: an impersonation copy of the process token
/// on the calling thread; a thread that already impersonates cannot raise
/// the level.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlImpersonateSelf(level: u32) -> NtStatus {
    if level > 3 {
        return STATUS_BAD_IMPERSONATION_LEVEL;
    }
    match thread_level() {
        Ok(Some(current)) if level > current => return STATUS_BAD_IMPERSONATION_LEVEL,
        Ok(_) => {}
        Err(e) => return e,
    }
    let mut process: isize = 0;
    // SAFETY: local out variable.
    let st = unsafe { NtOpenProcessToken(-1, TOKEN_DUPLICATE, &mut process) };
    if !st.is_success() {
        return st;
    }
    // OBJECT_ATTRIBUTES with a SECURITY_QUALITY_OF_SERVICE of that level.
    let mut qos = [0u8; 12];
    qos[0..4].copy_from_slice(&12u32.to_le_bytes());
    qos[4..8].copy_from_slice(&level.to_le_bytes());
    let mut oa = [0u8; 48];
    oa[0..4].copy_from_slice(&48u32.to_le_bytes());
    oa[40..48].copy_from_slice(&(qos.as_ptr() as u64).to_le_bytes());
    let mut dup: isize = 0;
    // SAFETY: local buffers; our handle.
    let st = unsafe {
        let st = NtDuplicateToken(process, TOKEN_IMPERSONATE | TOKEN_QUERY, oa.as_ptr(), 0, 2, &mut dup);
        NtClose(process);
        st
    };
    if !st.is_success() {
        return st;
    }
    let handle = (dup as u64).to_le_bytes();
    // SAFETY: local buffer; our handle.
    let st = unsafe {
        let st = NtSetInformationThread(-2, 5, handle.as_ptr(), 8);
        NtClose(dup);
        st
    };
    st
}
