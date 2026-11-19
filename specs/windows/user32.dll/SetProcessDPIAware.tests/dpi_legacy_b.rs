//! SetProcessDPIAware, then the DPI queries, in a program that sets nothing else, on a display
//! scaled to 200 percent.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SIZE {
    cx: i32,
    cy: i32,
}

/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    wnd_proc: WNDPROC,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HINSTANCE,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: HBRUSH,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    time: u32,
    pt: POINT,
    private: u32,
}

const WM_CREATE: u32 = 0x0001;
const WM_MOVE: u32 = 0x0003;
const WM_SIZE: u32 = 0x0005;
const WM_NCCREATE: u32 = 0x0081;
const WM_MOUSEMOVE: u32 = 0x0200;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const CS_VREDRAW: u32 = 0x0001;
const CS_HREDRAW: u32 = 0x0002;
const SW_SHOWNOACTIVATE: i32 = 4;
const GWL_STYLE: i32 = -16;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const QS_KEY: u32 = 0x0001;
const QS_MOUSEMOVE: u32 = 0x0002;
const QS_MOUSEBUTTON: u32 = 0x0004;
const QS_POSTMESSAGE: u32 = 0x0008;
const QS_TIMER: u32 = 0x0010;
const QS_PAINT: u32 = 0x0020;
const QS_SENDMESSAGE: u32 = 0x0040;
const QS_HOTKEY: u32 = 0x0080;
const QS_RAWINPUT: u32 = 0x0400;
const QS_INPUT: u32 = QS_MOUSEMOVE | QS_MOUSEBUTTON | QS_KEY | QS_RAWINPUT;
const QS_ALLEVENTS: u32 = QS_INPUT | QS_POSTMESSAGE | QS_TIMER | QS_PAINT | QS_HOTKEY;
const QS_ALLINPUT: u32 = QS_ALLEVENTS | QS_SENDMESSAGE;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SM_CYCAPTION: i32 = 4;
const SM_CXBORDER: i32 = 5;
const SM_CYBORDER: i32 = 6;
const SM_CXFRAME: i32 = 32;
const SM_CYFRAME: i32 = 33;
const SM_CXMIN: i32 = 28;
const SM_CYMIN: i32 = 29;
const SM_CXMINTRACK: i32 = 34;
const SM_CYMINTRACK: i32 = 35;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CXEDGE: i32 = 45;
const SM_CYMENU: i32 = 15;
const SM_CXVSCROLL: i32 = 2;
const SM_CYHSCROLL: i32 = 3;
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
}

/// The message log: lines appended by window procedures, printed by the test.
struct Log {
    buf: [u8; 32768],
    len: usize,
    on: bool,
}

static LOG: Global<Log> = Global::new(Log { buf: [0; 32768], len: 0, on: true });
impl core::fmt::Write for Log {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

/// Append one line to the log (when logging is on).
fn logf(args: core::fmt::Arguments) {
    let log = LOG.get();
    if log.on {
        let _ = log.write_fmt(args);
        let _ = log.write_str("\n");
    }
}

macro_rules! logln {
    ($($a:tt)*) => { logf(format_args!($($a)*)) };
}

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
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Register a class with `proc_` (and no menu, icon or cursor).
fn register(name: &[u16], proc_: WNDPROC, style: u32, wnd_extra: i32, brush: HBRUSH) -> u16 {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        wnd_proc: proc_,
        cls_extra: 0,
        wnd_extra,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: brush,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW; the name is NUL-terminated and static.
    unsafe { RegisterClassExW(&wc) }
}

/// CreateWindowExW with this executable's instance.
fn create(
    ex: u32,
    class: &[u16],
    name: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    // SAFETY: NUL-terminated names; the parent is a window or NULL.
    unsafe {
        CreateWindowExW(
            ex,
            class.as_ptr(),
            if name.is_empty() { core::ptr::null() } else { name.as_ptr() },
            style,
            x,
            y,
            w,
            h,
            parent,
            menu as HMENU,
            instance(),
            core::ptr::null_mut(),
        )
    }
}

/// Dispatch every waiting message.
fn pump() -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable; it is the message just removed.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        n += 1;
    }
    n
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

