//! GetAppContainerNamedObjectPath(token, sid, length, buffer, &needed): the object directory of a container
//! (session 0 of the oracle: `AppContainerNamedObjects\<sid>` without a `\Sessions\n\` prefix), with a SID or with an
//! AppContainer token, the sizes, and the errors for a caller that is no AppContainer. Exported by kernel32.dll and
//! kernelbase.dll.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 2 of the observations, the others write the rest.)
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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

/// Where observation lines go: the test's output file, or (in a child process) a pipe to the parent.
trait Sink {
    fn put(&mut self, s: &str);
}

impl Sink for Test {
    fn put(&mut self, s: &str) {
        if say_keep() {
            self.line(s);
        }
    }
}

/// Which observations of this run are written: 0 all of them, 1/2/3 every third one starting at the first/second/third
/// (a test that is kept in two files writes the two halves of its observations; every call is made in both).
static SAY_PART: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SAY_N: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn say_keep() -> bool {
    let p = SAY_PART.load(core::sync::atomic::Ordering::Relaxed);
    if p == 0 {
        return true;
    }
    let n = SAY_N.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    n % 3 + 1 == p
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<600>::new();
        let _ = write!(b, $($a)*);
        $t.put(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; characters outside the BMP are not used).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<600> {
    let mut b = Buf::<600>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    let low = |c: u16| if (0x41..=0x5a).contains(&c) { c + 0x20 } else { c };
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| low(x) == low(y))
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const INFINITE: u32 = 0xFFFF_FFFF;
/// A value no call sets as last error: "unchanged" when it is still there afterwards.
const MARK: u32 = 0xDEAD;

/// winbase.h `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: BOOL,
}

fn inheritable() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES { length: 24, security_descriptor: core::ptr::null_mut(), inherit_handle: 1 }
}

/// processthreadsapi.h `STARTUPINFOW` (104 bytes on x64).
#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    pos: [u32; 7],
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std: [HANDLE; 3],
}

/// processthreadsapi.h `PROCESS_INFORMATION`.
#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn Sleep(ms: u32);
    fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, put: *mut u32, ov: *mut c_void) -> BOOL;
    fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const SECURITY_ATTRIBUTES,
        ta: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn GetCommandLineW() -> *const u16;
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, n: u32) -> u32;
}

fn close(h: HANDLE) {
    if !h.is_null() && h != INVALID_HANDLE_VALUE {
        // SAFETY: a handle this test opened, closed once.
        unsafe { CloseHandle(h) };
    }
}

fn last_error() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetLastError() }
}

fn mark_error() {
    // SAFETY: sets this thread's last error.
    unsafe { SetLastError(MARK) };
}

/// The last error as text: `unchanged` when it still is the mark set before the call.
fn err_text() -> Buf<24> {
    let e = last_error();
    let mut b = Buf::<24>::new();
    if e == MARK {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{e}");
    }
    b
}

/// 1 for a usable handle, else 0.
fn opened(h: HANDLE) -> u32 {
    u32::from(!h.is_null() && h != INVALID_HANDLE_VALUE)
}

// ----------------------------------------------------- child processes

/// The ASCII text after ` --child ` on this process's command line.
fn child_args() -> Option<Buf<160>> {
    // SAFETY: GetCommandLineW returns a NUL-terminated string that lives as long as the process.
    let p = unsafe { GetCommandLineW() };
    let units = wstr_at(p, 700);
    let n = units.len();
    let key = b" --child ";
    let at = (0..n.saturating_sub(key.len())).find(|&i| (0..key.len()).all(|j| units[i + j] == u16::from(key[j])))?;
    let mut out = Buf::<160>::new();
    for &u in &units[at + key.len()..n] {
        let _ = write!(out, "{}", u as u8 as char);
    }
    Some(out)
}

/// The `i`th space-separated word of `args` as a hexadecimal handle value.
fn arg_handle(args: &str, i: usize) -> HANDLE {
    let v = args.split(' ').nth(i).and_then(|w| usize::from_str_radix(w, 16).ok()).unwrap_or(0);
    v as HANDLE
}

/// A child process: this test's own image started with `--child <args>`.
struct Child {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

/// Start this test's own image with `--child <args>` (handles inherited, creation `flags`).
fn start_child(args: &str, flags: u32) -> Option<Child> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut cmd = [0u16; 800];
    let mut m = 0;
    let mut push = |u: u16| {
        if m < 799 {
            cmd[m] = u;
            m += 1;
        }
    };
    push(u16::from(b'"'));
    for &u in &exe[..n] {
        push(u);
    }
    push(u16::from(b'"'));
    for b in " --child ".bytes().chain(args.bytes()) {
        push(u16::from(b));
    }
    let si = STARTUPINFOW {
        cb: 104,
        reserved: core::ptr::null_mut(),
        desktop: core::ptr::null_mut(),
        title: core::ptr::null_mut(),
        pos: [0; 7],
        flags: 0,
        show_window: 0,
        cb_reserved2: 0,
        reserved2: core::ptr::null_mut(),
        std: [core::ptr::null_mut(); 3],
    };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: `cmd` is mutable and NUL-terminated; the structures are live locals.
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            flags,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(Child { process: pi.process, thread: pi.thread, pid: pi.pid, tid: pi.tid })
}

