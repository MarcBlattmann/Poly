//! GetSystemMenu: the window menu of top-level windows. Which items it has
//! (by command id: SC_RESTORE, SC_MOVE, SC_SIZE, SC_MINIMIZE, SC_MAXIMIZE,
//! SC_CLOSE, separators) for windows with and without WS_SYSMENU, WS_THICKFRAME,
//! WS_MINIMIZEBOX and WS_MAXIMIZEBOX; their enabled or grayed state in the
//! normal, maximized and minimized states; a handle that stays the same, that
//! IsMenu accepts and that GetMenu does not return; changes of the menu and
//! GetSystemMenu(hwnd, TRUE) giving the default menu back (the call returns
//! NULL then); a child window, a null window and an invalid handle.
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

const WM_SYSCOMMAND: u32 = 0x0112;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const CS_NOCLOSE: u32 = 0x0200;
const SW_SHOWMINIMIZED: i32 = 2;
const SW_SHOWMAXIMIZED: i32 = 3;
const SW_RESTORE: i32 = 9;
const GWLP_USERDATA: i32 = -21;
const SC_CLOSE: usize = 0xF060;
const SC_MINIMIZE: usize = 0xF020;
const SC_MAXIMIZE: usize = 0xF030;
const SC_RESTORE: usize = 0xF120;
const SC_MOVE: usize = 0xF010;
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

const MF_BYCOMMAND: u32 = 0;
const MF_BYPOSITION: u32 = 0x400;
const MF_GRAYED: u32 = 1;
const MF_SEPARATOR: u32 = 0x800;
const MF_STRING: u32 = 0;
const SC_SIZE: usize = 0xF000;
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
    fn GetSystemMenu(hwnd: HWND, revert: BOOL) -> HMENU;
    fn GetMenuItemCount(menu: HMENU) -> i32;
    fn GetMenuItemID(menu: HMENU, pos: i32) -> u32;
    fn GetMenuState(menu: HMENU, id: u32, flags: u32) -> u32;
    fn IsMenu(menu: HMENU) -> BOOL;
    fn EnableMenuItem(menu: HMENU, id: u32, flags: u32) -> BOOL;
    fn AppendMenuW(menu: HMENU, flags: u32, id: usize, text: *const u16) -> BOOL;
    fn GetMenu(hwnd: HWND) -> HMENU;
}

poly_wtest::main!(test);

fn items(hwnd: HWND, out: &mut Buf<200>) {
    // SAFETY: a window of ours; the menu belongs to it.
    unsafe {
        let m = GetSystemMenu(hwnd, 0);
        if m.is_null() {
            let _ = write!(out, "no menu");
            return;
        }
        let n = GetMenuItemCount(m);
        for i in 0..n {
            let id = GetMenuItemID(m, i);
            let st = GetMenuState(m, i as u32, MF_BYPOSITION);
            let g = if id != 0 && st & MF_GRAYED != 0 { "g" } else { "" };
            let name = match id as usize {
                SC_RESTORE => "restore",
                SC_MOVE => "move",
                SC_SIZE => "size",
                SC_MINIMIZE => "minimize",
                SC_MAXIMIZE => "maximize",
                SC_CLOSE => "close",
                _ if id == 0 => "sep",
                _ => "other",
            };
            let _ = write!(out, "{name}{g} ");
        }
    }
}

