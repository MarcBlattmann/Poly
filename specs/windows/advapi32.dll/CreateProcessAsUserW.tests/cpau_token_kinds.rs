//! CreateProcessAsUserW with different kinds of token taken from the (elevated, no SeAssignPrimaryTokenPrivilege)
//! test process: plain and restricted primary duplicates, the process token itself, tokens missing the rights the
//! call needs, impersonation tokens, a primary token made from an impersonation token, null and bad handles; what
//! the child sees of its token; and that a thread's impersonation token does not pick the child's token.
//! The ambient token of the oracle account shapes the numbers.
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
const INVALID_HANDLE: HANDLE = -1isize as HANDLE;
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
const STANDARD_RIGHTS_READ: u32 = 0x0002_0000;
const STANDARD_RIGHTS_EXECUTE: u32 = 0x0002_0000;
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

const TOKEN_READ: u32 = STANDARD_RIGHTS_READ | TOKEN_QUERY;
const TOKEN_EXECUTE: u32 = STANDARD_RIGHTS_EXECUTE;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_GROUPS_CLASS: u32 = 2;
const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_RESTRICTED_SIDS_CLASS: u32 = 11;
const TOKEN_SANDBOX_INERT_CLASS: u32 = 15;
const TOKEN_ELEVATION_TYPE_CLASS: u32 = 18;
const TOKEN_HAS_RESTRICTIONS_CLASS: u32 = 21;
const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_ANONYMOUS: u32 = 0;
const SECURITY_IDENTIFICATION: u32 = 1;
const SECURITY_IMPERSONATION: u32 = 2;
const SECURITY_DELEGATION: u32 = 3;
const SE_GROUP_INTEGRITY: u32 = 0x20;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
const SECURITY_MANDATORY_LOW_RID: u32 = 0x1000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const DETACHED_PROCESS: u32 = 0x8;
const STARTF_USESTDHANDLES: u32 = 0x100;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const HANDLE_FLAG_INHERIT: u32 = 1;
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

/// The calling process's command line (UTF-16, no NUL), from the process
/// parameters block (winternl.h: `PEB.ProcessParameters` at 0x20,
/// `RTL_USER_PROCESS_PARAMETERS.CommandLine` at 0x70).
fn command_line() -> &'static [u16] {
    let peb: *const u8;
    // SAFETY: GS points at the TEB on x64; ProcessEnvironmentBlock is at 0x60.
    unsafe { core::arch::asm!("mov {}, gs:[0x60]", out(reg) peb, options(nostack, readonly, preserves_flags)) };
    // SAFETY: the PEB and the parameters block live as long as the process.
    unsafe {
        let params = *(peb.add(0x20) as *const *const u8);
        if params.is_null() {
            return &[];
        }
        let cl = &*(params.add(0x70) as *const UnicodeString);
        if cl.buffer.is_null() {
            return &[];
        }
        core::slice::from_raw_parts(cl.buffer, usize::from(cl.length) / 2)
    }
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

/// A child's exit code after it ended (waits up to ten seconds), or -1.
fn wait_exit(process: HANDLE) -> i64 {
    let mut code = 0u32;
    // SAFETY: `process` is a process handle with SYNCHRONIZE and PROCESS_QUERY_*.
    unsafe {
        if WaitForSingleObject(process, 10_000) != WAIT_OBJECT_0 {
            return -1;
        }
        if GetExitCodeProcess(process, &mut code) == 0 {
            return -2;
        }
    }
    i64::from(code)
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
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

/// A 32-bit token information value (class with a single DWORD), or -1 when the query fails.
fn tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let mut ret = 0u32;
    // SAFETY: `raw` is 64 writable bytes; the length given is the 4 bytes the classes need.
    let ok = unsafe { GetTokenInformation(token, class, raw.p(), 4, &mut ret) };
    if ok != 0 { i64::from(raw.u32_at(0)) } else { -1 }
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

const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
/// Text of a number: hex for values from zero up, decimal for the (negative) error markers.
fn job_hex(v: i64) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if v < 0 {
        let _ = write!(b, "{v}");
    } else {
        let _ = write!(b, "{v:#x}");
    }
    b
}

