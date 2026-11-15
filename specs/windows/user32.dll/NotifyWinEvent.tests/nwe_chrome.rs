//! NotifyWinEvent as Chrome's browser UI thread uses it (S3: 105 calls with
//! no hook installed by anyone: accessibility events about its windows with
//! OBJID_CLIENT and custom ids; a hook-less call never fails): Chrome's three
//! windows are built; 105 notifications (focus, name change, state change,
//! accessibility state change, value change, show, hide, location change,
//! an application-defined event; ids OBJID_CLIENT, OBJID_WINDOW and custom
//! positive ids; the windows main, rwh, hid) are made while no hook exists
//! (the last error is checked after each call, nothing arrives); then one
//! observer hook over the object events and the application-defined range
//! is installed and the same kind of calls are made around show, minimize,
//! restore and close, together with the system's own events for the same
//! steps, so the explicit and the generated callbacks can be compared.
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
const SW_MINIMIZE: i32 = 6;
const SW_RESTORE: i32 = 9;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
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

/// winuser.h
#[repr(C)]
struct TRACKMOUSEEVENT {
    cb_size: u32,
    flags: u32,
    track: HWND,
    hover_time: u32,
}

const TME_LEAVE: u32 = 0x0000_0002;
const WM_MOUSELEAVE: u32 = 0x02A3;
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
const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
const WINEVENT_OUTOFCONTEXT: u32 = 0;
const EVENT_OBJECT_CREATE: u32 = 0x8000;
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
const EVENT_OBJECT_SHOW: u32 = 0x8002;
const EVENT_OBJECT_HIDE: u32 = 0x8003;
const EVENT_OBJECT_FOCUS: u32 = 0x8005;
const EVENT_OBJECT_STATECHANGE: u32 = 0x800A;
const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
const EVENT_OBJECT_ACCESSIBILITYSTATECHANGE: u32 = 0x8012;
const OBJID_WINDOW: i32 = 0;
const OBJID_CLIENT: i32 = -4;
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

const EVENT_OBJECT_VALUECHANGE: u32 = 0x800E;
const EVENT_OBJECT_END: u32 = 0x80FF;
const EVENT_AIA_START: u32 = 0xA000;
const EVENT_AIA_END: u32 = 0xAFFF;
/// The name of a documented event constant, or None.
fn ev_name_of(e: u32) -> Option<&'static str> {
    Some(match e {
        0x0001 => "SYSTEM_SOUND",
        0x0002 => "SYSTEM_ALERT",
        0x0003 => "SYSTEM_FOREGROUND",
        0x0004 => "SYSTEM_MENUSTART",
        0x0005 => "SYSTEM_MENUEND",
        0x0008 => "SYSTEM_CAPTURESTART",
        0x0009 => "SYSTEM_CAPTUREEND",
        0x000A => "SYSTEM_MOVESIZESTART",
        0x000B => "SYSTEM_MOVESIZEEND",
        0x0016 => "SYSTEM_MINIMIZESTART",
        0x0017 => "SYSTEM_MINIMIZEEND",
        0x8000 => "OBJECT_CREATE",
        0x8001 => "OBJECT_DESTROY",
        0x8002 => "OBJECT_SHOW",
        0x8003 => "OBJECT_HIDE",
        0x8004 => "OBJECT_REORDER",
        0x8005 => "OBJECT_FOCUS",
        0x8006 => "OBJECT_SELECTION",
        0x800A => "OBJECT_STATECHANGE",
        0x800B => "OBJECT_LOCATIONCHANGE",
        0x800C => "OBJECT_NAMECHANGE",
        0x800E => "OBJECT_VALUECHANGE",
        0x800F => "OBJECT_PARENTCHANGE",
        0x8012 => "OBJECT_ACCESSIBILITYSTATECHANGE",
        _ => return None,
    })
}

/// An event code as printed: its name, or the hexadecimal value.
struct EvName(u32);

