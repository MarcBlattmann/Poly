//! NtFilterToken with SIDs to disable, privileges to delete and restricting SIDs: the effect on groups
//! (deny-only), privileges and restricting SIDs, SIDs the token does not have, the user SID, the attributes given
//! with the SIDs, lists of count 0 with non-NULL pointers.
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

type PSID = *mut c_void;
type PVOID = *mut c_void;
type BOOL = i32;
type DWORD = u32;
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

const SECURITY_MAX_SID_SIZE: usize = 68;
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
const TOKEN_RESTRICTED_SIDS_CLASS: u32 = 11;
const TOKEN_HAS_RESTRICTIONS_CLASS: u32 = 21;
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
/// A `SID_IDENTIFIER_AUTHORITY` plus up to 15 sub-authorities in a buffer.
#[repr(C)]
#[derive(Clone, Copy)]
struct SidBuf {
    b: [u8; SECURITY_MAX_SID_SIZE + 4],
}

impl SidBuf {
    const fn new() -> SidBuf {
        SidBuf { b: [0; SECURITY_MAX_SID_SIZE + 4] }
    }
    fn as_psid(&mut self) -> PSID {
        self.b.as_mut_ptr().cast()
    }
    fn as_const(&self) -> *const c_void {
        self.b.as_ptr().cast()
    }
    /// The bytes of the SID (its length from the header).
    fn bytes(&self) -> &[u8] {
        let n = 8 + 4 * usize::from(self.b[1]);
        &self.b[..n.min(self.b.len())]
    }
}

/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
}

/// winnt.h: `LUID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LUID_AND_ATTRIBUTES {
    luid: LUID,
    attributes: u32,
}

/// winnt.h: `TOKEN_PRIVILEGES` with room for `N` privileges.
#[repr(C)]
#[derive(Clone, Copy)]
struct TOKEN_PRIVILEGES_N<const N: usize> {
    count: u32,
    privileges: [LUID_AND_ATTRIBUTES; N],
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

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
}

/// A SID from its string form (up to 68 bytes), through ConvertStringSidToSidW.
fn sid_of(s: &str) -> SidBuf {
    let w = wide(s);
    let mut p: PSID = core::ptr::null_mut();
    let mut out = SidBuf::new();
    // SAFETY: `w` is NUL-terminated; `p` receives a LocalAlloc'ed SID, copied and freed below.
    unsafe {
        if ConvertStringSidToSidW(w.as_ptr(), &mut p) != 0 {
            let n = GetLengthSid(p) as usize;
            core::ptr::copy_nonoverlapping(p.cast::<u8>(), out.b.as_mut_ptr(), n.min(out.b.len()));
            LocalFree(p);
        }
    }
    out
}

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
}

/// `GetTokenInformation` into `buf`; (success, needed length, last error).
fn token_info<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>) -> (bool, u32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `buf` is N writable bytes, `ret` a valid out pointer.
    let ok = unsafe { GetTokenInformation(token, class, buf.p(), N as u32, &mut ret) } != 0;
    (ok, ret, gle())
}

