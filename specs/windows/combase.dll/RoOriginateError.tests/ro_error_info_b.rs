//! RoOriginateError and its relatives: the restricted error info a failing WinRT call leaves on the thread
//! (GetRestrictedErrorInfo takes it, SetRestrictedErrorInfo puts one back or clears it); its details; the reporting flags.
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

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
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

// ----------------------------------------------------- combase (Windows Runtime), looked up at run time

type HSTRING = *mut c_void;

/// winstring.h `HSTRING_HEADER` (24 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct HSTRING_HEADER {
    reserved: [usize; 3],
}

const NO_HEADER: HSTRING_HEADER = HSTRING_HEADER { reserved: [0; 3] };

/// guiddef.h `GUID`.
#[repr(C)]
#[derive(Clone, Copy)]
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
/// inspectable.h `IID_IInspectable`.
const IID_IInspectable: GUID = guid(0xAF86E2E0, 0xB12D, 0x4C6A, [0x9C, 0x5A, 0xD7, 0xAA, 0x65, 0x10, 0x1E, 0x90]);
/// activation.h `IID_IActivationFactory`.
const IID_IActivationFactory: GUID = guid(0x00000035, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// restrictederrorinfo.h `IID_IRestrictedErrorInfo`.
const IID_IRestrictedErrorInfo: GUID =
    guid(0x82BA7092, 0x4C88, 0x427D, [0xA7, 0xBC, 0x16, 0xDD, 0x93, 0xFE, 0xB6, 0x7E]);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// The address of `name` (NUL-terminated ASCII) in the DLL `dll`, or NULL.
fn proc_in(dll: &str, name: &[u8]) -> *mut c_void {
    let w = wide(dll);
    // SAFETY: a NUL-terminated name and function name.
    unsafe {
        let m = LoadLibraryW(w.as_ptr());
        if m.is_null() {
            return core::ptr::null_mut();
        }
        GetProcAddress(m, name.as_ptr())
    }
}

/// An export of combase.dll by name (`b"RoInitialize\0"`).
fn combase(name: &[u8]) -> *mut c_void {
    proc_in("combase.dll", name)
}

macro_rules! ro_fn {
    ($name:ident, $sym:literal, fn($($a:ty),*) -> $r:ty) => {
        fn $name() -> Option<unsafe extern "system" fn($($a),*) -> $r> {
            let p = combase(concat!($sym, "\0").as_bytes());
            if p.is_null() {
                None
            } else {
                // SAFETY: the export has this prototype (Microsoft Learn).
                Some(unsafe { core::mem::transmute::<*mut c_void, unsafe extern "system" fn($($a),*) -> $r>(p) })
            }
        }
    };
}

ro_fn!(f_ro_initialize, "RoInitialize", fn(i32) -> i32);
ro_fn!(f_ro_uninitialize, "RoUninitialize", fn() -> ());
ro_fn!(f_ro_get_activation_factory, "RoGetActivationFactory", fn(HSTRING, *const GUID, *mut *mut c_void) -> i32);
ro_fn!(f_ro_activate_instance, "RoActivateInstance", fn(HSTRING, *mut *mut c_void) -> i32);
ro_fn!(f_wcs, "WindowsCreateString", fn(*const u16, u32, *mut HSTRING) -> i32);
ro_fn!(f_wcsr, "WindowsCreateStringReference", fn(*const u16, u32, *mut HSTRING_HEADER, *mut HSTRING) -> i32);
ro_fn!(f_wds, "WindowsDeleteString", fn(HSTRING) -> i32);
ro_fn!(f_wdup, "WindowsDuplicateString", fn(HSTRING, *mut HSTRING) -> i32);
ro_fn!(f_wraw, "WindowsGetStringRawBuffer", fn(HSTRING, *mut u32) -> *const u16);
ro_fn!(f_wlen, "WindowsGetStringLen", fn(HSTRING) -> u32);
ro_fn!(f_wempty, "WindowsIsStringEmpty", fn(HSTRING) -> i32);
ro_fn!(f_wembnull, "WindowsStringHasEmbeddedNull", fn(HSTRING, *mut i32) -> i32);
ro_fn!(f_wcmp, "WindowsCompareStringOrdinal", fn(HSTRING, HSTRING, *mut i32) -> i32);
ro_fn!(f_wcat, "WindowsConcatString", fn(HSTRING, HSTRING, *mut HSTRING) -> i32);
ro_fn!(f_wsub, "WindowsSubstring", fn(HSTRING, u32, *mut HSTRING) -> i32);
ro_fn!(f_wsubn, "WindowsSubstringWithSpecifiedLength", fn(HSTRING, u32, u32, *mut HSTRING) -> i32);
ro_fn!(f_wprealloc, "WindowsPreallocateStringBuffer", fn(u32, *mut *mut u16, *mut HSTRING) -> i32);
ro_fn!(f_wpromote, "WindowsPromoteStringBuffer", fn(HSTRING, *mut HSTRING) -> i32);
ro_fn!(f_wdelbuf, "WindowsDeleteStringBuffer", fn(HSTRING) -> i32);
ro_fn!(f_wreplace, "WindowsReplaceString", fn(HSTRING, HSTRING, HSTRING, *mut HSTRING) -> i32);
ro_fn!(f_wtrims, "WindowsTrimStringStart", fn(HSTRING, HSTRING, *mut HSTRING) -> i32);
ro_fn!(f_wtrime, "WindowsTrimStringEnd", fn(HSTRING, HSTRING, *mut HSTRING) -> i32);
ro_fn!(f_ro_orig_err, "RoOriginateError", fn(i32, HSTRING) -> i32);
ro_fn!(f_ro_orig_errw, "RoOriginateErrorW", fn(i32, u32, *const u16) -> i32);
ro_fn!(f_ro_orig_lang, "RoOriginateLanguageException", fn(i32, HSTRING, *mut c_void) -> i32);
ro_fn!(f_get_rei, "GetRestrictedErrorInfo", fn(*mut *mut c_void) -> i32);
ro_fn!(f_set_rei, "SetRestrictedErrorInfo", fn(*mut c_void) -> i32);
ro_fn!(f_ro_clear_error, "RoClearError", fn() -> ());
ro_fn!(f_ro_get_err_flags, "RoGetErrorReportingFlags", fn(*mut u32) -> i32);
ro_fn!(f_ro_set_err_flags, "RoSetErrorReportingFlags", fn(u32) -> i32);

/// A vtable-first COM object: `(*obj).vtbl[index]` as a function pointer.
fn vslot(obj: *mut c_void, index: usize) -> *mut c_void {
    // SAFETY: `obj` is a live COM interface pointer; its first word points at its table of methods.
    unsafe {
        let vtbl = *(obj as *const *const *mut c_void);
        *vtbl.add(index)
    }
}

/// IUnknown::Release.
fn com_release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: slot 2 of every COM interface is Release.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vslot(obj, 2));
        f(obj)
    }
}

