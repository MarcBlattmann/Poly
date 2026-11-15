//! GetLastInputInfo the way Chrome's browser process uses it (S3; once in
//! a Chrome run, to learn how long the user has been idle) with its window
//! in the foreground: a call, a pause of 60 ms, one key (VK_F24, no
//! character, sent with SendInput while the test's window is in the
//! foreground) and a second call. Printed as relations only: the first
//! time is not in the future; the second is not older than the first; the
//! time moved (second differs from first: it does when the injected key
//! counts as input); the idle time (tick count minus dwTime) is below five
//! seconds after the key. The key messages the window gets (VK_F24 down and
//! up, no WM_CHAR) are recorded with the two thread hooks (see
//! cw_chrome_widget).
//!
//! The key is sent only while the test's window is in the foreground.
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
type COLORREF = u32;
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
struct CREATESTRUCTW {
    create_params: *mut c_void,
    instance: HINSTANCE,
    menu: HMENU,
    parent: HWND,
    cy: i32,
    cx: i32,
    y: i32,
    x: i32,
    style: i32,
    name: *const u16,
    class: *const u16,
    ex_style: u32,
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

/// winuser.h
#[repr(C)]
struct STYLESTRUCT {
    old: u32,
    new: u32,
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
const WM_USER: u32 = 0x0400;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_HSCROLL: u32 = 0x0010_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const CS_DBLCLKS: u32 = 0x0008;
const SW_HIDE: i32 = 0;
const SW_SHOWNORMAL: i32 = 1;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const HWND_BOTTOM: HWND = 1 as HWND;
const GWL_EXSTYLE: i32 = -20;
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
const SIZE_MINIMIZED: usize = 1;
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
struct TRACKMOUSEEVENT {
    cb_size: u32,
    flags: u32,
    track: HWND,
    hover_time: u32,
}

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const TME_LEAVE: u32 = 0x0000_0002;
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
const MAPVK_VK_TO_VSC: u32 = 0;
const VK_F24: i32 = 0x87;
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

/// Whether the foreground window belongs to this process.
fn fg_is_ours() -> bool {
    let mut pid = 0u32;
    // SAFETY: plain queries; pid is writable.
    unsafe {
        let fg = GetForegroundWindow();
        !fg.is_null() && GetWindowThreadProcessId(fg, &mut pid) != 0 && pid == GetCurrentProcessId()
    }
}

/// Make `h` (a visible top-level window of this thread) the foreground
/// window: SetForegroundWindow, and if that is not enough a real click in
/// its client area at (cx, cy), only when `h` is what is under that point.
/// Returns whether `h` is the foreground window afterwards. Empties the log.
fn make_foreground(h: HWND, cx: i32, cy: i32) -> bool {
    // SAFETY: plain calls on our window.
    unsafe {
        if GetForegroundWindow() != h {
            SetForegroundWindow(h);
            settle(50);
        }
        if GetForegroundWindow() != h {
            let mut p = POINT { x: cx, y: cy };
            ClientToScreen(h, &mut p);
            SetCursorPos(p.x, p.y);
            settle(50);
            if cursor_pos() == p && WindowFromPoint(p) == h {
                click_here();
                settle(100);
            }
        }
        log_clear();
        GetForegroundWindow() == h
    }
}

/// Send keyboard input only while a window of this process is in the
/// foreground (so nothing reaches another program); returns how many
/// events were inserted (0: refused).
fn send_keys(inputs: &[INPUT]) -> u32 {
    if !fg_is_ours() {
        return 0;
    }
    send(inputs)
}

/// Press and release a virtual key (scan code from MapVirtualKeyW), only
/// while this process is in the foreground.
fn tap(vk: i32) -> u32 {
    // SAFETY: plain query.
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    send_keys(&[key_input(vk as u16, scan, 0), key_input(vk as u16, scan, KEYEVENTF_KEYUP)])
}

/// Mouse button events at the cursor, only when a window of this process
/// is under it; returns how many events were inserted (0: refused).
fn buttons_here(flags: &[u32]) -> u32 {
    let p = cursor_pos();
    let mut pid = 0u32;
    // SAFETY: plain queries; pid is writable.
    let ours = unsafe {
        let w = WindowFromPoint(p);
        !w.is_null() && GetWindowThreadProcessId(w, &mut pid) != 0 && pid == GetCurrentProcessId()
    };
    if !ours {
        return 0;
    }
    let mut ins = [mouse_input(0, 0, 0, 0); 8];
    let n = flags.len().min(8);
    for i in 0..n {
        ins[i] = mouse_input(0, 0, 0, flags[i]);
    }
    send(&ins[..n])
}

/// A left click at the cursor (see `buttons_here`).
fn click_here() -> u32 {
    buttons_here(&[MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP])
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

type HHOOK = *mut c_void;
type HOOKPROC = Option<unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT>;
/// winuser.h
#[repr(C)]
struct CWPSTRUCT {
    lparam: LPARAM,
    wparam: WPARAM,
    message: u32,
    hwnd: HWND,
}

const WH_GETMESSAGE: i32 = 3;
const WH_CALLWNDPROC: i32 = 4;
const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
const WM_IME_REQUEST: u32 = 0x0288;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const WM_DWMNCRENDERINGCHANGED: u32 = 0x031F;
const WM_DISPLAYCHANGE: u32 = 0x007E;
const WM_CHANGEUISTATE: u32 = 0x0127;
const WM_UPDATEUISTATE: u32 = 0x0128;
const WM_QUERYUISTATE: u32 = 0x0129;
const WM_THEMECHANGED: u32 = 0x031A;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_UAHDESTROYWINDOW: u32 = 0x0090;
const WM_MOUSEHWHEEL: u32 = 0x020E;
const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
/// How a 64-bit parameter is printed when its value is machine-dependent:
/// small values as they are, anything that looks like a pointer or handle
/// as "big".
struct Cls(usize);

impl core::fmt::Display for Cls {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 < 0x1_0000 { write!(f, "{:#x}", self.0) } else { f.write_str("big") }
    }
}

/// The role of a window of this test by its class name (what the recording
/// of the real application calls them): "main" Chrome_WidgetWin_1, "rwh"
/// Chrome_RenderWidgetHostHWND, "hid" Chrome_WidgetWin_0, "ime" the default
/// IME window, "ctf" MSCTFIME UI, "msg" Chrome_MessageWindow, "d3d" the
/// Intermediate D3D Window, else the class name's first letters.
fn role_of(hwnd: HWND) -> &'static str {
    let mut buf = [0u16; 64];
    // SAFETY: buf is writable for 64 characters.
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), 64) } as usize;
    let mut s = Buf::<64>::new();
    for &c in &buf[..n.min(64)] {
        let _ = s.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
    match s.as_str() {
        "Chrome_WidgetWin_1" => "main",
        "Chrome_RenderWidgetHostHWND" => "rwh",
        "Chrome_WidgetWin_0" => "hid",
        "IME" => "ime",
        "MSCTFIME UI" => "ctf",
        "Chrome_MessageWindow" => "msg",
        "Intermediate D3D Window" => "d3d",
        "Base_PowerMessageWindow" => "pow",
        _ => "other",
    }
}