impl Child {
    /// Wait for the child (10 s at most, then it is ended with 0xDEAD); close its handles; its exit code.
    fn finish(self) -> u32 {
        let mut code = 0u32;
        // SAFETY: the handles of the process this test started, closed once.
        unsafe {
            if WaitForSingleObject(self.process, 10_000) != 0 {
                TerminateProcess(self.process, 0xDEAD);
                WaitForSingleObject(self.process, 5_000);
            }
            GetExitCodeProcess(self.process, &mut code);
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
        code
    }
    /// End the child now and close its handles.
    fn kill(self) {
        // SAFETY: the handles of the process this test started, closed once.
        unsafe {
            TerminateProcess(self.process, 0xDEAD);
            WaitForSingleObject(self.process, 5_000);
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

/// Start this test's own image with `--child <args>` and wait for it; its exit code.
fn run_child(args: &str) -> Option<u32> {
    start_child(args, 0).map(Child::finish)
}

/// The child's report: lines written to an inherited anonymous pipe. The parent side.
struct Report {
    read: HANDLE,
    write: HANDLE,
}

impl Report {
    fn new() -> Report {
        let mut r = Report { read: core::ptr::null_mut(), write: core::ptr::null_mut() };
        let sa = inheritable();
        // SAFETY: live out pointers and attributes.
        unsafe { CreatePipe(&mut r.read, &mut r.write, &sa, 8192) };
        r
    }
    /// Close the write end and print what the child wrote, each line with `child: ` before it.
    fn print(self, t: &mut Test) {
        close(self.write);
        let mut all = [0u8; 8192];
        let mut n = 0usize;
        loop {
            let mut got = 0u32;
            // SAFETY: reads into the free part of `all`.
            let ok = unsafe {
                ReadFile(self.read, all.as_mut_ptr().add(n), (all.len() - n) as u32, &mut got, core::ptr::null_mut())
            };
            if ok == 0 || got == 0 {
                break;
            }
            n += got as usize;
            if n == all.len() {
                break;
            }
        }
        close(self.read);
        for line in core::str::from_utf8(&all[..n]).unwrap_or("?").split('\n') {
            if !line.is_empty() {
                say!(t, "child: {line}");
            }
        }
    }
}

/// The child side of a report: lines go into the inherited pipe.
struct ChildOut {
    h: HANDLE,
}

impl Sink for ChildOut {
    fn put(&mut self, s: &str) {
        let mut put = 0u32;
        // SAFETY: writes the bytes of `s` and a newline to the inherited pipe handle.
        unsafe {
            WriteFile(self.h, s.as_ptr(), s.len() as u32, &mut put, core::ptr::null_mut());
            WriteFile(self.h, b"\n".as_ptr(), 1, &mut put, core::ptr::null_mut());
        }
    }
}

/// The entry point of a test that starts its own image again: a process started with
/// `--child <args>` runs `$child(args) -> u32` (its exit code) and never opens the observation
/// file (the parent holds it); the other one runs `$test` like `poly_wtest::main!`.
macro_rules! main_with_child {
    ($child:path, $test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_args() {
                exit($child(a.as_str()))
            }
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

// securitybaseapi.h, processthreadsapi.h, winnt.h, sddl.h: tokens, restricted tokens, CreateProcessAsUserW.
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn OpenThreadToken(thread: HANDLE, access: u32, as_self: BOOL, token: *mut HANDLE) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        sa: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        new: *mut HANDLE,
    ) -> BOOL;
    fn CreateRestrictedToken(
        existing: HANDLE,
        flags: u32,
        n_disable: u32,
        disable: *const SID_AND_ATTRIBUTES,
        n_delete: u32,
        delete: *const c_void,
        n_restrict: u32,
        restrict: *const SID_AND_ATTRIBUTES,
        new: *mut HANDLE,
    ) -> BOOL;
    fn SetTokenInformation(token: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: *mut c_void, len: u32, ret_len: *mut u32) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut *mut c_void) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, s: *mut *mut u16) -> BOOL;
    fn CreateProcessAsUserW(
        token: HANDLE,
        app: *const u16,
        cmd: *mut u16,
        pa: *const SECURITY_ATTRIBUTES,
        ta: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn SetThreadToken(thread: *const HANDLE, token: HANDLE) -> BOOL;
    fn RevertToSelf() -> BOOL;
    fn ImpersonateLoggedOnUser(token: HANDLE) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn ResumeThread(h: HANDLE) -> u32;
}

/// winnt.h `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: *mut c_void,
    attributes: u32,
}

/// winnt.h `TOKEN_MANDATORY_LABEL`.
#[repr(C)]
struct TOKEN_MANDATORY_LABEL {
    label: SID_AND_ATTRIBUTES,
}

const TOKEN_ASSIGN_PRIMARY: u32 = 0x1;
const TOKEN_DUPLICATE: u32 = 0x2;
const TOKEN_IMPERSONATE: u32 = 0x4;
const TOKEN_QUERY: u32 = 0x8;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x20;
const TOKEN_ADJUST_DEFAULT: u32 = 0x80;
const TOKEN_ALL_ACCESS: u32 = 0xF01FF;
const TokenPrimary: u32 = 1;
const TokenImpersonation: u32 = 2;
const SecurityAnonymous: u32 = 0;
const SecurityIdentification: u32 = 1;
const SecurityImpersonation: u32 = 2;
const TokenIntegrityLevel: u32 = 25;
const TokenDefaultDacl: u32 = 6;
const TokenType: u32 = 8;
const TokenGroups: u32 = 2;
const TokenPrivileges: u32 = 3;
const TokenRestrictedSids: u32 = 11;
const TokenHasRestrictions: u32 = 21;
const TokenIsRestricted: u32 = 40;
const DISABLE_MAX_PRIVILEGE: u32 = 1;
const SE_GROUP_INTEGRITY: u32 = 0x20;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_SUSPENDED: u32 = 0x4;
const DETACHED_PROCESS: u32 = 0x8;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;

/// A SID from its string form (leaked: the tests are short-lived).
fn sid(s: &str) -> *mut c_void {
    let w = wide(s);
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: `w` is NUL-terminated, `p` is a live out pointer.
    unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut p) };
    p
}

