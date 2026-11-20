//! CoMarshalInterface of an object of the test library into a memory stream: the result for each marshalling context
//! and flag, how far the stream advances, what the first bytes are (the OBJREF of [MS-DCOM] 2.2.18: signature
//! `MEOW`, flags, interface id), the table flags, and the marshalled data's effect on the object's reference count.
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

// ===================================================================== COM helpers (spec-win-com-typelib)
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

// ===================================================================== type-library helpers (spec-win-com-typelib)
// Declarations from Microsoft Learn (oaidl.h, oleauto.h, wtypes.h, objidl.h, objbase.h, winerror.h); the helpers are
// this test's own. A test builds its own small type library with CreateTypeLib2 (ICreateTypeLib2 / ICreateTypeInfo),
// registers it, and talks to objects through interfaces the library describes: a custom `oleautomation` interface
// (`ITlEcho`, IUnknown-derived), a second one used for callbacks (`ITlCallback`) and a dual interface (`ITlDual`).

use core::sync::atomic::AtomicI32;

type BSTR = *mut u16;

const VT_EMPTY: u16 = 0;
const VT_NULL: u16 = 1;
const VT_I2: u16 = 2;
const VT_I4: u16 = 3;
const VT_R8: u16 = 5;
const VT_BSTR: u16 = 8;
const VT_DISPATCH: u16 = 9;
const VT_ERROR: u16 = 10;
const VT_BOOL: u16 = 11;
const VT_VARIANT: u16 = 12;
const VT_UNKNOWN: u16 = 13;
const VT_UI1: u16 = 17;
const VT_HRESULT: u16 = 25;
const VT_PTR: u16 = 26;
const VT_SAFEARRAY: u16 = 27;
const VT_USERDEFINED: u16 = 29;
const VT_ARRAY: u16 = 0x2000;
const VT_BYREF: u16 = 0x4000;

const TKIND_INTERFACE: i32 = 3;
const TKIND_DISPATCH: i32 = 4;
const TKIND_COCLASS: i32 = 5;

const TYPEFLAG_FDUAL: u16 = 0x40;
const TYPEFLAG_FOLEAUTOMATION: u16 = 0x100;
const TYPEFLAG_FDISPATCHABLE: u16 = 0x1000;
const TYPEFLAG_FNONEXTENSIBLE: u16 = 0x80;

const PARAMFLAG_FIN: u16 = 1;
const PARAMFLAG_FOUT: u16 = 2;
const PARAMFLAG_FLCID: u16 = 4;
const PARAMFLAG_FRETVAL: u16 = 8;

const FUNC_PUREVIRTUAL: i32 = 1;
const FUNC_DISPATCH: i32 = 4;
const INVOKE_FUNC: i32 = 1;
const CC_STDCALL: i32 = 4;

const REGKIND_DEFAULT: i32 = 0;
const REGKIND_REGISTER: i32 = 1;
const REGKIND_NONE: i32 = 2;

const SYS_WIN32: i32 = 1;
const SYS_WIN64: i32 = 3;

const MSHCTX_LOCAL: u32 = 0;
const MSHCTX_NOSHAREDMEM: u32 = 1;
const MSHCTX_INPROC: u32 = 4;
const MSHLFLAGS_NORMAL: u32 = 0;
const MSHLFLAGS_TABLESTRONG: u32 = 1;
const MSHLFLAGS_TABLEWEAK: u32 = 2;
const MSHLFLAGS_NOPING: u32 = 4;

const DISPATCH_METHOD: u16 = 1;
const DISPATCH_PROPERTYGET: u16 = 2;

const E_FAIL: i32 = 0x8000_4005_u32 as i32;
const E_NOTIMPL: i32 = 0x8000_4001_u32 as i32;
const E_UNEXPECTED: i32 = 0x8000_FFFF_u32 as i32;
const DISP_E_MEMBERNOTFOUND: i32 = 0x8002_0003_u32 as i32;

/// wtypes.h / oaidl.h `TYPEDESC` (16 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct TYPEDESC {
    p: usize,
    vt: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PARAMDESC {
    ex: usize,
    flags: u16,
}

/// `ELEMDESC` with the `PARAMDESC` arm of its union (32 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct ELEMDESC {
    tdesc: TYPEDESC,
    pd: PARAMDESC,
}

#[repr(C)]
struct FUNCDESC {
    memid: i32,
    scodes: *mut i32,
    params: *mut ELEMDESC,
    funckind: i32,
    invkind: i32,
    callconv: i32,
    c_params: i16,
    c_params_opt: i16,
    ovft: i16,
    c_scodes: i16,
    ret: ELEMDESC,
    flags: u16,
}

#[repr(C)]
struct TYPEATTR {
    guid: GUID,
    lcid: u32,
    reserved: u32,
    memid_ctor: i32,
    memid_dtor: i32,
    schema: *mut u16,
    cb_size_instance: u32,
    typekind: i32,
    c_funcs: u16,
    c_vars: u16,
    c_impl_types: u16,
    cb_size_vft: u16,
    cb_alignment: u16,
    type_flags: u16,
    major: u16,
    minor: u16,
    tdesc_alias: TYPEDESC,
    idl: [usize; 2],
}

#[repr(C)]
struct TLIBATTR {
    guid: GUID,
    lcid: u32,
    syskind: i32,
    major: u16,
    minor: u16,
    flags: u16,
}

/// oaidl.h `VARIANT` (24 bytes on x64): the type, three reserved words and the 8-byte value arm.
#[repr(C)]
#[derive(Clone, Copy)]
struct VARIANT {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
    val: [u64; 2],
}

fn variant_empty() -> VARIANT {
    VARIANT { vt: 0, r1: 0, r2: 0, r3: 0, val: [0; 2] }
}

#[repr(C)]
struct SAFEARRAYBOUND {
    c_elements: u32,
    l_lbound: i32,
}

#[repr(C)]
struct DISPPARAMS {
    rgvarg: *mut VARIANT,
    named: *mut i32,
    c_args: u32,
    c_named: u32,
}