/// "ok" after a success, "fail(code)" after a failure; call it right after the API call.
fn job_r(ok: i32) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if ok != 0 {
        let _ = write!(b, "ok");
    } else {
        let _ = write!(b, "fail({})", gle());
    }
    b
}

/// A child process of this executable (`--child <mode>`); terminated, waited for and closed when dropped.
struct JobChild {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    ok: bool,
    err: i64,
}

impl Drop for JobChild {
    fn drop(&mut self) {
        // SAFETY: the handles belong to this value; terminating an exited process just fails.
        unsafe {
            if !self.process.is_null() {
                TerminateProcess(self.process, 99);
                WaitForSingleObject(self.process, 5000);
                CloseHandle(self.process);
            }
            if !self.thread.is_null() {
                CloseHandle(self.thread);
            }
        }
    }
}

impl JobChild {
    /// Still running after a third of a second.
    fn alive(&self) -> bool {
        // SAFETY: a process handle with SYNCHRONIZE.
        unsafe { WaitForSingleObject(self.process, 300) == WAIT_TIMEOUT }
    }
    /// The exit code once it ended (up to ten seconds), or -1.
    fn code(&self) -> i64 {
        wait_exit(self.process)
    }
    /// ResumeThread of the main thread: the previous suspend count.
    fn resume(&self) -> u32 {
        // SAFETY: the main thread's handle.
        unsafe { ResumeThread(self.thread) }
    }
    /// TerminateProcess with `code`.
    fn kill(&self, code: u32) -> BOOL {
        // SAFETY: the process handle.
        unsafe { TerminateProcess(self.process, code) }
    }
}

/// Starts this executable again with `--child <mode>` (DETACHED_PROCESS plus `flags`: no console, so no console host process joins a job). A null `token` uses
/// CreateProcessW, otherwise CreateProcessAsUserW. A non-null `out` (the write end of a pipe) becomes the child's
/// standard output and error and handles are inherited.
fn job_start(mode: &str, flags: u32, token: HANDLE, out: HANDLE) -> JobChild {
    let mut a = Buf::<64>::new();
    let _ = write!(a, "--child {mode}");
    let mut cmd = self_command(a.as_str());
    let mut si = STARTUPINFOW::new();
    let mut pi = PROCESS_INFORMATION::new();
    let mut inherit = FALSE;
    if !out.is_null() {
        si.flags = STARTF_USESTDHANDLES;
        si.std_output = out;
        si.std_error = out;
        inherit = TRUE;
    }
    clear_error();
    // SAFETY: the command line is a writable NUL-terminated buffer; the structures are valid.
    let ok = unsafe {
        if token.is_null() {
            CreateProcessW(
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                inherit,
                DETACHED_PROCESS | flags,
                core::ptr::null_mut(),
                core::ptr::null(),
                &mut si,
                &mut pi,
            )
        } else {
            CreateProcessAsUserW(
                token,
                core::ptr::null(),
                cmd.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                inherit,
                DETACHED_PROCESS | flags,
                core::ptr::null_mut(),
                core::ptr::null(),
                &mut si,
                &mut pi,
            )
        }
    };
    JobChild { process: pi.process, thread: pi.thread, pid: pi.process_id, ok: ok != 0, err: gle_if(ok) }
}

/// An anonymous pipe whose write end is inheritable and whose read end is not.
struct JobPipe {
    read: HANDLE,
    write: HANDLE,
}

impl Drop for JobPipe {
    fn drop(&mut self) {
        // SAFETY: the handles belong to this value.
        unsafe {
            if !self.read.is_null() {
                CloseHandle(self.read);
            }
            if !self.write.is_null() {
                CloseHandle(self.write);
            }
        }
    }
}

