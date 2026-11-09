//! GetLayeredWindowAttributes on a layered window never given attributes
//! (FALSE, the last error is not set), after SetLayeredWindowAttributes
//! with each flag combination, with NULL for any of the three out
//! pointers (all three NULL still succeeds), on a window without
//! WS_EX_LAYERED (87) and on a NULL or made-up handle (1400).
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
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const GWLP_USERDATA: i32 = -21;
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

/// One log line for a message, with its parameters decoded.
fn log_msg(tag: &str, msg: u32, w: WPARAM, l: LPARAM) {
    match msg {
        WM_NCCREATE | WM_CREATE => {
            // SAFETY: lParam is a CREATESTRUCTW for these messages.
            let cs = unsafe { &*(l as *const CREATESTRUCTW) };
            logln!(
                "{tag} {} pos={},{} size={},{} style={:#x} ex={:#x}",
                MsgName(msg),
                cs.x,
                cs.y,
                cs.cx,
                cs.cy,
                cs.style as u32,
                cs.ex_style
            );
        }
        WM_GETMINMAXINFO => {
            // SAFETY: lParam is a MINMAXINFO.
            let m = unsafe { &*(l as *const MINMAXINFO) };
            logln!(
                "{tag} WM_GETMINMAXINFO maxsize={},{} maxpos={},{} mintrack={},{} maxtrack={},{}",
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
                // SAFETY: lParam is NCCALCSIZE_PARAMS when wParam is TRUE.
                let p = unsafe { &*(l as *const NCCALCSIZE_PARAMS) };
                let r = p.rgrc[0];
                // SAFETY: lppos is a WINDOWPOS when wParam is TRUE.
                let wp = unsafe { &*p.pos };
                logln!(
                    "{tag} WM_NCCALCSIZE 1 new={},{},{},{} pos={},{},{},{} flags={:#x}",
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    wp.x,
                    wp.y,
                    wp.cx,
                    wp.cy,
                    wp.flags
                );
            } else {
                // SAFETY: lParam is a RECT when wParam is FALSE.
                let r = unsafe { &*(l as *const RECT) };
                logln!("{tag} WM_NCCALCSIZE 0 rect={},{},{},{}", r.left, r.top, r.right, r.bottom);
            }
        }
        WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
            // SAFETY: lParam is a WINDOWPOS.
            let wp = unsafe { &*(l as *const WINDOWPOS) };
            logln!("{tag} {} pos={},{} size={},{} flags={:#x}", MsgName(msg), wp.x, wp.y, wp.cx, wp.cy, wp.flags);
        }
        WM_STYLECHANGING | WM_STYLECHANGED => {
            // SAFETY: lParam is a STYLESTRUCT.
            let ss = unsafe { &*(l as *const STYLESTRUCT) };
            logln!("{tag} {} which={} old={:#x} new={:#x}", MsgName(msg), w as isize, ss.old, ss.new);
        }
        WM_SIZE | WM_MOVE => {
            logln!("{tag} {} w={} l={},{}", MsgName(msg), w, lo(l), hi(l));
        }
        WM_PARENTNOTIFY => {
            logln!(
                "{tag} WM_PARENTNOTIFY event={} id={} l={}",
                MsgName((w & 0xFFFF) as u32),
                w >> 16,
                if l == 0 { 0 } else { 1 }
            );
        }
        WM_SETTEXT => {
            let mut b = Buf::<128>::new();
            if l != 0 {
                let mut p = l as *const u16;
                // SAFETY: lParam is a NUL-terminated string for WM_SETTEXT.
                unsafe {
                    while *p != 0 && b.n < 120 {
                        let c = *p;
                        let _ = b.write_char(if c < 0x80 { c as u8 as char } else { '?' });
                        p = p.add(1);
                    }
                }
            }
            logln!("{tag} WM_SETTEXT \"{}\"{}", b.as_str(), if l == 0 { " (null)" } else { "" });
        }
        WM_ERASEBKGND | WM_PAINT | WM_NCPAINT => {
            logln!("{tag} {}{}", MsgName(msg), if msg == WM_ERASEBKGND && w == 0 { " hdc=0" } else { "" });
        }
        _ => {
            logln!("{tag} {} w={:#x} l={:#x}", MsgName(msg), w, l);
        }
    }
}

/// The tag the next window created by `create_tagged` gets.
static NEXT_TAG: Global<u8> = Global::new(b'?');

/// A window's one-letter tag (kept in GWLP_USERDATA from WM_NCCREATE on;
/// before that, the tag of the window being created).
fn tag_of(hwnd: HWND) -> u8 {
    // SAFETY: a plain query of a window of this thread.
    let v = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if v == 0 { *NEXT_TAG.get() } else { v as u8 }
}

