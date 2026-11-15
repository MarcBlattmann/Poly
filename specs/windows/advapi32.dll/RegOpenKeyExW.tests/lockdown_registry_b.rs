//! RegOpenKeyExW / RegCreateKeyExW in a locked-down target: the process token is a restricted Untrusted
//! token (privileges removed, Administrators and Users denied, restricted to the Null SID), the main thread
//! carries the broker's less restricted impersonation token until the target calls RevertToSelf. In stage 2
//! the confined targets get ERROR_ACCESS_DENIED from RegOpenKeyExW(HKLM, "SOFTWARE\Microsoft\Windows NT\
//! CurrentVersion") and from NtCreateKey, which succeed in stage 1. The same calls from the unrestricted parent
//! are listed for contrast.
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

// winuser.h: window stations and desktops (CreateWindowStationW, CreateDesktopW, GetUserObjectInformationW, ...).
#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateWindowStationW(name: *const u16, flags: u32, access: u32, sa: *const SECURITY_ATTRIBUTES) -> HANDLE;
    fn OpenWindowStationW(name: *const u16, inherit: BOOL, access: u32) -> HANDLE;
    fn CloseWindowStation(h: HANDLE) -> BOOL;
    fn GetProcessWindowStation() -> HANDLE;
    fn SetProcessWindowStation(h: HANDLE) -> BOOL;
    fn CreateDesktopW(
        name: *const u16,
        device: *const u16,
        devmode: *const c_void,
        flags: u32,
        access: u32,
        sa: *const SECURITY_ATTRIBUTES,
    ) -> HANDLE;
    fn OpenDesktopW(name: *const u16, flags: u32, inherit: BOOL, access: u32) -> HANDLE;
    fn CloseDesktop(h: HANDLE) -> BOOL;
    fn GetThreadDesktop(tid: u32) -> HANDLE;
    fn SetThreadDesktop(h: HANDLE) -> BOOL;
    fn GetUserObjectInformationW(h: HANDLE, index: i32, info: *mut c_void, len: u32, needed: *mut u32) -> BOOL;
    fn SetUserObjectInformationW(h: HANDLE, index: i32, info: *const c_void, len: u32) -> BOOL;
}

const UOI_FLAGS: i32 = 1;
const UOI_NAME: i32 = 2;
const UOI_TYPE: i32 = 3;
const UOI_USER_SID: i32 = 4;
const UOI_HEAPSIZE: i32 = 5;
const UOI_IO: i32 = 6;
const GENERIC_ALL: u32 = 0x1000_0000;
const WINSTA_ALL_ACCESS: u32 = 0x37F;
const WINSTA_ENUMDESKTOPS: u32 = 0x1;
const WINSTA_READATTRIBUTES: u32 = 0x2;
const WINSTA_ACCESSCLIPBOARD: u32 = 0x4;
const WINSTA_CREATEDESKTOP: u32 = 0x8;
const WINSTA_WRITEATTRIBUTES: u32 = 0x10;
const WINSTA_ACCESSGLOBALATOMS: u32 = 0x20;
const WINSTA_EXITWINDOWS: u32 = 0x40;
const WINSTA_ENUMERATE: u32 = 0x100;
const WINSTA_READSCREEN: u32 = 0x200;
const DESKTOP_READOBJECTS: u32 = 0x1;
const DESKTOP_CREATEWINDOW: u32 = 0x2;
const DESKTOP_CREATEMENU: u32 = 0x4;
const DESKTOP_HOOKCONTROL: u32 = 0x8;
const DESKTOP_JOURNALRECORD: u32 = 0x10;
const DESKTOP_JOURNALPLAYBACK: u32 = 0x20;
const DESKTOP_ENUMERATE: u32 = 0x40;
const DESKTOP_WRITEOBJECTS: u32 = 0x80;
const DESKTOP_SWITCHDESKTOP: u32 = 0x100;
const DESKTOP_ALL: u32 = 0x1FF | 0x000F_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const DELETE: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const CWF_CREATE_ONLY: u32 = 1;

