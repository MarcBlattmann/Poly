//! WM_NCHITTEST (DefWindowProc) runs along the four edges of windows of several styles and sizes (the corner length against the window size and style).
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
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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
struct MINMAXINFO {
    reserved: POINT,
    max_size: POINT,
    max_position: POINT,
    min_track_size: POINT,
    max_track_size: POINT,
}

/// winuser.h
#[repr(C)]
struct WINDOWPOS {
    hwnd: HWND,
    insert_after: HWND,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
}

/// winuser.h
#[repr(C)]
struct NCCALCSIZE_PARAMS {
    rgrc: [RECT; 3],
    pos: *mut WINDOWPOS,
}

const WM_NULL: u32 = 0x0000;
const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_MOVE: u32 = 0x0003;
const WM_SIZE: u32 = 0x0005;
const WM_ACTIVATE: u32 = 0x0006;
const WM_SETFOCUS: u32 = 0x0007;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_ENABLE: u32 = 0x000A;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SHOWWINDOW: u32 = 0x0018;
const WM_ACTIVATEAPP: u32 = 0x001C;
const WM_SETCURSOR: u32 = 0x0020;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const WM_WINDOWPOSCHANGED: u32 = 0x0047;
const WM_STYLECHANGING: u32 = 0x007C;
const WM_STYLECHANGED: u32 = 0x007D;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCDESTROY: u32 = 0x0082;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCHITTEST: u32 = 0x0084;
const WM_NCPAINT: u32 = 0x0085;
const WM_NCACTIVATE: u32 = 0x0086;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_DEADCHAR: u32 = 0x0103;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
const WM_COMMAND: u32 = 0x0111;
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_CHILDACTIVATE: u32 = 0x0022;
const WM_NCMOUSEMOVE: u32 = 0x00A0;
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
const WM_CANCELMODE: u32 = 0x001F;
const WM_INPUTLANGCHANGE: u32 = 0x0051;
const WM_USER: u32 = 0x0400;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
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
const SM_CXFRAME: i32 = 32;
const SM_CYFRAME: i32 = 33;
const SM_CXMINTRACK: i32 = 34;
const SM_CYMINTRACK: i32 = 35;
const SM_CXPADDEDBORDER: i32 = 92;
const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTNOWHERE: isize = 0;
const HTERROR: isize = -2;
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

fn makelparam(lo: i32, hi: i32) -> LPARAM {
    ((lo as u32 & 0xFFFF) | ((hi as u32 & 0xFFFF) << 16)) as i32 as isize
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

fn rect_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

fn client_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetClientRect(hwnd, &mut r) };
    r
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

/// The documented messages the sequence tests record; everything else
/// (undocumented and environment-dependent traffic) is left out, and so are
/// the painting messages (WM_PAINT, WM_ERASEBKGND, WM_NCPAINT), which come
/// only for windows on a visible desktop.
fn documented(msg: u32) -> bool {
    matches!(
        msg,
        WM_NCCREATE
            | WM_CREATE
            | WM_NCCALCSIZE
            | WM_GETMINMAXINFO
            | WM_SIZE
            | WM_MOVE
            | WM_SHOWWINDOW
            | WM_WINDOWPOSCHANGING
            | WM_WINDOWPOSCHANGED
            | WM_DESTROY
            | WM_NCDESTROY
            | WM_PARENTNOTIFY
            | WM_SETTEXT
            | WM_GETTEXT
            | WM_GETTEXTLENGTH
            | WM_STYLECHANGING
            | WM_STYLECHANGED
            | WM_CLOSE
            | WM_TIMER
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_SYSKEYDOWN
            | WM_SYSKEYUP
            | WM_SYSCHAR
            | WM_DEADCHAR
            | WM_SYSCOMMAND
            | WM_CONTEXTMENU
            | WM_COMMAND
            | WM_ENABLE
            | WM_CHILDACTIVATE
            | WM_QUIT
            | WM_NULL
    ) || (WM_USER..0xC000).contains(&msg)
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// The rows of a small top-down 32-bit DIB as text: one character per
/// pixel from `key` (pixel value, char), '?' for others.
fn rows(t: &mut Test, bits: *mut u32, w: i32, h: i32, key: &[(u32, char)]) {
    for y in 0..h {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "  |");
        for x in 0..w {
            let v = px(bits, w, x, y);
            let c = key.iter().find(|k| k.0 == v).map(|k| k.1).unwrap_or('?');
            let _ = b.write_char(c);
        }
        let _ = b.write_str("|");
        t.line(b.as_str());
    }
}