impl core::fmt::Display for EvName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match ev_name_of(self.0) {
            Some(n) => f.write_str(n),
            None => write!(f, "{:#x}", self.0),
        }
    }
}

const EV_MAX_RECS: usize = 120;
/// One callback call: which hook (its label), the event, the window (a
/// name registered with `ev_known`), idObject, idChild, whether the event
/// thread is the test's thread and whether the callback ran in it.
#[derive(Clone, Copy)]
struct EvRec {
    label: u8,
    event: u32,
    win: u8,
    obj: i32,
    child: i32,
    ev_thread_ours: bool,
    cb_thread_ours: bool,
}

/// The recorder: windows and hooks known by name, the recorded calls.
struct EvLog {
    recs: [EvRec; EV_MAX_RECS],
    n: usize,
    lost: usize,
    total: u32,
    wins: [(usize, &'static str); 16],
    nwins: usize,
    hooks: [(usize, u8); 16],
    nhooks: usize,
    tid: u32,
    only_known: bool,
}

impl EvLog {
    const fn new() -> Self {
        EvLog {
            recs: [EvRec { label: 0, event: 0, win: 0, obj: 0, child: 0, ev_thread_ours: false, cb_thread_ours: false }; EV_MAX_RECS],
            n: 0,
            lost: 0,
            total: 0,
            wins: [(0, ""); 16],
            nwins: 0,
            hooks: [(0, 0); 16],
            nhooks: 0,
            tid: 0,
            only_known: true,
        }
    }
}

static EV: Global<EvLog> = Global::new(EvLog::new());
/// Start a recording: this thread is "ours", windows and hooks known so far
/// are forgotten, and only events of registered windows (and of null) are
/// recorded when `only_known` (events of the thread's other windows, such as
/// the default IME window, are then left out).
fn ev_reset(only_known: bool) {
    let l = EV.get();
    l.n = 0;
    l.lost = 0;
    l.total = 0;
    l.nwins = 0;
    l.nhooks = 0;
    l.only_known = only_known;
    // SAFETY: plain query.
    l.tid = unsafe { GetCurrentThreadId() };
}

/// Give a window a name for the recording (at most 16).
fn ev_known(h: HWND, name: &'static str) {
    let l = EV.get();
    if l.nwins < 16 {
        l.wins[l.nwins] = (h as usize, name);
        l.nwins += 1;
    }
}

/// Give a hook a one-letter label for the recording (at most 16).
fn ev_label(h: HHOOK, label: u8) {
    let l = EV.get();
    if l.nhooks < 16 {
        l.hooks[l.nhooks] = (h as usize, label);
        l.nhooks += 1;
    }
}

/// How many callbacks were recorded (those left out are not counted).
fn ev_count() -> usize {
    EV.get().n
}

/// How many callbacks ran at all since `ev_reset`, recorded or not.
fn ev_total() -> u32 {
    EV.get().total
}

/// Forget the recorded callbacks (names and labels stay).
fn ev_clear() {
    let l = EV.get();
    l.n = 0;
    l.lost = 0;
}

/// The callback of the hooks of these tests: records its arguments.
unsafe extern "system" fn ev_cb(hook: HHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, thread: u32, _time: u32) {
    let l = EV.get();
    l.total += 1;
    let mut win = if hwnd.is_null() { 1u8 } else { 255u8 };
    for i in 0..l.nwins {
        if l.wins[i].0 == hwnd as usize {
            win = 2 + i as u8;
        }
    }
    if win == 255 && l.only_known {
        return;
    }
    let mut label = b'?';
    for i in 0..l.nhooks {
        if l.hooks[i].0 == hook as usize {
            label = l.hooks[i].1;
        }
    }
    if l.n >= EV_MAX_RECS {
        l.lost += 1;
        return;
    }
    // SAFETY: plain query.
    let me = unsafe { GetCurrentThreadId() };
    l.recs[l.n] = EvRec { label, event, win, obj, child, ev_thread_ours: thread == l.tid, cb_thread_ours: me == l.tid };
    l.n += 1;
}

/// The recorded callbacks as lines "[label] EVENT win=name obj=.. child=..
/// threads=..", then cleared.
fn ev_dump(t: &mut Test, prefix: &str) {
    let l = EV.get();
    for i in 0..l.n {
        let r = l.recs[i];
        let name = match r.win {
            1 => "null",
            255 => "unknown",
            k => l.wins[(k - 2) as usize].1,
        };
        say!(
            t,
            "{prefix}[{}] {} win={} obj={} child={} event thread {} callback thread {}",
            r.label as char,
            EvName(r.event),
            name,
            r.obj,
            r.child,
            if r.ev_thread_ours { "ours" } else { "other" },
            if r.cb_thread_ours { "ours" } else { "other" }
        );
    }
    if l.n == 0 {
        say!(t, "{prefix}(no callbacks)");
    }
    if l.lost != 0 {
        say!(t, "{prefix}(more callbacks than the recorder holds)");
    }
    l.n = 0;
    l.lost = 0;
}

/// Install an out-of-context hook with `ev_cb` for this process and thread
/// (when `own_thread`; else for every thread of the process) over the event
/// range, and label it. Returns the handle (null when it failed).
fn ev_hook(min: u32, max: u32, own_thread: bool, flags: u32, label: u8) -> HHOOK {
    // SAFETY: ev_cb has the WINEVENTPROC signature; plain queries.
    let h = unsafe {
        SetWinEventHook(
            min,
            max,
            core::ptr::null_mut(),
            ev_cb as unsafe extern "system" fn(HHOOK, u32, HWND, i32, i32, u32, u32) as usize as *mut c_void,
            GetCurrentProcessId(),
            if own_thread { GetCurrentThreadId() } else { 0 },
            flags,
        )
    };
    if !h.is_null() {
        ev_label(h, label);
    }
    h
}

/// "nonzero" or "zero" for a handle (a handle is never printed).
fn hk(h: HHOOK) -> &'static str {
    if h.is_null() { "zero" } else { "nonzero" }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentThreadId() -> u32;
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
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, w: i32, h: i32, flags: u32) -> BOOL;
    fn GetParent(hwnd: HWND) -> HWND;
    fn SetParent(hwnd: HWND, parent: HWND) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn SetWindowTextW(hwnd: HWND, text: *const u16) -> BOOL;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn TrackMouseEvent(tme: *mut TRACKMOUSEEVENT) -> BOOL;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn NotifyWinEvent(event: u32, hwnd: HWND, object: i32, child: i32);
    fn SetWinEventHook(min: u32, max: u32, module: HINSTANCE, f: *mut c_void, pid: u32, tid: u32, flags: u32) -> HHOOK;
    fn UnhookWinEvent(hook: HHOOK) -> BOOL;
    fn LoadIconW(instance: HINSTANCE, name: *const u16) -> *mut c_void;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
}