/// A UTF-16 buffer to text (up to its NUL).
fn wbuf_text(b: &[u16]) -> Buf<600> {
    let n = b.iter().position(|&u| u == 0).unwrap_or(b.len());
    units_text(&b[..n])
}

/// UOI_NAME of a window station or desktop.
fn uoi_name(h: HANDLE) -> Buf<600> {
    let mut b = [0u16; 128];
    let mut need = 0u32;
    // SAFETY: the buffer is 256 bytes, `need` is a live local.
    let ok = unsafe { GetUserObjectInformationW(h, UOI_NAME, b.as_mut_ptr().cast(), 256, &mut need) };
    let mut out = Buf::<600>::new();
    if ok == 0 {
        let _ = write!(out, "<error {}>", last_error());
    } else {
        let _ = write!(out, "{}", wbuf_text(&b).as_str());
    }
    out
}

/// Text of the information `index` of `h` as bytes in hex (first `n`), or the error.
fn uoi_bytes(h: HANDLE, index: i32, cap: usize) -> Buf<600> {
    let mut b = [0u8; 256];
    let mut need = 0u32;
    // SAFETY: the buffer holds 256 bytes, `need` is a live local.
    let ok = unsafe { GetUserObjectInformationW(h, index, b.as_mut_ptr().cast(), cap.min(256) as u32, &mut need) };
    let mut out = Buf::<600>::new();
    if ok == 0 {
        let _ = write!(out, "error {} need {}", last_error(), need);
    } else {
        let _ = write!(out, "ok need {} bytes", need);
        for x in &b[..(need as usize).min(256).min(cap)] {
            let _ = write!(out, " {x:02x}");
        }
    }
    out
}

fn same_name(h: HANDLE, want: &str) -> u32 {
    let n = uoi_name(h);
    u32::from(n.as_str() == want)
}

/// Like `uoi_bytes` but without the value (for values that depend on the machine).
fn uoi_status(h: HANDLE, index: i32, cap: usize) -> Buf<600> {
    let mut b = [0u8; 256];
    let mut need = 0u32;
    // SAFETY: the buffer holds 256 bytes, `need` is a live local.
    let ok = unsafe { GetUserObjectInformationW(h, index, b.as_mut_ptr().cast(), cap.min(256) as u32, &mut need) };
    let mut out = Buf::<600>::new();
    if ok == 0 {
        let _ = write!(out, "error {} need {}", last_error(), need);
    } else {
        let _ = write!(out, "ok need {}", need);
    }
    out
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

// The sandbox broker's way to start a locked-down target, built from public APIs (Learn: "CreateRestrictedToken",
// "SetThreadToken", "CreateProcessAsUserW", "Mandatory Integrity Control"): a restricted primary token for the
// process, a less restricted impersonation token set on the suspended main thread, the process resumed; the
// target lowers itself later with RevertToSelf. Declarations of ntdll's registry calls are from winternl.h/wdm.h.
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32, out: *mut isize) -> i32;
    fn RegCreateKeyExW(
        key: isize,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        sa: *const c_void,
        out: *mut isize,
        disposition: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
    fn RegDeleteKeyExW(key: isize, sub: *const u16, access: u32, reserved: u32) -> i32;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> i32;
}

const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_READ: u32 = 0x20019;
const KEY_QUERY_VALUE: u32 = 0x1;
const KEY_WRITE: u32 = 0x20006;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const REG_OPTION_VOLATILE: u32 = 1;

/// The restricting SIDs of the process token ("lockdown") and of the thread's impersonation token ("initial").
struct Tokens {
    process: HANDLE,
    thread: HANDLE,
}

