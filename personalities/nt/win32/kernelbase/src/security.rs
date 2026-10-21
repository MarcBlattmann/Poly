//! Tokens, SIDs, security descriptors and ACLs (ADR 0083), which
//! `advapi32` forwards here through `api-ms-win-core-processthreads-l1-1-0`
//! and `api-ms-win-security-base-l1-1-0`.
//!
//! Poly has one user and enforces nothing beyond handle access (ADR 0066):
//! these functions build, check and read the structures programs pass
//! around, laid out as `winnt.h` declares them; the logic is
//! `poly_security`'s (host-tested). Tokens come from the kernel's
//! `NtOpenProcessToken` / `NtQueryInformationToken`.

use crate::base::{FALSE, TRUE, fail, fail_status, process_heap, set_last_error};
use crate::nt::{self, Handle};
use poly_kernelbase::error::*;
use poly_ntobj::NtStatus;
use poly_security::sd::{self, Parts};
use poly_security::sid::{self, Sid};
use poly_security::{acl, error as serr, token};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtOpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> NtStatus;
    fn NtOpenThreadToken(thread: Handle, access: u32, as_self: u8, token: *mut Handle) -> NtStatus;
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
}

/// `ERROR_NO_IMPERSONATION_TOKEN` (`winerror.h`).
const ERROR_NO_IMPERSONATION_TOKEN: u32 = 1309;

// ----- tokens ------------------------------------------------------------------------------

/// `OpenProcessToken`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenProcessToken(process: Handle, access: u32, out: *mut Handle) -> i32 {
    if out.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let mut h = 0;
    // SAFETY: `h` is a local.
    let st = unsafe { NtOpenProcessToken(process, access, &mut h) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: checked non-null.
    unsafe { out.write(h) };
    TRUE
}

/// `OpenThreadToken`: Poly's threads never impersonate, so this is
/// `ERROR_NO_TOKEN` for any valid thread handle (Learn's documented case).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenThreadToken(thread: Handle, access: u32, as_self: i32, out: *mut Handle) -> i32 {
    if out.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let mut h = 0;
    // SAFETY: `h` is a local.
    let st = unsafe { NtOpenThreadToken(thread, access, u8::from(as_self != 0), &mut h) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: checked non-null.
    unsafe { out.write(h) };
    TRUE
}