/// The text of a SID with the machine-dependent parts masked: S-1-5-21-a-b-c-RID prints as
/// S-1-5-21-<machine>-RID and a logon-session SID S-1-5-5-x-y as S-1-5-5-<logon>.
fn mask_sid(sid: *const c_void) -> Buf<200> {
    let full = sid_text(sid);
    let s = full.as_str();
    let mut b = Buf::<200>::new();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let rid = s.rsplit('-').next().unwrap_or("?");
        if rid.parse::<u32>().map_or(false, |r| r >= 1000) {
            let _ = write!(b, "S-1-5-21-<machine>-<account>");
        } else {
            let _ = write!(b, "S-1-5-21-<machine>-{rid}");
        }
    } else if s.starts_with("S-1-5-5-") {
        let _ = write!(b, "S-1-5-5-<logon>");
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// Whether the token's TOKEN_GROUPS has `sid_text` with attributes `attrs`: returns the attributes or -1.
fn group_attrs(token: HANDLE, class: u32, want: &str) -> i64 {
    let mut raw = Raw::<4096>::new();
    let (ok, _, _) = token_info(token, class, &mut raw);
    if !ok {
        return -2;
    }
    let n = raw.u32_at(0) as usize;
    for i in 0..n.min(60) {
        let off = 8 + 16 * i;
        let sid = raw.u64_at(off) as usize as *const c_void;
        if sid_text(sid).as_str() == want {
            return i64::from(raw.u32_at(off + 8));
        }
    }
    -1
}

/// Whether the token has the privilege (by name) and its attributes, or -1.
fn privilege_attrs(token: HANDLE, name: &str) -> i64 {
    let w = wide(name);
    let mut luid = LUID::default();
    // SAFETY: NUL-terminated name, valid out pointer.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut luid) };
    let mut raw = Raw::<4096>::new();
    let (ok, _, _) = token_info(token, TOKEN_PRIVILEGES_CLASS, &mut raw);
    if !ok {
        return -2;
    }
    let n = raw.u32_at(0) as usize;
    for i in 0..n.min(60) {
        let off = 4 + 12 * i;
        if raw.u32_at(off) == luid.low && raw.u32_at(off + 4) as i32 == luid.high {
            return i64::from(raw.u32_at(off + 8));
        }
    }
    -1
}

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

/// A one-DWORD token class through NtQueryInformationToken, or -1 when it fails.
fn nt_tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let (st, _) = nt_tq(token, class, &mut raw, 4);
    if st >= 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

/// winnt.h: `TOKEN_GROUPS` with room for `N` entries (the array starts at offset 8).
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_GROUPS_N<const N: usize> {
    count: u32,
    groups: [SID_AND_ATTRIBUTES; N],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn IsTokenRestricted(token: HANDLE) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtFilterToken(
        token: HANDLE,
        flags: u32,
        disable: *const c_void,
        delete: *const c_void,
        restrict: *const c_void,
        out: *mut HANDLE,
    ) -> i32;
}

poly_wtest::main!(test);

fn groups(sids: &[&SidBuf], attrs: u32) -> NT_GROUPS_N<4> {
    let mut g = NT_GROUPS_N::<4> {
        count: sids.len() as u32,
        groups: [SID_AND_ATTRIBUTES { sid: core::ptr::null_mut(), attributes: 0 }; 4],
    };
    for (i, s) in sids.iter().enumerate() {
        g.groups[i] = SID_AND_ATTRIBUTES { sid: s.as_const() as PSID, attributes: attrs };
    }
    g
}

fn show(t: &mut Test, out: HANDLE) {
    let mut r = Raw::<4096>::new();
    nt_tq(out, TOKEN_RESTRICTED_SIDS_CLASS, &mut r, 4096);
    let rc = r.u32_at(0);
    let mut line = Buf::<300>::new();
    for i in 0..rc.min(6) as usize {
        let sid = r.u64_at(8 + 16 * i) as usize as *const c_void;
        let _ = write!(line, "{}:{:#x} ", mask_sid(sid).as_str(), r.u32_at(16 + 16 * i));
    }
    say!(
        t,
        "  restricted={} hasrestrictions={} restricting sids: {} {}| admins={} users={} everyone={} authenticated={}",
        unsafe { IsTokenRestricted(out) },
        nt_tok_u32(out, TOKEN_HAS_RESTRICTIONS_CLASS),
        rc,
        line.as_str(),
        group_attrs(out, TOKEN_GROUPS_CLASS, "S-1-5-32-544"),
        group_attrs(out, TOKEN_GROUPS_CLASS, "S-1-5-32-545"),
        group_attrs(out, TOKEN_GROUPS_CLASS, "S-1-1-0"),
        group_attrs(out, TOKEN_GROUPS_CLASS, "S-1-5-11")
    );
}

fn test(t: &mut Test) {
    let src = process_token(TOKEN_ALL_ACCESS);
    let everyone = sid_of("S-1-1-0");
    let admins = sid_of("S-1-5-32-544");
    let users = sid_of("S-1-5-32-545");
    let system = sid_of("S-1-5-18");
    let restricted = sid_of("S-1-5-12");
    let unknown = sid_of("S-1-5-99-1");
    let mut user = Raw::<2048>::new();
    nt_tq(src, TOKEN_USER_CLASS, &mut user, 2048);
    let mut base = Raw::<4096>::new();
    nt_tq(src, TOKEN_PRIVILEGES_CLASS, &mut base, 4096);
    let base_privs = base.u32_at(0);
    let user_sid = SID_AND_ATTRIBUTES { sid: first_sid(&user) as PSID, attributes: 0 };
    let mut ug = groups(&[], 0);
    ug.count = 1;
    ug.groups[0] = user_sid;
    for (label, list) in [
        ("disable administrators", groups(&[&admins], 0)),
        ("disable administrators and users", groups(&[&admins, &users], 0)),
        ("disable everyone", groups(&[&everyone], 0)),
        ("disable SYSTEM (not in the token)", groups(&[&system], 0)),
        ("disable administrators twice", groups(&[&admins, &admins], 0)),
        ("disable administrators, attributes 0x7 given", groups(&[&admins], 0x7)),
        ("disable the user SID", ug),
        ("disable, count 0", groups(&[], 0)),
    ] {
        let mut out: HANDLE = core::ptr::null_mut();
        // SAFETY: valid group set and out pointer.
        let st = unsafe {
            NtFilterToken(
                src,
                0,
                (&list as *const NT_GROUPS_N<4>).cast(),
                core::ptr::null(),
                core::ptr::null(),
                &mut out,
            )
        };
        say!(t, "{label}: status={st:#010x}");
        if st >= 0 {
            show(t, out);
            // SAFETY: closing the result.
            unsafe { NtClose(out) };
        }
    }
    for (label, list) in [
        ("restrict everyone", groups(&[&everyone], 0)),
        ("restrict everyone and restricted", groups(&[&everyone, &restricted], 0)),
        ("restrict users", groups(&[&users], 0)),
        ("restrict unknown S-1-5-99-1", groups(&[&unknown], 0)),
        ("restrict everyone with attributes 0x17", groups(&[&everyone], 0x17)),
        ("restrict, count 0", groups(&[], 0)),
    ] {
        let mut out: HANDLE = core::ptr::null_mut();
        // SAFETY: valid group set and out pointer.
        let st = unsafe {
            NtFilterToken(
                src,
                0,
                core::ptr::null(),
                core::ptr::null(),
                (&list as *const NT_GROUPS_N<4>).cast(),
                &mut out,
            )
        };
        say!(t, "{label}: status={st:#010x}");
        if st >= 0 {
            show(t, out);
            // SAFETY: closing the result.
            unsafe { NtClose(out) };
        }
    }
    // Privileges to delete.
    for (label, names) in [
        ("delete backup and restore", &["SeBackupPrivilege", "SeRestorePrivilege"][..]),
        ("delete a privilege the token lacks (SeTcb)", &["SeTcbPrivilege"][..]),
        ("delete the same privilege twice", &["SeBackupPrivilege", "SeBackupPrivilege"][..]),
        ("delete, count 0", &[][..]),
    ] {
        let mut p =
            TOKEN_PRIVILEGES_N::<2> { count: names.len() as u32, privileges: [LUID_AND_ATTRIBUTES::default(); 2] };
        for (i, n) in names.iter().enumerate() {
            let w = wide(n);
            // SAFETY: valid name and out LUID.
            unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut p.privileges[i].luid) };
        }
        let mut out: HANDLE = core::ptr::null_mut();
        // SAFETY: valid privilege set and out pointer.
        let st = unsafe {
            NtFilterToken(
                src,
                0,
                core::ptr::null(),
                (&p as *const TOKEN_PRIVILEGES_N<2>).cast(),
                core::ptr::null(),
                &mut out,
            )
        };
        if st >= 0 {
            let mut pr = Raw::<4096>::new();
            nt_tq(out, TOKEN_PRIVILEGES_CLASS, &mut pr, 4096);
            say!(
                t,
                "{label}: status={st:#010x} privileges dropped by {} backup attrs={} restore attrs={} shutdown attrs={}",
                i64::from(base_privs) - i64::from(pr.u32_at(0)),
                privilege_attrs(out, "SeBackupPrivilege"),
                privilege_attrs(out, "SeRestorePrivilege"),
                privilege_attrs(out, "SeShutdownPrivilege")
            );
            // SAFETY: closing the result.
            unsafe { NtClose(out) };
        } else {
            say!(t, "{label}: status={st:#010x}");
        }
    }
    // All three lists in one call.
    let d = groups(&[&users], 0);
    let r = groups(&[&everyone], 0);
    let mut p = TOKEN_PRIVILEGES_N::<1> { count: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
    let w = wide("SeBackupPrivilege");
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut p.privileges[0].luid) };
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: valid sets and out pointer.
    let st = unsafe {
        NtFilterToken(
            src,
            DISABLE_MAX_PRIVILEGE,
            (&d as *const NT_GROUPS_N<4>).cast(),
            (&p as *const TOKEN_PRIVILEGES_N<1>).cast(),
            (&r as *const NT_GROUPS_N<4>).cast(),
            &mut out,
        )
    };
    say!(t, "disable users + delete backup + restrict everyone + DISABLE_MAX_PRIVILEGE: status={st:#010x}");
    if st >= 0 {
        show(t, out);
        let mut pr = Raw::<4096>::new();
        nt_tq(out, TOKEN_PRIVILEGES_CLASS, &mut pr, 4096);
        say!(t, "  privilege count={} backup attrs={}", pr.u32_at(0), privilege_attrs(out, "SeBackupPrivilege"));
        // SAFETY: closing the result.
        unsafe { NtClose(out) };
    }
    // SAFETY: closing the source.
    unsafe { CloseHandle(src) };
}