/// Restricted primary token for the process: the privileges removed, Administrators and Users denied,
/// restricted to the Null SID, Untrusted integrity; and the impersonation token the broker puts on the
/// main thread while the target starts: restricted to Everyone, Users, RESTRICTED and the Null SID, Low integrity.
fn make_tokens(process_il: &str, thread_il: &str) -> Tokens {
    let admins = SID_AND_ATTRIBUTES { sid: sid("S-1-5-32-544"), attributes: 0 };
    let users = SID_AND_ATTRIBUTES { sid: sid("S-1-5-32-545"), attributes: 0 };
    let null = SID_AND_ATTRIBUTES { sid: sid("S-1-0-0"), attributes: 0 };
    let everyone = SID_AND_ATTRIBUTES { sid: sid("S-1-1-0"), attributes: 0 };
    let restricted = SID_AND_ATTRIBUTES { sid: sid("S-1-5-12"), attributes: 0 };
    let process = restricted_primary(DISABLE_MAX_PRIVILEGE, &[admins, users], &[null]);
    set_integrity(process, process_il);
    // the impersonation token: a restricted copy of the process token, then made an impersonation token
    let base = restricted_primary(DISABLE_MAX_PRIVILEGE, &[], &[everyone, users, restricted, null]);
    set_integrity(base, thread_il);
    let mut thread: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer.
    unsafe {
        DuplicateTokenEx(
            base,
            TOKEN_ALL_ACCESS,
            core::ptr::null(),
            SecurityImpersonation,
            TokenImpersonation,
            &mut thread,
        )
    };
    close(base);
    Tokens { process, thread }
}

/// Start the target suspended with the process token; `imp` puts the impersonation token on its main thread
/// first (SetThreadToken on a thread handle of the new process); resume it. The result of SetThreadToken
/// (1 / 0 and its last error) is returned with the child.
fn launch_locked(tokens: &Tokens, args: &str, imp: bool, extra_flags: u32) -> (Option<Child>, i32, u32) {
    let c = start_as(
        tokens.process,
        args,
        CREATE_SUSPENDED | DETACHED_PROCESS | CREATE_UNICODE_ENVIRONMENT | extra_flags,
        &[],
        1,
    );
    let c = match c {
        Some(c) => c,
        None => return (None, -1, last_error()),
    };
    let (mut r, mut e) = (-1, 0);
    if imp {
        mark_error();
        // SAFETY: a thread handle of the suspended child; the token is an impersonation token.
        r = unsafe { SetThreadToken(&c.thread, tokens.thread) };
        e = last_error();
    }
    // SAFETY: the thread handle of the suspended child.
    unsafe { ResumeThread(c.thread) };
    (Some(c), r, e)
}

/// Facts about the token the calling thread acts under, as lines through `out`.
fn token_facts(out: &mut dyn Sink, label: &str) {
    let mut th: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer; the pseudo handle is valid.
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 0, &mut th) };
    let e = last_error();
    let mut th_self: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let ok_self = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut th_self) };
    let e_self = last_error();
    close(th_self);
    let mut b = Buf::<300>::new();
    if ok == 0 {
        let _ = write!(b, "{label}: no thread token (OpenThreadToken error {e})");
    } else {
        let mut kind = 0u32;
        let mut n = 0u32;
        let mut restricted = 0u32;
        // SAFETY: the out buffers hold 4 bytes each.
        unsafe {
            GetTokenInformation(th, TokenType, (&mut kind as *mut u32).cast(), 4, &mut n);
            GetTokenInformation(th, TokenIsRestricted, (&mut restricted as *mut u32).cast(), 4, &mut n);
        }
        let _ = write!(
            b,
            "{label}: thread token type {kind} restricted {restricted} integrity {:#x}; opened as the process: {} (error {})",
            integrity_of(th),
            ok_self,
            e_self
        );
        close(th);
    }
    out.put(b.as_str());
}

/// The same facts for the process token.
fn process_token_facts(out: &mut dyn Sink, label: &str) {
    let tok = process_token_query();
    let mut restricted = 0u32;
    let mut n = 0u32;
    // SAFETY: the out buffer holds 4 bytes.
    unsafe { GetTokenInformation(tok, TokenIsRestricted, (&mut restricted as *mut u32).cast(), 4, &mut n) };
    let mut b = Buf::<300>::new();
    let _ = write!(b, "{label}: process token restricted {restricted} integrity {:#x}", integrity_of(tok));
    close(tok);
    out.put(b.as_str());
}