/// A window of this process as a script prints it: its role, or "null",
/// "other" (another process) or "invalid".
struct Rw(HWND);

impl core::fmt::Display for Rw {
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
        let r = role_of(self.0);
        if r == "other" {
            // SAFETY: a window of this process.
            let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
            if (0x41..=0x5A).contains(&tag) {
                return f.write_char(tag as u8 as char);
            }
        }
        f.write_str(r)
    }
}

/// Which messages the hooks write (set per test).
static HOOK_FILTER: Global<fn(u32) -> bool> = Global::new(|_| true);

/// Whether windows of this process with a class that is none of Chrome's
/// are logged too (their class names are not Chrome's, e.g. the windows
/// the text services create on their own are left out by default).
static HOOK_OTHER: Global<bool> = Global::new(false);

/// A line for a message seen by a hook: "S" sent (WH_CALLWNDPROC), "P"
/// posted (WH_GETMESSAGE), the window's role, the message and its
/// parameters reduced to classes that do not depend on the machine.
fn log_hooked(kind: char, hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) {
    if !(HOOK_FILTER.get())(msg) {
        return;
    }
    let who = Rw(hwnd);
    if !*HOOK_OTHER.get() && role_of(hwnd) == "other" {
        return;
    }
    if msg >= 0xC000 {
        return;
    }
    // notifications of the text services that arrive late and in no fixed order (IMN 0x11) and WM_IME_REQUEST
    if (msg == WM_IME_NOTIFY && w == 0x11) || msg == WM_IME_REQUEST {
        return;
    }
    match msg {
        WM_NCCREATE | WM_CREATE => {
            // SAFETY: lParam is a CREATESTRUCTW for these messages.
            let cs = unsafe { &*(l as *const CREATESTRUCTW) };
            logln!(
                "{kind} {who} {} pos={},{} size={},{} style={:#x} ex={:#x} parent={}",
                MsgName(msg),
                cs.x,
                cs.y,
                cs.cx,
                cs.cy,
                cs.style as u32,
                cs.ex_style,
                Rw(cs.parent)
            );
        }
        WM_GETMINMAXINFO => {
            // SAFETY: lParam is a MINMAXINFO.
            let m = unsafe { &*(l as *const MINMAXINFO) };
            logln!(
                "{kind} {who} WM_GETMINMAXINFO maxsize={},{} maxpos={},{} mintrack={},{} maxtrack={},{}",
                m.max_size.x,
                m.max_size.y,
                m.max_position.x,
                m.max_position.y,
                m.min_track_size.x,
                m.min_track_size.y,
                m.max_track_size.x,
                m.max_track_size.y
            );
        }
        WM_NCCALCSIZE => {
            if w != 0 {
                // SAFETY: lParam is an NCCALCSIZE_PARAMS when wParam is TRUE.
                let p = unsafe { &*(l as *const NCCALCSIZE_PARAMS) };
                logln!(
                    "{kind} {who} WM_NCCALCSIZE valid=1 new={} old={} oldclient={}",
                    R(p.rgrc[0]),
                    R(p.rgrc[1]),
                    R(p.rgrc[2])
                );
            } else {
                // SAFETY: lParam is a RECT when wParam is FALSE.
                let r = unsafe { &*(l as *const RECT) };
                logln!("{kind} {who} WM_NCCALCSIZE valid=0 rect={}", R(*r));
            }
        }
        WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
            // SAFETY: lParam is a WINDOWPOS.
            let p = unsafe { &*(l as *const WINDOWPOS) };
            let after = if p.insert_after.is_null() {
                "top"
            } else if p.insert_after == HWND_BOTTOM {
                "bottom"
            } else if p.insert_after as isize == -1 {
                "topmost"
            } else if p.insert_after as isize == -2 {
                "notopmost"
            } else {
                "window"
            };
            logln!(
                "{kind} {who} {} pos={},{} size={},{} flags={:#x} after={}",
                MsgName(msg),
                p.x,
                p.y,
                p.cx,
                p.cy,
                p.flags,
                after
            );
        }
        WM_SIZE => logln!("{kind} {who} WM_SIZE type={} client={}x{}", w, lo(l), hi(l)),
        WM_MOVE => logln!("{kind} {who} WM_MOVE client_origin={},{}", lo(l), hi(l)),
        WM_SHOWWINDOW => logln!("{kind} {who} WM_SHOWWINDOW show={} status={}", w, l),
        WM_STYLECHANGING | WM_STYLECHANGED => {
            // SAFETY: lParam is a STYLESTRUCT.
            let s = unsafe { &*(l as *const STYLESTRUCT) };
            logln!(
                "{kind} {who} {} which={} old={:#x} new={:#x}",
                MsgName(msg),
                if w as i32 == GWL_EXSTYLE { "ex" } else { "style" },
                s.old,
                s.new
            );
        }
        WM_ACTIVATE => logln!("{kind} {who} WM_ACTIVATE state={} minimized={} other={}", w & 0xFFFF, w >> 16, Rw(l as HWND)),
        WM_ACTIVATEAPP => logln!("{kind} {who} WM_ACTIVATEAPP active={}", w),
        WM_NCACTIVATE => logln!("{kind} {who} WM_NCACTIVATE active={} l={}", w, if l == 0 { "0" } else if l == -1 { "-1" } else { "other" }),
        WM_SETFOCUS | WM_KILLFOCUS => logln!("{kind} {who} {} other={}", MsgName(msg), Rw(w as HWND)),
        WM_MOUSEACTIVATE => logln!("{kind} {who} WM_MOUSEACTIVATE top={} hit={} msg={}", Rw(w as HWND), lo(l), MsgName(hi(l) as u32 & 0xFFFF)),
        WM_SETCURSOR => logln!("{kind} {who} WM_SETCURSOR window={} hit={} msg={}", Rw(w as HWND), lo(l) as i16, MsgName(hi(l) as u32 & 0xFFFF)),
        WM_NCHITTEST => logln!("{kind} {who} WM_NCHITTEST"),
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP => {
            logln!("{kind} {who} {} keys={:#x}", MsgName(msg), w)
        }
        WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP => logln!("{kind} {who} {} hit={}", MsgName(msg), w as isize),
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => logln!("{kind} {who} {}", MsgName(msg)),
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            logln!("{kind} {who} {} vk={:#04x} repeat={} flags={:#x}", MsgName(msg), w, l & 0xFFFF, (l >> 16) & 0xFF00)
        }
        WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_UNICHAR => logln!("{kind} {who} {} ch={:#06x}", MsgName(msg), w),
        WM_CAPTURECHANGED => logln!("{kind} {who} WM_CAPTURECHANGED new={}", Rw(l as HWND)),
        WM_PARENTNOTIFY => logln!("{kind} {who} WM_PARENTNOTIFY event={:#x}", w & 0xFFFF),
        WM_NCPAINT => logln!("{kind} {who} WM_NCPAINT rgn={}", if w == 1 { "1" } else { "region" }),
        WM_ERASEBKGND => logln!("{kind} {who} WM_ERASEBKGND"),
        WM_PAINT => logln!("{kind} {who} WM_PAINT"),
        WM_ENABLE => logln!("{kind} {who} WM_ENABLE enabled={}", w),
        WM_SETTEXT => logln!("{kind} {who} WM_SETTEXT"),
        WM_GETTEXT | WM_GETTEXTLENGTH => logln!("{kind} {who} {}", MsgName(msg)),
        WM_GETICON => logln!("{kind} {who} WM_GETICON type={} dpi={}", w, l),
        WM_IME_SETCONTEXT => logln!("{kind} {who} WM_IME_SETCONTEXT active={} flags={:#x}", w, l as u32),
        WM_IME_NOTIFY => logln!("{kind} {who} WM_IME_NOTIFY cmd={:#x}", w),
        WM_TIMER => logln!("{kind} {who} WM_TIMER"),
        WM_USER..=0x7FFF => logln!("{kind} {who} WM_USER+{} w={} l={}", msg - WM_USER, Cls(w), Cls(l as usize)),
        _ => match msg_name(msg) {
            Some(n) => logln!("{kind} {who} {n} w={} l={}", Cls(w), Cls(l as usize)),
            None => logln!("{kind} {who} msg_{:#06x} w={} l={}", msg, Cls(w), Cls(l as usize)),
        },
    }
}

