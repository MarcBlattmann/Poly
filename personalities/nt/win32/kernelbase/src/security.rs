//! Access tokens, impersonation and the security of kernel objects
//! (ADR 0083, 0171), which `advapi32` forwards here through
//! `api-ms-win-core-processthreads-l1-1-0` and
//! `api-ms-win-security-base-l1-1-0`.
//!
//! The functions are thin: the kernel's token services
//! (`NtOpenProcessToken`, `NtDuplicateToken`, `NtFilterToken`, ...) decide
//! and this layer builds their arguments from the Win32 ones and turns the
//! `NTSTATUS` into a last error. The SIDs, ACLs and security descriptors
//! are `secdata.rs`.

use crate::base::{FALSE, TRUE, fail, fail_status, set_last_error};
use crate::nt::Handle;
use crate::secdata::sid_arg;
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_ntobj::NtStatus;
use poly_security::sid::Sid;
use poly_security::token::{self, class};
use poly_security::{access, error as serr};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtOpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> NtStatus;
    fn NtOpenThreadToken(thread: Handle, access: u32, as_self: u8, token: *mut Handle) -> NtStatus;
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
    fn NtSetInformationToken(token: Handle, class: u32, info: *const u8, length: u32) -> NtStatus;
    fn NtDuplicateToken(
        token: Handle,
        access: u32,
        attributes: *const u8,
        effective_only: u8,
        ty: u32,
        new: *mut Handle,
    ) -> NtStatus;
    fn NtFilterToken(
        token: Handle,
        flags: u32,
        disable: *const u8,
        delete: *const u8,
        restrict: *const u8,
        new: *mut Handle,
    ) -> NtStatus;
    fn NtAdjustGroupsToken(
        token: Handle,
        reset: u8,
        new_state: *const u8,
        length: u32,
        previous: *mut u8,
        ret: *mut u32,
    ) -> NtStatus;
    fn NtPrivilegeCheck(token: Handle, set: *mut u8, result: *mut u8) -> NtStatus;
    fn NtAccessCheck(
        sd: *const u8,
        token: Handle,
        desired: u32,
        mapping: *const access::GenericMapping,
        privileges: *mut u8,
        privileges_length: *mut u32,
        granted: *mut u32,
        status: *mut u32,
    ) -> NtStatus;
    fn NtSetInformationThread(thread: Handle, class: u32, info: *const u8, length: u32) -> NtStatus;
    fn NtQuerySecurityObject(handle: Handle, info: u32, sd: *mut u8, length: u32, needed: *mut u32) -> NtStatus;
    fn NtSetSecurityObject(handle: Handle, info: u32, sd: *const u8) -> NtStatus;
    fn NtClose(handle: Handle) -> NtStatus;
    fn RtlImpersonateSelf(level: u32) -> NtStatus;
}

/// `ERROR_NOT_ALL_ASSIGNED` (`winerror.h`).
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
const CURRENT_THREAD: Handle = -2;

/// Success with `NTSTATUS`: `TRUE`, or the failure's last error.
fn done(st: NtStatus) -> i32 {
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

// ----- opening -----------------------------------------------------------------------------

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

/// `OpenThreadToken`: the impersonation token of a thread; none is
/// `ERROR_NO_TOKEN`.
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
    // SAFETY: `info` holds `length` bytes; `ret` is the caller's variable.
    done(unsafe { NtQueryInformationToken(token, class, info, length, ret) })
}

/// A token's information as bytes (laid out for the returned buffer).
fn query_token(h: Handle, class: u32) -> Option<Vec<u8>> {
    query_token_status(h, class).ok()
}