/// A primary token: a duplicate of this process's token.
fn primary_copy() -> HANDLE {
    let mut tok: HANDLE = core::ptr::null_mut();
    let mut dup: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointers; the pseudo handle is valid.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok);
        DuplicateTokenEx(tok, TOKEN_ALL_ACCESS, core::ptr::null(), SecurityImpersonation, TokenPrimary, &mut dup);
    }
    close(tok);
    dup
}

/// An impersonation token (level `level`): a duplicate of this process's token.
fn impersonation_copy(level: u32) -> HANDLE {
    let mut tok: HANDLE = core::ptr::null_mut();
    let mut dup: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointers; the pseudo handle is valid.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut tok);
        DuplicateTokenEx(tok, TOKEN_ALL_ACCESS, core::ptr::null(), level, TokenImpersonation, &mut dup);
    }
    close(tok);
    dup
}

/// Lower the integrity level of `token` to the one named by the SID string (`S-1-16-4096` = Low).
fn set_integrity(token: HANDLE, sid_text: &str) -> BOOL {
    let l = TOKEN_MANDATORY_LABEL { label: SID_AND_ATTRIBUTES { sid: sid(sid_text), attributes: SE_GROUP_INTEGRITY } };
    // SAFETY: `l` is a live TOKEN_MANDATORY_LABEL of 16 bytes.
    unsafe { SetTokenInformation(token, TokenIntegrityLevel, (&l as *const TOKEN_MANDATORY_LABEL).cast(), 16) }
}

/// The token's integrity level (the last sub-authority of the label SID), or 0xFFFF on failure.
fn integrity_of(token: HANDLE) -> u32 {
    let mut buf = [0u8; 64];
    let mut n = 0u32;
    // SAFETY: the buffer holds 64 bytes.
    let ok = unsafe { GetTokenInformation(token, TokenIntegrityLevel, buf.as_mut_ptr().cast(), 64, &mut n) };
    if ok == 0 {
        return 0xFFFF;
    }
    // SAFETY: the buffer starts with a TOKEN_MANDATORY_LABEL whose SID pointer points into the buffer.
    let p = unsafe { *(buf.as_ptr() as *const *const u8) };
    // SAFETY: a SID: revision, sub-authority count, 6 bytes of authority, then the sub-authorities.
    unsafe {
        let count = *p.add(1) as usize;
        let at = 8 + 4 * (count - 1);
        u32::from_le_bytes([*p.add(at), *p.add(at + 1), *p.add(at + 2), *p.add(at + 3)])
    }
}

