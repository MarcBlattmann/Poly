//! CharUpperW in Chrome's browser process (S3; about 14000 calls in a Chrome
//! run: it upper-cases strings and single characters, e.g. window and
//! class names and typed characters, to compare them without case).
//! With the three windows built: the window title (SetWindowTextW, then
//! GetWindowTextW) and the three class names (GetClassNameW) are
//! upper-cased in place and printed; 14000 calls in a loop, half of them
//! the single-character form (ASCII letters and Latin-1 letters), half the
//! string form on the word "chrome" and on the title, are all checked
//! (the result is the expected one every time), and the number of messages
//! the window receives while the loop runs is printed (none are expected:
//! the call neither sends nor pumps messages); then the characters p o l y
//! typed into the window as virtual keys are upper-cased in the window's
//! WM_CHAR handler with the single-character form (one line per character)
//! and the collected text is upper-cased as a string.
//!
//! Keys are sent only while the test's window is in the foreground.
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
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_CHILDACTIVATE: u32 = 0x0022;
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
const GWLP_WNDPROC: i32 = -4;
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
const WM_MOUSELEAVE: u32 = 0x02A3;
const MAPVK_VK_TO_VSC: u32 = 0;
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

const VK_P: usize = 0x50;
const VK_O: usize = 0x4F;
const VK_L: usize = 0x4C;
const VK_Y: usize = 0x59;
const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
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

/// A character as the tests print it: printable ASCII in quotes, control
/// characters as U+XXXX, anything else (layout dependent) as "non-ASCII".
struct Ch(u32);

impl core::fmt::Display for Ch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            0x20..=0x7E => write!(f, "'{}'", self.0 as u8 as char),
            0..=0x7F => write!(f, "U+{:04X}", self.0),
            _ => f.write_str("non-ASCII"),
        }
    }
}

