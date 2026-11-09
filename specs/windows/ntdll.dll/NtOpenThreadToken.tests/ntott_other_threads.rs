//! NtOpenThreadToken on another thread of this process (created suspended): no token, then with an impersonation
//! token set on it, through thread handles of different access; wrong handle types.
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

const TOKEN_IMPERSONATION_LEVEL_CLASS: u32 = 9;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const CREATE_SUSPENDED: u32 = 0x4;
const THREAD_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;
const THREAD_QUERY_INFORMATION: u32 = 0x40;
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

const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x800;
const THREAD_SET_THREAD_TOKEN: u32 = 0x80;
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

unsafe extern "system" fn nt_thread_main(_: PVOID) -> u32 {
    0
}

/// A suspended thread of this process (the caller resumes it): the handle or null.
fn nt_make_thread() -> HANDLE {
    let mut id = 0u32;
    // SAFETY: a valid start routine; the thread starts suspended.
    unsafe { CreateThread(core::ptr::null(), 0, nt_thread_main, core::ptr::null_mut(), CREATE_SUSPENDED, &mut id) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
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
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn ResumeThread(thread: HANDLE) -> u32;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn SetThreadToken(thread: *const HANDLE, token: HANDLE) -> BOOL;
    fn CreateThread(
        attrs: *const SECURITY_ATTRIBUTES,
        stack: usize,
        start: unsafe extern "system" fn(PVOID) -> u32,
        param: PVOID,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
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
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtOpenThreadToken(thread: HANDLE, access: u32, open_as_self: u8, token: *mut HANDLE) -> i32;
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn show(t: &mut Test, label: &str, thread: HANDLE) {
    let mut line = Buf::<300>::new();
    for as_self in 0..=1u8 {
        let mut h: HANDLE = core::ptr::null_mut();
        // SAFETY: a valid out pointer.
        let st = unsafe { NtOpenThreadToken(thread, TOKEN_QUERY, as_self, &mut h) };
        if st >= 0 {
            let _ = write!(
                line,
                "as_self={as_self}: ok(level {} granted {:#x}) ",
                nt_tok_u32(h, TOKEN_IMPERSONATION_LEVEL_CLASS),
                nt_granted(h)
            );
            // SAFETY: closing the handle just opened.
            unsafe { NtClose(h) };
        } else {
            let _ = write!(line, "as_self={as_self}: {st:#010x} ");
        }
    }
    say!(t, "{label}: {}", line.as_str());
}

fn test(t: &mut Test) {
    let th = nt_make_thread();
    if th.is_null() {
        say!(t, "thread could not be created");
        return;
    }
    let variants: [(&str, u32); 6] = [
        ("THREAD_QUERY_LIMITED_INFORMATION", THREAD_QUERY_LIMITED_INFORMATION),
        ("THREAD_QUERY_INFORMATION", THREAD_QUERY_INFORMATION),
        ("THREAD_SET_THREAD_TOKEN", THREAD_SET_THREAD_TOKEN),
        ("SYNCHRONIZE", SYNCHRONIZE),
        ("access 0", 0),
        ("THREAD_ALL_ACCESS", THREAD_ALL_ACCESS),
    ];
    for phase in 0..2 {
        let ph = if phase == 0 { "no token" } else { "token set" };
        if phase == 1 {
            let tok = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
            // SAFETY: `th` is a thread handle with THREAD_SET_THREAD_TOKEN, `tok` an impersonation token.
            let ok = unsafe { SetThreadToken(&th, tok) };
            say!(t, "SetThreadToken on the other thread: {ok}");
            // SAFETY: closing our handle of the token (the thread keeps its own reference).
            unsafe { CloseHandle(tok) };
        }
        show(t, ph, th);
        for (label, access) in variants {
            let h = nt_dup_access(th, access);
            if h.is_null() {
                say!(t, "{ph}, {label}: handle could not be made");
                continue;
            }
            let mut b = Buf::<100>::new();
            let _ = write!(b, "{ph}, {label}");
            show(t, b.as_str(), h);
            // SAFETY: closing the duplicate.
            unsafe { CloseHandle(h) };
        }
    }
    // SAFETY: an unnamed event, a process handle and the NULL handle used as thread handles.
    let ev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null()) };
    show(t, "event handle", ev);
    show(t, "process pseudo-handle", NT_CURRENT_PROCESS);
    show(t, "NULL handle", core::ptr::null_mut());
    // SAFETY: closing the event, then using the closed value; resuming and waiting for the thread.
    unsafe {
        CloseHandle(ev);
    }
    show(t, "closed handle", ev);
    // SAFETY: the thread is suspended; it ends at once when resumed.
    unsafe {
        ResumeThread(th);
        WaitForSingleObject(th, 10_000);
        CloseHandle(th);
    }
}
