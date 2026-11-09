//! A Windows GUI program for the desktop link's boot test (ADR 0131, ADR
//! 0136, boot group `desktop-win32`): it registers a class, creates an
//! overlapped window "Poly Fill" at (40, 400), 400 x 300, shows it, fills
//! its client area with one colour through GetDC / FillRect / ReleaseDC,
//! and keeps it up for a minute of messages before it destroys it and
//! ends. A left click turns the fill green (`CLICKED`); typed characters
//! are drawn at the top left, black on white, by its WM_PAINT handler. On
//! Poly's desktop the window shows up framed by Poly, its client area in
//! the colour (`FILL`), and repaints as the mouse and keyboard reach it.
//! Below the typed characters its WM_PAINT handler draws white text in
//! logical fonts (CreateFontW: Segoe UI, Arial bold, Times New Roman
//! italic, Courier New) with TextOutW and DrawTextW (ADR 0142), in the left
//! 240 pixels so that the points the boot test checks stay the fill.
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
const WM_PAINT: u32 = 0x000F;
const WM_CHAR: u32 = 0x0102;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_TIMER: u32 = 0x0113;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const SW_SHOW: i32 = 5;

/// The colour the client area is filled with: RGB(0x8A, 0x47, 0xC0), a
/// COLORREF (0x00BBGGRR).
const FILL: u32 = 0x00C0_478A;
/// The fill after a left click: RGB(0x22, 0xAA, 0x44).
const CLICKED: u32 = 0x0044_AA22;
/// `PAINTSTRUCT` (winuser.h): 72 bytes on x64.
#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    erase: BOOL,
    paint: RECT,
    restore: BOOL,
    inc_update: BOOL,
    reserved: [u8; 32],
}

/// What the window procedure keeps: the fill and the typed characters.
struct State {
    fill: u32,
    typed: [u16; 64],
    len: usize,
}

static mut STATE: State = State { fill: FILL, typed: [0; 64], len: 0 };

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
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn PostQuitMessage(code: i32);
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    fn InvalidateRect(hwnd: HWND, rect: *const RECT, erase: BOOL) -> BOOL;
    fn DrawTextW(hdc: HDC, text: *const u16, count: i32, rect: *mut RECT, format: u32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> HBRUSH;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        char_set: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> *mut c_void;
    fn SelectObject(hdc: HDC, obj: *mut c_void) -> *mut c_void;
    fn SetTextColor(hdc: HDC, color: u32) -> u32;
    fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    fn DeleteObject(obj: *mut c_void) -> BOOL;
    fn TextOutW(hdc: HDC, x: i32, y: i32, text: *const u16, len: i32) -> BOOL;
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: one thread runs the procedure; STATE is only touched here.
    let st = unsafe { &mut *core::ptr::addr_of_mut!(STATE) };
    match msg {
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        WM_LBUTTONDOWN => {
            st.fill = CLICKED;
            // SAFETY: our window.
            unsafe { InvalidateRect(hwnd, core::ptr::null(), 0) };
            return 0;
        }
        WM_CHAR => {
            if st.len < st.typed.len() && w >= 0x20 {
                st.typed[st.len] = w as u16;
                st.len += 1;
            }
            // SAFETY: our window.
            unsafe { InvalidateRect(hwnd, core::ptr::null(), 0) };
            return 0;
        }
        WM_PAINT => {
            // SAFETY: our window; the PAINTSTRUCT lives across the calls.
            unsafe {
                let mut ps: PAINTSTRUCT = core::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                GetClientRect(hwnd, &mut r);
                let brush = CreateSolidBrush(st.fill);
                FillRect(dc, &r, brush);
                DeleteObject(brush);
                if st.len > 0 {
                    TextOutW(dc, 10, 10, st.typed.as_ptr(), st.len as i32);
                }
                draw_samples(dc);
                EndPaint(hwnd, &ps);
            }
            return 0;
        }
        _ => {}
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

/// wingdi.h: TRANSPARENT, FW_NORMAL, FW_BOLD.
const TRANSPARENT: i32 = 1;
const FW_NORMAL: i32 = 400;
const FW_BOLD: i32 = 700;
/// winuser.h: DrawText formats.
const DT_CENTER: u32 = 0x0001;
const DT_WORDBREAK: u32 = 0x0010;
const DT_SINGLELINE: u32 = 0x0020;
const DT_END_ELLIPSIS: u32 = 0x8000;

/// White text in logical fonts below the typed characters: TextOutW in four
/// faces, then DrawTextW centred and wrapped, cut with an ellipsis, and
/// with mnemonics. Everything stays within x 10 to 240.
///
/// # Safety
/// `dc` is the DC of the window being painted.
unsafe fn draw_samples(dc: HDC) {
    // SAFETY: the caller's DC; every font is selected out and deleted again.
    unsafe {
        SetTextColor(dc, 0x00FF_FFFF);
        SetBkMode(dc, TRANSPARENT);
        let line = |y: i32, height: i32, weight: i32, italic: u32, face: &[u16], text: &[u16]| {
            let font = CreateFontW(height, 0, 0, 0, weight, italic, 0, 0, 1, 0, 0, 0, 0, face.as_ptr());
            let old = SelectObject(dc, font);
            TextOutW(dc, 10, y, text.as_ptr(), text.len() as i32 - 1);
            SelectObject(dc, old);
            DeleteObject(font);
        };
        line(40, -16, FW_NORMAL, 0, wz!("Segoe UI"), wz!("Segoe UI: Hello, Poly!"));
        line(62, -14, FW_BOLD, 0, wz!("Arial"), wz!("Arial bold: Windows text on Poly"));
        line(82, -16, FW_NORMAL, 1, wz!("Times New Roman"), wz!("Times New Roman italic"));
        line(104, -13, FW_NORMAL, 0, wz!("Courier New"), wz!("fn main() { 0xC0FFEE }"));
        let font = CreateFontW(-14, 0, 0, 0, FW_NORMAL, 0, 0, 0, 1, 0, 0, 0, 0, wz!("MS Shell Dlg").as_ptr());
        let old = SelectObject(dc, font);
        let mut r = RECT { left: 10, top: 134, right: 240, bottom: 190 };
        let text = wz!("Centred, wrapped text in a 230 pixel wide box with DrawTextW.");
        DrawTextW(dc, text.as_ptr(), -1, &mut r, DT_CENTER | DT_WORDBREAK);
        let mut r = RECT { left: 10, top: 200, right: 150, bottom: 218 };
        let text = wz!("A long caption that does not fit");
        DrawTextW(dc, text.as_ptr(), -1, &mut r, DT_SINGLELINE | DT_END_ELLIPSIS);
        let mut r = RECT { left: 10, top: 222, right: 240, bottom: 240 };
        let text = wz!("&File  &Edit  &View (mnemonics)");
        DrawTextW(dc, text.as_ptr(), -1, &mut r, DT_SINGLELINE);
        SelectObject(dc, old);
        DeleteObject(font);
    }
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
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    t.check("closed", true, 0);
}