/// `GetTokenInformation`: the kernel lays the structure out at `info`'s
/// address; a short buffer gets nothing and `ERROR_INSUFFICIENT_BUFFER`,
/// with the size needed in `*ret` (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTokenInformation(
    token: Handle,
    class: u32,
    info: *mut u8,
    length: u32,
    ret: *mut u32,
) -> i32 {
    if info.is_null() && length != 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: `info` holds `length` bytes; `ret` is the caller's variable.
    let st = unsafe { NtQueryInformationToken(token, class, info, length, ret) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// A token's information as bytes (laid out for the returned buffer).
fn query_token(h: Handle, class: u32) -> Option<alloc::vec::Vec<u8>> {
    let mut buf = alloc::vec![0u8; 512];
    loop {
        let mut need = 0;
        // SAFETY: `buf` holds `buf.len()` bytes.
        let st = unsafe { NtQueryInformationToken(h, class, buf.as_mut_ptr(), buf.len() as u32, &mut need) };
        if st.is_success() {
            buf.truncate(need as usize);
            return Some(buf);
        }
        if need as usize <= buf.len() {
            return None;
        }
        buf.resize(need as usize, 0);
    }
}

/// The process token's user (`GetCurrentProcessToken()`, `TokenUser`).
pub fn token_user() -> Option<Sid> {
    let b = query_token(token::CURRENT_PROCESS_TOKEN as Handle, token::class::TOKEN_USER)?;
    Sid::from_bytes(b.get(16..)?)
}

/// `CheckTokenMembership`: with a null token, the calling thread's
/// effective token (the process token: threads do not impersonate). Learn:
/// a token passed in must be an impersonation token; Poly's are primary
/// tokens, so that is `ERROR_NO_IMPERSONATION_TOKEN`. A SID is a member when
/// it is the user or an enabled group that is not deny-only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CheckTokenMembership(
    token_handle: Handle,
    sid_ptr: *const u8,
    is_member: *mut i32,
) -> i32 {
    if is_member.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if token_handle != 0 {
        return fail(ERROR_NO_IMPERSONATION_TOKEN, FALSE);
    }
    // SAFETY: the caller's SID.
    let Some(want) = Sid::from_bytes(unsafe { sid_arg(sid_ptr) }) else {
        return fail(serr::ERROR_INVALID_SID, FALSE);
    };
    let h = token::CURRENT_THREAD_EFFECTIVE_TOKEN as Handle;
    let Some(groups) = query_token(h, token::class::TOKEN_GROUPS) else { return fail(ERROR_INVALID_HANDLE, FALSE) };
    let base = groups.as_ptr() as u64;
    let count = u32::from_le_bytes(groups[0..4].try_into().unwrap_or_default()) as usize;
    let mut member = token_user().is_some_and(|u| u == want);
    for i in 0..count {
        let at = 8 + 16 * i;
        let (Some(p), Some(a)) = (groups.get(at..at + 8), groups.get(at + 8..at + 12)) else { break };
        let off = (u64::from_le_bytes(p.try_into().unwrap_or_default()).wrapping_sub(base)) as usize;
        let attrs = u32::from_le_bytes(a.try_into().unwrap_or_default());
        let enabled = attrs & token::SE_GROUP_ENABLED != 0 && attrs & token::SE_GROUP_USE_FOR_DENY_ONLY == 0;
        if enabled && groups.get(off..).and_then(Sid::from_bytes).is_some_and(|s| s == want) {
            member = true;
        }
    }
    // SAFETY: checked non-null.
    unsafe { is_member.write(i32::from(member)) };
    TRUE
}

// ----- SIDs --------------------------------------------------------------------------------

/// The SID at `p` as bytes: its full length when its header is valid, else
/// only the header (so validity checks fail without reading further).
///
/// # Safety
/// `p` is null or points at a SID header and, if valid, the whole SID.
unsafe fn sid_arg<'a>(p: *const u8) -> &'a [u8] {
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

/// `AllocateAndInitializeSid`: 1 to 8 sub-authorities (Learn), in the
/// process heap (`FreeSid` frees it).
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
    if authority.is_null() || out.is_null() || !(1..=8).contains(&count) {
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

/// `EqualSid`. Learn: when they differ, `GetLastError` has more; Poly
/// clears it (`ERROR_SUCCESS`, to be confirmed by the spec agent).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EqualSid(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's SIDs.
    let (a, b) = unsafe { (sid_arg(a), sid_arg(b)) };
    if sid::equal(a, b) {
        TRUE
    } else {
        set_last_error(ERROR_SUCCESS);
        FALSE
    }
}

/// `EqualPrefixSid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EqualPrefixSid(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's SIDs.
    let (a, b) = unsafe { (sid_arg(a), sid_arg(b)) };
    if sid::equal_prefix(a, b) {
        TRUE
    } else {
        set_last_error(ERROR_SUCCESS);
        FALSE
    }
}

/// `GetLengthSid` (undefined for an invalid SID, Learn: its header decides).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLengthSid(p: *const u8) -> u32 {
    // SAFETY: the caller's SID header.
    sid::length_of(unsafe { core::slice::from_raw_parts(p, 2) })
}

/// `IsValidSid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidSid(p: *const u8) -> i32 {
    // SAFETY: the caller's SID or null.
    i32::from(sid::is_valid(unsafe { sid_arg(p) }))
}

/// `CopySid`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopySid(length: u32, dest: *mut u8, src: *const u8) -> i32 {
    // SAFETY: the caller's SID or null.
    let s = unsafe { sid_arg(src) };
    let Some(len) = sid::valid_length(s) else { return fail(serr::ERROR_INVALID_SID, FALSE) };
    if (length as usize) < len {
        return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
    }
    // SAFETY: `dest` holds `length` >= `len` bytes.
    unsafe { core::ptr::copy(s.as_ptr(), dest, len) };
    TRUE
}