/// IUnknown::QueryInterface; the HRESULT, and the interface in `out`.
fn com_qi(obj: *mut c_void, iid: &GUID, out: &mut *mut c_void) -> i32 {
    // SAFETY: slot 0 of every COM interface is QueryInterface.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32 =
            core::mem::transmute(vslot(obj, 0));
        f(obj, iid, out)
    }
}

/// An HSTRING for the ASCII text `s` (a copy; NULL for the empty string).
fn hs(s: &str) -> HSTRING {
    let w = wide(s);
    let mut h: HSTRING = core::ptr::null_mut();
    if let Some(f) = f_wcs() {
        // SAFETY: a counted UTF-16 string and a live out slot.
        unsafe { f(w.as_ptr(), s.len() as u32, &mut h) };
    }
    h
}

/// The text of an HSTRING (printable ASCII as it is, anything else as `<xxxx>`).
fn hs_text(h: HSTRING) -> Buf<600> {
    let mut n = 0u32;
    let p = match f_wraw() {
        // SAFETY: an HSTRING from this process.
        Some(f) => unsafe { f(h, &mut n) },
        None => core::ptr::null(),
    };
    if p.is_null() {
        return Buf::<600>::new();
    }
    // SAFETY: the buffer holds `n` units.
    units_text(unsafe { core::slice::from_raw_parts(p, n as usize) })
}