/// Write a wide string (up to its NUL or `n` characters) as ASCII, with
/// every other character as \uXXXX.
fn w_show(buf: &[u16], n: usize, out: &mut Buf<300>) {
    for &c in buf.iter().take(n) {
        if c == 0 {
            break;
        }
        if (0x20..0x7f).contains(&c) {
            let _ = out.write_char(c as u8 as char);
        } else {
            let _ = write!(out, "\\u{c:04X}");
        }
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
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, w: i32, h: i32, flags: u32) -> BOOL;
    fn ClientToScreen(hwnd: HWND, pt: *mut POINT) -> BOOL;
    fn GetParent(hwnd: HWND) -> HWND;
    fn SetParent(hwnd: HWND, parent: HWND) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn SetWindowTextW(hwnd: HWND, text: *const u16) -> BOOL;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn CallWindowProcW(proc_: WNDPROC, hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
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
    fn CharUpperW(s: *mut u16) -> *mut u16;
    fn LoadIconW(instance: HINSTANCE, name: *const u16) -> *mut c_void;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
}

static ORIG: Global<WNDPROC> = Global::new(None);
static SEEN: Global<u32> = Global::new(0);
static TYPED: Global<[u16; 16]> = Global::new([0; 16]);
static NTYPED: Global<usize> = Global::new(0);

unsafe extern "system" fn proc_sub(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    *SEEN.get() += 1;
    // SAFETY: plain calls on the window being served.
    unsafe {
        if msg == WM_CHAR {
            let up = CharUpperW(w as *mut u16) as usize;
            logln!("  WM_CHAR {} -> CharUpperW {}", Ch(w as u32), Ch(up as u32));
            let n = *NTYPED.get();
            if n < 15 {
                TYPED.get()[n] = w as u16;
                *NTYPED.get() = n + 1;
            }
        }
        CallWindowProcW(*ORIG.get(), hwnd, msg, w, l)
    }
}

fn shown(buf: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    w_show(buf, buf.len(), &mut b);
    b
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    let c = chrome_build(40, 40, 640, 400);
    // SAFETY: our windows; buffers are sized and NUL-terminated.
    unsafe {
        SetWindowPos(c.rwh, core::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        ShowWindow(c.main, SW_SHOWNORMAL);
        SetFocus(c.main);
        SetWindowTextW(c.main, wz!("Untitled - Google Chrome for Testing").as_ptr());
    }
    settle(200);

    say!(t, "== window text and class names, upper-cased in place");
    let mut title = [0u16; 128];
    // SAFETY: the buffer has 128 characters.
    let n = unsafe { GetWindowTextW(c.main, title.as_mut_ptr(), 128) };
    // SAFETY: the buffer is NUL-terminated and writable.
    let p = unsafe { CharUpperW(title.as_mut_ptr()) };
    say!(t, "  title ({n} characters): \"{}\" same pointer: {}", shown(&title[..n as usize]).as_str(), (p == title.as_mut_ptr()) as i32);
    for (name, h) in [("main", c.main), ("rwh", c.rwh), ("hid", c.hid)] {
        let mut cls = [0u16; 64];
        // SAFETY: the buffer has 64 characters.
        let n = unsafe { GetClassNameW(h, cls.as_mut_ptr(), 64) };
        // SAFETY: the buffer is NUL-terminated and writable.
        unsafe { CharUpperW(cls.as_mut_ptr()) };
        say!(t, "  class of {name} ({n} characters): \"{}\"", shown(&cls[..n as usize]).as_str());
    }

    say!(t, "== 14000 calls");
    // SAFETY: our window; the new procedure wraps the old one.
    unsafe {
        let old = SetWindowLongPtrW(c.main, GWLP_WNDPROC, proc_sub as *const () as usize as isize);
        *ORIG.get() = core::mem::transmute::<isize, WNDPROC>(old);
    }
    *SEEN.get() = 0;
    let (mut single_ok, mut string_ok, mut calls) = (0, 0, 0);
    for i in 0..7000u32 {
        let ch = if i % 2 == 0 { 0x61 + (i % 26) as u16 } else { let k = 0xE0 + (i % 30) as u16; if k >= 0xF7 { k + 1 } else { k } };
        let want = ch - 0x20;
        // SAFETY: a zero high word is a character.
        let r = unsafe { CharUpperW(ch as usize as *mut u16) } as usize;
        single_ok += (r == want as usize) as i32;
        let mut word = [0x63u16, 0x68, 0x72, 0x6F, 0x6D, 0x65, 0];
        // SAFETY: the buffer is NUL-terminated and writable.
        unsafe { CharUpperW(word.as_mut_ptr()) };
        string_ok += (word == [0x43, 0x48, 0x52, 0x4F, 0x4D, 0x45, 0]) as i32;
        calls += 2;
    }
    say!(t, "  {calls} calls: single-character results as expected: {single_ok} of 7000, strings as expected: {string_ok} of 7000");
    say!(t, "  messages received by the window during the loop: {}", *SEEN.get());

    let fg = make_foreground(c.main, 300, 50);
    say!(t, "main is the foreground window: {}", fg as i32);
    if fg {
        log_clear();
        say!(t, "== p o l y typed as virtual keys");
        for vk in [VK_P as i32, VK_O as i32, VK_L as i32, VK_Y as i32] {
            let n = tap(vk);
            say!(t, "  sent {n} events for vk {vk:#x}");
            settle(80);
        }
        flush_log(t, "");
        let mut text = *TYPED.get();
        let n = *NTYPED.get();
        // SAFETY: text has 16 characters and is NUL-terminated (at most 15 typed).
        unsafe { CharUpperW(text.as_mut_ptr()) };
        say!(t, "  {n} characters typed, upper-cased as a string: \"{}\"", shown(&text[..n]).as_str());
    }
    // SAFETY: our windows.
    unsafe {
        DestroyWindow(c.main);
        DestroyWindow(c.hid);
    }
    settle(100);
}
