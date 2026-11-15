//! GetScrollRange on windows and controls that never had a range set, and
//! with arguments that are not valid.
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
const WM_NOTIFYFORMAT: u32 = 0x0055;
const WM_STYLECHANGING: u32 = 0x007C;
const WM_STYLECHANGED: u32 = 0x007D;
const WM_GETICON: u32 = 0x007F;
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
const WM_UNICHAR: u32 = 0x0109;
const WM_COMMAND: u32 = 0x0111;
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_QUERYENDSESSION: u32 = 0x0011;
const WM_SETREDRAW: u32 = 0x000B;
const WM_CHILDACTIVATE: u32 = 0x0022;
const WM_SETFONT: u32 = 0x0030;
const WM_GETFONT: u32 = 0x0031;
const WM_SETICON: u32 = 0x0080;
const WM_NCMOUSEMOVE: u32 = 0x00A0;
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_SYSDEADCHAR: u32 = 0x0107;
const WM_PRINTCLIENT: u32 = 0x0318;
const WM_SYNCPAINT: u32 = 0x0088;
const WM_GETDLGCODE: u32 = 0x0087;
const WM_CANCELMODE: u32 = 0x001F;
const WM_HELP: u32 = 0x0053;
const WM_INPUTLANGCHANGE: u32 = 0x0051;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_HSCROLL: u32 = 0x0010_0000;
const GWL_STYLE: i32 = -16;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const SM_CXVSCROLL: i32 = 2;
const SM_CYHSCROLL: i32 = 3;
const ERROR_ACCESS_DENIED: u32 = 5;
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

