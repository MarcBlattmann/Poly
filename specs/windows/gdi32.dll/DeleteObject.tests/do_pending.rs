//! The life of an object deleted while it is selected into a DC: what
//! GetObjectType, GetObject and GetCurrentObject say while it is selected,
//! after it is deselected (SelectObject of another object, or DeleteDC),
//! after a second DeleteObject and after a delay; and whether it can be
//! selected again. For a brush, a pen, a font and two kinds of bitmap.
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
type HFONT = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type COLORREF = u32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// wingdi.h (one RGBQUAD as a u32)
#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 1],
}

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
const PATCOPY: u32 = 0x00F0_0021;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const OBJ_PEN: u32 = 1;
const OBJ_BRUSH: u32 = 2;
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

/// A memory DC with a 32-bit top-down (or bottom-up) DIB section selected:
/// (dc, bitmap, pixels). Pixels are 0x00RRGGBB, row by row from the top
/// for top-down.
fn dib32(w: i32, h: i32, top_down: bool) -> (HDC, HBITMAP, *mut u32) {
    let info = BITMAPINFO {
        header: BITMAPINFOHEADER {
            size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            width: w,
            height: if top_down { -h } else { h },
            planes: 1,
            bit_count: 32,
            compression: BI_RGB,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(dc, bmp);
        (dc, bmp, bits as *mut u32)
    }
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// Set every pixel of a w x h 32-bit DIB to v.
fn fill_bits(bits: *mut u32, w: i32, h: i32, v: u32) {
    for i in 0..(w * h) as usize {
        // SAFETY: inside the DIB.
        unsafe { *bits.add(i) = v };
    }
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn CreatePen(style: i32, width: i32, color: COLORREF) -> HGDIOBJ;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HBITMAP;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
    fn PatBlt(hdc: HDC, x: i32, y: i32, w: i32, h: i32, rop: u32) -> BOOL;
    fn GdiFlush() -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontW(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        outp: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> HFONT;
    fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn LineTo(hdc: HDC, x: i32, y: i32) -> BOOL;
}

fn state(t: &mut Test, label: &str, h: HGDIOBJ) {
    let mut buf = [0u8; 128];
    // SAFETY: GetObjectW writes at most 128 bytes; h is a handle value.
    unsafe {
        set_error(0);
        let ty = GetObjectType(h);
        let n = GetObjectW(h, 128, buf.as_mut_ptr() as *mut c_void);
        say!(t, "  {label}: type {ty} GetObject {n}");
    }
}

fn kind_test(t: &mut Test, name: &str, make: &dyn Fn(HDC) -> HGDIOBJ, which: u32) {
    // SAFETY: GDI objects of this test, all released at the end.
    unsafe {
        let (dc, bmp, bits) = dib32(8, 8, true);
        let (dc2, bmp2, _bits2) = dib32(8, 8, true);
        say!(t, "{name}:");
        let o = make(dc);
        let prev = SelectObject(dc, o);
        say!(t, "  selected; DeleteObject {}", DeleteObject(o));
        state(t, "while selected", o);
        say!(t, "  GetCurrentObject is the object: {}", (GetCurrentObject(dc, which) == o) as i32);
        // A second delete while still selected.
        set_error(0);
        say!(t, "  second DeleteObject while selected: {} error {}", DeleteObject(o), last_error());
        // Draw: a pen draws a line, a brush fills.
        fill_bits(bits, 8, 8, 0x0010_2030);
        if which == OBJ_PEN {
            MoveToEx(dc, 0, 3, core::ptr::null_mut());
            LineTo(dc, 8, 3);
        } else {
            PatBlt(dc, 0, 0, 8, 8, PATCOPY);
        }
        GdiFlush();
        say!(t, "  draws: pixel {:06x}", px(bits, 8, 3, 3));
        // Select it into another DC while it is selected in the first.
        let in2 = SelectObject(dc2, o);
        say!(t, "  SelectObject into a second DC: previous null {}", in2.is_null() as i32);
        if !in2.is_null() {
            SelectObject(dc2, in2);
        }
        // Deselect.
        SelectObject(dc, prev);
        state(t, "after deselecting", o);
        set_error(0);
        say!(t, "  DeleteObject after deselecting: {} error {}", DeleteObject(o), last_error());
        state(t, "after that", o);
        settle(100);
        state(t, "100 ms later", o);
        // Can it be selected again?
        set_error(0);
        let again = SelectObject(dc, o);
        say!(t, "  SelectObject again: null {} error {}", again.is_null() as i32, last_error());
        fill_bits(bits, 8, 8, 0x0010_2030);
        if which == OBJ_PEN {
            MoveToEx(dc, 0, 3, core::ptr::null_mut());
            LineTo(dc, 8, 3);
        } else {
            PatBlt(dc, 0, 0, 8, 8, PATCOPY);
        }
        GdiFlush();
        say!(t, "  draws: pixel {:06x}", px(bits, 8, 3, 3));
        if !again.is_null() {
            SelectObject(dc, again);
        }
        DeleteDC(dc);
        DeleteDC(dc2);
        DeleteObject(bmp);
        DeleteObject(bmp2);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: the closures only create GDI objects.
    kind_test(t, "solid brush", &|_| unsafe { CreateSolidBrush(rgb(0x20, 0x40, 0x60)) }, OBJ_BRUSH);
    kind_test(t, "pen", &|_| unsafe { CreatePen(0, 1, rgb(0xE0, 0x10, 0x10)) }, OBJ_PEN);
    kind_test(t, "wide pen", &|_| unsafe { CreatePen(0, 3, rgb(0x10, 0xE0, 0x10)) }, OBJ_PEN);
    // SAFETY: GDI objects of this test.
    unsafe {
        let (dc, bmp, _bits) = dib32(4, 4, true);
        // Font.
        let face = wz!("Segoe UI");
        let font = CreateFontW(30, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, face.as_ptr());
        let prev = SelectObject(dc, font);
        say!(t, "font: selected; DeleteObject {}", DeleteObject(font));
        state(t, "while selected", font);
        SelectObject(dc, prev);
        state(t, "after deselecting", font);
        set_error(0);
        say!(t, "  DeleteObject after deselecting: {} error {}", DeleteObject(font), last_error());
        DeleteDC(dc);
        DeleteObject(bmp);
        // Bitmaps.
        for (label, dib) in [("compatible bitmap", false), ("DIB section", true)] {
            say!(t, "{label}:");
            let screen = GetDC(core::ptr::null_mut());
            let (dca, ba, _) = if dib { dib32(4, 4, true) } else { (CreateCompatibleDC(screen), CreateCompatibleBitmap(screen, 4, 4), core::ptr::null_mut()) };
            let b = if dib { ba } else { ba };
            let dcb = CreateCompatibleDC(screen);
            ReleaseDC(core::ptr::null_mut(), screen);
            let prev = if dib { core::ptr::null_mut() } else { SelectObject(dca, b) };
            say!(t, "  selected; DeleteObject {}", DeleteObject(b));
            state(t, "while selected", b);
            set_error(0);
            let busy = SelectObject(dcb, b);
            say!(t, "  SelectObject into another DC: null {} error {}", busy.is_null() as i32, last_error());
            if !dib {
                SelectObject(dca, prev);
            }
            state(t, "after deselecting", b);
            if dib {
                DeleteDC(dca);
                state(t, "after DeleteDC", b);
            }
            set_error(0);
            say!(t, "  DeleteObject again: {} error {}", DeleteObject(b), last_error());
            state(t, "after that", b);
            set_error(0);
            let reuse = SelectObject(dcb, b);
            say!(t, "  SelectObject afterwards: null {} error {}", reuse.is_null() as i32, last_error());
            settle(100);
            state(t, "100 ms later", b);
            if !dib {
                DeleteDC(dca);
            }
            DeleteDC(dcb);
        }
    }
}
