//! How Chrome's browser and GPU processes use stock objects: NULL_BRUSH
//! selected into a DC so that Rectangle only draws the outline (against
//! WHITE_BRUSH, which fills; the previous brush comes back from
//! SelectObject and is restored), DEFAULT_GUI_FONT read with GetObjectW and
//! copied with CreateFontIndirectW (the copy is a different object, the same
//! LOGFONT, deletable; the stock font is untouched), and the stock handle
//! seen from a second thread (the same object for the same index).
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
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type HFONT = *mut c_void;
type BOOL = i32;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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
const WAIT_OBJECT_0: u32 = 0;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const WHITE_BRUSH: i32 = 0;
const NULL_BRUSH: i32 = 5;
const BLACK_PEN: i32 = 7;
const SYSTEM_FONT: i32 = 13;
const DEFAULT_GUI_FONT: i32 = 17;
const OBJ_FONT: u32 = 6;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
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

/// The bounding box and count of pixels that are not `bg`.
fn ink(bits: *mut u32, w: i32, h: i32, bg: u32) -> (i32, i32, i32, i32, u32) {
    let (mut l, mut tp, mut r, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0);
    for y in 0..h {
        for x in 0..w {
            if px(bits, w, x, y) != bg {
                l = l.min(x);
                tp = tp.min(y);
                r = r.max(x + 1);
                b = b.max(y + 1);
                n += 1;
            }
        }
    }
    if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, r, b, n) }
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

/// wingdi.h (the A layout is not used; the W one has a 32-WCHAR face name)
#[repr(C)]
#[derive(Clone, Copy)]
struct LOGFONTW {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    out_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
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
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn Rectangle(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> BOOL;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
    fn GdiFlush() -> BOOL;
    fn CreateFontIndirectW(lf: *const LOGFONTW) -> HFONT;
}

poly_wtest::main!(test);

static SEEN: Global<[usize; 3]> = Global::new([0; 3]);

unsafe extern "system" fn other_thread(_: *mut c_void) -> u32 {
    // SAFETY: stock objects are process-wide; only reads.
    unsafe {
        let s = SEEN.get();
        s[0] = GetStockObject(NULL_BRUSH) as usize;
        s[1] = GetStockObject(DEFAULT_GUI_FONT) as usize;
        s[2] = GetStockObject(BLACK_PEN) as usize;
    }
    0
}

fn test(t: &mut Test) {
    // SAFETY: GDI objects and one thread of this test.
    unsafe {
        let (dc, bmp, bits) = dib32(8, 6, true);
        let bg = 0x0011_2233;
        let nb = GetStockObject(NULL_BRUSH);
        let wb = GetStockObject(WHITE_BRUSH);
        fill_bits(bits, 8, 6, bg);
        let prev = SelectObject(dc, nb);
        Rectangle(dc, 0, 0, 8, 6);
        GdiFlush();
        say!(t, "NULL_BRUSH: previous brush was WHITE_BRUSH {}; Rectangle: border {:06x} inside {:06x}", (prev == wb) as i32, px(bits, 8, 0, 0), px(bits, 8, 3, 3));
        fill_bits(bits, 8, 6, bg);
        let prev = SelectObject(dc, wb);
        Rectangle(dc, 0, 0, 8, 6);
        GdiFlush();
        say!(t, "WHITE_BRUSH: previous brush was NULL_BRUSH {}; Rectangle: border {:06x} inside {:06x}", (prev == nb) as i32, px(bits, 8, 0, 0), px(bits, 8, 3, 3));
        say!(t, "ink after the second Rectangle: {:?}", ink(bits, 8, 6, bg));

        let gui = GetStockObject(DEFAULT_GUI_FONT);
        let mut lf: LOGFONTW = zeroed();
        let r = GetObjectW(gui, core::mem::size_of::<LOGFONTW>() as i32, &mut lf as *mut LOGFONTW as *mut c_void);
        let copy = CreateFontIndirectW(&lf);
        let mut lf2: LOGFONTW = zeroed();
        let r2 = GetObjectW(copy, core::mem::size_of::<LOGFONTW>() as i32, &mut lf2 as *mut LOGFONTW as *mut c_void);
        say!(t, "stock font LOGFONT read {r}; copy created {} read {r2} type {}", (!copy.is_null()) as i32, GetObjectType(copy));
        say!(
            t,
            "copy: different handle {} same height {} weight {} charset {} quality {} face name {}",
            (copy != gui) as i32,
            (lf.height == lf2.height) as i32,
            (lf.weight == lf2.weight) as i32,
            (lf.char_set == lf2.char_set) as i32,
            (lf.quality == lf2.quality) as i32,
            (lf.face_name == lf2.face_name) as i32
        );
        let prev = SelectObject(dc, copy);
        say!(t, "select the copy: previous is SYSTEM_FONT {}; current is the copy {}", (prev == GetStockObject(SYSTEM_FONT)) as i32, (GetCurrentObject(dc, OBJ_FONT) == copy) as i32);
        say!(t, "delete the copy while selected: {}", DeleteObject(copy));
        SelectObject(dc, prev);
        say!(t, "delete the copy after unselecting: {} ; type {}", DeleteObject(copy), GetObjectType(copy));
        say!(t, "stock font after that: type {} same handle {}", GetObjectType(gui), (GetStockObject(DEFAULT_GUI_FONT) == gui) as i32);

        let mut id = 0u32;
        let th = CreateThread(core::ptr::null_mut(), 0, other_thread, core::ptr::null_mut(), 0, &mut id);
        let w = WaitForSingleObject(th, 10000);
        let s = SEEN.get();
        say!(
            t,
            "thread finished {}: NULL_BRUSH same {} DEFAULT_GUI_FONT same {} BLACK_PEN same {}",
            (w == WAIT_OBJECT_0) as i32,
            (s[0] == nb as usize) as i32,
            (s[1] == gui as usize) as i32,
            (s[2] == GetStockObject(BLACK_PEN) as usize) as i32
        );
        CloseHandle(th);
        say!(t, "DeleteDC {} ; DeleteObject {}", DeleteDC(dc), DeleteObject(bmp));
    }
}
