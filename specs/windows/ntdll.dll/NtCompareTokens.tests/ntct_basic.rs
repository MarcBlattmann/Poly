//! NtCompareTokens: a token against itself, its duplicate (primary and impersonation), copies that differ in one
//! respect (a disabled privilege, a removed privilege, a deny-only group, a lowered integrity level, a restricting
//! SID, DISABLE_MAX_PRIVILEGE) and the child's token; status and the BOOLEAN result. (Ambient: the elevated
//! administrator token.)
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
const FALSE: BOOL = 0;
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

const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IDENTIFICATION: u32 = 1;
const SECURITY_IMPERSONATION: u32 = 2;
const SE_GROUP_INTEGRITY: u32 = 0x20;
const SE_PRIVILEGE_REMOVED: u32 = 0x4;
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
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

/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// winbase.h: `STARTUPINFOW`.
#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2: u16,
    reserved2_ptr: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

impl STARTUPINFOW {
    fn new() -> STARTUPINFOW {
        // SAFETY: all-zero is a valid STARTUPINFOW (null pointers, zero numbers).
        let mut s: STARTUPINFOW = unsafe { core::mem::zeroed() };
        s.cb = core::mem::size_of::<STARTUPINFOW>() as u32;
        s
    }
}

/// processthreadsapi.h: `PROCESS_INFORMATION`.
#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