/// The name of a message (winuser.h), or None.
fn msg_name(m: u32) -> Option<&'static str> {
    Some(match m {
        0x0000 => "WM_NULL",
        0x0001 => "WM_CREATE",
        0x0002 => "WM_DESTROY",
        0x0003 => "WM_MOVE",
        0x0005 => "WM_SIZE",
        0x0006 => "WM_ACTIVATE",
        0x0007 => "WM_SETFOCUS",
        0x0008 => "WM_KILLFOCUS",
        0x000A => "WM_ENABLE",
        0x000B => "WM_SETREDRAW",
        0x000C => "WM_SETTEXT",
        0x000D => "WM_GETTEXT",
        0x000E => "WM_GETTEXTLENGTH",
        0x000F => "WM_PAINT",
        0x0010 => "WM_CLOSE",
        0x0011 => "WM_QUERYENDSESSION",
        0x0012 => "WM_QUIT",
        0x0013 => "WM_QUERYOPEN",
        0x0014 => "WM_ERASEBKGND",
        0x0015 => "WM_SYSCOLORCHANGE",
        0x0018 => "WM_SHOWWINDOW",
        0x001A => "WM_SETTINGCHANGE",
        0x001C => "WM_ACTIVATEAPP",
        0x001F => "WM_CANCELMODE",
        0x0020 => "WM_SETCURSOR",
        0x0021 => "WM_MOUSEACTIVATE",
        0x0022 => "WM_CHILDACTIVATE",
        0x0024 => "WM_GETMINMAXINFO",
        0x0026 => "WM_PAINTICON",
        0x0027 => "WM_ICONERASEBKGND",
        0x0028 => "WM_NEXTDLGCTL",
        0x002B => "WM_DRAWITEM",
        0x0030 => "WM_SETFONT",
        0x0031 => "WM_GETFONT",
        0x003D => "WM_GETOBJECT",
        0x0046 => "WM_WINDOWPOSCHANGING",
        0x0047 => "WM_WINDOWPOSCHANGED",
        0x004E => "WM_NOTIFY",
        0x0050 => "WM_INPUTLANGCHANGEREQUEST",
        0x0051 => "WM_INPUTLANGCHANGE",
        0x0053 => "WM_HELP",
        0x0055 => "WM_NOTIFYFORMAT",
        0x007B => "WM_CONTEXTMENU",
        0x007C => "WM_STYLECHANGING",
        0x007D => "WM_STYLECHANGED",
        0x007E => "WM_DISPLAYCHANGE",
        0x007F => "WM_GETICON",
        0x0080 => "WM_SETICON",
        0x0081 => "WM_NCCREATE",
        0x0082 => "WM_NCDESTROY",
        0x0083 => "WM_NCCALCSIZE",
        0x0084 => "WM_NCHITTEST",
        0x0085 => "WM_NCPAINT",
        0x0086 => "WM_NCACTIVATE",
        0x0087 => "WM_GETDLGCODE",
        0x0088 => "WM_SYNCPAINT",
        0x00A0 => "WM_NCMOUSEMOVE",
        0x00A1 => "WM_NCLBUTTONDOWN",
        0x00A2 => "WM_NCLBUTTONUP",
        0x00A3 => "WM_NCLBUTTONDBLCLK",
        0x00A4 => "WM_NCRBUTTONDOWN",
        0x00A5 => "WM_NCRBUTTONUP",
        0x0090 => "WM_UAHDESTROYWINDOW?",
        0x0100 => "WM_KEYDOWN",
        0x0101 => "WM_KEYUP",
        0x0102 => "WM_CHAR",
        0x0103 => "WM_DEADCHAR",
        0x0104 => "WM_SYSKEYDOWN",
        0x0105 => "WM_SYSKEYUP",
        0x0106 => "WM_SYSCHAR",
        0x0107 => "WM_SYSDEADCHAR",
        0x0109 => "WM_UNICHAR",
        0x010D => "WM_IME_STARTCOMPOSITION",
        0x0110 => "WM_INITDIALOG",
        0x0111 => "WM_COMMAND",
        0x0112 => "WM_SYSCOMMAND",
        0x0113 => "WM_TIMER",
        0x0114 => "WM_HSCROLL",
        0x0115 => "WM_VSCROLL",
        0x0116 => "WM_INITMENU",
        0x0117 => "WM_INITMENUPOPUP",
        0x011F => "WM_MENUSELECT",
        0x0120 => "WM_MENUCHAR",
        0x0121 => "WM_ENTERIDLE",
        0x0127 => "WM_CHANGEUISTATE",
        0x0128 => "WM_UPDATEUISTATE",
        0x0129 => "WM_QUERYUISTATE",
        0x0200 => "WM_MOUSEMOVE",
        0x0201 => "WM_LBUTTONDOWN",
        0x0202 => "WM_LBUTTONUP",
        0x0203 => "WM_LBUTTONDBLCLK",
        0x0204 => "WM_RBUTTONDOWN",
        0x0205 => "WM_RBUTTONUP",
        0x0206 => "WM_RBUTTONDBLCLK",
        0x0207 => "WM_MBUTTONDOWN",
        0x0208 => "WM_MBUTTONUP",
        0x0209 => "WM_MBUTTONDBLCLK",
        0x020A => "WM_MOUSEWHEEL",
        0x020E => "WM_MOUSEHWHEEL",
        0x0210 => "WM_PARENTNOTIFY",
        0x0211 => "WM_ENTERMENULOOP",
        0x0212 => "WM_EXITMENULOOP",
        0x0214 => "WM_SIZING",
        0x0215 => "WM_CAPTURECHANGED",
        0x0216 => "WM_MOVING",
        0x0231 => "WM_ENTERSIZEMOVE",
        0x0232 => "WM_EXITSIZEMOVE",
        0x0281 => "WM_IME_SETCONTEXT",
        0x0282 => "WM_IME_NOTIFY",
        0x02A1 => "WM_MOUSEHOVER",
        0x02A2 => "WM_NCMOUSELEAVE",
        0x02A3 => "WM_MOUSELEAVE",
        0x02E0 => "WM_DPICHANGED",
        0x0300 => "WM_CUT",
        0x0301 => "WM_COPY",
        0x0302 => "WM_PASTE",
        0x0317 => "WM_PRINT",
        0x0318 => "WM_PRINTCLIENT",
        0x031A => "WM_THEMECHANGED",
        0x031E => "WM_DWMCOMPOSITIONCHANGED",
        0x031F => "WM_DWMNCRENDERINGCHANGED",
        0x0320 => "WM_DWMCOLORIZATIONCOLORCHANGED",
        _ => return None,
    })
}

