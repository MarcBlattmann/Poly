//! CreateCompatibleDC(NULL): a memory DC (OBJ_MEMDC) compatible with the
//! screen (BITSPIXEL/PLANES of the DC as observed) holding a 1x1
//! monochrome bitmap; default state (text black, background white,
//! OPAQUE, stock objects); CreateCompatibleBitmap of the memory DC gives a
//! monochrome bitmap (1 bpp) while of the screen DC it has the screen's
//! depth; CreateCompatibleDC of a memory DC and of a window DC work;
//! DeleteDC returns nonzero and the handle is no object any more.
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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
}

const WS_POPUP: u32 = 0x8000_0000;
const WHITE_BRUSH: i32 = 0;
const BLACK_PEN: i32 = 7;
const SYSTEM_FONT: i32 = 13;
const OBJ_PEN: u32 = 1;
const OBJ_BRUSH: u32 = 2;
const OBJ_FONT: u32 = 6;
const OBJ_BITMAP: u32 = 7;
const BITSPIXEL: i32 = 12;
const PLANES: i32 = 14;
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
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
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn GetTextColor(hdc: HDC) -> COLORREF;
    fn GetBkColor(hdc: HDC) -> COLORREF;
    fn GetBkMode(hdc: HDC) -> i32;
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HBITMAP;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

unsafe extern "system" fn proc_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn bitmap_info(t: &mut Test, label: &str, b: HBITMAP) {
    let mut bm = BITMAP::default();
    // SAFETY: bm is a writable BITMAP.
    let n = unsafe { GetObjectW(b, core::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut c_void) };
    say!(
        t,
        "{label}: GetObject={} {}x{} bpp={} planes={} widthbytes={} bits_null={}",
        n,
        bm.width,
        bm.height,
        bm.bits_pixel,
        bm.planes,
        bm.width_bytes,
        (bm.bits == 0) as i32
    );
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: GDI calls of this test.
    unsafe {
        let m = CreateCompatibleDC(core::ptr::null_mut());
        say!(
            t,
            "type={} bitspixel={} planes={} text={:#08x} bk={:#08x} mode={}",
            GetObjectType(m),
            GetDeviceCaps(m, BITSPIXEL),
            GetDeviceCaps(m, PLANES),
            GetTextColor(m),
            GetBkColor(m),
            GetBkMode(m)
        );
        say!(
            t,
            "stock objects: brush white {} pen black {} font system {}",
            (GetCurrentObject(m, OBJ_BRUSH) == GetStockObject(WHITE_BRUSH)) as i32,
            (GetCurrentObject(m, OBJ_PEN) == GetStockObject(BLACK_PEN)) as i32,
            (GetCurrentObject(m, OBJ_FONT) == GetStockObject(SYSTEM_FONT)) as i32
        );
        bitmap_info(t, "its bitmap", GetCurrentObject(m, OBJ_BITMAP));
        let mut clip = RECT::default();
        let k = GetClipBox(m, &mut clip);
        say!(t, "clip box kind={} {},{},{},{}", k, clip.left, clip.top, clip.right, clip.bottom);
        let cb = CreateCompatibleBitmap(m, 10, 5);
        bitmap_info(t, "CreateCompatibleBitmap(memory DC, 10, 5)", cb);
        let s = GetDC(core::ptr::null_mut());
        let sb = CreateCompatibleBitmap(s, 10, 5);
        bitmap_info(t, "CreateCompatibleBitmap(screen, 10, 5)", sb);
        let m2 = CreateCompatibleDC(m);
        say!(t, "compatible with a memory DC: type={}", GetObjectType(m2));
        ReleaseDC(core::ptr::null_mut(), s);
        t.check("registered", register(wz!("CcdBasic"), Some(proc_a), 0, 0, core::ptr::null_mut()) != 0, 0);
        let h = create(0, wz!("CcdBasic"), wz!(""), WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
        let wdc = GetDC(h);
        let m3 = CreateCompatibleDC(wdc);
        say!(t, "compatible with a window DC: type={} bitspixel={}", GetObjectType(m3), GetDeviceCaps(m3, BITSPIXEL));
        ReleaseDC(h, wdc);
        SelectObject(m, cb);
        let r = CreateCompatibleBitmap(m, 0, 0);
        say!(t, "CreateCompatibleBitmap 0x0: {}", hwnd_kind(r));
        if !r.is_null() {
            bitmap_info(t, "  it", r);
            DeleteObject(r);
        }
        say!(t, "DeleteDC: {} {} {}", DeleteDC(m), DeleteDC(m2), DeleteDC(m3));
        say!(t, "type after DeleteDC: {}", GetObjectType(m));
        set_error(0);
        say!(t, "DeleteDC again: {} error {}", DeleteDC(m), last_error());
        DeleteObject(cb);
        DeleteObject(sb);
        DestroyWindow(h);
    }
}
