//! CreateFontIndirectW with a face no machine has: the font is made; GetObjectW gives back the
//! LOGFONT as it was asked for (the absent face included); selected into a DC the font is another,
//! installed face (GetTextFaceW): the one the same LOGFONT with an empty face gets; its height
//! minus internal leading is the requested character height. Classes only.
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
        self.line(s);
    }
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

// Font declarations: user32 (SystemParametersInfoW, GetDC), gdi32 (fonts, DCs) and dwrite
// (factory, collection, fallback, number substitution). Prototypes from Microsoft Learn;
// layouts and constants from winuser.h, wingdi.h, dwrite.h, dwrite_1.h, dwrite_2.h and
// winerror.h. The text-analysis source and the helpers are this test's own.

type HRESULT = i32;
const E_NOINTERFACE: i32 = 0x80004002u32 as i32;

/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}
const IID_IUnknown: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// wingdi.h LOGFONTW
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct LOGFONTW {
    lfHeight: i32,
    lfWidth: i32,
    lfEscapement: i32,
    lfOrientation: i32,
    lfWeight: i32,
    lfItalic: u8,
    lfUnderline: u8,
    lfStrikeOut: u8,
    lfCharSet: u8,
    lfOutPrecision: u8,
    lfClipPrecision: u8,
    lfQuality: u8,
    lfPitchAndFamily: u8,
    lfFaceName: [u16; 32],
}

impl LOGFONTW {
    fn zero() -> LOGFONTW {
        // SAFETY: plain integers; all-zero is a valid value.
        unsafe { core::mem::zeroed() }
    }
    fn with_face(height: i32, face: &str) -> LOGFONTW {
        let mut lf = LOGFONTW::zero();
        lf.lfHeight = height;
        lf.lfCharSet = DEFAULT_CHARSET;
        for (i, c) in face.chars().enumerate().take(31) {
            lf.lfFaceName[i] = c as u32 as u16;
        }
        lf
    }
    fn face(&self) -> &[u16] {
        let n = self.lfFaceName.iter().position(|&c| c == 0).unwrap_or(32);
        &self.lfFaceName[..n]
    }
}

/// winuser.h NONCLIENTMETRICSW (with iPaddedBorderWidth: 504 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
struct NONCLIENTMETRICSW {
    cbSize: u32,
    iBorderWidth: i32,
    iScrollWidth: i32,
    iScrollHeight: i32,
    iCaptionWidth: i32,
    iCaptionHeight: i32,
    lfCaptionFont: LOGFONTW,
    iSmCaptionWidth: i32,
    iSmCaptionHeight: i32,
    lfSmCaptionFont: LOGFONTW,
    iMenuWidth: i32,
    iMenuHeight: i32,
    lfMenuFont: LOGFONTW,
    lfStatusFont: LOGFONTW,
    lfMessageFont: LOGFONTW,
    iPaddedBorderWidth: i32,
}

/// wingdi.h TEXTMETRICW
#[repr(C)]
#[derive(Clone, Copy)]
struct TEXTMETRICW {
    tmHeight: i32,
    tmAscent: i32,
    tmDescent: i32,
    tmInternalLeading: i32,
    tmExternalLeading: i32,
    tmAveCharWidth: i32,
    tmMaxCharWidth: i32,
    tmWeight: i32,
    tmOverhang: i32,
    tmDigitizedAspectX: i32,
    tmDigitizedAspectY: i32,
    tmFirstChar: u16,
    tmLastChar: u16,
    tmDefaultChar: u16,
    tmBreakChar: u16,
    tmItalic: u8,
    tmUnderlined: u8,
    tmStruckOut: u8,
    tmPitchAndFamily: u8,
    tmCharSet: u8,
}