/// Start `cmd` (this image with `--child <args>`) with `token` (a primary token; NULL = CreateProcessW's way)
/// and the startup desktop `desktop` (UTF-16, NUL-terminated, or empty for none); `flags` are creation flags.
fn start_as(token: HANDLE, args: &str, flags: u32, desktop: &[u16], inherit: BOOL) -> Option<Child> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut cmd = [0u16; 800];
    let mut m = 0;
    let mut push = |u: u16| {
        if m < 799 {
            cmd[m] = u;
            m += 1;
        }
    };
    push(u16::from(b'"'));
    for &u in &exe[..n] {
        push(u);
    }
    push(u16::from(b'"'));
    for b in " --child ".bytes().chain(args.bytes()) {
        push(u16::from(b));
    }
    let si = STARTUPINFOW {
        cb: 104,
        reserved: core::ptr::null_mut(),
        desktop: if desktop.is_empty() { core::ptr::null_mut() } else { desktop.as_ptr() as *mut u16 },
        title: core::ptr::null_mut(),
        pos: [0; 7],
        flags: 0,
        show_window: 0,
        cb_reserved2: 0,
        reserved2: core::ptr::null_mut(),
        std: [core::ptr::null_mut(); 3],
    };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: `cmd` is mutable and NUL-terminated; the structures are live locals.
    let ok = unsafe {
        CreateProcessAsUserW(
            token,
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            inherit,
            flags,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(Child { process: pi.process, thread: pi.thread, pid: pi.pid, tid: pi.tid })
}

/// `a\b` as UTF-16 with a terminating NUL.
fn join_names(a: &str, b: &str) -> ([u16; 128], usize) {
    let mut w = [0u16; 128];
    let mut k = 0;
    for c in a.chars() {
        w[k] = c as u16;
        k += 1;
    }
    w[k] = 0x5c;
    k += 1;
    for c in b.chars() {
        w[k] = c as u16;
        k += 1;
    }
    (w, k + 1)
}

/// The process token, opened with all access.
fn process_token() -> HANDLE {
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer; the pseudo handle is valid.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut tok) };
    tok
}

/// A restricted primary token made from the process token as the broker does (the source is the process
/// token itself, so CreateProcessAsUserW needs no extra privilege): `flags`, SIDs to disable, SIDs to restrict.
fn restricted_primary(flags: u32, disable: &[SID_AND_ATTRIBUTES], restrict: &[SID_AND_ATTRIBUTES]) -> HANDLE {
    let src = process_token();
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: the arrays are live; the counts are their lengths.
    unsafe {
        CreateRestrictedToken(
            src,
            flags,
            disable.len() as u32,
            if disable.is_empty() { core::ptr::null() } else { disable.as_ptr() },
            0,
            core::ptr::null(),
            restrict.len() as u32,
            if restrict.is_empty() { core::ptr::null() } else { restrict.as_ptr() },
            &mut out,
        );
    }
    close(src);
    out
}

// userenv.h, securityappcontainer.h, securitybaseapi.h, winnt.h: AppContainer profiles, SIDs and tokens.
#[link(name = "userenv", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateAppContainerProfile(
        name: *const u16,
        display: *const u16,
        desc: *const u16,
        caps: *const SID_AND_ATTRIBUTES,
        n_caps: u32,
        sid: *mut *mut c_void,
    ) -> i32;
    fn DeleteAppContainerProfile(name: *const u16) -> i32;
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut c_void) -> i32;
    fn GetAppContainerFolderPath(sid_text: *const u16, path: *mut *mut u16) -> i32;
    fn GetAppContainerRegistryLocation(access: u32, key: *mut *mut c_void) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn GetProcAddress(module: HANDLE, name: *const u8) -> *const c_void;
    fn GetAppContainerNamedObjectPath(
        token: HANDLE,
        sid: *const c_void,
        len: u32,
        path: *mut u16,
        ret: *mut u32,
    ) -> BOOL;
    fn CreateMutexW(sa: *const SECURITY_ATTRIBUTES, owner: BOOL, name: *const u16) -> HANDLE;
    fn OpenMutexW(access: u32, inherit: BOOL, name: *const u16) -> HANDLE;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disp: u32,
        flags: u32,
        tmpl: HANDLE,
    ) -> HANDLE;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemFree(p: *mut c_void);
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
    fn LookupAccountSidW(
        system: *const u16,
        sid: *const c_void,
        name: *mut u16,
        name_len: *mut u32,
        domain: *mut u16,
        domain_len: *mut u32,
        use_: *mut u32,
    ) -> BOOL;
    fn IsValidSid(sid: *const c_void) -> BOOL;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn EqualSid(a: *const c_void, b: *const c_void) -> BOOL;
    fn CheckTokenMembership(token: HANDLE, sid: *const c_void, is_member: *mut BOOL) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        rev: u32,
        sd: *mut *mut c_void,
        len: *mut u32,
    ) -> BOOL;
}

/// winnt.h `SECURITY_CAPABILITIES`.
#[repr(C)]
struct SECURITY_CAPABILITIES {
    app_container_sid: *mut c_void,
    capabilities: *const SID_AND_ATTRIBUTES,
    capability_count: u32,
    reserved: u32,
}