type HMONITOR = *mut c_void;
/// winuser.h: INPUT on x64 (type, then the union at offset 8; 40 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct INPUT {
    kind: u32,
    pad: u32,
    data: [u64; 4],
}

/// winuser.h
#[repr(C)]
struct MONITORINFOEXW {
    cb_size: u32,
    monitor: RECT,
    work: RECT,
    flags: u32,
    device: [u16; 32],
}

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;
const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_NCLBUTTONUP: u32 = 0x00A2;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
const SM_CXSIZE: i32 = 30;
/// A mouse INPUT (Learn: MOUSEINPUT at offset 8: dx, dy, mouseData,
/// dwFlags, time, dwExtraInfo).
fn mouse_input(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
    let mut i = INPUT { kind: INPUT_MOUSE, pad: 0, data: [0; 4] };
    i.data[0] = (dx as u32 as u64) | ((dy as u32 as u64) << 32);
    i.data[1] = (data as u64) | ((flags as u64) << 32);
    i
}

/// A keyboard INPUT (KEYBDINPUT at offset 8: wVk, wScan, dwFlags, time,
/// dwExtraInfo).
fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    let mut i = INPUT { kind: INPUT_KEYBOARD, pad: 0, data: [0; 4] };
    i.data[0] = (vk as u64) | ((scan as u64) << 16) | ((flags as u64) << 32);
    i
}

/// SendInput of a list; returns how many were inserted.
fn send(inputs: &[INPUT]) -> u32 {
    // SAFETY: a slice of filled INPUTs of the documented size.
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), core::mem::size_of::<INPUT>() as i32) }
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

const WM_ENTERSIZEMOVE: u32 = 0x0231;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn Sleep(ms: u32);
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
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn ClientToScreen(hwnd: HWND, pt: *mut POINT) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SendInput(n: u32, inputs: *const INPUT, size: i32) -> u32;
    fn MonitorFromWindow(hwnd: HWND, flags: u32) -> HMONITOR;
    fn GetMonitorInfoW(monitor: HMONITOR, info: *mut MONITORINFOEXW) -> BOOL;
}

use core::fmt::Write as _;
use poly_wtest::*;

fn ht_name(h: isize) -> &'static str {
    match h {
        -2 => "HTERROR",
        -1 => "HTTRANSPARENT",
        0 => "HTNOWHERE",
        1 => "HTCLIENT",
        2 => "HTCAPTION",
        3 => "HTSYSMENU",
        4 => "HTGROWBOX",
        5 => "HTMENU",
        6 => "HTHSCROLL",
        7 => "HTVSCROLL",
        8 => "HTMINBUTTON",
        9 => "HTMAXBUTTON",
        10 => "HTLEFT",
        11 => "HTRIGHT",
        12 => "HTTOP",
        13 => "HTTOPLEFT",
        14 => "HTTOPRIGHT",
        15 => "HTBOTTOM",
        16 => "HTBOTTOMLEFT",
        17 => "HTBOTTOMRIGHT",
        18 => "HTBORDER",
        20 => "HTCLOSE",
        21 => "HTHELP",
        _ => "HT?",
    }
}

