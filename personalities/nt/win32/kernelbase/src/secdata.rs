//! SIDs, ACLs and security descriptors (`winnt.h`, `securitybaseapi.h`),
//! which `advapi32` forwards here through `api-ms-win-security-base-l1-1-0`.
//!
//! These functions build, check and read the structures programs pass
//! around, laid out as `winnt.h` declares them; the logic, and the errors
//! each failure reports, are `poly_security`'s (host-tested against the spec
//! sheets of `specs/windows/advapi32.dll/`). They work on the caller's memory
//! the way Windows does: a SID is as long as its count byte says, an ACL as
//! long as its `AclSize` says, nothing is validated that Windows does not
//! validate.

use crate::base::{FALSE, TRUE, fail, process_heap, set_last_error};
use crate::nt;
use poly_kernelbase::error::*;
use poly_security::acl::{self, AclPtr};
use poly_security::err::{SecErr, SecResult};
use poly_security::sd::{self, OutBuf, SdPtr};
use poly_security::sid::{self, Sid};
use poly_security::{access, wellknown};

/// Whether `n` bytes at `p` are committed, readable memory (the probe
/// `IsValidSecurityDescriptor` uses where Windows would catch a fault).
fn readable(p: *const u8, n: usize) -> bool {
    let mut at = p as usize;
    let end = at.saturating_add(n.max(1));
    while at < end {
        let mut mbi = [0u64; 6];
        let mut got = 0usize;
        // SAFETY: a MEMORY_BASIC_INFORMATION-sized buffer.
        let st = unsafe {
            nt::NtQueryVirtualMemory(nt::CURRENT_PROCESS, at as *const u8, 0, mbi.as_mut_ptr().cast(), 48, &mut got)
        };
        if !st.is_success() {
            return false;
        }
        // BaseAddress, AllocationBase, AllocationProtect/PartitionId, RegionSize, State, Protect.
        let (state, protect) = ((mbi[4] & 0xFFFF_FFFF) as u32, (mbi[4] >> 32) as u32);
        const MEM_COMMIT: u32 = 0x1000;
        const NOACCESS_OR_GUARD: u32 = 0x101;
        if state != MEM_COMMIT || protect & NOACCESS_OR_GUARD != 0 || protect == 0 {
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

fn done(r: SecResult<()>) -> i32 {
    match r {
        Ok(()) => TRUE,
        Err(e) => fail(e.win32(), FALSE),
    }
}

/// The SID at `p` as bytes: its full length when its header is valid, else
/// only the header (so validity checks fail without reading further).
///
/// # Safety
/// `p` is null or points at a SID header and, if valid, the whole SID.
pub unsafe fn sid_arg<'a>(p: *const u8) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    // SAFETY: the caller's contract.
    unsafe {
        let header = core::slice::from_raw_parts(p, 2);
        let len = if header[0] == sid::SID_REVISION && usize::from(header[1]) <= sid::SID_MAX_SUB_AUTHORITIES {
            sid::length_required(header[1]) as usize
        } else {
            2
        };
        core::slice::from_raw_parts(p, len)
    }
}

// ----- SIDs --------------------------------------------------------------------------------

/// `AllocateAndInitializeSid`: 0 to 8 sub-authorities, in the process heap
/// (`FreeSid` frees it); more is `ERROR_INVALID_SID`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AllocateAndInitializeSid(
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
) -> i32 {
    if count > 8 {
        return fail(SecErr::InvalidSid.win32(), FALSE);
    }
    if authority.is_null() || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let subs = [s0, s1, s2, s3, s4, s5, s6, s7];
    // SAFETY: checked non-null; six bytes.
    let auth = sid::authority_of(unsafe { &*authority });
    let bytes = Sid::new(auth, &subs[..usize::from(count)]).to_bytes();
    // SAFETY: the process heap.
    let p = unsafe { nt::RtlAllocateHeap(process_heap(), 0, bytes.len()) };
    if p.is_null() {
        return fail(ERROR_NOT_ENOUGH_MEMORY, FALSE);
    }
    // SAFETY: `p` holds `bytes.len()` bytes; `out` is checked non-null.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        out.write(p);
    }
    TRUE
}