/// `InitializeSid`: the header only (Learn: the sub-authorities are set
/// with `GetSidSubAuthority`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSid(p: *mut u8, authority: *const [u8; 6], count: u8) -> i32 {
    if p.is_null() || authority.is_null() || usize::from(count) > sid::SID_MAX_SUB_AUTHORITIES {
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

/// `GetSidIdentifierAuthority`.
#[unsafe(no_mangle)]
pub extern "system" fn GetSidIdentifierAuthority(p: *mut u8) -> *mut u8 {
    set_last_error(ERROR_SUCCESS);
    p.wrapping_add(2)
}

/// `GetSidSubAuthority` (no check: Learn says the result for a bad index is
/// undefined).
#[unsafe(no_mangle)]
pub extern "system" fn GetSidSubAuthority(p: *mut u8, index: u32) -> *mut u32 {
    set_last_error(ERROR_SUCCESS);
    p.wrapping_add(sid::SID_HEADER + 4 * index as usize).cast()
}

/// `GetSidSubAuthorityCount`.
#[unsafe(no_mangle)]
pub extern "system" fn GetSidSubAuthorityCount(p: *mut u8) -> *mut u8 {
    set_last_error(ERROR_SUCCESS);
    p.wrapping_add(1)
}

// ----- ACLs ---------------------------------------------------------------------------------

/// The ACL at `p` as its `AclSize` bytes (only the header if the size is
/// below it).
///
/// # Safety
/// `p` points at an ACL of `AclSize` bytes.
unsafe fn acl_arg<'a>(p: *mut u8) -> &'a mut [u8] {
    // SAFETY: the caller's contract.
    unsafe {
        let size = usize::from(u16::from_le_bytes([*p.add(2), *p.add(3)])).max(acl::ACL_HEADER);
        core::slice::from_raw_parts_mut(p, size)
    }
}

fn bool_result(r: Result<(), u32>) -> i32 {
    match r {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// `InitializeAcl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeAcl(p: *mut u8, length: u32, revision: u32) -> i32 {
    if p.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if (length as usize) < acl::ACL_HEADER {
        return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
    }
    // SAFETY: the caller's buffer of `length` bytes.
    let buf = unsafe { core::slice::from_raw_parts_mut(p, length.min(0xFFFF) as usize) };
    bool_result(acl::initialize(buf, length, revision))
}

/// `IsValidAcl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidAcl(p: *mut u8) -> i32 {
    // SAFETY: the caller's ACL.
    i32::from(!p.is_null() && acl::is_valid(unsafe { acl_arg(p) }))
}

/// `GetAclInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAclInformation(p: *mut u8, info: *mut u8, length: u32, class: u32) -> i32 {
    // SAFETY: the caller's ACL.
    match acl::information(unsafe { acl_arg(p) }, class) {
        Ok(b) if (length as usize) < b.len() => fail(ERROR_INSUFFICIENT_BUFFER, FALSE),
        Ok(b) => {
            // SAFETY: `info` holds `length` >= `b.len()` bytes.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), info, b.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// The `Add*Ace*` functions.
///
/// # Safety
/// `p` is an ACL, `s` a SID.
unsafe fn add_ace(p: *mut u8, revision: u32, kind: u8, flags: u32, mask: u32, s: *const u8) -> i32 {
    // SAFETY: the caller's ACL and SID.
    bool_result(unsafe { acl::add(acl_arg(p), revision, kind, flags, mask, sid_arg(s)) })
}

/// `AddAccessAllowedAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessAllowedAce(p: *mut u8, revision: u32, mask: u32, s: *const u8) -> i32 {
    // SAFETY: the caller's ACL and SID.
    unsafe { add_ace(p, revision, acl::ACCESS_ALLOWED_ACE_TYPE, 0, mask, s) }
}

/// `AddAccessAllowedAceEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessAllowedAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    s: *const u8,
) -> i32 {
    // SAFETY: the caller's ACL and SID.
    unsafe { add_ace(p, revision, acl::ACCESS_ALLOWED_ACE_TYPE, flags, mask, s) }
}

/// `AddAccessDeniedAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessDeniedAce(p: *mut u8, revision: u32, mask: u32, s: *const u8) -> i32 {
    // SAFETY: the caller's ACL and SID.
    unsafe { add_ace(p, revision, acl::ACCESS_DENIED_ACE_TYPE, 0, mask, s) }
}

/// `AddAccessDeniedAceEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAccessDeniedAceEx(
    p: *mut u8,
    revision: u32,
    flags: u32,
    mask: u32,
    s: *const u8,
) -> i32 {
    // SAFETY: the caller's ACL and SID.
    unsafe { add_ace(p, revision, acl::ACCESS_DENIED_ACE_TYPE, flags, mask, s) }
}

