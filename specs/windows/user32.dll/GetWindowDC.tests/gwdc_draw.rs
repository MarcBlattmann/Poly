//! GetWindowDC and drawing: what is drawn with the window DC lands in the
//! window, outside the client area too; read back from the same DC with
//! GetPixel (a window of ours, on top, so nothing else covers it). Drawing in the
//! caption of a WS_EX_NOREDIRECTIONBITMAP-free window is overpainted by the
//! system only when it paints the frame; a window without a frame is used
//! here so that the test does not depend on theme colors. Also the clip: with the window region set, outside it
//! the DC has no pixels; clip rectangles of the DC and ExcludeClipRect; the
//! window DC is not clipped by child windows unless WS_CLIPCHILDREN; and a DC
//! used from a message of the window (WM_NCPAINT) versus outside.
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
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
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

const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOACTIVATE: u32 = 0x0010;
const GWL_STYLE: i32 = -16;
const GWLP_USERDATA: i32 = -21;
const SIMPLEREGION: i32 = 2;
const COMPLEXREGION: i32 = 3;
const CLR_INVALID: u32 = 0xFFFF_FFFF;
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

fn rect_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

fn client_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetClientRect(hwnd, &mut r) };
    r
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

type HRGN = *mut c_void;
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

/// A window procedure that does the default handling of everything.
unsafe extern "system" fn proc_def(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

const HWND_TOPMOST: HWND = -1isize as HWND;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
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
    fn UpdateWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, w: i32, h: i32, flags: u32) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetDC(hwnd: HWND) -> HDC;
    fn GetWindowDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetPixel(hdc: HDC, x: i32, y: i32) -> COLORREF;
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn SetPixelV(hdc: HDC, x: i32, y: i32, color: COLORREF) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn SelectClipRgn(hdc: HDC, rgn: HRGN) -> i32;
    fn ExcludeClipRect(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    register(wz!("GwdcDraw"), Some(proc_def), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our windows and their DCs.
    unsafe {
        // a thick-bordered popup: the border is non-client area, 4 pixels wide
        let w = create(0, wz!("GwdcDraw"), wz!("d"), WS_POPUP | WS_THICKFRAME | WS_VISIBLE, 100, 100, 200, 120, none, 0);
        SetWindowPos(w, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        UpdateWindow(w);
        let cr = client_of(w);
        let wr = rect_of(w);
        let frame = (wr.right - wr.left - cr.right) / 2;
        say!(t, "the thick frame is wider than 0 pixels: {}", frame > 0);
        let dc = GetWindowDC(w);
        let red = rgb(255, 0, 0);
        let blue = rgb(0, 0, 255);
        // a pixel in the frame (non-client) and one in the client area
        SetPixelV(dc, 1, 1, red);
        SetPixelV(dc, frame + 5, frame + 5, blue);
        say!(t, "pixel in the frame reads back red: {}, in the client area blue: {}", GetPixel(dc, 1, 1) == red, GetPixel(dc, frame + 5, frame + 5) == blue);
        // the same pixels through the client DC (origin shifted by the frame)
        let cdc = GetDC(w);
        say!(t, "the blue pixel through GetDC at (5,5): {}", GetPixel(cdc, 5, 5) == blue);
        say!(t, "a coordinate left of the client area through GetDC reads {} (outside the client clip: CLR_INVALID 0xFFFFFFFF)", if GetPixel(cdc, -2, -2) == 0xFFFF_FFFF { "CLR_INVALID" } else { "a color" });
        say!(t, "a coordinate outside the window through the window DC reads {}", if GetPixel(dc, -3, -3) == 0xFFFF_FFFF { "CLR_INVALID" } else { "a color" });
        ReleaseDC(w, cdc);
        // draw a filled rectangle with FillRect-free means: many pixels
        for x in 20..30 {
            SetPixelV(dc, x, 40, rgb(0, 255, 0));
        }
        let mut ok = true;
        for x in 20..30 {
            ok &= GetPixel(dc, x, 40) == rgb(0, 255, 0);
        }
        say!(t, "a row of ten green pixels reads back: {ok}");
        // a clip rectangle of the DC
        let clip_rgn = CreateRectRgn(0, 0, 50, 50);
        SelectClipRgn(dc, clip_rgn);
        let r = SetPixelV(dc, 10, 10, rgb(1, 2, 3));
        let r2 = SetPixelV(dc, 60, 60, rgb(1, 2, 3));
        say!(t, "with a clip region of 50x50: a pixel inside is set {}, one outside is not {}", r != 0, r2 == 0 || GetPixel(dc, 60, 60) != rgb(1, 2, 3));
        let mut box_ = RECT::default();
        GetClipBox(dc, &mut box_);
        say!(t, "clip box now {}x{} at {},{}", box_.right - box_.left, box_.bottom - box_.top, box_.left, box_.top);
        SelectClipRgn(dc, core::ptr::null_mut());
        DeleteObject(clip_rgn);
        let ex = ExcludeClipRect(dc, 0, 0, frame + 10, frame + 10);
        let mut box2 = RECT::default();
        GetClipBox(dc, &mut box2);
        say!(t, "after ExcludeClipRect of the top-left corner: region kind {ex} (COMPLEXREGION 3), a pixel in the corner is not set {}", SetPixelV(dc, 1, 1, rgb(9, 9, 9)) == 0);
        ReleaseDC(w, dc);
        // a child window above part of the frame: with and without WS_CLIPCHILDREN on the parent
        let k = create(0, wz!("GwdcDraw"), wz!("kid"), WS_CHILD | WS_VISIBLE, 10, 10, 60, 40, w, 1);
        UpdateWindow(k);
        let dc = GetWindowDC(w);
        let (kx, ky) = (frame + 20, frame + 20);
        SetPixelV(dc, kx, ky, rgb(200, 100, 50));
        say!(t, "a pixel under a child window, parent without WS_CLIPCHILDREN, window DC: set {}", GetPixel(dc, kx, ky) == rgb(200, 100, 50));
        ReleaseDC(w, dc);
        let st = GetWindowLongPtrW(w, GWL_STYLE);
        SetWindowLongPtrW(w, GWL_STYLE, st | WS_CLIPCHILDREN as isize);
        let dc = GetWindowDC(w);
        let mut b3 = RECT::default();
        let kind = GetClipBox(dc, &mut b3);
        say!(t, "with WS_CLIPCHILDREN set on the parent: the window DC is still one rectangle (region kind {kind}, SIMPLEREGION 2): {}", kind == 2);
        ReleaseDC(w, dc);
        DestroyWindow(k);
        DestroyWindow(w);
    }
}
