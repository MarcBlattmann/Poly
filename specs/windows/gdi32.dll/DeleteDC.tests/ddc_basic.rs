//! DeleteDC: a memory DC (TRUE once, then FALSE with a last error, and the
//! handle no longer has a type), NULL and a made-up handle, a bitmap
//! handle and a stock brush passed as a DC, the DC of an own window from
//! GetDC (a window DC is released with ReleaseDC; what DeleteDC says to it
//! and what ReleaseDC says afterwards), a DC from CreateDCW and CreateICW,
//! and DeleteObject on a DC handle. Then the objects of a DC: a bitmap that
//! is selected cannot be deleted until it is selected out; after DeleteDC a
//! bitmap, a brush and a pen that were selected are still valid, can be
//! selected into another DC and are deleted by us.
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
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
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

const WM_NCCREATE: u32 = 0x0081;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

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
const WHITE_BRUSH: i32 = 0;
const OBJ_PEN: u32 = 1;
const OBJ_BRUSH: u32 = 2;
const OBJ_MEMDC: u32 = 10;
const BITSPIXEL: i32 = 12;
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

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
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

/// The tag the next window created by `create_tagged` gets.
static NEXT_TAG: Global<u8> = Global::new(b'?');

/// `create` for a window whose procedure logs under `tag`.
fn create_tagged(
    tag: u8,
    ex: u32,
    class: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    *NEXT_TAG.get() = tag;
    let hwnd = create(ex, class, &[], style, x, y, w, h, parent, menu);
    *NEXT_TAG.get() = b'?';
    hwnd
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

/// A window procedure that only remembers the window's tag (the letter the
/// next `mkw` asked for) in GWLP_USERDATA and does the default handling.
unsafe extern "system" fn proc_tag(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: the window is being created on this thread.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, *NEXT_TAG.get() as isize) };
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Register the class `Tagged` of `mkw`.
fn tag_register() {
    register(wz!("Tagged"), Some(proc_tag), 0, 0, core::ptr::null_mut());
}

/// A window of the class `Tagged` with the tag letter `tag` (children get
/// the id 1).
fn mkw(tag: u8, ex: u32, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND) -> HWND {
    create_tagged(tag, ex, wz!("Tagged"), style, x, y, w, h, parent, if style & WS_CHILD != 0 { 1 } else { 0 })
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
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn CreatePen(style: i32, width: i32, color: COLORREF) -> HGDIOBJ;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
    fn CreateDCW(driver: *const u16, device: *const u16, port: *const u16, mode: *const c_void) -> HDC;
    fn CreateICW(driver: *const u16, device: *const u16, port: *const u16, mode: *const c_void) -> HDC;
}

poly_wtest::main!(test);

fn err(e: u32) -> u32 {
    if e == 0x5A5A { 0 } else { e }
}

fn test(t: &mut Test) {
    tag_register();
    // SAFETY: GDI objects and one window of this test.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        say!(t, "memory DC: type {} (OBJ_MEMDC is {})", GetObjectType(dc), OBJ_MEMDC);
        set_error(0x5A5A);
        let r1 = DeleteDC(dc);
        let e1 = last_error();
        set_error(0x5A5A);
        let r2 = DeleteDC(dc);
        let e2 = last_error();
        say!(t, "DeleteDC: first {r1} (error {}), second {r2} (error {}); type after {}; GetDeviceCaps after {}", err(e1), err(e2), GetObjectType(dc), GetDeviceCaps(dc, BITSPIXEL));

        for (name, h) in [("NULL", core::ptr::null_mut()), ("made-up handle", 0x00123456usize as HDC), ("stock brush", GetStockObject(WHITE_BRUSH) as HDC)] {
            set_error(0x5A5A);
            let r = DeleteDC(h);
            say!(t, "DeleteDC({name}): {r} (error {})", err(last_error()));
        }
        let bmp = CreateBitmap(4, 4, 1, 1, core::ptr::null());
        set_error(0x5A5A);
        let r = DeleteDC(bmp as HDC);
        say!(t, "DeleteDC(bitmap handle): {r} (error {}); the bitmap is still there: {}", err(last_error()), GetObjectType(bmp));
        DeleteObject(bmp);

        let win = mkw(b'A', 0, WS_OVERLAPPEDWINDOW, 40, 40, 200, 150, core::ptr::null_mut());
        let wdc = GetDC(win);
        say!(t, "GetDC(window): type {}", GetObjectType(wdc));
        set_error(0x5A5A);
        let r = DeleteDC(wdc);
        say!(t, "DeleteDC(window DC): {r} (error {})", err(last_error()));
        let again = GetDC(win);
        say!(t, "GetDC(window) again works {} ; ReleaseDC(window DC) {}", (!again.is_null()) as i32, ReleaseDC(win, again));
        say!(t, "ReleaseDC of the first, deleted one: {}", ReleaseDC(win, wdc));
        DestroyWindow(win);

        let disp = CreateDCW(wz!("DISPLAY").as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null());
        let ic = CreateICW(wz!("DISPLAY").as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null());
        say!(t, "DISPLAY DC type {} information DC type {}", GetObjectType(disp), GetObjectType(ic));
        say!(t, "DeleteDC(display) {} DeleteDC(information DC) {} again {} {}", DeleteDC(disp), DeleteDC(ic), DeleteDC(disp), DeleteDC(ic));

        let m = CreateCompatibleDC(core::ptr::null_mut());
        set_error(0x5A5A);
        let r = DeleteObject(m);
        say!(t, "DeleteObject(memory DC): {r} (error {}); DC still there: type {}", err(last_error()), GetObjectType(m));
        say!(t, "then DeleteDC {}", DeleteDC(m));

        // Objects of a DC.
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateBitmap(8, 8, 1, 1, core::ptr::null());
        let brush0 = CreateSolidBrush(rgb(1, 2, 3));
        let pen0 = CreatePen(0, 1, rgb(4, 5, 6));
        SelectObject(dc, bmp);
        SelectObject(dc, brush0);
        SelectObject(dc, pen0);
        say!(t, "selected: DeleteObject(bitmap) {} DeleteObject(brush) {} DeleteObject(pen) {}", DeleteObject(bmp), DeleteObject(brush0), DeleteObject(pen0));
        let brush = CreateSolidBrush(rgb(1, 2, 3));
        let pen = CreatePen(0, 1, rgb(4, 5, 6));
        SelectObject(dc, brush);
        SelectObject(dc, pen);
        say!(t, "DeleteDC with them selected: {}", DeleteDC(dc));
        say!(t, "the first brush and pen (deleted while selected): types {} {}", GetObjectType(brush0), GetObjectType(pen0));
        DeleteObject(brush0);
        DeleteObject(pen0);
        say!(t, "after: bitmap type {} brush type {} pen type {}", GetObjectType(bmp), GetObjectType(brush), GetObjectType(pen));
        let dc2 = CreateCompatibleDC(core::ptr::null_mut());
        let p = SelectObject(dc2, bmp);
        say!(t, "bitmap into another DC: previous type {}", GetObjectType(p));
        SelectObject(dc2, brush);
        SelectObject(dc2, pen);
        say!(t, "current in the other DC: brush is ours {} pen is ours {}", (GetCurrentObject(dc2, OBJ_BRUSH) == brush) as i32, (GetCurrentObject(dc2, OBJ_PEN) == pen) as i32);
        say!(t, "DeleteDC(other) {}", DeleteDC(dc2));
        say!(t, "DeleteObject {} {} {}", DeleteObject(bmp), DeleteObject(brush), DeleteObject(pen));
    }
    settle(20);
}
