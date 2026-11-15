//! GetWindowDisplayAffinity (and SetWindowDisplayAffinity, to have something to
//! read): a new window reports WDA_NONE (0); WDA_MONITOR (1) and
//! WDA_EXCLUDEFROMCAPTURE (0x11) are read back; unknown values; child
//! windows; windows of other threads; NULL and invalid
//! handles; destroyed windows; the return value and the output parameter on
//! failure; and Chrome's use (asked twice for its top-level window right
//! after it is shown).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

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

const SW_HIDE: i32 = 0;
const SW_MINIMIZE: i32 = 6;
const GWLP_USERDATA: i32 = -21;
const ERROR_ACCESS_DENIED: u32 = 5;
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
const WDA_NONE: u32 = 0;
const WDA_MONITOR: u32 = 1;
const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;
/// A window procedure that does the default handling of everything.
unsafe extern "system" fn proc_def(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
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
    fn GetExitCodeThread(thread: HANDLE, code: *mut u32) -> BOOL;
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
    fn GetDesktopWindow() -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn GetWindowDisplayAffinity(hwnd: HWND, a: *mut u32) -> BOOL;
    fn SetWindowDisplayAffinity(hwnd: HWND, affinity: u32) -> BOOL;
}

poly_wtest::main!(test);

unsafe extern "system" fn other_thread(param: *mut c_void) -> u32 {
    // SAFETY: the window belongs to the thread that created it (the main one).
    unsafe {
        let w = *(param as *const usize) as HWND;
        let mut a = 0xAAu32;
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(w, &mut a);
        let ok = r != 0 && a == 0;
        let r2 = SetWindowDisplayAffinity(w, WDA_MONITOR);
        let e2 = last_error();
        (ok as u32) | ((r2 == 0) as u32) << 1 | ((e2 == ERROR_ACCESS_DENIED) as u32) << 2
    }
}

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    register(wz!("GwdaClass"), Some(proc_def), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our windows.
    unsafe {
        let w = create(WS_EX_NOREDIRECTIONBITMAP, wz!("GwdaClass"), wz!("a"), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 100, 100, 300, 200, none, 0);
        let mut a = 0xAAu32;
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(w, &mut a);
        say!(t, "a new window: returns {r}, affinity {a}, last error left alone {}", last_error() == 0xDEAD);
        let mut a2 = 0xBBu32;
        let r = GetWindowDisplayAffinity(w, &mut a2);
        say!(t, "asked again: returns {r}, affinity {a2}");

        for (name, v) in [("WDA_MONITOR", WDA_MONITOR), ("WDA_EXCLUDEFROMCAPTURE", WDA_EXCLUDEFROMCAPTURE), ("WDA_NONE", WDA_NONE)] {
            set_error(0xDEAD);
            let r = SetWindowDisplayAffinity(w, v);
            let mut g = 0xAAu32;
            let r2 = GetWindowDisplayAffinity(w, &mut g);
            say!(t, "Set {name}: returns {r}, Get returns {r2} with {g:#x}, last error left alone {}", last_error() == 0xDEAD);
        }
        for (name, v) in [("2", 2u32), ("0x11 again", 0x11)] {
            set_error(0xDEAD);
            let r = SetWindowDisplayAffinity(w, v);
            let e = last_error();
            let mut g = 0xAAu32;
            GetWindowDisplayAffinity(w, &mut g);
            say!(t, "Set {name}: returns {r}, last error {}, Get then gives {g:#x}", if e == 0xDEAD { 0 } else { e });
            SetWindowDisplayAffinity(w, WDA_NONE);
        }
        // child windows
        let k = create(0, wz!("GwdaClass"), wz!("k"), WS_CHILD | WS_VISIBLE, 10, 10, 50, 50, w, 1);
        let mut g = 0xAAu32;
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(k, &mut g);
        say!(t, "child window: Get returns {r}, affinity {g:#x}, last error {}", last_error());
        set_error(0xDEAD);
        let r = SetWindowDisplayAffinity(k, WDA_MONITOR);
        say!(t, "child window: Set returns {r}, last error {}", last_error());
        // NULL and invalid handles, destroyed window
        g = 0xAA;
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(none, &mut g);
        say!(t, "NULL window: returns {r}, output left alone {}, last error {}", g == 0xAA, last_error());
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(0x1234usize as HWND, &mut g);
        say!(t, "invalid handle: returns {r}, last error {}", last_error());
        let x = create(0, wz!("GwdaClass"), wz!("x"), WS_POPUP, 0, 0, 10, 10, none, 0);
        DestroyWindow(x);
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(x, &mut g);
        say!(t, "destroyed window: returns {r}, last error {}", last_error());
        set_error(0xDEAD);
        let r = GetWindowDisplayAffinity(w, core::ptr::null_mut());
        say!(t, "NULL output pointer: returns {r}, last error {}", last_error());
        // the desktop window and the taskbar of another process
        g = 0xAA;
        let r = GetWindowDisplayAffinity(GetDesktopWindow(), &mut g);
        say!(t, "the desktop window: returns {r}, affinity {g:#x}");
        // from another thread of the process
        let mut cell = w as usize;
        let h = CreateThread(core::ptr::null_mut(), 0, other_thread, &mut cell as *mut usize as *mut c_void, 0, core::ptr::null_mut());
        WaitForSingleObject(h, 10_000);
        let mut code = 0u32;
        GetExitCodeThread(h, &mut code);
        CloseHandle(h);
        say!(t, "from another thread of the process: Get works {}, Set of a window of the main thread fails {}", code & 1 != 0, code & 2 != 0);
        // the affinity belongs to the window: it does not change with visibility or state
        SetWindowDisplayAffinity(w, WDA_MONITOR);
        ShowWindow(w, SW_MINIMIZE);
        ShowWindow(w, SW_HIDE);
        let mut g = 0xAAu32;
        GetWindowDisplayAffinity(w, &mut g);
        say!(t, "affinity after minimize and hide: {g:#x}");
        DestroyWindow(k);
        DestroyWindow(w);
    }
}