/// "WM_NAME" or "msg 0x....".
struct MsgName(u32);

impl core::fmt::Display for MsgName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match msg_name(self.0) {
            Some(n) => f.write_str(n),
            None => write!(f, "msg_{:#06x}", self.0),
        }
    }
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

const WM_MOUSEHOVER: u32 = 0x02A1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_RBUTTONDBLCLK: u32 = 0x0206;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_NCLBUTTONUP: u32 = 0x00A2;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
const WM_PRINT: u32 = 0x0317;
const WM_GETOBJECT: u32 = 0x003D;
const WM_IME_SETCONTEXT: u32 = 0x0281;
const WM_IME_NOTIFY: u32 = 0x0282;
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

const WM_INITDIALOG: u32 = 0x0110;
const WM_INITMENU: u32 = 0x0116;
const WM_INITMENUPOPUP: u32 = 0x0117;
const WM_MENUSELECT: u32 = 0x011F;
const WM_MENUCHAR: u32 = 0x0120;
const WM_ENTERIDLE: u32 = 0x0121;
const WM_ENTERMENULOOP: u32 = 0x0211;
const WM_EXITMENULOOP: u32 = 0x0212;
const WM_NEXTDLGCTL: u32 = 0x0028;
const WM_DRAWITEM: u32 = 0x002B;
const WM_NOTIFY: u32 = 0x004E;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
/// winuser.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SCROLLINFO {
    cb_size: u32,
    mask: u32,
    min: i32,
    max: i32,
    page: u32,
    pos: i32,
    track_pos: i32,
}

const SB_HORZ: i32 = 0;
const SB_VERT: i32 = 1;
const SB_CTL: i32 = 2;
const SB_BOTH: i32 = 3;
const SBS_HORZ: u32 = 0;
const SBS_VERT: u32 = 1;
const WM_HSCROLL_: u32 = 0x0114;
const WM_VSCROLL_: u32 = 0x0115;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn AdjustWindowRectEx(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetScrollInfo(hwnd: HWND, bar: i32, info: *const SCROLLINFO, redraw: BOOL) -> i32;
    fn GetScrollInfo(hwnd: HWND, bar: i32, info: *mut SCROLLINFO) -> BOOL;
    fn SetScrollRange(hwnd: HWND, bar: i32, min: i32, max: i32, redraw: BOOL) -> BOOL;
    fn GetScrollRange(hwnd: HWND, bar: i32, min: *mut i32, max: *mut i32) -> BOOL;
    fn SetScrollPos(hwnd: HWND, bar: i32, pos: i32, redraw: BOOL) -> i32;
    fn GetScrollPos(hwnd: HWND, bar: i32) -> i32;
}

// ---- scroll bar helpers of these tests ----

const WS_HSCROLL_: u32 = 0x0010_0000;
const WS_VSCROLL_: u32 = 0x0020_0000;