fn process_token_query() -> HANDLE {
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer; the pseudo handle is valid.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) };
    tok
}

/// The text of a Win32 registry result (0 = success).
fn reg_text(r: i32) -> Buf<24> {
    let mut b = Buf::<24>::new();
    let _ = write!(b, "{r}");
    b
}

main_with_child!(child, test);

fn open_line(out: &mut dyn Sink, phase: &str, label: &str, root: isize, path: &str, access: u32) {
    let mut k: isize = 0;
    // SAFETY: live out pointer and NUL-terminated name.
    let r = unsafe { RegOpenKeyExW(root, wide(path).as_ptr(), 0, access, &mut k) };
    let mut b = Buf::<240>::new();
    let _ = write!(b, "{phase}: RegOpenKeyExW({label}) = {}", r);
    out.put(b.as_str());
    if r == 0 {
        // SAFETY: a key this call opened.
        unsafe { RegCloseKey(k) };
    }
}

fn create_line(out: &mut dyn Sink, phase: &str, label: &str, root: isize, path: &str) {
    let mut k: isize = 0;
    let mut disp = 0u32;
    // SAFETY: live out pointers and NUL-terminated name.
    let r = unsafe {
        RegCreateKeyExW(
            root,
            wide(path).as_ptr(),
            0,
            core::ptr::null(),
            REG_OPTION_VOLATILE,
            KEY_WRITE,
            core::ptr::null(),
            &mut k,
            &mut disp,
        )
    };
    let mut b = Buf::<240>::new();
    let _ = write!(b, "{phase}: RegCreateKeyExW({label}, volatile) = {}", r);
    out.put(b.as_str());
    if r == 0 {
        // SAFETY: a key this call created and that is empty.
        unsafe {
            RegCloseKey(k);
            RegDeleteKeyExW(root, wide(path).as_ptr(), 0, 0);
        }
    }
}

fn battery(out: &mut dyn Sink, phase: &str) {
    token_facts(out, phase);
    open_line(
        out,
        phase,
        "HKLM CurrentVersion, KEY_READ",
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
        KEY_READ,
    );
    open_line(
        out,
        phase,
        "HKLM CurrentVersion, KEY_QUERY_VALUE",
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
        KEY_QUERY_VALUE,
    );
    open_line(out, phase, "HKLM SOFTWARE, KEY_READ", HKEY_LOCAL_MACHINE, "SOFTWARE", KEY_READ);
    open_line(out, phase, "HKCU Software, KEY_READ", HKEY_CURRENT_USER, "Software", KEY_READ);
    open_line(out, phase, "HKLM, a missing key", HKEY_LOCAL_MACHINE, "SOFTWARE\\PolyS2NoSuchKey", KEY_READ);
    create_line(out, phase, "HKCU Software\\PolyS2LockProbe", HKEY_CURRENT_USER, "Software\\PolyS2LockProbe");
}

fn child(args: &str) -> u32 {
    let h = arg_handle(args, 0);
    let mut out = ChildOut { h };
    battery(&mut out, "target, impersonating");
    // SAFETY: plain call.
    let r = unsafe { RevertToSelf() };
    let mut b = Buf::<100>::new();
    let _ = write!(b, "target: RevertToSelf = {r}");
    out.put(b.as_str());
    battery(&mut out, "target, after RevertToSelf");
    0
}

fn test(t: &mut Test) {
    SAY_PART.store(2, core::sync::atomic::Ordering::Relaxed);
    battery(t, "parent, unrestricted");
    let tokens = make_tokens("S-1-16-0", "S-1-16-0");
    let rep = Report::new();
    let mut a = Buf::<60>::new();
    let _ = write!(a, "{:x}", rep.write as usize);
    let (c, r, e) = launch_locked(&tokens, a.as_str(), true, 0);
    match c {
        Some(c) => {
            say!(t, "SetThreadToken on the suspended main thread = {r}");
            let code = c.finish();
            rep.print(t);
            say!(t, "target exit code {code:#x}");
        }
        None => say!(t, "CreateProcessAsUserW failed err={e}"),
    }
}