const SE_GROUP_ENABLED: u32 = 0x4;
const TokenOwner: u32 = 4;
const TokenPrimaryGroup: u32 = 5;
const TokenUser: u32 = 1;
const TokenSessionId: u32 = 12;
const TokenElevationType: u32 = 18;
const TokenIsAppContainer: u32 = 29;
const TokenCapabilities: u32 = 30;
const TokenAppContainerSid: u32 = 31;
const TokenAppContainerNumber: u32 = 32;
const TokenIsLessPrivilegedAppContainer: u32 = 46;
const TokenIsSandboxed: u32 = 47;
const TokenImpersonationLevel: u32 = 9;
const TokenLogonSid: u32 = 28;
const TokenMandatoryPolicy: u32 = 27;

/// A SID as text (`S-1-...`) or `<null>`.
fn sid_str(p: *const c_void) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if p.is_null() {
        let _ = write!(b, "<null>");
        return b;
    }
    let mut s: *mut u16 = core::ptr::null_mut();
    // SAFETY: `p` is a SID the system returned or one built by this test; `s` is a live out pointer.
    unsafe {
        if ConvertSidToStringSidW(p, &mut s) == 0 {
            let _ = write!(b, "<invalid>");
            return b;
        }
        let u = wstr_at(s, 190);
        let t = units_text(u);
        let _ = write!(b, "{}", t.as_str());
        LocalFree(s.cast());
    }
    b
}

/// The profile's SID (derived from the name); null if the derivation fails.
fn derive(name: &str) -> *mut c_void {
    let w = wide(name);
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: NUL-terminated name, live out pointer.
    unsafe { DeriveAppContainerSidFromAppContainerName(w.as_ptr(), &mut p) };
    p
}

/// Delete the profile `name` (ignore the result).
fn delete_profile(name: &str) -> i32 {
    let w = wide(name);
    // SAFETY: NUL-terminated name.
    unsafe { DeleteAppContainerProfile(w.as_ptr()) }
}

/// Create the profile `name`; the SID (or null) and the HRESULT.
fn create_profile(name: &str) -> (*mut c_void, i32) {
    let w = wide(name);
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: NUL-terminated strings, live out pointer.
    let hr = unsafe { CreateAppContainerProfile(w.as_ptr(), w.as_ptr(), w.as_ptr(), core::ptr::null(), 0, &mut p) };
    (p, hr)
}

/// A kernelbase.dll / ntdll.dll export by name, or null.
fn export_of(dll: &str, name: &[u8]) -> *const c_void {
    let w = wide(dll);
    // SAFETY: NUL-terminated names; the module stays loaded.
    unsafe {
        let m = LoadLibraryW(w.as_ptr());
        if m.is_null() {
            return core::ptr::null();
        }
        GetProcAddress(m, name.as_ptr())
    }
}

/// `GetTokenInformation` into a buffer: (ok, length returned, last error).
fn token_info(token: HANDLE, class: u32, buf: &mut [u8]) -> (bool, u32, u32) {
    let mut n = 0u32;
    // SAFETY: the buffer is live and its length is passed.
    let ok = unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n) };
    (ok != 0, n, last_error())
}

/// A DWORD token information value, or None.
fn token_dword(token: HANDLE, class: u32) -> Option<u32> {
    let mut b = [0u8; 8];
    let (ok, _, _) = token_info(token, class, &mut b[..4]);
    ok.then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The pointer-sized SID of a TOKEN_APPCONTAINER_INFORMATION / TOKEN_OWNER-like buffer (first field).
fn first_sid(buf: &[u8]) -> *const c_void {
    // SAFETY: the buffer starts with a pointer (the system writes the SID after the structure inside the same buffer).
    unsafe { *(buf.as_ptr() as *const *const c_void) }
}

/// The SIDs and attributes of a TOKEN_GROUPS-like list as lines of text.
fn say_groups(t: &mut Test, what: &str, token: HANDLE, class: u32) {
    let mut buf = [0u8; 4096];
    let (ok, _, e) = token_info(token, class, &mut buf);
    if !ok {
        say!(t, "{what}: failed, error {e}");
        return;
    }
    let count = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    say!(t, "{what}: {count} entries");
    for i in 0..count as usize {
        let at = 8 + i * 16;
        let mut pb = [0u8; 8];
        pb.copy_from_slice(&buf[at..at + 8]);
        let p = usize::from_le_bytes(pb) as *const c_void;
        let attr = u32::from_le_bytes([buf[at + 8], buf[at + 9], buf[at + 10], buf[at + 11]]);
        say!(t, "  {} attributes {attr:#x}", sid_str(p).as_str());
    }
}

// More helpers for the AppContainer tests: moniker lookups, registry values, security descriptors, AccessCheck.
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AccessCheck(
        sd: *const c_void,
        client: HANDLE,
        desired: u32,
        mapping: *const GENERIC_MAPPING,
        privs: *mut c_void,
        privs_len: *mut u32,
        granted: *mut u32,
        status: *mut BOOL,
    ) -> BOOL;
    fn RegOpenKeyExW(key: HANDLE, sub: *const u16, options: u32, sam: u32, out: *mut HANDLE) -> i32;
    fn RegQueryValueExW(
        key: HANDLE,
        name: *const u16,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: HANDLE) -> i32;
}

