//! NtOpenProcessToken with different process handles: pseudo-handles, NULL, garbage, a closed handle, an event, a
//! real handle of this process, a suspended child process with handles of different access, and what the child's
//! token is (same user, not the same token object, type primary).
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
const TRUE: BOOL = 1;
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

const TOKEN_QUERY: u32 = 0x0008;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_STATISTICS_CLASS: u32 = 10;
const TOKEN_SESSION_ID_CLASS: u32 = 12;
const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x1;
const PROCESS_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;
/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
}

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

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// `GetTokenInformation` into `buf`; (success, needed length, last error).
fn token_info<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>) -> (bool, u32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `buf` is N writable bytes, `ret` a valid out pointer.
    let ok = unsafe { GetTokenInformation(token, class, buf.p(), N as u32, &mut ret) } != 0;
    (ok, ret, gle())
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

/// winternl.h: `PUBLIC_OBJECT_BASIC_INFORMATION` (class 0 of NtQueryObject).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PUBLIC_OBJECT_BASIC_INFORMATION {
    attributes: u32,
    granted_access: u32,
    handle_count: u32,
    pointer_count: u32,
    reserved: [u32; 10],
}

/// The access mask a handle was opened with (NtQueryObject, ObjectBasicInformation), or 0xFFFF_FFFF.
fn granted_access(h: HANDLE) -> u32 {
    let mut info = PUBLIC_OBJECT_BASIC_INFORMATION::default();
    let mut ret = 0u32;
    // SAFETY: `info` is 56 bytes, as the class requires.
    let st = unsafe { NtQueryObject(h, 0, (&mut info as *mut PUBLIC_OBJECT_BASIC_INFORMATION).cast(), 56, &mut ret) };
    if st < 0 { 0xFFFF_FFFF } else { info.granted_access }
}

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// The token's integrity RID (TokenIntegrityLevel), or -1.
fn integrity_rid(token: HANDLE) -> i64 {
    let mut raw = Raw::<2048>::new();
    let (ok, _, _) = token_info(token, TOKEN_INTEGRITY_LEVEL_CLASS, &mut raw);
    if !ok {
        return -1;
    }
    let sid = first_sid(&raw);
    // SAFETY: the SID is inside `raw`.
    unsafe {
        let n = *GetSidSubAuthorityCount(sid);
        i64::from(*GetSidSubAuthority(sid, u32::from(n) - 1))
    }
}