impl JobPipe {
    fn new() -> JobPipe {
        let sa = SECURITY_ATTRIBUTES { length: 24, descriptor: core::ptr::null_mut(), inherit: TRUE };
        let mut r: HANDLE = core::ptr::null_mut();
        let mut w: HANDLE = core::ptr::null_mut();
        // SAFETY: valid out pointers and attributes.
        unsafe {
            job_create_pipe(&mut r, &mut w, &sa, 0);
            job_set_handle_info(r, HANDLE_FLAG_INHERIT, 0);
        }
        JobPipe { read: r, write: w }
    }
    /// Closes the parent's copy of the write end (so the read side sees the end of the data).
    fn close_write(&mut self) {
        if !self.write.is_null() {
            // SAFETY: closing our own handle once.
            unsafe { CloseHandle(self.write) };
            self.write = core::ptr::null_mut();
        }
    }
    /// What the child has written so far; waits up to `wait_ms` for the first data.
    fn text(&self, wait_ms: u32) -> Buf<1200> {
        let mut out = Buf::<1200>::new();
        let mut waited = 0;
        loop {
            let mut avail = 0u32;
            // SAFETY: valid out pointer; the other pointers are null.
            let ok = unsafe {
                job_peek_pipe(
                    self.read,
                    core::ptr::null_mut(),
                    0,
                    core::ptr::null_mut(),
                    &mut avail,
                    core::ptr::null_mut(),
                )
            };
            if ok == 0 {
                break;
            }
            if avail > 0 {
                let mut tmp = [0u8; 1000];
                let mut got = 0u32;
                // SAFETY: `tmp` has room for the 1000 bytes asked for.
                let rd = unsafe {
                    job_read_file(self.read, tmp.as_mut_ptr(), avail.min(1000), &mut got, core::ptr::null_mut())
                };
                if rd != 0 {
                    let _ = out.write_str(core::str::from_utf8(&tmp[..got as usize]).unwrap_or("?"));
                }
                break;
            }
            if waited >= wait_ms {
                break;
            }
            // SAFETY: plain sleep.
            unsafe { Sleep(25) };
            waited += 25;
        }
        out
    }
}

/// Prints each non-empty line of `text` as "label: line" (the `in_job=` line depends on the runner and is left out).
fn job_lines(t: &mut Test, label: &str, text: &str) {
    for line in text.split('\n') {
        if !line.is_empty() && !line.starts_with("in_job=") {
            say!(t, "{label}: {line}");
        }
    }
}

/// The facts of a token as text lines (one `key=value` pair group per line).
fn job_token_report(token: HANDLE) -> Buf<1200> {
    let mut b = Buf::<1200>::new();
    let mut p = Raw::<4096>::new();
    let (ok, _, _) = token_info(token, TOKEN_PRIVILEGES_CLASS, &mut p);
    let mut count = 0u32;
    let mut enabled = 0u32;
    if ok {
        count = p.u32_at(0);
        for i in 0..(count as usize).min(60) {
            if p.u32_at(4 + 12 * i + 8) & SE_PRIVILEGE_ENABLED != 0 {
                enabled += 1;
            }
        }
    }
    let mut rs = Raw::<4096>::new();
    let (ok2, _, _) = token_info(token, TOKEN_RESTRICTED_SIDS_CLASS, &mut rs);
    let restricting = if ok2 { i64::from(rs.u32_at(0)) } else { -1 };
    let mut u = Raw::<2048>::new();
    let (ok3, _, _) = token_info(token, TOKEN_USER_CLASS, &mut u);
    let user = if ok3 { mask_sid(first_sid(&u)) } else { Buf::<200>::new() };
    // SAFETY: the handle is a token handle or the call fails.
    let restricted = unsafe { IsTokenRestricted(token) };
    let _ = writeln!(
        b,
        "type={} integrity={} is_restricted={} has_restrictions={} sandbox_inert={}",
        tok_u32(token, TOKEN_TYPE_CLASS),
        job_hex(integrity_rid(token)).as_str(),
        restricted,
        tok_u32(token, TOKEN_HAS_RESTRICTIONS_CLASS),
        tok_u32(token, TOKEN_SANDBOX_INERT_CLASS)
    );
    let _ = writeln!(b, "privileges={count} enabled={enabled} restricting_sids={restricting} user={}", user.as_str());
    let _ = writeln!(
        b,
        "admins={} users={} everyone={} elevation_type={}",
        job_hex(group_attrs(token, TOKEN_GROUPS_CLASS, "S-1-5-32-544")).as_str(),
        job_hex(group_attrs(token, TOKEN_GROUPS_CLASS, "S-1-5-32-545")).as_str(),
        job_hex(group_attrs(token, TOKEN_GROUPS_CLASS, "S-1-1-0")).as_str(),
        tok_u32(token, TOKEN_ELEVATION_TYPE_CLASS)
    );
    b
}