type HMONITOR = *mut c_void;
const MONITOR_DEFAULTTONULL: u32 = 0;
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
const SM_CXDOUBLECLK: i32 = 36;
const SM_CXDRAG: i32 = 68;
const SM_CXHTHUMB: i32 = 10;
const SM_CYVTHUMB: i32 = 9;
const SM_CXSIZE: i32 = 30;
const SM_CYSIZE: i32 = 31;
const SM_CXSMSIZE: i32 = 52;
const SM_CYSMCAPTION: i32 = 51;
const SM_CXMENUSIZE: i32 = 54;
const SM_CXMENUCHECK: i32 = 71;
const SM_CXICONSPACING: i32 = 38;
/// Dispatch everything waiting, then keep waiting up to `ms` for anything
/// further (input arrives asynchronously), dispatching as it comes.
fn settle(ms: u32) -> u32 {
    let mut n = pump();
    // SAFETY: plain timing call.
    let start = unsafe { GetTickCount() };
    loop {
        // SAFETY: plain timing call.
        let elapsed = unsafe { GetTickCount() }.wrapping_sub(start);
        if elapsed >= ms {
            break;
        }
        // SAFETY: no handles; wakes on any input or message.
        unsafe { MsgWaitForMultipleObjects(0, core::ptr::null(), 0, ms - elapsed, QS_ALLINPUT) };
        n += pump();
    }
    n
}

/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

const WM_DPICHANGED: u32 = 0x02E0;
const WM_DPICHANGED_BEFOREPARENT: u32 = 0x02E2;
const WM_DPICHANGED_AFTERPARENT: u32 = 0x02E3;
const WM_GETDPISCALEDSIZE: u32 = 0x02E4;
type HANDLE = *mut c_void;
/// A window procedure that logs the DPI messages and the ones a window gets while it is made.
unsafe extern "system" fn dpi_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the window of this message.
    let style = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
    let kind = if style & WS_CHILD != 0 { "child" } else if style & WS_POPUP != 0 { "popup" } else { "top" };
    match m {
        WM_NCCREATE => {
            if ENCS.get().0 {
                set_error(0);
                // SAFETY: the window of this message.
                let r = unsafe { EnableNonClientDpiScaling(h) };
                logln!("  {kind} WM_NCCREATE: EnableNonClientDpiScaling -> {} error {}", r != 0, last_error());
            } else {
                logln!("  {kind} WM_NCCREATE");
            }
        }
        WM_CREATE => logln!("  {kind} WM_CREATE"),
        WM_GETDPISCALEDSIZE => {
            // SAFETY: l points to a SIZE for this message.
            let s = unsafe { &mut *(l as *mut SIZE) };
            logln!("  {kind} WM_GETDPISCALEDSIZE dpi {} size {}x{}", w, s.cx, s.cy);
            return 0;
        }
        WM_DPICHANGED => {
            // SAFETY: l points to the suggested RECT.
            let r = unsafe { *(l as *const RECT) };
            logln!("  {kind} WM_DPICHANGED dpi x {} y {} suggested {}", w & 0xFFFF, w >> 16, R(r));
        }
        WM_DPICHANGED_BEFOREPARENT => logln!("  {kind} WM_DPICHANGED_BEFOREPARENT"),
        WM_DPICHANGED_AFTERPARENT => logln!("  {kind} WM_DPICHANGED_AFTERPARENT"),
        WM_SIZE => logln!("  {kind} WM_SIZE type {} client {}x{}", w, lo(l), hi(l)),
        WM_MOVE => logln!("  {kind} WM_MOVE {},{}", lo(l), hi(l)),
        WM_MOUSEMOVE => {
            if ENCS.get().1 {
                logln!("  {kind} WM_MOUSEMOVE client {},{}", lo(l), hi(l));
            }
        }
        _ => {}
    }
    // SAFETY: the default procedure of this window.
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// (call EnableNonClientDpiScaling in WM_NCCREATE, log WM_MOUSEMOVE)
static ENCS: Global<(bool, bool)> = Global::new((false, false));

static CLASS_DONE: Global<bool> = Global::new(false);
/// A window of the test's class (registered at the first call), shown without activation.
fn dpi_window(x: i32, y: i32, w: i32, h: i32, style: u32, ex: u32, parent: HWND) -> HWND {
    let cls = wz!("PolyDpiWindow");
    if !*CLASS_DONE.get() {
        *CLASS_DONE.get() = true;
        register(cls, Some(dpi_proc), CS_HREDRAW | CS_VREDRAW, 0, core::ptr::null_mut());
    }
    let h = create(ex | WS_EX_NOACTIVATE, cls, wz!("dpi"), style, x, y, w, h, parent, 0);
    if !h.is_null() {
        // SAFETY: our window.
        unsafe { ShowWindow(h, SW_SHOWNOACTIVATE) };
        settle(150);
    }
    h
}

fn hr_of(h: i32) -> u32 {
    h as u32
}

