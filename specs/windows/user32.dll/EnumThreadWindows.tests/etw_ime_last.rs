//! EnumThreadWindows lists the thread's windows like EnumWindows: the z order of the walk, with the windows of the classes IME and MSCTFIME UI at the end. Printed next to the walk, in the same steps as gw_ime_zorder.
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

const WM_SIZE: u32 = 0x0005;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
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
const GW_HWNDNEXT: u32 = 2;
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

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
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

const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
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
    fn IsWindowVisible(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, w: i32, h: i32, flags: u32) -> BOOL;
    fn GetParent(hwnd: HWND) -> HWND;
    fn SetParent(hwnd: HWND, parent: HWND) -> HWND;
    fn GetWindow(hwnd: HWND, cmd: u32) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    fn GetTopWindow(hwnd: HWND) -> HWND;
    fn SetCapture(hwnd: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn TrackMouseEvent(tme: *mut TRACKMOUSEEVENT) -> BOOL;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn EnumThreadWindows(thread: u32, f: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>, l: LPARAM) -> BOOL;
    fn EnumWindows(f: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>, l: LPARAM) -> BOOL;
    fn LoadIconW(instance: HINSTANCE, name: *const u16) -> *mut c_void;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
}

#[link(name = "imm32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImmGetDefaultIMEWnd(hwnd: HWND) -> HWND;
}

poly_wtest::main!(test);

static TMP: Global<Buf<300>> = Global::new(Buf::<300>::new());
static ALL_IME_LAST: Global<(bool, bool, u32)> = Global::new((true, false, 0));

fn is_ime_class(h: HWND) -> bool {
    let r = role_of(h);
    r == "ime" || r == "ctf"
}

unsafe extern "system" fn cb_ours(h: HWND, _: LPARAM) -> BOOL {
    // SAFETY: plain queries.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid == GetCurrentProcessId() {
            let r = role_of(h);
            if r != "other" {
                let _ = write!(*TMP.get(), "{}{} ", r, if IsWindowVisible(h) == 0 { "~" } else { "" });
            }
        }
    }
    1
}

/// Over the whole list of the desktop: has any window of the IME classes (of any process) been seen
/// before a window that is not of them?
unsafe extern "system" fn cb_all(h: HWND, _: LPARAM) -> BOOL {
    let g = ALL_IME_LAST.get();
    if is_ime_class(h) {
        g.1 = true;
        g.2 += 1;
    } else if g.1 {
        g.0 = false;
    }
    1
}

fn walk_list() -> Buf<300> {
    let mut z = Buf::<300>::new();
    // SAFETY: plain queries.
    unsafe {
        let mut h = GetTopWindow(core::ptr::null_mut());
        let mut n = 0;
        let mut pid = 0u32;
        while !h.is_null() && n < 5000 {
            if GetWindowThreadProcessId(h, &mut pid) != 0 && pid == GetCurrentProcessId() {
                let r = role_of(h);
                if r != "other" {
                    let _ = write!(z, "{}{} ", r, if IsWindowVisible(h) == 0 { "~" } else { "" });
                }
            }
            h = GetWindow(h, GW_HWNDNEXT);
            n += 1;
        }
    }
    z
}

/// Is an IME-class window of any process in the walk before a window that is not of them?
fn walk_interleaves() -> bool {
    // SAFETY: plain queries.
    unsafe {
        let mut seen_ime = false;
        let mut h = GetTopWindow(core::ptr::null_mut());
        let mut n = 0;
        while !h.is_null() && n < 5000 {
            if is_ime_class(h) {
                seen_ime = true;
            } else if seen_ime {
                return true;
            }
            h = GetWindow(h, GW_HWNDNEXT);
            n += 1;
        }
    }
    false
}

fn show(t: &mut Test, what: &str, c: &Chrome) {
    // SAFETY: plain queries of windows of this thread.
    unsafe {
        let ime = ImmGetDefaultIMEWnd(c.main);
        let z = walk_list();
        *TMP.get() = Buf::<300>::new();
        EnumWindows(Some(cb_ours), 0);
        let mut ews = Buf::<300>::new();
        let _ = write!(ews, "{}", TMP.get().as_str().trim_end());
        *ALL_IME_LAST.get() = (true, false, 0);
        EnumWindows(Some(cb_all), 0);
        let (last, _, count) = *ALL_IME_LAST.get();
        *TMP.get() = Buf::<300>::new();
        EnumThreadWindows(GetCurrentThreadId(), Some(cb_ours), 0);
        say!(
            t,
            "{what}: EnumThreadWindows: [{}]; the walk of the z order: [{}]; in the enumeration the IME and MSCTFIME UI windows are last: {}",
            TMP.get().as_str().trim_end(),
            z.as_str().trim_end(),
            {
                let s = TMP.get().as_str().trim_end();
                let mut seen = false;
                let mut ok = true;
                for w in s.split(' ') {
                    let r = w.trim_end_matches('~');
                    if r == "ime" || r == "ctf" {
                        seen = true;
                    } else if seen {
                        ok = false;
                    }
                }
                ok
            }
        );
    }
}

unsafe fn scenario(t: &mut Test) {
    let c = chrome_build(40, 40, 640, 400);
    show(t, "built", &c);
    // SAFETY: our windows throughout.
    unsafe {
        SetWindowPos(c.rwh, core::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        ShowWindow(c.main, SW_SHOWNORMAL);
        settle(150);
        SetForegroundWindow(c.main);
        SetFocus(c.main);
        settle(150);
        show(t, "shown and active", &c);
        ShowWindow(c.main, SW_MINIMIZE);
        settle(150);
        show(t, "minimized", &c);
        ShowWindow(c.main, SW_RESTORE);
        settle(150);
        show(t, "restored", &c);
        DestroyWindow(c.main);
        settle(100);
        show(t, "top-level destroyed", &c);
        DestroyWindow(c.hid);
        settle(50);
    }
}

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    // SAFETY: our windows.
    unsafe { scenario(t) };
    say!(t, "(the same for the windows of one thread)");
}
