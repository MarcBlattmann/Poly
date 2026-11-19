//! Which proxies a blanket set with CoSetProxyBlanket applies to: the pointer it was set on, other interfaces of the same
//! object obtained afterwards or before (QueryInterface), another object of the same server, another class object of the same
//! class, a copy made with CoCopyProxy before and after. Each proxy's blanket is read back with CoQueryProxyBlanket after
//! changing the impersonation level on one of them.
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

// ===================================================================== COM helpers (spec-win-com-oop)
// Declarations from Microsoft Learn (combaseapi.h, objbase.h, unknwn.h, winerror.h, winreg.h); the helpers are this
// test's own. A test is one image: the process started with `--child srv <tag> <flags>` is the test's own COM
// server (it registers a class factory and serves it until a quit file appears), the other one is the client.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering::SeqCst};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
    GUID { d1, d2, d3, d4 }
}

const IID_IUnknown: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IClassFactory: GUID = guid(1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMarshal: GUID = guid(3, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDispatch: GUID = guid(0x20400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IExternalConnection: GUID = guid(0x19, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IStdMarshalInfo: GUID = guid(0x18, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IServerSecurity: GUID = guid(0x13E, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IClientSecurity: GUID = guid(0x13D, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// An interface id nothing implements (made up for these tests).
const IID_Unknown1: GUID = guid(0xB2A0_0001, 0x7001, 0x4001, [0x80, 0x01, 0x70, 0x6F, 0x6C, 0x79, 0x77, 0x73]);

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_POINTER: i32 = 0x8000_4003_u32 as i32;

const CLSCTX_INPROC_SERVER: u32 = 1;
const CLSCTX_INPROC_HANDLER: u32 = 2;
const CLSCTX_LOCAL_SERVER: u32 = 4;
const CLSCTX_REMOTE_SERVER: u32 = 0x10;
const CLSCTX_ALL: u32 = 0x17;
const CLSCTX_ENABLE_CODE_DOWNLOAD: u32 = 0x2000;
const CLSCTX_NO_CODE_DOWNLOAD: u32 = 0x400;
const CLSCTX_ACTIVATE_64_BIT_SERVER: u32 = 0x80000;

const REGCLS_SINGLEUSE: u32 = 0;
const REGCLS_MULTIPLEUSE: u32 = 1;
const REGCLS_MULTI_SEPARATE: u32 = 2;
const REGCLS_SUSPENDED: u32 = 4;
const REGCLS_SURROGATE: u32 = 8;

const COINIT_MULTITHREADED: u32 = 0;
const COINIT_APARTMENTTHREADED: u32 = 2;

#[repr(C)]
struct MULTI_QI {
    iid: *const GUID,
    itf: *mut c_void,
    hr: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HANDLE,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt_x: i32,
    pt_y: i32,
    private: u32,
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        clsid: *const GUID,
        outer: *mut c_void,
        ctx: u32,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> i32;
    fn CoCreateInstanceEx(
        clsid: *const GUID,
        outer: *mut c_void,
        ctx: u32,
        server: *mut c_void,
        n: u32,
        qi: *mut MULTI_QI,
    ) -> i32;
    fn CoGetClassObject(
        clsid: *const GUID,
        ctx: u32,
        server: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> i32;
    fn CoRegisterClassObject(clsid: *const GUID, unk: *mut c_void, ctx: u32, flags: u32, cookie: *mut u32) -> i32;
    fn CoRevokeClassObject(cookie: u32) -> i32;
    fn CoResumeClassObjects() -> i32;
    fn CoSuspendClassObjects() -> i32;
    fn CoAddRefServerProcess() -> u32;
    fn CoReleaseServerProcess() -> u32;
    fn CoTaskMemAlloc(n: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
    fn StringFromGUID2(g: *const GUID, buf: *mut u16, n: i32) -> i32;
    fn CoDisconnectObject(unk: *mut c_void, reserved: u32) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PeekMessageW(msg: *mut MSG, hwnd: HANDLE, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: HANDLE,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        sa: *const c_void,
        out: *mut HANDLE,
        disp: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(key: HANDLE, sub: *const u16, options: u32, access: u32, out: *mut HANDLE) -> i32;
    fn RegSetValueExW(key: HANDLE, name: *const u16, reserved: u32, ty: u32, data: *const u8, n: u32) -> i32;
    fn RegDeleteKeyW(key: HANDLE, sub: *const u16) -> i32;
    fn RegCloseKey(key: HANDLE) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const c_void,
        disp: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn CreateThread(
        sa: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> HANDLE;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
}

const HKLM: HANDLE = 0x8000_0002_u32 as usize as HANDLE;
const HKCU: HANDLE = 0x8000_0001_u32 as usize as HANDLE;
const KEY_ALL_ACCESS_ISH: u32 = 0xF003F;
const REG_SZ: u32 = 1;

// ---------------------------------------------------------------- small text helpers

fn hex8(v: i32) -> Buf<12> {
    let mut b = Buf::<12>::new();
    let _ = write!(b, "{:#010x}", v as u32);
    b
}

fn guid_text(g: &GUID) -> Buf<48> {
    let mut w = [0u16; 48];
    // SAFETY: the buffer holds 48 units; a GUID text needs 39.
    let n = unsafe { StringFromGUID2(g, w.as_mut_ptr(), 48) };
    let mut b = Buf::<48>::new();
    for &u in &w[..(n.max(1) as usize - 1)] {
        let _ = write!(b, "{}", u as u8 as char);
    }
    b
}

/// A name for the interface ids the tests meet; others by their full text.
fn iid_name(g: &GUID) -> Buf<48> {
    let mut b = Buf::<48>::new();
    let n = if *g == IID_IUnknown {
        "IUnknown"
    } else if *g == IID_IClassFactory {
        "IClassFactory"
    } else if *g == IID_IMarshal {
        "IMarshal"
    } else if *g == IID_IDispatch {
        "IDispatch"
    } else if *g == IID_IExternalConnection {
        "IExternalConnection"
    } else if *g == IID_IStdMarshalInfo {
        "IStdMarshalInfo"
    } else if *g == IID_IServerSecurity {
        "IServerSecurity"
    } else if *g == IID_IClientSecurity {
        "IClientSecurity"
    } else if *g == IID_Unknown1 {
        "IID_Unknown1"
    } else {
        return guid_text(g);
    };
    let _ = write!(b, "{n}");
    b
}

fn hexval(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` as a GUID (the text is the tests' own).
fn guid_from_str(s: &str) -> GUID {
    let b: &[u8] = s.as_bytes();
    let h = |i: usize, n: usize| -> u64 {
        let mut v = 0u64;
        for k in 0..n {
            v = v << 4 | u64::from(hexval(b[i + k]));
        }
        v
    };
    let mut d4 = [0u8; 8];
    d4[0] = h(20, 2) as u8;
    d4[1] = h(22, 2) as u8;
    for k in 0..6 {
        d4[2 + k] = h(25 + 2 * k, 2) as u8;
    }
    GUID { d1: h(1, 8) as u32, d2: h(10, 4) as u16, d3: h(15, 4) as u16, d4 }
}

// ---------------------------------------------------------------- files beside the image

/// A wide path of a file beside the executable: `<dir of the image>\<name>`.
fn beside(name: &str) -> [u16; 700] {
    let mut p = [0u16; 700];
    // SAFETY: the buffer holds 700 units; a NULL module is the executable.
    let mut n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 600) } as usize;
    while n > 0 && p[n - 1] != u16::from(b'\\') {
        n -= 1;
    }
    for b in name.bytes() {
        p[n] = u16::from(b);
        n += 1;
    }
    p[n] = 0;
    p
}

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_APPEND_DATA: u32 = 4;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const CREATE_ALWAYS: u32 = 2;

/// Append `line` and a newline to a file (made if missing); readable and writable by others meanwhile.
fn append_line(path: &[u16], line: &str) {
    // SAFETY: `path` is NUL-terminated; the handle is closed here.
    unsafe {
        let h = CreateFileW(
            path.as_ptr(),
            FILE_APPEND_DATA,
            7,
            core::ptr::null(),
            OPEN_ALWAYS,
            0x80,
            core::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return;
        }
        let mut put = 0u32;
        let mut buf = [0u8; 400];
        let n = line.len().min(398);
        buf[..n].copy_from_slice(&line.as_bytes()[..n]);
        buf[n] = b'\n';
        WriteFile(h, buf.as_ptr(), (n + 1) as u32, &mut put, core::ptr::null_mut());
        CloseHandle(h);
    }
}

fn make_file(path: &[u16]) {
    // SAFETY: `path` is NUL-terminated; the handle is closed here.
    unsafe {
        let h =
            CreateFileW(path.as_ptr(), 0x4000_0000, 7, core::ptr::null(), CREATE_ALWAYS, 0x80, core::ptr::null_mut());
        close(h);
    }
}

fn file_exists(path: &[u16]) -> bool {
    // SAFETY: `path` is NUL-terminated.
    unsafe { GetFileAttributesW(path.as_ptr()) != 0xFFFF_FFFF }
}

fn delete_file(path: &[u16]) {
    // SAFETY: `path` is NUL-terminated.
    unsafe { DeleteFileW(path.as_ptr()) };
}

/// The contents of a file (up to 12 KB; empty when missing).
struct Text {
    b: [u8; 12288],
    n: usize,
}

impl Text {
    fn s(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

fn read_text(path: &[u16]) -> Text {
    let mut t = Text { b: [0; 12288], n: 0 };
    // SAFETY: `path` is NUL-terminated; reads into `t.b`; the handle is closed here.
    unsafe {
        let h =
            CreateFileW(path.as_ptr(), GENERIC_READ, 7, core::ptr::null(), OPEN_EXISTING, 0x80, core::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return t;
        }
        let mut got = 0u32;
        ReadFile(h, t.b.as_mut_ptr(), 12288, &mut got, core::ptr::null_mut());
        t.n = got as usize;
        CloseHandle(h);
    }
    t
}

/// The log file of the server with this tag.
fn srv_log_path(tag: &str) -> [u16; 700] {
    let mut n = Buf::<80>::new();
    let _ = write!(n, "polyws-srv-{tag}.log");
    beside(n.as_str())
}

fn quit_path(tag: &str) -> [u16; 700] {
    let mut n = Buf::<80>::new();
    let _ = write!(n, "polyws-quit-{tag}");
    beside(n.as_str())
}

/// Wait up to `ms` for the log of `tag` to contain `needle`.
fn wait_log(tag: &str, needle: &str, ms: u32) -> bool {
    let p = srv_log_path(tag);
    let mut waited = 0;
    loop {
        if read_text(&p).s().contains(needle) {
            return true;
        }
        if waited >= ms {
            return false;
        }
        // SAFETY: a plain sleep.
        unsafe { Sleep(10) };
        waited += 10;
    }
}

fn count_lines(text: &str, needle: &str) -> usize {
    text.split('\n').filter(|l| l.contains(needle)).count()
}

/// The number of different server processes that wrote `start pid=<n>` lines.
fn distinct_pids(text: &str) -> usize {
    let mut seen = [0u32; 16];
    let mut k = 0;
    for l in text.split('\n') {
        if let Some(r) = l.strip_prefix("start pid=") {
            let v: u32 = r.trim().parse().unwrap_or(0);
            if !seen[..k].contains(&v) && k < 16 {
                seen[k] = v;
                k += 1;
            }
        }
    }
    k
}

/// Print the server's log lines (with the process ids taken out) as `srv: ...` observations.
fn print_log(t: &mut Test, tag: &str) {
    let txt = read_text(&srv_log_path(tag));
    for l in txt.s().split('\n') {
        if l.is_empty() {
            continue;
        }
        if l.starts_with("start pid=") {
            say!(t, "srv: start");
        } else if l.starts_with("lock ") || l.starts_with("obj released") || l.starts_with("register ") {
            // the lock counts COM makes while it activates are not part of the contract
        } else if l.starts_with("obj qi") || l.starts_with("factory qi") {
            // the interfaces COM asks for while it marshals vary and are not part of the contract
        } else {
            say!(t, "srv: {l}");
        }
    }
}

fn cleanup_tag(tag: &str) {
    delete_file(&srv_log_path(tag));
    delete_file(&quit_path(tag));
    let mut rv = Buf::<80>::new();
    let _ = write!(rv, "polyws-revoke-{tag}");
    delete_file(&beside(rv.as_str()));
}

/// Ask the server with this tag to call CoRevokeClassObject (it keeps running).
fn ask_revoke(tag: &str) {
    let mut rv = Buf::<80>::new();
    let _ = write!(rv, "polyws-revoke-{tag}");
    make_file(&beside(rv.as_str()));
}

/// Ask the server with this tag to leave (and give it up to `ms` to log `exit`).
fn stop_server(tag: &str, ms: u32) -> bool {
    make_file(&quit_path(tag));
    wait_log(tag, "exit", ms)
}

// ---------------------------------------------------------------- registry helpers

fn wsz(s: &str) -> [u16; 400] {
    let mut w = [0u16; 400];
    for (i, c) in s.chars().enumerate().take(399) {
        w[i] = c as u32 as u16;
    }
    w
}

/// Create (or open) `HKLM\SOFTWARE\Classes\<sub>` (or HKCU) and set its default value to `value` (None: leave it).
fn reg_put(root: HANDLE, sub: &str, name: Option<&str>, value: &str) -> i32 {
    let mut path = Buf::<300>::new();
    let _ = write!(path, "SOFTWARE\\Classes\\{sub}");
    let p = wsz(path.as_str());
    let mut key: HANDLE = core::ptr::null_mut();
    let mut disp = 0u32;
    // SAFETY: live out pointers; the strings are NUL-terminated.
    let r = unsafe {
        RegCreateKeyExW(
            root,
            p.as_ptr(),
            0,
            core::ptr::null(),
            0,
            KEY_ALL_ACCESS_ISH,
            core::ptr::null(),
            &mut key,
            &mut disp,
        )
    };
    if r != 0 {
        return r;
    }
    let v = wsz(value);
    let n = name.map(wsz);
    let mut len = 0;
    while v[len] != 0 {
        len += 1;
    }
    // SAFETY: `v` holds `len` units and a NUL; the key was opened above and is closed here.
    let r = unsafe {
        let r = RegSetValueExW(
            key,
            n.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            0,
            REG_SZ,
            v.as_ptr().cast(),
            ((len + 1) * 2) as u32,
        );
        RegCloseKey(key);
        r
    };
    r
}

fn reg_del(root: HANDLE, sub: &str) -> i32 {
    let mut path = Buf::<300>::new();
    let _ = write!(path, "SOFTWARE\\Classes\\{sub}");
    let p = wsz(path.as_str());
    // SAFETY: NUL-terminated key path.
    unsafe { RegDeleteKeyW(root, p.as_ptr()) }
}

/// Register a class the tests own: `CLSID\{clsid}` with `LocalServer32` = `command` (and an AppID when given).
fn reg_class_local(root: HANDLE, clsid: &str, name: &str, command: &str, appid: Option<&str>) {
    let mut k = Buf::<120>::new();
    let _ = write!(k, "CLSID\\{clsid}");
    reg_put(root, k.as_str(), None, name);
    if let Some(a) = appid {
        reg_put(root, k.as_str(), Some("AppID"), a);
    }
    let mut k2 = Buf::<140>::new();
    let _ = write!(k2, "CLSID\\{clsid}\\LocalServer32");
    reg_put(root, k2.as_str(), None, command);
}

fn unreg_class(root: HANDLE, clsid: &str) {
    for suffix in ["\\LocalServer32", "\\InprocServer32", ""] {
        let mut k = Buf::<140>::new();
        let _ = write!(k, "CLSID\\{clsid}{suffix}");
        reg_del(root, k.as_str());
    }
}

/// The command line of a test's own server: `"<exe>" --child srv <tag> <flags>`.
fn server_command(tag: &str, clsid: &str, flags: &str) -> Buf<500> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut b = Buf::<500>::new();
    let _ = write!(b, "\"");
    for &u in &exe[..n] {
        let _ = write!(b, "{}", u as u8 as char);
    }
    let _ = write!(b, "\" --child srv {tag} clsid={clsid} {flags}");
    b
}

// ---------------------------------------------------------------- the test's own COM objects

#[repr(C)]
struct IUnknownVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    addref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IClassFactoryVtbl {
    base: IUnknownVtbl,
    create: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> i32,
    lock: unsafe extern "system" fn(*mut c_void, BOOL) -> i32,
}

/// A COM object: the vtable pointer first, then its reference count.
#[repr(C)]
struct Obj {
    vtbl: *const IUnknownVtbl,
    refs: AtomicU32,
}

#[repr(C)]
struct Factory {
    vtbl: *const IClassFactoryVtbl,
    refs: AtomicU32,
}

// SAFETY: the vtable pointer is a shared static; the count is atomic.
unsafe impl Sync for Factory {}

struct Sync_<T>(UnsafeCell<T>);
// SAFETY: the cells below are written before any other thread runs, or only through atomics.
unsafe impl<T> Sync for Sync_<T> {}

/// The server's log file (set once at the start of the server process).
static SRV_LOG: Sync_<[u16; 700]> = Sync_(UnsafeCell::new([0; 700]));
static SRV_CLSID: Sync_<GUID> = Sync_(UnsafeCell::new(guid(0, 0, 0, [0; 8])));
static LIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);
static LOCKS: AtomicU32 = AtomicU32::new(0);
static CREATED: AtomicU32 = AtomicU32::new(0);
/// What the server's objects do on their last Release: 0 nothing, 1 call CoReleaseServerProcess.
static OBJ_RELEASE_SERVER: AtomicU32 = AtomicU32::new(0);
/// Whether the factory call `CoAddRefServerProcess` per created object.
static OBJ_ADDREF_SERVER: AtomicU32 = AtomicU32::new(0);

fn srv_say(args: core::fmt::Arguments) {
    let mut b = Buf::<380>::new();
    let _ = b.write_fmt(args);
    // SAFETY: SRV_LOG is set before the factory exists and never changes afterwards.
    let p = unsafe { &*SRV_LOG.0.get() };
    append_line(p, b.as_str());
}

/// Log the interface probes of COM only when the server was started with the flag `qilog` (they are many and vary).
static QILOG: AtomicU32 = AtomicU32::new(0);

macro_rules! qsay {
    ($($a:tt)*) => { if QILOG.load(SeqCst) != 0 { srv_say(format_args!($($a)*)) } };
}

macro_rules! ssay {
    ($($a:tt)*) => { srv_say(format_args!($($a)*)) };
}

unsafe extern "system" fn obj_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32 {
    // SAFETY: COM passes valid pointers.
    unsafe {
        let g = &*iid;
        if *g == IID_IUnknown {
            *out = this;
            obj_addref(this);
            qsay!("obj qi IUnknown S_OK");
            S_OK
        } else {
            *out = core::ptr::null_mut();
            qsay!("obj qi {} E_NOINTERFACE", iid_name(g).as_str());
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn obj_addref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is one of our objects.
    unsafe { (*(this as *mut Obj)).refs.fetch_add(1, SeqCst) + 1 }
}

unsafe extern "system" fn obj_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is one of our objects; the last release frees it.
    unsafe {
        let n = (*(this as *mut Obj)).refs.fetch_sub(1, SeqCst) - 1;
        if n == 0 {
            CoTaskMemFree(this);
            let left = LIVE_OBJECTS.fetch_sub(1, SeqCst) - 1;
            ssay!("obj released live={left}");
            if OBJ_RELEASE_SERVER.load(SeqCst) != 0 {
                let c = CoReleaseServerProcess();
                ssay!("CoReleaseServerProcess -> {c}");
            }
        }
        n
    }
}

static OBJ_VTBL: IUnknownVtbl = IUnknownVtbl { qi: obj_qi, addref: obj_addref, release: obj_release };

/// A new object with one reference.
fn new_obj() -> *mut Obj {
    // SAFETY: the memory is ours; written before use.
    unsafe {
        let p = CoTaskMemAlloc(core::mem::size_of::<Obj>()) as *mut Obj;
        p.write(Obj { vtbl: &OBJ_VTBL, refs: AtomicU32::new(1) });
        LIVE_OBJECTS.fetch_add(1, SeqCst);
        CREATED.fetch_add(1, SeqCst);
        p
    }
}

unsafe extern "system" fn fac_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32 {
    // SAFETY: COM passes valid pointers; `this` is the static factory.
    unsafe {
        let g = &*iid;
        if *g == IID_IUnknown || *g == IID_IClassFactory {
            *out = this;
            fac_addref(this);
            qsay!("factory qi {} S_OK", iid_name(g).as_str());
            S_OK
        } else {
            *out = core::ptr::null_mut();
            qsay!("factory qi {} E_NOINTERFACE", iid_name(g).as_str());
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn fac_addref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is the static factory.
    unsafe { (*(this as *mut Factory)).refs.fetch_add(1, SeqCst) + 1 }
}

unsafe extern "system" fn fac_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is the static factory.
    unsafe { (*(this as *mut Factory)).refs.fetch_sub(1, SeqCst) - 1 }
}

unsafe extern "system" fn fac_create(
    _this: *mut c_void,
    outer: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> i32 {
    // SAFETY: COM passes valid pointers.
    unsafe {
        let g = &*iid;
        CREATE_TID.store(tid(), SeqCst);
        CREATE_CALLS.fetch_add(1, SeqCst);
        ssay!("create outer={} iid={}", if outer.is_null() { "null" } else { "set" }, iid_name(g).as_str());
        if !outer.is_null() {
            *out = core::ptr::null_mut();
            return 0x8004_0110_u32 as i32;
        }
        on_create();
        let o = new_obj();
        if OBJ_ADDREF_SERVER.load(SeqCst) != 0 {
            let c = CoAddRefServerProcess();
            ssay!("CoAddRefServerProcess -> {c}");
        }
        let hr = obj_qi(o as *mut c_void, iid, out);
        obj_release(o as *mut c_void);
        ssay!("create -> {}", hex8(hr).as_str());
        hr
    }
}

unsafe extern "system" fn fac_lock(_this: *mut c_void, lock: BOOL) -> i32 {
    let n = if lock != 0 { LOCKS.fetch_add(1, SeqCst) + 1 } else { LOCKS.fetch_sub(1, SeqCst) - 1 };
    ssay!("lock {} -> {}", if lock != 0 { "1" } else { "0" }, n);
    on_lock();
    S_OK
}

static FAC_VTBL: IClassFactoryVtbl = IClassFactoryVtbl {
    base: IUnknownVtbl { qi: fac_qi, addref: fac_addref, release: fac_release },
    create: fac_create,
    lock: fac_lock,
};
static FACTORY: Factory = Factory { vtbl: &FAC_VTBL, refs: AtomicU32::new(1) };

fn factory_ptr() -> *mut c_void {
    &FACTORY as *const Factory as *mut c_void
}

/// A bare object a test registers or marshals itself (not a class factory).
fn new_plain() -> *mut c_void {
    new_obj() as *mut c_void
}

// ---- calls through an interface pointer

fn unk_release(p: *mut c_void) -> u32 {
    if p.is_null() {
        return 0;
    }
    // SAFETY: `p` is a COM interface pointer the test holds.
    unsafe { ((*(*(p as *mut *const IUnknownVtbl))).release)(p) }
}

fn unk_addref(p: *mut c_void) -> u32 {
    // SAFETY: `p` is a COM interface pointer the test holds.
    unsafe { ((*(*(p as *mut *const IUnknownVtbl))).addref)(p) }
}

fn unk_qi(p: *mut c_void, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: `p` is a COM interface pointer the test holds.
    let hr = unsafe { ((*(*(p as *mut *const IUnknownVtbl))).qi)(p, iid, &mut out) };
    (hr, out)
}

/// Whether `p` is a proxy (it answers IMarshal; our own objects do not).
fn is_proxy(p: *mut c_void) -> bool {
    let (hr, o) = unk_qi(p, &IID_IMarshal);
    if hr == S_OK {
        unk_release(o);
        true
    } else {
        false
    }
}

/// IClassFactory::CreateInstance through a factory pointer.
fn fac_create_instance(f: *mut c_void, outer: *mut c_void, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: `f` is an IClassFactory pointer the test holds.
    let hr = unsafe { ((*(*(f as *mut *const IClassFactoryVtbl))).create)(f, outer, iid, &mut out) };
    (hr, out)
}

fn fac_lock_server(f: *mut c_void, lock: bool) -> i32 {
    // SAFETY: `f` is an IClassFactory pointer the test holds.
    unsafe { ((*(*(f as *mut *const IClassFactoryVtbl))).lock)(f, i32::from(lock)) }
}

fn co_init(mta: bool) -> i32 {
    // SAFETY: plain call.
    unsafe { CoInitializeEx(core::ptr::null_mut(), if mta { COINIT_MULTITHREADED } else { COINIT_APARTMENTTHREADED }) }
}

fn create_inst(clsid: &GUID, ctx: u32, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = 0x1111 as *mut c_void;
    // SAFETY: live out pointer.
    let hr = unsafe { CoCreateInstance(clsid, core::ptr::null_mut(), ctx, iid, &mut out) };
    (hr, out)
}

fn get_class_obj(clsid: &GUID, ctx: u32, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = 0x1111 as *mut c_void;
    // SAFETY: live out pointer.
    let hr = unsafe { CoGetClassObject(clsid, ctx, core::ptr::null_mut(), iid, &mut out) };
    (hr, out)
}

fn reg_class_obj(clsid: &GUID, unk: *mut c_void, ctx: u32, flags: u32) -> (i32, u32) {
    let mut cookie = 0xCAFEu32;
    // SAFETY: live out pointer.
    let hr = unsafe { CoRegisterClassObject(clsid, unk, ctx, flags, &mut cookie) };
    (hr, cookie)
}

fn revoke(cookie: u32) -> i32 {
    // SAFETY: plain call.
    unsafe { CoRevokeClassObject(cookie) }
}

/// Pump the window messages of this STA thread for `ms`.
fn pump(ms: u32) {
    let mut waited = 0u32;
    while waited < ms {
        let mut m = MSG {
            hwnd: core::ptr::null_mut(),
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            pt_x: 0,
            pt_y: 0,
            private: 0,
        };
        // SAFETY: live MSG; standard message loop calls.
        unsafe {
            while PeekMessageW(&mut m, core::ptr::null_mut(), 0, 0, 1) != 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            MsgWaitForMultipleObjects(0, core::ptr::null(), 0, 10, 0x4FF);
        }
        waited += 10;
    }
}

fn has_flag(flags: &str, word: &str) -> bool {
    flags.split(' ').any(|w| w == word)
}

/// `key=value` among the words of `flags`.
fn flag_val<'a>(flags: &'a str, key: &str) -> Option<&'a str> {
    for w in flags.split(' ') {
        if let Some((k, v)) = w.split_once('=') {
            if k == key {
                return Some(v);
            }
        }
    }
    None
}

fn flag_num(flags: &str, key: &str, default: u32) -> u32 {
    flag_val(flags, key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

// ---------------------------------------------------------------- the server process

/// The test's own COM server: `--child srv <tag> clsid={...} <flags>`.
///   sta | mta          the apartment of the main thread (default mta)
///   single | multi | sep   REGCLS_SINGLEUSE | MULTIPLEUSE (default) | MULTI_SEPARATE; `suspend` adds REGCLS_SUSPENDED
///   resume_after=<ms>  CoResumeClassObjects after this many ms (with `suspend`; default: at once); `noresume` never
///   noregister         exits without registering anything
///   delay=<ms>         sleeps this long before registering
///   addref             every created object does CoAddRefServerProcess, its last Release CoReleaseServerProcess
///   quit_on_zero       the server leaves when CoReleaseServerProcess returns 0 (after `addref`)
///   life=<ms>          leaves by itself after this long (default 8000)
/// The server leaves when the file `polyws-quit-<tag>` exists; its events go to `polyws-srv-<tag>.log`.
fn server(args: &str) -> u32 {
    let mut words = args.split(' ');
    let _srv = words.next();
    let tag = words.next().unwrap_or("x");
    let rest = args.splitn(3, ' ').nth(2).unwrap_or("");
    let clsid = guid_from_str(flag_val(rest, "clsid").unwrap_or("{00000000-0000-0000-0000-000000000000}"));
    // SAFETY: nothing else runs yet.
    unsafe {
        *SRV_LOG.0.get() = srv_log_path(tag);
        *SRV_CLSID.0.get() = clsid;
    }
    let mut b = Buf::<40>::new();
    // SAFETY: plain call.
    let _ = write!(b, "start pid={}", unsafe { GetCurrentProcessId() });
    ssay!("{}", b.as_str());
    let delay = flag_num(rest, "delay", 0);
    if delay != 0 {
        // SAFETY: a plain sleep.
        unsafe { Sleep(delay) };
    }
    if has_flag(rest, "noregister") {
        ssay!("exit (noregister)");
        return 0;
    }
    let sta = has_flag(rest, "sta");
    if has_flag(rest, "qilog") {
        QILOG.store(1, SeqCst);
    }
    let hr = co_init(!sta);
    ssay!("init {} {}", if sta { "sta" } else { "mta" }, hex8(hr).as_str());
    sec_server_init(rest);
    if has_flag(rest, "initonly") {
        // SAFETY: plain call.
        unsafe { CoUninitialize() };
        ssay!("exit (initonly)");
        return 0;
    }
    if has_flag(rest, "addref") {
        OBJ_ADDREF_SERVER.store(1, SeqCst);
        OBJ_RELEASE_SERVER.store(1, SeqCst);
    }
    let mut flags = if has_flag(rest, "single") {
        REGCLS_SINGLEUSE
    } else if has_flag(rest, "sep") {
        REGCLS_MULTI_SEPARATE
    } else {
        REGCLS_MULTIPLEUSE
    };
    let suspended = has_flag(rest, "suspend");
    if suspended {
        flags |= REGCLS_SUSPENDED;
    }
    let (hr, cookie) = reg_class_obj(&clsid, factory_ptr(), CLSCTX_LOCAL_SERVER, flags);
    ssay!(
        "register flags={flags} -> {} cookie_nonzero={}",
        hex8(hr).as_str(),
        u32::from(cookie != 0 && cookie != 0xCAFE)
    );
    let life = flag_num(rest, "life", 8000);
    let resume_after = flag_num(rest, "resume_after", 0);
    let mut resumed = !suspended;
    if suspended && resume_after == 0 && !has_flag(rest, "noresume") {
        // SAFETY: plain call.
        let hr = unsafe { CoResumeClassObjects() };
        ssay!("resume -> {}", hex8(hr).as_str());
        resumed = true;
    }
    let quit = quit_path(tag);
    let mut rv = Buf::<80>::new();
    let _ = write!(rv, "polyws-revoke-{tag}");
    let revoke_file = beside(rv.as_str());
    let mut revoked = false;
    let mut waited = 0u32;
    loop {
        if sta {
            pump(10);
        } else {
            // SAFETY: a plain sleep.
            unsafe { Sleep(10) };
        }
        waited += 10;
        if !resumed && !has_flag(rest, "noresume") && waited >= resume_after {
            // SAFETY: plain call.
            let hr = unsafe { CoResumeClassObjects() };
            ssay!("resume -> {}", hex8(hr).as_str());
            resumed = true;
        }
        if !revoked && file_exists(&revoke_file) {
            let hr = revoke(cookie);
            ssay!("revoke on request -> {}", hex8(hr).as_str());
            revoked = true;
        }
        if file_exists(&quit) || waited >= life {
            break;
        }
        if has_flag(rest, "quit_on_zero") && CREATED.load(SeqCst) > 0 && LIVE_OBJECTS.load(SeqCst) == 0 {
            ssay!("last object gone");
            break;
        }
    }
    if !revoked {
        let hr = revoke(cookie);
        ssay!("revoke -> {}", hex8(hr).as_str());
    }
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
    ssay!("exit");
    0
}

// ---------------------------------------------------------------- threads

/// Run `f(arg)` on a new thread; its handle.
fn spawn(f: unsafe extern "system" fn(*mut c_void) -> u32, arg: usize) -> HANDLE {
    let mut tid = 0u32;
    // SAFETY: a plain thread start; the start routine is an `extern "system"` function of this test.
    unsafe { CreateThread(core::ptr::null(), 0, f, arg as *mut c_void, 0, &mut tid) }
}

/// Wait for a thread (`ms` at most) and close its handle; whether it ended, and its exit code.
fn join(h: HANDLE, ms: u32) -> (bool, u32) {
    let mut code = 0u32;
    // SAFETY: the handle of a thread this test started.
    let ended = unsafe {
        let ended = WaitForSingleObject(h, ms) == 0;
        GetExitCodeThread(h, &mut code);
        ended
    };
    close(h);
    (ended, code)
}

fn tid() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetCurrentThreadId() }
}

/// The thread that ran the last `CreateInstance` of the test's own factory, and how many calls there were.
static CREATE_TID: AtomicU32 = AtomicU32::new(0);
static CREATE_CALLS: AtomicU32 = AtomicU32::new(0);

/// A class id text of this run: `base` with its second group replaced by the low 16 bits of the process id, so a
/// class whose activation was left pending by an earlier run cannot hold up this one.
fn uniq(base: &str) -> Buf<40> {
    let mut b = Buf::<40>::new();
    // SAFETY: no arguments.
    let pid = unsafe { GetCurrentProcessId() };
    for (i, c) in base.chars().enumerate() {
        if (10..14).contains(&i) {
            let d = (pid >> (4 * (13 - i))) & 0xf;
            let _ = write!(b, "{:x}", d);
        } else {
            let _ = write!(b, "{c}");
        }
    }
    b
}

// ===================================================================== a service of the test's own (AppID LocalService)
// Declarations from Microsoft Learn (winsvc.h). The service is this same image started by the service control manager with
// `--child svc <tag> clsid={...} <flags>`: it runs the test's COM server (see `server`) inside the service process and logs the
// arguments the service control manager gave it.

#[repr(C)]
struct SERVICE_TABLE_ENTRYW {
    name: *const u16,
    proc_: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SERVICE_STATUS {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit: u32,
    specific_exit: u32,
    check_point: u32,
    wait_hint: u32,
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenSCManagerW(machine: *const u16, db: *const u16, access: u32) -> HANDLE;
    fn CreateServiceW(
        scm: HANDLE,
        name: *const u16,
        display: *const u16,
        access: u32,
        ty: u32,
        start: u32,
        error: u32,
        binary: *const u16,
        group: *const u16,
        tag: *mut u32,
        deps: *const u16,
        account: *const u16,
        password: *const u16,
    ) -> HANDLE;
    fn OpenServiceW(scm: HANDLE, name: *const u16, access: u32) -> HANDLE;
    fn DeleteService(svc: HANDLE) -> BOOL;
    fn CloseServiceHandle(h: HANDLE) -> BOOL;
    fn ControlService(svc: HANDLE, control: u32, status: *mut SERVICE_STATUS) -> BOOL;
    fn QueryServiceStatus(svc: HANDLE, status: *mut SERVICE_STATUS) -> BOOL;
    fn StartServiceCtrlDispatcherW(table: *const SERVICE_TABLE_ENTRYW) -> BOOL;
    fn RegisterServiceCtrlHandlerExW(
        name: *const u16,
        handler: unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32,
        ctx: *mut c_void,
    ) -> HANDLE;
    fn SetServiceStatus(h: HANDLE, status: *const SERVICE_STATUS) -> BOOL;
}

const SC_MANAGER_ALL_ACCESS: u32 = 0xF003F;
const SERVICE_ALL_ACCESS: u32 = 0xF01FF;
const SERVICE_WIN32_OWN_PROCESS: u32 = 0x10;
const SERVICE_DEMAND_START: u32 = 3;
const SERVICE_ERROR_NORMAL: u32 = 1;
const SERVICE_CONTROL_STOP: u32 = 1;
const SERVICE_STOPPED: u32 = 1;
const SERVICE_START_PENDING: u32 = 2;
const SERVICE_RUNNING: u32 = 4;
const SERVICE_ACCEPT_STOP: u32 = 1;

/// Create a demand-start service of its own (LocalSystem) running `command`; the handles of the manager and the service.
fn svc_create(name: &str, command: &str) -> (HANDLE, HANDLE, u32) {
    let n = wsz(name);
    let c = wsz(command);
    // SAFETY: NUL-terminated strings, live out values.
    unsafe {
        let scm = OpenSCManagerW(core::ptr::null(), core::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return (scm, core::ptr::null_mut(), last_error());
        }
        let s = CreateServiceW(
            scm,
            n.as_ptr(),
            n.as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            c.as_ptr(),
            core::ptr::null(),
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
        );
        let e = if s.is_null() { last_error() } else { 0 };
        (scm, s, e)
    }
}

/// The state of a service (SERVICE_STOPPED 1, START_PENDING 2, RUNNING 4, ...), 0 when it cannot be asked.
fn svc_state(svc: HANDLE) -> u32 {
    let mut st = SERVICE_STATUS {
        service_type: 0,
        current_state: 0,
        controls_accepted: 0,
        win32_exit: 0,
        specific_exit: 0,
        check_point: 0,
        wait_hint: 0,
    };
    // SAFETY: a service handle of this test and a live status.
    if unsafe { QueryServiceStatus(svc, &mut st) } == 0 {
        return 0;
    }
    st.current_state
}

/// Wait until the service reaches `want` (up to `ms`); whether it did.
fn svc_wait(svc: HANDLE, want: u32, ms: u32) -> bool {
    let mut w = 0;
    loop {
        if svc_state(svc) == want {
            return true;
        }
        if w >= ms {
            return false;
        }
        // SAFETY: a plain sleep.
        unsafe { Sleep(20) };
        w += 20;
    }
}

/// Stop the service (control STOP) and wait for it to be stopped; delete it; close the handles.
fn svc_remove(scm: HANDLE, svc: HANDLE) -> (bool, bool) {
    let mut st = SERVICE_STATUS {
        service_type: 0,
        current_state: 0,
        controls_accepted: 0,
        win32_exit: 0,
        specific_exit: 0,
        check_point: 0,
        wait_hint: 0,
    };
    // SAFETY: handles of this test; closed once.
    unsafe {
        if svc_state(svc) != SERVICE_STOPPED {
            ControlService(svc, SERVICE_CONTROL_STOP, &mut st);
        }
        let stopped = svc_wait(svc, SERVICE_STOPPED, 5000);
        let deleted = DeleteService(svc) != 0;
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        (stopped, deleted)
    }
}

static SVC_STATUS_HANDLE: Sync_<HANDLE> = Sync_(UnsafeCell::new(core::ptr::null_mut()));
static SVC_TAG: Sync_<[u8; 16]> = Sync_(UnsafeCell::new([0; 16]));

fn set_status(state: u32, accept: u32) {
    let st = SERVICE_STATUS {
        service_type: SERVICE_WIN32_OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        win32_exit: 0,
        specific_exit: 0,
        check_point: 0,
        wait_hint: 0,
    };
    // SAFETY: the handle was registered by this process before; a live status.
    unsafe { SetServiceStatus(*SVC_STATUS_HANDLE.0.get(), &st) };
}

unsafe extern "system" fn svc_handler(control: u32, _event: u32, _data: *mut c_void, _ctx: *mut c_void) -> u32 {
    if control == SERVICE_CONTROL_STOP {
        // SAFETY: SVC_TAG was written before the handler was registered.
        let tag = unsafe { *SVC_TAG.0.get() };
        let n = tag.iter().position(|&b| b == 0).unwrap_or(16);
        make_file(&quit_path(core::str::from_utf8(&tag[..n]).unwrap_or("x")));
    }
    0
}

unsafe extern "system" fn svc_main(argc: u32, argv: *mut *mut u16) {
    let args = child_args().unwrap_or_else(Buf::new);
    let rest = args.as_str();
    let tag = rest.split(' ').nth(1).unwrap_or("x");
    // SAFETY: nothing else runs yet.
    unsafe {
        let t = &mut *SVC_TAG.0.get();
        for (i, b) in tag.bytes().enumerate().take(15) {
            t[i] = b;
        }
        *SRV_LOG.0.get() = srv_log_path(tag);
    }
    ssay!("service main argc={argc}");
    for i in 0..argc as usize {
        // SAFETY: the service control manager passes `argc` strings.
        let a = wstr_at(unsafe { *argv.add(i) }, 100);
        ssay!("service arg{i}={}", units_text(a).as_str());
    }
    let name = wsz("polyws");
    // SAFETY: NUL-terminated name; the handler is a function of this image.
    let h = unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), svc_handler, core::ptr::null_mut()) };
    // SAFETY: nothing else runs yet.
    unsafe { *SVC_STATUS_HANDLE.0.get() = h };
    set_status(SERVICE_START_PENDING, 0);
    set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP);
    server(rest);
    set_status(SERVICE_STOPPED, 0);
}

/// The process side: `--child svc <tag> clsid={...} <flags>`.
fn service(_args: &str) -> u32 {
    let name = wsz("polyws");
    let table = [
        SERVICE_TABLE_ENTRYW { name: name.as_ptr(), proc_: Some(svc_main) },
        SERVICE_TABLE_ENTRYW { name: core::ptr::null(), proc_: None },
    ];
    // SAFETY: a valid table that ends with a NULL entry.
    let ok = unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
    u32::from(ok == 0)
}

/// One entry for a test that is both the client and the service host: the child with `srv` serves a class, with `svc` is
/// the service.
fn child_main(args: &str) -> u32 {
    if args.starts_with("svc") { service(args) } else { server(args) }
}

/// The command line of the test's own service: `"<exe>" --child svc <tag> clsid=<clsid> <flags>`.
fn svc_command(tag: &str, clsid: &str, flags: &str) -> Buf<500> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut b = Buf::<500>::new();
    let _ = write!(b, "\"");
    for &u in &exe[..n] {
        let _ = write!(b, "{}", u as u8 as char);
    }
    let _ = write!(b, "\" --child svc {tag} clsid={clsid} {flags}");
    b
}

fn close_scm(h: HANDLE) {
    // SAFETY: a handle of this test.
    unsafe { CloseServiceHandle(h) };
}

// ===================================================================== COM security helpers (spec-win-com-sec)
// Declarations from Microsoft Learn (combaseapi.h, objidl.h, rpcdce.h, wtypes.h, winnt.h, securitybaseapi.h, processthreadsapi.h);
// the helpers are this test's own. `rpcrt4!I_RpcOpenClientProcess` has no header: it is looked up by name at run time.

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoGetCallContext(riid: *const GUID, out: *mut *mut c_void) -> i32;
    fn CoQueryClientBlanket(
        authn: *mut u32,
        authz: *mut u32,
        princ: *mut *mut u16,
        level: *mut u32,
        imp: *mut u32,
        privs: *mut *mut c_void,
        caps: *mut u32,
    ) -> i32;
    fn CoImpersonateClient() -> i32;
    fn CoRevertToSelf() -> i32;
    fn CoInitializeSecurity(
        sd: *mut c_void,
        cauth: i32,
        auth: *mut c_void,
        r1: *mut c_void,
        level: u32,
        imp: u32,
        authlist: *mut c_void,
        caps: u32,
        r3: *mut c_void,
    ) -> i32;
    fn CoSetProxyBlanket(
        proxy: *mut c_void,
        authn: u32,
        authz: u32,
        princ: *const u16,
        level: u32,
        imp: u32,
        info: *mut c_void,
        caps: u32,
    ) -> i32;
    fn CoQueryProxyBlanket(
        proxy: *mut c_void,
        authn: *mut u32,
        authz: *mut u32,
        princ: *mut *mut u16,
        level: *mut u32,
        imp: *mut u32,
        info: *mut *mut c_void,
        caps: *mut u32,
    ) -> i32;
    fn CoCopyProxy(proxy: *mut c_void, copy: *mut *mut c_void) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenThreadToken(thread: HANDLE, access: u32, as_self: BOOL, out: *mut HANDLE) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, out: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> BOOL;
    fn EqualSid(a: *const c_void, b: *const c_void) -> BOOL;
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn AllocateAndInitializeSid(
        auth: *const [u8; 6],
        count: u8,
        a0: u32,
        a1: u32,
        a2: u32,
        a3: u32,
        a4: u32,
        a5: u32,
        a6: u32,
        a7: u32,
        out: *mut *mut c_void,
    ) -> BOOL;
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
    fn CheckTokenMembership(token: HANDLE, sid: *const c_void, is_member: *mut BOOL) -> BOOL;
    fn RevertToSelf() -> BOOL;
    fn GetUserNameW(buf: *mut u16, n: *mut u32) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessId(h: HANDLE) -> u32;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn GetProcAddress(module: HANDLE, name: *const u8) -> *const c_void;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

const TOKEN_QUERY: u32 = 8;
const TOKEN_IMPERSONATE: u32 = 4;

/// Token information classes (winnt.h `TOKEN_INFORMATION_CLASS`).
const TokenUser: u32 = 1;
const TokenType: u32 = 8;
const TokenImpersonationLevel: u32 = 9;
const TokenSessionId: u32 = 12;
const TokenElevationType: u32 = 18;
const TokenElevation: u32 = 20;
const TokenIntegrityLevel: u32 = 25;

/// A few `RPC_C_*` values (rpcdce.h).
const RPC_C_AUTHN_NONE: u32 = 0;
const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHN_GSS_NEGOTIATE: u32 = 9;
const RPC_C_AUTHN_DEFAULT: u32 = 0xFFFF_FFFF;
const RPC_C_AUTHZ_NONE: u32 = 0;
const RPC_C_AUTHZ_DEFAULT: u32 = 0xFFFF_FFFF;
const RPC_C_AUTHN_LEVEL_DEFAULT: u32 = 0;
const RPC_C_AUTHN_LEVEL_NONE: u32 = 1;
const RPC_C_AUTHN_LEVEL_CONNECT: u32 = 2;
const RPC_C_AUTHN_LEVEL_CALL: u32 = 3;
const RPC_C_AUTHN_LEVEL_PKT: u32 = 4;
const RPC_C_AUTHN_LEVEL_PKT_INTEGRITY: u32 = 5;
const RPC_C_AUTHN_LEVEL_PKT_PRIVACY: u32 = 6;
const RPC_C_IMP_LEVEL_DEFAULT: u32 = 0;
const RPC_C_IMP_LEVEL_ANONYMOUS: u32 = 1;
const RPC_C_IMP_LEVEL_IDENTIFY: u32 = 2;
const RPC_C_IMP_LEVEL_IMPERSONATE: u32 = 3;
const RPC_C_IMP_LEVEL_DELEGATE: u32 = 4;
/// `EOLE_AUTHENTICATION_CAPABILITIES` (objidl.h).
const EOAC_NONE: u32 = 0;
const EOAC_MUTUAL_AUTH: u32 = 1;
const EOAC_STATIC_CLOAKING: u32 = 0x20;
const EOAC_DYNAMIC_CLOAKING: u32 = 0x40;
const EOAC_ANY_AUTHORITY: u32 = 0x80;
const EOAC_MAKE_FULLSIC: u32 = 0x100;
const EOAC_DEFAULT: u32 = 0x800;
const EOAC_SECURE_REFS: u32 = 0x2;
const EOAC_ACCESS_CONTROL: u32 = 0x4;
const EOAC_APPID: u32 = 0x8;
const EOAC_DYNAMIC: u32 = 0x10;
const EOAC_REQUIRE_FULLSIC: u32 = 0x200;
const EOAC_AUTO_IMPERSONATE: u32 = 0x400;
const EOAC_DISABLE_AAA: u32 = 0x1000;
const EOAC_NO_CUSTOM_MARSHAL: u32 = 0x2000;
const EOAC_RESERVED1: u32 = 0x4000;

const RPC_E_TOO_LATE: i32 = 0x8001_0119_u32 as i32;
const RPC_E_CALL_COMPLETE: i32 = 0x8001_0117_u32 as i32;
const RPC_E_ACCESS_DENIED: i32 = 0x8001_0005_u32 as i32;
const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
const RPC_E_NO_GOOD_SECURITY_PACKAGES: i32 = 0x8001_0121_u32 as i32;

#[repr(C)]
struct IServerSecurityVtbl {
    base: IUnknownVtbl,
    query_blanket: unsafe extern "system" fn(
        *mut c_void,
        *mut u32,
        *mut u32,
        *mut *mut u16,
        *mut u32,
        *mut u32,
        *mut *mut c_void,
        *mut u32,
    ) -> i32,
    impersonate: unsafe extern "system" fn(*mut c_void) -> i32,
    revert: unsafe extern "system" fn(*mut c_void) -> i32,
    is_impersonating: unsafe extern "system" fn(*mut c_void) -> BOOL,
}

#[repr(C)]
struct IClientSecurityVtbl {
    base: IUnknownVtbl,
    query_blanket: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut u32,
        *mut u32,
        *mut *mut u16,
        *mut u32,
        *mut u32,
        *mut *mut c_void,
        *mut u32,
    ) -> i32,
    set_blanket:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, *const u16, u32, u32, *mut c_void, u32) -> i32,
    copy_proxy: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32,
}

fn vt<T>(p: *mut c_void) -> &'static T {
    // SAFETY: `p` is a COM interface pointer of this process; its first word is the vtable pointer.
    unsafe { &**(p as *mut *const T) }
}

/// A wide string returned by COM as text (`<null>` for NULL); freed with CoTaskMemFree.
fn take_wstr(p: *mut u16) -> Buf<120> {
    let mut b = Buf::<120>::new();
    if p.is_null() {
        let _ = write!(b, "<null>");
        return b;
    }
    let u = wstr_at(p, 100);
    let _ = write!(b, "{}", units_text(u).as_str());
    // SAFETY: COM allocated the string with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(p.cast()) };
    b
}

/// A wide string owned by COM (not freed), as text.
fn pc_text(p: *mut c_void) -> Buf<120> {
    let mut b = Buf::<120>::new();
    if p.is_null() {
        let _ = write!(b, "<null>");
        return b;
    }
    let u = wstr_at(p as *const u16, 100);
    let _ = write!(b, "{}", units_text(u).as_str());
    b
}

fn get_sec_ctx() -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live out pointer.
    let hr = unsafe { CoGetCallContext(&IID_IServerSecurity, &mut out) };
    (hr, out)
}

fn hx(v: u32) -> Buf<12> {
    let mut b = Buf::<12>::new();
    if v == 0xAAAA_AAAA {
        let _ = write!(b, "unset");
    } else {
        let _ = write!(b, "{v}");
    }
    b
}

/// `IServerSecurity::QueryBlanket` through `ss`, as a line.
fn ss_blanket_text(ss: *mut c_void) -> Buf<400> {
    let (mut an, mut az, mut lv, mut im, mut cap) =
        (0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32);
    let mut pr: *mut u16 = 0x1111 as *mut u16;
    let mut pv: *mut c_void = 0x1111 as *mut c_void;
    // SAFETY: live out pointers; `ss` is an IServerSecurity pointer.
    let hr = unsafe {
        (vt::<IServerSecurityVtbl>(ss).query_blanket)(
            ss, &mut an, &mut az, &mut pr, &mut lv, &mut im, &mut pv, &mut cap,
        )
    };
    let mut b = Buf::<400>::new();
    let _ = write!(
        b,
        "{} authn={} authz={} level={} imp={} caps={:#x}",
        hex8(hr).as_str(),
        hx(an).as_str(),
        hx(az).as_str(),
        hx(lv).as_str(),
        hx(im).as_str(),
        cap
    );
    if hr == 0 {
        let _ = write!(b, " princ={} privs={}", account_text(pr).as_str(), pc_text(pv).as_str());
    }
    b
}

/// What the token of the calling thread (an impersonation token after CoImpersonateClient) says, as text.
fn thread_token_text() -> Buf<300> {
    let mut b = Buf::<300>::new();
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: live out handle; the current thread.
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut tok) };
    if ok == 0 {
        let _ = write!(b, "no thread token (error {})", last_error());
        return b;
    }
    token_text(tok, &mut b);
    close(tok);
    b
}

fn tok_u32(tok: HANDLE, class: u32) -> Option<u32> {
    let mut v = [0u8; 4];
    let mut ret = 0u32;
    // SAFETY: a token handle of this test and a 4-byte buffer.
    let ok = unsafe { GetTokenInformation(tok, class, v.as_mut_ptr(), 4, &mut ret) };
    if ok == 0 { None } else { Some(u32::from_le_bytes(v)) }
}

fn token_text(tok: HANDLE, b: &mut Buf<300>) {
    let _ = write!(
        b,
        "type={:?} implevel={:?} elev={:?} elevtype={:?} il={:?} session={:?}",
        tok_u32(tok, TokenType),
        tok_u32(tok, TokenImpersonationLevel),
        tok_u32(tok, TokenElevation),
        tok_u32(tok, TokenElevationType),
        tok_integrity(tok),
        tok_u32(tok, TokenSessionId)
    );
    let _ = write!(b, " admin_member={:?}", token_is_member_admin(tok));
    let _ = write!(b, " user={}", user_kind(tok).as_str());
}

fn tok_integrity(tok: HANDLE) -> Option<u32> {
    let mut buf = [0u8; 128];
    let mut ret = 0u32;
    // SAFETY: a token handle and a 128-byte buffer.
    let ok = unsafe { GetTokenInformation(tok, TokenIntegrityLevel, buf.as_mut_ptr(), 128, &mut ret) };
    if ok == 0 {
        return None;
    }
    // SAFETY: TOKEN_MANDATORY_LABEL: a SID pointer first; the SID follows in the same buffer.
    unsafe {
        let sid = *(buf.as_ptr() as *const *const c_void);
        let n = *GetSidSubAuthorityCount(sid);
        Some(*GetSidSubAuthority(sid, u32::from(n) - 1))
    }
}

fn token_is_member_admin(tok: HANDLE) -> Option<bool> {
    let nt: [u8; 6] = [0, 0, 0, 0, 0, 5];
    let mut sid: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer; BUILTIN\Administrators = S-1-5-32-544.
    unsafe {
        if AllocateAndInitializeSid(&nt, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut sid) == 0 {
            return None;
        }
        let mut m: BOOL = 0;
        let ok = CheckTokenMembership(tok, sid, &mut m);
        FreeSid(sid);
        if ok == 0 { None } else { Some(m != 0) }
    }
}

/// `system` when the token's user is S-1-5-18, else `same` when it is the user of this process's own token (taken at the start by
/// `cache_my_user`), else `other`.
fn user_kind(tok: HANDLE) -> Buf<10> {
    let mut b = Buf::<10>::new();
    let mut u1 = [0u8; 256];
    let mut r = 0u32;
    // SAFETY: buffers of 256 bytes; a handle of this test; the cached SID was written at the start.
    unsafe {
        let a = GetTokenInformation(tok, TokenUser, u1.as_mut_ptr(), 256, &mut r);
        let mine = &*MY_SID.0.get();
        if a == 0 || MY_SID_OK.load(SeqCst) == 0 {
            let _ = write!(b, "?");
            return b;
        }
        let s1 = *(u1.as_ptr() as *const *const c_void);
        let s2 = *(mine.as_ptr() as *const *const c_void);
        let n = *GetSidSubAuthorityCount(s1);
        let first = *GetSidSubAuthority(s1, 0);
        if n == 1 && first == 18 {
            let _ = write!(b, "system");
        } else if EqualSid(s1, s2) != 0 {
            let _ = write!(b, "same");
        } else {
            let _ = write!(b, "other");
        }
    }
    b
}

/// `rpcrt4!I_RpcOpenClientProcess(binding, access, &process)`, looked up by name; None when the DLL has no such export.
fn rpc_open_client_process(binding: *mut c_void, access: u32) -> Option<(i32, HANDLE)> {
    let name = wide("rpcrt4.dll");
    // SAFETY: NUL-terminated names; the function is called with the three arguments its callers pass.
    unsafe {
        let m = LoadLibraryW(name.as_ptr());
        if m.is_null() {
            return None;
        }
        let f = GetProcAddress(m, b"I_RpcOpenClientProcess\0".as_ptr());
        if f.is_null() {
            return None;
        }
        let f: unsafe extern "system" fn(*mut c_void, u32, *mut HANDLE) -> i32 = core::mem::transmute(f);
        let mut h: HANDLE = 0x1111 as HANDLE;
        let r = f(binding, access, &mut h);
        Some((r, h))
    }
}

static PROBE_MODE: AtomicU32 = AtomicU32::new(0);

// ===================================================================== the test server's security probes (spec-win-com-sec)
// The server (see `server` in the COM helpers) runs these inside the calls it serves: `CreateInstance` (`at` bit 1) and
// `LockServer` (bit 2) of its class factory. Flags of the server command line:
//   p=bl,imp,pid,ctx,thr   which probes run (blanket, impersonation, caller process, call context ids, another thread)
//   at=1|2|3               in which calls (default 3: both)
//   user=<name> csess=<n>  the client's user name and session, to compare with
//   clientfile=<name>      the file the client wrote its process id into
//   secinit=acl:<spec> | null | appid:{guid}   the server's own CoInitializeSecurity (lvl= imp= caps= with it)

static CLIENT_FILE: Sync_<[u16; 700]> = Sync_(UnsafeCell::new([0; 700]));
static P_MASK: AtomicU32 = AtomicU32::new(0);
static P_AT: AtomicU32 = AtomicU32::new(3);
static P_CSESS: AtomicU32 = AtomicU32::new(0xFFFF);
static P_USER: Sync_<[u8; 64]> = Sync_(UnsafeCell::new([0; 64]));

const P_BL: u32 = 1;
const P_IMP: u32 = 2;
const P_PID: u32 = 4;
const P_CTX: u32 = 8;
const P_THR: u32 = 16;
const P_NEST: u32 = 32;
const P_REF: u32 = 64;
const P_STALE: u32 = 128;
const P_ARGS: u32 = 256;

fn client_pid_from_file() -> u32 {
    // SAFETY: written once before the factory exists.
    let p = unsafe { &*CLIENT_FILE.0.get() };
    let txt = read_text(p);
    txt.s().trim().parse().unwrap_or(0)
}

fn probe_user() -> Buf<64> {
    let mut b = Buf::<64>::new();
    // SAFETY: written once before the factory exists.
    let u = unsafe { &*P_USER.0.get() };
    let n = u.iter().position(|&c| c == 0).unwrap_or(64);
    let _ = write!(b, "{}", core::str::from_utf8(&u[..n]).unwrap_or(""));
    b
}

/// The user SID of this process token, taken at the start (while nothing is impersonated).
static MY_SID: Sync_<[u8; 200]> = Sync_(UnsafeCell::new([0; 200]));
static MY_SID_OK: AtomicU32 = AtomicU32::new(0);

fn cache_my_user() {
    let mut tok: HANDLE = core::ptr::null_mut();
    let mut r = 0u32;
    // SAFETY: live out handle and a 200-byte buffer; nothing else runs yet.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) != 0 {
            if GetTokenInformation(tok, TokenUser, (*MY_SID.0.get()).as_mut_ptr(), 200, &mut r) != 0 {
                MY_SID_OK.store(1, SeqCst);
            }
            close(tok);
        }
    }
}

fn sec_server_init(rest: &str) {
    cache_my_user();
    if let Some(n) = flag_val(rest, "clientfile") {
        // SAFETY: nothing else runs yet.
        unsafe { *CLIENT_FILE.0.get() = beside(n) };
    }
    let mut mask = 0;
    for w in flag_val(rest, "p").unwrap_or("").split(',') {
        mask |= match w {
            "bl" => P_BL,
            "imp" => P_IMP,
            "pid" => P_PID,
            "ctx" => P_CTX,
            "thr" => P_THR,
            "nest" => P_NEST,
            "ref" => P_REF,
            "stale" => P_STALE,
            "args" => P_ARGS,
            _ => 0,
        };
    }
    P_MASK.store(mask, SeqCst);
    P_AT.store(flag_num(rest, "at", 3), SeqCst);
    P_CSESS.store(flag_num(rest, "csess", 0xFFFF), SeqCst);
    if let Some(u) = flag_val(rest, "user") {
        // SAFETY: nothing else runs yet.
        unsafe {
            let d = &mut *P_USER.0.get();
            for (i, c) in u.bytes().enumerate().take(63) {
                d[i] = c;
            }
        }
    }
    if let Some(si) = flag_val(rest, "secinit") {
        let lvl = flag_num(rest, "lvl", RPC_C_AUTHN_LEVEL_DEFAULT);
        let imp = flag_num(rest, "imp", RPC_C_IMP_LEVEL_IDENTIFY);
        let caps = flag_num(rest, "caps", EOAC_NONE);
        let mut sd: *mut c_void = core::ptr::null_mut();
        let mut appid = guid(0, 0, 0, [0; 8]);
        if let Some(text) = si.strip_prefix("acl:") {
            sd = abs_sd(text);
        } else if let Some(text) = si.strip_prefix("sddl:") {
            let w = wide(text);
            // SAFETY: NUL-terminated string; live out pointer.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), 1, &mut sd, core::ptr::null_mut())
            };
        } else if let Some(text) = si.strip_prefix("appid:") {
            appid = guid_from_str(text);
            sd = &mut appid as *mut GUID as *mut c_void;
        }
        // SAFETY: the descriptor (or the AppID) lives for the call.
        let hr = unsafe {
            CoInitializeSecurity(
                sd,
                -1,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                lvl,
                imp,
                core::ptr::null_mut(),
                caps,
                core::ptr::null_mut(),
            )
        };
        ssay!("CoInitializeSecurity({}) = {}", si.split(':').next().unwrap_or(""), hex8(hr).as_str());
    }
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        rev: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> BOOL;
    fn InitializeSecurityDescriptor(sd: *mut c_void, rev: u32) -> BOOL;
    fn SetSecurityDescriptorOwner(sd: *mut c_void, sid: *mut c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorGroup(sd: *mut c_void, sid: *mut c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: *mut c_void, present: BOOL, acl: *mut c_void, defaulted: BOOL) -> BOOL;
    fn InitializeAcl(acl: *mut c_void, size: u32, rev: u32) -> BOOL;
    fn AddAccessAllowedAce(acl: *mut c_void, rev: u32, mask: u32, sid: *mut c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: *mut c_void, rev: u32, mask: u32, sid: *mut c_void) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut *mut c_void) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetComputerNameW(buf: *mut u16, n: *mut u32) -> BOOL;
}

#[repr(C, align(8))]
struct Raw<const N: usize>([u8; N]);
static ABS_SD: Sync_<Raw<64>> = Sync_(UnsafeCell::new(Raw([0; 64])));
static ABS_ACL: Sync_<Raw<1024>> = Sync_(UnsafeCell::new(Raw([0; 1024])));

fn sid_of(text: &str) -> *mut c_void {
    let w = wide(text);
    let mut sid: *mut c_void = core::ptr::null_mut();
    // SAFETY: NUL-terminated string, live out pointer; the SID is kept for the life of the process.
    unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) };
    sid
}

/// An absolute security descriptor (owner and group Administrators) with the DACL `spec`: entries `+<sid>=<mask>` (allow) and
/// `-<sid>=<mask>` (deny) separated by commas, in that order; `<sid>` is an SDDL SID string ("WD", "BA", "SY", "BU", "IU",
/// "AU", ... or S-1-...), `<mask>` a hexadecimal number. A spec of `-` is an empty (present) DACL.
fn abs_sd(spec: &str) -> *mut c_void {
    // SAFETY: the static buffers are written before any other thread uses them (server start).
    unsafe {
        let sd = ABS_SD.0.get() as *mut c_void;
        let acl = ABS_ACL.0.get() as *mut c_void;
        InitializeSecurityDescriptor(sd, 1);
        let owner = sid_of("BA");
        if !spec.contains("!noowner") {
            SetSecurityDescriptorOwner(sd, owner, 0);
        }
        if !spec.contains("!nogroup") {
            SetSecurityDescriptorGroup(sd, owner, 0);
        }
        InitializeAcl(acl, 1024, 2);
        for e in spec.split(',') {
            if e.len() < 4 || e.starts_with('!') {
                continue;
            }
            let allow = e.starts_with('+');
            let (sid, mask) = e[1..].split_once('=').unwrap_or((e, "0"));
            let m = u32::from_str_radix(mask, 16).unwrap_or(0);
            let s = sid_of(sid);
            if allow {
                AddAccessAllowedAce(acl, 2, m, s);
            } else {
                AddAccessDeniedAce(acl, 2, m, s);
            }
        }
        if !spec.contains("!nodacl") {
            SetSecurityDescriptorDacl(sd, 1, acl, 0);
        } else if spec.contains("!nulldacl") {
            SetSecurityDescriptorDacl(sd, 1, core::ptr::null_mut(), 0);
        }
        sd
    }
}

fn computer_name() -> Buf<64> {
    let mut w = [0u16; 64];
    let mut n = 64u32;
    // SAFETY: a 64-unit buffer and its size.
    unsafe { GetComputerNameW(w.as_mut_ptr(), &mut n) };
    let mut b = Buf::<64>::new();
    for &u in &w[..(n as usize).min(63)] {
        let _ = write!(b, "{}", u as u8 as char);
    }
    b
}

/// The client name COM reports ("domain\user") as `has_domain` (a backslash), `domain_is_computer`, `user_is_client`.
fn privs_summary(p: *mut c_void) -> Buf<100> {
    let mut b = Buf::<100>::new();
    if p.is_null() {
        let _ = write!(b, "privs=<null>");
        return b;
    }
    let units = wstr_at(p as *const u16, 100);
    let mut text = Buf::<120>::new();
    for &u in units {
        let _ = write!(text, "{}", u as u8 as char);
    }
    match text.as_str().split_once('\\') {
        None => {
            let _ = write!(
                b,
                "privs has_domain=0 user_is_client={}",
                u32::from(eq_ascii_nocase(text.as_str(), probe_user().as_str()))
            );
        }
        Some((d, u)) => {
            let _ = write!(
                b,
                "privs has_domain=1 domain_is_computer={} user_is_client={}",
                u32::from(eq_ascii_nocase(d, computer_name().as_str())),
                u32::from(eq_ascii_nocase(u, probe_user().as_str()))
            );
        }
    }
    b
}

fn eq_ascii_nocase(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn on_create() {
    if P_AT.load(SeqCst) & 1 != 0 {
        probe("create");
    }
}

fn on_lock() {
    if P_AT.load(SeqCst) & 2 != 0 {
        probe("lock");
    }
}

fn probe(lab: &str) {
    let m = P_MASK.load(SeqCst);
    if m & P_CTX != 0 {
        probe_ctx(lab);
    }
    if m & P_BL != 0 {
        probe_bl(lab);
    }
    if m & P_IMP != 0 {
        probe_imp(lab);
    }
    if m & P_PID != 0 {
        probe_pid(lab);
    }
    if m & P_THR != 0 {
        probe_thr(lab);
    }
    if m & P_NEST != 0 {
        probe_nest(lab);
    }
    if m & P_REF != 0 {
        probe_ref(lab);
    }
    if m & P_STALE != 0 {
        probe_stale(lab);
    }
    if m & P_ARGS != 0 {
        probe_args(lab);
    }
}

/// NULL output pointers of IServerSecurity::QueryBlanket and CoQueryClientBlanket, each alone and all together.
fn probe_args(lab: &str) {
    let nu = core::ptr::null_mut::<u32>();
    let np = core::ptr::null_mut::<*mut u16>();
    let nv = core::ptr::null_mut::<*mut c_void>();
    let (mut an, mut lv) = (0xAAAA_AAAAu32, 0xAAAA_AAAAu32);
    let mut pr: *mut u16 = core::ptr::null_mut();
    let mut pv: *mut c_void = core::ptr::null_mut();
    // SAFETY (all calls below): live out pointers or NULL.
    unsafe {
        ssay!(
            "{lab} args: CoQueryClientBlanket all NULL = {}",
            hex8(CoQueryClientBlanket(nu, nu, np, nu, nu, nv, nu)).as_str()
        );
        let hr = CoQueryClientBlanket(&mut an, nu, np, nu, nu, nv, nu);
        ssay!("{lab} args: only the authentication service = {} {}", hex8(hr).as_str(), hx(an).as_str());
        let hr = CoQueryClientBlanket(nu, nu, np, &mut lv, nu, nv, nu);
        ssay!("{lab} args: only the level = {} {}", hex8(hr).as_str(), hx(lv).as_str());
        let hr = CoQueryClientBlanket(nu, nu, &mut pr, nu, nu, nv, nu);
        ssay!(
            "{lab} args: only the server principal name = {} {}",
            hex8(hr).as_str(),
            if pr.is_null() { "NULL" } else { "string" }
        );
        if !pr.is_null() {
            CoTaskMemFree(pr.cast());
        }
        let hr = CoQueryClientBlanket(nu, nu, np, nu, nu, &mut pv, nu);
        ssay!("{lab} args: only the client name = {} {}", hex8(hr).as_str(), privs_summary(pv).as_str());
        let (h, ss) = get_sec_ctx();
        if h == 0 {
            let v = vt::<IServerSecurityVtbl>(ss);
            ssay!(
                "{lab} args: IServerSecurity::QueryBlanket all NULL = {}",
                hex8((v.query_blanket)(ss, nu, nu, np, nu, nu, nv, nu)).as_str()
            );
            let hr = (v.query_blanket)(ss, &mut an, nu, np, nu, nu, nv, nu);
            ssay!(
                "{lab} args: IServerSecurity::QueryBlanket only the authentication service = {} {}",
                hex8(hr).as_str(),
                hx(an).as_str()
            );
            unk_release(ss);
        }
    }
}

static STALE_SS: AtomicUsize = AtomicUsize::new(0);

fn probe_nest(lab: &str) {
    let (hr, ss) = get_sec_ctx();
    if hr != 0 {
        ssay!("{lab} nest: no context {}", hex8(hr).as_str());
        return;
    }
    let mut ln = Lines::new();
    let mut put = |text: core::fmt::Arguments| {
        let mut b = Buf::<300>::new();
        let _ = b.write_fmt(text);
        ln.add(lab, b.as_str());
    };
    // SAFETY: `ss` is an IServerSecurity pointer.
    unsafe {
        let v = vt::<IServerSecurityVtbl>(ss);
        let r0 = (v.revert)(ss);
        put(format_args!(
            "nest: RevertToSelf while not impersonating = {} IsImpersonating {}",
            hex8(r0).as_str(),
            (v.is_impersonating)(ss)
        ));
        let a = (v.impersonate)(ss);
        let b = (v.impersonate)(ss);
        let isimp = (v.is_impersonating)(ss);
        let tk = thread_token_line();
        let c = (v.revert)(ss);
        let i1 = (v.is_impersonating)(ss);
        let d = (v.revert)(ss);
        let i2 = (v.is_impersonating)(ss);
        put(format_args!(
            "nest: ImpersonateClient twice = {} {} IsImpersonating {}",
            hex8(a).as_str(),
            hex8(b).as_str(),
            isimp
        ));
        put(format_args!("nest:   {}", tk.as_str()));
        put(format_args!("nest: first RevertToSelf = {} IsImpersonating {}", hex8(c).as_str(), i1));
        put(format_args!("nest: second RevertToSelf = {} IsImpersonating {}", hex8(d).as_str(), i2));
        // IsImpersonating through another context pointer, while this thread impersonates
        let _ = (v.impersonate)(ss);
        let (_, ss2) = get_sec_ctx();
        let other = (vt::<IServerSecurityVtbl>(ss2).is_impersonating)(ss2);
        let _ = (v.revert)(ss);
        unk_release(ss2);
        put(format_args!("nest: other pointer IsImpersonating {}", other));
        // CoRevertToSelf reverts what ImpersonateClient started
        let _ = (v.impersonate)(ss);
        let r = CoRevertToSelf();
        put(format_args!(
            "nest: ImpersonateClient then CoRevertToSelf = {} IsImpersonating {}",
            hex8(r).as_str(),
            (v.is_impersonating)(ss)
        ));
    }
    unk_release(ss);
    ln.flush();
}

fn probe_ref(lab: &str) {
    let (h1, a) = get_sec_ctx();
    let (h2, b) = get_sec_ctx();
    ssay!(
        "{lab} ref: two CoGetCallContext calls = {} {} same pointer {}",
        hex8(h1).as_str(),
        hex8(h2).as_str(),
        u32::from(a == b)
    );
    if h1 == 0 {
        let n1 = unk_addref(a);
        let n2 = unk_release(a);
        ssay!(
            "{lab} ref: AddRef returns {} more than a Release that follows returns {}",
            n1 as i64 - n2 as i64,
            n2 as i64 - n2 as i64
        );
        let (hq, q) = unk_qi(a, &IID_IUnknown);
        ssay!("{lab} ref: QueryInterface(IUnknown) = {} same pointer {}", hex8(hq).as_str(), u32::from(q == a));
        if hq == 0 {
            unk_release(q);
        }
        let (hq, q) = unk_qi(a, &IID_IServerSecurity);
        ssay!("{lab} ref: QueryInterface(IServerSecurity) = {} same pointer {}", hex8(hq).as_str(), u32::from(q == a));
        if hq == 0 {
            unk_release(q);
        }
        let (hq, _) = unk_qi(a, &IID_IMarshal);
        ssay!("{lab} ref: QueryInterface(IMarshal) = {}", hex8(hq).as_str());
        let (hq, _) = unk_qi(a, &IID_IClientSecurity);
        ssay!("{lab} ref: QueryInterface(IClientSecurity) = {}", hex8(hq).as_str());
        unk_release(a);
    }
    if h2 == 0 {
        unk_release(b);
    }
}

fn probe_stale(lab: &str) {
    let prev = STALE_SS.load(SeqCst);
    if prev != 0 {
        let ss = prev as *mut c_void;
        ssay!("{lab} stale: pointer kept from an earlier call: QueryBlanket {}", ss_blanket_text(ss).as_str());
        // SAFETY: the reference was kept (AddRef) on purpose; the object stays alive.
        let r = unsafe { (vt::<IServerSecurityVtbl>(ss).impersonate)(ss) };
        ssay!("{lab} stale: ImpersonateClient on it = {}", hex8(r).as_str());
        // SAFETY: as above.
        unsafe { (vt::<IServerSecurityVtbl>(ss).revert)(ss) };
        unk_release(ss);
        STALE_SS.store(0, SeqCst);
    } else {
        let (hr, ss) = get_sec_ctx();
        if hr == 0 {
            STALE_SS.store(ss as usize, SeqCst);
            ssay!("{lab} stale: pointer kept");
        }
    }
}

fn q_client_blanket() -> (i32, Buf<260>) {
    let (mut an, mut az, mut lv, mut im, mut cap) =
        (0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32);
    let mut pr: *mut u16 = 0x1111 as *mut u16;
    let mut pv: *mut c_void = 0x1111 as *mut c_void;
    // SAFETY: live out pointers.
    let hr = unsafe { CoQueryClientBlanket(&mut an, &mut az, &mut pr, &mut lv, &mut im, &mut pv, &mut cap) };
    blanket_line(hr, an, az, pr, lv, im, pv, cap)
}

#[allow(clippy::too_many_arguments)]
fn blanket_line(
    hr: i32,
    an: u32,
    az: u32,
    pr: *mut u16,
    lv: u32,
    im: u32,
    pv: *mut c_void,
    cap: u32,
) -> (i32, Buf<260>) {
    let mut b = Buf::<260>::new();
    let _ = write!(
        b,
        "{} authn={} authz={} level={} imp={} caps={:#x}",
        hex8(hr).as_str(),
        hx(an).as_str(),
        hx(az).as_str(),
        hx(lv).as_str(),
        hx(im).as_str(),
        if cap == 0xAAAA_AAAA { 0 } else { cap }
    );
    if hr == 0 {
        let name = take_wstr(pr);
        let _ = write!(b, " princ={} {}", name.as_str(), privs_summary(pv).as_str());
    }
    (hr, b)
}

fn probe_bl(lab: &str) {
    let (hr, ss) = get_sec_ctx();
    ssay!("{lab} bl: CoGetCallContext(IServerSecurity) = {} pointer {}", hex8(hr).as_str(), u32::from(!ss.is_null()));
    if hr == 0 {
        let (mut an, mut az, mut lv, mut im, mut cap) =
            (0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32);
        let mut pr: *mut u16 = 0x1111 as *mut u16;
        let mut pv: *mut c_void = 0x1111 as *mut c_void;
        // SAFETY: live out pointers; `ss` is an IServerSecurity pointer.
        let hr = unsafe {
            (vt::<IServerSecurityVtbl>(ss).query_blanket)(
                ss, &mut an, &mut az, &mut pr, &mut lv, &mut im, &mut pv, &mut cap,
            )
        };
        let (_, line) = blanket_line(hr, an, az, pr, lv, im, pv, cap);
        ssay!("{lab} bl: IServerSecurity::QueryBlanket = {}", line.as_str());
        unk_release(ss);
    }
    let (_, line) = q_client_blanket();
    ssay!("{lab} bl: CoQueryClientBlanket = {}", line.as_str());
}

/// Whether the calling thread has an impersonation token, then its facts.
fn thread_token_line() -> Buf<300> {
    let mut b = Buf::<300>::new();
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: live out handle; the current thread.
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut tok) };
    if ok == 0 {
        let _ = write!(b, "no thread token (error {})", last_error());
        return b;
    }
    token_text(tok, &mut b);
    let cs = P_CSESS.load(SeqCst);
    if cs != 0xFFFF {
        let _ = write!(b, " session_is_client={}", u32::from(tok_u32(tok, TokenSessionId) == Some(cs)));
    }
    close(tok);
    b
}

/// Lines collected while the thread impersonates (a log file cannot be written then: an identification-level token opens nothing)
/// and written after the revert.
struct Lines {
    b: Buf<1800>,
}

impl Lines {
    fn new() -> Lines {
        Lines { b: Buf::new() }
    }
    fn add(&mut self, lab: &str, text: &str) {
        let _ = write!(self.b, "{lab} {text}{}", char::from(10));
    }
    fn flush(&self) {
        for l in self.b.as_str().split(char::from(10)) {
            if !l.is_empty() {
                ssay!("{}", l);
            }
        }
    }
}

fn probe_imp(lab: &str) {
    let mut ln = Lines::new();
    let (hr, ss) = get_sec_ctx();
    if hr == 0 {
        // SAFETY: `ss` is an IServerSecurity pointer.
        unsafe {
            let v = vt::<IServerSecurityVtbl>(ss);
            let i0 = (v.is_impersonating)(ss);
            let r = (v.impersonate)(ss);
            let i1 = (v.is_impersonating)(ss);
            let mut x = Buf::<200>::new();
            let _ = write!(
                x,
                "imp: IServerSecurity::ImpersonateClient = {} IsImpersonating {i0} -> {i1}",
                hex8(r).as_str()
            );
            let tk = thread_token_line();
            let r2 = (v.revert)(ss);
            let i2 = (v.is_impersonating)(ss);
            ln.add(lab, x.as_str());
            let mut y = Buf::<340>::new();
            let _ = write!(y, "imp:   {}", tk.as_str());
            ln.add(lab, y.as_str());
            let mut z = Buf::<200>::new();
            let _ = write!(z, "imp: IServerSecurity::RevertToSelf = {} IsImpersonating -> {i2}", hex8(r2).as_str());
            ln.add(lab, z.as_str());
            let mut w = Buf::<200>::new();
            let _ = write!(w, "imp:   {}", thread_token_line().as_str());
            ln.add(lab, w.as_str());
        }
        unk_release(ss);
    }
    // SAFETY: plain calls.
    let r = unsafe { CoImpersonateClient() };
    let tk = thread_token_line();
    // SAFETY: plain call.
    let r2 = unsafe { CoRevertToSelf() };
    let mut x = Buf::<200>::new();
    let _ = write!(x, "imp: CoImpersonateClient = {}", hex8(r).as_str());
    ln.add(lab, x.as_str());
    let mut y = Buf::<340>::new();
    let _ = write!(y, "imp:   {}", tk.as_str());
    ln.add(lab, y.as_str());
    let mut z = Buf::<200>::new();
    let _ = write!(z, "imp: CoRevertToSelf = {}", hex8(r2).as_str());
    ln.add(lab, z.as_str());
    let mut w = Buf::<200>::new();
    let _ = write!(w, "imp:   {}", thread_token_line().as_str());
    ln.add(lab, w.as_str());
    ln.flush();
}

fn probe_pid(lab: &str) {
    let me = {
        // SAFETY: no arguments.
        unsafe { GetCurrentProcessId() }
    };
    match rpc_open_client_process(core::ptr::null_mut(), 0x1000) {
        None => ssay!("{lab} pid: I_RpcOpenClientProcess is not exported"),
        Some((r, h)) => {
            let ok = !h.is_null() && h != 0x1111 as HANDLE && h != INVALID_HANDLE_VALUE;
            let mut extra = Buf::<80>::new();
            if r == 0 && ok {
                // SAFETY: the handle returned above.
                let pid = unsafe { GetProcessId(h) };
                let want = client_pid_from_file();
                let _ = write!(
                    extra,
                    " is_client={} is_self={}",
                    u32::from(pid == want && want != 0),
                    u32::from(pid == me)
                );
                close(h);
            }
            ssay!(
                "{lab} pid: I_RpcOpenClientProcess(NULL, PROCESS_QUERY_LIMITED_INFORMATION) = {r} handle {}{}",
                u32::from(ok),
                extra.as_str()
            );
        }
    }
    if let Some((r, h)) = rpc_open_client_process(core::ptr::null_mut(), 0x0010) {
        let ok = !h.is_null() && h != 0x1111 as HANDLE && h != INVALID_HANDLE_VALUE;
        ssay!("{lab} pid: ... with PROCESS_VM_READ = {r} handle {}", u32::from(ok));
        if r == 0 && ok {
            close(h);
        }
    }
}

fn probe_ctx(lab: &str) {
    let ids: [(&str, GUID); 9] = [
        ("IServerSecurity", IID_IServerSecurity),
        ("IUnknown", IID_IUnknown),
        ("IClientSecurity", IID_IClientSecurity),
        ("IMarshal", IID_IMarshal),
        ("IDispatch", IID_IDispatch),
        ("IContextCallback", guid(0x1DA, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46])),
        ("IObjContext", guid(0x1C6, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46])),
        ("ISecurityCallContext", guid(0xCAFC823E, 0xB441, 0x11D1, [0xB8, 0x2B, 0, 0, 0xF8, 0x75, 0x7E, 0x2A])),
        ("GUID_NULL", guid(0, 0, 0, [0; 8])),
    ];
    for (n, g) in ids.iter() {
        let mut out: *mut c_void = core::ptr::null_mut();
        // SAFETY: live out pointer.
        let hr = unsafe { CoGetCallContext(g, &mut out) };
        ssay!("{lab} ctx: CoGetCallContext({n}) = {} pointer {}", hex8(hr).as_str(), u32::from(!out.is_null()));
        if hr == 0 {
            unk_release(out);
        }
    }
}

unsafe extern "system" fn thr_probe(_a: *mut c_void) -> u32 {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer; plain calls on a thread that serves no call.
    unsafe {
        let hr = CoGetCallContext(&IID_IServerSecurity, &mut out);
        ssay!("  other thread: CoGetCallContext(IServerSecurity) = {}", hex8(hr).as_str());
        let (hr2, _) = q_client_blanket();
        ssay!("  other thread: CoQueryClientBlanket = {}", hex8(hr2).as_str());
        let hr3 = CoImpersonateClient();
        ssay!("  other thread: CoImpersonateClient = {}", hex8(hr3).as_str());
        let hr4 = CoRevertToSelf();
        ssay!("  other thread: CoRevertToSelf = {}", hex8(hr4).as_str());
        match rpc_open_client_process(core::ptr::null_mut(), 0x1000) {
            None => ssay!("  other thread: I_RpcOpenClientProcess is not exported"),
            Some((r, h)) => {
                ssay!("  other thread: I_RpcOpenClientProcess = {r}");
                if r == 0 {
                    close(h);
                }
            }
        }
    }
    0
}

fn probe_thr(lab: &str) {
    ssay!("{lab} thr:");
    let h = spawn(thr_probe, 0);
    join(h, 5000);
}

fn unk_qi_into(p: *mut c_void, iid: &GUID, out: &mut *mut c_void) -> i32 {
    // SAFETY: `p` is a COM interface pointer the test holds.
    unsafe { ((*(*(p as *mut *const IUnknownVtbl))).qi)(p, iid, out) }
}

// ===================================================================== client side of the security tests (spec-win-com-sec)

/// Whether this process has the full (elevated) administrator token (TokenElevation).
fn is_elevated() -> bool {
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: live out handle; the current process.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let e = tok_u32(tok, TokenElevation);
        close(tok);
        e == Some(1)
    }
}

/// The class registrations go where this process's COM looks: HKLM for the elevated token (an elevated process does not read
/// the user's classes), HKCU for the filtered one (it cannot write HKLM).
fn root() -> HANDLE {
    if is_elevated() { HKLM } else { HKCU }
}

fn my_user() -> Buf<64> {
    let mut w = [0u16; 64];
    let mut n = 64u32;
    // SAFETY: a 64-unit buffer and its size.
    unsafe { GetUserNameW(w.as_mut_ptr(), &mut n) };
    let mut b = Buf::<64>::new();
    for &u in &w[..(n as usize).saturating_sub(1).min(63)] {
        let _ = write!(b, "{}", u as u8 as char);
    }
    b
}

fn my_session() -> u32 {
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: live out handle; the current process.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return 0xFFFF;
        }
        let s = tok_u32(tok, TokenSessionId).unwrap_or(0xFFFF);
        close(tok);
        s
    }
}

/// A server of the test: a class registered for `LocalServer32` (the server runs as the client's user) or hosted in a service.
struct Env {
    tag: &'static str,
    ctext: Buf<40>,
    clsid: GUID,
    root: HANDLE,
    appid: Option<Buf<40>>,
    svc: Option<(HANDLE, HANDLE)>,
    /// Do not print the server's log at the end.
    quiet: bool,
}

fn client_file_name(tag: &str) -> Buf<40> {
    let mut f = Buf::<40>::new();
    let _ = write!(f, "polyws-cpid-{tag}");
    f
}

/// Write this process's id into the client file of `tag` (the server compares what it finds with it).
fn write_client_pid(tag: &str) {
    let f = client_file_name(tag);
    let p = beside(f.as_str());
    delete_file(&p);
    let mut pb = Buf::<20>::new();
    // SAFETY: no arguments.
    let _ = write!(pb, "{}", unsafe { GetCurrentProcessId() });
    append_line(&p, pb.as_str());
}

fn server_flags(tag: &str, flags: &str) -> Buf<420> {
    let mut b = Buf::<420>::new();
    let _ = write!(
        b,
        "mta multi {flags} user={} csess={} clientfile={}",
        my_user().as_str(),
        my_session(),
        client_file_name(tag).as_str()
    );
    b
}

/// Register the class `base` (made unique per run) with a `LocalServer32` server started with `flags`.
fn env_local(tag: &'static str, base: &str, flags: &str) -> Env {
    let ctext = uniq(base);
    let clsid = guid_from_str(ctext.as_str());
    cleanup_tag(tag);
    write_client_pid(tag);
    let r = root();
    let cmd = server_command(tag, ctext.as_str(), server_flags(tag, flags).as_str());
    reg_class_local(r, ctext.as_str(), "polyws-com-sec", cmd.as_str(), None);
    Env { tag, ctext, clsid, root: r, appid: None, svc: None, quiet: false }
}

/// The same for a class hosted in a service (LocalSystem) named `svc`; only an elevated test can make it.
fn env_service(tag: &'static str, base: &str, appid_base: &str, svc: &str, flags: &str) -> Option<Env> {
    let ctext = uniq(base);
    let clsid = guid_from_str(ctext.as_str());
    cleanup_tag(tag);
    write_client_pid(tag);
    let cmd = svc_command(tag, ctext.as_str(), server_flags(tag, flags).as_str());
    let (scm, s, _e) = svc_create(svc, cmd.as_str());
    if s.is_null() {
        close_scm(scm);
        return None;
    }
    let atext = uniq(appid_base);
    let mut k = Buf::<120>::new();
    let _ = write!(k, "CLSID\\{}", ctext.as_str());
    reg_put(HKLM, k.as_str(), None, "polyws-com-sec-svc");
    reg_put(HKLM, k.as_str(), Some("AppID"), atext.as_str());
    let mut ka = Buf::<120>::new();
    let _ = write!(ka, "AppID\\{}", atext.as_str());
    reg_put(HKLM, ka.as_str(), None, "polyws-com-sec-appid");
    reg_put(HKLM, ka.as_str(), Some("LocalService"), svc);
    Some(Env { tag, ctext, clsid, root: HKLM, appid: Some(atext), svc: Some((scm, s)), quiet: false })
}

/// Stop the server, print its log (`srv: ...` lines) and remove everything the environment made.
fn env_end(t: &mut Test, e: Env) {
    if let Some((scm, s)) = e.svc {
        let (stopped, deleted) = svc_remove(scm, s);
        say!(t, "service removed: stopped {} deleted {}", u32::from(stopped), u32::from(deleted));
    } else if read_text(&srv_log_path(e.tag)).s().contains("start pid=") {
        stop_server(e.tag, 3000);
    }
    if !e.quiet {
        print_log(t, e.tag);
    }
    if let Some(a) = &e.appid {
        let mut ka = Buf::<120>::new();
        let _ = write!(ka, "AppID\\{}", a.as_str());
        reg_del(HKLM, ka.as_str());
    }
    unreg_class(e.root, e.ctext.as_str());
    cleanup_tag(e.tag);
    delete_file(&beside(client_file_name(e.tag).as_str()));
}

/// The class object of the environment's class through COM, as the client gets it: (hr, IClassFactory proxy).
fn env_factory(e: &Env) -> (i32, *mut c_void) {
    get_class_obj(&e.clsid, CLSCTX_LOCAL_SERVER, &IID_IClassFactory)
}

fn env_create(e: &Env) -> (i32, *mut c_void) {
    create_inst(&e.clsid, CLSCTX_LOCAL_SERVER, &IID_IUnknown)
}

/// `CoQueryProxyBlanket` on `p`, as a line (the strings are freed).
fn proxy_blanket_line(p: *mut c_void) -> Buf<260> {
    let (mut an, mut az, mut lv, mut im, mut cap) =
        (0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32, 0xAAAA_AAAAu32);
    let mut pr: *mut u16 = 0x1111 as *mut u16;
    let mut info: *mut c_void = 0x1111 as *mut c_void;
    // SAFETY: live out pointers.
    let hr = unsafe { CoQueryProxyBlanket(p, &mut an, &mut az, &mut pr, &mut lv, &mut im, &mut info, &mut cap) };
    let mut b = Buf::<260>::new();
    let _ = write!(
        b,
        "{} authn={} authz={} level={} imp={} caps={:#x}",
        hex8(hr).as_str(),
        hx(an).as_str(),
        hx(az).as_str(),
        hx(lv).as_str(),
        hx(im).as_str(),
        if cap == 0xAAAA_AAAA { 0 } else { cap }
    );
    if hr == 0 {
        let _ =
            write!(b, " princ={} authinfo={}", account_text(pr).as_str(), if info.is_null() { "null" } else { "set" });
    }
    b
}

fn set_blanket(p: *mut c_void, authn: u32, authz: u32, princ: *const u16, lvl: u32, imp: u32, caps: u32) -> i32 {
    // SAFETY: `p` is a COM pointer of this process; the name (when given) is NUL-terminated.
    unsafe { CoSetProxyBlanket(p, authn, authz, princ, lvl, imp, core::ptr::null_mut(), caps) }
}

/// A separator line in the server's log (the client writes it between its steps).
fn mark(tag: &str, text: &str) {
    let mut l = Buf::<120>::new();
    let _ = write!(l, "---- {text}");
    append_line(&srv_log_path(tag), l.as_str());
}

/// Run `f` in a child process of this image (`--child <arg>`) and return its exit code (0xDEAD when it hangs).
fn child_code(arg: &str) -> u32 {
    run_child(arg).unwrap_or(0xFFFF_FFFE)
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSecurityDescriptorLength(sd: *mut c_void) -> u32;
    fn RegDeleteValueW(key: HANDLE, name: *const u16) -> i32;
}

/// Write a REG_BINARY value (the self-relative descriptor made from `sddl`) under `HKLM\SOFTWARE\Classes\<sub>`.
fn reg_put_sd(root: HANDLE, sub: &str, name: &str, sddl: &str) -> i32 {
    let w = wide(sddl);
    let mut sd: *mut c_void = core::ptr::null_mut();
    // SAFETY: NUL-terminated string, live out pointer; the descriptor is freed below.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), 1, &mut sd, core::ptr::null_mut()) == 0 {
            return -1;
        }
        let n = GetSecurityDescriptorLength(sd);
        let mut path = Buf::<300>::new();
        let _ = write!(path, r"SOFTWARE\Classes\{sub}");
        let p = wsz(path.as_str());
        let nm = wsz(name);
        let mut key: HANDLE = core::ptr::null_mut();
        let mut disp = 0u32;
        let r = RegCreateKeyExW(
            root,
            p.as_ptr(),
            0,
            core::ptr::null(),
            0,
            KEY_ALL_ACCESS_ISH,
            core::ptr::null(),
            &mut key,
            &mut disp,
        );
        if r == 0 {
            RegSetValueExW(key, nm.as_ptr(), 0, 3, sd as *const u8, n);
            RegCloseKey(key);
        }
        LocalFree(sd);
        r
    }
}

fn reg_del_value(root: HANDLE, sub: &str, name: &str) {
    let mut path = Buf::<300>::new();
    let _ = write!(path, r"SOFTWARE\Classes\{sub}");
    let p = wsz(path.as_str());
    let nm = wsz(name);
    let mut key: HANDLE = core::ptr::null_mut();
    // SAFETY: NUL-terminated strings, live out handle.
    unsafe {
        if RegOpenKeyExW(root, p.as_ptr(), 0, KEY_ALL_ACCESS_ISH, &mut key) == 0 {
            RegDeleteValueW(key, nm.as_ptr());
            RegCloseKey(key);
        }
    }
}

/// Like `env_local`, with an AppID value on the class and an AppID key (made unique per run).
fn env_local_appid(tag: &'static str, base: &str, appid_text: &str, flags: &str) -> Env {
    let ctext = uniq(base);
    let clsid = guid_from_str(ctext.as_str());
    cleanup_tag(tag);
    write_client_pid(tag);
    let r = root();
    let mut atext = Buf::<40>::new();
    let _ = write!(atext, "{appid_text}");
    let cmd = server_command(tag, ctext.as_str(), server_flags(tag, flags).as_str());
    reg_class_local(r, ctext.as_str(), "polyws-com-sec", cmd.as_str(), Some(atext.as_str()));
    let mut ka = Buf::<120>::new();
    let _ = write!(ka, r"AppID\{}", atext.as_str());
    reg_put(r, ka.as_str(), None, "polyws-com-sec-appid");
    Env { tag, ctext, clsid, root: r, appid: Some(atext), svc: None, quiet: false }
}

/// The registry path of the environment's AppID key.
fn appid_sub(e: &Env) -> Buf<120> {
    let mut ka = Buf::<120>::new();
    let _ = write!(ka, r"AppID\{}", e.appid.as_ref().map_or("", |a| a.as_str()));
    ka
}

/// A principal name COM returned, as text without the name itself: `<null>`, `account of this user` when it is
/// computer name, a backslash and user name of the user running this process (case ignored), else `other name`. The string is freed.
fn account_text(p: *mut u16) -> Buf<40> {
    let mut b = Buf::<40>::new();
    if p.is_null() {
        let _ = write!(b, "<null>");
        return b;
    }
    let mut got = Buf::<160>::new();
    for &u in wstr_at(p, 150) {
        let _ = write!(got, "{}", u as u8 as char);
    }
    let mut want = Buf::<160>::new();
    let _ = write!(want, "{}{}{}", computer_name().as_str(), char::from(92), my_user().as_str());
    // SAFETY: allocated by COM with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(p.cast()) };
    let _ = write!(
        b,
        "{}",
        if eq_ascii_nocase(got.as_str(), want.as_str()) { "account of this user" } else { "other name" }
    );
    b
}

main_with_child!(server, test);

const C0: &str = "{B2A0D103-0000-4000-8000-706F6C797773}";

fn imp_of(p: *mut c_void) -> u32 {
    let (mut an, mut az, mut lv, mut im, mut cap) = (0u32, 0u32, 0u32, 0xFFFFu32, 0u32);
    let mut pr: *mut u16 = core::ptr::null_mut();
    let mut info: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointers.
    let hr = unsafe { CoQueryProxyBlanket(p, &mut an, &mut az, &mut pr, &mut lv, &mut im, &mut info, &mut cap) };
    if !pr.is_null() {
        // SAFETY: allocated by COM with CoTaskMemAlloc.
        unsafe { CoTaskMemFree(pr.cast()) };
    }
    if hr != 0 { 0xEEEE } else { im }
}

fn test(t: &mut Test) {
    co_init(true);
    let e = env_local("s3", C0, "");
    let (_, f) = env_factory(&e);
    let (_, f_unk) = unk_qi(f, &IID_IUnknown);
    let (_, f2) = env_factory(&e);
    let (_, o) = env_create(&e);
    let (_, o2) = env_create(&e);
    let (_, cp) = {
        let mut c: *mut c_void = core::ptr::null_mut();
        // SAFETY: live out pointer; `f` is a proxy.
        let hr = unsafe { CoCopyProxy(f, &mut c) };
        (hr, c)
    };
    let all = |t: &mut Test, what: &str| {
        say!(
            t,
            "{what}: factory {} its IUnknown {} second class object {} object {} second object {} copy {}",
            imp_of(f),
            imp_of(f_unk),
            imp_of(f2),
            imp_of(o),
            imp_of(o2),
            imp_of(cp)
        );
    };
    all(t, "default impersonation levels");
    set_blanket(
        f,
        RPC_C_AUTHN_DEFAULT,
        RPC_C_AUTHZ_DEFAULT,
        core::ptr::null(),
        RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_IMP_LEVEL_IMPERSONATE,
        EOAC_NONE,
    );
    all(t, "after IMPERSONATE on the factory pointer");
    set_blanket(
        f_unk,
        RPC_C_AUTHN_DEFAULT,
        RPC_C_AUTHZ_DEFAULT,
        core::ptr::null(),
        RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_IMP_LEVEL_DELEGATE,
        EOAC_NONE,
    );
    all(t, "after DELEGATE on the IUnknown of the factory");
    set_blanket(
        o,
        RPC_C_AUTHN_DEFAULT,
        RPC_C_AUTHZ_DEFAULT,
        core::ptr::null(),
        RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_IMP_LEVEL_IMPERSONATE,
        EOAC_NONE,
    );
    all(t, "after IMPERSONATE on the first object");
    set_blanket(
        cp,
        RPC_C_AUTHN_DEFAULT,
        RPC_C_AUTHZ_DEFAULT,
        core::ptr::null(),
        RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_IMP_LEVEL_ANONYMOUS,
        EOAC_NONE,
    );
    all(t, "after ANONYMOUS on the copy");
    let (_, f_late) = unk_qi(f, &IID_IClassFactory);
    let (_, unk_late) = unk_qi(f, &IID_IUnknown);
    say!(t, "interfaces queried afterwards: IClassFactory {} IUnknown {}", imp_of(f_late), imp_of(unk_late));
    let mut c2: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer; `f` is a proxy.
    unsafe { CoCopyProxy(f, &mut c2) };
    say!(t, "a copy made now: {}", imp_of(c2));
    for p in [f, f_unk, f2, o, o2, cp, f_late, unk_late, c2] {
        unk_release(p);
    }
    env_end(t, e);
}