/// Tries to create (and delete again on close) a file beside the executable: the NTSTATUS.
fn job_try_create_file() -> i32 {
    let Some(path) = path_beside_image(utf16!("poly-job-child.tmp")) else {
        return -1;
    };
    let name = path.unicode_string();
    let attributes = ObjectAttributes::named(&name);
    let mut handle: HANDLE = core::ptr::null_mut();
    let mut iosb = IoStatusBlock::new();
    // SAFETY: every pointer is to a live local; no EA buffer.
    let st = unsafe {
        NtCreateFile(
            &mut handle,
            GENERIC_WRITE | DELETE | SYNCHRONIZE,
            &attributes,
            &mut iosb,
            core::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            0,
            FILE_CREATE,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE | FILE_DELETE_ON_CLOSE,
            core::ptr::null(),
            0,
        )
    };
    if st >= 0 {
        // SAFETY: closing the handle (the file is deleted on close).
        unsafe { NtClose(handle) };
    }
    st
}

/// The text a `--child report` child writes: its token facts, job membership and a file creation attempt.
fn job_child_report() -> Buf<1200> {
    let tok = process_token(TOKEN_QUERY);
    let mut b = job_token_report(tok);
    let mut r: BOOL = 0x55;
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    let ok = unsafe { IsProcessInJob(GetCurrentProcess(), core::ptr::null_mut(), &mut r) };
    let injob = if ok == 0 { -1 } else { i64::from(r != 0) };
    let _ = writeln!(b, "in_job={injob}");
    let _ = writeln!(b, "create_file={:#x}", job_try_create_file());
    b
}

/// Writes text to this process's standard output handle.
fn job_child_out(text: &str) {
    // SAFETY: the standard output handle is checked before use; the buffer is the text.
    unsafe {
        let h = job_get_std_handle(STD_OUTPUT_HANDLE);
        if h.is_null() || h == INVALID_HANDLE {
            return;
        }
        let mut w = 0u32;
        job_write_file(h, text.as_ptr(), text.len() as u32, &mut w, core::ptr::null_mut());
    }
}

/// A child that does nothing but wait; it ends itself after a minute.
fn job_child_wait() -> ! {
    for _ in 0..240 {
        // SAFETY: plain sleep.
        unsafe { Sleep(250) };
    }
    exit(77)
}

/// The rest of the command line after `--child ` (ASCII), if this process is a child of a test.
fn job_child_mode() -> Option<([u8; 48], usize)> {
    let cl = command_line();
    let key = b"--child ";
    if cl.len() < key.len() {
        return None;
    }
    let mut start = None;
    for i in 0..=(cl.len() - key.len()) {
        if cl[i..i + key.len()].iter().zip(key.iter()).all(|(&c, &k)| c == u16::from(k)) {
            start = Some(i + key.len());
            break;
        }
    }
    let s = start?;
    let mut out = [0u8; 48];
    let mut n = 0;
    for &c in &cl[s..] {
        if n < 48 && c < 0x80 {
            out[n] = c as u8;
            n += 1;
        }
    }
    Some((out, n))
}

