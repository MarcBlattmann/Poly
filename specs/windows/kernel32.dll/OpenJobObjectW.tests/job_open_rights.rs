//! OpenJobObjectW: the access mask of the handle for each requested right (specific, standard, generic, maximum
//! allowed, undefined bits), the inherit flag, names that do not exist, empty names, and names of other objects.
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

const TOKEN_QUERY: u32 = 0x0008;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_GROUPS_CLASS: u32 = 2;
const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_RESTRICTED_SIDS_CLASS: u32 = 11;
const TOKEN_SANDBOX_INERT_CLASS: u32 = 15;
const TOKEN_ELEVATION_TYPE_CLASS: u32 = 18;
const TOKEN_HAS_RESTRICTIONS_CLASS: u32 = 21;
const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
const GENERIC_WRITE_RIGHT: u32 = 0x4000_0000;
const GENERIC_EXECUTE_RIGHT: u32 = 0x2000_0000;
const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x1;
const JOB_OBJECT_SET_ATTRIBUTES: u32 = 0x2;
const JOB_OBJECT_QUERY: u32 = 0x4;
const JOB_OBJECT_TERMINATE: u32 = 0x8;
const JOB_OBJECT_SET_SECURITY_ATTRIBUTES: u32 = 0x10;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const DETACHED_PROCESS: u32 = 0x8;
const STARTF_USESTDHANDLES: u32 = 0x100;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const HANDLE_FLAG_INHERIT: u32 = 1;
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

/// A unique object name for this process: `PolySecJob<tag>_<pid>`.
fn job_name(tag: &str) -> [u16; 256] {
    let mut s = Buf::<120>::new();
    // SAFETY: reads the process id.
    let pid = unsafe { GetCurrentProcessId() };
    let _ = write!(s, "PolySecJob{tag}_{pid:x}");
    wide(s.as_str())
}