/// winnt.h `GENERIC_MAPPING`.
#[repr(C)]
struct GENERIC_MAPPING {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}

/// The mapping of file objects (winnt.h FILE_GENERIC_READ / WRITE / EXECUTE, FILE_ALL_ACCESS).
const FILE_MAPPING: GENERIC_MAPPING =
    GENERIC_MAPPING { read: 0x0012_0089, write: 0x0012_0116, execute: 0x0012_00A0, all: 0x001F_01FF };
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const HKEY_CURRENT_USER: HANDLE = 0x8000_0001usize as HANDLE;
const KEY_READ: u32 = 0x0002_0019;

type FnRegisterSid = unsafe extern "system" fn(*const c_void, *const u16, *const u16) -> i32;
type FnLookupMoniker = unsafe extern "system" fn(*const c_void, *mut *mut u16) -> i32;
type FnCreateToken = unsafe extern "system" fn(HANDLE, *const SECURITY_CAPABILITIES, *mut HANDLE) -> BOOL;
type FnDeriveCaps =
    unsafe extern "system" fn(*const u16, *mut *mut *mut c_void, *mut u32, *mut *mut *mut c_void, *mut u32) -> BOOL;
type FnObjectPath = unsafe extern "system" fn(HANDLE, *const c_void, u32, *mut u16, *mut u32) -> BOOL;

fn register_fn() -> Option<FnRegisterSid> {
    let p = export_of("kernelbase.dll", b"AppContainerRegisterSid\0");
    // SAFETY: the export has the shape inferred from its calls (a PSID and two strings, an HRESULT).
    (!p.is_null()).then(|| unsafe { core::mem::transmute::<*const c_void, FnRegisterSid>(p) })
}

fn lookup_fn() -> Option<FnLookupMoniker> {
    let p = export_of("kernelbase.dll", b"AppContainerLookupMoniker\0");
    // SAFETY: the export has the shape inferred from its calls (a PSID and an out string pointer, an HRESULT).
    (!p.is_null()).then(|| unsafe { core::mem::transmute::<*const c_void, FnLookupMoniker>(p) })
}

fn token_fn() -> Option<FnCreateToken> {
    let p = export_of("kernelbase.dll", b"CreateAppContainerToken\0");
    // SAFETY: the export has the shape inferred from its calls (a token, SECURITY_CAPABILITIES, an out handle, a BOOL).
    (!p.is_null()).then(|| unsafe { core::mem::transmute::<*const c_void, FnCreateToken>(p) })
}

/// `AppContainerRegisterSid(sid, moniker, display)`; None when the export is absent.
fn register_sid(sid: *const c_void, moniker: Option<&[u16]>, display: Option<&[u16]>) -> Option<i32> {
    let f = register_fn()?;
    let m = moniker.map_or(core::ptr::null(), |m| m.as_ptr());
    let d = display.map_or(core::ptr::null(), |d| d.as_ptr());
    // SAFETY: NUL-terminated strings or NULL, a SID of this test.
    Some(unsafe { f(sid, m, d) })
}

/// `AppContainerLookupMoniker(sid)`: the HRESULT and the moniker text (empty when none came back).
fn lookup_moniker(sid: *const c_void) -> (i32, Buf<200>) {
    let mut text = Buf::<200>::new();
    let Some(f) = lookup_fn() else { return (-1, text) };
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: a SID of this test and a live out pointer.
    let hr = unsafe { f(sid, &mut out) };
    if !out.is_null() {
        let t = units_text(wstr_at(out, 190));
        let _ = write!(text, "{}", t.as_str());
    }
    (hr, text)
}

/// A string value of `Mappings\<sid>` under the user's AppContainer registry key, or `<none>`.
fn mapping_value(sid_text: &str, value: &str) -> Buf<200> {
    let mut path = Buf::<400>::new();
    let _ = write!(
        path,
        "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppContainer\\Mappings\\{sid_text}"
    );
    let w = wide(path.as_str());
    let v = wide(value);
    let mut out = Buf::<200>::new();
    let mut key: HANDLE = core::ptr::null_mut();
    // SAFETY: NUL-terminated names, live out pointers, a buffer of 400 bytes.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, w.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            let _ = write!(out, "<no key>");
            return out;
        }
        let mut buf = [0u8; 400];
        let mut n = 400u32;
        let mut ty = 0u32;
        let r = RegQueryValueExW(key, v.as_ptr(), core::ptr::null_mut(), &mut ty, buf.as_mut_ptr(), &mut n);
        RegCloseKey(key);
        if r != 0 {
            let _ = write!(out, "<no value>");
            return out;
        }
        let units = core::slice::from_raw_parts(buf.as_ptr() as *const u16, (n as usize / 2).min(200));
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        let t = units_text(&units[..end]);
        let _ = write!(out, "{}", t.as_str());
    }
    out
}