impl PROCESS_INFORMATION {
    fn new() -> PROCESS_INFORMATION {
        PROCESS_INFORMATION {
            process: core::ptr::null_mut(),
            thread: core::ptr::null_mut(),
            process_id: 0,
            thread_id: 0,
        }
    }
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
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

/// The command line to start this very executable with `arg` appended.
fn self_command(arg: &str) -> [u16; 600] {
    let mut out = [0u16; 600];
    let img = image_path();
    let mut n = 0;
    out[n] = b'"' as u16;
    n += 1;
    for &c in img {
        if n < 590 {
            out[n] = c;
            n += 1;
        }
    }
    out[n] = b'"' as u16;
    n += 1;
    out[n] = b' ' as u16;
    n += 1;
    for b in arg.bytes() {
        if n < 598 {
            out[n] = u16::from(b);
            n += 1;
        }
    }
    out
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

/// A suspended child process of this very executable (never resumed): (process, thread) or nulls. It is created
/// outside any job the test runs in when breakaway is allowed (otherwise it inherits the test's job).
fn nt_spawn() -> (HANDLE, HANDLE) {
    for flags in [CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB, CREATE_SUSPENDED | CREATE_NO_WINDOW]
    {
        let mut cmd = self_command("--child");
        let mut si = STARTUPINFOW::new();
        let mut pi = PROCESS_INFORMATION::new();
        // SAFETY: NUL-terminated writable command line, valid structures.
        let ok = unsafe {
            CreateProcessW(
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                FALSE,
                flags,
                core::ptr::null_mut(),
                core::ptr::null(),
                &mut si,
                &mut pi,
            )
        };
        if ok != 0 {
            return (pi.process, pi.thread);
        }
    }
    (core::ptr::null_mut(), core::ptr::null_mut())
}

/// End a child made by `nt_spawn` and close its handles.
fn nt_reap(process: HANDLE, thread: HANDLE) {
    // SAFETY: handles from CreateProcess; the child is terminated and waited for.
    unsafe {
        TerminateProcess(process, 0);
        WaitForSingleObject(process, 10_000);
        CloseHandle(thread);
        CloseHandle(process);
    }
}

/// A one-entry TOKEN_PRIVILEGES for the named privilege with `attrs`.
fn nt_priv(name: &str, attrs: u32) -> TOKEN_PRIVILEGES_N<1> {
    let mut st = TOKEN_PRIVILEGES_N::<1> { count: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
    let w = wide(name);
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut st.privileges[0].luid) };
    st.privileges[0].attributes = attrs;
    st
}

/// A one-entry TOKEN_GROUPS.
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_GROUPS1 {
    count: u32,
    sa: SID_AND_ATTRIBUTES,
}

fn nt_group1(sid: &SidBuf, attrs: u32) -> NT_GROUPS1 {
    NT_GROUPS1 { count: 1, sa: SID_AND_ATTRIBUTES { sid: sid.as_const() as PSID, attributes: attrs } }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn TerminateProcess(process: HANDLE, code: u32) -> BOOL;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        proc_attrs: *const SECURITY_ATTRIBUTES,
        thread_attrs: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: PVOID,
        dir: *const u16,
        si: *mut STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtOpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> i32;
    fn NtSetInformationToken(token: HANDLE, class: u32, info: *const c_void, len: u32) -> i32;
    fn NtFilterToken(
        token: HANDLE,
        flags: u32,
        disable: *const c_void,
        delete: *const c_void,
        restrict: *const c_void,
        out: *mut HANDLE,
    ) -> i32;
    fn NtAdjustPrivilegesToken(
        token: HANDLE,
        disable_all: u8,
        new_state: *const c_void,
        buf_len: u32,
        previous: *mut c_void,
        ret_len: *mut u32,
    ) -> i32;
    fn NtCompareTokens(first: HANDLE, second: HANDLE, equal: *mut u8) -> i32;
}

poly_wtest::main!(test);

fn cmp(t: &mut Test, label: &str, a: HANDLE, b: HANDLE) {
    let mut eq: u8 = 0x77;
    // SAFETY: valid token handles and out pointer.
    let st = unsafe { NtCompareTokens(a, b, &mut eq) };
    say!(t, "{label}: status={st:#010x} equal={eq:#x}");
}

fn test(t: &mut Test) {
    let base = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    cmp(t, "token with itself", base, base);
    let same = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    cmp(t, "token with another duplicate", base, same);
    cmp(t, "swapped", same, base);
    let imp = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    cmp(t, "primary with impersonation duplicate", base, imp);
    let ident = dup_token(TOKEN_ALL_ACCESS, SECURITY_IDENTIFICATION, TOKEN_IMPERSONATION);
    cmp(t, "impersonation with identification duplicate", imp, ident);
    let own = process_token(TOKEN_QUERY);
    cmp(t, "process token with its duplicate", own, base);
    // Differing in one respect.
    let d = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    let off = nt_priv("SeShutdownPrivilege", 0);
    // SAFETY: valid privilege set.
    unsafe {
        NtAdjustPrivilegesToken(
            d,
            0,
            (&off as *const TOKEN_PRIVILEGES_N<1>).cast(),
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    cmp(t, "one privilege disabled", base, d);
    let rem = nt_priv("SeBackupPrivilege", SE_PRIVILEGE_REMOVED);
    // SAFETY: valid privilege set.
    unsafe {
        NtAdjustPrivilegesToken(
            d,
            0,
            (&rem as *const TOKEN_PRIVILEGES_N<1>).cast(),
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    cmp(t, "one disabled, one removed", base, d);
    let admins = sid_of("S-1-5-32-544");
    let dis = nt_group1(&admins, 0);
    let mut f: HANDLE = core::ptr::null_mut();
    // SAFETY: valid group set and out pointer.
    unsafe { NtFilterToken(base, 0, (&dis as *const NT_GROUPS1).cast(), core::ptr::null(), core::ptr::null(), &mut f) };
    cmp(t, "Administrators deny-only", base, f);
    let mut m: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer.
    unsafe {
        NtFilterToken(base, DISABLE_MAX_PRIVILEGE, core::ptr::null(), core::ptr::null(), core::ptr::null(), &mut m)
    };
    cmp(t, "DISABLE_MAX_PRIVILEGE", base, m);
    let everyone = sid_of("S-1-1-0");
    let r = NT_GROUPS1 { count: 1, sa: SID_AND_ATTRIBUTES { sid: everyone.as_const() as PSID, attributes: 0 } };
    let mut rt: HANDLE = core::ptr::null_mut();
    // SAFETY: valid group set and out pointer.
    unsafe { NtFilterToken(base, 0, core::ptr::null(), core::ptr::null(), (&r as *const NT_GROUPS1).cast(), &mut rt) };
    cmp(t, "restricting SID Everyone", base, rt);
    cmp(t, "restricted with restricted (the same token)", rt, rt);
    let low = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    let low_sid = sid_of("S-1-16-4096");
    let label = [low_sid.as_const() as usize as u64, u64::from(SE_GROUP_INTEGRITY)];
    // SAFETY: a TOKEN_MANDATORY_LABEL (pointer and attributes) with the SID alive.
    let st = unsafe {
        NtSetInformationToken(
            low,
            TOKEN_INTEGRITY_LEVEL_CLASS,
            label.as_ptr().cast(),
            16 + low_sid.bytes().len() as u32,
        )
    };
    say!(t, "lowering a copy to Low: status={st:#010x}");
    cmp(t, "integrity Low", base, low);
    let (p, th) = nt_spawn();
    let mut ct: HANDLE = core::ptr::null_mut();
    // SAFETY: valid child process handle and out pointer.
    let st = unsafe { NtOpenProcessToken(p, TOKEN_QUERY, &mut ct) };
    say!(t, "child token: {st:#010x}");
    cmp(t, "own process token with the child's", own, ct);
    // SAFETY: closing the handles.
    unsafe {
        NtClose(ct);
        for h in [base, same, imp, ident, own, d, f, m, rt, low] {
            NtClose(h);
        }
    }
    if !p.is_null() {
        nt_reap(p, th);
    }
}
