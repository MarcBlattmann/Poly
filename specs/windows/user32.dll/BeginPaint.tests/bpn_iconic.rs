//! BeginPaint and the paint messages of a minimized window: after
//! InvalidateRect(TRUE) of a minimized captioned window, what GetUpdateRect
//! (bErase FALSE, then TRUE) and the WM_PAINT it gets send, what a window
//! procedure that calls BeginPaint sees (rcPaint, fErase, the clip box, the
//! DC), the same through DefWindowProc, with a class that has an icon
//! (WM_ICONERASEBKGND) and without, BeginPaint called outside WM_PAINT on a
//! minimized window, and the window restored (what is invalid, what is
//! painted).
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

/// winuser.h
#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    erase: BOOL,
    paint: RECT,
    restore: BOOL,
    inc_update: BOOL,
    reserved: [u8; 32],
}

const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_NCPAINT: u32 = 0x0085;
const WS_POPUP: u32 = 0x8000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const SW_SHOWMINNOACTIVE: i32 = 7;
const SW_RESTORE: i32 = 9;
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
const COLOR_WINDOW: i32 = 5;
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

/// The cursor's position.
fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    // SAFETY: p is writable.
    unsafe { GetCursorPos(&mut p) };
    p
}

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

/// Puts the cursor back where it was when created (keep one alive for the
/// whole test in every test that moves the cursor).
struct CursorGuard(POINT);

impl CursorGuard {
    /// Remember the cursor's position and park it at (1500, 600), away from
    /// the test windows (which tests place in the top-left 1000x500 of the
    /// 1920x1080 screen), so no test window starts under it.
    fn new() -> Self {
        let g = CursorGuard(cursor_pos());
        // SAFETY: plain call.
        unsafe { SetCursorPos(1500, 600) };
        g
    }
}