/// A security descriptor from its SDDL text (leaked; the tests are short-lived).
fn sd_from(sddl: &str) -> *mut c_void {
    let w = wide(sddl);
    let mut sd: *mut c_void = core::ptr::null_mut();
    // SAFETY: a NUL-terminated string and a live out pointer.
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), 1, &mut sd, core::ptr::null_mut()) };
    sd
}

/// `AccessCheck(sd, token, desired)` with the file mapping: (call ok, granted mask, access status, last error).
fn access(sd: *const c_void, token: HANDLE, desired: u32) -> (bool, u32, bool, u32) {
    let mut privs = [0u8; 64];
    let mut privs_len = 64u32;
    let mut granted = 0xAAAAu32;
    let mut status: BOOL = -1;
    // SAFETY: the descriptor and the impersonation token are the test's own; the buffers are live.
    let ok = unsafe {
        AccessCheck(
            sd,
            token,
            desired,
            &FILE_MAPPING,
            privs.as_mut_ptr().cast(),
            &mut privs_len,
            &mut granted,
            &mut status,
        )
    };
    (ok != 0, granted, status == 1, last_error())
}

/// One line of an access decision: `<what>: <allowed|denied|failed> granted <mask>`.
fn say_access(t: &mut Test, what: &str, sd_text: &str, token: HANDLE, desired: u32) {
    let sd = sd_from(sd_text);
    if sd.is_null() {
        say!(t, "{what}: descriptor refused");
        return;
    }
    let (ok, granted, status, err) = access(sd, token, desired);
    if !ok {
        say!(t, "{what}: AccessCheck failed, error {err}");
    } else if status {
        say!(t, "{what}: allowed, granted {granted:#x}");
    } else {
        say!(t, "{what}: denied (error {err}), granted {granted:#x}");
    }
}

/// An AppContainer token (primary) made from the process token for the profile SID `sid` with the capability SIDs
/// given as strings (attribute SE_GROUP_ENABLED); NULL when the call fails.
fn lowbox(sid: *mut c_void, caps: &[&str]) -> HANDLE {
    let Some(f) = token_fn() else { return core::ptr::null_mut() };
    let mut list = [SID_AND_ATTRIBUTES { sid: core::ptr::null_mut(), attributes: 0 }; 8];
    for (i, c) in caps.iter().enumerate().take(8) {
        list[i] = SID_AND_ATTRIBUTES { sid: self::sid(c), attributes: SE_GROUP_ENABLED };
    }
    let sc = SECURITY_CAPABILITIES {
        app_container_sid: sid,
        capabilities: if caps.is_empty() { core::ptr::null() } else { list.as_ptr() },
        capability_count: caps.len() as u32,
        reserved: 0,
    };
    let src = process_token();
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: the structure and the list outlive the call; the process token has all access.
    unsafe { f(src, &sc, &mut out) };
    close(src);
    out
}

/// An impersonation copy (identification excluded: level 2) of `token`.
fn as_impersonation(token: HANDLE) -> HANDLE {
    let mut dup: HANDLE = core::ptr::null_mut();
    // SAFETY: a live out pointer.
    unsafe {
        DuplicateTokenEx(
            token,
            TOKEN_ALL_ACCESS,
            core::ptr::null(),
            SecurityImpersonation,
            TokenImpersonation,
            &mut dup,
        )
    };
    dup
}

/// The SID of an AppContainer token's `TokenAppContainerSid`, as text (`<none>` when the token has none).
fn token_ac_sid(token: HANDLE) -> Buf<200> {
    let mut buf = [0u8; 256];
    let (ok, _, _) = token_info(token, TokenAppContainerSid, &mut buf);
    if !ok || first_sid(&buf).is_null() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<none>");
        return b;
    }
    sid_str(first_sid(&buf))
}

/// An optional HRESULT as text: `0x...` or `absent`.
fn hr_text(r: Option<i32>) -> Buf<16> {
    let mut b = Buf::<16>::new();
    match r {
        Some(v) => {
            let _ = write!(b, "{v:#x}");
        }
        None => {
            let _ = write!(b, "absent");
        }
    }
    b
}

main_with_child!(child, test);

fn child(_: &str) -> u32 {
    0
}