/// `FreeSid`: NULL on success, the SID back on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeSid(p: *mut u8) -> *mut u8 {
    if p.is_null() {
        return p;
    }
    // SAFETY: a block of the process heap (AllocateAndInitializeSid).
    if unsafe { nt::RtlFreeHeap(process_heap(), 0, p) } != 0 { core::ptr::null_mut() } else { p }
}

/// `EqualSid`: no validity check.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EqualSid(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's SIDs.
    i32::from(sid::equal(unsafe { sid::raw(a) }, unsafe { sid::raw(b) }))
}

/// `EqualPrefixSid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EqualPrefixSid(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's SIDs.
    i32::from(sid::equal_prefix(unsafe { sid::raw(a) }, unsafe { sid::raw(b) }))
}

/// `GetLengthSid`: its count byte decides, nothing is validated.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLengthSid(p: *const u8) -> u32 {
    // SAFETY: the caller's SID header.
    sid::length_of(unsafe { core::slice::from_raw_parts(p, 2) })
}

/// `IsValidSid`: revision 1 and at most 15 sub-authorities.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidSid(p: *const u8) -> i32 {
    // SAFETY: the caller's SID or null.
    let ok = !p.is_null() && sid::is_valid(unsafe { sid_arg(p) });
    if ok { TRUE } else { fail(SecErr::InvalidSid.win32(), FALSE) }
}

/// `CopySid`: `GetLengthSid(source)` bytes, if the destination length is at
/// least that; the source is not validated.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopySid(length: u32, dest: *mut u8, src: *const u8) -> i32 {
    // SAFETY: the caller's SID.
    let s = unsafe { sid::raw(src) };
    if (length as usize) < s.len() {
        return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
    }
    // SAFETY: `dest` holds `length` >= `s.len()` bytes.
    unsafe { core::ptr::copy(s.as_ptr(), dest, s.len()) };
    TRUE
}

/// `InitializeSid`: the 8-byte header only; the sub-authorities are set with
/// `GetSidSubAuthority`. A count above 15 writes nothing.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSid(p: *mut u8, authority: *const [u8; 6], count: u8) -> i32 {
    if usize::from(count) > sid::SID_MAX_SUB_AUTHORITIES {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: `p` holds at least the 8-byte header; `authority` six bytes.
    unsafe {
        p.write(sid::SID_REVISION);
        p.add(1).write(count);
        core::ptr::copy_nonoverlapping(authority.cast::<u8>(), p.add(2), 6);
    }
    TRUE
}

/// `GetSidLengthRequired`.
#[unsafe(no_mangle)]
pub extern "system" fn GetSidLengthRequired(count: u8) -> u32 {
    sid::length_required(count)
}

/// `GetSidIdentifierAuthority`: a pointer into the SID.
#[unsafe(no_mangle)]
pub extern "system" fn GetSidIdentifierAuthority(p: *mut u8) -> *mut u8 {
    p.wrapping_add(2)
}

/// `GetSidSubAuthority`: no range check.
#[unsafe(no_mangle)]
pub extern "system" fn GetSidSubAuthority(p: *mut u8, index: u32) -> *mut u32 {
    p.wrapping_add(sid::SID_HEADER + 4 * index as usize).cast()
}

/// `GetSidSubAuthorityCount`.
#[unsafe(no_mangle)]
pub extern "system" fn GetSidSubAuthorityCount(p: *mut u8) -> *mut u8 {
    p.wrapping_add(1)
}

/// `CreateWellKnownSid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateWellKnownSid(kind: u32, domain: *const u8, out: *mut u8, size: *mut u32) -> i32 {
    if size.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's optional domain SID.
    let d = (!domain.is_null()).then(|| unsafe { sid_arg(domain) }).and_then(Sid::from_bytes);
    if !domain.is_null() && d.is_none() {
        // A domain SID that is not a SID: Windows builds from its bytes;
        // an invalid one cannot be (ERROR_INVALID_PARAMETER).
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let s = match wellknown::create(kind, d.as_ref()) {
        Ok(s) => s,
        Err(e) => return fail(e, FALSE),
    };
    let bytes = s.to_bytes();
    // SAFETY: the caller's variable and buffer of `*size` bytes.
    unsafe {
        let have = size.read();
        size.write(bytes.len() as u32);
        if (have as usize) < bytes.len() || out.is_null() {
            return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
    }
    TRUE
}

/// `IsWellKnownSid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsWellKnownSid(p: *const u8, kind: u32) -> i32 {
    // SAFETY: the caller's SID.
    let Some(s) = (unsafe { Sid::from_bytes(sid_arg(p)) }) else { return FALSE };
    i32::from(wellknown::is_well_known(&s, kind))
}