/// `AddAce`: insert `length` bytes of ACEs before ACE `start`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddAce(p: *mut u8, revision: u32, start: u32, list: *const u8, length: u32) -> i32 {
    if list.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's ACL and `length` bytes of ACEs.
    bool_result(unsafe { acl::insert(acl_arg(p), revision, start, core::slice::from_raw_parts(list, length as usize)) })
}

/// `GetAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAce(p: *mut u8, index: u32, out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's ACL.
    match acl::ace_offset(unsafe { acl_arg(p) }, index) {
        Ok(at) => {
            // SAFETY: the caller's variable; `at` is inside the ACL.
            unsafe { out.write(p.add(at)) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `DeleteAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteAce(p: *mut u8, index: u32) -> i32 {
    // SAFETY: the caller's ACL.
    bool_result(acl::delete(unsafe { acl_arg(p) }, index))
}

/// `FindFirstFreeAce`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstFreeAce(p: *mut u8, out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's ACL.
    match acl::first_free(unsafe { acl_arg(p) }) {
        Ok(at) => {
            // SAFETY: the caller's variable; `at` is at most the ACL's size.
            unsafe { out.write(p.add(at)) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

// ----- security descriptors ------------------------------------------------------------

fn read_u16(p: *const u8, at: usize) -> u16 {
    // SAFETY: the callers pass descriptors of at least 20 bytes.
    unsafe { core::ptr::read_unaligned(p.add(at).cast::<u16>()) }
}

fn read_u32(p: *const u8, at: usize) -> u32 {
    // SAFETY: as above.
    unsafe { core::ptr::read_unaligned(p.add(at).cast::<u32>()) }
}

fn read_ptr(p: *const u8, at: usize) -> *mut u8 {
    // SAFETY: the callers pass absolute descriptors of 40 bytes.
    unsafe { core::ptr::read_unaligned(p.add(at).cast::<*mut u8>()) }
}

fn write_ptr(p: *mut u8, at: usize, v: *const u8) {
    // SAFETY: as above, writable.
    unsafe { core::ptr::write_unaligned(p.add(at).cast::<*const u8>(), v) }
}

fn control(p: *const u8) -> u16 {
    read_u16(p, 2)
}

fn set_control_bits(p: *mut u8, c: u16) {
    // SAFETY: the descriptor's control word.
    unsafe { core::ptr::write_unaligned(p.add(2).cast::<u16>(), c) }
}

/// The four part pointers of a descriptor in either form (null: absent).
fn part_pointers(p: *const u8) -> [*mut u8; 4] {
    let c = control(p);
    if c & sd::SE_SELF_RELATIVE != 0 {
        [sd::REL_OWNER, sd::REL_GROUP, sd::REL_SACL, sd::REL_DACL].map(|at| match read_u32(p, at) {
            0 => core::ptr::null_mut(),
            off => p.wrapping_add(off as usize).cast_mut(),
        })
    } else {
        [sd::ABS_OWNER, sd::ABS_GROUP, sd::ABS_SACL, sd::ABS_DACL].map(|at| read_ptr(p, at))
    }
}

/// The parts of a descriptor as slices.
///
/// # Safety
/// `p` is a descriptor whose parts are readable.
unsafe fn parts<'a>(p: *const u8) -> Parts<'a> {
    let [o, g, s, d] = part_pointers(p);
    let c = control(p);
    // SAFETY: the caller's contract.
    unsafe {
        let acl_of = |a: *mut u8, present: bool| if a.is_null() || !present { None } else { Some(&*acl_arg(a)) };
        Parts {
            control: c,
            owner: (!o.is_null()).then(|| sid_arg(o)),
            group: (!g.is_null()).then(|| sid_arg(g)),
            sacl: acl_of(s, c & sd::SE_SACL_PRESENT != 0),
            dacl: acl_of(d, c & sd::SE_DACL_PRESENT != 0),
        }
    }
}

/// The revision and self-relative checks every `Set*` function makes.
fn check_absolute(p: *const u8) -> Result<(), u32> {
    if p.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's descriptor header.
    sd::check_absolute(unsafe { *p }, control(p))
}

/// `InitializeSecurityDescriptor`: an empty absolute descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSecurityDescriptor(p: *mut u8, revision: u32) -> i32 {
    if revision != u32::from(sd::SECURITY_DESCRIPTOR_REVISION) {
        return fail(serr::ERROR_UNKNOWN_REVISION, FALSE);
    }
    // SAFETY: the caller's SECURITY_DESCRIPTOR (40 bytes).
    unsafe {
        core::ptr::write_bytes(p, 0, sd::SECURITY_DESCRIPTOR_MIN_LENGTH);
        p.write(sd::SECURITY_DESCRIPTOR_REVISION);
    }
    TRUE
}

/// `IsValidSecurityDescriptor`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidSecurityDescriptor(p: *const u8) -> i32 {
    // SAFETY: the caller's descriptor.
    if p.is_null() || unsafe { *p } != sd::SECURITY_DESCRIPTOR_REVISION {
        return fail(serr::ERROR_INVALID_SECURITY_DESCR, FALSE);
    }
    // SAFETY: as above.
    bool_result(unsafe { parts(p) }.check())
}

/// `GetSecurityDescriptorLength`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorLength(p: *const u8) -> u32 {
    // SAFETY: the caller's descriptor.
    let parts = unsafe { parts(p) };
    if parts.control & sd::SE_SELF_RELATIVE != 0 {
        let end = |s: Option<&[u8]>| s.map_or(0, |s| s.as_ptr() as usize - p as usize + s.len());
        [sd::RELATIVE_HEADER, end(parts.owner), end(parts.group), end(parts.sacl), end(parts.dacl)]
            .into_iter()
            .max()
            .unwrap_or(sd::RELATIVE_HEADER) as u32
    } else {
        parts.absolute_length() as u32
    }
}

/// `GetSecurityDescriptorControl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorControl(p: *const u8, out: *mut u16, revision: *mut u32) -> i32 {
    if p.is_null() || out.is_null() || revision.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: checked non-null; the caller's variables.
    unsafe {
        out.write(control(p));
        revision.write(u32::from(*p));
    }
    TRUE
}

/// `SetSecurityDescriptorControl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorControl(p: *mut u8, mask: u16, bits: u16) -> i32 {
    if p.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's descriptor header.
    if unsafe { *p } != sd::SECURITY_DESCRIPTOR_REVISION {
        return fail(serr::ERROR_UNKNOWN_REVISION, FALSE);
    }
    match sd::set_control(control(p), mask, bits) {
        Ok(c) => {
            set_control_bits(p, c);
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

fn set_acl(
    p: *mut u8,
    field: usize,
    present_bit: u16,
    defaulted_bit: u16,
    present: i32,
    a: *mut u8,
    defaulted: i32,
) -> i32 {
    if let Err(e) = check_absolute(p) {
        return fail(e, FALSE);
    }
    let c = sd::set_acl_control(control(p), present_bit, defaulted_bit, present != 0, defaulted != 0);
    set_control_bits(p, c);
    write_ptr(p, field, if present != 0 { a } else { core::ptr::null() });
    TRUE
}

/// `SetSecurityDescriptorDacl` (the DACL is referenced, not copied: Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorDacl(p: *mut u8, present: i32, a: *mut u8, defaulted: i32) -> i32 {
    set_acl(p, sd::ABS_DACL, sd::SE_DACL_PRESENT, sd::SE_DACL_DEFAULTED, present, a, defaulted)
}

/// `SetSecurityDescriptorSacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorSacl(p: *mut u8, present: i32, a: *mut u8, defaulted: i32) -> i32 {
    set_acl(p, sd::ABS_SACL, sd::SE_SACL_PRESENT, sd::SE_SACL_DEFAULTED, present, a, defaulted)
}

/// `GetSecurityDescriptorDacl`/`Sacl`: the pointer and the defaulted flag
/// are only written when the ACL is present (Learn).
///
/// # Safety
/// The caller's descriptor and variables.
unsafe fn get_acl(
    p: *const u8,
    index: usize,
    present_bit: u16,
    defaulted_bit: u16,
    present: *mut i32,
    out: *mut *mut u8,
    defaulted: *mut i32,
) -> i32 {
    if p.is_null() || present.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's descriptor header.
    if unsafe { *p } != sd::SECURITY_DESCRIPTOR_REVISION {
        return fail(serr::ERROR_UNKNOWN_REVISION, FALSE);
    }
    let c = control(p);
    let is_present = c & present_bit != 0;
    // SAFETY: the caller's variables (the optional ones checked).
    unsafe {
        present.write(i32::from(is_present));
        if is_present {
            if !out.is_null() {
                out.write(part_pointers(p)[index]);
            }
            if !defaulted.is_null() {
                defaulted.write(i32::from(c & defaulted_bit != 0));
            }
        }
    }
    TRUE
}

/// `GetSecurityDescriptorDacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorDacl(
    p: *const u8,
    present: *mut i32,
    out: *mut *mut u8,
    defaulted: *mut i32,
) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    unsafe { get_acl(p, 3, sd::SE_DACL_PRESENT, sd::SE_DACL_DEFAULTED, present, out, defaulted) }
}

/// `GetSecurityDescriptorSacl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorSacl(
    p: *const u8,
    present: *mut i32,
    out: *mut *mut u8,
    defaulted: *mut i32,
) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    unsafe { get_acl(p, 2, sd::SE_SACL_PRESENT, sd::SE_SACL_DEFAULTED, present, out, defaulted) }
}