#[repr(C)]
struct EXCEPINFO {
    code: u16,
    reserved: u16,
    source: BSTR,
    description: BSTR,
    help_file: BSTR,
    help_context: u32,
    reserved2: *mut c_void,
    deferred: *mut c_void,
    scode: i32,
}

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateTypeLib2(syskind: i32, file: *const u16, out: *mut *mut c_void) -> i32;
    fn LoadTypeLibEx(file: *const u16, regkind: i32, out: *mut *mut c_void) -> i32;
    fn LoadTypeLib(file: *const u16, out: *mut *mut c_void) -> i32;
    fn LoadRegTypeLib(guid: *const GUID, major: u16, minor: u16, lcid: u32, out: *mut *mut c_void) -> i32;
    fn RegisterTypeLib(tlib: *mut c_void, path: *const u16, help_dir: *const u16) -> i32;
    fn UnRegisterTypeLib(guid: *const GUID, major: u16, minor: u16, lcid: u32, syskind: i32) -> i32;
    fn QueryPathOfRegTypeLib(guid: *const GUID, major: u16, minor: u16, lcid: u32, path: *mut BSTR) -> i32;
    fn SysAllocString(s: *const u16) -> BSTR;
    fn SysAllocStringLen(s: *const u16, n: u32) -> BSTR;
    fn SysAllocStringByteLen(s: *const u8, n: u32) -> BSTR;
    fn SysFreeString(b: BSTR);
    fn SysStringLen(b: BSTR) -> u32;
    fn SysStringByteLen(b: BSTR) -> u32;
    fn VariantInit(v: *mut VARIANT);
    fn VariantClear(v: *mut VARIANT) -> i32;
    fn VariantCopy(dst: *mut VARIANT, src: *const VARIANT) -> i32;
    fn VariantCopyInd(dst: *mut VARIANT, src: *const VARIANT) -> i32;
    fn SafeArrayCreate(vt: u16, dims: u32, bounds: *const SAFEARRAYBOUND) -> *mut c_void;
    fn SafeArrayCreateVector(vt: u16, lbound: i32, n: u32) -> *mut c_void;
    fn SafeArrayDestroy(sa: *mut c_void) -> i32;
    fn SafeArrayGetDim(sa: *mut c_void) -> u32;
    fn SafeArrayGetUBound(sa: *mut c_void, dim: u32, out: *mut i32) -> i32;
    fn SafeArrayGetLBound(sa: *mut c_void, dim: u32, out: *mut i32) -> i32;
    fn SafeArrayAccessData(sa: *mut c_void, data: *mut *mut c_void) -> i32;
    fn SafeArrayUnaccessData(sa: *mut c_void) -> i32;
    fn SafeArrayGetVartype(sa: *mut c_void, vt: *mut u16) -> i32;
    fn CreateErrorInfo(out: *mut *mut c_void) -> i32;
    fn SetErrorInfo(reserved: u32, info: *mut c_void) -> i32;
    fn GetErrorInfo(reserved: u32, out: *mut *mut c_void) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoMarshalInterface(
        stm: *mut c_void,
        iid: *const GUID,
        unk: *mut c_void,
        ctx: u32,
        ctx_ptr: *mut c_void,
        flags: u32,
    ) -> i32;
    fn CoUnmarshalInterface(stm: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32;
    fn CoReleaseMarshalData(stm: *mut c_void) -> i32;
    fn CoGetMarshalSizeMax(
        size: *mut u32,
        iid: *const GUID,
        unk: *mut c_void,
        ctx: u32,
        ctx_ptr: *mut c_void,
        flags: u32,
    ) -> i32;
    fn CoGetStandardMarshal(
        iid: *const GUID,
        unk: *mut c_void,
        ctx: u32,
        ctx_ptr: *mut c_void,
        flags: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn CoGetPSClsid(iid: *const GUID, clsid: *mut GUID) -> i32;
    fn CoRegisterPSClsid(iid: *const GUID, clsid: *const GUID) -> i32;
    fn CreateStreamOnHGlobal(hglobal: *mut c_void, delete: BOOL, out: *mut *mut c_void) -> i32;
    fn GetHGlobalFromStream(stm: *mut c_void, h: *mut *mut c_void) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GlobalLock(h: *mut c_void) -> *mut u8;
    fn GlobalUnlock(h: *mut c_void) -> BOOL;
    fn GlobalSize(h: *mut c_void) -> usize;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegQueryValueExW(
        key: HANDLE,
        name: *const u16,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        n: *mut u32,
    ) -> i32;
    fn RegEnumKeyExW(
        key: HANDLE,
        index: u32,
        name: *mut u16,
        n: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        nc: *mut u32,
        time: *mut u64,
    ) -> i32;
    fn RegEnumValueW(
        key: HANDLE,
        index: u32,
        name: *mut u16,
        n: *mut u32,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        nd: *mut u32,
    ) -> i32;
    fn RegDeleteTreeW(key: HANDLE, sub: *const u16) -> i32;
}

// ---------------------------------------------------------------- calling through a vtable

/// `vcall!(p, index, (ArgTypes...) -> Ret, args...)`: call slot `index` of the vtable of the COM pointer `p`.
macro_rules! vcall {
    ($p:expr, $i:expr, ($($at:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let p: *mut c_void = $p;
        // SAFETY: `p` is a COM interface pointer the test holds; the slot has the stated signature.
        unsafe {
            let slot = *(*(p as *mut *const *const c_void)).add($i);
            let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $r = core::mem::transmute(slot);
            f(p $(, $a)*)
        }
    }};
}

fn hrs(hr: i32) -> Buf<12> {
    hex8(hr)
}

/// The text of a wide string pointer (ASCII only; others as `<xxxx>`).
fn wtext(p: *const u16) -> Buf<600> {
    units_text(wstr_at(p, 500))
}

/// A BSTR's text (`(null)` for a NULL BSTR) and its length in characters.
fn bstr_text(b: BSTR) -> Buf<600> {
    if b.is_null() {
        let mut o = Buf::<600>::new();
        let _ = write!(o, "(null)");
        return o;
    }
    // SAFETY: a BSTR the test holds has its length before it.
    let n = unsafe { SysStringLen(b) } as usize;
    // SAFETY: `n` characters follow the pointer.
    units_text(unsafe { core::slice::from_raw_parts(b, n.min(500)) })
}

fn bstr_new(s: &str) -> BSTR {
    let w = wide(s);
    // SAFETY: a NUL-terminated buffer.
    unsafe { SysAllocString(w.as_ptr()) }
}

fn bstr_free(b: BSTR) {
    // SAFETY: a BSTR the test owns (or NULL).
    unsafe { SysFreeString(b) };
}

// ---------------------------------------------------------------- ids for one run

/// A GUID of this test's own: `B2B0<kind>-<seed>-4000-8000-706F6C797773`.
fn gid(kind: u32, seed: u32) -> GUID {
    guid(0xB2B0_0000 | kind, seed as u16, 0x4000, [0x80, 0x00, 0x70, 0x6F, 0x6C, 0x79, 0x77, 0x73])
}

const K_LIB: u32 = 0xA001;
const K_ECHO: u32 = 0xA002;
const K_CB: u32 = 0xA003;
const K_DUAL: u32 = 0xA004;
const K_CLASS: u32 = 0xA005;
const K_PS: u32 = 0xA006;

fn run_seed() -> u32 {
    // SAFETY: no arguments.
    let pid = unsafe { GetCurrentProcessId() };
    (pid & 0xffff) | 0x1000
}

// ---------------------------------------------------------------- the type library the tests build

/// How a parameter or result is typed in the library.
#[derive(Clone, Copy)]
enum P {
    I4,
    PI4,
    /// `[in] LPWSTR` (VT_LPWSTR)
    Lpwstr,
    /// `hyper*` as an `[out]`
    PI8,
    Bstr,
    PBstr,
    Var,
    PVar,
    SaI4,
    PSaI4,
    PUnk,
    /// `[in] ITlCallback*` (an interface pointer given by reference to a type in this library)
    Itf(u32),
    /// `ITlXxx**` as an `[out]`
    PItf(u32),
}

/// Storage for the TYPEDESC chains of one function while it is added (the chains point into this).
struct FuncStore {
    td: [[TYPEDESC; 3]; 14],
    el: [ELEMDESC; 14],
}

fn td_none() -> TYPEDESC {
    TYPEDESC { p: 0, vt: 0 }
}

fn fill_param(store: &mut FuncStore, i: usize, p: P, flags: u16) {
    let t: *mut [TYPEDESC; 3] = &mut store.td[i];
    let mut a = [td_none(); 3];
    // chains: a[0] points to a[1] points to a[2]; the pointers are made after the values
    match p {
        P::I4 => a[0].vt = VT_I4,
        P::PI4 => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_I4;
        }
        P::Bstr => a[0].vt = VT_BSTR,
        P::Lpwstr => a[0].vt = 31,
        P::PI8 => {
            a[0].vt = VT_PTR;
            a[1].vt = 20;
        }
        P::PBstr => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_BSTR;
        }
        P::Var => a[0].vt = VT_VARIANT,
        P::PVar => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_VARIANT;
        }
        P::SaI4 => {
            a[0].vt = VT_SAFEARRAY;
            a[1].vt = VT_I4;
        }
        P::PSaI4 => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_SAFEARRAY;
            a[2].vt = VT_I4;
        }
        P::PUnk => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_UNKNOWN;
        }
        P::Itf(h) => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_USERDEFINED;
            a[1].p = h as usize;
        }
        P::PItf(h) => {
            a[0].vt = VT_PTR;
            a[1].vt = VT_PTR;
            a[2].vt = VT_USERDEFINED;
            a[2].p = h as usize;
        }
    }
    // SAFETY: `t` points to this function's own storage; the chain pointers stay inside it.
    unsafe {
        *t = a;
        let base = t as *mut TYPEDESC;
        let depth = match p {
            P::I4 | P::Bstr | P::Var | P::SaI4 | P::Lpwstr => 0,
            P::PI4 | P::PBstr | P::PVar | P::PUnk | P::Itf(_) | P::PI8 => 1,
            P::PSaI4 | P::PItf(_) => 2,
        };
        // SaI4: [0]=SAFEARRAY -> [1]=I4 (one link); PSaI4: [0]=PTR -> [1]=SAFEARRAY -> [2]=I4
        let links = match p {
            P::SaI4 => 1,
            _ => depth,
        };
        for k in 0..links {
            (*base.add(k)).p = base.add(k + 1) as usize;
        }
    }
    store.el[i] = ELEMDESC { tdesc: store.td[i][0], pd: PARAMDESC { ex: 0, flags } };
}

/// One function of an interface: memid, name, its parameters (name, type, flags) and whether it is dispatched.
struct Func {
    memid: i32,
    name: &'static str,
    params: &'static [(&'static str, P, u16)],
    dispatch: bool,
    /// INVOKE_FUNC (1) or INVOKE_PROPERTYGET (2)
    inv: i32,
}

fn add_func(cti: *mut c_void, index: u32, f: &Func, vft_base: usize) -> i32 {
    let mut store = FuncStore {
        td: [[td_none(); 3]; 14],
        el: [ELEMDESC { tdesc: td_none(), pd: PARAMDESC { ex: 0, flags: 0 } }; 14],
    };
    for (i, (_, ty, fl)) in f.params.iter().enumerate() {
        fill_param(&mut store, i, *ty, *fl);
    }
    let fd = FUNCDESC {
        memid: f.memid,
        scodes: core::ptr::null_mut(),
        params: store.el.as_mut_ptr(),
        funckind: if f.dispatch { FUNC_DISPATCH } else { FUNC_PUREVIRTUAL },
        invkind: f.inv,
        callconv: CC_STDCALL,
        c_params: f.params.len() as i16,
        c_params_opt: 0,
        ovft: ((vft_base + index as usize) * 8) as i16,
        c_scodes: 0,
        ret: ELEMDESC { tdesc: TYPEDESC { p: 0, vt: VT_HRESULT }, pd: PARAMDESC { ex: 0, flags: 0 } },
        flags: 0,
    };
    // ICreateTypeInfo::AddFuncDesc (slot 9)
    let hr = vcall!(cti, 9, (u32, *const FUNCDESC) -> i32, index, &fd);
    if hr < 0 {
        return hr;
    }
    // SetFuncAndParamNames (slot 15)
    let mut names = [core::ptr::null_mut::<u16>(); 16];
    let mut bufs = [[0u16; 40]; 16];
    let all = core::iter::once(f.name).chain(f.params.iter().map(|p| p.0));
    let mut n = 0;
    for s in all {
        for (k, c) in s.chars().enumerate().take(39) {
            bufs[n][k] = c as u16;
        }
        names[n] = bufs[n].as_mut_ptr();
        n += 1;
    }
    vcall!(cti, 15, (u32, *mut *mut u16, u32) -> i32, index, names.as_mut_ptr(), n as u32)
}

