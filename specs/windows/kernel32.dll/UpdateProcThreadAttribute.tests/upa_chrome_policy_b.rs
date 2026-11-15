//! UpdateProcThreadAttribute with the attributes of Chrome's sandbox broker: the 16-byte mitigation policy
//! (two 64-bit words: the three sets Chrome uses for its GPU process, storage service and renderers), the child
//! process policy (4 bytes, value 1), attribute 0x2001A (4 bytes, value 1: PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER)
//! and the handle list. What each does to the child as the child reports it (GetProcessMitigationPolicy of
//! itself, whether it may start another process, whether it can still load a library that is not signed by
//! Microsoft or use win32k), and what the broker's six-attribute list does when sizes are wrong.
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

main_with_child!(child, test);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> BOOL;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        ret: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn GetProcessMitigationPolicy(process: HANDLE, policy: u32, buf: *mut c_void, len: usize) -> BOOL;
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn GetProcAddress(m: HANDLE, name: *const u8) -> usize;
    fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
}

const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x20002;
const PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY: usize = 0x20007;
const PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY: usize = 0x2000E;
const PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER: usize = 0x2001A;
const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x2000D;

/// STARTUPINFOEXW: the startup info and the attribute list pointer (112 bytes).
#[repr(C)]
struct STARTUPINFOEXW {
    si: STARTUPINFOW,
    list: *mut c_void,
}

const GPU: [u64; 2] = [0x0111_0001_0001_1000, 0x0100_0000_0001_0000];
const STORAGE: [u64; 2] = [0x0111_0001_1001_1000, 0x0100_0000_0001_0000];
const RENDERER: [u64; 2] = [0x0111_1001_1001_1000, 0x0100_0000_2001_1000];