const SPI_GETNONCLIENTMETRICS: u32 = 0x0029;
const DEFAULT_CHARSET: u8 = 1;
const ANSI_CHARSET: u8 = 0;
const SYMBOL_CHARSET: u8 = 2;
const LOGPIXELSY: i32 = 90;
const OBJ_FONT: u32 = 6;
const SYSTEM_FONT: i32 = 13;
const DEFAULT_GUI_FONT: i32 = 17;
const TMPF_TRUETYPE: u8 = 0x04;
const TMPF_VECTOR: u8 = 0x02;
const FIXED_PITCH: u8 = 1;
const VARIABLE_PITCH: u8 = 2;
const FF_ROMAN: u8 = 0x10;
const FF_SWISS: u8 = 0x20;
const FF_MODERN: u8 = 0x30;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
    fn GetDC(hwnd: HANDLE) -> HANDLE;
    fn ReleaseDC(hwnd: HANDLE, hdc: HANDLE) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontIndirectW(lf: *const LOGFONTW) -> HANDLE;
    fn GetObjectW(h: HANDLE, n: i32, out: *mut c_void) -> i32;
    fn GetObjectType(h: HANDLE) -> u32;
    fn SelectObject(hdc: HANDLE, obj: HANDLE) -> HANDLE;
    fn DeleteObject(obj: HANDLE) -> BOOL;
    fn GetTextFaceW(hdc: HANDLE, n: i32, out: *mut u16) -> i32;
    fn GetTextMetricsW(hdc: HANDLE, tm: *mut TEXTMETRICW) -> BOOL;
    fn GetStockObject(i: i32) -> HANDLE;
    fn GetDeviceCaps(hdc: HANDLE, i: i32) -> i32;
    fn CreateCompatibleDC(hdc: HANDLE) -> HANDLE;
    fn DeleteDC(hdc: HANDLE) -> BOOL;
    fn EnumFontFamiliesExW(
        hdc: HANDLE,
        lf: *const LOGFONTW,
        proc_: unsafe extern "system" fn(*const LOGFONTW, *const c_void, u32, isize) -> i32,
        lparam: isize,
        flags: u32,
    ) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetUserDefaultLocaleName(name: *mut u16, n: i32) -> i32;
}

/// The message font metrics (cbSize 504), or None.
fn ncm() -> Option<NONCLIENTMETRICSW> {
    // SAFETY: all-zero is a valid NONCLIENTMETRICSW; the pointer is valid for 504 bytes.
    unsafe {
        let mut m: NONCLIENTMETRICSW = core::mem::zeroed();
        m.cbSize = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        if SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, m.cbSize, &mut m as *mut _ as *mut c_void, 0) == 0 {
            None
        } else {
            Some(m)
        }
    }
}

