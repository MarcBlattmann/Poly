//! CreateDialogParamA: a modeless dialog from a template given as a number
//! or as a string (any case); the dialog and its controls as windows; the
//! strings the ANSI dialog procedure gets; the result is NULL with the error
//! of the missing template.
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

const WM_NULL: u32 = 0x0000;
const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_MOVE: u32 = 0x0003;
const WM_SIZE: u32 = 0x0005;
const WM_ENABLE: u32 = 0x000A;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SHOWWINDOW: u32 = 0x0018;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const WM_WINDOWPOSCHANGED: u32 = 0x0047;
const WM_STYLECHANGING: u32 = 0x007C;
const WM_STYLECHANGED: u32 = 0x007D;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCDESTROY: u32 = 0x0082;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCPAINT: u32 = 0x0085;
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
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_CHILDACTIVATE: u32 = 0x0022;
const WM_USER: u32 = 0x0400;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
const GW_HWNDNEXT: u32 = 2;
const GW_CHILD: u32 = 5;
const PM_REMOVE: u32 = 0x0001;
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
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
struct GUITHREADINFO {
    cb_size: u32,
    flags: u32,
    active: HWND,
    focus: HWND,
    capture: HWND,
    menu_owner: HWND,
    move_size: HWND,
    caret: HWND,
    caret_rect: RECT,
}

/// SendInput of a list; returns how many were inserted.
fn send(inputs: &[INPUT]) -> u32 {
    // SAFETY: a slice of filled INPUTs of the documented size.
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), core::mem::size_of::<INPUT>() as i32) }
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

type DLGPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> isize>;
const WM_INITDIALOG: u32 = 0x0110;
const DM_GETDEFID: u32 = 0x0400;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindow(hwnd: HWND, cmd: u32) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn IsWindowUnicode(hwnd: HWND) -> BOOL;
    fn SetActiveWindow(hwnd: HWND) -> HWND;
    fn GetWindowTextA(hwnd: HWND, buf: *mut u8, max: i32) -> i32;
    fn SendInput(n: u32, inputs: *const INPUT, size: i32) -> u32;
    fn GetGUIThreadInfo(thread: u32, info: *mut GUITHREADINFO) -> BOOL;
    fn CreateDialogParamW(instance: HINSTANCE, name: *const u16, parent: HWND, proc_: DLGPROC, param: isize) -> HWND;
    fn DialogBoxParamW(instance: HINSTANCE, name: *const u16, parent: HWND, proc_: DLGPROC, param: isize) -> isize;
    fn EndDialog(hwnd: HWND, result: isize) -> BOOL;
    fn GetDlgCtrlID(hwnd: HWND) -> i32;
    fn SetWindowTextA(hwnd: HWND, text: *const u8) -> BOOL;
    fn GetCaretPos(pt: *mut POINT) -> BOOL;
    fn CreateDialogParamA(instance: HINSTANCE, name: *const u8, parent: HWND, proc_: DLGPROC, param: isize) -> HWND;
    fn RegisterClassExA(class: *const c_void) -> u16;
    fn CreateWindowExA(ex: u32, class: *const u8, name: *const u8, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: HMENU, instance: HINSTANCE, param: *mut c_void) -> HWND;
    fn SendMessageA(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn DefWindowProcA(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetClassNameA(hwnd: HWND, buf: *mut u8, max: i32) -> i32;
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

// ---- caret helpers of these tests ----

unsafe extern "system" fn caret_wp(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: default processing.
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// A top-level window of the test (class registered once); `style` as given.
fn caret_window(style: u32, parent: HWND) -> HWND {
    static DONE: Global<bool> = Global::new(false);
    if !*DONE.get() {
        register(wz!("CaretTestWnd"), Some(caret_wp), 0, 0, core::ptr::null_mut());
        *DONE.get() = true;
    }
    create(0, wz!("CaretTestWnd"), wz!("caret"), style, 10, 10, 200, 100, parent, 0)
}

/// The thread's caret as GetGUIThreadInfo reports it: "owner <ours|null> rect (l,t,r,b) flags N".
fn caret_info(t: &mut Test, label: &str, ours: HWND) {
    let mut gi: GUITHREADINFO = zeroed();
    gi.cb_size = core::mem::size_of::<GUITHREADINFO>() as u32;
    // SAFETY: gi is a GUITHREADINFO.
    let ok = unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut gi) };
    let who = if gi.caret.is_null() { "null" } else if gi.caret == ours { "ours" } else { "other" };
    say!(t, "{label}: GetGUIThreadInfo {ok} caret window {who} rect ({},{},{},{}) caret-blinking {}", gi.caret_rect.left, gi.caret_rect.top, gi.caret_rect.right, gi.caret_rect.bottom, gi.flags & 1);
}

/// GetCaretPos with the result and last error.
fn caret_pos(t: &mut Test, label: &str) -> POINT {
    let mut p = POINT { x: -7777, y: -7777 };
    set_error(0xDEAD);
    // SAFETY: p is a POINT.
    let r = unsafe { GetCaretPos(&mut p) };
    let e = last_error();
    say!(t, "{label}: GetCaretPos = {r} err={} point ({},{})", if r != 0 { -2 } else if e == 0xDEAD { -1 } else { e as i64 }, p.x, p.y);
    p
}

/// A window of the test that has the focus (the caret blinks only then).
fn focused_window() -> HWND {
    let w = caret_window(WS_OVERLAPPEDWINDOW | WS_VISIBLE, core::ptr::null_mut());
    pump();
    // SAFETY: the window is ours.
    unsafe {
        SetActiveWindow(w);
        SetFocus(w);
    }
    w
}

/// Whether the caret is shown (GUI_CARETBLINKING) as a word.
fn caret_shown() -> &'static str {
    let mut gi: GUITHREADINFO = zeroed();
    gi.cb_size = core::mem::size_of::<GUITHREADINFO>() as u32;
    // SAFETY: gi is a GUITHREADINFO.
    unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut gi) };
    if gi.flags & 1 != 0 { "shown" } else { "hidden" }
}

