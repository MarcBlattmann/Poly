//! EnumDisplayMonitors with a device context: the screen DC gives one callback
//! with that DC (the DC passed to the callback is not NULL), whose clip box is
//! the callback rectangle (simple region) and LOGPIXELSX 96; with a clip
//! rectangle both are cut; a window DC reports the rectangle in the DC's
//! coordinates (0,0,width,height of the window); the clip is in the DC's coordinates
//! too; a clip outside gives no callback and TRUE.
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
type HDC = *mut c_void;
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

const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const SW_SHOWNOACTIVATE: i32 = 4;
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
const LOGPIXELSX: i32 = 88;
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

/// Print every logged line (prefixed) and empty the log.
fn flush_log(t: &mut Test, prefix: &str) {
    let log = LOG.get();
    let mut start = 0;
    for i in 0..log.len {
        if log.buf[i] == b'\n' {
            let line = core::str::from_utf8(&log.buf[start..i]).unwrap_or("?");
            let mut b = Buf::<300>::new();
            let _ = write!(b, "{prefix}{line}");
            t.line(b.as_str());
            start = i + 1;
        }
    }
    log.len = 0;
}

fn log_clear() {
    LOG.get().len = 0;
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
/// winuser.h
#[repr(C)]
struct MONITORINFOEXW {
    cb_size: u32,
    monitor: RECT,
    work: RECT,
    flags: u32,
    device: [u16; 32],
}

const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
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

/// The primary monitor.
fn dp_primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// A window procedure that does nothing but the default handling (nothing
/// is logged).
unsafe extern "system" fn dp_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Register `name` with `dp_proc` and create a hidden popup window.
fn dp_popup(name: &[u16]) -> HWND {
    register(name, Some(dp_proc), 0, 0, core::ptr::null_mut());
    create(WS_EX_NOACTIVATE, name, wz!(""), WS_POPUP, 10, 10, 100, 50, core::ptr::null_mut(), 0)
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
    fn MoveWindow(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, repaint: BOOL) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn GetMonitorInfoW(monitor: HMONITOR, info: *mut MONITORINFOEXW) -> BOOL;
    fn EnumDisplayMonitors(
        hdc: HDC,
        clip: *const RECT,
        proc_: unsafe extern "system" fn(HMONITOR, HDC, *mut RECT, LPARAM) -> BOOL,
        data: LPARAM,
    ) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

poly_wtest::main!(test);

static E_N: Global<u32> = Global::new(0);
static E_STOP: Global<u32> = Global::new(0);

unsafe extern "system" fn e_cb(mon: HMONITOR, dc: HDC, r: *mut RECT, data: LPARAM) -> BOOL {
    *E_N.get() += 1;
    let n = *E_N.get();
    // SAFETY: r is the RECT the system gives.
    let rect = unsafe { *r };
    let mut mi: MONITORINFOEXW = zeroed();
    mi.cb_size = 104;
    // SAFETY: mi is valid.
    let ok = unsafe { GetMonitorInfoW(mon, &mut mi) };
    logln!(
        "  callback {}: monitor is primary {} dc NULL {} rect {} data {:#x} GetMonitorInfoW {} monitor {}",
        n,
        mon == dp_primary(),
        dc.is_null(),
        R(rect),
        data,
        ok,
        R(mi.monitor)
    );
    if !dc.is_null() {
        let mut cb = RECT::default();
        // SAFETY: the DC is valid inside the callback.
        let kind = unsafe { GetClipBox(dc, &mut cb) };
        logln!("    dc clip box kind {} {} LOGPIXELSX {}", kind, R(cb), unsafe { GetDeviceCaps(dc, LOGPIXELSX) });
    }
    if *E_STOP.get() == n { 0 } else { 1 }
}

fn run(t: &mut Test, label: &str, dc: HDC, clip: Option<RECT>, stop_at: u32) {
    *E_N.get() = 0;
    *E_STOP.get() = stop_at;
    log_clear();
    let cp = match &clip {
        Some(r) => r as *const RECT,
        None => core::ptr::null(),
    };
    set_error(0x1234);
    // SAFETY: the callback is valid; clip is NULL or a RECT.
    let r = unsafe { EnumDisplayMonitors(dc, cp, e_cb, 0x5A5A) };
    say!(t, "{label}: returns {} last error {:#x} callbacks {}", r, last_error(), *E_N.get());
    flush_log(t, "");
}

fn test(t: &mut Test) {
    // SAFETY: DCs are released.
    unsafe {
        let sdc = GetDC(core::ptr::null_mut());
        run(t, "screen DC", sdc, None, 0);
        run(t, "screen DC, clip inside", sdc, Some(RECT { left: 100, top: 100, right: 300, bottom: 300 }), 0);
        run(t, "screen DC, clip off", sdc, Some(RECT { left: 3000, top: 100, right: 3200, bottom: 300 }), 0);
        ReleaseDC(core::ptr::null_mut(), sdc);
        let w = dp_popup(wz!("EdmDc"));
        MoveWindow(w, 300, 200, 200, 150, 0);
        ShowWindow(w, SW_SHOWNOACTIVATE);
        settle(30);
        let wdc = GetDC(w);
        run(t, "window DC (window at 300,200 size 200x150)", wdc, None, 0);
        run(t, "window DC, clip 10,10,50,50", wdc, Some(RECT { left: 10, top: 10, right: 50, bottom: 50 }), 0);
        run(t, "window DC, clip beyond the window", wdc, Some(RECT { left: 250, top: 10, right: 350, bottom: 50 }), 0);
        ReleaseDC(w, wdc);
        DestroyWindow(w);
    }
}