/// The five fonts of NONCLIENTMETRICSW with their names.
fn ncm_fonts(m: &NONCLIENTMETRICSW) -> [(&'static str, LOGFONTW); 5] {
    [
        ("caption", m.lfCaptionFont),
        ("small caption", m.lfSmCaptionFont),
        ("menu", m.lfMenuFont),
        ("status", m.lfStatusFont),
        ("message", m.lfMessageFont),
    ]
}

/// Counts the callbacks of EnumFontFamiliesExW and whether a face equal (ignoring case) to the
/// one asked for was among them.
struct EnumCount {
    calls: u32,
    want: [u16; 32],
    same: u32,
}

unsafe extern "system" fn enum_count(lf: *const LOGFONTW, _tm: *const c_void, _ty: u32, lp: isize) -> i32 {
    // SAFETY: `lp` is the EnumCount the caller passed; `lf` is valid for the callback.
    unsafe {
        let c = &mut *(lp as *mut EnumCount);
        c.calls += 1;
        let n = c.want.iter().position(|&x| x == 0).unwrap_or(32);
        if eq_nocase((*lf).face(), &c.want[..n]) {
            c.same += 1;
        }
    }
    1
}

/// EnumFontFamiliesExW for one face (DEFAULT_CHARSET) on `hdc`: (return value, callbacks, callbacks
/// whose face is the one asked for).
fn enum_face(hdc: HANDLE, face: &[u16]) -> (i32, u32, u32) {
    let mut lf = LOGFONTW::zero();
    lf.lfCharSet = DEFAULT_CHARSET;
    let n = face.len().min(31);
    lf.lfFaceName[..n].copy_from_slice(&face[..n]);
    let mut c = EnumCount { calls: 0, want: lf.lfFaceName, same: 0 };
    // SAFETY: valid pointers; the callback only touches `c`.
    let r = unsafe { EnumFontFamiliesExW(hdc, &lf, enum_count, &mut c as *mut EnumCount as isize, 0) };
    (r, c.calls, c.same)
}

/// The face GetTextFaceW reports for the font selected into `hdc`.
fn text_face(hdc: HANDLE) -> ([u16; 64], usize, i32) {
    let mut b = [0u16; 64];
    // SAFETY: the buffer holds 64 units.
    let r = unsafe { GetTextFaceW(hdc, 64, b.as_mut_ptr()) };
    let n = b.iter().position(|&c| c == 0).unwrap_or(64);
    (b, n, r)
}

// ---- DirectWrite ----

/// A call through a COM interface pointer: `com!(p, Method(args))`.
macro_rules! com {
    ($p:expr, $m:ident($($a:expr),* $(,)?)) => {{
        let p = $p;
        ((*(*p).vtbl).$m)(p as *mut c_void $(, $a)*)
    }};
}

/// `&mut x` as the `*mut c_void` of an out parameter.
macro_rules! outp {
    ($e:expr) => {
        &mut $e as *mut _ as *mut c_void
    };
}

#[repr(C)]
struct IUnknownVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
}
#[repr(C)]
struct IUnknown {
    vtbl: *const IUnknownVtbl,
}

/// Release on any COM object; the new reference count.
fn release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: `obj` is a live COM object this test holds a reference to.
    unsafe { ((*(*(obj as *mut IUnknown)).vtbl).Release)(obj) }
}

/// QueryInterface on any COM object.
fn query(obj: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut out: *mut c_void = 0x10usize as *mut c_void;
    // SAFETY: `obj` is a live COM object; the pointers are valid.
    let hr = unsafe {
        ((*(*(obj as *mut IUnknown)).vtbl).QueryInterface)(obj, iid as *const GUID as *mut c_void, outp!(out))
    };
    (hr, out)
}

#[repr(C)]
struct IDWriteLocalizedStringsVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetCount: unsafe extern "system" fn(*mut c_void) -> u32,
    FindLocaleName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    GetLocaleNameLength: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetLocaleName: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
    GetStringLength: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetString: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32,
}
#[repr(C)]
struct IDWriteLocalizedStrings {
    vtbl: *const IDWriteLocalizedStringsVtbl,
}

#[repr(C)]
struct IDWriteFontCollectionVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamilyCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamily: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    FindFamilyName: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    GetFontFromFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
}
#[repr(C)]
struct IDWriteFontCollection {
    vtbl: *const IDWriteFontCollectionVtbl,
}

#[repr(C)]
struct IDWriteFontFamilyVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFontCount: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFont: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    GetFamilyNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetFirstMatchingFont: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
    GetMatchingFonts: unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32,
}
#[repr(C)]
struct IDWriteFontFamily {
    vtbl: *const IDWriteFontFamilyVtbl,
}

#[repr(C)]
struct IDWriteFontVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetFontFamily: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetWeight: unsafe extern "system" fn(*mut c_void) -> u32,
    GetStretch: unsafe extern "system" fn(*mut c_void) -> u32,
    GetStyle: unsafe extern "system" fn(*mut c_void) -> u32,
    IsSymbolFont: unsafe extern "system" fn(*mut c_void) -> i32,
    GetFaceNames: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetInformationalStrings: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    GetSimulations: unsafe extern "system" fn(*mut c_void) -> u32,
    GetMetrics: unsafe extern "system" fn(*mut c_void, *mut c_void),
    HasCharacter: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32,
    CreateFontFace: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}
#[repr(C)]
struct IDWriteFont {
    vtbl: *const IDWriteFontVtbl,
}

