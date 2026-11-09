//! A Windows GUI program for the desktop link's boot test (ADR 0131, boot
//! group `desktop-win32`): it registers a class, creates an overlapped
//! window "Poly Fill" at (40, 400), 400 x 300, shows it, fills its client
//! area with one colour through GetDC / FillRect / ReleaseDC (WM_PAINT is
//! a later slice), and keeps it up for a minute of messages before it
//! destroys it and ends. On Poly's desktop the window shows up framed by
//! Poly, its client area in the colour (`FILL`).
#![no_std]
#![no_main]
#![allow(non_camel_case_types)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h and windef.h (constants,
// layouts).

use core::ffi::c_void;
use poly_wtest::*;

type HWND = *mut c_void;
type HDC = *mut c_void;
type HBRUSH = *mut c_void;
type HINSTANCE = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
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
const WM_TIMER: u32 = 0x0113;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const SW_SHOW: i32 = 5;

/// The colour the client area is filled with: RGB(0x8A, 0x47, 0xC0), a
/// COLORREF (0x00BBGGRR).
const FILL: u32 = 0x00C0_478A;
/// How long the window stays up.
const SHOW_MS: u32 = 60_000;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
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
        menu: *mut c_void,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
    fn SetTimer(hwnd: HWND, id: usize, ms: u32, func: *mut c_void) -> usize;
    fn GetMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn PostQuitMessage(code: i32);
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> HBRUSH;
    fn DeleteObject(obj: *mut c_void) -> BOOL;
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        // SAFETY: ends this thread's message loop.
        unsafe { PostQuitMessage(0) };
        return 0;
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let class = wz!("PolyFill");
    // SAFETY: NULL names the executable.
    let instance = unsafe { GetModuleHandleW(core::ptr::null()) };
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        wnd_proc: Some(window_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: class.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW with a static, NUL-terminated name.
    let atom = unsafe { RegisterClassExW(&wc) };
    t.check("class_registered", atom != 0, i64::from(atom));
    // SAFETY: NUL-terminated static names, no parent.
    let h = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            wz!("Poly Fill").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            40,
            400,
            400,
            300,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            instance,
            core::ptr::null_mut(),
        )
    };
    t.check("window_created", !h.is_null(), 0);
    if h.is_null() {
        return;
    }
    // SAFETY: our window; the RECT is writable; the DC is released and
    // the brush deleted before the loop.
    unsafe {
        ShowWindow(h, SW_SHOW);
        let mut r = RECT::default();
        t.check("client_rect", GetClientRect(h, &mut r) != 0 && r.right > 0 && r.bottom > 0, i64::from(r.right));
        let dc = GetDC(h);
        t.check("got_dc", !dc.is_null(), 0);
        let brush = CreateSolidBrush(FILL);
        t.check("filled", FillRect(dc, &r, brush) != 0, 0);
        t.check("released", ReleaseDC(h, dc) == 1, 0);
        DeleteObject(brush);
        SetTimer(h, 1, SHOW_MS, core::ptr::null_mut());
        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER && msg.hwnd == h {
                DestroyWindow(h);
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
    t.check("closed", true, 0);
}