/// `EqualDomainSid`: whether two SIDs are in the same domain (an account
/// domain `S-1-5-21-a-b-c`, or the built-in domain `S-1-5-32`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EqualDomainSid(a: *const u8, b: *const u8, equal: *mut i32) -> i32 {
    // SAFETY: the caller's SIDs.
    let (x, y) = unsafe { (Sid::from_bytes(sid_arg(a)), Sid::from_bytes(sid_arg(b))) };
    let (Some(x), Some(y)) = (x, y) else { return fail(SecErr::InvalidSid.win32(), FALSE) };
    let domain = |s: &Sid| -> Option<(bool, Option<Sid>)> {
        if s.authority != sid::SECURITY_NT_AUTHORITY {
            return None;
        }
        if s.subs.first() == Some(&32) {
            return Some((true, None));
        }
        sid::account_domain(s).map(|d| (false, Some(d)))
    };
    let (Some(dx), Some(dy)) = (domain(&x), domain(&y)) else { return fail(SecErr::NonDomainSid.win32(), FALSE) };
    // SAFETY: the caller's variable.
    unsafe { equal.write(i32::from(dx == dy)) };
    TRUE
}

/// `GetWindowsAccountDomainSid`: the domain of an account SID.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowsAccountDomainSid(p: *const u8, out: *mut u8, size: *mut u32) -> i32 {
    // SAFETY: the caller's SID.
    let Some(s) = (unsafe { Sid::from_bytes(sid_arg(p)) }) else { return fail(SecErr::InvalidSid.win32(), FALSE) };
    let Some(d) = sid::account_domain(&s) else { return fail(SecErr::NonAccountSid.win32(), FALSE) };
    let bytes = d.to_bytes();
    // SAFETY: the caller's variable and buffer.
    unsafe {
        let have = size.read();
        size.write(bytes.len() as u32);
        if (have as usize) < bytes.len() || out.is_null() {
            return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
    }
    TRUE
}

// ----- ACLs ---------------------------------------------------------------------------------

fn acl_of(p: *mut u8) -> AclPtr {
    AclPtr(p)
}

/// `InitializeAcl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeAcl(p: *mut u8, length: u32, revision: u32) -> i32 {
    // SAFETY: the caller's buffer.
    done(unsafe { acl::initialize(acl_of(p), length, revision) })
}

/// `IsValidAcl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidAcl(p: *mut u8) -> i32 {
    // SAFETY: the caller's ACL.
    if !p.is_null() && unsafe { acl::is_valid(acl_of(p)) } { TRUE } else { fail(SecErr::InvalidAcl.win32(), FALSE) }
}

/// `GetAclInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAclInformation(p: *mut u8, info: *mut u8, length: u32, class: u32) -> i32 {
    // SAFETY: the caller's ACL.
    match unsafe { acl::information(acl_of(p), class, length, true) } {
        Ok(bytes) => {
            // SAFETY: `info` holds `length` >= `bytes.len()` bytes.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), info, bytes.len()) };
            TRUE
        }
        Err(e) => fail(e.win32(), FALSE),
    }
}

/// `SetAclInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetAclInformation(p: *mut u8, info: *const u8, length: u32, class: u32) -> i32 {
    // SAFETY: the caller's buffer of `length` bytes (at least 4 when read).
    let bytes = if length >= 4 && !info.is_null() { unsafe { core::slice::from_raw_parts(info, 4) } } else { &[][..] };
    // SAFETY: the caller's ACL.
    done(unsafe { acl::set_information(acl_of(p), class, bytes) })
}

