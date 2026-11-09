//! The layout NtQueryInformationToken returns for the pointer-bearing classes (user, owner, primary group,
//! integrity level, groups, restricted SIDs, logon SID, privileges, default DACL, statistics, source, origin,
//! access information): where the SIDs sit inside the buffer (offsets from its start) and the sizes.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PVOID = *mut c_void;
type BOOL = i32;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const TOKEN_ASSIGN_PRIMARY: u32 = 0x0001;
const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_IMPERSONATE: u32 = 0x0004;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_QUERY_SOURCE: u32 = 0x0010;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_ADJUST_GROUPS: u32 = 0x0040;
const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
const TOKEN_ADJUST_SESSIONID: u32 = 0x0100;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const TOKEN_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED
    | TOKEN_ASSIGN_PRIMARY
    | TOKEN_DUPLICATE
    | TOKEN_IMPERSONATE
    | TOKEN_QUERY
    | TOKEN_QUERY_SOURCE
    | TOKEN_ADJUST_PRIVILEGES
    | TOKEN_ADJUST_GROUPS
    | TOKEN_ADJUST_DEFAULT
    | TOKEN_ADJUST_SESSIONID;

const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_GROUPS_CLASS: u32 = 2;
const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const TOKEN_OWNER_CLASS: u32 = 4;
const TOKEN_PRIMARY_GROUP_CLASS: u32 = 5;
const TOKEN_DEFAULT_DACL_CLASS: u32 = 6;
const TOKEN_SOURCE_CLASS: u32 = 7;
const TOKEN_STATISTICS_CLASS: u32 = 10;
const TOKEN_RESTRICTED_SIDS_CLASS: u32 = 11;
const TOKEN_ORIGIN_CLASS: u32 = 17;
const TOKEN_ACCESS_INFORMATION_CLASS: u32 = 22;
const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const TOKEN_LOGON_SID_CLASS: u32 = 28;
const TOKEN_DEVICE_GROUPS_CLASS: u32 = 37;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// An 8-aligned scratch buffer for token information and other results.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Raw<const N: usize> {
    b: [u8; N],
}

impl<const N: usize> Raw<N> {
    const fn new() -> Raw<N> {
        Raw { b: [0; N] }
    }
    fn p(&mut self) -> PVOID {
        self.b.as_mut_ptr().cast()
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.b[off], self.b[off + 1]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[off..off + 8]);
        u64::from_le_bytes(a)
    }
}

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
}

/// A pointer stored in a result buffer, as an offset from the buffer start
/// (so printed values do not depend on addresses); -1 for null, -2 outside.
fn rel(base: *const u8, len: usize, p: *const c_void) -> i64 {
    if p.is_null() {
        return -1;
    }
    let d = (p as usize).wrapping_sub(base as usize);
    if d < len { d as i64 } else { -2 }
}

/// A duplicate of the process token (`kind` TOKEN_PRIMARY / TOKEN_IMPERSONATION) or null.
fn dup_token(access: u32, level: u32, kind: u32) -> HANDLE {
    let src = process_token(TOKEN_DUPLICATE | TOKEN_QUERY);
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: `src` is the process token with TOKEN_DUPLICATE; valid out pointer.
    unsafe {
        DuplicateTokenEx(src, access, core::ptr::null(), level, kind, &mut out);
        CloseHandle(src);
    }
    out
}