#[repr(C)]
struct IDWriteFactory2Vtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    GetSystemFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    CreateCustomFontCollection:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32, *mut c_void) -> i32,
    RegisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    UnregisterFontCollectionLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateFontFileReference: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32,
    CreateCustomFontFileReference:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32,
    CreateFontFace: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, u32, u32, *mut c_void) -> i32,
    CreateRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateMonitorRenderingParams: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    CreateCustomRenderingParams: unsafe extern "system" fn(*mut c_void, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    RegisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    UnregisterFontFileLoader: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateTextFormat: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        f32,
        *mut c_void,
        *mut c_void,
    ) -> i32,
    CreateTypography: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    GetGdiInterop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateTextLayout:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, f32, f32, *mut c_void) -> i32,
    CreateGdiCompatibleTextLayout: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        f32,
        f32,
        f32,
        *mut c_void,
        i32,
        *mut c_void,
    ) -> i32,
    CreateEllipsisTrimmingSign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    CreateTextAnalyzer: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    CreateNumberSubstitution: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, i32, *mut c_void) -> i32,
    CreateGlyphRunAnalysis:
        unsafe extern "system" fn(*mut c_void, *mut c_void, f32, *mut c_void, u32, u32, f32, f32, *mut c_void) -> i32,
    GetEudcFontCollection: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> i32,
    CreateCustomRenderingParams_1:
        unsafe extern "system" fn(*mut c_void, f32, f32, f32, f32, u32, u32, *mut c_void) -> i32,
    GetSystemFontFallback: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}
#[repr(C)]
struct IDWriteFactory2 {
    vtbl: *const IDWriteFactory2Vtbl,
}

#[repr(C)]
struct IDWriteFontFallbackVtbl {
    QueryInterface: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    Release: unsafe extern "system" fn(*mut c_void) -> u32,
    MapCharacters: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *mut c_void,
        *const u16,
        u32,
        u32,
        u32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> i32,
}
#[repr(C)]
struct IDWriteFontFallback {
    vtbl: *const IDWriteFontFallbackVtbl,
}

const IID_IDWriteFactory: GUID =
    GUID { d1: 0xB859EE5A, d2: 0xD838, d3: 0x4B5B, d4: [0xA2, 0xE8, 0x1A, 0xDC, 0x7D, 0x93, 0xDB, 0x48] };
const IID_IDWriteFactory2: GUID =
    GUID { d1: 0x0439FC60, d2: 0xCA44, d3: 0x4994, d4: [0x8D, 0xEE, 0x3A, 0x9A, 0xF7, 0xB7, 0x32, 0xEC] };
const IID_IDWriteTextAnalysisSource: GUID =
    GUID { d1: 0x688E1A58, d2: 0x5094, d3: 0x47C8, d4: [0xAD, 0xC8, 0xFB, 0xCE, 0xA6, 0x0A, 0xE9, 0x2B] };
const IID_IDWriteNumberSubstitution: GUID =
    GUID { d1: 0x14885CC9, d2: 0xBAB0, d3: 0x4F90, d4: [0xB6, 0xED, 0x5C, 0x36, 0x6A, 0x2C, 0xD0, 0x3D] };
const DWRITE_FACTORY_TYPE_SHARED: u32 = 0;
const DWRITE_FACTORY_TYPE_ISOLATED: u32 = 1;

#[link(name = "dwrite", kind = "raw-dylib")]
unsafe extern "system" {
    fn DWriteCreateFactory(kind: u32, iid: *const GUID, factory: *mut *mut c_void) -> HRESULT;
}

/// HRESULT as `0x........`.
struct Hr(HRESULT);
impl core::fmt::Display for Hr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:08x}", self.0 as u32)
    }
}

/// A factory (as IDWriteFactory2, the layout is a prefix-compatible extension), exit 9 if none.
fn factory2(kind: u32) -> *mut IDWriteFactory2 {
    let mut f: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { DWriteCreateFactory(kind, &IID_IDWriteFactory2, &mut f) };
    if hr != 0 || f.is_null() {
        exit(9);
    }
    f as *mut IDWriteFactory2
}