/// WH_CALLWNDPROC: every message sent to a window of this thread, before
/// the window procedure sees it.
unsafe extern "system" fn hook_sent(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: lParam is a CWPSTRUCT for this hook.
        let c = unsafe { &*(l as *const CWPSTRUCT) };
        log_hooked('S', c.hwnd, c.message, c.wparam, c.lparam);
    }
    // SAFETY: passing the call on.
    unsafe { CallNextHookEx(core::ptr::null_mut(), code, w, l) }
}

/// WH_GETMESSAGE: every message the thread takes from its queue.
unsafe extern "system" fn hook_posted(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && w == PM_REMOVE as usize {
        // SAFETY: lParam is the MSG just taken.
        let m = unsafe { &*(l as *const MSG) };
        log_hooked('P', m.hwnd, m.message, m.wparam, m.lparam);
    }
    // SAFETY: passing the call on.
    unsafe { CallNextHookEx(core::ptr::null_mut(), code, w, l) }
}

/// The two hooks of the Chrome recording on this thread.
struct Hooks(HHOOK, HHOOK);

impl Hooks {
    fn on() -> Hooks {
        // SAFETY: thread hooks of this thread with procedures of this module.
        unsafe {
            let t = GetCurrentThreadId();
            Hooks(
                SetWindowsHookExW(WH_CALLWNDPROC, Some(hook_sent), core::ptr::null_mut(), t),
                SetWindowsHookExW(WH_GETMESSAGE, Some(hook_posted), core::ptr::null_mut(), t),
            )
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: the hooks installed by `on`.
        unsafe {
            UnhookWindowsHookEx(self.0);
            UnhookWindowsHookEx(self.1);
        }
    }
}

/// Chrome's window procedure shape for these tests: everything goes to the
/// default handling (the hooks see the messages first).
unsafe extern "system" fn proc_plain(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Class names (NUL-terminated UTF-16) of Chrome's windows.
fn cls_main() -> &'static [u16] {
    wz!("Chrome_WidgetWin_1")
}

fn cls_rwh() -> &'static [u16] {
    wz!("Chrome_RenderWidgetHostHWND")
}