fn query_token_status(h: Handle, class: u32) -> Result<Vec<u8>, NtStatus> {
    let mut buf = alloc::vec![0u8; 512];
    loop {
        let mut need = 0;
        // SAFETY: `buf` holds `buf.len()` bytes.
        let st = unsafe { NtQueryInformationToken(h, class, buf.as_mut_ptr(), buf.len() as u32, &mut need) };
        if st.is_success() {
            buf.truncate(need as usize);
            return Ok(buf);
        }
        if st.0 == 0xC000_0023 && need as usize > buf.len() {
            buf.resize(need as usize, 0);
            continue;
        }
        return Err(st);
    }
}

/// The process token's user (`GetCurrentProcessToken()`, `TokenUser`).
pub fn token_user() -> Option<Sid> {
    let b = query_token(token::CURRENT_PROCESS_TOKEN as Handle, class::TOKEN_USER)?;
    Sid::from_bytes(b.get(16..)?)
}

/// `SetTokenInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetTokenInformation(token: Handle, class: u32, info: *const u8, length: u32) -> i32 {
    // SAFETY: the caller's buffer.
    done(unsafe { NtSetInformationToken(token, class, info, length) })
}

/// `IsTokenRestricted`: whether the token has restricting SIDs.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsTokenRestricted(token: Handle) -> i32 {
    match query_token_status(token, class::TOKEN_RESTRICTED_SIDS) {
        Ok(b) => i32::from(b.get(0..4).is_some_and(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) != 0)),
        Err(st) => fail_status(st, FALSE),
    }
}

// ----- adjusting ---------------------------------------------------------------------------

/// `AdjustTokenGroups`: success with the last error
/// `ERROR_NOT_ALL_ASSIGNED` when the token lacks a group named.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustTokenGroups(
    token: Handle,
    reset: i32,
    new_state: *const u8,
    length: u32,
    previous: *mut u8,
    ret: *mut u32,
) -> i32 {
    // SAFETY: the caller's structures, checked by the kernel.
    let st = unsafe { NtAdjustGroupsToken(token, u8::from(reset != 0), new_state, length, previous, ret) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    set_last_error(if st == token::STATUS_NOT_ALL_ASSIGNED { ERROR_NOT_ALL_ASSIGNED } else { ERROR_SUCCESS });
    TRUE
}

// ----- duplicating and restricting ---------------------------------------------------------

/// `SECURITY_ATTRIBUTES` (`minwinbase.h`, x64).
#[repr(C)]
pub struct SecurityAttributes {
    pub length: u32,
    pub descriptor: *const u8,
    pub inherit: i32,
}

/// The `OBJECT_ATTRIBUTES` and `SECURITY_QUALITY_OF_SERVICE` for a call
/// that takes security attributes and (if `level`) an impersonation level.
struct Attrs {
    oa: [u64; 6],
    qos: [u32; 3],
}

impl Attrs {
    fn new(sa: *const SecurityAttributes, level: Option<u32>) -> alloc::boxed::Box<Attrs> {
        let mut a = alloc::boxed::Box::new(Attrs { oa: [0; 6], qos: [12, level.unwrap_or(0), 0] });
        a.oa[0] = 48;
        if !sa.is_null() {
            // SAFETY: the caller's SECURITY_ATTRIBUTES.
            let sa = unsafe { &*sa };
            a.oa[4] = sa.descriptor as u64;
            if sa.inherit != 0 {
                a.oa[3] = u64::from(crate::nt::OBJ_INHERIT);
            }
        }
        if level.is_some() {
            a.oa[5] = a.qos.as_ptr() as u64;
        }
        a
    }
}

/// `DuplicateTokenEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DuplicateTokenEx(
    token: Handle,
    access: u32,
    sa: *const SecurityAttributes,
    level: u32,
    ty: u32,
    out: *mut Handle,
) -> i32 {
    let a = Attrs::new(sa, Some(level));
    let mut h = 0;
    // SAFETY: the attribute block and `h` are locals.
    let st = unsafe { NtDuplicateToken(token, access, a.oa.as_ptr().cast(), 0, ty, &mut h) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's variable.
    unsafe { out.write(h) };
    TRUE
}

/// `DuplicateToken`: an impersonation token with access
/// `TOKEN_IMPERSONATE | TOKEN_QUERY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DuplicateToken(token: Handle, level: u32, out: *mut Handle) -> i32 {
    // SAFETY: as `DuplicateTokenEx`.
    unsafe { DuplicateTokenEx(token, token::TOKEN_IMPERSONATE | token::TOKEN_QUERY, core::ptr::null(), level, 2, out) }
}