/// The system font collection, exit 9 if none.
fn sys_collection(fac: *mut IDWriteFactory2) -> *mut IDWriteFontCollection {
    let mut c: *mut c_void = core::ptr::null_mut();
    // SAFETY: the factory is live.
    let hr = unsafe { com!(fac, GetSystemFontCollection(outp!(c), 0)) };
    if hr != 0 || c.is_null() {
        exit(9);
    }
    c as *mut IDWriteFontCollection
}

/// FindFamilyName with a face (no NUL): (hr, exists, index).
fn find_family(c: *mut IDWriteFontCollection, face: &[u16]) -> (HRESULT, i32, u32) {
    let mut w = [0u16; 64];
    let n = face.len().min(63);
    w[..n].copy_from_slice(&face[..n]);
    let (mut idx, mut ex) = (7u32, 7i32);
    // SAFETY: valid pointers.
    let hr = unsafe { com!(c, FindFamilyName(w.as_mut_ptr() as *mut c_void, outp!(idx), outp!(ex))) };
    (hr, ex, idx)
}

/// The en-us (else first) family name of a font's family, UTF-16 without NUL, in a buffer.
fn family_name_of_font(font: *mut IDWriteFont) -> ([u16; 64], usize) {
    let mut out = [0u16; 64];
    let mut fam: *mut c_void = core::ptr::null_mut();
    // SAFETY: the font is live; the pointers are valid.
    unsafe {
        if com!(font, GetFontFamily(outp!(fam))) != 0 || fam.is_null() {
            return (out, 0);
        }
        let n = family_names_first(fam as *mut IDWriteFontFamily, &mut out);
        release(fam);
        (out, n)
    }
}

/// The en-us (else first) name of a family into `out`; its length.
fn family_names_first(fam: *mut IDWriteFontFamily, out: &mut [u16; 64]) -> usize {
    let mut names: *mut c_void = core::ptr::null_mut();
    // SAFETY: the family is live; the pointers are valid.
    unsafe {
        if com!(fam, GetFamilyNames(outp!(names))) != 0 || names.is_null() {
            return 0;
        }
        let s = names as *mut IDWriteLocalizedStrings;
        let en: [u16; 6] = [b'e' as u16, b'n' as u16, b'-' as u16, b'u' as u16, b's' as u16, 0];
        let (mut idx, mut ex) = (0u32, 0i32);
        com!(s, FindLocaleName(en.as_ptr() as *mut c_void, outp!(idx), outp!(ex)));
        if ex == 0 {
            idx = 0;
        }
        com!(s, GetString(idx, out.as_mut_ptr() as *mut c_void, 64));
        release(names);
    }
    out.iter().position(|&c| c == 0).unwrap_or(64)
}