fn cls_hid() -> &'static [u16] {
    wz!("Chrome_WidgetWin_0")
}

/// Chrome's class and style bits as recorded (specs/windows/apps/chrome,
/// s3-windows.md): the top-level window, its render-widget-host child,
/// the hidden holder window.
const CHROME_MAIN_STYLE: u32 = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_CLIPSIBLINGS;

const CHROME_MAIN_EX: u32 = WS_EX_NOREDIRECTIONBITMAP;
const CHROME_RWH_STYLE: u32 = WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | WS_HSCROLL | WS_VSCROLL;
const CHROME_RWH_EX: u32 = WS_EX_TRANSPARENT;
const CHROME_HID_STYLE: u32 = WS_POPUP | WS_DISABLED | WS_CLIPSIBLINGS;
const CHROME_HID_EX: u32 = WS_EX_TOOLWINDOW;
/// The top-level window's procedure, as far as the recording shows it:
/// the frame is Chrome's own (the client area starts at the window's top
/// edge and keeps the side and bottom borders), the render widget host
/// child follows the client area, a mouse move arms a WM_MOUSELEAVE, the
/// left button takes and gives back the mouse capture, painting is
/// BeginPaint/EndPaint, the background is not erased, and a minimize moves
/// the child under the hidden holder window and a restore moves it back.
/// Everything else is the default handling.
static CH_RWH: Global<HWND> = Global::new(core::ptr::null_mut());