fn show(t: &mut Test, what: &str, hwnd: HWND) {
    let mut b = Buf::<200>::new();
    items(hwnd, &mut b);
    say!(t, "{what}: {}", b.as_str().trim_end());
}

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    register(wz!("GsmClass"), Some(proc_def), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our own windows and menus.
    unsafe {
        let full = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        let w = create(0, wz!("GsmClass"), wz!("full"), full | WS_VISIBLE, 50, 50, 300, 200, none, 0);
        show(t, "caption, sysmenu, thick frame, both boxes, normal (items; g = grayed)", w);
        ShowWindow(w, SW_SHOWMAXIMIZED);
        show(t, "maximized", w);
        ShowWindow(w, SW_SHOWMINIMIZED);
        show(t, "minimized", w);
        ShowWindow(w, SW_RESTORE);
        show(t, "restored", w);

        for (what, style) in [
            ("no thick frame", WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX),
            ("no maximize box", WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX),
            ("no minimize box", WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX),
            ("no boxes", WS_CAPTION | WS_SYSMENU | WS_THICKFRAME),
            ("no WS_SYSMENU", WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX),
            ("popup with WS_SYSMENU only", WS_POPUP | WS_SYSMENU),
            ("plain popup", WS_POPUP),
            ("dialog frame", WS_CAPTION | WS_SYSMENU | WS_DLGFRAME),
        ] {
            let x = create(0, wz!("GsmClass"), wz!("x"), style | WS_VISIBLE, 60, 60, 300, 200, none, 0);
            show(t, what, x);
            DestroyWindow(x);
        }
        // tool window and a window with a class that closes disabled (CS_NOCLOSE)
        let x = create(WS_EX_TOOLWINDOW, wz!("GsmClass"), wz!("x"), full | WS_VISIBLE, 60, 60, 300, 200, none, 0);
        show(t, "tool window", x);
        DestroyWindow(x);
        register(wz!("GsmNoClose"), Some(proc_def), 0x0200, 0, core::ptr::null_mut());
        let x = create(0, wz!("GsmNoClose"), wz!("x"), full | WS_VISIBLE, 60, 60, 300, 200, none, 0);
        show(t, "class style CS_NOCLOSE", x);
        DestroyWindow(x);
        // hidden window: the same menu
        let x = create(0, wz!("GsmClass"), wz!("x"), full, 60, 60, 300, 200, none, 0);
        show(t, "hidden window", x);
        DestroyWindow(x);

        // the handle
        let m1 = GetSystemMenu(w, 0);
        let m2 = GetSystemMenu(w, 0);
        say!(t, "handle: not NULL {}, the same every time {}, IsMenu {}, GetMenu(window) is NULL {}, a popup menu of its own: IsMenu(m) after a sub query {}", !m1.is_null(), m1 == m2, IsMenu(m1), GetMenu(w).is_null(), IsMenu(m1));
        // revert: NULL, and the menu is the default again
        let n0 = GetMenuItemCount(m1);
        AppendMenuW(m1, MF_SEPARATOR, 0, core::ptr::null());
        AppendMenuW(m1, MF_STRING, 0x3000, wz!("Extra").as_ptr());
        let n1 = GetMenuItemCount(m1);
        EnableMenuItem(m1, SC_CLOSE as u32, MF_GRAYED | MF_BYCOMMAND);
        let closed = GetMenuState(m1, SC_CLOSE as u32, MF_BYCOMMAND) & MF_GRAYED != 0;
        say!(t, "after appending a separator and an item: {} items more, close grayed {closed}", n1 - n0);
        set_error(0xDEAD);
        let r = GetSystemMenu(w, 1);
        say!(t, "GetSystemMenu(window, TRUE) returns NULL: {}, last error left alone {}", r.is_null(), last_error() == 0xDEAD);
        let m3 = GetSystemMenu(w, 0);
        say!(t, "after the revert: {} items again, close not grayed {}, the old handle still a menu {}", GetMenuItemCount(m3) == n0, GetMenuState(m3, SC_CLOSE as u32, MF_BYCOMMAND) & MF_GRAYED == 0, IsMenu(m1));
        // a command of the menu arrives as WM_SYSCOMMAND: SC_CLOSE of a window whose procedure is the default one closes it (tested elsewhere); here only an id of our own
        // a child window
        let k = create(0, wz!("GsmClass"), wz!("kid"), WS_CHILD | WS_VISIBLE | WS_SYSMENU | WS_CAPTION, 5, 5, 100, 80, w, 1);
        set_error(0xDEAD);
        let m = GetSystemMenu(k, 0);
        say!(t, "a child window with WS_SYSMENU: menu {}, last error left alone {}", if m.is_null() { "NULL" } else { "returned" }, last_error() == 0xDEAD);
        // NULL and invalid handles
        set_error(0xDEAD);
        let m = GetSystemMenu(none, 0);
        say!(t, "NULL window: {} (last error {})", if m.is_null() { "NULL" } else { "menu" }, last_error());
        set_error(0xDEAD);
        let m = GetSystemMenu(0x1234usize as HWND, 0);
        say!(t, "invalid handle: {} (last error {})", if m.is_null() { "NULL" } else { "menu" }, last_error());
        set_error(0xDEAD);
        let m = GetSystemMenu(none, 1);
        say!(t, "NULL window with revert: {} (last error {})", if m.is_null() { "NULL" } else { "menu" }, last_error());
        // the desktop window and a window of another process (the taskbar)
        let m = GetSystemMenu(GetDesktopWindow(), 0);
        say!(t, "the desktop window: {}", if m.is_null() { "NULL" } else { "menu" });
        // a destroyed window's menu handle
        let x = create(0, wz!("GsmClass"), wz!("x"), full | WS_VISIBLE, 60, 60, 300, 200, none, 0);
        let mx = GetSystemMenu(x, 0);
        DestroyWindow(x);
        say!(t, "the menu of a destroyed window is no menu any more: {}", IsMenu(mx) == 0);
        DestroyWindow(w);
    }
}