fn policy_line(out: &mut dyn Sink) {
    // the policies the words set, as the child sees them: first four bytes of each (DEP 8)
    for (name, p, len) in [
        ("ASLR", 1u32, 4usize),
        ("DynamicCode", 2, 4),
        ("StrictHandleCheck", 3, 4),
        ("SystemCallDisable", 4, 4),
        ("ExtensionPointDisable", 6, 4),
        ("Signature", 8, 4),
        ("FontDisable", 9, 4),
        ("ImageLoad", 10, 4),
        ("ChildProcess", 13, 4),
        ("SideChannelIsolation", 14, 4),
        ("UserShadowStack", 15, 4),
    ] {
        let mut b = [0u8; 16];
        // SAFETY: a buffer of 16 bytes, the exact lengths the policies take.
        let ok = unsafe { GetProcessMitigationPolicy(GetCurrentProcess(), p, b.as_mut_ptr().cast(), len) };
        let mut l = Buf::<200>::new();
        let _ = write!(l, "policy {name} ({p}): ok={ok} flags {:#x}", u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        out.put(l.as_str());
    }
}

fn child(args: &str) -> u32 {
    if args.starts_with("grand") {
        return 0;
    }
    let h = arg_handle(args, 0);
    let mut out = ChildOut { h };
    policy_line(&mut out);
    // may this process start another one? (child process policy)
    let c = start_child("grand", 0);
    let mut l = Buf::<200>::new();
    match c {
        Some(c) => {
            let _ = write!(l, "starting a child process works, its exit code {:#x}", c.finish());
        }
        None => {
            let _ = write!(l, "starting a child process fails, error {}", last_error());
        }
    }
    out.put(l.as_str());
    // dynamic code
    // SAFETY: allocates a page and frees nothing (the process ends soon).
    let p = unsafe { VirtualAlloc(core::ptr::null_mut(), 4096, 0x3000, 0x40) };
    let mut l = Buf::<200>::new();
    let _ = write!(l, "VirtualAlloc(PAGE_EXECUTE_READWRITE): allocated {}", opened(p));
    out.put(l.as_str());
    // a library that is signed by Microsoft loads; user32 needs win32k
    for name in ["winmm.dll", "user32.dll"] {
        // SAFETY: plain calls.
        let m = unsafe { LoadLibraryW(wide(name).as_ptr()) };
        let e = if m.is_null() { last_error() } else { 0 };
        let mut l = Buf::<200>::new();
        let _ = write!(l, "LoadLibraryW({name}): loaded={} error {e}", opened(m));
        out.put(l.as_str());
        if name == "user32.dll" && opened(m) == 1 {
            // SAFETY: the export is looked up by name and called with no arguments only when found.
            unsafe {
                let f = GetProcAddress(m, b"GetDesktopWindow\0".as_ptr());
                let mut l = Buf::<200>::new();
                let _ = write!(l, "user32 GetDesktopWindow export found={}", u32::from(f != 0));
                out.put(l.as_str());
                if f != 0 {
                    let g: extern "system" fn() -> usize = core::mem::transmute(f);
                    let r = g();
                    let mut l = Buf::<200>::new();
                    let _ = write!(l, "GetDesktopWindow() returns a window: {}", u32::from(r != 0));
                    out.put(l.as_str());
                }
            }
        }
    }
    0
}

/// Start this image as a child with the attributes named; returns (CreateProcessW result, last error, child).
fn launch(t: &mut Test, label: &str, mitigation: Option<&[u8]>, child_policy: Option<u32>, component: Option<u32>) {
    let mut count = 1u32;
    count += u32::from(mitigation.is_some()) + u32::from(child_policy.is_some()) + u32::from(component.is_some());
    let mut size = 0usize;
    // SAFETY: the sizing call writes the needed size and fails with 122.
    unsafe { InitializeProcThreadAttributeList(core::ptr::null_mut(), count, 0, &mut size) };
    let mut store = [0u64; 64];
    if size > 512 {
        say!(t, "{label}: list size {size} too big for the test");
        return;
    }
    let list = store.as_mut_ptr().cast::<c_void>();
    // SAFETY: the buffer holds 512 bytes and is 8-byte aligned.
    let ok = unsafe { InitializeProcThreadAttributeList(list, count, 0, &mut size) };
    let rep = Report::new();
    let mut handles = [rep.write];
    let mut all_ok = ok;
    // SAFETY: the values live until the process is created.
    unsafe {
        all_ok &= UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            handles.as_mut_ptr().cast(),
            8,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
        if let Some(m) = mitigation {
            all_ok &= UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
                m.as_ptr().cast(),
                m.len(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            );
        }
        if let Some(c) = child_policy.as_ref() {
            all_ok &= UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
                (c as *const u32).cast(),
                4,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            );
        }
        if let Some(c) = component.as_ref() {
            all_ok &= UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER,
                (c as *const u32).cast(),
                4,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            );
        }
    }
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut cmd = [0u16; 800];
    let mut m = 0;
    let mut a = Buf::<60>::new();
    let _ = write!(a, "{:x}", rep.write as usize);
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
    for b in " --child ".bytes().chain(a.as_str().bytes()) {
        push(u16::from(b));
    }
    let sx = STARTUPINFOEXW {
        si: STARTUPINFOW {
            cb: 112,
            reserved: core::ptr::null_mut(),
            desktop: core::ptr::null_mut(),
            title: core::ptr::null_mut(),
            pos: [0; 7],
            flags: 0,
            show_window: 0,
            cb_reserved2: 0,
            reserved2: core::ptr::null_mut(),
            std: [core::ptr::null_mut(); 3],
        },
        list,
    };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    mark_error();
    // SAFETY: the structures are live; the command line is mutable and NUL-terminated.
    let r = unsafe {
        CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT,
            core::ptr::null(),
            core::ptr::null(),
            &sx.si,
            &mut pi,
        )
    };
    let e = err_text();
    say!(t, "{label}: list of {count} attributes built {all_ok}, CreateProcessW = {r} error {}", e.as_str());
    if r != 0 {
        let c = Child { process: pi.process, thread: pi.thread, pid: pi.pid, tid: pi.tid };
        let code = c.finish();
        rep.print(t);
        say!(t, "{label}: child exit code {code:#x}");
    } else {
        close(rep.read);
        close(rep.write);
    }
    // SAFETY: the list was initialised.
    unsafe { DeleteProcThreadAttributeList(list) };
}