fn primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// GetDpiForMonitor in every flavour, GetScaleFactorForMonitor and the monitor handles.
fn sec_mon(t: &mut Test) {
    // SAFETY: writable outputs; plain calls.
    unsafe {
        let m = primary();
        for (n, k) in [("effective", 0), ("angular", 1), ("raw", 2), ("3", 3), ("-1", -1)] {
            let (mut x, mut y) = (7u32, 7u32);
            let hr = GetDpiForMonitor(m, k, &mut x, &mut y);
            say!(t, "GetDpiForMonitor(primary, {n}) hr {:#x} x {} y {}", hr_of(hr), x, y);
        }
        let (mut x, mut y) = (7u32, 7u32);
        let hr = GetDpiForMonitor(core::ptr::null_mut(), 0, &mut x, &mut y);
        say!(t, "GetDpiForMonitor(NULL, effective) hr {:#x} x {} y {}", hr_of(hr), x, y);
        let mut s = -1i32;
        let hr = GetScaleFactorForMonitor(m, &mut s);
        say!(t, "GetScaleFactorForMonitor(primary) hr {:#x} factor {}", hr_of(hr), s);
        let mut s = -1i32;
        let hr = GetScaleFactorForMonitor(core::ptr::null_mut(), &mut s);
        say!(t, "GetScaleFactorForMonitor(NULL) hr {:#x} factor {}", hr_of(hr), s);
        let h = dpi_window(100, 100, 400, 300, WS_OVERLAPPEDWINDOW, 0, core::ptr::null_mut());
        let mw = MonitorFromWindow(h, MONITOR_DEFAULTTONULL);
        say!(t, "the window's monitor is the primary: {}", mw == m);
        let (mut x, mut y) = (7u32, 7u32);
        let hr = GetDpiForMonitor(mw, 0, &mut x, &mut y);
        say!(t, "GetDpiForMonitor(window's monitor, effective) hr {:#x} x {} y {}", hr_of(hr), x, y);
        DestroyWindow(h);
    }
}

impl R {
    fn to_string_eq(&self, o: &R) -> bool {
        self.0.left == o.0.left && self.0.top == o.0.top && self.0.right == o.0.right && self.0.bottom == o.0.bottom
    }
}

/// GetSystemMetrics against GetSystemMetricsForDpi.
fn sec_gsm(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe {
        let dpi = GetDpiForSystem();
        say!(t, "GetDpiForSystem {} SM_CXSCREEN x SM_CYSCREEN {}x{}", dpi, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let items = [
            ("CXVSCROLL", SM_CXVSCROLL),
            ("CYHSCROLL", SM_CYHSCROLL),
            ("CYCAPTION", SM_CYCAPTION),
            ("CXBORDER", SM_CXBORDER),
            ("CYBORDER", SM_CYBORDER),
            ("CYVTHUMB", SM_CYVTHUMB),
            ("CXHTHUMB", SM_CXHTHUMB),
            ("CXICON", SM_CXICON),
            ("CYICON", SM_CYICON),
            ("CYMENU", SM_CYMENU),
            ("CXMIN", SM_CXMIN),
            ("CYMIN", SM_CYMIN),
            ("CXSIZE", SM_CXSIZE),
            ("CYSIZE", SM_CYSIZE),
            ("CXFRAME", SM_CXFRAME),
            ("CYFRAME", SM_CYFRAME),
            ("CXMINTRACK", SM_CXMINTRACK),
            ("CYMINTRACK", SM_CYMINTRACK),
            ("CXDOUBLECLK", SM_CXDOUBLECLK),
            ("CXICONSPACING", SM_CXICONSPACING),
            ("CXEDGE", SM_CXEDGE),
            ("CXSMICON", SM_CXSMICON),
            ("CYSMICON", SM_CYSMICON),
            ("CYSMCAPTION", SM_CYSMCAPTION),
            ("CXSMSIZE", SM_CXSMSIZE),
            ("CXMENUSIZE", SM_CXMENUSIZE),
            ("CXDRAG", SM_CXDRAG),
            ("CXMENUCHECK", SM_CXMENUCHECK),
            ("CXPADDEDBORDER", SM_CXPADDEDBORDER),
        ];
        for (n, i) in items {
            let a = GetSystemMetrics(i);
            let f = GetSystemMetricsForDpi(i, dpi);
            say!(t, "{n}: {} for the system dpi {} for 96 {} for 192 {}", a, f, GetSystemMetricsForDpi(i, 96), GetSystemMetricsForDpi(i, 192));
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn MonitorFromWindow(hwnd: HWND, flags: u32) -> HMONITOR;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn GetDpiForSystem() -> u32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn SetProcessDPIAware() -> BOOL;
    fn EnableNonClientDpiScaling(hwnd: HWND) -> BOOL;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: HMONITOR, kind: i32, x: *mut u32, y: *mut u32) -> i32;
    fn GetScaleFactorForMonitor(monitor: HMONITOR, scale: *mut i32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain call.
    let r = unsafe { SetProcessDPIAware() };
    say!(t, "SetProcessDPIAware -> {}", r != 0);
    sec_gsm(t);
    sec_mon(t);
}