/// `AddAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAce(p: *mut u8, revision: u32, start: u32, list: *const u8, length: u32) -> i32 {
    // SAFETY: the caller's list of `length` bytes.
    let l = if length == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(list, length as usize) } };
    // SAFETY: the caller's ACL.
    done(unsafe { acl::insert(acl_of(p), revision, start, l) })
}

/// `GetAce`: a pointer to ACE `index` inside the ACL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAce(p: *mut u8, index: u32, out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's ACL.
    match unsafe { acl::ace_offset(acl_of(p), index) } {
        Ok(off) => {
            // SAFETY: the caller's out pointer.
            unsafe { out.write(p.add(off)) };
            TRUE
        }
        Err((e, at)) => {
            if let Some(off) = at {
                // SAFETY: the caller's out pointer; the address is only stored.
                unsafe { out.write(p.wrapping_add(off)) };
            }
            fail(e.win32(), FALSE)
        }
    }
}

/// `DeleteAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteAce(p: *mut u8, index: u32) -> i32 {
    // SAFETY: the caller's ACL.
    done(unsafe { acl::delete(acl_of(p), index) })
}

/// `FindFirstFreeAce`: the end of the ACEs; `TRUE` with NULL when they run
/// past the ACL's size.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstFreeAce(p: *mut u8, out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's ACL.
    match unsafe { acl::first_free(acl_of(p)) } {
        Ok(at) => {
            // SAFETY: the caller's out pointer.
            unsafe { out.write(at.map_or(core::ptr::null_mut(), |o| p.add(o))) };
            TRUE
        }
        Err(()) => {
            // The walk left the ACL: no free position, as `RtlFirstFreeAce`
            // (which this function wraps) reports it.
            // SAFETY: the caller's out pointer.
            unsafe { out.write(core::ptr::null_mut()) };
            fail(SecErr::InvalidAcl.win32(), FALSE)
        }
    }
}

unsafe fn add_ace(p: *mut u8, revision: u32, kind: u8, flags: u8, mask: u32, sid: *const u8) -> i32 {
    // SAFETY: the caller's ACL and SID.
    done(unsafe { acl::add(acl_of(p), revision, kind, flags, mask, sid_arg(sid)) })
}

/// The flags an `Ex` function accepts: `allowed` bits, nothing above a byte.
fn ex_flags(flags: u32, allowed: u32) -> SecResult<u8> {
    if flags & !allowed != 0 { Err(SecErr::InvalidFlags) } else { Ok(flags as u8) }
}

/// `AddAccessAllowedAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessAllowedAce(p: *mut u8, revision: u32, mask: u32, sid: *const u8) -> i32 {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::ACCESS_ALLOWED_ACE_TYPE, 0, mask, sid) }
}

/// `AddAccessAllowedAceEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessAllowedAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    sid: *const u8,
) -> i32 {
    match ex_flags(flags, 0x3F) {
        // SAFETY: as `add_ace`.
        Ok(f) => unsafe { add_ace(p, revision, acl::ACCESS_ALLOWED_ACE_TYPE, f, mask, sid) },
        Err(e) => fail(e.win32(), FALSE),
    }
}

/// `AddAccessDeniedAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessDeniedAce(p: *mut u8, revision: u32, mask: u32, sid: *const u8) -> i32 {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::ACCESS_DENIED_ACE_TYPE, 0, mask, sid) }
}

/// `AddAccessDeniedAceEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessDeniedAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    sid: *const u8,
) -> i32 {
    match ex_flags(flags, 0x1F) {
        // SAFETY: as `add_ace`.
        Ok(f) => unsafe { add_ace(p, revision, acl::ACCESS_DENIED_ACE_TYPE, f, mask, sid) },
        Err(e) => fail(e.win32(), FALSE),
    }
}

fn audit_flags(base: u8, success: i32, failure: i32) -> u8 {
    base | if success as u8 != 0 { acl::SUCCESSFUL_ACCESS_ACE_FLAG } else { 0 }
        | if failure as u8 != 0 { acl::FAILED_ACCESS_ACE_FLAG } else { 0 }
}