/// `CreateRestrictedToken`: the lists become a `TOKEN_GROUPS` (the
/// `SID_AND_ATTRIBUTES` arrays are laid out as one) and a `TOKEN_PRIVILEGES`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateRestrictedToken(
    token: Handle,
    flags: u32,
    disable_count: u32,
    disable: *const u8,
    delete_count: u32,
    delete: *const u8,
    restrict_count: u32,
    restrict: *const u8,
    out: *mut Handle,
) -> i32 {
    let groups = |count: u32, p: *const u8| -> Option<Vec<u8>> {
        if count == 0 || p.is_null() {
            return None;
        }
        let mut v = Vec::with_capacity(8 + 16 * count as usize);
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        // SAFETY: the caller's array of `count` SID_AND_ATTRIBUTES.
        v.extend_from_slice(unsafe { core::slice::from_raw_parts(p, 16 * count as usize) });
        Some(v)
    };
    let d = groups(disable_count, disable);
    let r = groups(restrict_count, restrict);
    let privileges = if delete_count == 0 || delete.is_null() {
        None
    } else {
        let mut v = Vec::with_capacity(4 + 12 * delete_count as usize);
        v.extend_from_slice(&delete_count.to_le_bytes());
        // SAFETY: the caller's array of `delete_count` LUID_AND_ATTRIBUTES.
        v.extend_from_slice(unsafe { core::slice::from_raw_parts(delete, 12 * delete_count as usize) });
        Some(v)
    };
    let mut h = 0;
    // SAFETY: the buffers are locals.
    let st = unsafe {
        NtFilterToken(
            token,
            flags,
            d.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()),
            privileges.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()),
            r.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()),
            &mut h,
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's variable.
    unsafe { out.write(h) };
    TRUE
}

// ----- checking ----------------------------------------------------------------------------

/// `PrivilegeCheck`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PrivilegeCheck(token: Handle, set: *mut u8, result: *mut i32) -> i32 {
    let mut r = 0u8;
    // SAFETY: the caller's privilege set.
    let st = unsafe { NtPrivilegeCheck(token, set, &mut r) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's variable.
    unsafe { result.write(i32::from(r)) };
    TRUE
}