/// Whether the handle is inheritable (1/0), or -1 when the query fails.
fn job_inheritable(h: HANDLE) -> i64 {
    let mut f = 0u32;
    // SAFETY: valid out pointer.
    let ok = unsafe { GetHandleInformation(h, &mut f) };
    if ok == 0 { -1 } else { i64::from(f & HANDLE_FLAG_INHERIT) }
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
    fn GetCurrentProcessId() -> u32;
    fn CreateMutexW(attrs: *const SECURITY_ATTRIBUTES, owner: BOOL, name: *const u16) -> HANDLE;
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
    fn CreateJobObjectW(attrs: *const SECURITY_ATTRIBUTES, name: *const u16) -> HANDLE;
    fn OpenJobObjectW(access: u32, inherit: BOOL, name: *const u16) -> HANDLE;
    fn IsProcessInJob(process: HANDLE, job: HANDLE, result: *mut BOOL) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn Sleep(ms: u32);
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
    #[link_name = "WriteFile"]
    fn job_write_file(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, overlapped: PVOID) -> BOOL;
    #[link_name = "GetStdHandle"]
    fn job_get_std_handle(which: u32) -> HANDLE;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
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

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

job_main!(test);

fn test(t: &mut Test) {
    let name = job_name("Open");
    // SAFETY: NUL-terminated name.
    let job = unsafe { CreateJobObjectW(core::ptr::null(), name.as_ptr()) };
    say!(t, "creator: valid={} access={:#x}", !job.is_null(), granted_access(job));
    for (label, access) in [
        ("no rights", 0u32),
        ("ASSIGN_PROCESS", JOB_OBJECT_ASSIGN_PROCESS),
        ("SET_ATTRIBUTES", JOB_OBJECT_SET_ATTRIBUTES),
        ("QUERY", JOB_OBJECT_QUERY),
        ("TERMINATE", JOB_OBJECT_TERMINATE),
        ("SET_SECURITY_ATTRIBUTES", JOB_OBJECT_SET_SECURITY_ATTRIBUTES),
        ("0x1f (all specific)", 0x1F),
        ("JOB_OBJECT_ALL_ACCESS", JOB_OBJECT_ALL_ACCESS),
        ("SYNCHRONIZE", SYNCHRONIZE),
        ("READ_CONTROL", READ_CONTROL),
        ("DELETE", DELETE),
        ("WRITE_DAC", WRITE_DAC),
        ("WRITE_OWNER", WRITE_OWNER),
        ("MAXIMUM_ALLOWED", MAXIMUM_ALLOWED),
        ("GENERIC_ALL", GENERIC_ALL_RIGHTS),
        ("GENERIC_READ", GENERIC_READ_RIGHT),
        ("GENERIC_WRITE", GENERIC_WRITE_RIGHT),
        ("GENERIC_EXECUTE", GENERIC_EXECUTE_RIGHT),
        ("undefined specific bit 0x20", 0x20),
        ("undefined specific bit 0x8000", 0x8000),
        ("ACCESS_SYSTEM_SECURITY", ACCESS_SYSTEM_SECURITY),
    ] {
        clear_error();
        // SAFETY: NUL-terminated name.
        let h = unsafe { OpenJobObjectW(access, FALSE, name.as_ptr()) };
        if h.is_null() {
            say!(t, "open with {label} ({access:#x}): fail({})", gle());
        } else {
            say!(
                t,
                "open with {label} ({access:#x}): access={:#x} inheritable={}",
                granted_access(h),
                job_inheritable(h)
            );
            job_close(h);
        }
    }
    // The inherit flag.
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, TRUE, name.as_ptr()) };
    say!(t, "inherit=TRUE: valid={} inheritable={}", !h.is_null(), job_inheritable(h));
    job_close(h);
    // Other spellings of the name.
    let mut s = Buf::<200>::new();
    let _ = write!(s, "Local\\{}", wide_text::<120>(name.as_ptr()).as_str());
    let local = wide(s.as_str());
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, local.as_ptr()) };
    say!(t, "Local\\ spelling: valid={}", !h.is_null());
    job_close(h);
    let mut up = name;
    for c in up.iter_mut() {
        if *c >= u16::from(b'a') && *c <= u16::from(b'z') {
            *c -= 32;
        }
    }
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, up.as_ptr()) };
    say!(t, "upper-case spelling: valid={}", !h.is_null());
    job_close(h);

    // Names that do not give a job.
    let missing = job_name("Missing");
    clear_error();
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, missing.as_ptr()) };
    say!(t, "name that does not exist: valid={} err={}", !h.is_null(), if h.is_null() { i64::from(gle()) } else { -1 });
    let empty = [0u16; 2];
    clear_error();
    // SAFETY: NUL-terminated empty name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, empty.as_ptr()) };
    say!(t, "empty name: valid={} err={}", !h.is_null(), if h.is_null() { i64::from(gle()) } else { -1 });
    let mut s = Buf::<200>::new();
    let _ = write!(s, "NoSuchDir\\{}", wide_text::<120>(name.as_ptr()).as_str());
    let bad = wide(s.as_str());
    clear_error();
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, bad.as_ptr()) };
    say!(
        t,
        "name in a directory that does not exist: valid={} err={}",
        !h.is_null(),
        if h.is_null() { i64::from(gle()) } else { -1 }
    );
    // A mutex with the same kind of name.
    let mname = job_name("Mutex");
    // SAFETY: NUL-terminated name.
    let mx = unsafe { CreateMutexW(core::ptr::null(), FALSE, mname.as_ptr()) };
    clear_error();
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, mname.as_ptr()) };
    say!(
        t,
        "name of a mutex: mutex valid={} job valid={} err={}",
        !mx.is_null(),
        !h.is_null(),
        if h.is_null() { i64::from(gle()) } else { -1 }
    );
    job_close(mx);
    // After the last handle closes the name is gone.
    job_close(job);
    clear_error();
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, name.as_ptr()) };
    say!(
        t,
        "after the creator closed its handle: valid={} err={}",
        !h.is_null(),
        if h.is_null() { i64::from(gle()) } else { -1 }
    );
}