/// Runs the child behaviour when the command line asks for one (never returns then):
/// `wait`, `exit0`, `exit7`, `report` (token facts to standard output), `spawn1`/`spawn2` (start a grandchild that
/// exits with 7, plain/with CREATE_BREAKAWAY_FROM_JOB, report through the exit code), `spawn3`/`spawn4` (start a
/// grandchild that waits, write its id to standard output, wait for it), `probe <hex>` (is that handle value valid here).
fn job_child_dispatch() {
    let Some((mode, n)) = job_child_mode() else {
        return;
    };
    let m = &mode[..n];
    if m == b"wait" {
        job_child_wait();
    }
    if m == b"exit0" {
        exit(0);
    }
    if m == b"exit7" {
        exit(7);
    }
    if m == b"report" {
        job_child_out(job_child_report().as_str());
        exit(0);
    }
    if m == b"spawn1" || m == b"spawn2" {
        let flags = if m == b"spawn2" { CREATE_BREAKAWAY_FROM_JOB } else { 0 };
        let gc = job_start("exit7", flags, core::ptr::null_mut(), core::ptr::null_mut());
        if !gc.ok {
            exit(0x1000_0000 | (gc.err as u32 & 0xFFFF));
        }
        let code = wait_exit(gc.process);
        exit(0x2000_0000 | if code < 0 { 0xFFFF } else { (code as u32) & 0xFF_FFFF });
    }
    if m == b"spawn3" || m == b"spawn4" {
        let flags = if m == b"spawn4" { CREATE_BREAKAWAY_FROM_JOB } else { 0 };
        let gc = job_start("wait", flags, core::ptr::null_mut(), core::ptr::null_mut());
        let mut b = Buf::<64>::new();
        if gc.ok {
            let _ = writeln!(b, "gc=ok pid={}", gc.pid);
        } else {
            let _ = writeln!(b, "gc=err {}", gc.err);
        }
        job_child_out(b.as_str());
        if gc.ok {
            // SAFETY: the grandchild's process handle.
            unsafe { WaitForSingleObject(gc.process, 20_000) };
            gc.kill(98);
        }
        exit(0);
    }
    if m.starts_with(b"probe ") {
        let mut v = 0usize;
        for &c in &m[6..] {
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => 0,
            };
            v = v * 16 + usize::from(d);
        }
        let mut f = 0u32;
        // SAFETY: valid out pointer; an invalid handle value just fails.
        let ok = unsafe { GetHandleInformation(v as HANDLE, &mut f) };
        exit(if ok != 0 { 0 } else { 0x100 | (gle() & 0xFF) });
    }
    exit(0x7777);
}

/// The program's entry point for tests that start copies of themselves.
macro_rules! job_main {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            job_child_dispatch();
            if open_output().is_err() {
                exit(EXIT_NO_OUTPUT);
            }
            let mut t = Test::new();
            $test(&mut t);
            let code = t.finish();
            exit(code)
        }
    };
}

/// winnt.h: `TOKEN_MANDATORY_LABEL`.
#[repr(C)]
struct JobLabel {
    sid: PSID,
    attrs: u32,
}

/// Sets the integrity level of `token` to the RID (0 untrusted, 0x1000 low, 0x2000 medium): "ok" or "fail(code)".
fn job_set_integrity(token: HANDLE, rid: u32) -> Buf<24> {
    let sid = sid_of(match rid {
        0 => "S-1-16-0",
        0x1000 => "S-1-16-4096",
        0x2000 => "S-1-16-8192",
        _ => "S-1-16-12288",
    });
    let l = JobLabel { sid: sid.as_const() as PSID, attrs: SE_GROUP_INTEGRITY };
    clear_error();
    // SAFETY: a valid TOKEN_MANDATORY_LABEL followed by the SID's length.
    let ok = unsafe {
        SetTokenInformation(
            token,
            TOKEN_INTEGRITY_LEVEL_CLASS,
            (&l as *const JobLabel).cast(),
            core::mem::size_of::<JobLabel>() as u32 + sid.bytes().len() as u32,
        )
    };
    job_r(ok)
}

/// A primary-token duplicate of the process token with all access (null on failure).
fn job_primary() -> HANDLE {
    dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY)
}

