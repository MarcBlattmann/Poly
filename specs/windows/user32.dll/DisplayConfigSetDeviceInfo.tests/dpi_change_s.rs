//! The display scale changing one step down and back while windows are open: the messages the
//! windows get and what the DPI calls say; the display is at 200 percent at the start; the program
//! is a system DPI aware process (SetProcessDpiAwarenessContext(SYSTEM_AWARE)).
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
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_CHILDACTIVATE: u32 = 0x0022;
const WM_USER: u32 = 0x0400;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
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
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
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

const CTX_UNAWARE: isize = -1;
const CTX_SYSTEM: isize = -2;
const CTX_PM: isize = -3;
const CTX_PMV2: isize = -4;
const CTX_GDI: isize = -5;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_DPICHANGED_BEFOREPARENT: u32 = 0x02E2;
const WM_DPICHANGED_AFTERPARENT: u32 = 0x02E3;
const WM_GETDPISCALEDSIZE: u32 = 0x02E4;
type HANDLE = *mut c_void;
/// The name of the pseudo context a context equals ("other" when none).
fn ctx_name(c: isize) -> &'static str {
    // SAFETY: plain comparisons of context values.
    unsafe {
        if AreDpiAwarenessContextsEqual(c, CTX_UNAWARE) != 0 {
            "unaware"
        } else if AreDpiAwarenessContextsEqual(c, CTX_SYSTEM) != 0 {
            "system"
        } else if AreDpiAwarenessContextsEqual(c, CTX_PM) != 0 {
            "per-monitor"
        } else if AreDpiAwarenessContextsEqual(c, CTX_PMV2) != 0 {
            "per-monitor-v2"
        } else if AreDpiAwarenessContextsEqual(c, CTX_GDI) != 0 {
            "unaware-gdi-scaled"
        } else {
            "other"
        }
    }
}

fn thread_ctx_name() -> &'static str {
    // SAFETY: plain query.
    ctx_name(unsafe { GetThreadDpiAwarenessContext() })
}

fn pseudo(name: &str) -> isize {
    match name {
        "unaware" => CTX_UNAWARE,
        "system" => CTX_SYSTEM,
        "pm" => CTX_PM,
        "pmv2" => CTX_PMV2,
        _ => CTX_GDI,
    }
}

/// The label of a probe's mode letter: u none, U unaware, s system, p per-monitor v1, v per-monitor v2,
/// g unaware GDI scaled (a letter in capitals M: the awareness comes from the manifest).
fn mode_label(m: u8) -> &'static str {
    match m {
        b'u' => "no call",
        b'U' => "unaware",
        b's' => "system",
        b'p' => "per-monitor",
        b'v' => "per-monitor-v2",
        b'g' => "unaware-gdi-scaled",
        _ => "manifest",
    }
}

/// Puts the process into the probe's mode with SetProcessDpiAwarenessContext (modes u and M: nothing).
fn enter(t: &mut Test, m: u8) {
    let ctx = match m {
        b'U' => CTX_UNAWARE,
        b's' => CTX_SYSTEM,
        b'p' => CTX_PM,
        b'v' => CTX_PMV2,
        b'g' => CTX_GDI,
        _ => 0,
    };
    if ctx == 0 {
        say!(t, "mode {}: no SetProcessDpiAwarenessContext call", mode_label(m));
        return;
    }
    set_error(0);
    // SAFETY: plain call.
    let r = unsafe { SetProcessDpiAwarenessContext(ctx) };
    say!(t, "mode {}: SetProcessDpiAwarenessContext -> {} error {}", mode_label(m), r != 0, last_error());
}

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

fn primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

fn monitor_dpi_x() -> u32 {
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: writable outputs.
    unsafe { GetDpiForMonitor(primary(), 0, &mut x, &mut y) };
    x
}

impl R {
    fn to_string_eq(&self, o: &R) -> bool {
        self.0.left == o.0.left && self.0.top == o.0.top && self.0.right == o.0.right && self.0.bottom == o.0.bottom
    }
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The adapter and source of the first active path.
fn first_path() -> Option<([u8; 12], u32)> {
    let (mut np, mut nm) = (0u32, 0u32);
    // SAFETY: writable counts and buffers of the documented element sizes (72 and 64 bytes).
    unsafe {
        if GetDisplayConfigBufferSizes(2, &mut np, &mut nm) != 0 || np == 0 || np > 4 || nm > 8 {
            return None;
        }
        let mut paths = [0u8; 72 * 4];
        let mut modes = [0u8; 64 * 8];
        if QueryDisplayConfig(2, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), core::ptr::null_mut()) != 0 {
            return None;
        }
        let mut a = [0u8; 12];
        a.copy_from_slice(&paths[0..12]);
        Some((a, u32::from_le_bytes([paths[8], paths[9], paths[10], paths[11]])))
    }
}