unsafe extern "system" fn sc_wp(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_SIZE {
        logln!("WM_SIZE type {w}");
    } else if m == WM_NCCALCSIZE || m == WM_WINDOWPOSCHANGING || m == WM_WINDOWPOSCHANGED || m == WM_PAINT || m == WM_ERASEBKGND || m == WM_STYLECHANGING || m == WM_STYLECHANGED || m == WM_HSCROLL_ || m == WM_VSCROLL_ {
        logln!("{}", MsgName(m));
    }
    let _ = l;
    // SAFETY: default processing.
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// A visible top-level window with the given scroll bar styles (client area 300 x 200).
fn sc_window(bars: u32) -> HWND {
    static DONE: Global<bool> = Global::new(false);
    if !*DONE.get() {
        register(wz!("ScrollTestWnd"), Some(sc_wp), 0, 0, core::ptr::null_mut());
        *DONE.get() = true;
    }
    let style = WS_OVERLAPPED | WS_CAPTION | WS_VISIBLE | bars;
    let mut r = RECT { left: 0, top: 0, right: 300, bottom: 200 };
    // SAFETY: r is writable.
    unsafe { AdjustWindowRectEx(&mut r, style, 0, 0) };
    let h = create(0, wz!("ScrollTestWnd"), wz!("scroll"), style, 20, 20, r.right - r.left, r.bottom - r.top, core::ptr::null_mut(), 0);
    pump();
    log_clear();
    h
}

/// A scroll bar control of the given style in a parent window.
fn sc_control(style: u32, parent: HWND) -> HWND {
    create(0, wz!("SCROLLBAR"), wz!(""), WS_CHILD | WS_VISIBLE | style, 5, 5, if style & SBS_VERT != 0 { 17 } else { 200 }, if style & SBS_VERT != 0 { 150 } else { 17 }, parent, 77)
}

fn si(mask: u32, min: i32, max: i32, page: u32, pos: i32) -> SCROLLINFO {
    SCROLLINFO { cb_size: core::mem::size_of::<SCROLLINFO>() as u32, mask, min, max, page, pos, track_pos: 0x5A5A }
}

fn bar_name(b: i32) -> &'static str {
    match b {
        0 => "SB_HORZ",
        1 => "SB_VERT",
        2 => "SB_CTL",
        3 => "SB_BOTH",
        _ => "bar?",
    }
}

/// GetScrollInfo with the given mask: "label: ret r err e min max page pos track".
fn gsi(t: &mut Test, label: &str, h: HWND, bar: i32, mask: u32) -> SCROLLINFO {
    let mut s = SCROLLINFO { cb_size: core::mem::size_of::<SCROLLINFO>() as u32, mask, min: 0x1111, max: 0x2222, page: 0x3333, pos: 0x4444, track_pos: 0x5555 };
    set_error(0xDEAD);
    // SAFETY: s is a SCROLLINFO.
    let r = unsafe { GetScrollInfo(h, bar, &mut s) };
    let e = last_error();
    say!(t, "{label}: GetScrollInfo({}, mask {:#x}) = {r}{} min {} max {} page {} pos {} track {}", bar_name(bar), mask, if r == 0 { if e == 0xDEAD { " err -1" } else { " err" } } else { "" }, s.min, s.max, s.page, s.pos, s.track_pos);
    if r == 0 && e != 0xDEAD {
        say!(t, "  last error {e}");
    }
    s
}

/// SetScrollInfo: "label: SetScrollInfo(bar, mask) = result".
fn ssi(t: &mut Test, label: &str, h: HWND, bar: i32, s: &SCROLLINFO, redraw: i32) -> i32 {
    set_error(0xDEAD);
    // SAFETY: s is a SCROLLINFO.
    let r = unsafe { SetScrollInfo(h, bar, s, redraw) };
    let e = last_error();
    say!(t, "{label}: SetScrollInfo({}, mask {:#x}, min {}, max {}, page {}, pos {}) = {r}", bar_name(bar), s.mask, s.min, s.max, s.page, s.pos);
    // the last error is only specified for arguments that are not valid
    if e != 0xDEAD && (!(0..=3).contains(&bar) || h.is_null() || h as usize == 0x1234) {
        say!(t, "  last error {e}");
    }
    r
}