fn set_sid(p: *mut u8, field: usize, defaulted_bit: u16, s: *mut u8, defaulted: i32) -> i32 {
    if let Err(e) = check_absolute(p) {
        return fail(e, FALSE);
    }
    let c = if defaulted != 0 { control(p) | defaulted_bit } else { control(p) & !defaulted_bit };
    set_control_bits(p, c);
    write_ptr(p, field, s);
    TRUE
}

/// `SetSecurityDescriptorOwner`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorOwner(p: *mut u8, s: *mut u8, defaulted: i32) -> i32 {
    set_sid(p, sd::ABS_OWNER, sd::SE_OWNER_DEFAULTED, s, defaulted)
}

/// `SetSecurityDescriptorGroup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityDescriptorGroup(p: *mut u8, s: *mut u8, defaulted: i32) -> i32 {
    set_sid(p, sd::ABS_GROUP, sd::SE_GROUP_DEFAULTED, s, defaulted)
}

/// # Safety
/// The caller's descriptor and variables.
unsafe fn get_sid(p: *const u8, index: usize, defaulted_bit: u16, out: *mut *mut u8, defaulted: *mut i32) -> i32 {
    if p.is_null() || out.is_null() || defaulted.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's descriptor header.
    if unsafe { *p } != sd::SECURITY_DESCRIPTOR_REVISION {
        return fail(serr::ERROR_UNKNOWN_REVISION, FALSE);
    }
    // SAFETY: checked non-null.
    unsafe {
        out.write(part_pointers(p)[index]);
        defaulted.write(i32::from(control(p) & defaulted_bit != 0));
    }
    TRUE
}