fn hs_free(h: HSTRING) {
    if let Some(f) = f_wds() {
        // SAFETY: an HSTRING this test made.
        unsafe { f(h) };
    }
}

/// An HRESULT as `0x........`.
fn hr(v: i32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    let _ = write!(b, "{:#010x}", v as u32);
    b
}

poly_wtest::main!(test);

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SysStringLen(b: *const u16) -> u32;
    fn SysFreeString(b: *const u16);
}

fn bstr_text(b: *const u16) -> Buf<300> {
    let mut out = Buf::<300>::new();
    if b.is_null() {
        let _ = write!(out, "<null>");
        return out;
    }
    // SAFETY: a BSTR from the system.
    let n = unsafe { SysStringLen(b) } as usize;
    let s = units_text(unsafe { core::slice::from_raw_parts(b, n) });
    let _ = write!(out, "{}", s.as_str());
    out
}

/// IRestrictedErrorInfo::GetErrorDetails (vtable slot 3).
fn details(obj: *mut c_void) -> (i32, i32, Buf<300>, Buf<300>, Buf<300>) {
    let mut d: *const u16 = core::ptr::null();
    let mut e = 0i32;
    let mut r: *const u16 = core::ptr::null();
    let mut c: *const u16 = core::ptr::null();
    // SAFETY: slot 3 of IRestrictedErrorInfo is GetErrorDetails(BSTR*, HRESULT*, BSTR*, BSTR*).
    let h = unsafe {
        let f: unsafe extern "system" fn(
            *mut c_void,
            *mut *const u16,
            *mut i32,
            *mut *const u16,
            *mut *const u16,
        ) -> i32 = core::mem::transmute(vslot(obj, 3));
        f(obj, &mut d, &mut e, &mut r, &mut c)
    };
    let out = (h, e, bstr_text(d), bstr_text(r), bstr_text(c));
    // SAFETY: BSTRs the call returned.
    unsafe {
        SysFreeString(d);
        SysFreeString(r);
        SysFreeString(c);
    }
    out
}

fn show(t: &mut Test, what: &str, p: *mut c_void) {
    if p.is_null() {
        say!(t, "{what}: no error info");
        return;
    }
    let (h, e, d, rd, cap) = details(p);
    say!(
        t,
        "{what}: GetErrorDetails = {} error {} description [{}] restricted [{}] sid [{}]",
        hr(h),
        hr(e),
        d,
        rd,
        cap
    );
}

fn get_info() -> *mut c_void {
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live out slot.
    unsafe { f_get_rei().unwrap()(&mut p) };
    p
}