/// Style bits and the client size of a window relative to its starting client size 300 x 200.
fn win_state(t: &mut Test, label: &str, h: HWND) {
    // SAFETY: plain calls.
    let st = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
    let c = client_of(h);
    // SAFETY: plain calls.
    let (vw, hh) = unsafe { (GetSystemMetrics(2), GetSystemMetrics(3)) };
    say!(
        t,
        "{label}: WS_HSCROLL {} WS_VSCROLL {} client width {} client height {} (SM_CXVSCROLL and SM_CYHSCROLL are taken off only for a bar that is shown)",
        u32::from(st & WS_HSCROLL_ != 0),
        u32::from(st & WS_VSCROLL_ != 0),
        if c.right == 300 { "full" } else if c.right == 300 - vw { "less the vertical bar" } else { "other" },
        if c.bottom == 200 { "full" } else if c.bottom == 200 - hh { "less the horizontal bar" } else { "other" }
    );
}

/// Print and clear the messages the window received.
fn sc_log(t: &mut Test, label: &str) {
    pump();
    say!(t, "{label}: messages:");
    flush_log(t, "    ");
}

fn errtxt(r: i64, ok_when_nonzero: bool) -> i64 {
    let e = last_error();
    // 5 (ERROR_ACCESS_DENIED) is what some calls on scroll bar controls leave behind; it is not specified and printed like "unchanged"
    if ok_when_nonzero && r != 0 {
        -2
    } else if e == 0xDEAD || e == 5 {
        -1
    } else {
        e as i64
    }
}

/// SetScrollRange: "label: SetScrollRange(bar, min, max) = r err e".
fn ssr(t: &mut Test, label: &str, h: HWND, bar: i32, min: i32, max: i32, redraw: i32) -> i32 {
    set_error(0xDEAD);
    // SAFETY: plain call.
    let r = unsafe { SetScrollRange(h, bar, min, max, redraw) };
    say!(t, "{label}: SetScrollRange({}, {min}, {max}) = {r} err {}", bar_name(bar), errtxt(r as i64, true));
    r
}

fn gsr(t: &mut Test, label: &str, h: HWND, bar: i32) -> (i32, i32) {
    let (mut lo, mut hi) = (0x1111, 0x2222);
    set_error(0xDEAD);
    // SAFETY: lo and hi are writable.
    let r = unsafe { GetScrollRange(h, bar, &mut lo, &mut hi) };
    say!(t, "{label}: GetScrollRange({}) = {r} err {} min {lo} max {hi}", bar_name(bar), errtxt(r as i64, true));
    (lo, hi)
}

fn ssp(t: &mut Test, label: &str, h: HWND, bar: i32, pos: i32, redraw: i32) -> i32 {
    set_error(0xDEAD);
    // SAFETY: plain call.
    let r = unsafe { SetScrollPos(h, bar, pos, redraw) };
    say!(t, "{label}: SetScrollPos({}, {pos}) = {r} err {}", bar_name(bar), errtxt(r as i64, false));
    r
}

fn gsp(t: &mut Test, label: &str, h: HWND, bar: i32) -> i32 {
    set_error(0xDEAD);
    // SAFETY: plain call.
    let r = unsafe { GetScrollPos(h, bar) };
    say!(t, "{label}: GetScrollPos({}) = {r} err {}", bar_name(bar), errtxt(r as i64, false));
    r
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    let w = sc_window(0);
    gsr(t, "window without bars, vertical", w, SB_VERT);
    gsr(t, "horizontal", w, SB_HORZ);
    gsr(t, "SB_CTL", w, SB_CTL);
    gsr(t, "bar 4", w, 4);
    gsr(t, "bar -1", w, -1);
    gsr(t, "made-up window", 0x1234 as HWND, SB_VERT);
    for st in [SBS_HORZ, SBS_VERT] {
        let c = sc_control(st, w);
        gsr(t, "fresh control", c, SB_CTL);
        gsr(t, "control as SB_VERT", c, SB_VERT);
        gsr(t, "control as SB_HORZ", c, SB_HORZ);
        gsp(t, "position", c, SB_CTL);
    }
    // SAFETY: our window.
    unsafe { DestroyWindow(w) };
}