/// A call made on another thread: runs `f` there and waits.
fn on_thread(f: unsafe extern "system" fn(*mut c_void) -> u32, arg: usize) {
    // SAFETY: a thread with its own stack, joined at once.
    unsafe {
        let th = CreateThread(core::ptr::null_mut(), 0, f, arg as *mut c_void, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10000);
        CloseHandle(th);
    }
}

// ---- ANSI helpers of these tests ----

// The code page of the oracle's ANSI strings (the tests record what depends on it).
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetACP() -> u32;
    fn GetOEMCP() -> u32;
}

/// Bytes up to the first NUL: printable ASCII as is, any other byte as <hh>.
fn abytes(buf: &[u8]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    for &c in buf {
        if c == 0 {
            break;
        }
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as char);
        } else {
            let _ = write!(b, "<{c:02x}>");
        }
    }
    b
}

/// UTF-16 up to the first NUL: printable ASCII as is, any other unit as <hhhh>.
fn wshow(buf: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    for &c in buf {
        if c == 0 {
            break;
        }
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// Length up to the first NUL.
fn alen(b: &[u8]) -> usize {
    b.iter().position(|&c| c == 0).unwrap_or(b.len())
}

/// Text of a window through GetWindowTextW, shown with `wshow`.
fn wtext_of(h: HWND) -> Buf<300> {
    let mut v = [0u16; 200];
    // SAFETY: v has 200 units.
    unsafe { GetWindowTextW(h, v.as_mut_ptr(), 200) };
    wshow(&v)
}

/// Text of a window through GetWindowTextA, shown with `abytes`.
fn atext_of(h: HWND) -> Buf<300> {
    let mut v = [0u8; 300];
    // SAFETY: v has 300 bytes.
    unsafe { GetWindowTextA(h, v.as_mut_ptr(), 300) };
    abytes(&v)
}

// A few strings; the bytes are those of the Windows ANSI code page 1252, the
// UTF-16 units what they stand for there.
/// "caf" + E9, space, 80 (the euro sign in 1252), space, 81 (not assigned in 1252), space, FF
const A_HIGH: &[u8] = b"caf\xE9 \x80 \x81 \xFF\0";
/// "caf" + U+00E9, U+4E2D (a Chinese character no single-byte code page has), U+0416 (Cyrillic), U+20AC (euro)
const W_MIXED: [u16; 8] = [0x63, 0x61, 0x66, 0xE9, 0x4E2D, 0x0416, 0x20AC, 0];

// A pair of window classes that keep the text they are given the way an edit control
// would, one registered with RegisterClassExA (an ANSI window procedure), one with
// RegisterClassExW. They log what their procedure receives.
static TXT_A: Global<[u8; 80]> = Global::new([0; 80]);
static TXT_W: Global<[u16; 80]> = Global::new([0; 80]);

unsafe extern "system" fn a_child(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: l is a NUL-terminated string (WM_SETTEXT) or a buffer of w bytes (WM_GETTEXT).
    unsafe {
        if m >= WM_USER && m < 0xC000 {
            logln!("   [A window] message {:#x} wParam {:#x} lParam {:#x}", m, w, l);
            return 0x1234;
        }
        match m {
            WM_SETTEXT => {
                let p = l as *const u8;
                let mut n = 0;
                while n < 79 && *p.add(n) != 0 {
                    (*TXT_A.get())[n] = *p.add(n);
                    n += 1;
                }
                (*TXT_A.get())[n] = 0;
                logln!("   [A window] WM_SETTEXT got bytes '{}'", abytes(&*TXT_A.get()).as_str());
                1
            }
            WM_GETTEXT => {
                let n = alen(&*TXT_A.get()).min((w as usize).saturating_sub(1));
                let p = l as *mut u8;
                for i in 0..n {
                    *p.add(i) = (*TXT_A.get())[i];
                }
                if w > 0 {
                    *p.add(n) = 0;
                }
                logln!("   [A window] WM_GETTEXT wParam {w} gave {n} bytes");
                n as LRESULT
            }
            WM_GETTEXTLENGTH => alen(&*TXT_A.get()) as LRESULT,
            _ => DefWindowProcA(h, m, w, l),
        }
    }
}

unsafe extern "system" fn w_child(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: l is a NUL-terminated string (WM_SETTEXT) or a buffer of w units (WM_GETTEXT).
    unsafe {
        if m >= WM_USER && m < 0xC000 {
            logln!("   [W window] message {:#x} wParam {:#x} lParam {:#x}", m, w, l);
            return 0x1234;
        }
        match m {
            WM_SETTEXT => {
                let p = l as *const u16;
                let mut n = 0;
                while n < 79 && *p.add(n) != 0 {
                    (*TXT_W.get())[n] = *p.add(n);
                    n += 1;
                }
                (*TXT_W.get())[n] = 0;
                logln!("   [W window] WM_SETTEXT got units '{}'", wshow(&*TXT_W.get()).as_str());
                1
            }
            WM_GETTEXT => {
                let mut n = 0;
                while (*TXT_W.get())[n] != 0 {
                    n += 1;
                }
                let n = n.min((w as usize).saturating_sub(1));
                let p = l as *mut u16;
                for i in 0..n {
                    *p.add(i) = (*TXT_W.get())[i];
                }
                if w > 0 {
                    *p.add(n) = 0;
                }
                logln!("   [W window] WM_GETTEXT wParam {w} gave {n} units");
                n as LRESULT
            }
            WM_GETTEXTLENGTH => {
                let mut n = 0;
                while (*TXT_W.get())[n] != 0 {
                    n += 1;
                }
                n as LRESULT
            }
            _ => DefWindowProcW(h, m, w, l),
        }
    }
}

/// A parent window (a Unicode one) with an ANSI child with id 11 and a Unicode child with id 12.
fn ansi_pair() -> (HWND, HWND, HWND) {
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cb_size = core::mem::size_of::<WNDCLASSEXW>() as u32;
    wc.instance = instance();
    wc.wnd_proc = Some(a_child);
    wc.class_name = b"AnsiKid\0".as_ptr() as *const u16;
    // SAFETY: WNDCLASSEXA and WNDCLASSEXW have the same layout; the name is a byte string here.
    unsafe { RegisterClassExA(&wc as *const WNDCLASSEXW as *const c_void) };
    register(wz!("WideKid"), Some(w_child), 0, 0, core::ptr::null_mut());
    register(wz!("KidParent"), Some(caret_like_wp), 0, 0, core::ptr::null_mut());
    let p = create(0, wz!("KidParent"), wz!("parent"), WS_OVERLAPPEDWINDOW, 0, 0, 200, 100, core::ptr::null_mut(), 0);
    // SAFETY: class names are NUL-terminated byte strings and UTF-16 strings; the parent is ours.
    unsafe {
        let a = CreateWindowExA(0, b"AnsiKid\0".as_ptr(), b"\0".as_ptr(), WS_CHILD, 0, 0, 50, 20, p, 11 as HMENU, instance(), core::ptr::null_mut());
        let w = create(0, wz!("WideKid"), wz!(""), WS_CHILD, 60, 0, 50, 20, p, 12);
        (p, a, w)
    }
}

unsafe extern "system" fn caret_like_wp(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: default processing.
    unsafe { DefWindowProcW(h, m, w, l) }
}

// ---- ANSI dialog procedure helpers of these tests ----

/// The dialog procedure of the ANSI tests: logs what it is given for the text messages
/// (bytes through an ANSI procedure) and numbers for the others; WM_INITDIALOG calls PROBE.
static PROBE: Global<Option<fn(&mut Test, HWND)>> = Global::new(None);
static PROBE_T: Global<usize> = Global::new(0);
static PROBE_END: Global<bool> = Global::new(true);

unsafe extern "system" fn dlg_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    // SAFETY: l is a NUL-terminated byte string for WM_SETTEXT; the test body runs in WM_INITDIALOG.
    unsafe {
        match msg {
            WM_SETTEXT => {
                let p = l as *const u8;
                let mut v = [0u8; 100];
                let mut n = 0;
                while n < 99 && *p.add(n) != 0 {
                    v[n] = *p.add(n);
                    n += 1;
                }
                logln!("   [dialog procedure] WM_SETTEXT bytes '{}'", abytes(&v).as_str());
                return 0;
            }
            WM_INITDIALOG => {
                logln!("   [dialog procedure] WM_INITDIALOG lParam {l}");
                if let Some(f) = *PROBE.get() {
                    let t = &mut *(*PROBE_T.get() as *mut Test);
                    f(t, hwnd);
                }
                if *PROBE_END.get() {
                    EndDialog(hwnd, 55);
                }
                return 0;
            }
            m if m == WM_USER + 5 || m == WM_USER + 6 => {
                logln!("   [dialog procedure] message {m:#x} wParam {w:#x} lParam {l:#x}");
                return 0;
            }
            _ => {}
        }
    }
    0
}

/// What the dialog and its controls are, as windows.
fn dlg_report(t: &mut Test, d: HWND) {
    // SAFETY: queries on the dialog's windows.
    unsafe {
        say!(t, "dialog: IsWindowUnicode {} title through GetWindowTextA '{}' through GetWindowTextW '{}'", IsWindowUnicode(d), atext_of(d).as_str(), wtext_of(d).as_str());
        let mut c = GetWindow(d, GW_CHILD);
        let mut n = 0;
        while !c.is_null() && n < 20 {
            let mut cn = [0u8; 40];
            GetClassNameA(c, cn.as_mut_ptr(), 40);
            say!(t, "  control {} class '{}' IsWindowUnicode {} text A '{}' W '{}'", GetDlgCtrlID(c), abytes(&cn).as_str(), IsWindowUnicode(c), atext_of(c).as_str(), wtext_of(c).as_str());
            c = GetWindow(c, GW_HWNDNEXT);
            n += 1;
        }
    }
}

/// The messages the ANSI tests send to the dialog from inside its procedure.
fn dlg_messages(t: &mut Test, d: HWND) {
    // SAFETY: messages to the dialog window with strings that live through the calls.
    unsafe {
        log_clear();
        let r = SendMessageA(d, WM_SETTEXT, 0, b"Titl\xE9 A\0".as_ptr() as LPARAM);
        say!(t, "SendMessageA(WM_SETTEXT) = {r}; title now W '{}'", wtext_of(d).as_str());
        let r = SendMessageW(d, WM_SETTEXT, 0, W_MIXED.as_ptr() as LPARAM);
        say!(t, "SendMessageW(WM_SETTEXT, mixed text) = {r}; title now W '{}' A '{}'", wtext_of(d).as_str(), atext_of(d).as_str());
        let r = SendMessageA(d, WM_USER + 5, 0x66, 0x77);
        say!(t, "SendMessageA(WM_USER+5) = {r}");
        let r = SendMessageW(d, WM_USER + 6, 0x66, 0x77);
        say!(t, "SendMessageW(WM_USER+6) = {r}");
        let r = SetWindowTextA(d, b"SetWindowTextA \xFF\0".as_ptr());
        say!(t, "SetWindowTextA = {r}; title now W '{}'", wtext_of(d).as_str());
        flush_log(t, "");
    }
}

core::arch::global_asm!(
    ".section .rsrc$01,\"dr\"\n",
    ".balign 4\n",
    ".long 0, 0, 0\n",
    ".short 0, 2\n",
    ".long 4\n",
    ".long 2147483680\n",
    ".long 5\n",
    ".long 2147483704\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 200\n",
    ".long 2147483792\n",
    ".long 0, 0, 0\n",
    ".short 1, 8\n",
    ".long 2147484192\n",
    ".long 2147483816\n",
    ".long 100\n",
    ".long 2147483840\n",
    ".long 101\n",
    ".long 2147483864\n",
    ".long 102\n",
    ".long 2147483888\n",
    ".long 103\n",
    ".long 2147483912\n",
    ".long 104\n",
    ".long 2147483936\n",
    ".long 110\n",
    ".long 2147483960\n",
    ".long 111\n",
    ".long 2147483984\n",
    ".long 112\n",
    ".long 2147484008\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 384\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 400\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 416\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 432\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 448\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 464\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 480\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 496\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 512\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 528\n",
    ".rva .Lrsrc_d0\n",
    ".long 32, 0, 0\n",
    ".rva .Lrsrc_d1\n",
    ".long 64, 0, 0\n",
    ".rva .Lrsrc_d2\n",
    ".long 480, 0, 0\n",
    ".rva .Lrsrc_d3\n",
    ".long 312, 0, 0\n",
    ".rva .Lrsrc_d4\n",
    ".long 24, 0, 0\n",
    ".rva .Lrsrc_d5\n",
    ".long 114, 0, 0\n",
    ".rva .Lrsrc_d6\n",
    ".long 88, 0, 0\n",
    ".rva .Lrsrc_d7\n",
    ".long 390, 0, 0\n",
    ".rva .Lrsrc_d8\n",
    ".long 246, 0, 0\n",
    ".rva .Lrsrc_d9\n",
    ".long 246, 0, 0\n",
    ".short 8\n",
    ".short 78, 65, 77, 69, 68, 68, 76, 71\n",
    ".section .rsrc$02,\"dr\"\n",
    ".balign 4\n",
    ".Lrsrc_d0:\n",
    ".byte 0, 0, 0, 0, 144, 0, 38, 0, 70, 0, 105, 0, 108, 0, 101, 0, 0, 0, 128, 0, 45, 1, 79, 0\n",
    ".byte 112, 0, 101, 0, 110, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d1:\n",
    ".byte 128, 0, 0, 128, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 50, 0, 30, 0, 0, 0, 0, 0, 110, 0\n",
    ".byte 97, 0, 109, 0, 101, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0\n",
    ".byte 20, 0, 10, 0, 5, 0, 255, 255, 130, 0, 110, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d2:\n",
    ".byte 192, 0, 200, 128, 0, 0, 0, 0, 11, 0, 10, 0, 10, 0, 160, 0, 120, 0, 0, 0, 0, 0, 84, 0\n",
    ".byte 101, 0, 115, 0, 116, 0, 32, 0, 100, 0, 105, 0, 97, 0, 108, 0, 111, 0, 103, 0, 0, 0, 8, 0\n",
    ".byte 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 10, 0, 233, 3, 255, 255\n",
    ".byte 130, 0, 38, 0, 78, 0, 97, 0, 109, 0, 101, 0, 58, 0, 0, 0, 0, 0, 0, 0, 128, 0, 129, 80\n",
    ".byte 0, 0, 0, 0, 5, 0, 18, 0, 80, 0, 12, 0, 234, 3, 255, 255, 129, 0, 105, 0, 110, 0, 105, 0\n",
    ".byte 116, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 3, 0, 1, 80, 0, 0, 0, 0, 5, 0, 34, 0\n",
    ".byte 60, 0, 10, 0, 235, 3, 255, 255, 128, 0, 38, 0, 67, 0, 104, 0, 101, 0, 99, 0, 107, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 9, 0, 3, 80, 0, 0, 0, 0, 5, 0, 46, 0, 40, 0, 10, 0, 236, 3, 255, 255\n",
    ".byte 128, 0, 38, 0, 79, 0, 110, 0, 101, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0\n",
    ".byte 50, 0, 46, 0, 40, 0, 10, 0, 237, 3, 255, 255, 128, 0, 84, 0, 38, 0, 119, 0, 111, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0, 95, 0, 46, 0, 40, 0, 10, 0, 238, 3, 255, 255\n",
    ".byte 128, 0, 84, 0, 38, 0, 104, 0, 114, 0, 101, 0, 101, 0, 0, 0, 0, 0, 0, 0, 6, 0, 3, 80\n",
    ".byte 0, 0, 0, 0, 5, 0, 58, 0, 60, 0, 10, 0, 239, 3, 255, 255, 128, 0, 84, 0, 38, 0, 114, 0\n",
    ".byte 105, 0, 0, 0, 0, 0, 0, 0, 1, 0, 129, 80, 0, 0, 0, 0, 5, 0, 70, 0, 60, 0, 30, 0\n",
    ".byte 240, 3, 255, 255, 131, 0, 0, 0, 0, 0, 0, 0, 1, 0, 3, 80, 0, 0, 0, 0, 100, 0, 100, 0\n",
    ".byte 50, 0, 14, 0, 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80\n",
    ".byte 0, 0, 0, 0, 45, 0, 100, 0, 50, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0\n",
    ".byte 99, 0, 101, 0, 108, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 80, 0, 0, 0, 0, 90, 0, 5, 0\n",
    ".byte 65, 0, 40, 0, 241, 3, 255, 255, 128, 0, 71, 0, 114, 0, 111, 0, 117, 0, 112, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d3:\n",
    ".byte 1, 0, 255, 255, 99, 0, 0, 0, 0, 0, 0, 0, 192, 0, 192, 128, 5, 0, 10, 0, 10, 0, 100, 0\n",
    ".byte 60, 0, 0, 0, 0, 0, 69, 0, 120, 0, 32, 0, 100, 0, 105, 0, 97, 0, 108, 0, 111, 0, 103, 0\n",
    ".byte 0, 0, 8, 0, 188, 2, 0, 1, 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0\n",
    ".byte 32, 0, 68, 0, 108, 0, 103, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 80\n",
    ".byte 5, 0, 5, 0, 40, 0, 10, 0, 255, 255, 0, 0, 255, 255, 130, 0, 83, 0, 116, 0, 97, 0, 116, 0\n",
    ".byte 105, 0, 99, 0, 32, 0, 45, 0, 49, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 129, 80, 5, 0, 18, 0, 40, 0, 12, 0, 69, 35, 1, 0, 255, 255, 129, 0, 98, 0, 105, 0\n",
    ".byte 103, 0, 32, 0, 105, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80\n",
    ".byte 5, 0, 34, 0, 40, 0, 12, 0, 7, 0, 0, 0, 255, 255, 129, 0, 115, 0, 101, 0, 118, 0, 101, 0\n",
    ".byte 110, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 50, 0, 34, 0\n",
    ".byte 40, 0, 12, 0, 7, 0, 0, 0, 255, 255, 129, 0, 115, 0, 101, 0, 118, 0, 101, 0, 110, 0, 32, 0\n",
    ".byte 116, 0, 111, 0, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80\n",
    ".byte 50, 0, 45, 0, 40, 0, 12, 0, 1, 0, 0, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d4:\n",
    ".byte 128, 0, 0, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 0, 30, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d5:\n",
    ".byte 128, 0, 0, 128, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 50, 0, 30, 0, 0, 0, 0, 0, 117, 0\n",
    ".byte 110, 0, 107, 0, 110, 0, 111, 0, 119, 0, 110, 0, 32, 0, 99, 0, 108, 0, 97, 0, 115, 0, 115, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 20, 0, 10, 0, 1, 0, 78, 0\n",
    ".byte 111, 0, 83, 0, 117, 0, 99, 0, 104, 0, 67, 0, 111, 0, 110, 0, 116, 0, 114, 0, 111, 0, 108, 0\n",
    ".byte 67, 0, 108, 0, 97, 0, 115, 0, 115, 0, 0, 0, 120, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d6:\n",
    ".byte 192, 0, 192, 128, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 80, 0, 40, 0, 255, 255, 200, 0, 0, 0\n",
    ".byte 119, 0, 105, 0, 116, 0, 104, 0, 32, 0, 109, 0, 101, 0, 110, 0, 117, 0, 0, 0, 10, 0, 65, 0\n",
    ".byte 114, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0, 5, 0, 5, 0\n",
    ".byte 40, 0, 12, 0, 234, 3, 255, 255, 129, 0, 101, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d7:\n",
    ".byte 192, 0, 200, 128, 0, 0, 0, 0, 9, 0, 10, 0, 10, 0, 160, 0, 90, 0, 0, 0, 0, 0, 77, 0\n",
    ".byte 110, 0, 101, 0, 109, 0, 111, 0, 110, 0, 105, 0, 99, 0, 115, 0, 32, 0, 233, 0, 45, 78, 0, 0\n",
    ".byte 8, 0, 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0\n",
    ".byte 103, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 10, 0, 209, 7, 255, 255\n",
    ".byte 130, 0, 38, 0, 220, 0, 98, 0, 101, 0, 114, 0, 58, 0, 0, 0, 0, 0, 0, 0, 128, 0, 129, 80\n",
    ".byte 0, 0, 0, 0, 70, 0, 5, 0, 60, 0, 12, 0, 210, 7, 255, 255, 129, 0, 111, 0, 110, 0, 101, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 20, 0, 60, 0, 10, 0, 211, 7, 255, 255\n",
    ".byte 130, 0, 38, 0, 233, 0, 116, 0, 233, 0, 58, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0\n",
    ".byte 70, 0, 20, 0, 60, 0, 12, 0, 212, 7, 255, 255, 129, 0, 116, 0, 119, 0, 111, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 35, 0, 60, 0, 10, 0, 213, 7, 255, 255, 130, 0, 38, 0\n",
    ".byte 45, 78, 58, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 70, 0, 35, 0, 60, 0, 12, 0\n",
    ".byte 214, 7, 255, 255, 129, 0, 116, 0, 104, 0, 114, 0, 101, 0, 101, 0, 0, 0, 0, 0, 3, 0, 1, 80\n",
    ".byte 0, 0, 0, 0, 5, 0, 50, 0, 60, 0, 10, 0, 215, 7, 255, 255, 128, 0, 38, 0, 67, 0, 97, 0\n",
    ".byte 102, 0, 233, 0, 0, 0, 0, 0, 1, 0, 3, 80, 0, 0, 0, 0, 100, 0, 70, 0, 50, 0, 14, 0\n",
    ".byte 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0\n",
    ".byte 45, 0, 70, 0, 50, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0\n",
    ".byte 108, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d8:\n",
    ".byte 192, 0, 192, 128, 0, 0, 0, 0, 4, 0, 10, 0, 10, 0, 120, 0, 60, 0, 0, 0, 68, 0, 108, 0\n",
    ".byte 103, 0, 80, 0, 114, 0, 111, 0, 99, 0, 84, 0, 101, 0, 115, 0, 116, 0, 65, 0, 0, 0, 79, 0\n",
    ".byte 119, 0, 110, 0, 32, 0, 99, 0, 108, 0, 97, 0, 115, 0, 115, 0, 32, 0, 65, 0, 0, 0, 8, 0\n",
    ".byte 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0\n",
    ".byte 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 12, 0, 185, 11, 255, 255\n",
    ".byte 129, 0, 102, 0, 105, 0, 114, 0, 115, 0, 116, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0\n",
    ".byte 5, 0, 20, 0, 60, 0, 12, 0, 186, 11, 255, 255, 129, 0, 115, 0, 101, 0, 99, 0, 111, 0, 110, 0\n",
    ".byte 100, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 70, 0, 40, 0, 40, 0, 14, 0\n",
    ".byte 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0\n",
    ".byte 25, 0, 40, 0, 40, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0\n",
    ".byte 108, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d9:\n",
    ".byte 192, 0, 192, 128, 0, 0, 0, 0, 4, 0, 10, 0, 10, 0, 120, 0, 60, 0, 0, 0, 68, 0, 108, 0\n",
    ".byte 103, 0, 80, 0, 114, 0, 111, 0, 99, 0, 84, 0, 101, 0, 115, 0, 116, 0, 87, 0, 0, 0, 79, 0\n",
    ".byte 119, 0, 110, 0, 32, 0, 99, 0, 108, 0, 97, 0, 115, 0, 115, 0, 32, 0, 87, 0, 0, 0, 8, 0\n",
    ".byte 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0\n",
    ".byte 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 12, 0, 185, 11, 255, 255\n",
    ".byte 129, 0, 102, 0, 105, 0, 114, 0, 115, 0, 116, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0\n",
    ".byte 5, 0, 20, 0, 60, 0, 12, 0, 186, 11, 255, 255, 129, 0, 115, 0, 101, 0, 99, 0, 111, 0, 110, 0\n",
    ".byte 100, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 70, 0, 40, 0, 40, 0, 14, 0\n",
    ".byte 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0\n",
    ".byte 25, 0, 40, 0, 40, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0\n",
    ".byte 108, 0, 0, 0, 0, 0\n",
);

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    let inst = instance();
    *PROBE.get() = None;
    *PROBE_END.get() = false;
    log_clear();
    set_error(0xDEAD);
    let d = unsafe { CreateDialogParamA(inst, mira(100), core::ptr::null_mut(), Some(dlg_a), 21) };
    say!(t, "CreateDialogParamA(100): {} err={}", hwnd_kind(d), { let e = last_error(); if e == 0xDEAD { -1 } else { e as i64 } });
    flush_log(t, "");
    dlg_report(t, d);
    dlg_messages(t, d);
    say!(t, "DM_GETDEFID {:#x}", unsafe { SendMessageA(d, DM_GETDEFID, 0, 0) } as u32);
    call!(t, "DestroyWindow", unsafe { DestroyWindow(d) });
    let d = unsafe { CreateDialogParamA(inst, b"NAMEDDLG\0".as_ptr(), core::ptr::null_mut(), Some(dlg_a), 22) };
    say!(t, "CreateDialogParamA('NAMEDDLG'): {}", hwnd_kind(d));
    unsafe { DestroyWindow(d) };
    let d = unsafe { CreateDialogParamA(inst, b"nameddlg\0".as_ptr(), core::ptr::null_mut(), Some(dlg_a), 23) };
    say!(t, "CreateDialogParamA('nameddlg'): {}", hwnd_kind(d));
    unsafe { DestroyWindow(d) };
    call!(t, "CreateDialogParamA('NoSuch')", unsafe { CreateDialogParamA(inst, b"NoSuch\0".as_ptr(), core::ptr::null_mut(), Some(dlg_a), 0) }.is_null() as i32);
    call!(t, "CreateDialogParamA(999)", unsafe { CreateDialogParamA(inst, mira(999), core::ptr::null_mut(), Some(dlg_a), 0) }.is_null() as i32);
    call!(t, "made-up module", unsafe { CreateDialogParamA(0x1000_0000 as HINSTANCE, mira(100), core::ptr::null_mut(), Some(dlg_a), 0) }.is_null() as i32);
    call!(t, "bad parent", unsafe { CreateDialogParamA(inst, mira(102), 0x0bad_0bad as HWND, Some(dlg_a), 0) }.is_null() as i32);
    // a wide-created dialog with the same procedure
    log_clear();
    let d = unsafe { CreateDialogParamW(inst, mir(100), core::ptr::null_mut(), Some(dlg_a), 24) };
    dlg_report(t, d);
    dlg_messages(t, d);
    unsafe { DestroyWindow(d) };
}