/// `CheckTokenMembership`: whether `sid` is the user or an enabled group of
/// the token (also a restricting SID, if it has any). A token has to be an
/// impersonation token; none means the calling thread's, or a copy of the
/// process's. A restricted thread token cannot be opened by its own thread.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CheckTokenMembership(
    token_handle: Handle,
    sid_ptr: *const u8,
    is_member: *mut i32,
) -> i32 {
    if is_member.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's variable.
    unsafe { is_member.write(0) };
    // SAFETY: the caller's SID.
    let Some(want) = Sid::from_bytes(unsafe { sid_arg(sid_ptr) }) else {
        return fail(serr::ERROR_INVALID_SID, FALSE);
    };
    let (h, owned) = if token_handle != 0 {
        let mut ty = [0u8; 4];
        let mut ret = 0;
        // SAFETY: local buffer.
        let st = unsafe { NtQueryInformationToken(token_handle, class::TOKEN_TYPE, ty.as_mut_ptr(), 4, &mut ret) };
        if !st.is_success() {
            return fail_status(st, FALSE);
        }
        if u32::from_le_bytes(ty) != token::TOKEN_IMPERSONATION {
            return fail(1309, FALSE);
        }
        (token_handle, false)
    } else {
        let mut h = 0;
        // SAFETY: local variable.
        let st = unsafe { NtOpenThreadToken(CURRENT_THREAD, token::TOKEN_QUERY, 0, &mut h) };
        if st.is_success() {
            (h, true)
        } else if st.0 == 0xC000_007C {
            // No thread token: an identification copy of the process token.
            let mut p = 0;
            // SAFETY: local variables; the attribute block is local.
            let st = unsafe { NtOpenProcessToken(-1, token::TOKEN_DUPLICATE | token::TOKEN_QUERY, &mut p) };
            if !st.is_success() {
                return fail_status(st, FALSE);
            }
            let a = Attrs::new(core::ptr::null(), Some(token::SECURITY_IDENTIFICATION));
            // SAFETY: as above.
            let st = unsafe { NtDuplicateToken(p, token::TOKEN_QUERY, a.oa.as_ptr().cast(), 0, 2, &mut h) };
            // SAFETY: our handle.
            unsafe { NtClose(p) };
            if !st.is_success() {
                return fail_status(st, FALSE);
            }
            (h, true)
        } else {
            return fail_status(st, FALSE);
        }
    };
    let member = member_of(h, &want);
    if owned {
        // SAFETY: our handle.
        unsafe { NtClose(h) };
    }
    match member {
        Some(m) => {
            // SAFETY: the caller's variable.
            unsafe { is_member.write(i32::from(m)) };
            TRUE
        }
        None => fail(ERROR_INVALID_HANDLE, FALSE),
    }
}

/// Whether `want` is the user or an enabled group of token `h`, and a
/// restricting SID too if the token has some.
fn member_of(h: Handle, want: &Sid) -> Option<bool> {
    let groups_of = |class: u32| -> Option<Vec<(Sid, u32)>> {
        let b = query_token(h, class)?;
        let base = b.as_ptr() as u64;
        let count = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize;
        let mut out = Vec::new();
        for i in 0..count {
            let at = 8 + 16 * i;
            let off = u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?).wrapping_sub(base) as usize;
            let attrs = u32::from_le_bytes(b.get(at + 8..at + 12)?.try_into().ok()?);
            out.push((Sid::from_bytes(b.get(off..)?)?, attrs));
        }
        Some(out)
    };
    let user = {
        let b = query_token(h, class::TOKEN_USER)?;
        (Sid::from_bytes(b.get(16..)?)?, u32::from_le_bytes(b.get(8..12)?.try_into().ok()?))
    };
    let enabled = |attrs: u32| attrs & token::SE_GROUP_ENABLED != 0 && attrs & token::SE_GROUP_USE_FOR_DENY_ONLY == 0;
    let groups = groups_of(class::TOKEN_GROUPS)?;
    let normal = (user.0 == *want && user.1 & token::SE_GROUP_USE_FOR_DENY_ONLY == 0)
        || groups.iter().any(|(g, a)| g == want && enabled(*a));
    let restricted = groups_of(class::TOKEN_RESTRICTED_SIDS)?;
    Some(normal && (restricted.is_empty() || restricted.iter().any(|(g, a)| g == want && enabled(*a))))
}