impl Drop for CursorGuard {
    fn drop(&mut self) {
        // SAFETY: plain call.
        unsafe { SetCursorPos(self.0.x, self.0.y) };
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
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
    fn UpdateWindow(hwnd: HWND) -> BOOL;
    fn IsIconic(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetQueueStatus(flags: u32) -> u32;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn InvalidateRect(hwnd: HWND, rect: *const RECT, erase: BOOL) -> BOOL;
    fn GetUpdateRect(hwnd: HWND, rect: *mut RECT, erase: BOOL) -> BOOL;
    fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadIconW(instance: HINSTANCE, name: *const u16) -> *mut c_void;
}

static IC_PAINT: Global<i32> = Global::new(0); // 1: WM_PAINT calls BeginPaint
static IC_ERASE: Global<isize> = Global::new(1);

unsafe extern "system" fn ic_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_NCPAINT => logln!("  WM_NCPAINT rgn={}", if w == 1 { "1" } else { "region" }),
        WM_ERASEBKGND => {
            logln!("  WM_ERASEBKGND returns {}", *IC_ERASE.get());
            return *IC_ERASE.get();
        }
        0x0027 => logln!("  WM_ICONERASEBKGND"),
        WM_PAINT => {
            if *IC_PAINT.get() == 1 {
                let mut ps: PAINTSTRUCT = zeroed();
                let mut cb = RECT::default();
                // SAFETY: inside WM_PAINT of our window; ps and cb are writable.
                unsafe {
                    let dc = BeginPaint(hwnd, &mut ps);
                    let kind = GetClipBox(dc, &mut cb);
                    logln!(
                        "  WM_PAINT: BeginPaint dc={} erase={} rcPaint={} clipbox kind {} {}",
                        (!dc.is_null()) as i32,
                        ps.erase,
                        R(ps.paint),
                        kind,
                        R(cb)
                    );
                    EndPaint(hwnd, &ps);
                }
                return 0;
            }
            logln!("  WM_PAINT (DefWindowProc)");
        }
        _ => {}
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn ic_reg(name: &[u16], icon: bool) {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        wnd_proc: Some(ic_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance: instance(),
        // SAFETY: IDI_APPLICATION (32512) is a stock icon id.
        icon: if icon { unsafe { LoadIconW(core::ptr::null_mut(), 32512usize as *const u16) } } else { core::ptr::null_mut() },
        cursor: core::ptr::null_mut(),
        background: (COLOR_WINDOW + 1) as usize as HBRUSH,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW.
    unsafe { RegisterClassExW(&wc) };
}

fn ic_state(t: &mut Test, label: &str, h: HWND, erase: i32) {
    let mut r = RECT { left: 7, top: 7, right: 7, bottom: 7 };
    // SAFETY: r is writable; our window.
    let any = unsafe { GetUpdateRect(h, &mut r, erase) };
    // SAFETY: plain query.
    let qs = unsafe { GetQueueStatus(QS_PAINT) };
    say!(t, "{label}: GetUpdateRect(bErase={erase}) {} {} QS_PAINT now={}", any, R(r), ((qs >> 16) & QS_PAINT != 0) as i32);
    flush_log(t, "");
}

fn ic_run(t: &mut Test, tag: &str, class: &[u16]) {
    let ex = WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
    let f = create(
        ex,
        class,
        wz!("ic"),
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
        100,
        60,
        200,
        120,
        core::ptr::null_mut(),
        0,
    );
    settle(30);
    // SAFETY: our window.
    unsafe { ShowWindow(f, SW_SHOWMINNOACTIVE) };
    settle(30);
    log_clear();
    // SAFETY: plain query.
    say!(t, "== {tag}: minimized {}", unsafe { IsIconic(f) });
    for paint in 0..2 {
        *IC_PAINT.get() = paint;
        say!(t, "-- WM_PAINT {}", if paint == 1 { "calls BeginPaint" } else { "goes to DefWindowProc" });
        // SAFETY: our window.
        unsafe { InvalidateRect(f, core::ptr::null(), 1) };
        ic_state(t, "invalidated, erase", f, 0);
        ic_state(t, "same", f, 1);
        settle(30);
        flush_log(t, "pumped:");
        ic_state(t, "after pumping", f, 0);
        // SAFETY: our window.
        unsafe { InvalidateRect(f, core::ptr::null(), 0) };
        settle(30);
        flush_log(t, "pumped (no erase):");
    }
    *IC_PAINT.get() = 1;
    t.line("-- BeginPaint outside WM_PAINT, window minimized and invalid");
    // SAFETY: our window.
    unsafe { InvalidateRect(f, core::ptr::null(), 1) };
    let mut ps: PAINTSTRUCT = zeroed();
    let mut cb = RECT::default();
    // SAFETY: our window; ps and cb are writable.
    unsafe {
        let dc = BeginPaint(f, &mut ps);
        let kind = GetClipBox(dc, &mut cb);
        say!(t, "BeginPaint dc={} erase={} rcPaint={} clipbox kind {} {}", (!dc.is_null()) as i32, ps.erase, R(ps.paint), kind, R(cb));
        EndPaint(f, &ps);
    }
    flush_log(t, "during:");
    ic_state(t, "after", f, 0);
    settle(30);
    flush_log(t, "pumped:");
    t.line("-- UpdateWindow on the minimized invalid window");
    // SAFETY: our window.
    unsafe { InvalidateRect(f, core::ptr::null(), 1) };
    // SAFETY: our window.
    let r = unsafe { UpdateWindow(f) };
    say!(t, "UpdateWindow {r}");
    flush_log(t, "during:");
    ic_state(t, "after", f, 0);
    t.line("-- restored while invalid");
    // SAFETY: our window.
    unsafe { InvalidateRect(f, core::ptr::null(), 1) };
    // SAFETY: our window.
    unsafe { ShowWindow(f, SW_RESTORE) };
    // SAFETY: plain query.
    say!(t, "restored, minimized {}", unsafe { IsIconic(f) });
    log_clear();
    ic_state(t, "just restored", f, 0);
    settle(40);
    flush_log(t, "pumped:");
    ic_state(t, "after pumping", f, 0);
    // SAFETY: our window.
    unsafe { DestroyWindow(f) };
    log_clear();
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    ic_reg(wz!("IcNoIcon"), false);
    ic_reg(wz!("IcIcon"), true);
    ic_run(t, "class without an icon", wz!("IcNoIcon"));
    *IC_ERASE.get() = 1;
    ic_run(t, "class with an icon", wz!("IcIcon"));
}