fn nm(m: u32) -> &'static str {
    match m {
        0x0001 => "WM_CREATE",
        0x0003 => "WM_MOVE",
        0x0005 => "WM_SIZE",
        0x0006 => "WM_ACTIVATE",
        0x0007 => "WM_SETFOCUS",
        0x0008 => "WM_KILLFOCUS",
        0x0018 => "WM_SHOWWINDOW",
        0x001C => "WM_ACTIVATEAPP",
        0x001F => "WM_CANCELMODE",
        0x0020 => "WM_SETCURSOR",
        0x0021 => "WM_MOUSEACTIVATE",
        0x0024 => "WM_GETMINMAXINFO",
        0x0046 => "WM_WINDOWPOSCHANGING",
        0x0047 => "WM_WINDOWPOSCHANGED",
        0x0083 => "WM_NCCALCSIZE",
        0x0084 => "WM_NCHITTEST",
        0x0086 => "WM_NCACTIVATE",
        0x00A0 => "WM_NCMOUSEMOVE",
        0x00A1 => "WM_NCLBUTTONDOWN",
        0x00A2 => "WM_NCLBUTTONUP",
        0x00A3 => "WM_NCLBUTTONDBLCLK",
        0x0100 => "WM_KEYDOWN",
        0x0101 => "WM_KEYUP",
        0x0104 => "WM_SYSKEYDOWN",
        0x0105 => "WM_SYSKEYUP",
        0x0112 => "WM_SYSCOMMAND",
        0x0200 => "WM_MOUSEMOVE",
        0x0201 => "WM_LBUTTONDOWN",
        0x0202 => "WM_LBUTTONUP",
        0x0203 => "WM_LBUTTONDBLCLK",
        0x020A => "WM_MOUSEWHEEL",
        0x020E => "WM_MOUSEHWHEEL",
        0x0214 => "WM_SIZING",
        0x0215 => "WM_CAPTURECHANGED",
        0x0216 => "WM_MOVING",
        0x0231 => "WM_ENTERSIZEMOVE",
        0x0232 => "WM_EXITSIZEMOVE",
        0x02A3 => "WM_MOUSELEAVE",
        0x02A2 => "WM_NCMOUSELEAVE",
        0x0050 => "WM_INPUTLANGCHANGEREQUEST",
        0x0051 => "WM_INPUTLANGCHANGE",
        0x00A4 => "WM_NCRBUTTONDOWN",
        0x00A5 => "WM_NCRBUTTONUP",
        _ => "",
    }
}

/// The messages left out of every trace: painting (its count and timing vary), timers, icons,
/// the shell's and the compositor's undocumented traffic, registered messages.
fn quiet(m: u32) -> bool {
    if *QUIET_MOUSE.get() && matches!(m, 0x0020 | 0x0084 | 0x0200 | 0x00A0 | 0x02A2 | 0x02A3) {
        return true;
    }
    matches!(m, 0x000F | 0x0014 | 0x0085 | 0x0088 | 0x007F | 0x0113 | 0x0060 | 0x0090 | 0x0096 | 0x00AE | 0x0272 | 0x0287
        | 0x031E | 0x031F | 0x0320 | 0x0321 | 0x02B1 | 0x0081 | 0x0082 | 0x0002 | 0x0010 | 0x0011 | 0x0012 | 0x0129 | 0x0030
        | 0x0090 | 0x00C1 | 0x00C2 | 0x00C3 | 0x0049 | 0x0281 | 0x0282 | 0x0283 | 0x0285 | 0x0286)
        || m >= 0xC000
}

/// The monitor's work area, set by `set_rel`: [left, top, width, height]. Numbers close to it
/// (within 40) are printed relative to it ("L-8", "W+16"), so the traces do not depend on where the
/// taskbar is.
static REL: Global<[i32; 4]> = Global::new([0, 0, 0, 0]);
static REL_ON: Global<bool> = Global::new(false);

/// A number printed relative to a base (a letter and a signed offset) when it is close to it.
struct Sym(i32, i32, &'static str);
impl core::fmt::Display for Sym {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let d = self.0 - self.1;
        if *REL_ON.get() && (-40..=40).contains(&d) {
            write!(f, "{}{:+}", self.2, d)
        } else {
            write!(f, "{}", self.0)
        }
    }
}
fn sx(v: i32) -> Sym {
    Sym(v, REL.get()[0], "L")
}
fn sy(v: i32) -> Sym {
    Sym(v, REL.get()[1], "T")
}
fn sw(v: i32) -> Sym {
    Sym(v, REL.get()[2], "W")
}
fn sh(v: i32) -> Sym {
    Sym(v, REL.get()[3], "H")
}
fn sr(v: i32) -> Sym {
    Sym(v, REL.get()[0] + REL.get()[2], "R")
}
fn sb(v: i32) -> Sym {
    Sym(v, REL.get()[1] + REL.get()[3], "B")
}