poly_wtest::main!(test);

const MARK: u32 = 0x1234_5678;

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    ev_reset(true);
    let c = chrome_build(40, 40, 640, 400);
    ev_known(c.main, "main");
    ev_known(c.rwh, "rwh");
    ev_known(c.hid, "hid");
    // SAFETY: our windows and hook throughout.
    unsafe {
        ShowWindow(c.main, SW_SHOWNORMAL);
        SetWindowPos(c.rwh, core::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        SetFocus(c.main);
        settle(100);

        say!(t, "== 105 calls with no hook installed");
        let events = [
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_NAMECHANGE,
            EVENT_OBJECT_STATECHANGE,
            EVENT_OBJECT_ACCESSIBILITYSTATECHANGE,
            EVENT_OBJECT_VALUECHANGE,
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE,
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_AIA_START,
        ];
        let wins = [c.main, c.rwh, c.hid];
        let objs = [OBJID_CLIENT, OBJID_WINDOW, 1, 1000];
        let mut changed = 0;
        let mut calls = 0;
        ev_clear();
        for i in 0..105usize {
            set_error(MARK);
            NotifyWinEvent(events[i % 9], wins[i % 3], objs[(i / 3) % 4], (i % 5) as i32);
            calls += 1;
            if last_error() != MARK {
                changed += 1;
            }
            if i % 15 == 14 {
                settle(5);
            }
        }
        settle(60);
        say!(t, "  {calls} calls made, last error changed by {changed} of them, callbacks that arrived: {}", ev_total());

        say!(t, "== one observer hook (own process, every thread) over 0x8000..0x80FF and 0xA000..0xAFFF");
        let o = ev_hook(EVENT_OBJECT_CREATE, EVENT_OBJECT_END, false, WINEVENT_OUTOFCONTEXT, b'o');
        let p = ev_hook(EVENT_AIA_START, EVENT_AIA_END, false, WINEVENT_OUTOFCONTEXT, b'p');
        say!(t, "  hooks: {} {}", hk(o), hk(p));
        ev_clear();

        say!(t, "== explicit notifications on the visible windows");
        NotifyWinEvent(EVENT_OBJECT_FOCUS, c.main, OBJID_CLIENT, 0);
        NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, c.main, OBJID_WINDOW, 0);
        NotifyWinEvent(EVENT_OBJECT_STATECHANGE, c.rwh, OBJID_CLIENT, 0);
        NotifyWinEvent(EVENT_OBJECT_ACCESSIBILITYSTATECHANGE, c.main, OBJID_CLIENT, 0);
        NotifyWinEvent(EVENT_OBJECT_VALUECHANGE, c.rwh, 1000, 3);
        NotifyWinEvent(EVENT_AIA_START, c.main, OBJID_CLIENT, 0);
        NotifyWinEvent(EVENT_AIA_START + 1, c.hid, 7, 0);
        settle(60);
        ev_dump(t, "  ");

        say!(t, "== SetWindowTextW (a generated NAMECHANGE) followed by the same notification made explicitly");
        SetWindowTextW(c.main, wz!("New Tab - Google Chrome for Testing").as_ptr());
        NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, c.main, OBJID_WINDOW, 0);
        settle(60);
        ev_dump(t, "  ");

        say!(t, "== minimize, restore with a notification of each state change");
        ShowWindow(c.main, SW_MINIMIZE);
        NotifyWinEvent(EVENT_OBJECT_STATECHANGE, c.main, OBJID_WINDOW, 0);
        settle(100);
        ev_dump(t, "  ");
        ShowWindow(c.main, SW_RESTORE);
        NotifyWinEvent(EVENT_OBJECT_STATECHANGE, c.main, OBJID_WINDOW, 0);
        settle(100);
        ev_dump(t, "  ");

        say!(t, "== close: notify, hide, destroy, notify for the destroyed window");
        NotifyWinEvent(EVENT_OBJECT_HIDE, c.main, OBJID_WINDOW, 0);
        ShowWindow(c.main, SW_HIDE);
        DestroyWindow(c.rwh);
        NotifyWinEvent(EVENT_OBJECT_DESTROY, c.rwh, OBJID_CLIENT, 0);
        DestroyWindow(c.main);
        NotifyWinEvent(EVENT_AIA_START, c.main, OBJID_CLIENT, 0);
        DestroyWindow(c.hid);
        settle(100);
        ev_dump(t, "  ");

        say!(t, "== hooks removed, then one more call");
        UnhookWinEvent(o);
        UnhookWinEvent(p);
        set_error(MARK);
        NotifyWinEvent(EVENT_AIA_START, c.main, OBJID_CLIENT, 0);
        say!(t, "  last error unchanged: {}", (last_error() == MARK) as i32);
        settle(40);
        say!(t, "  callbacks: {}", ev_count());
    }
    settle(30);
}
