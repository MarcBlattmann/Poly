//! DestroyCaret: removes the thread's caret; without a caret it fails; the
//! caret position, GetGUIThreadInfo and the other caret functions afterwards;
//! a bitmap given to CreateCaret survives; destroying the window that owns
//! the caret also removes the caret.
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
type HBITMAP = *mut c_void;
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

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
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
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn SetActiveWindow(hwnd: HWND) -> HWND;
    fn GetGUIThreadInfo(thread: u32, info: *mut GUITHREADINFO) -> BOOL;
    fn CreateCaret(hwnd: HWND, bitmap: HBITMAP, w: i32, h: i32) -> BOOL;
    fn DestroyCaret() -> BOOL;
    fn ShowCaret(hwnd: HWND) -> BOOL;
    fn HideCaret(hwnd: HWND) -> BOOL;
    fn SetCaretPos(x: i32, y: i32) -> BOOL;
    fn GetCaretPos(pt: *mut POINT) -> BOOL;
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

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    let w = caret_window(WS_OVERLAPPEDWINDOW, core::ptr::null_mut());
    call!(t, "DestroyCaret without a caret", unsafe { DestroyCaret() });
    call!(t, "CreateCaret 3 x 14", unsafe { CreateCaret(w, core::ptr::null_mut(), 3, 14) });
    call!(t, "SetCaretPos(20, 30)", unsafe { SetCaretPos(20, 30) });
    caret_info(t, "caret before DestroyCaret", w);
    call!(t, "DestroyCaret", unsafe { DestroyCaret() });
    caret_info(t, "after DestroyCaret", w);
    caret_pos(t, "GetCaretPos after DestroyCaret");
    call!(t, "SetCaretPos after DestroyCaret", unsafe { SetCaretPos(1, 1) });
    call!(t, "ShowCaret after DestroyCaret", unsafe { ShowCaret(w) });
    call!(t, "HideCaret after DestroyCaret", unsafe { HideCaret(w) });
    call!(t, "DestroyCaret a second time", unsafe { DestroyCaret() });
    // a new caret starts again at (0, 0)? (the position of the old one is gone)
    call!(t, "CreateCaret 3 x 14 again", unsafe { CreateCaret(w, core::ptr::null_mut(), 3, 14) });
    caret_pos(t, "position of the new caret");
    // the owner window is destroyed: the caret goes
    call!(t, "DestroyWindow of the owner", unsafe { DestroyWindow(w) });
    caret_info(t, "after DestroyWindow", core::ptr::null_mut());
    caret_pos(t, "GetCaretPos after DestroyWindow");
    call!(t, "DestroyCaret after DestroyWindow", unsafe { DestroyCaret() });
    // a window that is not the owner being destroyed leaves the caret
    let a = caret_window(WS_OVERLAPPEDWINDOW, core::ptr::null_mut());
    let b = caret_window(WS_OVERLAPPEDWINDOW, core::ptr::null_mut());
    call!(t, "CreateCaret in a", unsafe { CreateCaret(a, core::ptr::null_mut(), 2, 8) });
    call!(t, "DestroyWindow(b)", unsafe { DestroyWindow(b) });
    caret_info(t, "caret still in a", a);
    call!(t, "DestroyCaret", unsafe { DestroyCaret() });
    unsafe { DestroyWindow(a) };
}