static CH_HID: Global<HWND> = Global::new(core::ptr::null_mut());
static CH_TRACKING: Global<bool> = Global::new(false);
static CH_PAINTS: Global<u32> = Global::new(0);
static CH_ERASE_RESULT: Global<LRESULT> = Global::new(1);
static CH_CLOSE_STEPS: Global<bool> = Global::new(false);
/// The top edge of the child inside the client area (the tab strip and
/// toolbar of the real window).
const CH_TOP: i32 = 100;

unsafe extern "system" fn proc_main(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: calls on the window being served and its children.
    unsafe {
        match msg {
            WM_NCCALCSIZE if w != 0 => {
                let p = &mut *(l as *mut NCCALCSIZE_PARAMS);
                let top = p.rgrc[0].top;
                let r = DefWindowProcW(hwnd, msg, w, l);
                p.rgrc[0].top = top;
                return r;
            }
            WM_NCCALCSIZE => {
                let rc = &mut *(l as *mut RECT);
                let top = rc.top;
                let r = DefWindowProcW(hwnd, msg, w, l);
                rc.top = top;
                return r;
            }
            WM_SIZE => {
                let rwh = *CH_RWH.get();
                if !rwh.is_null() {
                    if w == SIZE_MINIMIZED {
                        SetParent(rwh, *CH_HID.get());
                    } else {
                        if GetParent(rwh) != hwnd {
                            SetParent(rwh, hwnd);
                        }
                        let mut c = RECT::default();
                        GetClientRect(hwnd, &mut c);
                        SetWindowPos(
                            rwh,
                            core::ptr::null_mut(),
                            0,
                            CH_TOP,
                            c.right,
                            (c.bottom - CH_TOP).max(0),
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                return DefWindowProcW(hwnd, msg, w, l);
            }
            WM_MOUSEMOVE => {
                if !*CH_TRACKING.get() {
                    let mut e = TRACKMOUSEEVENT {
                        cb_size: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        flags: TME_LEAVE,
                        track: hwnd,
                        hover_time: 0,
                    };
                    TrackMouseEvent(&mut e);
                    *CH_TRACKING.get() = true;
                }
                return DefWindowProcW(hwnd, msg, w, l);
            }
            WM_MOUSELEAVE => {
                *CH_TRACKING.get() = false;
                return 0;
            }
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                return 0;
            }
            WM_LBUTTONUP => {
                ReleaseCapture();
                return 0;
            }
            WM_ERASEBKGND => return *CH_ERASE_RESULT.get(),
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                BeginPaint(hwnd, &mut ps);
                EndPaint(hwnd, &ps);
                *CH_PAINTS.get() += 1;
                return 0;
            }
            WM_CLOSE if *CH_CLOSE_STEPS.get() => {
                ShowWindow(hwnd, SW_HIDE);
                let rwh = *CH_RWH.get();
                if !rwh.is_null() {
                    SetParent(rwh, *CH_HID.get());
                    DestroyWindow(rwh);
                    *CH_RWH.get() = core::ptr::null_mut();
                }
                DestroyWindow(hwnd);
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

/// The child's procedure: its client area is its whole rectangle (no
/// scroll bar space although it has both scroll bar styles), the rest is
/// the default handling.
unsafe extern "system" fn proc_rwh(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCALCSIZE {
        return 0;
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Chrome's three windows of the scenarios.
struct Chrome {
    main: HWND,
    rwh: HWND,
    hid: HWND,
}

/// Register Chrome's classes and create the hidden holder window, the
/// top-level window (hidden, at x,y with size w x h) and its child
/// (hidden, placed under the top part of the client area), in the order of
/// the recording: holder, top-level, child.
fn chrome_build(x: i32, y: i32, w: i32, h: i32) -> Chrome {
    // SAFETY: plain calls.
    let (icon, brush) = unsafe { (LoadIconW(core::ptr::null_mut(), 32512 as *const u16), CreateSolidBrush(0x00FF_FFFF)) };
    let mk = |name: &'static [u16], proc_: WNDPROC, style: u32, bg: HBRUSH, ic: *mut c_void| {
        let wc = WNDCLASSEXW {
            cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style,
            wnd_proc: proc_,
            cls_extra: 0,
            wnd_extra: 0,
            instance: instance(),
            icon: ic,
            cursor: core::ptr::null_mut(),
            background: bg,
            menu_name: core::ptr::null(),
            class_name: name.as_ptr(),
            icon_sm: ic,
        };
        // SAFETY: a filled WNDCLASSEXW with a NUL-terminated name.
        unsafe { RegisterClassExW(&wc) }
    };
    mk(cls_hid(), Some(proc_plain), CS_DBLCLKS, core::ptr::null_mut(), core::ptr::null_mut());
    mk(cls_main(), Some(proc_main), CS_DBLCLKS, brush, icon);
    mk(cls_rwh(), Some(proc_rwh), CS_DBLCLKS, core::ptr::null_mut(), core::ptr::null_mut());
    let hid = create(CHROME_HID_EX, cls_hid(), &[], CHROME_HID_STYLE, 0, 0, 0, 0, core::ptr::null_mut(), 0);
    *CH_HID.get() = hid;
    let main = create(CHROME_MAIN_EX, cls_main(), &[], CHROME_MAIN_STYLE, x, y, w, h, core::ptr::null_mut(), 0);
    let rwh = create(CHROME_RWH_EX, cls_rwh(), wz!("Chrome Legacy Window"), CHROME_RWH_STYLE, 0, 0, 0, 0, main, 656);
    *CH_RWH.get() = rwh;
    let mut c = RECT::default();
    // SAFETY: our windows.
    unsafe {
        GetClientRect(main, &mut c);
        SetWindowPos(rwh, core::ptr::null_mut(), 0, CH_TOP, c.right, c.bottom - CH_TOP, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    Chrome { main, rwh, hid }
}

/// The messages every scenario of the Chrome recording leaves out: the
/// shell's icon and timer traffic and undocumented messages that come from
/// the shell and the desktop (0x60, 0x90, 0x96, 0xAE, 0x272, 0x287).
fn chrome_keep(msg: u32) -> bool {
    !matches!(msg, WM_GETICON | WM_TIMER | 0x0060 | 0x0090 | 0x0096 | 0x00AE | 0x0272 | 0x0287)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
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
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, w: i32, h: i32, flags: u32) -> BOOL;
    fn ClientToScreen(hwnd: HWND, pt: *mut POINT) -> BOOL;
    fn GetParent(hwnd: HWND) -> HWND;
    fn SetParent(hwnd: HWND, parent: HWND) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn GetForegroundWindow() -> HWND;
    fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn SendInput(n: u32, inputs: *const INPUT, size: i32) -> u32;
    fn TrackMouseEvent(tme: *mut TRACKMOUSEEVENT) -> BOOL;
    fn WindowFromPoint(pt: POINT) -> HWND;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn GetLastInputInfo(info: *mut [u32; 2]) -> BOOL;
    fn SetWindowsHookExW(id: i32, proc_: HOOKPROC, module: HINSTANCE, thread: u32) -> HHOOK;
    fn UnhookWindowsHookEx(hook: HHOOK) -> BOOL;
    fn CallNextHookEx(hook: HHOOK, code: i32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn LoadIconW(instance: HINSTANCE, name: *const u16) -> *mut c_void;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
}

fn li_keep(msg: u32) -> bool {
    chrome_keep(msg) && matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_CHAR | WM_SYSKEYDOWN | WM_SYSKEYUP)
}

fn last_input() -> (i32, u32) {
    let mut li = [8u32, 0];
    // SAFETY: li is a LASTINPUTINFO with its size set.
    let r = unsafe { GetLastInputInfo(&mut li) };
    (r, li[1])
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    *HOOK_FILTER.get() = li_keep;
    let c = chrome_build(40, 40, 640, 400);
    // SAFETY: our windows.
    unsafe {
        SetWindowPos(c.rwh, core::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        ShowWindow(c.main, SW_SHOWNORMAL);
        SetFocus(c.main);
    }
    settle(200);
    let fg = make_foreground(c.main, 300, 50);
    say!(t, "main is the foreground window: {}", fg as i32);
    if fg {
        let hooks = Hooks::on();
        log_clear();
        let (r1, first) = last_input();
        // SAFETY: plain call.
        let tick = unsafe { GetTickCount() };
        say!(t, "first call: {r1}, the time is not in the future: {}", ((tick.wrapping_sub(first) as i32) >= 0) as i32);
        settle(60);
        let n = tap(VK_F24);
        say!(t, "sent {n} events for VK_F24");
        settle(100);
        let (r2, second) = last_input();
        // SAFETY: plain call.
        let tick = unsafe { GetTickCount() };
        say!(t, "second call: {r2}, not older than the first: {}", ((second.wrapping_sub(first) as i32) >= 0) as i32);
        say!(t, "the time moved: {}", (second != first) as i32);
        say!(t, "idle time after the key is below five seconds: {}", ((tick.wrapping_sub(second) as i32) < 5000 && (tick.wrapping_sub(second) as i32) >= 0) as i32);
        flush_log(t, "  ");
        drop(hooks);
    }
    // SAFETY: our windows.
    unsafe {
        DestroyWindow(c.main);
        DestroyWindow(c.hid);
    }
    settle(100);
}