/// The monitor's work area of a window.
fn workarea(hwnd: HWND) -> RECT {
    let mut mi: MONITORINFOEXW = zeroed();
    mi.cb_size = core::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: mi is writable.
    unsafe {
        let m = MonitorFromWindow(hwnd, 2);
        GetMonitorInfoW(m, &mut mi);
    }
    mi.work
}

/// Print numbers near the work area relative to it from now on.
fn set_rel(hwnd: HWND) {
    let wa = workarea(hwnd);
    *REL.get() = [wa.left, wa.top, wa.right - wa.left, wa.bottom - wa.top];
    *REL_ON.get() = true;
}

fn rect_at(l: LPARAM) -> RECT {
    // SAFETY: a RECT pointer that the message carries.
    unsafe { *(l as *const RECT) }
}

/// What a window procedure logs for a message, one line, before the default handling.
fn describe(msg: u32, w: WPARAM, l: LPARAM, out: &mut Buf<300>) {
    let n = nm(msg);
    if n.is_empty() {
        let _ = write!(out, "msg {msg:#06x} w={w:#x}");
        return;
    }
    let _ = out.write_str(n);
    match msg {
        0x0003 => {
            let _ = write!(out, " x={} y={}", sx(lo(l)), sy(hi(l)));
        }
        0x0005 => {
            let _ = write!(out, " type={} w={} h={}", w, sw(lo(l) as u16 as i32), sh(hi(l) as u16 as i32));
        }
        0x0006 => {
            let _ = write!(out, " state={} minimized={}", w & 0xFFFF, w >> 16);
        }
        0x0018 => {
            let _ = write!(out, " show={} status={}", w, l);
        }
        0x0020 => {
            let _ = write!(out, " hit={} {} msg={:#06x}", ht_name(lo(l) as i16 as isize), lo(l) as i16, hi(l) as u16);
        }
        0x0021 => {
            let _ = write!(out, " hit={} msg={:#06x}", ht_name(lo(l) as i16 as isize), hi(l) as u16);
        }
        0x0046 | 0x0047 => {
            // SAFETY: the WINDOWPOS the message carries.
            let p = unsafe { &*(l as *const WINDOWPOS) };
            let after = p.insert_after as isize;
            let _ = write!(
                out,
                " x={} y={} cx={} cy={} flags={:#06x} after={}",
                sx(p.x),
                sy(p.y),
                sw(p.cx),
                sh(p.cy),
                p.flags,
                if after == 0 || after == 1 || after == -1 || after == -2 { after } else { 7 }
            );
        }
        0x0083 => {
            let r = rect_at(l);
            if w != 0 {
                // SAFETY: NCCALCSIZE_PARAMS: rgrc[0] is the first RECT.
                let r0 = unsafe { *(l as *const RECT) };
                let _ = write!(out, " calc_valid_rects=1 rect={},{},{},{}", sx(r0.left), sy(r0.top), sr(r0.right), sb(r0.bottom));
            } else {
                let _ = write!(out, " calc_valid_rects=0 rect={},{},{},{}", sx(r.left), sy(r.top), sr(r.right), sb(r.bottom));
            }
        }
        0x0084 => {
            let _ = write!(out, " screen={},{}", lo(l), hi(l));
        }
        0x0086 => {
            let _ = write!(out, " active={}", w);
        }
        0x00A0 | 0x00A1 | 0x00A2 | 0x00A3 | 0x00A4 | 0x00A5 => {
            let _ = write!(out, " hit={} screen={},{}", ht_name(w as isize), lo(l), hi(l));
        }
        0x0112 => {
            let _ = write!(out, " cmd={:#06x} low={:#x} at={},{}", w & 0xFFF0, w & 0xF, lo(l), hi(l));
        }
        0x0200 | 0x0201 | 0x0202 | 0x0203 => {
            let _ = write!(out, " keys={:#x} client={},{}", w, lo(l), hi(l));
        }
        0x020A | 0x020E => {
            let _ = write!(out, " keys={:#x} delta={} screen={},{}", w & 0xFFFF, (w >> 16) as u16 as i16, lo(l), hi(l));
        }
        0x0214 => {
            let _ = write!(out, " edge={} rect={}", w, R(rect_at(l)));
        }
        0x0216 => {
            let _ = write!(out, " rect={}", R(rect_at(l)));
        }
        0x0100 | 0x0101 | 0x0104 | 0x0105 => {
            let _ = write!(out, " vk={:#04x}", w);
        }
        0x0215 => {}
        0x0050 => {
            let _ = write!(out, " flags={:#x} hkl={:#010x}", w, l as u32);
        }
        0x0051 => {
            let _ = write!(out, " charset={} hkl={:#010x}", w, l as u32);
        }
        _ => {}
    }
}

