//! CoSuspendClassObjects and the threads: a class registered by one thread and CoSuspendClassObjects called by the same thread or by
//! another one (STA or MTA), and which threads then still find the class, for the local-server and the in-process context.
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
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

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

main_with_child!(server, test);

const CX: &str = "{B2A0D014-0000-4000-8000-706F6C797773}";

static CMD: AtomicU32 = AtomicU32::new(0);
static RESP: AtomicU32 = AtomicU32::new(0);
static RHR: AtomicU32 = AtomicU32::new(0);
static READY: AtomicU32 = AtomicU32::new(0);
static WCTX: AtomicU32 = AtomicU32::new(0);
static WMTA: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn worker(_a: *mut c_void) -> u32 {
    let clsid = guid_from_str(CX);
    co_init(WMTA.load(SeqCst) != 0);
    let ctx = WCTX.load(SeqCst);
    let (_hr, cookie) = reg_class_obj(&clsid, factory_ptr(), ctx, REGCLS_MULTIPLEUSE);
    READY.store(1, SeqCst);
    loop {
        pump(10);
        match CMD.load(SeqCst) {
            1 => {
                CMD.store(0, SeqCst);
                let (hr, p) = get_class_obj(&clsid, ctx, &IID_IClassFactory);
                unk_release(p);
                RHR.store(hr as u32, SeqCst);
                RESP.store(1, SeqCst);
            }
            3 => {
                CMD.store(0, SeqCst);
                // SAFETY: plain call.
                let hr = unsafe { CoSuspendClassObjects() };
                RHR.store(hr as u32, SeqCst);
                RESP.store(1, SeqCst);
            }
            2 => break,
            _ => {}
        }
    }
    revoke(cookie);
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
    0
}

fn ask_cmd(t: &mut Test, what: &str, cmd: u32) {
    RESP.store(0, SeqCst);
    CMD.store(cmd, SeqCst);
    let mut w = 0;
    while RESP.load(SeqCst) == 0 && w < 1500 {
        // SAFETY: a plain sleep.
        unsafe { Sleep(10) };
        w += 10;
    }
    if RESP.load(SeqCst) == 0 {
        say!(t, "  {what}: blocked after 1.5 s");
    } else {
        say!(t, "  {what}: {}", hex8(RHR.load(SeqCst) as i32).as_str());
    }
}

fn main_lookup(t: &mut Test, what: &str, ctx: u32) {
    let (hr, p) = get_class_obj(&guid_from_str(CX), ctx, &IID_IClassFactory);
    unk_release(p);
    say!(t, "  {what}: {}", hex8(hr).as_str());
}

fn test(t: &mut Test) {
    co_init(true);
    for (cn, ctx) in [("local server", CLSCTX_LOCAL_SERVER), ("in-process", CLSCTX_INPROC_SERVER)] {
        for mta in [1u32, 0] {
            say!(t, "{cn}, registering thread is {}", if mta != 0 { "MTA" } else { "STA" });
            CMD.store(0, SeqCst);
            READY.store(0, SeqCst);
            WCTX.store(ctx, SeqCst);
            WMTA.store(mta, SeqCst);
            let h = spawn(worker, 0);
            let mut w = 0;
            while READY.load(SeqCst) == 0 && w < 2000 {
                // SAFETY: a plain sleep.
                unsafe { Sleep(10) };
                w += 10;
            }
            ask_cmd(t, "lookup in the registering thread", 1);
            main_lookup(t, "lookup from the main MTA thread", ctx);
            // SAFETY: plain call.
            let hr = unsafe { CoSuspendClassObjects() };
            say!(t, "  suspend from the main MTA thread {}", hex8(hr).as_str());
            ask_cmd(t, "lookup in the registering thread", 1);
            main_lookup(t, "lookup from the main MTA thread", ctx);
            ask_cmd(t, "suspend from the registering thread", 3);
            ask_cmd(t, "lookup in the registering thread", 1);
            CMD.store(2, SeqCst);
            join(h, 3000);
        }
    }
}