/// Closes a handle if it is not null.
fn job_close(h: HANDLE) {
    if !h.is_null() {
        // SAFETY: closing a handle the caller owns.
        unsafe { CloseHandle(h) };
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn TerminateProcess(process: HANDLE, code: u32) -> BOOL;
    fn ResumeThread(thread: HANDLE) -> u32;
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
    fn IsProcessInJob(process: HANDLE, job: HANDLE, result: *mut BOOL) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn SetThreadToken(thread: *const HANDLE, token: HANDLE) -> BOOL;
    fn Sleep(ms: u32);
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
    #[link_name = "CreatePipe"]
    fn job_create_pipe(read: *mut HANDLE, write: *mut HANDLE, attrs: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    #[link_name = "ReadFile"]
    fn job_read_file(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, overlapped: PVOID) -> BOOL;
    #[link_name = "WriteFile"]
    fn job_write_file(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, overlapped: PVOID) -> BOOL;
    #[link_name = "PeekNamedPipe"]
    fn job_peek_pipe(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, avail: *mut u32, left: *mut u32) -> BOOL;
    #[link_name = "GetStdHandle"]
    fn job_get_std_handle(which: u32) -> HANDLE;
    #[link_name = "SetHandleInformation"]
    fn job_set_handle_info(h: HANDLE, mask: u32, flags: u32) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn SetTokenInformation(token: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
    fn CreateRestrictedToken(
        token: HANDLE,
        flags: u32,
        disable_count: u32,
        disable: *const SID_AND_ATTRIBUTES,
        delete_count: u32,
        delete: *const LUID_AND_ATTRIBUTES,
        restrict_count: u32,
        restrict: *const SID_AND_ATTRIBUTES,
        out: *mut HANDLE,
    ) -> BOOL;
    fn IsTokenRestricted(token: HANDLE) -> BOOL;
    fn CreateProcessAsUserW(
        token: HANDLE,
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

job_main!(test);

fn run(t: &mut Test, label: &str, token: HANDLE, full: bool) {
    let mut pipe = JobPipe::new();
    let c = job_start("report", 0, token, pipe.write);
    pipe.close_write();
    if !c.ok {
        say!(t, "{label}: CreateProcessAsUserW failed, error {}", c.err);
        return;
    }
    let code = c.code();
    let text = pipe.text(3000);
    say!(t, "{label}: started, child exit code {}", job_hex(code).as_str());
    if full {
        job_lines(t, "  child", text.as_str());
    } else {
        for line in text.as_str().lines().take(2) {
            say!(t, "  child: {line}");
        }
    }
}

/// CreateProcessAsUserW with an arbitrary token handle on a child that exits with 7.
fn raw(t: &mut Test, label: &str, token: HANDLE) {
    let mut cmd = self_command("--child exit7");
    let mut si = STARTUPINFOW::new();
    let mut pi = PROCESS_INFORMATION::new();
    clear_error();
    // SAFETY: valid command line and structures; the token handle is checked by the call.
    let ok = unsafe {
        CreateProcessAsUserW(
            token,
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            FALSE,
            DETACHED_PROCESS,
            core::ptr::null_mut(),
            core::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    if ok == 0 {
        say!(t, "{label}: failed, error {}", gle());
        return;
    }
    let code = wait_exit(pi.process);
    say!(t, "{label}: started, child exit code {}", job_hex(code).as_str());
    job_close(pi.process);
    job_close(pi.thread);
}

fn dup_as(access: u32, level: u32, kind: u32) -> HANDLE {
    dup_token(access, level, kind)
}

fn test(t: &mut Test) {
    let parent = process_token(TOKEN_QUERY);
    say!(t, "ambient: this process's token facts:");
    job_lines(t, "  parent", job_token_report(parent).as_str());
    job_close(parent);

    let plain = job_primary();
    run(t, "primary duplicate, TOKEN_ALL_ACCESS", plain, true);
    job_close(plain);

    let three = dup_as(TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    run(t, "primary duplicate with exactly QUERY|DUPLICATE|ASSIGN_PRIMARY", three, false);
    job_close(three);
    for (label, access) in [
        ("without ASSIGN_PRIMARY (QUERY|DUPLICATE)", TOKEN_QUERY | TOKEN_DUPLICATE),
        ("without DUPLICATE (QUERY|ASSIGN_PRIMARY)", TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY),
        ("without QUERY (DUPLICATE|ASSIGN_PRIMARY)", TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY),
        ("only ASSIGN_PRIMARY", TOKEN_ASSIGN_PRIMARY),
        ("only TOKEN_QUERY", TOKEN_QUERY),
        ("TOKEN_READ | TOKEN_EXECUTE", TOKEN_READ | TOKEN_EXECUTE),
    ] {
        let tok = dup_as(access, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        let mut b = Buf::<120>::new();
        let _ = write!(b, "primary duplicate {label}");
        run(t, b.as_str(), tok, false);
        job_close(tok);
    }

    // The process token itself.
    let own = process_token(TOKEN_ALL_ACCESS);
    run(t, "the process token itself (TOKEN_ALL_ACCESS)", own, false);
    job_close(own);
    let own = process_token(TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY);
    run(t, "the process token opened with QUERY|DUPLICATE|ASSIGN_PRIMARY", own, false);
    job_close(own);

    // A restricted token (privileges removed).
    let src = process_token(TOKEN_ALL_ACCESS);
    let mut res: HANDLE = core::ptr::null_mut();
    // SAFETY: valid source token and out pointer.
    let ok = unsafe {
        CreateRestrictedToken(
            src,
            DISABLE_MAX_PRIVILEGE,
            0,
            core::ptr::null(),
            0,
            core::ptr::null(),
            0,
            core::ptr::null(),
            &mut res,
        )
    };
    say!(t, "CreateRestrictedToken(DISABLE_MAX_PRIVILEGE): ok={ok}");
    run(t, "restricted token (DISABLE_MAX_PRIVILEGE)", res, true);
    job_close(res);
    job_close(src);

    // Impersonation tokens.
    for (label, level) in [
        ("SecurityImpersonation", SECURITY_IMPERSONATION),
        ("SecurityIdentification", SECURITY_IDENTIFICATION),
        ("SecurityAnonymous", SECURITY_ANONYMOUS),
        ("SecurityDelegation", SECURITY_DELEGATION),
    ] {
        let imp = dup_as(TOKEN_ALL_ACCESS, level, TOKEN_IMPERSONATION);
        let mut b = Buf::<120>::new();
        let _ = write!(b, "impersonation token, {label}");
        run(t, b.as_str(), imp, false);
        job_close(imp);
    }
    // A primary token made from an impersonation token.
    let imp = dup_as(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    let mut prim: HANDLE = core::ptr::null_mut();
    // SAFETY: valid source token and out pointer.
    let ok = unsafe {
        DuplicateTokenEx(imp, TOKEN_ALL_ACCESS, core::ptr::null(), SECURITY_IMPERSONATION, TOKEN_PRIMARY, &mut prim)
    };
    say!(t, "DuplicateTokenEx(impersonation -> primary): ok={ok}");
    run(t, "primary token made from an impersonation token", prim, false);
    job_close(prim);
    job_close(imp);

    // Handles that are not tokens.
    // SAFETY: a plain event.
    let ev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null()) };
    raw(t, "an event as the token", ev);
    raw(t, "an invalid handle value as the token", 0x1234usize as HANDLE);
    raw(t, "INVALID_HANDLE_VALUE as the token", INVALID_HANDLE);
    raw(t, "a null token", core::ptr::null_mut());
    job_close(ev);
    let tok = job_primary();
    job_close(tok);
    raw(t, "a closed token handle", tok);

    // A thread with an impersonation token: the child still gets the process token (CreateProcessW) and the
    // token passed to CreateProcessAsUserW.
    let low = dup_as(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    say!(t, "lower the impersonation token to Low: {}", job_set_integrity(low, SECURITY_MANDATORY_LOW_RID).as_str());
    clear_error();
    // SAFETY: valid token handle; null thread pointer means the calling thread.
    let ok = unsafe { SetThreadToken(core::ptr::null(), low) };
    say!(t, "thread token set: {}", job_r(ok).as_str());
    let mut pipe = JobPipe::new();
    let c = job_start("report", 0, core::ptr::null_mut(), pipe.write);
    pipe.close_write();
    c.code();
    let text = pipe.text(3000);
    say!(
        t,
        "CreateProcessW from a thread with a Low impersonation token: child {}",
        text.as_str().lines().next().unwrap_or("")
    );
    let plain = job_primary();
    let mut pipe = JobPipe::new();
    let c = job_start("report", 0, plain, pipe.write);
    pipe.close_write();
    c.code();
    let text = pipe.text(3000);
    say!(
        t,
        "CreateProcessAsUserW(primary duplicate) from that thread: child {}",
        text.as_str().lines().next().unwrap_or("")
    );
    // SAFETY: removes the thread token again.
    unsafe { SetThreadToken(core::ptr::null(), core::ptr::null_mut()) };
    job_close(plain);
    job_close(low);
}
