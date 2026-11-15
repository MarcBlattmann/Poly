//! DrawMenuBar on windows of several kinds with menus of several states:
//! a popup window, a message-only window, an overlapped window whose menu
//! was destroyed, the class menu of a window; no change to the menu items.
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
type HBITMAP = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
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
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const HWND_MESSAGE: HWND = -3isize as HWND;
const GWLP_USERDATA: i32 = -21;
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

const MIIM_STATE: u32 = 0x0001;
const MIIM_ID: u32 = 0x0002;
const MIIM_SUBMENU: u32 = 0x0004;
const MIIM_CHECKMARKS: u32 = 0x0008;
const MIIM_DATA: u32 = 0x0020;
const MIIM_STRING: u32 = 0x0040;
const MIIM_FTYPE: u32 = 0x0100;
/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MENUITEMINFOW {
    cb_size: u32,
    f_mask: u32,
    f_type: u32,
    f_state: u32,
    w_id: u32,
    h_sub_menu: HMENU,
    hbmp_checked: HBITMAP,
    hbmp_unchecked: HBITMAP,
    dw_item_data: usize,
    dw_type_data: *mut u16,
    cch: u32,
    hbmp_item: HBITMAP,
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn DestroyMenu(menu: HMENU) -> BOOL;
    fn GetMenuItemInfoW(menu: HMENU, item: u32, by_pos: BOOL, info: *mut MENUITEMINFOW) -> BOOL;
    fn GetMenuItemCount(menu: HMENU) -> i32;
    fn GetMenuStringW(menu: HMENU, id: u32, buf: *mut u16, max: i32, flags: u32) -> i32;
    fn SetMenu(hwnd: HWND, menu: HMENU) -> BOOL;
    fn DrawMenuBar(hwnd: HWND) -> BOOL;
    fn LoadMenuW(instance: HINSTANCE, name: *const u16) -> HMENU;
}

// ---- menu helpers of these tests ----

/// The text of an item as GetMenuStringW returns it: (return value, text).
fn mtext(menu: HMENU, item: u32, flags: u32) -> (i32, Buf<300>) {
    let mut buf = [0u16; 200];
    // SAFETY: buf has 200 units.
    let n = unsafe { GetMenuStringW(menu, item, buf.as_mut_ptr(), 200, flags) };
    let mut b = Buf::<300>::new();
    ascii_of(&buf, &mut b);
    (n, b)
}

fn mk_item(mask: u32) -> MENUITEMINFOW {
    let mut m: MENUITEMINFOW = zeroed();
    m.cb_size = core::mem::size_of::<MENUITEMINFOW>() as u32;
    m.f_mask = mask;
    m
}

/// One line per item: position, id (or sub), state flags, type flags, text.
fn mdump(t: &mut Test, label: &str, menu: HMENU) {
    // SAFETY: plain query.
    let n = unsafe { GetMenuItemCount(menu) };
    say!(t, "{label}: count {n}");
    for i in 0..n.max(0) {
        let mut buf = [0u16; 100];
        let mut m = mk_item(MIIM_ID | MIIM_STATE | MIIM_FTYPE | MIIM_SUBMENU | MIIM_STRING | MIIM_DATA | MIIM_CHECKMARKS);
        m.dw_type_data = buf.as_mut_ptr();
        m.cch = 100;
        // SAFETY: m describes buf.
        let ok = unsafe { GetMenuItemInfoW(menu, i as u32, 1, &mut m) };
        let mut b = Buf::<300>::new();
        ascii_of(&buf, &mut b);
        let id = if !m.h_sub_menu.is_null() { 0xFFFF_FFFF } else { m.w_id };
        say!(
            t,
            "  [{i}] ok={ok} id={id:#x} sub={} state={:#x} type={:#x} data={:#x} text='{}' cch={}",
            if m.h_sub_menu.is_null() { 0 } else { 1 },
            m.f_state,
            m.f_type,
            m.dw_item_data,
            b.as_str(),
            m.cch
        );
    }
}

/// "ok" or "err N" after a call that returned `r` (error read at once).
fn res(t: &mut Test, label: &str, r: i64) {
    let e = last_error();
    say!(t, "{label}: {r} err={e}");
}