fn bytes_of(w: &[u64; 2]) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&w[0].to_le_bytes());
    b[8..].copy_from_slice(&w[1].to_le_bytes());
    b
}

fn test(t: &mut Test) {
    SAY_PART.store(2, core::sync::atomic::Ordering::Relaxed);
    // sizes of the mitigation attribute value
    let mut size = 0usize;
    // SAFETY: the sizing call writes the needed size and fails with 122.
    let r = unsafe { InitializeProcThreadAttributeList(core::ptr::null_mut(), 6, 0, &mut size) };
    say!(
        t,
        "InitializeProcThreadAttributeList(NULL, 6): {r}, error {}, a size is returned {}",
        last_error(),
        u32::from(size >= 48)
    );
    let mut store = [0u64; 64];
    let list = store.as_mut_ptr().cast::<c_void>();
    // SAFETY: a list of 6 attributes in a buffer that is big enough.
    unsafe { InitializeProcThreadAttributeList(list, 6, 0, &mut size) };
    let g = bytes_of(&GPU);
    for (label, len) in [
        ("16 bytes (Chrome)", 16usize),
        ("8 bytes", 8),
        ("4 bytes", 4),
        ("12 bytes", 12),
        ("24 bytes", 24),
        ("0 bytes", 0),
    ] {
        let mut v = [0u8; 24];
        v[..16].copy_from_slice(&g);
        mark_error();
        // SAFETY: the value buffer holds 24 bytes.
        let r = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
                v.as_ptr().cast(),
                len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        say!(t, "UpdateProcThreadAttribute(mitigation policy, {label}) = {r} error {}", err_text().as_str());
        if r != 0 {
            // a second update of the same attribute is refused: start the next list
            // SAFETY: the list is initialised.
            unsafe { DeleteProcThreadAttributeList(list) };
            // SAFETY: as above.
            unsafe { InitializeProcThreadAttributeList(list, 6, 0, &mut size) };
        }
    }
    for (label, attr, len) in [
        ("child process policy, 4 bytes", PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, 4usize),
        ("child process policy, 8 bytes", PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, 8),
        ("child process policy, 2 bytes", PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, 2),
        ("0x2001A, 4 bytes", PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER, 4),
        ("0x2001A, 8 bytes", PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER, 8),
        ("0x2001A, 2 bytes", PROC_THREAD_ATTRIBUTE_COMPONENT_FILTER, 2),
        ("job list, 8 bytes", PROC_THREAD_ATTRIBUTE_JOB_LIST, 8),
    ] {
        let v = [1u64, 0];
        mark_error();
        // SAFETY: the value buffer holds 16 bytes.
        let r = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                attr,
                v.as_ptr().cast(),
                len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        say!(t, "UpdateProcThreadAttribute({label}) = {r} error {}", err_text().as_str());
        if r != 0 {
            // SAFETY: the list is initialised.
            unsafe { DeleteProcThreadAttributeList(list) };
            // SAFETY: as above.
            unsafe { InitializeProcThreadAttributeList(list, 6, 0, &mut size) };
        }
    }
    // SAFETY: the list is initialised.
    unsafe { DeleteProcThreadAttributeList(list) };
    // children
    let gpu = bytes_of(&GPU);
    let storage = bytes_of(&STORAGE);
    let renderer = bytes_of(&RENDERER);
    // the pipe's write end goes to the child through the handle list
    for (label, m, cp, cf) in [
        ("no mitigation attribute", None, None, None),
        ("GPU words", Some(&gpu[..]), Some(1u32), Some(1u32)),
        ("storage-service words (win32k disabled)", Some(&storage[..]), Some(1), Some(1)),
        ("renderer words (Microsoft-signed only)", Some(&renderer[..]), Some(1), Some(1)),
        ("child process policy alone", None, Some(1), None),
        ("0x2001A alone", None, None, Some(1)),
    ] {
        launch(t, label, m, cp, cf);
    }
}