/// `AccessCheck`: decides whether the impersonation token may have
/// `desired` access to an object with the descriptor. A completed decision
/// is `TRUE`; a denial sets `*status` to FALSE, `*granted` to 0 and the last
/// error `ERROR_ACCESS_DENIED` (`ERROR_PRIVILEGE_NOT_HELD` for
/// `ACCESS_SYSTEM_SECURITY`). The privilege set has to have room for one
/// entry (20 bytes) even when none is used.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AccessCheck(
    sd: *const u8,
    token: Handle,
    desired: u32,
    mapping: *const access::GenericMapping,
    privileges: *mut u8,
    privileges_length: *mut u32,
    granted: *mut u32,
    status: *mut i32,
) -> i32 {
    if privileges_length.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's variable.
    unsafe {
        if privileges_length.read() < 20 || privileges.is_null() {
            privileges_length.write(20);
            return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
        }
    }
    let mut st_out = 0xFFFF_FFFFu32;
    let mut g = 0u32;
    // SAFETY: the caller's pointers and locals.
    let st = unsafe { NtAccessCheck(sd, token, desired, mapping, privileges, privileges_length, &mut g, &mut st_out) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's variables.
    unsafe {
        granted.write(g);
        status.write(i32::from(st_out == 0));
    }
    if st_out != 0 {
        set_last_error(if st_out == 0xC000_0061 { 1314 } else { ERROR_ACCESS_DENIED });
    }
    TRUE
}

// ----- impersonation -----------------------------------------------------------------------

/// `ImpersonateSelf`.
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateSelf(level: u32) -> i32 {
    // SAFETY: no arguments to misuse.
    done(unsafe { RtlImpersonateSelf(level) })
}

/// `RevertToSelf`: always `TRUE`.
#[unsafe(no_mangle)]
pub extern "system" fn RevertToSelf() -> i32 {
    let none = 0u64.to_le_bytes();
    // SAFETY: a local buffer.
    let _ = unsafe { NtSetInformationThread(CURRENT_THREAD, 5, none.as_ptr(), 8) };
    TRUE
}

/// `SetThreadToken`: the impersonation token of the thread `*thread` (the
/// calling thread for NULL); NULL as the token removes it.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadToken(thread: *const Handle, token: Handle) -> i32 {
    // SAFETY: the caller's thread handle variable.
    let t = if thread.is_null() { CURRENT_THREAD } else { unsafe { thread.read() } };
    let b = (token as u64).to_le_bytes();
    // SAFETY: a local buffer.
    done(unsafe { NtSetInformationThread(t, 5, b.as_ptr(), 8) })
}

/// `ImpersonateLoggedOnUser`: a primary token is duplicated to an
/// impersonation token of level Impersonation first.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImpersonateLoggedOnUser(token: Handle) -> i32 {
    if token == 0 {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    let mut ty = [0u8; 4];
    let mut ret = 0;
    // SAFETY: local buffer.
    let st = unsafe { NtQueryInformationToken(token, class::TOKEN_TYPE, ty.as_mut_ptr(), 4, &mut ret) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let (h, owned) = if u32::from_le_bytes(ty) == token::TOKEN_PRIMARY {
        let a = Attrs::new(core::ptr::null(), Some(token::SECURITY_IMPERSONATION));
        let mut d = 0;
        // SAFETY: locals.
        let st = unsafe {
            NtDuplicateToken(token, token::TOKEN_IMPERSONATE | token::TOKEN_QUERY, a.oa.as_ptr().cast(), 0, 2, &mut d)
        };
        if !st.is_success() {
            return fail_status(st, FALSE);
        }
        (d, true)
    } else {
        (token, false)
    };
    let b = (h as u64).to_le_bytes();
    // SAFETY: a local buffer.
    let st = unsafe { NtSetInformationThread(CURRENT_THREAD, 5, b.as_ptr(), 8) };
    if owned {
        // SAFETY: our handle.
        unsafe { NtClose(h) };
    }
    done(st)
}

// ----- the security of kernel objects ------------------------------------------------------

/// `GetKernelObjectSecurity`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKernelObjectSecurity(
    handle: Handle,
    information: u32,
    sd: *mut u8,
    length: u32,
    needed: *mut u32,
) -> i32 {
    // SAFETY: the caller's buffer of `length` bytes and variable.
    done(unsafe { NtQuerySecurityObject(handle, information, sd, length, needed) })
}

/// `SetKernelObjectSecurity`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetKernelObjectSecurity(handle: Handle, information: u32, sd: *const u8) -> i32 {
    // SAFETY: the caller's descriptor.
    done(unsafe { NtSetSecurityObject(handle, information, sd) })
}