/// `AddAuditAccessAce`: only the low byte of each BOOL counts.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAuditAccessAce(
    p: *mut u8,
    revision: u32,
    mask: u32,
    sid: *const u8,
    success: i32,
    failure: i32,
) -> i32 {
    // SAFETY: as `add_ace`.
    unsafe { add_ace(p, revision, acl::SYSTEM_AUDIT_ACE_TYPE, audit_flags(0, success, failure), mask, sid) }
}

/// `AddAuditAccessAceEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAuditAccessAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    sid: *const u8,
    success: i32,
    failure: i32,
) -> i32 {
    match ex_flags(flags, 0xDF) {
        // SAFETY: as `add_ace`.
        Ok(f) => unsafe {
            add_ace(p, revision, acl::SYSTEM_AUDIT_ACE_TYPE, audit_flags(f, success, failure), mask, sid)
        },
        Err(e) => fail(e.win32(), FALSE),
    }
}

/// The checks of a mandatory label ACE: a policy of 0 to 7, inheritance
/// flags only, a valid SID in the mandatory label authority.
pub fn label_args(flags: u32, policy: u32, sid: &[u8]) -> SecResult<u8> {
    if sid::valid_length(sid).is_none() {
        return Err(SecErr::InvalidSid);
    }
    if sid::authority_of(&sid[2..8]) != sid::SECURITY_MANDATORY_LABEL_AUTHORITY || policy > 7 || flags & !0x1F != 0 {
        return Err(SecErr::InvalidParameter);
    }
    Ok(flags as u8)
}

/// `AddMandatoryAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddMandatoryAce(
    p: *mut u8,
    revision: u32,
    flags: u32,
    policy: u32,
    sid: *const u8,
) -> i32 {
    // SAFETY: the caller's SID.
    let s = unsafe { sid_arg(sid) };
    match label_args(flags, policy, s) {
        // SAFETY: as `add_ace`.
        Ok(f) => unsafe { add_ace(p, revision, acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE, f, policy, sid) },
        Err(e) => fail(e.win32(), FALSE),
    }
}

// ----- security descriptors -----------------------------------------------------------------

fn sd_of(p: *mut u8) -> SdPtr {
    SdPtr(p)
}

/// `InitializeSecurityDescriptor`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSecurityDescriptor(p: *mut u8, revision: u32) -> i32 {
    // SAFETY: the caller's 40 bytes.
    done(unsafe { sd::create(sd_of(p), revision) })
}

/// `IsValidSecurityDescriptor`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidSecurityDescriptor(p: *mut u8) -> i32 {
    // SAFETY: the caller's descriptor.
    if !p.is_null() && unsafe { sd::is_valid(sd_of(p), &readable) } {
        TRUE
    } else {
        fail(SecErr::InvalidSd.win32(), FALSE)
    }
}

/// `GetSecurityDescriptorLength`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorLength(p: *mut u8) -> u32 {
    // SAFETY: the caller's descriptor.
    unsafe { sd::length(sd_of(p)) as u32 }
}

/// `GetSecurityDescriptorControl`: the revision is written even for a bad
/// one (whose control is not).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorControl(p: *mut u8, control: *mut u16, revision: *mut u32) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    unsafe {
        let s = sd_of(p);
        let rev = s.revision();
        if !revision.is_null() {
            revision.write(u32::from(rev));
        }
        if rev != sd::SECURITY_DESCRIPTOR_REVISION {
            return fail(SecErr::UnknownRevision.win32(), FALSE);
        }
        if !control.is_null() {
            control.write(s.control());
        }
    }
    TRUE
}

/// `SetSecurityDescriptorControl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorControl(p: *mut u8, mask: u16, bits: u16) -> i32 {
    // SAFETY: the caller's descriptor.
    done(unsafe { sd::set_control(sd_of(p), mask, bits) })
}

/// `SetSecurityDescriptorOwner`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorOwner(p: *mut u8, owner: *mut u8, defaulted: i32) -> i32 {
    // SAFETY: the caller's descriptor.
    done(unsafe { sd::set_owner_or_group(sd_of(p), false, owner, defaulted != 0) })
}