/// `rel()` of a pointer result inside the ACL buffer: the offset, -1 for NULL, -2 outside.
fn acl_off<const N: usize>(buf: &Raw<N>, p: *const c_void) -> i64 {
    rel(buf.b.as_ptr(), N, p)
}

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    let mut raw = Raw::<4096>::new();
    for (name, class) in [
        ("user", TOKEN_USER_CLASS),
        ("owner", TOKEN_OWNER_CLASS),
        ("primary_group", TOKEN_PRIMARY_GROUP_CLASS),
        ("integrity", TOKEN_INTEGRITY_LEVEL_CLASS),
    ] {
        let (st, len) = nt_tq(token, class, &mut raw, 4096);
        let p = raw.u64_at(0) as usize as *const c_void;
        let off = rel(raw.b.as_ptr(), 4096, p);
        // SAFETY: the SID is inside the buffer.
        let sidlen = unsafe { GetLengthSid(p) };
        let attrs = if class == TOKEN_USER_CLASS || class == TOKEN_INTEGRITY_LEVEL_CLASS {
            i64::from(raw.u32_at(8))
        } else {
            -1
        };
        say!(
            t,
            "{name}: status={st:#010x} length={len} sid_offset={off} sid_length={sidlen} struct_attrs={attrs} total_matches={}",
            len == off as u32 + sidlen || len == 8 + sidlen
        );
    }
    for (name, class) in [
        ("groups", TOKEN_GROUPS_CLASS),
        ("restricted_sids", TOKEN_RESTRICTED_SIDS_CLASS),
        ("logon_sid", TOKEN_LOGON_SID_CLASS),
        ("device_groups", TOKEN_DEVICE_GROUPS_CLASS),
    ] {
        let (st, len) = nt_tq(token, class, &mut raw, 4096);
        let n = raw.u32_at(0) as usize;
        let first = if n > 0 { rel(raw.b.as_ptr(), 4096, raw.u64_at(8) as usize as *const c_void) } else { -1 };
        let after_array = 8 + 16 * n as i64;
        say!(t, "{name}: status={st:#010x} count={n} length={len} first_sid_offset={first} array_end={after_array}");
        if n > 0 {
            let mut sids = 0u32;
            for i in 0..n {
                // SAFETY: each pointer addresses a SID inside the buffer.
                sids += unsafe { GetLengthSid(raw.u64_at(8 + 16 * i) as usize as *const c_void) };
            }
            say!(t, "{name}: array {} + sids {sids} = {} (length {len})", after_array, after_array + i64::from(sids));
        }
    }
    let (st, len) = nt_tq(token, TOKEN_PRIVILEGES_CLASS, &mut raw, 4096);
    let n = raw.u32_at(0);
    say!(t, "privileges: status={st:#010x} count={n} length={len} = 4 + 12 * count: {}", len == 4 + 12 * n);
    let (st, len) = nt_tq(token, TOKEN_DEFAULT_DACL_CLASS, &mut raw, 4096);
    let acl_off = rel(raw.b.as_ptr(), 4096, raw.u64_at(0) as usize as *const c_void);
    say!(
        t,
        "default_dacl: status={st:#010x} length={len} acl_offset={acl_off} acl_size={} total_matches={}",
        raw.u16_at(acl_off as usize + 2),
        len == acl_off as u32 + u32::from(raw.u16_at(acl_off as usize + 2))
    );
    let (st, len) = nt_tq(token, TOKEN_STATISTICS_CLASS, &mut raw, 4096);
    say!(
        t,
        "statistics: status={st:#010x} length={len} type={} level={} dynamic_charged_field_nonzero={} dynamic_available_field_nonzero={} group_count={} privilege_count={}",
        raw.u32_at(24),
        raw.u32_at(28),
        raw.u32_at(32) != 0,
        raw.u32_at(36) != 0,
        raw.u32_at(40),
        raw.u32_at(44)
    );
    let (st, len) = nt_tq(token, TOKEN_SOURCE_CLASS, &mut raw, 4096);
    say!(t, "source: status={st:#010x} length={len}");
    let (st, len) = nt_tq(token, TOKEN_ORIGIN_CLASS, &mut raw, 4096);
    say!(t, "origin: status={st:#010x} length={len}");
    let (st, len) = nt_tq(token, TOKEN_ACCESS_INFORMATION_CLASS, &mut raw, 4096);
    let ph = rel(raw.b.as_ptr(), 4096, raw.u64_at(16) as usize as *const c_void);
    say!(
        t,
        "access_information: status={st:#010x} length={len} privileges_ptr_offset_in_buffer={} type={} level={} mandatory_policy={} flags={:#x} app_container_number={}",
        ph >= 0,
        raw.u32_at(32),
        raw.u32_at(36),
        raw.u32_at(40),
        raw.u32_at(44),
        raw.u32_at(48)
    );
    // SAFETY: closing the duplicate.
    unsafe { CloseHandle(token) };
}
