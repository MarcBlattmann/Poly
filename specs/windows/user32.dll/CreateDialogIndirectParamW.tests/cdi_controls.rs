//! CreateDialogIndirectParamW controls and messages: the order and styles of
//! the controls, text as UTF-16, the default button, tab order with
//! GetNextDlgTabItem, SetDlgItemText / GetDlgItemText on the new dialog,
//! WM_CLOSE sent to it, IsDialogMessage on it; the same results as for a
//! dialog made from a resource.
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
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
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
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const GWLP_USERDATA: i32 = -21;
const GW_HWNDNEXT: u32 = 2;
const GW_CHILD: u32 = 5;
const PM_REMOVE: u32 = 0x0001;
const VK_RETURN: usize = 0x0D;
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

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Remove every waiting message without dispatching (log them if `log`).
fn drain(log: bool) -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        if log {
            logln!("drained {} w={:#x} l={:#x}", MsgName(msg.message), msg.wparam, msg.lparam);
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

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
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

type DLGPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> isize>;
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
const DM_GETDEFID: u32 = 0x0400;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn GetWindow(hwnd: HWND, cmd: u32) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn CreateDialogIndirectParamW(instance: HINSTANCE, template: *const c_void, parent: HWND, proc_: DLGPROC, param: isize) -> HWND;
    fn DialogBoxParamW(instance: HINSTANCE, name: *const u16, parent: HWND, proc_: DLGPROC, param: isize) -> isize;
    fn EndDialog(hwnd: HWND, result: isize) -> BOOL;
    fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
    fn GetDlgCtrlID(hwnd: HWND) -> i32;
    fn SetDlgItemTextW(hwnd: HWND, id: i32, text: *const u16) -> BOOL;
    fn GetDlgItemTextW(hwnd: HWND, id: i32, buf: *mut u16, max: i32) -> u32;
    fn IsDialogMessageW(hwnd: HWND, msg: *mut MSG) -> BOOL;
    fn GetNextDlgTabItem(hwnd: HWND, ctl: HWND, previous: BOOL) -> HWND;
}

/// Call `$e` with the last error set to a marker; print "label: result err=N"
/// (err -1: the call did not touch the last error). Gives the result.
macro_rules! call {
    ($t:expr, $label:expr, $e:expr) => {{
        set_error(0xDEAD);
        let r = ($e) as i64;
        let e = last_error();
        if stringify!($e).contains("is_null") && r == 0 {
            say!($t, "{}: {}", $label, r);
        } else if r == 0 || r == -1 || stringify!($e).contains("is_null") {
            say!($t, "{}: {} err={}", $label, r, if e == 0xDEAD { -1 } else { e as i64 });
        } else {
            say!($t, "{}: {}", $label, r);
        }
        r
    }};
}

/// MAKEINTRESOURCE
fn mir(id: usize) -> *const u16 {
    id as *const u16
}

/// A menu handle made up by the test (never a real one).
const BOGUS_MENU: usize = 0x0bad_0bad;

/// MAKEINTRESOURCE for the ANSI functions
fn mira(id: usize) -> *const u8 {
    id as *const u8
}

// ---- dialog helpers of these tests ----

static CUR_T: Global<usize> = Global::new(0);
static CUR_BODY: Global<Option<fn(&mut Test, HWND)>> = Global::new(None);
static CUR_PARAM: Global<isize> = Global::new(0);

/// A dialog procedure that runs the body of the test inside WM_INITDIALOG and
/// ends the dialog with `result` (the body may call EndDialog itself first).
unsafe extern "system" fn run_proc(hwnd: HWND, msg: u32, _w: WPARAM, l: LPARAM) -> isize {
    if msg == WM_INITDIALOG {
        *CUR_PARAM.get() = l;
        if let Some(f) = *CUR_BODY.get() {
            // SAFETY: CUR_T holds the address of the running test's `Test`, alive for the whole dialog.
            let t = unsafe { &mut *(*CUR_T.get() as *mut Test) };
            f(t, hwnd);
        }
        // SAFETY: the dialog window.
        unsafe { EndDialog(hwnd, 99) };
        return 0;
    }
    0
}

/// Show the dialog resource `id` modally and run `body` inside it.
fn with_dialog(t: &mut Test, id: usize, body: fn(&mut Test, HWND)) -> isize {
    *CUR_T.get() = t as *mut Test as usize;
    *CUR_BODY.get() = Some(body);
    // SAFETY: a resource dialog of this executable; run_proc is a dialog procedure.
    unsafe { DialogBoxParamW(instance(), mir(id), core::ptr::null_mut(), Some(run_proc), 7) }
}

/// The class name of a window as ASCII.
fn class_of(h: HWND, out: &mut Buf<300>) {
    let mut b = [0u16; 60];
    // SAFETY: b has 60 units.
    unsafe { GetClassNameW(h, b.as_mut_ptr(), 60) };
    ascii_of(&b, out);
}

/// The text of a window as ASCII.
fn text_of(h: HWND, out: &mut Buf<300>) {
    let mut b = [0u16; 200];
    // SAFETY: b has 200 units.
    unsafe { GetWindowTextW(h, b.as_mut_ptr(), 200) };
    ascii_of(&b, out);
}