const ECHO_FUNCS: &[Func] = &[
    Func {
        memid: 0x6001,
        name: "AddLong",
        params: &[
            ("a", P::I4, PARAMFLAG_FIN),
            ("b", P::I4, PARAMFLAG_FIN),
            ("r", P::PI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL),
        ],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x6002,
        name: "EchoBstr",
        params: &[("s", P::Bstr, PARAMFLAG_FIN), ("r", P::PBstr, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x6003,
        name: "ConcatBstr",
        params: &[
            ("a", P::Bstr, PARAMFLAG_FIN),
            ("b", P::Bstr, PARAMFLAG_FIN),
            ("r", P::PBstr, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL),
        ],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x6004,
        name: "EchoVariant",
        params: &[("v", P::Var, PARAMFLAG_FIN), ("r", P::PVar, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x6005,
        name: "SumArray",
        params: &[("a", P::SaI4, PARAMFLAG_FIN), ("r", P::PI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x6006,
        name: "MakeArray",
        params: &[("n", P::I4, PARAMFLAG_FIN), ("r", P::PSaI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x6007,
        name: "SwapInOut",
        params: &[("a", P::PI4, PARAMFLAG_FIN | PARAMFLAG_FOUT), ("s", P::PBstr, PARAMFLAG_FIN | PARAMFLAG_FOUT)],
        dispatch: false,
        inv: 1,
    },
    Func { memid: 0x6008, name: "FailHr", params: &[("hr", P::I4, PARAMFLAG_FIN)], dispatch: false, inv: 1 },
    Func { memid: 0x6009, name: "FailInfo", params: &[("desc", P::Bstr, PARAMFLAG_FIN)], dispatch: false, inv: 1 },
    Func {
        memid: 0x600A,
        name: "GetUnknown",
        params: &[("r", P::PUnk, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x600B,
        name: "GetNew",
        params: &[("r", P::PItf(0), PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x600C,
        name: "Notify",
        params: &[("cb", P::Itf(1), PARAMFLAG_FIN), ("v", P::I4, PARAMFLAG_FIN)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x600D,
        name: "GetPid",
        params: &[("r", P::PI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x600E,
        name: "LpwstrLen",
        params: &[("s", P::Lpwstr, PARAMFLAG_FIN), ("r", P::PI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 1,
    },
    Func {
        memid: 0x600F,
        name: "Bytes",
        params: &[("r", P::PI8, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: false,
        inv: 2,
    },
];

const CB_FUNCS: &[Func] =
    &[Func { memid: 0x7001, name: "Run", params: &[("v", P::I4, PARAMFLAG_FIN)], dispatch: false, inv: 1 }];

const DUAL_FUNCS: &[Func] = &[
    Func {
        memid: 1,
        name: "AddLong",
        params: &[
            ("a", P::I4, PARAMFLAG_FIN),
            ("b", P::I4, PARAMFLAG_FIN),
            ("r", P::PI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL),
        ],
        dispatch: true,
        inv: 1,
    },
    Func {
        memid: 2,
        name: "ConcatBstr",
        params: &[
            ("a", P::Bstr, PARAMFLAG_FIN),
            ("b", P::Bstr, PARAMFLAG_FIN),
            ("r", P::PBstr, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL),
        ],
        dispatch: true,
        inv: 1,
    },
    Func {
        memid: 3,
        name: "EchoVariant",
        params: &[("v", P::Var, PARAMFLAG_FIN), ("r", P::PVar, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: true,
        inv: 1,
    },
    Func { memid: 4, name: "FailHr", params: &[("hr", P::I4, PARAMFLAG_FIN)], dispatch: true, inv: 1 },
    Func {
        memid: 5,
        name: "GetPid",
        params: &[("r", P::PI4, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: true,
        inv: 1,
    },
    Func {
        memid: 6,
        name: "Label",
        params: &[("r", P::PBstr, PARAMFLAG_FOUT | PARAMFLAG_FRETVAL)],
        dispatch: true,
        inv: 2,
    },
];

fn wcopy(s: &str) -> [u16; 80] {
    let mut w = [0u16; 80];
    for (i, c) in s.chars().enumerate().take(79) {
        w[i] = c as u16;
    }
    w
}

/// Which interfaces of the library a test wants and how the dual one is made.
#[derive(Clone, Copy)]
struct Shape {
    /// the dual interface is created as TKIND_DISPATCH (like MIDL does) instead of TKIND_INTERFACE
    dual_dispatch: bool,
    /// leave the dual interface out
    no_dual: bool,
    /// the library's version
    ver: (u16, u16),
    /// the library's LCID
    lcid: u32,
    /// the library's flags (SetLibFlags; 0: not called)
    libflags: u16,
    /// the custom interfaces are not flagged `oleautomation`
    no_automation: bool,
}

const SHAPE_DEFAULT: Shape =
    Shape { dual_dispatch: true, no_dual: false, ver: (1, 0), lcid: 0, libflags: 0, no_automation: false };
/// the dual interface as a TKIND_INTERFACE with the dual flag (the form the marshaller accepts from CreateTypeLib2)
const SHAPE_INTERFACE: Shape =
    Shape { dual_dispatch: false, no_dual: false, ver: (1, 0), lcid: 0, libflags: 0, no_automation: false };

/// `ITypeInfo*` of an interface of the standard `stdole` library (IUnknown or IDispatch).
fn stdole_typeinfo(iid: &GUID) -> *mut c_void {
    let mut lib: *mut c_void = core::ptr::null_mut();
    let stdole = guid(0x20430, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    // SAFETY: live out pointer.
    let hr = unsafe { LoadRegTypeLib(&stdole, 2, 0, 0, &mut lib) };
    if hr < 0 {
        return core::ptr::null_mut();
    }
    let mut ti: *mut c_void = core::ptr::null_mut();
    // ITypeLib::GetTypeInfoOfGuid (slot 6)
    vcall!(lib, 6, (*const GUID, *mut *mut c_void) -> i32, iid, &mut ti);
    unk_release(lib);
    ti
}

/// Build the test library at `path` for run `seed`; 0 on success, else the first failing step's HRESULT (with the step
/// in `step`). The library: ITlCallback, ITlEcho (custom, oleautomation) and ITlDual (dual).
fn build_testlib(path: &[u16], seed: u32, shape: Shape, step: &mut u32) -> i32 {
    let mut ctl: *mut c_void = core::ptr::null_mut();
    macro_rules! chk {
        ($n:expr, $e:expr) => {{
            *step = $n;
            let hr: i32 = $e;
            if hr < 0 {
                return hr;
            }
        }};
    }
    // SAFETY: live out pointer, NUL-terminated path.
    chk!(1, unsafe { CreateTypeLib2(SYS_WIN64, path.as_ptr(), &mut ctl) });
    let libid = gid(K_LIB, seed);
    chk!(2, vcall!(ctl, 6, (*const GUID) -> i32, &libid)); // SetGuid
    chk!(3, vcall!(ctl, 5, (u16, u16) -> i32, shape.ver.0, shape.ver.1)); // SetVersion
    let mut nm = wcopy("PolyWsTl");
    chk!(4, vcall!(ctl, 4, (*mut u16) -> i32, nm.as_mut_ptr())); // SetName
    let mut doc = wcopy("Poly test library");
    chk!(5, vcall!(ctl, 7, (*mut u16) -> i32, doc.as_mut_ptr())); // SetDocString
    chk!(6, vcall!(ctl, 10, (u32) -> i32, shape.lcid)); // SetLcid
    if shape.libflags != 0 {
        chk!(8, vcall!(ctl, 11, (u16) -> i32, shape.libflags)); // SetLibFlags
    }
    let unk_ti = stdole_typeinfo(&IID_IUnknown);
    let disp_ti = stdole_typeinfo(&IID_IDispatch);
    if unk_ti.is_null() || disp_ti.is_null() {
        *step = 7;
        return E_FAIL;
    }
    // ITlCallback
    let mut cti_cb: *mut c_void = core::ptr::null_mut();
    let mut n = wcopy("ITlCallback");
    chk!(10, vcall!(ctl, 3, (*mut u16, i32, *mut *mut c_void) -> i32, n.as_mut_ptr(), TKIND_INTERFACE, &mut cti_cb));
    let id = gid(K_CB, seed);
    chk!(11, vcall!(cti_cb, 3, (*const GUID) -> i32, &id));
    let auto = if shape.no_automation { 0 } else { TYPEFLAG_FOLEAUTOMATION };
    chk!(12, vcall!(cti_cb, 4, (u16) -> i32, auto | TYPEFLAG_FNONEXTENSIBLE));
    chk!(13, vcall!(cti_cb, 7, (u16, u16) -> i32, 1, 0));
    let mut href = 0u32;
    chk!(14, vcall!(cti_cb, 8, (*mut c_void, *mut u32) -> i32, unk_ti, &mut href));
    chk!(15, vcall!(cti_cb, 10, (u32, u32) -> i32, 0, href));
    for (i, f) in CB_FUNCS.iter().enumerate() {
        chk!(16, add_func(cti_cb, i as u32, f, 3));
    }
    chk!(17, vcall!(cti_cb, 25, () -> i32));
    // an ITypeInfo of the callback, to refer to it
    let mut cb_ti: *mut c_void = core::ptr::null_mut();
    let (_, _) = (0, 0);
    let hr = unk_qi(cti_cb, &guid(0x20401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]));
    cb_ti = hr.1;
    if cb_ti.is_null() {
        *step = 18;
        return E_FAIL;
    }
    // ITlEcho
    let mut cti_echo: *mut c_void = core::ptr::null_mut();
    let mut n = wcopy("ITlEcho");
    chk!(20, vcall!(ctl, 3, (*mut u16, i32, *mut *mut c_void) -> i32, n.as_mut_ptr(), TKIND_INTERFACE, &mut cti_echo));
    let id = gid(K_ECHO, seed);
    chk!(21, vcall!(cti_echo, 3, (*const GUID) -> i32, &id));
    chk!(22, vcall!(cti_echo, 4, (u16) -> i32, auto | TYPEFLAG_FNONEXTENSIBLE));
    chk!(23, vcall!(cti_echo, 7, (u16, u16) -> i32, 1, 0));
    let mut href_unk = 0u32;
    chk!(24, vcall!(cti_echo, 8, (*mut c_void, *mut u32) -> i32, unk_ti, &mut href_unk));
    chk!(25, vcall!(cti_echo, 10, (u32, u32) -> i32, 0, href_unk));
    let mut href_cb = 0u32;
    chk!(26, vcall!(cti_echo, 8, (*mut c_void, *mut u32) -> i32, cb_ti, &mut href_cb));
    let mut href_self = 0u32;
    let mut self_ti: *mut c_void = core::ptr::null_mut();
    let r = unk_qi(cti_echo, &guid(0x20401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]));
    self_ti = r.1;
    chk!(27, vcall!(cti_echo, 8, (*mut c_void, *mut u32) -> i32, self_ti, &mut href_self));
    for (i, f) in ECHO_FUNCS.iter().enumerate() {
        // the references are resolved here: Itf(1) is the callback, PItf(0) is the interface itself
        let params: [(&str, P, u16); 3] = [("", P::I4, 0); 3];
        let _ = params;
        let mut g = Func { memid: f.memid, name: f.name, params: f.params, dispatch: f.dispatch, inv: f.inv };
        let mapped: [(&'static str, P, u16); 4];
        let mut tmp: [(&'static str, P, u16); 4] = [("", P::I4, 0); 4];
        let mut needs = false;
        for (k, p) in f.params.iter().enumerate().take(4) {
            tmp[k] = *p;
            match p.1 {
                P::Itf(_) => {
                    tmp[k].1 = P::Itf(href_cb);
                    needs = true;
                }
                P::PItf(_) => {
                    tmp[k].1 = P::PItf(href_self);
                    needs = true;
                }
                _ => {}
            }
        }
        mapped = tmp;
        if needs {
            g.params = &[]; // replaced below by the mapped copy
            let r = add_func_params(cti_echo, i as u32, &g, &mapped[..f.params.len()], 3);
            chk!(28, r);
        } else {
            chk!(29, add_func(cti_echo, i as u32, f, 3));
        }
    }
    chk!(30, vcall!(cti_echo, 25, () -> i32));
    // ITlDual
    if !shape.no_dual {
        let mut cti_dual: *mut c_void = core::ptr::null_mut();
        let mut n = wcopy("ITlDual");
        let kind = if shape.dual_dispatch { TKIND_DISPATCH } else { TKIND_INTERFACE };
        chk!(40, vcall!(ctl, 3, (*mut u16, i32, *mut *mut c_void) -> i32, n.as_mut_ptr(), kind, &mut cti_dual));
        let id = gid(K_DUAL, seed);
        chk!(41, vcall!(cti_dual, 3, (*const GUID) -> i32, &id));
        let dflags = if shape.dual_dispatch {
            TYPEFLAG_FDUAL | TYPEFLAG_FDISPATCHABLE | TYPEFLAG_FNONEXTENSIBLE
        } else {
            TYPEFLAG_FDUAL | TYPEFLAG_FOLEAUTOMATION | TYPEFLAG_FDISPATCHABLE | TYPEFLAG_FNONEXTENSIBLE
        };
        chk!(42, vcall!(cti_dual, 4, (u16) -> i32, dflags));
        chk!(43, vcall!(cti_dual, 7, (u16, u16) -> i32, 1, 0));
        let mut href_d = 0u32;
        chk!(44, vcall!(cti_dual, 8, (*mut c_void, *mut u32) -> i32, disp_ti, &mut href_d));
        chk!(45, vcall!(cti_dual, 10, (u32, u32) -> i32, 0, href_d));
        for (i, f) in DUAL_FUNCS.iter().enumerate() {
            let mut g =
                Func { memid: f.memid, name: f.name, params: f.params, dispatch: shape.dual_dispatch, inv: f.inv };
            g.dispatch = shape.dual_dispatch;
            chk!(46, add_func(cti_dual, i as u32, &g, 7));
        }
        chk!(47, vcall!(cti_dual, 25, () -> i32));
        unk_release(cti_dual);
    }
    chk!(50, vcall!(ctl, 12, () -> i32)); // SaveAllChanges
    unk_release(self_ti);
    unk_release(cb_ti);
    unk_release(cti_echo);
    unk_release(cti_cb);
    unk_release(unk_ti);
    unk_release(disp_ti);
    unk_release(ctl);
    *step = 0;
    0
}

/// `add_func` with the parameter list given separately (the references to types of the library resolved).
fn add_func_params(cti: *mut c_void, index: u32, f: &Func, params: &[(&'static str, P, u16)], vft_base: usize) -> i32 {
    let mut store = FuncStore {
        td: [[td_none(); 3]; 14],
        el: [ELEMDESC { tdesc: td_none(), pd: PARAMDESC { ex: 0, flags: 0 } }; 14],
    };
    for (i, (_, ty, fl)) in params.iter().enumerate() {
        fill_param(&mut store, i, *ty, *fl);
    }
    let fd = FUNCDESC {
        memid: f.memid,
        scodes: core::ptr::null_mut(),
        params: store.el.as_mut_ptr(),
        funckind: if f.dispatch { FUNC_DISPATCH } else { FUNC_PUREVIRTUAL },
        invkind: f.inv,
        callconv: CC_STDCALL,
        c_params: params.len() as i16,
        c_params_opt: 0,
        ovft: ((vft_base + index as usize) * 8) as i16,
        c_scodes: 0,
        ret: ELEMDESC { tdesc: TYPEDESC { p: 0, vt: VT_HRESULT }, pd: PARAMDESC { ex: 0, flags: 0 } },
        flags: 0,
    };
    let hr = vcall!(cti, 9, (u32, *const FUNCDESC) -> i32, index, &fd);
    if hr < 0 {
        return hr;
    }
    let mut names = [core::ptr::null_mut::<u16>(); 16];
    let mut bufs = [[0u16; 40]; 16];
    let all = core::iter::once(f.name).chain(params.iter().map(|p| p.0));
    let mut n = 0;
    for s in all {
        for (k, c) in s.chars().enumerate().take(39) {
            bufs[n][k] = c as u16;
        }
        names[n] = bufs[n].as_mut_ptr();
        n += 1;
    }
    vcall!(cti, 15, (u32, *mut *mut u16, u32) -> i32, index, names.as_mut_ptr(), n as u32)
}

// ---------------------------------------------------------------- reading a library back (ITypeLib / ITypeInfo)

/// The documentation of a library (index -1) or of one of its type infos: name and doc string.
fn tl_doc(tlib: *mut c_void, index: i32) -> (i32, Buf<80>, Buf<80>) {
    let mut name: BSTR = core::ptr::null_mut();
    let mut doc: BSTR = core::ptr::null_mut();
    // ITypeLib::GetDocumentation (slot 9)
    let hr = vcall!(tlib, 9, (i32, *mut BSTR, *mut BSTR, *mut u32, *mut BSTR) -> i32, index, &mut name, &mut doc, core::ptr::null_mut(), core::ptr::null_mut());
    let mut a = Buf::<80>::new();
    let mut b = Buf::<80>::new();
    if !name.is_null() {
        let _ = write!(a, "{}", bstr_text(name).as_str());
    } else {
        let _ = write!(a, "(null)");
    }
    if !doc.is_null() {
        let _ = write!(b, "{}", bstr_text(doc).as_str());
    } else {
        let _ = write!(b, "(null)");
    }
    bstr_free(name);
    bstr_free(doc);
    (hr, a, b)
}

/// What the library file looks like through ITypeLib: its attributes and every type info (name, kind, flags, counts).
fn dump_lib(t: &mut Test, tlib: *mut c_void, seed: u32) {
    let mut attr: *mut TLIBATTR = core::ptr::null_mut();
    // ITypeLib::GetLibAttr (slot 7)
    let hr = vcall!(tlib, 7, (*mut *mut TLIBATTR) -> i32, &mut attr);
    if hr == S_OK && !attr.is_null() {
        // SAFETY: the attributes the library returned.
        let a = unsafe { &*attr };
        say!(
            t,
            "  lib attr: guid is this run's {}, lcid {}, syskind {}, version {}.{}, flags {:#x}",
            u32::from(a.guid == gid(K_LIB, seed)),
            a.lcid,
            a.syskind,
            a.major,
            a.minor,
            a.flags
        );
        vcall!(tlib, 12, (*mut TLIBATTR) -> (), attr); // ReleaseTLibAttr
    } else {
        say!(t, "  GetLibAttr = {}", hrs(hr).as_str());
    }
    let (_, name, doc) = tl_doc(tlib, -1);
    say!(t, "  lib name {} doc {}", name.as_str(), doc.as_str());
    let n = vcall!(tlib, 3, () -> u32); // GetTypeInfoCount
    say!(t, "  type infos: {n}");
    for i in 0..n {
        let mut ti: *mut c_void = core::ptr::null_mut();
        let hr = vcall!(tlib, 4, (u32, *mut *mut c_void) -> i32, i, &mut ti);
        let (_, nm, _) = tl_doc(tlib, i as i32);
        if hr != S_OK {
            say!(t, "  [{i}] {}: GetTypeInfo = {}", nm.as_str(), hrs(hr).as_str());
            continue;
        }
        let mut ta: *mut TYPEATTR = core::ptr::null_mut();
        vcall!(ti, 3, (*mut *mut TYPEATTR) -> i32, &mut ta); // GetTypeAttr
        if !ta.is_null() {
            // SAFETY: the attributes the type info returned.
            let a = unsafe { &*ta };
            say!(
                t,
                "  [{i}] {}: kind {}, flags {:#x}, funcs {}, impl types {}, vft size {}, version {}.{}",
                nm.as_str(),
                a.typekind,
                a.type_flags,
                a.c_funcs,
                a.c_impl_types,
                a.cb_size_vft,
                a.major,
                a.minor
            );
            vcall!(ti, 19, (*mut TYPEATTR) -> (), ta); // ReleaseTypeAttr
        }
        unk_release(ti);
    }
}

// ---------------------------------------------------------------- registry reads

fn reg_open(root: HANDLE, sub: &str) -> HANDLE {
    let mut path = Buf::<300>::new();
    let _ = write!(path, "SOFTWARE\\Classes\\{sub}");
    let p = wsz(path.as_str());
    let mut key: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer, NUL-terminated path.
    let r = unsafe { RegOpenKeyExW(root, p.as_ptr(), 0, 0x20019, &mut key) };
    if r == 0 { key } else { core::ptr::null_mut() }
}

fn reg_exists(root: HANDLE, sub: &str) -> bool {
    let k = reg_open(root, sub);
    if k.is_null() {
        return false;
    }
    // SAFETY: a key opened above.
    unsafe { RegCloseKey(k) };
    true
}

/// The value `name` (None: the default value) of `HKxx\SOFTWARE\Classes\<sub>` as text, or None.
fn reg_get(root: HANDLE, sub: &str, name: Option<&str>) -> Option<Buf<300>> {
    let k = reg_open(root, sub);
    if k.is_null() {
        return None;
    }
    let n = name.map(wsz);
    let mut data = [0u8; 600];
    let mut len = 600u32;
    let mut ty = 0u32;
    // SAFETY: live out pointers; the key was opened above and is closed here.
    let r = unsafe {
        let r = RegQueryValueExW(
            k,
            n.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            core::ptr::null_mut(),
            &mut ty,
            data.as_mut_ptr(),
            &mut len,
        );
        RegCloseKey(k);
        r
    };
    if r != 0 {
        return None;
    }
    let mut out = Buf::<300>::new();
    if ty == 1 || ty == 2 {
        let units =
            unsafe { core::slice::from_raw_parts(data.as_ptr() as *const u16, (len as usize / 2).saturating_sub(1)) };
        let _ = write!(out, "{}", units_text(units).as_str());
    } else if ty == 4 {
        let _ = write!(out, "dword {}", u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
    } else {
        let _ = write!(out, "type {ty} len {len}");
    }
    Some(out)
}

/// The names of the subkeys of `HKxx\SOFTWARE\Classes\<sub>` (up to 8, sorted by the enumeration order).
fn reg_subkeys(root: HANDLE, sub: &str) -> ([Buf<80>; 8], usize) {
    let mut out = [const { Buf::<80>::new() }; 8];
    let k = reg_open(root, sub);
    if k.is_null() {
        return (out, 0);
    }
    let mut i = 0usize;
    while i < 8 {
        let mut name = [0u16; 128];
        let mut n = 128u32;
        // SAFETY: live out pointers; the key was opened above.
        let r = unsafe {
            RegEnumKeyExW(
                k,
                i as u32,
                name.as_mut_ptr(),
                &mut n,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        if r != 0 {
            break;
        }
        let _ = write!(out[i], "{}", units_text(&name[..n as usize]).as_str());
        i += 1;
    }
    // SAFETY: the key opened above.
    unsafe { RegCloseKey(k) };
    (out, i)
}

/// The names of the values of a key (up to 12).
fn reg_values(root: HANDLE, sub: &str) -> ([Buf<40>; 12], usize) {
    let mut out = [const { Buf::<40>::new() }; 12];
    let k = reg_open(root, sub);
    if k.is_null() {
        return (out, 0);
    }
    let mut i = 0usize;
    while i < 12 {
        let mut name = [0u16; 128];
        let mut n = 128u32;
        // SAFETY: live out pointers; the key was opened above.
        let r = unsafe {
            RegEnumValueW(
                k,
                i as u32,
                name.as_mut_ptr(),
                &mut n,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        if r != 0 {
            break;
        }
        let _ = write!(out[i], "{}", units_text(&name[..n as usize]).as_str());
        i += 1;
    }
    // SAFETY: the key opened above.
    unsafe { RegCloseKey(k) };
    (out, i)
}

fn reg_delete_tree(root: HANDLE, sub: &str) {
    let mut path = Buf::<300>::new();
    let _ = write!(path, "SOFTWARE\\Classes\\{sub}");
    let p = wsz(path.as_str());
    // SAFETY: NUL-terminated key path.
    unsafe { RegDeleteTreeW(root, p.as_ptr()) };
}

fn guid_braced(g: &GUID) -> Buf<48> {
    guid_text(g)
}

/// Remove everything the tests of run `seed` may have registered (type library, interfaces, classes) from HKLM and HKCU.
fn tl_cleanup(seed: u32) {
    for root in [HKLM, HKCU] {
        let mut k = Buf::<80>::new();
        let _ = write!(k, "TypeLib\\{}", guid_braced(&gid(K_LIB, seed)).as_str());
        reg_delete_tree(root, k.as_str());
        for kind in [K_ECHO, K_CB, K_DUAL] {
            let mut k = Buf::<80>::new();
            let _ = write!(k, "Interface\\{}", guid_braced(&gid(kind, seed)).as_str());
            reg_delete_tree(root, k.as_str());
        }
        let mut k = Buf::<80>::new();
        let _ = write!(k, "CLSID\\{}", guid_braced(&gid(K_CLASS, seed)).as_str());
        reg_delete_tree(root, k.as_str());
        let mut k = Buf::<80>::new();
        let _ = write!(k, "CLSID\\{}", guid_braced(&gid(K_PS, seed)).as_str());
        reg_delete_tree(root, k.as_str());
    }
}

/// The path of a file beside the image for run `seed` (`polyws-tl-<seed>.<ext>`).
fn tl_file(ext: &str) -> [u16; 700] {
    tl_file_for(run_seed(), ext)
}

fn tl_file_for(seed: u32, ext: &str) -> [u16; 700] {
    let mut n = Buf::<80>::new();
    let _ = write!(n, "polyws-tl-{seed}.{ext}");
    beside(n.as_str())
}

/// Append raw bytes to a file (made if missing).
fn append_bytes(path: &[u16], data: &[u8]) {
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
        WriteFile(h, data.as_ptr(), data.len() as u32, &mut put, core::ptr::null_mut());
        CloseHandle(h);
    }
}

// ===================================================================== objects of the test library (spec-win-com-typelib)
// The server side of ITlEcho / ITlDual (and a client-side ITlCallback), written to the interfaces the test library
// describes. The server is the test's own image started with `--child tls <tag> clsid={..} seed=<n> [flags]`.

#[repr(C)]
struct EchoVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    addref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    add_long: unsafe extern "system" fn(*mut c_void, i32, i32, *mut i32) -> i32,
    echo_bstr: unsafe extern "system" fn(*mut c_void, BSTR, *mut BSTR) -> i32,
    concat: unsafe extern "system" fn(*mut c_void, BSTR, BSTR, *mut BSTR) -> i32,
    echo_variant: unsafe extern "system" fn(*mut c_void, *const VARIANT, *mut VARIANT) -> i32,
    sum_array: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> i32,
    make_array: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> i32,
    swap: unsafe extern "system" fn(*mut c_void, *mut i32, *mut BSTR) -> i32,
    fail_hr: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    fail_info: unsafe extern "system" fn(*mut c_void, BSTR) -> i32,
    get_unknown: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    get_new: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    notify: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    get_pid: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
    lpwstr_len: unsafe extern "system" fn(*mut c_void, *const u16, *mut i32) -> i32,
    bytes: unsafe extern "system" fn(*mut c_void, *mut i64) -> i32,
}

#[repr(C)]
struct DualVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    addref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    type_info_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    type_info: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32,
    ids_of_names: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut u16, u32, u32, *mut i32) -> i32,
    invoke: unsafe extern "system" fn(
        *mut c_void,
        i32,
        *const GUID,
        u32,
        u16,
        *mut DISPPARAMS,
        *mut VARIANT,
        *mut EXCEPINFO,
        *mut u32,
    ) -> i32,
    add_long: unsafe extern "system" fn(*mut c_void, i32, i32, *mut i32) -> i32,
    concat: unsafe extern "system" fn(*mut c_void, BSTR, BSTR, *mut BSTR) -> i32,
    echo_variant: unsafe extern "system" fn(*mut c_void, *const VARIANT, *mut VARIANT) -> i32,
    fail_hr: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    get_pid: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
    label: unsafe extern "system" fn(*mut c_void, *mut BSTR) -> i32,
}

/// One object of the server: the ITlEcho interface pointer, the ITlDual one, the reference count.
#[repr(C)]
struct EObj {
    echo: *const EchoVtbl,
    dual: *const DualVtbl,
    refs: AtomicU32,
}

static SEED: AtomicU32 = AtomicU32::new(0);
static DUAL_TI: Sync_<usize> = Sync_(UnsafeCell::new(0));
static E_LIVE: AtomicU32 = AtomicU32::new(0);

fn e_seed() -> u32 {
    SEED.load(SeqCst)
}

unsafe extern "system" fn e_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32 {
    // SAFETY: COM passes valid pointers; `this` is the ITlEcho pointer of an EObj.
    unsafe {
        let g = &*iid;
        let o = this as *mut EObj;
        if *g == IID_IUnknown || *g == gid(K_ECHO, e_seed()) {
            *out = this;
        } else if *g == IID_IDispatch || *g == gid(K_DUAL, e_seed()) {
            *out = core::ptr::addr_of_mut!((*o).dual) as *mut c_void;
        } else {
            *out = core::ptr::null_mut();
            return E_NOINTERFACE;
        }
        (*o).refs.fetch_add(1, SeqCst);
        S_OK
    }
}

unsafe extern "system" fn e_addref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is an EObj.
    unsafe { (*(this as *mut EObj)).refs.fetch_add(1, SeqCst) + 1 }
}

unsafe extern "system" fn e_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is an EObj; the last release frees it.
    unsafe {
        let n = (*(this as *mut EObj)).refs.fetch_sub(1, SeqCst) - 1;
        if n == 0 {
            CoTaskMemFree(this);
            let left = E_LIVE.fetch_sub(1, SeqCst) - 1;
            ssay!("object released live={left}");
        }
        n
    }
}

unsafe extern "system" fn d_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32 {
    // SAFETY: `this` is the dual interface pointer of an EObj (8 bytes into it).
    unsafe { e_qi((this as *mut u8).sub(8) as *mut c_void, iid, out) }
}

unsafe extern "system" fn d_addref(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { e_addref((this as *mut u8).sub(8) as *mut c_void) }
}

unsafe extern "system" fn d_release(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { e_release((this as *mut u8).sub(8) as *mut c_void) }
}

unsafe extern "system" fn m_add_long(_t: *mut c_void, a: i32, b: i32, r: *mut i32) -> i32 {
    ssay!("AddLong {a} {b}");
    // SAFETY: the stub passes a valid out pointer.
    unsafe { *r = a.wrapping_add(b) };
    S_OK
}

unsafe extern "system" fn m_echo_bstr(_t: *mut c_void, s: BSTR, r: *mut BSTR) -> i32 {
    // SAFETY: the stub passes a valid out pointer; the BSTR's length is before it.
    unsafe {
        if s.is_null() {
            ssay!("EchoBstr null");
            *r = core::ptr::null_mut();
        } else {
            let n = SysStringLen(s);
            let bl = SysStringByteLen(s);
            ssay!("EchoBstr len={n} bytelen={bl}");
            *r = SysAllocStringLen(s, n);
        }
    }
    S_OK
}

unsafe extern "system" fn m_concat(_t: *mut c_void, a: BSTR, b: BSTR, r: *mut BSTR) -> i32 {
    // SAFETY: the stub passes valid BSTRs (or NULL) and a valid out pointer.
    unsafe {
        let la = if a.is_null() { 0 } else { SysStringLen(a) } as usize;
        let lb = if b.is_null() { 0 } else { SysStringLen(b) } as usize;
        ssay!("ConcatBstr {la} {lb}");
        let mut buf = [0u16; 400];
        for i in 0..la.min(190) {
            buf[i] = *a.add(i);
        }
        for i in 0..lb.min(190) {
            buf[la.min(190) + i] = *b.add(i);
        }
        *r = SysAllocStringLen(buf.as_ptr(), (la.min(190) + lb.min(190)) as u32);
    }
    S_OK
}

unsafe extern "system" fn m_echo_variant(_t: *mut c_void, v: *const VARIANT, r: *mut VARIANT) -> i32 {
    // SAFETY: the stub passes valid pointers.
    unsafe {
        ssay!("EchoVariant vt={:#x}", (*v).vt);
        VariantInit(r);
        VariantCopyInd(r, v)
    }
}

unsafe extern "system" fn m_sum_array(_t: *mut c_void, a: *mut c_void, r: *mut i32) -> i32 {
    // SAFETY: the stub passes a valid array (or NULL) and out pointer.
    unsafe {
        if a.is_null() {
            ssay!("SumArray null");
            *r = -1;
            return S_OK;
        }
        let (mut lo, mut hi) = (0i32, 0i32);
        SafeArrayGetLBound(a, 1, &mut lo);
        SafeArrayGetUBound(a, 1, &mut hi);
        let mut data: *mut c_void = core::ptr::null_mut();
        SafeArrayAccessData(a, &mut data);
        let mut sum = 0i32;
        for i in 0..(hi - lo + 1).max(0) {
            sum = sum.wrapping_add(*(data as *const i32).add(i as usize));
        }
        SafeArrayUnaccessData(a);
        ssay!("SumArray lbound={lo} ubound={hi} sum={sum}");
        *r = sum;
    }
    S_OK
}

unsafe extern "system" fn m_make_array(_t: *mut c_void, n: i32, r: *mut *mut c_void) -> i32 {
    // SAFETY: the stub passes a valid out pointer.
    unsafe {
        ssay!("MakeArray {n}");
        let sa = SafeArrayCreateVector(VT_I4, 5, n as u32);
        let mut data: *mut c_void = core::ptr::null_mut();
        SafeArrayAccessData(sa, &mut data);
        for i in 0..n {
            *(data as *mut i32).add(i as usize) = i * 10;
        }
        SafeArrayUnaccessData(sa);
        *r = sa;
    }
    S_OK
}

unsafe extern "system" fn m_swap(_t: *mut c_void, a: *mut i32, s: *mut BSTR) -> i32 {
    // SAFETY: the stub passes valid in/out pointers.
    unsafe {
        let old = *a;
        let ls = if (*s).is_null() { 0 } else { SysStringLen(*s) } as usize;
        ssay!("SwapInOut a={old} len={ls}");
        *a = old + 1;
        let mut buf = [0u16; 200];
        for i in 0..ls.min(190) {
            buf[i] = *(*s).add(i);
        }
        buf[ls.min(190)] = u16::from(b'!');
        SysFreeString(*s);
        *s = SysAllocStringLen(buf.as_ptr(), ls.min(190) as u32 + 1);
    }
    S_OK
}

unsafe extern "system" fn m_fail_hr(_t: *mut c_void, hr: i32) -> i32 {
    ssay!("FailHr {}", hex8(hr).as_str());
    hr
}

unsafe extern "system" fn m_fail_info(_t: *mut c_void, desc: BSTR) -> i32 {
    ssay!("FailInfo");
    let mut cei: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let hr = unsafe { CreateErrorInfo(&mut cei) };
    if hr == S_OK {
        let src = bstr_new("PolyWsTl.Server");
        let hf = bstr_new("polyws.hlp");
        let g = gid(K_ECHO, e_seed());
        vcall!(cei, 3, (*const GUID) -> i32, &g); // SetGUID
        // SAFETY: wide strings the call copies.
        vcall!(cei, 4, (*const u16) -> i32, src); // SetSource
        vcall!(cei, 5, (*const u16) -> i32, desc); // SetDescription
        vcall!(cei, 6, (*const u16) -> i32, hf); // SetHelpFile
        vcall!(cei, 7, (u32) -> i32, 4711); // SetHelpContext
        bstr_free(src);
        bstr_free(hf);
        let (_, ei) = unk_qi(cei, &guid(0x1CF2B120, 0x547D, 0x101B, [0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19]));
        // SAFETY: the error info object made above.
        unsafe { SetErrorInfo(0, ei) };
        unk_release(ei);
        unk_release(cei);
    }
    E_FAIL
}

fn new_eobj() -> *mut EObj {
    // SAFETY: the memory is ours and written before use.
    unsafe {
        let p = CoTaskMemAlloc(core::mem::size_of::<EObj>()) as *mut EObj;
        p.write(EObj { echo: &ECHO_VTBL, dual: &DUAL_VTBL, refs: AtomicU32::new(1) });
        E_LIVE.fetch_add(1, SeqCst);
        p
    }
}

unsafe extern "system" fn m_get_unknown(_t: *mut c_void, r: *mut *mut c_void) -> i32 {
    ssay!("GetUnknown");
    let o = new_eobj();
    // SAFETY: the stub passes a valid out pointer; the object's one reference goes to the caller.
    unsafe { *r = o as *mut c_void };
    S_OK
}

unsafe extern "system" fn m_get_new(_t: *mut c_void, r: *mut *mut c_void) -> i32 {
    ssay!("GetNew");
    let o = new_eobj();
    // SAFETY: as above.
    unsafe { *r = o as *mut c_void };
    S_OK
}

unsafe extern "system" fn m_notify(_t: *mut c_void, cb: *mut c_void, v: i32) -> i32 {
    if cb.is_null() {
        ssay!("Notify null");
        return S_OK;
    }
    ssay!("Notify {v}");
    let r1 = vcall!(cb, 3, (i32) -> i32, v);
    ssay!("  Run({v}) -> {}", hex8(r1).as_str());
    let r2 = vcall!(cb, 3, (i32) -> i32, v + 1);
    ssay!("  Run({}) -> {}", v + 1, hex8(r2).as_str());
    if r1 < 0 { r1 } else { r2 }
}

unsafe extern "system" fn m_get_pid(_t: *mut c_void, r: *mut i32) -> i32 {
    // SAFETY: the stub passes a valid out pointer.
    unsafe { *r = GetCurrentProcessId() as i32 };
    S_OK
}

unsafe extern "system" fn m_lpwstr_len(_t: *mut c_void, s: *const u16, r: *mut i32) -> i32 {
    // SAFETY: the stub passes a valid out pointer; the string is NUL-terminated or NULL.
    unsafe {
        if s.is_null() {
            ssay!("LpwstrLen null");
            *r = -1;
        } else {
            let n = wstr_at(s, 100_000).len();
            ssay!("LpwstrLen {n}");
            *r = n as i32;
        }
    }
    S_OK
}

unsafe extern "system" fn m_bytes(_t: *mut c_void, r: *mut i64) -> i32 {
    ssay!("Bytes");
    // SAFETY: the stub passes a valid out pointer.
    unsafe { *r = 0x1_0000_0005 };
    S_OK
}

unsafe extern "system" fn m_label(_t: *mut c_void, r: *mut BSTR) -> i32 {
    ssay!("Label");
    // SAFETY: the stub passes a valid out pointer.
    unsafe { *r = bstr_new("dual label") };
    S_OK
}

unsafe extern "system" fn d_type_info_count(_t: *mut c_void, n: *mut u32) -> i32 {
    // SAFETY: valid out pointer.
    unsafe { *n = 1 };
    S_OK
}

unsafe extern "system" fn d_type_info(_t: *mut c_void, i: u32, _lcid: u32, out: *mut *mut c_void) -> i32 {
    // SAFETY: valid out pointer; DUAL_TI is set before the server serves.
    unsafe {
        if i != 0 {
            *out = core::ptr::null_mut();
            return 0x8002_000B_u32 as i32; // DISP_E_BADINDEX
        }
        let ti = *DUAL_TI.0.get() as *mut c_void;
        unk_addref(ti);
        *out = ti;
    }
    S_OK
}

unsafe extern "system" fn d_ids_of_names(
    _t: *mut c_void,
    _iid: *const GUID,
    names: *mut *mut u16,
    n: u32,
    _lcid: u32,
    ids: *mut i32,
) -> i32 {
    // SAFETY: DUAL_TI is set before the server serves.
    let ti = unsafe { *DUAL_TI.0.get() } as *mut c_void;
    vcall!(ti, 10, (*mut *mut u16, u32, *mut i32) -> i32, names, n, ids)
}

unsafe extern "system" fn d_invoke(
    this: *mut c_void,
    id: i32,
    _iid: *const GUID,
    _lcid: u32,
    flags: u16,
    params: *mut DISPPARAMS,
    result: *mut VARIANT,
    excep: *mut EXCEPINFO,
    arg_err: *mut u32,
) -> i32 {
    ssay!("Invoke id={id} flags={flags}");
    // SAFETY: DUAL_TI is set before the server serves.
    let ti = unsafe { *DUAL_TI.0.get() } as *mut c_void;
    let hr = vcall!(ti, 11, (*mut c_void, i32, u16, *mut DISPPARAMS, *mut VARIANT, *mut EXCEPINFO, *mut u32) -> i32, this, id, flags, params, result, excep, arg_err);
    ssay!("  Invoke -> {}", hex8(hr).as_str());
    hr
}

static ECHO_VTBL: EchoVtbl = EchoVtbl {
    qi: e_qi,
    addref: e_addref,
    release: e_release,
    add_long: m_add_long,
    echo_bstr: m_echo_bstr,
    concat: m_concat,
    echo_variant: m_echo_variant,
    sum_array: m_sum_array,
    make_array: m_make_array,
    swap: m_swap,
    fail_hr: m_fail_hr,
    fail_info: m_fail_info,
    get_unknown: m_get_unknown,
    get_new: m_get_new,
    notify: m_notify,
    get_pid: m_get_pid,
    lpwstr_len: m_lpwstr_len,
    bytes: m_bytes,
};

static DUAL_VTBL: DualVtbl = DualVtbl {
    qi: d_qi,
    addref: d_addref,
    release: d_release,
    type_info_count: d_type_info_count,
    type_info: d_type_info,
    ids_of_names: d_ids_of_names,
    invoke: d_invoke,
    add_long: m_add_long,
    concat: m_concat,
    echo_variant: m_echo_variant,
    fail_hr: m_fail_hr,
    get_pid: m_get_pid,
    label: m_label,
};

// ---- the class factory of the server

unsafe extern "system" fn tf_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32 {
    // SAFETY: valid pointers from COM.
    unsafe {
        let g = &*iid;
        if *g == IID_IUnknown || *g == IID_IClassFactory {
            *out = this;
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn tf_addref(_this: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn tf_release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn tf_create(
    _this: *mut c_void,
    outer: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> i32 {
    // SAFETY: valid pointers from COM.
    unsafe {
        if !outer.is_null() {
            *out = core::ptr::null_mut();
            return 0x8004_0110_u32 as i32;
        }
        let o = new_eobj();
        let hr = e_qi(o as *mut c_void, iid, out);
        e_release(o as *mut c_void);
        ssay!("create {} -> {}", iid_name(&*iid).as_str(), hex8(hr).as_str());
        hr
    }
}

unsafe extern "system" fn tf_lock(_this: *mut c_void, _lock: BOOL) -> i32 {
    S_OK
}

static TF_VTBL: IClassFactoryVtbl = IClassFactoryVtbl {
    base: IUnknownVtbl { qi: tf_qi, addref: tf_addref, release: tf_release },
    create: tf_create,
    lock: tf_lock,
};
static TFACTORY: Factory = Factory { vtbl: &TF_VTBL, refs: AtomicU32::new(1) };

/// The server process: loads the registered library, registers the factory for the class of this run (local server,
/// multiple use) and serves until the quit file appears (or `life` ms pass).
///   sta | mta   the apartment (default mta);  life=<ms> (default 8000);  seed=<n> the run's ids
fn tl_server(args: &str) -> u32 {
    let mut words = args.split(' ');
    let _tls = words.next();
    let tag = words.next().unwrap_or("x");
    let rest = args.splitn(3, ' ').nth(2).unwrap_or("");
    let seed = flag_num(rest, "seed", 0);
    SEED.store(seed, SeqCst);
    let clsid = gid(K_CLASS, seed);
    // SAFETY: nothing else runs yet.
    unsafe {
        *SRV_LOG.0.get() = srv_log_path(tag);
    }
    ssay!("start");
    let sta = has_flag(rest, "sta");
    let hr = co_init(!sta);
    ssay!("init {} {}", if sta { "sta" } else { "mta" }, hex8(hr).as_str());
    // the dual interface's type info, for IDispatch
    let mut lib: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let hr = unsafe { LoadRegTypeLib(&gid(K_LIB, seed), 1, 0, 0, &mut lib) };
    ssay!("LoadRegTypeLib -> {}", hex8(hr).as_str());
    if hr == S_OK {
        let mut ti: *mut c_void = core::ptr::null_mut();
        let hr = vcall!(lib, 6, (*const GUID, *mut *mut c_void) -> i32, &gid(K_DUAL, seed), &mut ti);
        ssay!("dual type info -> {}", hex8(hr).as_str());
        // SAFETY: set once, before the factory is registered.
        unsafe { *DUAL_TI.0.get() = ti as usize };
        unk_release(lib);
    }
    let (hr, cookie) =
        reg_class_obj(&clsid, &TFACTORY as *const Factory as *mut c_void, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE);
    // only a failure is logged: a client may already be served (and log its calls) before this line could be written
    if hr != S_OK {
        ssay!("register failed -> {}", hex8(hr).as_str());
    }
    let life = flag_num(rest, "life", 8000);
    let quit = quit_path(tag);
    let mut waited = 0u32;
    loop {
        if sta {
            pump(10);
        } else {
            // SAFETY: a plain sleep.
            unsafe { Sleep(10) };
        }
        waited += 10;
        if file_exists(&quit) || waited >= life {
            break;
        }
    }
    let hr = revoke(cookie);
    ssay!("revoke -> {}", hex8(hr).as_str());
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
    ssay!("exit");
    0
}

// ---------------------------------------------------------------- client side: the callback object and call helpers

#[repr(C)]
struct CbVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    addref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    run: unsafe extern "system" fn(*mut c_void, i32) -> i32,
}

static CB_REFS: AtomicU32 = AtomicU32::new(0);
static CB_CALLS: AtomicU32 = AtomicU32::new(0);
static CB_LAST: AtomicI32 = AtomicI32::new(0);
static CB_TID: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn cb_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32 {
    // SAFETY: valid pointers from COM.
    unsafe {
        let g = &*iid;
        if *g == IID_IUnknown || *g == gid(K_CB, e_seed()) {
            *out = this;
            CB_REFS.fetch_add(1, SeqCst);
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn cb_addref(_t: *mut c_void) -> u32 {
    CB_REFS.fetch_add(1, SeqCst) + 1
}

unsafe extern "system" fn cb_release(_t: *mut c_void) -> u32 {
    CB_REFS.fetch_sub(1, SeqCst) - 1
}

unsafe extern "system" fn cb_run(_t: *mut c_void, v: i32) -> i32 {
    CB_CALLS.fetch_add(1, SeqCst);
    CB_LAST.store(v, SeqCst);
    CB_TID.store(tid(), SeqCst);
    if v == 99 { 0x8004_0300_u32 as i32 } else { S_OK }
}

static CB_VTBL: CbVtbl = CbVtbl { qi: cb_qi, addref: cb_addref, release: cb_release, run: cb_run };
/// The one callback object of the client: a static with a vtable pointer.
static CB_OBJ: Sync_<[usize; 1]> = Sync_(UnsafeCell::new([0]));

fn cb_ptr() -> *mut c_void {
    // SAFETY: the cell holds the vtable pointer, set here before the object is handed out.
    unsafe {
        (*CB_OBJ.0.get())[0] = &CB_VTBL as *const CbVtbl as usize;
        CB_OBJ.0.get() as *mut c_void
    }
}

/// The class registrations of a run: the class of the server, `LocalServer32` = this image as a `tls` server.
struct TlRun {
    seed: u32,
    tag: &'static str,
    clsid: GUID,
    path: [u16; 700],
}

/// Build and register the library of this run (REGKIND via RegisterTypeLib) and register the server's class under
/// HKLM; 0 on success, else the failing step.
fn tl_setup(seed: u32, tag: &'static str, shape: Shape) -> Result<TlRun, (u32, i32)> {
    SEED.store(seed, SeqCst);
    let path = tl_file("tlb");
    let mut step = 0;
    let hr = build_testlib(&path, seed, shape, &mut step);
    if hr < 0 {
        return Err((step, hr));
    }
    let mut lib: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let hr = unsafe { LoadTypeLibEx(path.as_ptr(), REGKIND_NONE, &mut lib) };
    if hr < 0 {
        return Err((100, hr));
    }
    // SAFETY: the library and path of this run.
    let hr = unsafe { RegisterTypeLib(lib, path.as_ptr(), core::ptr::null()) };
    unk_release(lib);
    if hr < 0 {
        return Err((101, hr));
    }
    let clsid = gid(K_CLASS, seed);
    let cs = guid_braced(&clsid);
    let cmd = server_command_tls(tag, seed);
    reg_class_local(HKLM, cs.as_str(), "polyws-com-typelib", cmd.as_str(), None);
    Ok(TlRun { seed, tag, clsid, path })
}

/// The command line of the `tls` server: `"<exe>" --child tls <tag> seed=<n>`.
fn server_command_tls(tag: &str, seed: u32) -> Buf<500> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut b = Buf::<500>::new();
    let _ = write!(b, "\"");
    for &u in &exe[..n] {
        let _ = write!(b, "{}", u as u8 as char);
    }
    let _ = write!(b, "\" --child tls {tag} seed={seed}");
    b
}

impl TlRun {
    /// Undo everything: unregister the library, remove the class and the interface keys, delete the file.
    fn teardown(&self) {
        // SAFETY: plain call.
        unsafe { UnRegisterTypeLib(&gid(K_LIB, self.seed), 1, 0, 0, SYS_WIN64) };
        tl_cleanup(self.seed);
        delete_file(&self.path);
        cleanup_tag(self.tag);
    }
}

/// `CoCreateInstance(class of the run, LOCAL_SERVER, iid)`.
fn tl_create(run: &TlRun, iid: &GUID) -> (i32, *mut c_void) {
    create_inst(&run.clsid, CLSCTX_LOCAL_SERVER, iid)
}

// ---- method calls through ITlEcho / ITlDual interface pointers (slots of the vtable)

fn echo_add_long(p: *mut c_void, a: i32, b: i32) -> (i32, i32) {
    let mut r = 0x5A5A;
    (vcall!(p, 3, (i32, i32, *mut i32) -> i32, a, b, &mut r), r)
}

fn echo_bstr(p: *mut c_void, s: BSTR) -> (i32, BSTR) {
    let mut r: BSTR = 0x1111 as BSTR;
    (vcall!(p, 4, (BSTR, *mut BSTR) -> i32, s, &mut r), r)
}

fn echo_concat(p: *mut c_void, a: BSTR, b: BSTR) -> (i32, BSTR) {
    let mut r: BSTR = 0x1111 as BSTR;
    (vcall!(p, 5, (BSTR, BSTR, *mut BSTR) -> i32, a, b, &mut r), r)
}

fn echo_variant(p: *mut c_void, v: &VARIANT) -> (i32, VARIANT) {
    let mut r = variant_empty();
    r.vt = 0x7777;
    (vcall!(p, 6, (*const VARIANT, *mut VARIANT) -> i32, v, &mut r), r)
}

fn echo_sum(p: *mut c_void, sa: *mut c_void) -> (i32, i32) {
    let mut r = 0x5A5A;
    (vcall!(p, 7, (*mut c_void, *mut i32) -> i32, sa, &mut r), r)
}

fn echo_make(p: *mut c_void, n: i32) -> (i32, *mut c_void) {
    let mut r: *mut c_void = 0x1111 as *mut c_void;
    (vcall!(p, 8, (i32, *mut *mut c_void) -> i32, n, &mut r), r)
}

fn echo_swap(p: *mut c_void, a: &mut i32, s: &mut BSTR) -> i32 {
    vcall!(p, 9, (*mut i32, *mut BSTR) -> i32, a, s)
}

fn echo_fail_hr(p: *mut c_void, hr: i32) -> i32 {
    vcall!(p, 10, (i32) -> i32, hr)
}

fn echo_fail_info(p: *mut c_void, desc: BSTR) -> i32 {
    vcall!(p, 11, (BSTR) -> i32, desc)
}

fn echo_get_unknown(p: *mut c_void) -> (i32, *mut c_void) {
    let mut r: *mut c_void = 0x1111 as *mut c_void;
    (vcall!(p, 12, (*mut *mut c_void) -> i32, &mut r), r)
}

fn echo_get_new(p: *mut c_void) -> (i32, *mut c_void) {
    let mut r: *mut c_void = 0x1111 as *mut c_void;
    (vcall!(p, 13, (*mut *mut c_void) -> i32, &mut r), r)
}

fn echo_notify(p: *mut c_void, cb: *mut c_void, v: i32) -> i32 {
    vcall!(p, 14, (*mut c_void, i32) -> i32, cb, v)
}

fn echo_pid(p: *mut c_void) -> (i32, i32) {
    let mut r = 0;
    (vcall!(p, 15, (*mut i32) -> i32, &mut r), r)
}

fn echo_lpwstr_len(p: *mut c_void, s: *const u16) -> (i32, i32) {
    let mut r = 0x5A5A;
    (vcall!(p, 16, (*const u16, *mut i32) -> i32, s, &mut r), r)
}

fn echo_bytes(p: *mut c_void) -> (i32, i64) {
    let mut r = 0x5A5A_i64;
    (vcall!(p, 17, (*mut i64) -> i32, &mut r), r)
}

/// The text of the current thread's IErrorInfo (cleared by reading it): `none` or `src=.. desc=.. help=..`.
fn error_info_text() -> Buf<300> {
    let mut o = Buf::<300>::new();
    let mut ei: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer.
    let hr = unsafe { GetErrorInfo(0, &mut ei) };
    if hr != S_OK || ei.is_null() {
        let _ = write!(o, "none ({})", hex8(hr).as_str());
        return o;
    }
    let mut g = guid(0, 0, 0, [0; 8]);
    let mut src: BSTR = core::ptr::null_mut();
    let mut desc: BSTR = core::ptr::null_mut();
    let mut help: BSTR = core::ptr::null_mut();
    let mut ctx = 0u32;
    vcall!(ei, 3, (*mut GUID) -> i32, &mut g);
    vcall!(ei, 4, (*mut BSTR) -> i32, &mut src);
    vcall!(ei, 5, (*mut BSTR) -> i32, &mut desc);
    vcall!(ei, 6, (*mut BSTR) -> i32, &mut help);
    vcall!(ei, 7, (*mut u32) -> i32, &mut ctx);
    let _ = write!(
        o,
        "guid is ITlEcho {} src={} desc={} help={} ctx={}",
        u32::from(g == gid(K_ECHO, e_seed())),
        bstr_text(src).as_str(),
        bstr_text(desc).as_str(),
        bstr_text(help).as_str(),
        ctx
    );
    bstr_free(src);
    bstr_free(desc);
    bstr_free(help);
    unk_release(ei);
    o
}

/// The child process entry of a test: `tls` is the server; anything else goes to `extra`.
fn tl_child_main(a: &str, extra: fn(&str) -> u32) -> u32 {
    if a.starts_with("tls ") {
        return tl_server(a);
    }
    extra(a)
}

fn no_extra(_: &str) -> u32 {
    0
}

// ---- the skeleton most tests share

/// Set up the run (library built and registered, class registered), initialise COM (`mta` or STA) and create the
/// server's object as ITlEcho. Prints what failed and returns None.
fn boot(t: &mut Test, tag: &'static str, mta: bool, shape: Shape) -> Option<(TlRun, *mut c_void)> {
    let seed = run_seed();
    let run = match tl_setup(seed, tag, shape) {
        Ok(r) => r,
        Err((s, hr)) => {
            say!(t, "setup failed at step {s}: {}", hrs(hr).as_str());
            tl_cleanup(seed);
            return None;
        }
    };
    let hr = co_init(mta);
    if hr != S_OK {
        say!(t, "CoInitializeEx = {}", hrs(hr).as_str());
    }
    let (hr, echo) = tl_create(&run, &gid(K_ECHO, seed));
    if hr != S_OK {
        say!(t, "CoCreateInstance(ITlEcho) = {}", hrs(hr).as_str());
        run.teardown();
        return None;
    }
    Some((run, echo))
}

/// Release the object, stop the server, print its log and remove everything the run made.
fn finish_run(t: &mut Test, run: TlRun, objs: &[*mut c_void], log: bool) {
    for &o in objs {
        unk_release(o);
    }
    let left = stop_server(run.tag, 4000);
    if !left {
        say!(t, "server did not leave");
    }
    if log {
        print_tl_log(t, run.tag);
    }
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
    run.teardown();
}

/// Wait up to `ms` until the log of `tag` has `n` lines containing `needle`.
fn wait_log_count(tag: &str, needle: &str, n: usize, ms: u32) -> bool {
    let p = srv_log_path(tag);
    let mut waited = 0;
    loop {
        if count_lines(read_text(&p).s(), needle) >= n {
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

/// Like `finish_run` for a test whose log holds the lines of `servers` server processes.
fn finish_run_n(t: &mut Test, run: TlRun, objs: &[*mut c_void], servers: usize) {
    for &o in objs {
        unk_release(o);
    }
    make_file(&quit_path(run.tag));
    if !wait_log_count(run.tag, "exit", servers, 5000) {
        say!(t, "a server did not leave");
    }
    print_tl_log(t, run.tag);
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
    run.teardown();
}

// ---- streams and marshalling in one process

const IID_IPSFactoryBuffer: GUID = guid(0xD5F569D0, 0x593B, 0x101A, [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A]);
const IID_IRpcProxyBuffer: GUID = guid(0xD5F56A34, 0x593B, 0x101A, [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A]);
const IID_IRpcStubBuffer: GUID = guid(0xD5F56AFC, 0x593B, 0x101A, [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A]);
const CLSID_PSOAInterface: GUID = guid(0x20424, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const CLSID_PSDispatch: GUID = guid(0x20420, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const CLSID_StdMarshal: GUID = guid(0x17, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IStream: GUID = guid(0x0C, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IErrorInfo: GUID = guid(0x1CF2B120, 0x547D, 0x101B, [0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19]);
const REGDB_E_IIDNOTREG: i32 = 0x8004_0155_u32 as i32;
const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;

fn stream_new() -> *mut c_void {
    let mut s: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer; a stream on a global memory block of its own.
    unsafe { CreateStreamOnHGlobal(core::ptr::null_mut(), 1, &mut s) };
    s
}

fn stream_rewind(s: *mut c_void) {
    let mut pos = 0u64;
    vcall!(s, 5, (i64, u32, *mut u64) -> i32, 0, 0, &mut pos);
}

fn stream_pos(s: *mut c_void) -> u64 {
    let mut pos = 0u64;
    vcall!(s, 5, (i64, u32, *mut u64) -> i32, 0, 1, &mut pos);
    pos
}

fn stream_len(s: *mut c_void) -> u64 {
    let mut pos = 0u64;
    vcall!(s, 5, (i64, u32, *mut u64) -> i32, 0, 2, &mut pos);
    pos
}

/// The bytes written to a memory stream (the first up to 600).
fn stream_bytes(s: *mut c_void) -> ([u8; 600], usize) {
    let mut out = [0u8; 600];
    let mut h: *mut c_void = core::ptr::null_mut();
    // SAFETY: the stream was made by CreateStreamOnHGlobal.
    unsafe {
        GetHGlobalFromStream(s, &mut h);
        let n = (stream_len(s) as usize).min(600);
        let p = GlobalLock(h);
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(p, out.as_mut_ptr(), n);
        }
        GlobalUnlock(h);
        (out, n)
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A stream holding the bytes `data`, positioned at the start.
fn stream_from(data: &[u8]) -> *mut c_void {
    let s = stream_new();
    let mut put = 0u32;
    vcall!(s, 4, (*const u8, u32, *mut u32) -> i32, data.as_ptr(), data.len() as u32, &mut put);
    stream_rewind(s);
    s
}

fn marshal_to(stm: *mut c_void, iid: &GUID, unk: *mut c_void, ctx: u32, flags: u32) -> i32 {
    // SAFETY: a stream and an object the test holds.
    unsafe { CoMarshalInterface(stm, iid, unk, ctx, core::ptr::null_mut(), flags) }
}

fn unmarshal_from(stm: *mut c_void, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = 0x1111 as *mut c_void;
    // SAFETY: a stream the test holds; live out pointer.
    let hr = unsafe { CoUnmarshalInterface(stm, iid, &mut out) };
    (hr, out)
}

/// Set up the run without starting a server (library and class registered) and initialise COM.
fn local_boot(t: &mut Test, tag: &'static str, mta: bool) -> Option<TlRun> {
    let seed = run_seed();
    match tl_setup(seed, tag, SHAPE_INTERFACE) {
        Ok(r) => {
            let hr = co_init(mta);
            if hr != S_OK {
                say!(t, "CoInitializeEx = {}", hrs(hr).as_str());
            }
            Some(r)
        }
        Err((s, hr)) => {
            say!(t, "setup failed at step {s}: {}", hrs(hr).as_str());
            tl_cleanup(seed);
            None
        }
    }
}

fn local_end(run: TlRun) {
    // SAFETY: plain call.
    unsafe { CoUninitialize() };
    run.teardown();
}

/// A new object of the test library living in this process (one reference).
fn local_echo() -> *mut c_void {
    new_eobj() as *mut c_void
}

/// Print the server's log. The lines of the calls the client makes one after the other keep their order; the lines
/// "object released live=N" (written by the server when COM drops the last reference of an object, which can happen
/// on another thread, before or after the client's next call) are not printed where they are but as one sorted summary.
fn print_tl_log(t: &mut Test, tag: &str) {
    let txt = read_text(&srv_log_path(tag));
    let mut live = [0i32; 64];
    let mut n = 0usize;
    for l in txt.s().split(char::from(10)) {
        if l.is_empty() {
            continue;
        }
        if let Some(v) = l.strip_prefix("object released live=") {
            if n < 64 {
                live[n] = v.trim().parse().unwrap_or(-1);
                n += 1;
            }
        } else {
            say!(t, "srv: {l}");
        }
    }
    if n > 0 {
        live[..n].sort_unstable_by(|a, b| b.cmp(a));
        let mut b = Buf::<300>::new();
        for (i, v) in live[..n].iter().enumerate() {
            let _ = write!(b, "{}{v}", if i == 0 { "" } else { " " });
        }
        say!(t, "srv: objects released, {n} times, live counts after each: {}", b.as_str());
    }
}

poly_wtest::main!(test);

fn describe(t: &mut Test, name: &str, stm: *mut c_void, seed: u32) {
    let (b, n) = stream_bytes(stm);
    let len = stream_len(stm);
    let pos = stream_pos(stm);
    if n >= 24 {
        let iid = GUID {
            d1: le32(&b, 8),
            d2: u16::from_le_bytes([b[12], b[13]]),
            d3: u16::from_le_bytes([b[14], b[15]]),
            d4: [b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]],
        };
        let which = if iid == gid(K_ECHO, seed) { "ITlEcho".into_buf() } else { iid_name(&iid) };
        say!(
            t,
            "  {name}: position is the end {}, signature {:#010x}, flags {}, interface {}, length {}",
            u32::from(len == pos),
            le32(&b, 0),
            le32(&b, 4),
            which.as_str(),
            if le32(&b, 4) == 1 && len < 100 { len as i64 } else { -1 }
        );
    } else {
        say!(t, "  {name}: {len} bytes (position {pos})");
    }
}

fn refs_of(obj: *mut c_void) -> u32 {
    let n = unk_addref(obj);
    unk_release(obj);
    n - 1
}

trait IntoBuf {
    fn into_buf(self) -> Buf<48>;
}

impl IntoBuf for &str {
    fn into_buf(self) -> Buf<48> {
        let mut b = Buf::<48>::new();
        let _ = write!(b, "{self}");
        b
    }
}

fn test(t: &mut Test) {
    let Some(run) = local_boot(t, "ms", true) else { return };
    let seed = run.seed;
    let obj = local_echo();
    let echo_iid = gid(K_ECHO, seed);
    let r0 = refs_of(obj);
    say!(t, "the object starts with {r0} reference");
    for (name, ctx, flags) in [
        ("LOCAL, NORMAL", MSHCTX_LOCAL, MSHLFLAGS_NORMAL),
        ("LOCAL, TABLESTRONG", MSHCTX_LOCAL, MSHLFLAGS_TABLESTRONG),
        ("LOCAL, TABLEWEAK", MSHCTX_LOCAL, MSHLFLAGS_TABLEWEAK),
        ("LOCAL, NORMAL|NOPING", MSHCTX_LOCAL, MSHLFLAGS_NORMAL | MSHLFLAGS_NOPING),
        ("NOSHAREDMEM, NORMAL", MSHCTX_NOSHAREDMEM, MSHLFLAGS_NORMAL),
        ("INPROC, NORMAL", MSHCTX_INPROC, MSHLFLAGS_NORMAL),
    ] {
        let stm = stream_new();
        let before = refs_of(obj);
        let hr = marshal_to(stm, &echo_iid, obj, ctx, flags);
        let after = refs_of(obj);
        say!(
            t,
            "{name}: CoMarshalInterface = {}, the marshalling holds references on the object {}",
            hrs(hr).as_str(),
            u32::from(after > before)
        );
        describe(t, "stream", stm, seed);
        if hr == S_OK {
            stream_rewind(stm);
            // SAFETY: releases the marshalled data of the stream.
            let hr = unsafe { CoReleaseMarshalData(stm) };
            say!(
                t,
                "  CoReleaseMarshalData = {}, the object's references are back to {}",
                hrs(hr).as_str(),
                refs_of(obj)
            );
        }
        unk_release(stm);
    }
    // the same object as IUnknown
    let stm = stream_new();
    let hr = marshal_to(stm, &IID_IUnknown, obj, MSHCTX_LOCAL, MSHLFLAGS_NORMAL);
    say!(t, "as IUnknown: {}", hrs(hr).as_str());
    describe(t, "stream", stm, seed);
    stream_rewind(stm);
    // SAFETY: as above.
    unsafe { CoReleaseMarshalData(stm) };
    unk_release(stm);
    // as ITlDual (QueryInterface by the marshaller)
    let stm = stream_new();
    let hr = marshal_to(stm, &gid(K_DUAL, seed), obj, MSHCTX_LOCAL, MSHLFLAGS_NORMAL);
    say!(t, "as ITlDual: {}", hrs(hr).as_str());
    stream_rewind(stm);
    // SAFETY: as above.
    unsafe { CoReleaseMarshalData(stm) };
    unk_release(stm);
    // as IDispatch (the dual interface answers it)
    let stm = stream_new();
    let hr = marshal_to(stm, &IID_IDispatch, obj, MSHCTX_LOCAL, MSHLFLAGS_NORMAL);
    say!(t, "as IDispatch: {}", hrs(hr).as_str());
    describe(t, "stream", stm, seed);
    stream_rewind(stm);
    // SAFETY: as above.
    unsafe { CoReleaseMarshalData(stm) };
    unk_release(stm);
    // writing into a stream that already holds data: the data is appended at the position
    let stm = stream_new();
    let mut put = 0u32;
    vcall!(stm, 4, (*const u8, u32, *mut u32) -> i32, b"0123456789".as_ptr(), 10, &mut put);
    let hr = marshal_to(stm, &echo_iid, obj, MSHCTX_LOCAL, MSHLFLAGS_NORMAL);
    say!(
        t,
        "after 10 bytes in the stream: {}, length {} position {}",
        hrs(hr).as_str(),
        stream_len(stm) > 10,
        u32::from(stream_pos(stm) == stream_len(stm))
    );
    stream_rewind(stm);
    // SAFETY: as above (the data starts at 0, not 10: this must fail or release garbage; only the result is shown).
    let mut skip = 0u64;
    vcall!(stm, 5, (i64, u32, *mut u64) -> i32, 10, 0, &mut skip);
    // SAFETY: as above.
    let hr = unsafe { CoReleaseMarshalData(stm) };
    say!(t, "  release from position 10 = {}", hrs(hr).as_str());
    unk_release(stm);
    unk_release(obj);
    local_end(run);
}