// A text-analysis source for MapCharacters (the caller's object: IUnknown + the five methods of
// IDWriteTextAnalysisSource), with a locale and a number substitution of the caller's.
#[repr(C)]
struct SrcVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    at: unsafe extern "system" fn(*mut c_void, u32, *mut *const u16, *mut u32) -> i32,
    before: unsafe extern "system" fn(*mut c_void, u32, *mut *const u16, *mut u32) -> i32,
    dir: unsafe extern "system" fn(*mut c_void) -> u32,
    locale: unsafe extern "system" fn(*mut c_void, u32, *mut u32, *mut *const u16) -> i32,
    subst: unsafe extern "system" fn(*mut c_void, u32, *mut u32, *mut *mut c_void) -> i32,
}
#[repr(C)]
struct Src {
    vtbl: *const SrcVtbl,
    text: *const u16,
    len: u32,
    locale: *const u16,
    subst: *mut c_void,
}
unsafe extern "system" fn src_q(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> i32 {
    // SAFETY: valid pointers from DirectWrite.
    unsafe {
        let iid = *(riid as *const GUID);
        let out = out as *mut *mut c_void;
        if iid == IID_IUnknown || iid == IID_IDWriteTextAnalysisSource {
            *out = this;
            0
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}
unsafe extern "system" fn src_ar(_: *mut c_void) -> u32 {
    2
}
unsafe extern "system" fn src_rl(_: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn src_at(this: *mut c_void, pos: u32, t: *mut *const u16, n: *mut u32) -> i32 {
    // SAFETY: `this` is our Src; the out pointers are valid.
    unsafe {
        let s = &*(this as *const Src);
        if pos >= s.len {
            *t = core::ptr::null();
            *n = 0;
        } else {
            *t = s.text.add(pos as usize);
            *n = s.len - pos;
        }
    }
    0
}
unsafe extern "system" fn src_before(this: *mut c_void, pos: u32, t: *mut *const u16, n: *mut u32) -> i32 {
    // SAFETY: `this` is our Src; the out pointers are valid.
    unsafe {
        let s = &*(this as *const Src);
        if pos == 0 || pos > s.len {
            *t = core::ptr::null();
            *n = 0;
        } else {
            *t = s.text;
            *n = pos;
        }
    }
    0
}
unsafe extern "system" fn src_dir(_: *mut c_void) -> u32 {
    0
}
unsafe extern "system" fn src_locale(this: *mut c_void, pos: u32, n: *mut u32, l: *mut *const u16) -> i32 {
    // SAFETY: `this` is our Src; the out pointers are valid.
    unsafe {
        let s = &*(this as *const Src);
        *n = s.len.saturating_sub(pos);
        *l = s.locale;
    }
    0
}
unsafe extern "system" fn src_subst(this: *mut c_void, pos: u32, n: *mut u32, o: *mut *mut c_void) -> i32 {
    // SAFETY: `this` is our Src; the out pointers are valid.
    unsafe {
        let s = &*(this as *const Src);
        *n = s.len.saturating_sub(pos);
        if !s.subst.is_null() {
            ((*(*(s.subst as *mut IUnknown)).vtbl).AddRef)(s.subst);
        }
        *o = s.subst;
    }
    0
}
static SRC_VTBL: SrcVtbl = SrcVtbl {
    qi: src_q,
    add_ref: src_ar,
    release: src_rl,
    at: src_at,
    before: src_before,
    dir: src_dir,
    locale: src_locale,
    subst: src_subst,
};

/// The user's default locale name (NUL-terminated) as Skia asks for it.
fn user_locale() -> [u16; 85] {
    let mut b = [0u16; 85];
    // SAFETY: the buffer holds 85 units (LOCALE_NAME_MAX_LENGTH).
    unsafe { GetUserDefaultLocaleName(b.as_mut_ptr(), 85) };
    b
}

/// One MapCharacters call over the whole of `text`: (hr, mapped length, font, scale).
fn map_chars(
    fb: *mut IDWriteFontFallback,
    text: &[u16],
    locale: *const u16,
    subst: *mut c_void,
    coll: *mut IDWriteFontCollection,
    base: *const u16,
    weight: u32,
    style: u32,
    stretch: u32,
) -> (HRESULT, u32, *mut IDWriteFont, f32) {
    let src = Src { vtbl: &SRC_VTBL, text: text.as_ptr(), len: text.len() as u32, locale, subst };
    let (mut mlen, mut font, mut scale) = (77u32, 0x10usize as *mut c_void, 7.0f32);
    // SAFETY: the fallback and the source are live during the call; the out pointers are valid.
    let hr = unsafe {
        com!(
            fb,
            MapCharacters(
                &src as *const Src as *mut c_void,
                0,
                text.len() as u32,
                coll as *mut c_void,
                base,
                weight,
                style,
                stretch,
                outp!(mlen),
                outp!(font),
                outp!(scale),
            )
        )
    };
    (hr, mlen, font as *mut IDWriteFont, scale)
}

poly_wtest::main!(test);

/// Make, select, observe; restore. (made, GetObjectW same, face, face length, tm ok, tm)
fn probe(dc: HANDLE, lf: &LOGFONTW) -> (bool, bool, [u16; 64], usize, i32, TEXTMETRICW) {
    // SAFETY: valid LOGFONTW and DC; the font is deleted before returning.
    unsafe {
        let f = CreateFontIndirectW(lf);
        let mut back = LOGFONTW::zero();
        GetObjectW(f, core::mem::size_of::<LOGFONTW>() as i32, &mut back as *mut _ as *mut c_void);
        let old = SelectObject(dc, f);
        let (face, n, _) = text_face(dc);
        let mut tm: TEXTMETRICW = core::mem::zeroed();
        let ok = GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        DeleteObject(f);
        (!f.is_null(), back == *lf, face, n, ok, tm)
    }
}

fn test(t: &mut Test) {
    // SAFETY: GetDC(NULL) is the screen DC; released below.
    let dc = unsafe { GetDC(core::ptr::null_mut()) };
    let msg = ncm().map(|m| m.lfMessageFont).unwrap_or(LOGFONTW::zero());
    let cases: [(&str, i32, i32, u8, u8, u8); 10] = [
        ("height -12, DEFAULT_CHARSET", -12, 400, 0, DEFAULT_CHARSET, 0),
        ("height -12, ANSI_CHARSET", -12, 400, 0, ANSI_CHARSET, 0),
        ("height -16, bold", -16, 700, 0, DEFAULT_CHARSET, 0),
        ("height -16, italic", -16, 400, 1, DEFAULT_CHARSET, 0),
        ("height -9", -9, 400, 0, DEFAULT_CHARSET, 0),
        ("height -24", -24, 400, 0, DEFAULT_CHARSET, 0),
        ("height 0", 0, 400, 0, DEFAULT_CHARSET, 0),
        ("height -12, FF_ROMAN|VARIABLE_PITCH", -12, 400, 0, DEFAULT_CHARSET, FF_ROMAN | VARIABLE_PITCH),
        ("height -12, FF_MODERN|FIXED_PITCH", -12, 400, 0, DEFAULT_CHARSET, FF_MODERN | FIXED_PITCH),
        ("height -12, FF_SWISS", -12, 400, 0, DEFAULT_CHARSET, FF_SWISS),
    ];
    for (name, h, w, it, cs, pf) in cases {
        for face in ["PolyNoSuch Face", "Segoe UI PolyNoSuch"] {
            let mut lf = LOGFONTW::with_face(h, face);
            lf.lfWeight = w;
            lf.lfItalic = it;
            lf.lfCharSet = cs;
            lf.lfPitchAndFamily = pf;
            let mut empty = lf;
            empty.lfFaceName = [0; 32];
            let (made, same, f, n, ok, tm) = probe(dc, &lf);
            let (_, _, ef, en, _, etm) = probe(dc, &empty);
            let (er, ecalls, _) = enum_face(dc, &f[..n]);
            say!(
                t,
                "'{}' {}: made {} GetObjectW gives the request {}; DC face: the requested {} empty {} the message font's {} the empty-face font's {} enumerated {} ({})",
                face,
                name,
                made as i32,
                same as i32,
                eq_nocase(&f[..n], lf.face()) as i32,
                (n == 0) as i32,
                eq_nocase(&f[..n], msg.face()) as i32,
                eq_nocase(&f[..n], &ef[..en]) as i32,
                (ecalls > 0) as i32,
                er
            );
            say!(
                t,
                "  GetTextMetricsW {} height-internal leading = -height {} (>0 {}) same height as the empty-face font {}",
                ok,
                (tm.tmHeight - tm.tmInternalLeading == -h) as i32,
                (tm.tmHeight - tm.tmInternalLeading > 0) as i32,
                (tm.tmHeight == etm.tmHeight) as i32
            );
        }
    }
    // SAFETY: the DC came from GetDC above.
    unsafe { ReleaseDC(core::ptr::null_mut(), dc) };
}
