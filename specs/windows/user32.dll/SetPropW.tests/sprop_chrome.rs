//! Window properties the way Chrome's windows use them (S3): the browser
//! sets properties on its top-level window and its child, reads one back
//! (1 call), removes them as the windows go away (7 RemovePropW calls, two
//! of which fail to find their property). The test sets a property named
//! after the class on the holder, the top-level window and the child, reads
//! them back, removes them in the order they are set, then destroys the
//! windows; checks that a property is still readable from within
//! WM_DESTROY (before the window is gone) and from WM_NCDESTROY (properties
//! of the window are still there: printed as observed) and not afterwards.
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

const WM_DESTROY: u32 = 0x0002;
const WM_NCDESTROY: u32 = 0x0082;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn SetPropW(hwnd: HWND, name: *const u16, data: *mut c_void) -> BOOL;
    fn GetPropW(hwnd: HWND, name: *const u16) -> *mut c_void;
    fn RemovePropW(hwnd: HWND, name: *const u16) -> *mut c_void;
}

static SEEN_DESTROY: Global<i32> = Global::new(-1);
static SEEN_NCDESTROY: Global<i32> = Global::new(-1);

unsafe extern "system" fn proc_p(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: plain queries of the window being served.
    unsafe {
        if msg == WM_DESTROY {
            *SEEN_DESTROY.get() = GetPropW(hwnd, wz!("Chrome_NativeWindow").as_ptr()) as usize as i32;
        }
        if msg == WM_NCDESTROY {
            *SEEN_NCDESTROY.get() = GetPropW(hwnd, wz!("Chrome_NativeWindow").as_ptr()) as usize as i32;
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    t.check("registered", register(wz!("PropWin"), Some(proc_p), 0, 0, core::ptr::null_mut()) != 0, 0);
    // SAFETY: our windows throughout.
    unsafe {
        let np = core::ptr::null_mut();
        let a = create(0, wz!("PropWin"), &[], WS_OVERLAPPEDWINDOW, 40, 40, 300, 200, np, 0);
        let c = create(0, wz!("PropWin"), &[], WS_CHILD, 0, 0, 50, 50, a, 1);
        let h = create(WS_EX_TOOLWINDOW, wz!("PropWin"), &[], WS_POPUP, 0, 0, 0, 0, np, 0);
        for (label, w, v) in [("holder", h, 1usize), ("top-level", a, 2), ("child", c, 3)] {
            let r = SetPropW(w, wz!("Chrome_NativeWindow").as_ptr(), v as *mut c_void);
            say!(t, "SetPropW({label}) = {r}");
        }
        for (label, w) in [("holder", h), ("top-level", a), ("child", c)] {
            say!(t, "GetPropW({label}) = {}", GetPropW(w, wz!("Chrome_NativeWindow").as_ptr()) as usize);
        }
        // the property of the top-level window is not that of its child
        say!(t, "the child's own value is its own: {}", (GetPropW(c, wz!("Chrome_NativeWindow").as_ptr()) as usize == 3) as i32);
        for (label, w) in [("holder", h), ("top-level", a)] {
            set_error(0);
            let r = RemovePropW(w, wz!("Chrome_NativeWindow").as_ptr()) as usize;
            say!(t, "RemovePropW({label}) = {r} (last error {})", last_error());
        }
        set_error(0);
        let r = RemovePropW(a, wz!("Chrome_NativeWindow").as_ptr()) as usize;
        say!(t, "RemovePropW({}) a second time = {r} (last error {})", "top-level", last_error());
        // destroy with the child's property still set
        DestroyWindow(a);
        say!(t, "WM_DESTROY of the child saw the property: {}", *SEEN_DESTROY.get());
        say!(t, "WM_NCDESTROY saw the property: {}", *SEEN_NCDESTROY.get());
        set_error(0);
        let r = GetPropW(c, wz!("Chrome_NativeWindow").as_ptr()) as usize;
        say!(t, "GetPropW(child) after the destroy = {r} (last error {})", last_error());
        DestroyWindow(h);
    }
    settle(50);
}