/// Log lines (the procedure's output) start with this prefix.
fn trace_line(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) {
    if quiet(msg) {
        return;
    }
    let mut b = Buf::<300>::new();
    // SAFETY: a plain query of a window of this thread.
    let tag = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u8;
    if tag != 0 {
        let _ = write!(b, "[{}] ", tag as char);
    }
    describe(msg, w, l, &mut b);
    logln!("{}", b.as_str());
}

/// The test window's procedure: logs, then the default.
unsafe extern "system" fn proc_trace(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    trace_line(hwnd, msg, w, l);
    // SAFETY: the default handling of the message just received.
    let r = unsafe { DefWindowProcW(hwnd, msg, w, l) };
    if msg == 0x0084 {
        logln!("  -> {} ({})", ht_name(r), r);
    }
    if msg == 0x0024 && !quiet(msg) {
        // SAFETY: the MINMAXINFO the message carries.
        let m = unsafe { &*(l as *const MINMAXINFO) };
        // SAFETY: plain queries.
        let (mx, my) = unsafe { (GetSystemMetrics(34), GetSystemMetrics(35)) };
        logln!(
            "  -> maxsize={},{} maxpos={},{} mintrack={}{} maxtrack={},{}",
            sw(m.max_size.x), sh(m.max_size.y), sx(m.max_position.x), sy(m.max_position.y),
            if m.min_track_size.x == mx && m.min_track_size.y == my { "SM_MINTRACK" } else { "other" }, "",
            m.max_track_size.x, m.max_track_size.y
        );
    }
    r
}