fn test(t: &mut Test) {
    SAY_PART.store(2, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: plain calls with live buffers.
    unsafe {
        say!(t, "RoInitialize(1) = {}", hr(f_ro_initialize().unwrap()(1)));
        let mut p: *mut c_void = 1usize as *mut c_void;
        say!(
            t,
            "GetRestrictedErrorInfo with nothing set = {} out_null {}",
            hr(f_get_rei().unwrap()(&mut p)),
            p.is_null()
        );
        let msg = hs("poly message");
        let r = f_ro_orig_err().unwrap()(0x80004005u32 as i32, msg);
        say!(t, "RoOriginateError(E_FAIL, msg) = {}", r);
        let p = get_info();
        say!(t, "GetRestrictedErrorInfo: out_null {}", p.is_null());
        if !p.is_null() {
            show(t, "  details", p);
            let mut q: *mut c_void = core::ptr::null_mut();
            say!(t, "  QI IRestrictedErrorInfo = {}", hr(com_qi(p, &IID_IRestrictedErrorInfo, &mut q)));
            com_release(q);
            let mut q: *mut c_void = core::ptr::null_mut();
            say!(t, "  QI IUnknown = {}", hr(com_qi(p, &IID_IUnknown, &mut q)));
            com_release(q);
            let mut q: *mut c_void = core::ptr::null_mut();
            say!(t, "  QI IInspectable = {}", hr(com_qi(p, &IID_IInspectable, &mut q)));
            com_release(q);
            let mut p2: *mut c_void = 1usize as *mut c_void;
            say!(
                t,
                "  second GetRestrictedErrorInfo = {} out_null {}",
                hr(f_get_rei().unwrap()(&mut p2)),
                p2.is_null()
            );
            say!(t, "  SetRestrictedErrorInfo(p) = {}", hr(f_set_rei().unwrap()(p)));
            let p3 = get_info();
            say!(t, "  get again: same_object {}", p3 == p);
            com_release(p3);
            com_release(p);
        }
        f_ro_orig_err().unwrap()(0x80070057u32 as i32, msg);
        say!(t, "SetRestrictedErrorInfo(NULL) = {}", hr(f_set_rei().unwrap()(core::ptr::null_mut())));
        let mut p: *mut c_void = 1usize as *mut c_void;
        say!(t, "  then GetRestrictedErrorInfo = {} out_null {}", hr(f_get_rei().unwrap()(&mut p)), p.is_null());
        say!(t, "RoOriginateError(S_OK, msg) = {}", f_ro_orig_err().unwrap()(0, msg));
        show(t, "  after S_OK", get_info());
        say!(
            t,
            "RoOriginateError(E_FAIL, NULL message) = {}",
            f_ro_orig_err().unwrap()(0x80004005u32 as i32, core::ptr::null_mut())
        );
        let p = get_info();
        show(t, "  details", p);
        com_release(p);
        let w = wide("wide message");
        say!(
            t,
            "RoOriginateErrorW(E_INVALIDARG, 12, text) = {}",
            f_ro_orig_errw().unwrap()(0x80070057u32 as i32, 12, w.as_ptr())
        );
        let p = get_info();
        show(t, "  details", p);
        com_release(p);
        say!(
            t,
            "RoOriginateErrorW(E_INVALIDARG, 4, text) (shorter than the text) = {}",
            f_ro_orig_errw().unwrap()(0x80070057u32 as i32, 4, w.as_ptr())
        );
        let p = get_info();
        show(t, "  details", p);
        com_release(p);
        say!(
            t,
            "RoOriginateLanguageException(E_ABORT, msg, NULL) = {}",
            f_ro_orig_lang().unwrap()(0x80004004u32 as i32, msg, core::ptr::null_mut())
        );
        let p = get_info();
        show(t, "  details", p);
        com_release(p);
        // a later originate replaces the earlier info
        f_ro_orig_err().unwrap()(0x80004005u32 as i32, msg);
        let m2 = hs("second");
        f_ro_orig_err().unwrap()(0x80070057u32 as i32, m2);
        let p = get_info();
        show(t, "two originates, then one get", p);
        com_release(p);
        show(t, "  and the next get", get_info());
        hs_free(m2);
        // flags
        let mut f = 99u32;
        say!(t, "RoGetErrorReportingFlags = {} {}", hr(f_ro_get_err_flags().unwrap()(&mut f)), f);
        say!(t, "RoSetErrorReportingFlags(8) = {}", hr(f_ro_set_err_flags().unwrap()(8)));
        let mut f = 99u32;
        say!(t, "  get = {} {}", hr(f_ro_get_err_flags().unwrap()(&mut f)), f);
        say!(t, "RoSetErrorReportingFlags(0x30) = {}", hr(f_ro_set_err_flags().unwrap()(0x30)));
        say!(t, "RoSetErrorReportingFlags(0) = {}", hr(f_ro_set_err_flags().unwrap()(0)));
        // RoClearError
        f_ro_orig_err().unwrap()(0x80004005u32 as i32, msg);
        f_ro_clear_error().unwrap()();
        show(t, "RoClearError then get", get_info());
        hs_free(msg);
        f_ro_uninitialize().unwrap()();
    }
}