/// `GetSecurityDescriptorOwner`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorOwner(p: *const u8, out: *mut *mut u8, defaulted: *mut i32) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    unsafe { get_sid(p, 0, sd::SE_OWNER_DEFAULTED, out, defaulted) }
}

/// `GetSecurityDescriptorGroup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityDescriptorGroup(p: *const u8, out: *mut *mut u8, defaulted: *mut i32) -> i32 {
    // SAFETY: the caller's descriptor and variables.
    unsafe { get_sid(p, 1, sd::SE_GROUP_DEFAULTED, out, defaulted) }
}

/// `MakeSelfRelativeSD`: a short buffer gets nothing and
/// `ERROR_INSUFFICIENT_BUFFER`, with the size needed in `*length` (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MakeSelfRelativeSD(abs: *const u8, rel: *mut u8, length: *mut u32) -> i32 {
    if abs.is_null() || length.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if control(abs) & sd::SE_SELF_RELATIVE != 0 {
        return fail(serr::ERROR_BAD_DESCRIPTOR_FORMAT, FALSE);
    }
    // SAFETY: the caller's absolute descriptor.
    let bytes = unsafe { parts(abs) }.to_relative();
    // SAFETY: the caller's variable and buffer of `*length` bytes.
    unsafe {
        if (length.read() as usize) < bytes.len() || rel.is_null() {
            length.write(bytes.len() as u32);
            return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), rel, bytes.len());
    }
    TRUE
}