/// The screen size, for absolute mouse coordinates.
fn screen() -> (i32, i32) {
    // SAFETY: plain queries.
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// A MOUSEEVENTF_ABSOLUTE move to the pixel (x, y).
fn abs_move(x: i32, y: i32) -> INPUT {
    let (sw, sh) = screen();
    let nx = ((x as i64 * 65536 + sw as i64 - 1) / sw as i64) as i32;
    let ny = ((y as i64 * 65536 + sh as i64 - 1) / sh as i64) as i32;
    mouse_input(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)
}

/// One step of the driver thread's script: kind 0 move to (a, b), 1 left down, 2 left up,
/// 3 key down vk a, 4 key up vk a, 5 sleep a ms, 6 wheel a, 7 hwheel a, 8 right down, 9 right up.
#[derive(Clone, Copy)]
struct Step {
    kind: u8,
    a: i32,
    b: i32,
}
static SCRIPT: Global<[Step; 40]> = Global::new([Step { kind: 255, a: 0, b: 0 }; 40]);
static SCRIPT_DONE: Global<bool> = Global::new(false);
static TARGET: Global<usize> = Global::new(0);
static QUIET_MOUSE: Global<bool> = Global::new(false);

fn script(steps: &[Step]) {
    let s = SCRIPT.get();
    for (i, st) in steps.iter().enumerate() {
        s[i] = *st;
    }
    if steps.len() < s.len() {
        s[steps.len()] = Step { kind: 255, a: 0, b: 0 };
    }
    *SCRIPT_DONE.get() = false;
}
const fn mv(x: i32, y: i32) -> Step {
    Step { kind: 0, a: x, b: y }
}
const DOWN: Step = Step { kind: 1, a: 0, b: 0 };
const UP: Step = Step { kind: 2, a: 0, b: 0 };
const fn sl(ms: i32) -> Step {
    Step { kind: 5, a: ms, b: 0 }
}
const fn kdn(vk: i32) -> Step {
    Step { kind: 3, a: vk, b: 0 }
}
const fn kup(vk: i32) -> Step {
    Step { kind: 4, a: vk, b: 0 }
}

/// The driver thread: runs the script with SendInput (mouse events and keys), one step at a time.
unsafe extern "system" fn driver(_: *mut c_void) -> u32 {
    let s = SCRIPT.get();
    for st in s.iter() {
        match st.kind {
            0 => {
                send(&[abs_move(st.a, st.b)]);
            }
            1 => {
                send(&[mouse_input(0, 0, 0, MOUSEEVENTF_LEFTDOWN)]);
            }
            2 => {
                send(&[mouse_input(0, 0, 0, MOUSEEVENTF_LEFTUP)]);
            }
            3 => {
                send(&[key_input(st.a as u16, 0, 0)]);
            }
            4 => {
                send(&[key_input(st.a as u16, 0, KEYEVENTF_KEYUP)]);
            }
            5 => {
                // SAFETY: plain timing call.
                unsafe { Sleep(st.a as u32) };
            }
            6 => {
                send(&[mouse_input(0, 0, st.a as u32, MOUSEEVENTF_WHEEL)]);
            }
            7 => {
                send(&[mouse_input(0, 0, st.a as u32, 0x1000)]);
            }
            8 => {
                send(&[mouse_input(0, 0, 0, MOUSEEVENTF_RIGHTDOWN)]);
            }
            9 => {
                send(&[mouse_input(0, 0, 0, MOUSEEVENTF_RIGHTUP)]);
            }
            10 => {
                // SAFETY: a window handle of the main thread; ShowWindow works across threads.
                unsafe { ShowWindow(*TARGET.get() as HWND, st.a) };
            }
            11 => {
                // SAFETY: as above.
                unsafe { PostMessageW(*TARGET.get() as HWND, st.a as u32, st.b as WPARAM, 0) };
            }
            _ => break,
        }
    }
    *SCRIPT_DONE.get() = true;
    0
}

/// Start the driver, then pump until it is done (the modal loops run inside the dispatching),
/// then a little longer.
fn run_driver(extra_ms: u32) {
    // SAFETY: the driver is a plain thread function; its handle is closed at once.
    unsafe {
        let h = CreateThread(core::ptr::null_mut(), 0, driver, core::ptr::null_mut(), 0, core::ptr::null_mut());
        let mut n = 0;
        while !*SCRIPT_DONE.get() && n < 400 {
            settle(20);
            n += 1;
        }
        settle(extra_ms);
        CloseHandle(h);
    }
}

/// The test window, a tag A, created at (x, y) with `style`.
fn mkwin(cls: &[u16], style: u32, x: i32, y: i32, w: i32, h: i32) -> HWND {
    register(cls, Some(proc_trace), 0, 0, core::ptr::null_mut());
    let hwnd = create(WS_EX_TOPMOST & 0, cls, wz!("Win"), style, x, y, w, h, core::ptr::null_mut(), 0);
    // SAFETY: our window.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 65) };
    settle(150);
    set_rel(hwnd);
    hwnd
}

fn say_rect(t: &mut Test, label: &str, hwnd: HWND) {
    let r = rect_of(hwnd);
    let c = client_of(hwnd);
    say!(t, "{label}: window {} client {}x{}", R(r), c.right, c.bottom);
}

/// The metrics a hit-test depends on.
fn say_metrics(t: &mut Test) {
    // SAFETY: plain queries.
    unsafe {
        say!(
            t,
            "metrics: SM_CXFRAME {} SM_CYFRAME {} SM_CXPADDEDBORDER {} SM_CXBORDER {} SM_CYCAPTION {} SM_CXSIZE {} SM_CXMINTRACK {} SM_CYMINTRACK {}",
            GetSystemMetrics(SM_CXFRAME),
            GetSystemMetrics(SM_CYFRAME),
            GetSystemMetrics(SM_CXPADDEDBORDER),
            GetSystemMetrics(SM_CXBORDER),
            GetSystemMetrics(SM_CYCAPTION),
            GetSystemMetrics(30),
            GetSystemMetrics(SM_CXMINTRACK),
            GetSystemMetrics(SM_CYMINTRACK)
        );
    }
}

/// Where WM_NCHITTEST answers at the screen point, by sending it to the window.
fn hit_at(hwnd: HWND, x: i32, y: i32) -> isize {
    // SAFETY: our window.
    unsafe { SendMessageW(hwnd, 0x0084, 0, makelparam(x, y)) }
}

/// Start the driver thread without waiting for it.
fn start_driver() -> HANDLE {
    // SAFETY: the driver is a plain thread function.
    unsafe { CreateThread(core::ptr::null_mut(), 0, driver, core::ptr::null_mut(), 0, core::ptr::null_mut()) }
}

/// Wait for the driver (pumping), then `extra_ms` more.
fn finish_driver(h: HANDLE, extra_ms: u32) {
    let mut n = 0;
    while !*SCRIPT_DONE.get() && n < 400 {
        settle(20);
        n += 1;
    }
    settle(extra_ms);
    // SAFETY: the thread handle from start_driver.
    unsafe { CloseHandle(h) };
}

/// A window of the test with a tag letter, traced.
fn mkwin2(tag: u8, cls: &[u16], style: u32, ex: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND) -> HWND {
    register(cls, Some(proc_trace), 0, 0, core::ptr::null_mut());
    let hwnd = create(ex, cls, wz!("Win"), style, x, y, w, h, parent, 0);
    // SAFETY: our window.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, tag as isize) };
    set_rel(hwnd);
    hwnd
}

/// Runs of equal WM_NCHITTEST answers along a line of `n` points, appended as "a..b=NAME".
fn runs(hwnd: HWND, n: i32, f: &dyn Fn(i32) -> (i32, i32), out: &mut Buf<300>) {
    let mut last = isize::MIN;
    let mut start = 0;
    for i in 0..=n {
        let (x, y) = f(i);
        let v = if i == n { isize::MIN + 1 } else { hit_at(hwnd, x, y) };
        if v != last {
            if last != isize::MIN {
                let _ = write!(out, " {}..{}={}", start, i - 1, &ht_name(last)[2..]);
            }
            last = v;
            start = i;
        }
    }
}

/// The four lines of a window: along the top (2 rows down), bottom (2 rows up), left (2 columns in)
/// and right (2 columns in); and the client origin.
fn four(t: &mut Test, label: &str, hwnd: HWND) {
    let r = rect_of(hwnd);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: our window; p writable.
    unsafe { ClientToScreen(hwnd, &mut p) };
    let c = client_of(hwnd);
    let mut b = Buf::<300>::new();
    let _ = write!(b, "{label} {w}x{h} client at {},{} size {}x{}:", p.x - r.left, p.y - r.top, c.right, c.bottom);
    t.line(b.as_str());
    let mut b = Buf::<300>::new();
    let _ = write!(b, "  top+2 x:");
    runs(hwnd, w, &|i| (r.left + i, r.top + 2), &mut b);
    t.line(b.as_str());
    let mut b = Buf::<300>::new();
    let _ = write!(b, "  bottom-3 x:");
    runs(hwnd, w, &|i| (r.left + i, r.bottom - 3), &mut b);
    t.line(b.as_str());
    let mut b = Buf::<300>::new();
    let _ = write!(b, "  left+2 y:");
    runs(hwnd, h, &|i| (r.left + 2, r.top + i), &mut b);
    t.line(b.as_str());
    let mut b = Buf::<300>::new();
    let _ = write!(b, "  right-3 y:");
    runs(hwnd, h, &|i| (r.right - 3, r.top + i), &mut b);
    t.line(b.as_str());
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    say_metrics(t);
    let cls = wz!("HitSizes");
    let styles: [(&str, u32, u32); 4] = [
        ("overlapped", WS_OVERLAPPEDWINDOW, 0),
        ("popup+thick", WS_POPUP | WS_THICKFRAME, 0),
        ("tool window", WS_OVERLAPPED | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU, WS_EX_TOOLWINDOW),
        ("popup+thick+caption", WS_POPUP | WS_THICKFRAME | WS_CAPTION, 0),
    ];
    let sizes: [(i32, i32); 5] = [(400, 300), (200, 100), (1000, 600), (160, 160), (300, 500)];
    for (name, style, ex) in styles {
        for (w, h) in sizes {
            let hwnd = mkwin2(b'H', cls, style | WS_VISIBLE, ex, 300, 100, w, h, core::ptr::null_mut());
            settle(40);
            four(t, name, hwnd);
            // SAFETY: our window.
            unsafe { DestroyWindow(hwnd) };
        }
    }
}