#[repr(C, align(4))]
struct Aligned<const N: usize>([u8; N]);
static T_OLD: Aligned<218> = Aligned([192, 0, 200, 128, 0, 0, 0, 0, 4, 0, 10, 0, 10, 0, 120, 0, 70, 0, 0, 0, 0, 0, 73, 0, 110, 0, 100, 0, 105, 0, 114, 0, 101, 0, 99, 0, 116, 0, 0, 0, 8, 0, 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 50, 0, 10, 0, 233, 3, 255, 255, 130, 0, 38, 0, 78, 0, 97, 0, 109, 0, 101, 0, 58, 0, 0, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 5, 0, 18, 0, 80, 0, 12, 0, 234, 3, 255, 255, 129, 0, 105, 0, 110, 0, 105, 0, 116, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 60, 0, 50, 0, 50, 0, 14, 0, 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0, 5, 0, 50, 0, 50, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0, 108, 0, 0, 0, 0, 0]);
static T_EX: Aligned<212> = Aligned([1, 0, 255, 255, 99, 0, 0, 0, 0, 0, 0, 0, 192, 0, 192, 128, 3, 0, 10, 0, 10, 0, 100, 0, 60, 0, 0, 0, 0, 0, 69, 0, 120, 0, 32, 0, 105, 0, 110, 0, 100, 0, 105, 0, 114, 0, 101, 0, 99, 0, 116, 0, 0, 0, 8, 0, 188, 2, 0, 1, 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 80, 5, 0, 5, 0, 40, 0, 10, 0, 69, 35, 1, 0, 255, 255, 130, 0, 98, 0, 105, 0, 103, 0, 32, 0, 105, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 129, 80, 5, 0, 18, 0, 40, 0, 12, 0, 7, 0, 0, 0, 255, 255, 129, 0, 115, 0, 101, 0, 118, 0, 101, 0, 110, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 50, 0, 45, 0, 40, 0, 12, 0, 1, 0, 0, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0]);

unsafe extern "system" fn dlg_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => return 1,
        WM_CLOSE => {
            logln!("WM_CLOSE");
            return 0;
        }
        WM_COMMAND => {
            if (w & 0xFFFF) < 3 && (w >> 16) == 0 {
                logln!("WM_COMMAND id {}", w & 0xFFFF);
            }
        }
        _ => {}
    }
    let _ = (hwnd, l);
    0
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    // SAFETY: calls on the new dialog's windows.
    unsafe {
        let d = CreateDialogIndirectParamW(instance(), &T_OLD as *const _ as *const c_void, core::ptr::null_mut(), Some(dlg_proc), 0);
        let mut c = GetWindow(d, GW_CHILD);
        while !c.is_null() {
            let mut cb = Buf::<300>::new();
            class_of(c, &mut cb);
            let mut tb = Buf::<300>::new();
            text_of(c, &mut tb);
            say!(t, "child id {} class '{}' style {:#x} text '{}'", GetDlgCtrlID(c), cb.as_str(), GetWindowLongW(c, GWL_STYLE) as u32, tb.as_str());
            c = GetWindow(c, GW_HWNDNEXT);
        }
        let first = GetNextDlgTabItem(d, core::ptr::null_mut(), 0);
        say!(t, "first tab stop is control {}", GetDlgCtrlID(first));
        let second = GetNextDlgTabItem(d, first, 0);
        say!(t, "after it {}", GetDlgCtrlID(second));
        let third = GetNextDlgTabItem(d, second, 0);
        say!(t, "after that {}", GetDlgCtrlID(third));
        call!(t, "SetDlgItemTextW(1002)", SetDlgItemTextW(d, 1002, wz!("new").as_ptr()));
        let mut b = [0u16; 20];
        call!(t, "GetDlgItemTextW(1002)", GetDlgItemTextW(d, 1002, b.as_mut_ptr(), 20));
        say!(t, "  text units {:#x} {:#x} {:#x}", b[0], b[1], b[2]);
        say!(t, "DM_GETDEFID {:#x}", SendMessageW(d, DM_GETDEFID, 0, 0) as u32);
        log_clear();
        say!(t, "WM_CLOSE: {}", SendMessageW(d, WM_CLOSE, 0, 0));
        say!(t, "messages left after WM_CLOSE: {}", drain(false));
        flush_log(t, "  ");
        // IsDialogMessageW on the dialog
        let mut m: MSG = zeroed();
        m.hwnd = GetDlgItem(d, 1002);
        m.message = WM_KEYDOWN;
        m.wparam = VK_RETURN as usize;
        log_clear();
        say!(t, "IsDialogMessageW Enter: {}", IsDialogMessageW(d, &mut m));
        flush_log(t, "  ");
        DestroyWindow(d);
        // the extended template's help id
        let e = CreateDialogIndirectParamW(instance(), &T_EX as *const _ as *const c_void, core::ptr::null_mut(), Some(dlg_proc), 0);
        say!(t, "GetWindowContextHelpId of the extended dialog {}", GetWindowContextHelpId(e));
        say!(t, "ex style {:#x}", GetWindowLongW(e, GWL_EXSTYLE) as u32);
        DestroyWindow(e);
    }
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowContextHelpId(hwnd: HWND) -> u32;
}