fn path_of(token: HANDLE, sid: *const c_void, len: u32) -> (bool, u32, u32, Buf<260>) {
    let mut buf = [0u16; 300];
    let mut ret = 0xAAAAu32;
    mark_error();
    // SAFETY: a buffer of 300 units (the length passed is at most that).
    let ok = unsafe { GetAppContainerNamedObjectPath(token, sid, len.min(300), buf.as_mut_ptr(), &mut ret) };
    let e = last_error();
    let mut b = Buf::<260>::new();
    let _ = write!(b, "{}", units_text(wstr_at(buf.as_ptr(), 250)).as_str());
    (ok != 0, ret, e, b)
}

fn test(t: &mut Test) {
    SAY_PART.store(2, core::sync::atomic::Ordering::Relaxed);
    let sid = derive("polys2b.nop1");
    let text = sid_str(sid);
    let (ok, ret, e, p) = path_of(core::ptr::null_mut(), sid, 300);
    say!(t, "NULL token, container SID: ok {ok}, length {ret}, path '{}'", p.as_str());
    say!(t, "  the length is the number of characters with the NUL: {}", ret as usize == p.as_str().len() + 1);
    say!(t, "  the path is `AppContainerNamedObjects\\` and the SID text: {}", {
        let mut want = Buf::<260>::new();
        let _ = write!(want, "AppContainerNamedObjects\\{}", text.as_str());
        want.as_str() == p.as_str()
    });
    let _ = e;
    let need = ret;
    for len in [0u32, 1, 10, need - 1, need, need + 1] {
        let (ok, ret, e, p) = path_of(core::ptr::null_mut(), sid, len);
        say!(
            t,
            "length {}: ok {ok}, needed {ret}, error {}, text length {}",
            if len == need {
                "= needed"
            } else if len + 1 == need {
                "= needed - 1"
            } else {
                "other"
            },
            if ok { 0 } else { e },
            p.as_str().len()
        );
    }
    // a different SID
    let other = derive("polys2b.nop2");
    let (_, _, _, p2) = path_of(core::ptr::null_mut(), other, 300);
    say!(
        t,
        "another SID gives another path with the same shape: {} {}",
        p2.as_str() != p.as_str(),
        p2.as_str().starts_with("AppContainerNamedObjects\\S-1-15-2-")
    );
    // an AppContainer token
    let tok = lowbox(sid, &[]);
    let (ok, ret, e, p3) = path_of(tok, core::ptr::null(), 300);
    say!(
        t,
        "AppContainer token, NULL SID: ok {ok}, length {ret}, same path as for the SID {}",
        p3.as_str() == p.as_str()
    );
    let _ = e;
    let (ok, _, e, _) = path_of(tok, sid, 300);
    say!(t, "AppContainer token and SID together: ok {ok}, error {}", if ok { 0 } else { e });
    let (ok, _, e, _) = path_of(tok, other, 300);
    say!(t, "AppContainer token and another SID: ok {ok}, error {}", if ok { 0 } else { e });
    let imp = as_impersonation(tok);
    let (ok, _, _, p4) = path_of(imp, core::ptr::null(), 300);
    say!(t, "impersonation copy of the token: ok {ok}, same path {}", p4.as_str() == p.as_str());
    let plain = process_token();
    let (ok, _, e, _) = path_of(plain, core::ptr::null(), 300);
    say!(t, "ordinary token, NULL SID: ok {ok}, error {}", if ok { 0 } else { e });
    let (ok, _, e, _) = path_of(plain, sid, 300);
    say!(t, "ordinary token and a container SID: ok {ok}, error {}", if ok { 0 } else { e });
    let (ok, _, e, _) = path_of(core::ptr::null_mut(), core::ptr::null(), 300);
    say!(t, "NULL token and NULL SID (this process is no container): ok {ok}, error {}", if ok { 0 } else { e });
    for (what, s) in [
        ("Administrators", "S-1-5-32-544"),
        ("ALL APPLICATION PACKAGES", "S-1-15-2-1"),
        ("a capability SID", "S-1-15-3-1"),
        ("9 sub-authorities", "S-1-15-2-1-2-3-4-5-6-7-8"),
    ] {
        let (ok, _, e, _) = path_of(core::ptr::null_mut(), sid_from(s), 300);
        say!(t, "SID {what}: ok {ok}, error {}", if ok { 0 } else { e });
    }
    let (ok, _, e, _) = path_of(0x7777usize as HANDLE, core::ptr::null(), 300);
    say!(t, "a token handle that is not valid: ok {ok}, error {}", if ok { 0 } else { e });
    // the two exports are the same function for callers
    let k = export_of("kernel32.dll", b"GetAppContainerNamedObjectPath\0");
    let kb = export_of("kernelbase.dll", b"GetAppContainerNamedObjectPath\0");
    say!(t, "exported by kernel32.dll {} and kernelbase.dll {}", !k.is_null(), !kb.is_null());
    close(tok);
    close(imp);
    close(plain);
}

fn sid_from(s: &str) -> *const c_void {
    sid(s)
}