/// `create` for a window whose procedure logs under `tag`.
fn create_tagged(
    tag: u8,
    ex: u32,
    class: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    *NEXT_TAG.get() = tag;
    let hwnd = create(ex, class, &[], style, x, y, w, h, parent, menu);
    *NEXT_TAG.get() = b'?';
    hwnd
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

/// One log line for a message of the slice B set, with handles printed as
/// `W` and the rest decoded; other messages as `log_msg` does.
fn log_b(tag: &str, msg: u32, w: WPARAM, l: LPARAM) {
    match msg {
        WM_ACTIVATE => logln!("{tag} WM_ACTIVATE state={} minimized={} other={}", w & 0xFFFF, w >> 16, W(l as HWND)),
        WM_ACTIVATEAPP => logln!("{tag} WM_ACTIVATEAPP active={}", w),
        WM_NCACTIVATE => {
            logln!(
                "{tag} WM_NCACTIVATE active={} l={}",
                w,
                if l == 0 {
                    "0"
                } else if l == -1 {
                    "-1"
                } else {
                    "handle"
                }
            )
        }
        WM_SETFOCUS | WM_KILLFOCUS => logln!("{tag} {} other={}", MsgName(msg), W(w as HWND)),
        WM_MOUSEACTIVATE => {
            logln!("{tag} WM_MOUSEACTIVATE top={} hit={} msg={}", W(w as HWND), lo(l), MsgName(hi(l) as u32 & 0xFFFF))
        }
        WM_SETCURSOR => {
            logln!("{tag} WM_SETCURSOR window={} hit={} msg={}", W(w as HWND), lo(l), MsgName(hi(l) as u32 & 0xFFFF))
        }
        WM_NCHITTEST => logln!("{tag} WM_NCHITTEST screen={},{}", lo(l), hi(l)),
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
        | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MOUSEHOVER => {
            logln!("{tag} {} keys={:#x} client={},{}", MsgName(msg), w, lo(l), hi(l))
        }
        WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP | WM_NCLBUTTONDBLCLK => {
            logln!("{tag} {} hit={} screen={},{}", MsgName(msg), w as isize, lo(l), hi(l))
        }
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => logln!("{tag} {}", MsgName(msg)),
        WM_MOUSEWHEEL => {
            logln!(
                "{tag} WM_MOUSEWHEEL keys={:#x} delta={} screen={},{}",
                w & 0xFFFF,
                (w >> 16) as u16 as i16,
                lo(l),
                hi(l)
            )
        }
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            logln!("{tag} {} vk={:#04x} l={:#010x}", MsgName(msg), w, l as u32)
        }
        WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => {
            logln!("{tag} {} ch={:#06x} l={:#010x}", MsgName(msg), w, l as u32)
        }
        WM_CAPTURECHANGED => logln!("{tag} WM_CAPTURECHANGED new={}", W(l as HWND)),
        WM_NCPAINT => logln!("{tag} WM_NCPAINT rgn={}", if w == 1 { "1" } else { "region" }),
        WM_ERASEBKGND => logln!("{tag} WM_ERASEBKGND{}", if w == 0 { " hdc=0" } else { "" }),
        WM_PAINT => logln!("{tag} WM_PAINT"),
        WM_ENABLE => logln!("{tag} WM_ENABLE enabled={}", w),
        WM_CANCELMODE => logln!("{tag} WM_CANCELMODE"),
        WM_SHOWWINDOW => logln!("{tag} WM_SHOWWINDOW show={} status={}", w, l),
        _ => log_msg(tag, msg, w, l),
    }
}

/// Log a message of the slice B set (`documented_b`) under the window's tag.
fn log_tagged_b(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) {
    if msg == WM_NCCREATE {
        // SAFETY: the window is being created on this thread.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, *NEXT_TAG.get() as isize) };
    }
    if documented_b(msg) {
        let t = [tag_of(hwnd)];
        log_b(core::str::from_utf8(&t).unwrap_or("?"), msg, w, l);
    }
}

/// A window procedure that logs the slice B set and does the default.
unsafe extern "system" fn proc_b(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    log_tagged_b(hwnd, msg, w, l);
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// The documented painting, focus, activation and input messages the slice
/// B tests record, besides those of `documented`.
fn documented_b(msg: u32) -> bool {
    documented(msg)
        || matches!(
            msg,
            WM_PAINT
                | WM_ERASEBKGND
                | WM_NCPAINT
                | WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_ACTIVATE
                | WM_ACTIVATEAPP
                | WM_NCACTIVATE
                | WM_MOUSEACTIVATE
                | WM_SETCURSOR
                | WM_NCHITTEST
                | WM_MOUSEMOVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_LBUTTONDBLCLK
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_RBUTTONDBLCLK
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
                | WM_MOUSEWHEEL
                | WM_MOUSEHOVER
                | WM_MOUSELEAVE
                | WM_NCMOUSELEAVE
                | WM_CAPTURECHANGED
                | WM_NCMOUSEMOVE
                | WM_NCLBUTTONDOWN
                | WM_NCLBUTTONUP
                | WM_NCLBUTTONDBLCLK
                | WM_CANCELMODE
        )
}

const WM_CUT: u32 = 0x0300;
const WM_COPY: u32 = 0x0301;
const WM_PASTE: u32 = 0x0302;
const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
const WM_DWMCOMPOSITIONCHANGED: u32 = 0x031E;
const WM_DWMNCRENDERINGCHANGED: u32 = 0x031F;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
/// winuser.h: SetLayeredWindowAttributes / GetLayeredWindowAttributes flags.
const LWA_COLORKEY: u32 = 0x0000_0001;

const LWA_ALPHA: u32 = 0x0000_0002;
/// "(lo-hi)" style result text of a BOOL.
fn ly_b(v: BOOL) -> &'static str {
    if v != 0 { "TRUE" } else { "FALSE" }
}