/// `SetSecurityDescriptorGroup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorGroup(p: *mut u8, group: *mut u8, defaulted: i32) -> i32 {
    // SAFETY: the caller's descriptor.
    done(unsafe { sd::set_owner_or_group(sd_of(p), true, group, defaulted != 0) })
}

/// `SetSecurityDescriptorDacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorDacl(
    p: *mut u8,
    present: i32,
    dacl: *mut u8,
    defaulted: i32,
) -> i32 {
    // SAFETY: the caller's descriptor.
    done(unsafe { sd::set_acl(sd_of(p), false, present != 0, dacl, defaulted != 0) })
}

/// `SetSecurityDescriptorSacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorSacl(
    p: *mut u8,
    present: i32,
    sacl: *mut u8,
    defaulted: i32,
) -> i32 {
    // SAFETY: the caller's descriptor.
    done(unsafe { sd::set_acl(sd_of(p), true, present != 0, sacl, defaulted != 0) })
}

unsafe fn get_owner_or_group(p: *mut u8, out: *mut *mut u8, defaulted: *mut i32, group: bool) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    match unsafe { sd::get_owner_or_group(sd_of(p), group) } {
        Ok((sid, d)) => {
            unsafe {
                out.write(sid);
                defaulted.write(i32::from(d));
            }
            TRUE
        }
        Err(e) => fail(e.win32(), FALSE),
    }
}

/// `GetSecurityDescriptorOwner`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorOwner(p: *mut u8, owner: *mut *mut u8, defaulted: *mut i32) -> i32 {
    // SAFETY: as above.
    unsafe { get_owner_or_group(p, owner, defaulted, false) }
}

/// `GetSecurityDescriptorGroup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorGroup(p: *mut u8, group: *mut *mut u8, defaulted: *mut i32) -> i32 {
    // SAFETY: as above.
    unsafe { get_owner_or_group(p, group, defaulted, true) }
}

unsafe fn get_acl(p: *mut u8, present: *mut i32, out: *mut *mut u8, defaulted: *mut i32, sacl: bool) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    match unsafe { sd::get_acl(sd_of(p), sacl) } {
        Ok(i) => {
            unsafe {
                present.write(i32::from(i.present));
                if i.present {
                    out.write(i.acl);
                }
                defaulted.write(i32::from(i.defaulted));
            }
            TRUE
        }
        Err(e) => fail(e.win32(), FALSE),
    }
}

/// `GetSecurityDescriptorDacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorDacl(
    p: *mut u8,
    present: *mut i32,
    dacl: *mut *mut u8,
    defaulted: *mut i32,
) -> i32 {
    // SAFETY: as above.
    unsafe { get_acl(p, present, dacl, defaulted, false) }
}

/// `GetSecurityDescriptorSacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorSacl(
    p: *mut u8,
    present: *mut i32,
    sacl: *mut *mut u8,
    defaulted: *mut i32,
) -> i32 {
    // SAFETY: as above.
    unsafe { get_acl(p, present, sacl, defaulted, true) }
}

/// `MakeSelfRelativeSD`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MakeSelfRelativeSD(abs: *mut u8, rel: *mut u8, length: *mut u32) -> i32 {
    // SAFETY: the caller's descriptors and variable.
    done(unsafe { sd::make_self_relative(sd_of(abs), rel, &mut *length, false) })
}

/// `MakeAbsoluteSD`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MakeAbsoluteSD(
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
) -> i32 {
    // SAFETY: the caller's buffers and variables.
    done(unsafe {
        sd::make_absolute(
            sd_of(rel),
            OutBuf { ptr: abs, size: &mut *abs_size },
            OutBuf { ptr: dacl, size: &mut *dacl_size },
            OutBuf { ptr: sacl, size: &mut *sacl_size },
            OutBuf { ptr: owner, size: &mut *owner_size },
            OutBuf { ptr: group, size: &mut *group_size },
        )
    })
}

/// `MapGenericMask`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapGenericMask(mask: *mut u32, mapping: *const access::GenericMapping) {
    // SAFETY: the caller's variable and mapping.
    unsafe { mask.write((*mapping).map(mask.read())) }
}

#[allow(dead_code)]
fn clear_error() {
    set_last_error(ERROR_SUCCESS);
}