const PROCESS_DUP_HANDLE: u32 = 0x40;
/// The Win32 error an NTSTATUS maps to (0xFFFFFFFF text for "no mapping" is the number 317).
fn nt_dos(status: i32) -> u32 {
    // SAFETY: a pure table lookup.
    unsafe { RtlNtStatusToDosError(status) }
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

/// The handle's type name (NtQueryObject, ObjectTypeInformation, whose first field is the name).
fn nt_type_name(h: HANDLE) -> Buf<64> {
    let mut raw = Raw::<512>::new();
    let mut ret = 0u32;
    let mut out = Buf::<64>::new();
    // SAFETY: 512 writable bytes, as the class needs.
    let st = unsafe { NtQueryObject(h, 2, raw.p(), 512, &mut ret) };
    if st < 0 {
        let _ = write!(out, "<error {:#010x}>", st);
        return out;
    }
    let len = usize::from(raw.u16_at(0)) / 2;
    let p = raw.u64_at(8) as usize as *const u16;
    for i in 0..len.min(30) {
        // SAFETY: the name lies inside `raw` (len units from the pointer in the structure).
        let c = unsafe { *p.add(i) };
        let _ = write!(out, "{}", if c < 0x80 { c as u8 as char } else { '?' });
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

/// A duplicate of `h` (in this process) with exactly `access`, or null.
fn nt_dup_access(h: HANDLE, access: u32) -> HANDLE {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: DuplicateHandle with an explicit access mask (no DUPLICATE_SAME_ACCESS).
    unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut out, access, FALSE, 0) };
    out
}

/// The information NtQueryObject gives for a handle's basic information: the granted access, or 0xFFFFFFFF.
fn nt_granted(h: HANDLE) -> u32 {
    granted_access(h)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn DuplicateHandle(
        src_process: HANDLE,
        src: HANDLE,
        dst_process: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
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
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn EqualSid(a: *const c_void, b: *const c_void) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtOpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> i32;
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

poly_wtest::main!(test);

fn try_open(t: &mut Test, label: &str, process: HANDLE, access: u32) {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: a valid out pointer.
    let st = unsafe { NtOpenProcessToken(process, access, &mut h) };
    if st >= 0 {
        say!(t, "{label}: status={st:#010x} granted={:#x} type={}", nt_granted(h), nt_type_name(h).as_str());
        // SAFETY: closing the handle just opened.
        unsafe { NtClose(h) };
    } else {
        say!(t, "{label}: status={st:#010x} dos_error={}", nt_dos(st));
    }
}

fn test(t: &mut Test) {
    try_open(t, "NULL process handle", core::ptr::null_mut(), TOKEN_QUERY);
    try_open(t, "current process pseudo-handle", NT_CURRENT_PROCESS, TOKEN_QUERY);
    try_open(t, "current thread pseudo-handle", -2isize as HANDLE, TOKEN_QUERY);
    try_open(t, "garbage handle value", 0x7FF4 as HANDLE, TOKEN_QUERY);
    // SAFETY: creating and closing an unnamed event.
    let ev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null()) };
    try_open(t, "event handle", ev, TOKEN_QUERY);
    // SAFETY: closing the event.
    unsafe { CloseHandle(ev) };
    try_open(t, "closed handle", ev, TOKEN_QUERY);
    // SAFETY: opening this process by its id.
    let own = unsafe { OpenProcess(PROCESS_ALL_ACCESS, FALSE, GetCurrentProcessId()) };
    try_open(t, "real handle of this process (all access)", own, TOKEN_QUERY);
    // SAFETY: closing the handle.
    unsafe { CloseHandle(own) };
    let (p, th) = nt_spawn();
    if p.is_null() {
        say!(t, "child process could not be started");
        return;
    }
    for (label, access) in [
        ("child, all access", PROCESS_ALL_ACCESS),
        ("child, PROCESS_QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION),
        ("child, PROCESS_QUERY_INFORMATION", PROCESS_QUERY_INFORMATION),
        ("child, SYNCHRONIZE", SYNCHRONIZE),
        ("child, PROCESS_TERMINATE", PROCESS_TERMINATE),
        ("child, PROCESS_DUP_HANDLE", PROCESS_DUP_HANDLE),
        ("child, access 0", 0),
    ] {
        let h = nt_dup_access(p, access);
        if h.is_null() {
            say!(t, "{label}: handle could not be made");
            continue;
        }
        try_open(t, label, h, TOKEN_QUERY);
        // SAFETY: closing the duplicate.
        unsafe { CloseHandle(h) };
    }
    let mut a: HANDLE = core::ptr::null_mut();
    let mut b: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointers.
    let (sa, sb) = unsafe {
        (NtOpenProcessToken(NT_CURRENT_PROCESS, TOKEN_QUERY, &mut a), NtOpenProcessToken(p, TOKEN_QUERY, &mut b))
    };
    say!(t, "own token {sa:#010x}, child token {sb:#010x}");
    let mut ra = Raw::<2048>::new();
    let mut rb = Raw::<2048>::new();
    nt_tq(a, TOKEN_USER_CLASS, &mut ra, 2048);
    nt_tq(b, TOKEN_USER_CLASS, &mut rb, 2048);
    // SAFETY: both SIDs lie inside valid buffers.
    let same_user = unsafe { EqualSid(first_sid(&ra), first_sid(&rb)) } != 0;
    let mut sa_ = Raw::<64>::new();
    let mut sb_ = Raw::<64>::new();
    nt_tq(a, TOKEN_STATISTICS_CLASS, &mut sa_, 64);
    nt_tq(b, TOKEN_STATISTICS_CLASS, &mut sb_, 64);
    say!(
        t,
        "child token: same user={same_user} same token id={} same logon session={} type={} impersonation_level_field={} session={} integrity={:#x}",
        sa_.u64_at(0) == sb_.u64_at(0),
        sa_.u64_at(8) == sb_.u64_at(8),
        sb_.u32_at(24),
        sb_.u32_at(28),
        nt_tok_u32(b, TOKEN_SESSION_ID_CLASS),
        integrity_rid(b)
    );
    // SAFETY: closing the handles.
    unsafe {
        NtClose(a);
        NtClose(b);
    }
    nt_reap(p, th);
}