/// Call `$e` with the last error set to a marker; print "label: result err=N"
/// (err -1: the call did not touch the last error). Gives the result.
macro_rules! call {
    ($t:expr, $label:expr, $e:expr) => {{
        set_error(0xDEAD);
        let r = ($e) as i64;
        let e = last_error();
        if r == 0 || r == -1 || stringify!($e).contains("is_null") {
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

core::arch::global_asm!(
    ".section .rsrc$01,\"dr\"\n",
    ".balign 4\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 4\n",
    ".long 2147483672\n",
    ".long 0, 0, 0\n",
    ".short 1, 2\n",
    ".long 2147483832\n",
    ".long 2147483712\n",
    ".long 100\n",
    ".long 2147483736\n",
    ".long 200\n",
    ".long 2147483760\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 136\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 152\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 168\n",
    ".rva .Lrsrc_d0\n",
    ".long 28, 0, 0\n",
    ".rva .Lrsrc_d1\n",
    ".long 238, 0, 0\n",
    ".rva .Lrsrc_d2\n",
    ".long 276, 0, 0\n",
    ".short 7\n",
    ".short 65, 80, 80, 77, 69, 78, 85\n",
    ".section .rsrc$02,\"dr\"\n",
    ".balign 4\n",
    ".Lrsrc_d0:\n",
    ".byte 0, 0, 0, 0, 144, 0, 78, 0, 97, 0, 109, 0, 101, 0, 100, 0, 0, 0, 128, 0, 145, 1, 110, 0\n",
    ".byte 49, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d1:\n",
    ".byte 0, 0, 0, 0, 16, 0, 38, 0, 70, 0, 105, 0, 108, 0, 101, 0, 0, 0, 0, 0, 45, 1, 38, 0\n",
    ".byte 79, 0, 112, 0, 101, 0, 110, 0, 9, 0, 67, 0, 116, 0, 114, 0, 108, 0, 43, 0, 79, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 8, 0, 46, 1, 67, 0, 104, 0, 101, 0, 99, 0, 107, 0, 101, 0, 100, 0\n",
    ".byte 0, 0, 1, 0, 47, 1, 71, 0, 114, 0, 97, 0, 121, 0, 101, 0, 100, 0, 0, 0, 2, 0, 48, 1\n",
    ".byte 68, 0, 105, 0, 115, 0, 97, 0, 98, 0, 108, 0, 101, 0, 100, 0, 0, 0, 128, 0, 49, 1, 69, 0\n",
    ".byte 38, 0, 120, 0, 105, 0, 116, 0, 0, 0, 16, 0, 38, 0, 69, 0, 100, 0, 105, 0, 116, 0, 0, 0\n",
    ".byte 0, 0, 55, 1, 67, 0, 117, 0, 116, 0, 0, 0, 144, 0, 77, 0, 111, 0, 114, 0, 101, 0, 0, 0\n",
    ".byte 0, 0, 65, 1, 68, 0, 101, 0, 101, 0, 112, 0, 0, 0, 192, 0, 66, 1, 68, 0, 101, 0, 101, 0\n",
    ".byte 112, 0, 101, 0, 114, 0, 0, 0, 32, 0, 75, 1, 66, 0, 97, 0, 114, 0, 98, 0, 114, 0, 101, 0\n",
    ".byte 97, 0, 107, 0, 0, 0, 128, 64, 76, 1, 38, 0, 72, 0, 101, 0, 108, 0, 112, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d2:\n",
    ".byte 1, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 244, 1, 0, 0, 0, 0, 80, 0\n",
    ".byte 108, 0, 97, 0, 105, 0, 110, 0, 0, 0, 0, 0, 0, 2, 0, 0, 8, 0, 0, 0, 245, 1, 0, 0\n",
    ".byte 0, 0, 82, 0, 97, 0, 100, 0, 105, 0, 111, 0, 32, 0, 99, 0, 104, 0, 101, 0, 99, 0, 107, 0\n",
    ".byte 101, 0, 100, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 3, 0, 0, 0, 246, 1, 0, 0, 0, 0, 71, 0, 114, 0, 97, 0, 121, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 16, 0, 0, 247, 1, 0, 0, 0, 0, 68, 0, 101, 0, 102, 0, 97, 0, 117, 0\n",
    ".byte 108, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 248, 1, 0, 0, 1, 0, 77, 0\n",
    ".byte 101, 0, 110, 0, 117, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 254, 1, 0, 0\n",
    ".byte 0, 0, 83, 0, 117, 0, 98, 0, 32, 0, 49, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 255, 1, 0, 0, 128, 0, 83, 0, 117, 0, 98, 0, 32, 0, 50, 0, 32, 0, 98, 0, 114, 0, 101, 0\n",
    ".byte 97, 0, 107, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 249, 1, 0, 0, 128, 0, 82, 0\n",
    ".byte 105, 0, 103, 0, 104, 0, 116, 0, 0, 0, 0, 0\n",
);

unsafe extern "system" fn wproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: default processing.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    let inst = instance();
    register(wz!("DmbBarCls"), Some(wproc), 0, 0, core::ptr::null_mut());
    let w = create(0, wz!("DmbBarCls"), wz!("w"), WS_OVERLAPPEDWINDOW, 0, 0, 200, 100, core::ptr::null_mut(), 0);
    let pw = create(0, wz!("DmbBarCls"), wz!("pw"), WS_POPUP, 0, 0, 100, 50, core::ptr::null_mut(), 0);
    let mw = create(0, wz!("DmbBarCls"), wz!("mw"), 0, 0, 0, 0, 0, HWND_MESSAGE, 0);
    let bar = unsafe { LoadMenuW(inst, mir(100)) };
    unsafe { SetMenu(w, bar) };
    call!(t, "overlapped window with a loaded menu", unsafe { DrawMenuBar(w) });
    say!(t, "items before {} after {}", unsafe { GetMenuItemCount(bar) }, { unsafe { DrawMenuBar(w) }; unsafe { GetMenuItemCount(bar) } });
    call!(t, "popup window without a menu", unsafe { DrawMenuBar(pw) });
    call!(t, "message-only window", unsafe { DrawMenuBar(mw) });
    unsafe { DestroyMenu(bar) };
    call!(t, "window whose menu was destroyed (a stale handle)", unsafe { DrawMenuBar(w) });
    unsafe { SetMenu(w, core::ptr::null_mut()) };
    call!(t, "after SetMenu(NULL)", unsafe { DrawMenuBar(w) });
    for h in [w, pw, mw] {
        unsafe { DestroyWindow(h) };
    }
    call!(t, "a destroyed window", unsafe { DrawMenuBar(w) });
}