/// The display's scale: (minimum, current, maximum) steps relative to the recommended one.
fn scale_get() -> Option<(i32, i32, i32)> {
    let (a, _) = first_path()?;
    let mut p = [0u8; 32];
    p[0..4].copy_from_slice(&(-3i32).to_le_bytes());
    p[4..8].copy_from_slice(&32u32.to_le_bytes());
    p[8..16].copy_from_slice(&a[0..8]);
    p[16..20].copy_from_slice(&a[8..12]);
    // SAFETY: a packet of the size the header says.
    if unsafe { DisplayConfigGetDeviceInfo(p.as_mut_ptr()) } != 0 {
        return None;
    }
    Some((i32_at(&p, 20), i32_at(&p, 24), i32_at(&p, 28)))
}

/// Sets the display's scale to a step relative to the recommended one; true when the call succeeded.
fn scale_set(rel: i32) -> bool {
    let Some((a, _)) = first_path() else { return false };
    let mut p = [0u8; 24];
    p[0..4].copy_from_slice(&(-4i32).to_le_bytes());
    p[4..8].copy_from_slice(&24u32.to_le_bytes());
    p[8..16].copy_from_slice(&a[0..8]);
    p[16..20].copy_from_slice(&a[8..12]);
    p[20..24].copy_from_slice(&rel.to_le_bytes());
    // SAFETY: a packet of the size the header says.
    unsafe { DisplayConfigSetDeviceInfo(p.as_ptr()) == 0 }
}

/// The DPI numbers a window of any kind can see now.
fn dpi_state(t: &mut Test, label: &str, wins: &[(&str, HWND)]) {
    // SAFETY: plain calls.
    unsafe {
        let dc = GetDC(core::ptr::null_mut());
        say!(
            t,
            "{label}: GetDpiForSystem {} LOGPIXELSX {} monitor dpi {} screen {}x{} CYCAPTION {}",
            GetDpiForSystem(),
            GetDeviceCaps(dc, LOGPIXELSX),
            monitor_dpi_x(),
            GetSystemMetrics(SM_CXSCREEN),
            GetSystemMetrics(SM_CYSCREEN),
            GetSystemMetrics(SM_CYCAPTION)
        );
        ReleaseDC(core::ptr::null_mut(), dc);
        for (n, h) in wins {
            say!(t, "  {n}: GetDpiForWindow {} rect {} client {}", GetDpiForWindow(*h), R(rect_of(*h)), R(client_of(*h)));
        }
    }
}

/// The scale changes by one step and back while windows are open; what messages arrive and what the DPI calls say.
fn sec_change(t: &mut Test) {
    // SAFETY: plain calls on windows of this test.
    unsafe {
        ENCS.get().0 = thread_ctx_name() == "per-monitor";
        let top = dpi_window(100, 100, 400, 300, WS_OVERLAPPEDWINDOW, 0, core::ptr::null_mut());
        let child = dpi_window(10, 10, 100, 50, WS_CHILD | WS_VISIBLE, 0, top);
        let pop = dpi_window(600, 150, 250, 120, WS_POPUP | WS_VISIBLE, 0, core::ptr::null_mut());
        let wins = [("top-level", top), ("child", child), ("popup", pop)];
        let Some((min, cur, max)) = scale_get() else {
            say!(t, "the display scale cannot be read");
            return;
        };
        say!(t, "scale steps: current {} (above the minimum by {}, below the maximum by {})", cur, cur - min, max - cur);
        log_clear();
        dpi_state(t, "before", &wins);
        let ok = scale_set(cur - 1);
        say!(t, "scale one step down -> {}", ok);
        settle(2500);
        flush_log(t, "msg");
        dpi_state(t, "one step down", &wins);
        log_clear();
        let ok = scale_set(cur);
        say!(t, "scale back up -> {}", ok);
        settle(2500);
        flush_log(t, "msg");
        dpi_state(t, "back", &wins);
        let (_, now, _) = scale_get().unwrap_or((0, 99, 0));
        say!(t, "the scale is the starting one again: {}", now == cur);
        DestroyWindow(pop);
        DestroyWindow(child);
        DestroyWindow(top);
        ENCS.get().0 = false;
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
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn GetDpiForSystem() -> u32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> BOOL;
    fn EnableNonClientDpiScaling(hwnd: HWND) -> BOOL;
    fn GetDisplayConfigBufferSizes(flags: u32, paths: *mut u32, modes: *mut u32) -> i32;
    fn QueryDisplayConfig(flags: u32, paths: *mut u32, path_buf: *mut u8, modes: *mut u32, mode_buf: *mut u8, topology: *mut u32) -> i32;
    fn DisplayConfigGetDeviceInfo(packet: *mut u8) -> i32;
    fn DisplayConfigSetDeviceInfo(packet: *const u8) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: HMONITOR, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

const MODE: u8 = b's';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    enter(t, MODE);
    sec_change(t);
}