/// Window class with a solid brush: registers `name` and returns the brush.
fn ly_register(name: &[u16], color: COLORREF) -> HBRUSH {
    // SAFETY: a plain brush.
    let b = unsafe { CreateSolidBrush(color) };
    register(name, Some(proc_b), 0, 0, b);
    b
}

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
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SetLayeredWindowAttributes(hwnd: HWND, key: COLORREF, alpha: u8, flags: u32) -> BOOL;
    fn GetLayeredWindowAttributes(hwnd: HWND, key: *mut COLORREF, alpha: *mut u8, flags: *mut u32) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cls = wz!("GlwaBasic");
    ly_register(cls, 0x0000FF);
    let plain = create(0, cls, &[], WS_POPUP, 100, 100, 80, 60, core::ptr::null_mut(), 0);
    let l = create(WS_EX_LAYERED, cls, &[], WS_POPUP, 100, 100, 80, 60, core::ptr::null_mut(), 0);
    // SAFETY: our windows; out pointers are valid or NULL.
    unsafe {
        let (mut k, mut a, mut f) = (0xAAAA_AAAAu32, 0xAAu8, 0xAAAA_AAAAu32);
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(l, &mut k, &mut a, &mut f);
        say!(t, "never set: {} error {:#x} key {:#x} alpha {} flags {:#x}", ly_b(r), last_error(), k, a, f);
        for (lbl, key, al, fl) in [
            ("alpha 10", 0x0000_0000u32, 10u8, LWA_ALPHA),
            ("key only", 0x00AA_BBCC, 20, LWA_COLORKEY),
            ("both", 0x0011_2233, 30, LWA_COLORKEY | LWA_ALPHA),
            ("alpha 255", 0x0011_2233, 255, LWA_ALPHA),
            ("alpha 0", 0x0011_2233, 0, LWA_ALPHA),
        ] {
            let r = SetLayeredWindowAttributes(l, key, al, fl);
            let (mut k, mut a, mut f) = (0xAAAA_AAAAu32, 0xAAu8, 0xAAAA_AAAAu32);
            set_error(0x1234);
            let g = GetLayeredWindowAttributes(l, &mut k, &mut a, &mut f);
            say!(
                t,
                "Set {lbl} -> {}; Get {} error {:#x} key {:#x} alpha {} flags {:#x}",
                ly_b(r),
                ly_b(g),
                last_error(),
                k,
                a,
                f
            );
        }
        // pointers
        let (mut k, mut a, mut f) = (0xAAAA_AAAAu32, 0xAAu8, 0xAAAA_AAAAu32);
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(l, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "all three NULL: {} error {:#x}", ly_b(r), last_error());
        let r = GetLayeredWindowAttributes(l, &mut k, core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "key only: {} key {:#x} (alpha, flags untouched {:#x} {:#x})", ly_b(r), k, a, f);
        let r = GetLayeredWindowAttributes(l, core::ptr::null_mut(), &mut a, core::ptr::null_mut());
        say!(t, "alpha only: {} alpha {} (flags untouched {:#x})", ly_b(r), a, f);
        let r = GetLayeredWindowAttributes(l, core::ptr::null_mut(), core::ptr::null_mut(), &mut f);
        say!(t, "flags only: {} flags {:#x}", ly_b(r), f);
        // errors
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(plain, &mut k, &mut a, &mut f);
        say!(t, "non-layered window: {} error {}", ly_b(r), last_error());
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(plain, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "non-layered window, all NULL: {} error {}", ly_b(r), last_error());
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(core::ptr::null_mut(), &mut k, &mut a, &mut f);
        say!(t, "NULL window: {} error {}", ly_b(r), last_error());
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(0x0bad_0bad_usize as HWND, &mut k, &mut a, &mut f);
        say!(t, "made-up window handle: {} error {}", ly_b(r), last_error());
        DestroyWindow(l);
        set_error(0x1234);
        let r = GetLayeredWindowAttributes(l, &mut k, &mut a, &mut f);
        say!(t, "destroyed window: {} error {}", ly_b(r), last_error());
    }
}
