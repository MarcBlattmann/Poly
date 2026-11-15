//! What the stock objects are: GetObjectW of the stock brushes (LOGBRUSH
//! style and colour), pens (LOGPEN style and colour), fonts (LOGFONTW: the
//! size GetObjectW returns, the sign of the height, a short buffer, a NULL
//! buffer) and the default palette (the entry count); then SelectObject of
//! stock objects into a fresh memory DC: what comes back as the previous
//! object, what GetCurrentObject says, and the answers for a palette, a NULL
//! object and a NULL DC.
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
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type BOOL = i32;
type COLORREF = u32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LOGBRUSH {
    style: u32,
    color: COLORREF,
    hatch: usize,
}

const GWLP_USERDATA: i32 = -21;
const WHITE_BRUSH: i32 = 0;
const LTGRAY_BRUSH: i32 = 1;
const GRAY_BRUSH: i32 = 2;
const DKGRAY_BRUSH: i32 = 3;
const BLACK_BRUSH: i32 = 4;
const NULL_BRUSH: i32 = 5;
const WHITE_PEN: i32 = 6;
const BLACK_PEN: i32 = 7;
const NULL_PEN: i32 = 8;
const OEM_FIXED_FONT: i32 = 10;
const ANSI_FIXED_FONT: i32 = 11;
const ANSI_VAR_FONT: i32 = 12;
const SYSTEM_FONT: i32 = 13;
const DEVICE_DEFAULT_FONT: i32 = 14;
const DEFAULT_PALETTE: i32 = 15;
const SYSTEM_FIXED_FONT: i32 = 16;
const DEFAULT_GUI_FONT: i32 = 17;
const DC_BRUSH: i32 = 18;
const DC_PEN: i32 = 19;
const OBJ_PEN: u32 = 1;
const OBJ_BRUSH: u32 = 2;
const OBJ_FONT: u32 = 6;
const HGDI_ERROR: HGDIOBJ = -1isize as HGDIOBJ;
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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct LOGPEN {
    style: u32,
    width: POINT,
    color: COLORREF,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: stock objects and one memory DC of this test; every buffer is
    // at least as large as the size passed.
    unsafe {
        for i in [WHITE_BRUSH, LTGRAY_BRUSH, GRAY_BRUSH, DKGRAY_BRUSH, BLACK_BRUSH, NULL_BRUSH, DC_BRUSH] {
            let h = GetStockObject(i);
            let mut lb = LOGBRUSH::default();
            let size = core::mem::size_of::<LOGBRUSH>() as i32;
            let q = GetObjectW(h, 0, core::ptr::null_mut());
            let r = GetObjectW(h, size, &mut lb as *mut LOGBRUSH as *mut c_void);
            say!(t, "brush {i:2}: size query {q} read {r} style {} colour {:06x} hatch {}", lb.style, lb.color, lb.hatch);
        }
        for i in [WHITE_PEN, BLACK_PEN, NULL_PEN, DC_PEN] {
            let h = GetStockObject(i);
            let mut lp: LOGPEN = zeroed();
            let size = core::mem::size_of::<LOGPEN>() as i32;
            let q = GetObjectW(h, 0, core::ptr::null_mut());
            let r = GetObjectW(h, size, &mut lp as *mut LOGPEN as *mut c_void);
            say!(t, "pen {i:2}: size query {q} read {r} style {} width {},{} colour {:06x}", lp.style, lp.width.x, lp.width.y, lp.color);
        }
        for i in [OEM_FIXED_FONT, ANSI_FIXED_FONT, ANSI_VAR_FONT, SYSTEM_FONT, DEVICE_DEFAULT_FONT, SYSTEM_FIXED_FONT, DEFAULT_GUI_FONT] {
            let h = GetStockObject(i);
            let mut lf: LOGFONTW = zeroed();
            let size = core::mem::size_of::<LOGFONTW>() as i32;
            let q = GetObjectW(h, 0, core::ptr::null_mut());
            let r = GetObjectW(h, size, &mut lf as *mut LOGFONTW as *mut c_void);
            let mut part = [0u8; 8];
            let p = GetObjectW(h, 4, part.as_mut_ptr() as *mut c_void);
            let mut lf2: LOGFONTW = zeroed();
            let big = GetObjectW(h, 1000, &mut lf2 as *mut LOGFONTW as *mut c_void);
            say!(
                t,
                "font {i:2}: size query {q} read {r} 4-byte read {p} (height field set {}) oversize read {big}; height sign {}; weight>=400 {}; escapement {}",
                (i32::from_le_bytes([part[0], part[1], part[2], part[3]]) != 0) as i32,
                lf.height.signum(),
                (lf.weight >= 400) as i32,
                lf.escapement
            );
        }
        let pal = GetStockObject(DEFAULT_PALETTE);
        let mut n: u16 = 0xFFFF;
        let q = GetObjectW(pal, 0, core::ptr::null_mut());
        let r = GetObjectW(pal, 2, &mut n as *mut u16 as *mut c_void);
        say!(t, "default palette: type {} size query {q} read {r} entries {n}", GetObjectType(pal));

        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let wb = GetStockObject(WHITE_BRUSH);
        let nb = GetStockObject(NULL_BRUSH);
        let bb = GetStockObject(BLACK_BRUSH);
        let prev = SelectObject(dc, nb);
        say!(t, "select NULL_BRUSH: previous is WHITE_BRUSH {} ; current is NULL_BRUSH {}", (prev == wb) as i32, (GetCurrentObject(dc, OBJ_BRUSH) == nb) as i32);
        let prev = SelectObject(dc, bb);
        say!(t, "select BLACK_BRUSH: previous is NULL_BRUSH {}", (prev == nb) as i32);
        let prev = SelectObject(dc, bb);
        say!(t, "select BLACK_BRUSH again: previous is BLACK_BRUSH {}", (prev == bb) as i32);
        let wp = GetStockObject(WHITE_PEN);
        let kp = GetStockObject(BLACK_PEN);
        let prev = SelectObject(dc, wp);
        say!(t, "select WHITE_PEN: previous is BLACK_PEN {} ; current is WHITE_PEN {}", (prev == kp) as i32, (GetCurrentObject(dc, OBJ_PEN) == wp) as i32);
        let sf = GetStockObject(SYSTEM_FONT);
        let gf = GetStockObject(DEFAULT_GUI_FONT);
        let prev = SelectObject(dc, gf);
        say!(t, "select DEFAULT_GUI_FONT: previous is SYSTEM_FONT {} ; current is it {}", (prev == sf) as i32, (GetCurrentObject(dc, OBJ_FONT) == gf) as i32);
        let prev = SelectObject(dc, sf);
        say!(t, "select SYSTEM_FONT: previous is DEFAULT_GUI_FONT {}", (prev == gf) as i32);
        set_error(0x5A5A);
        let p = SelectObject(dc, pal);
        say!(t, "select DEFAULT_PALETTE with SelectObject: null {} HGDI_ERROR {} error unchanged {}", p.is_null() as i32, (p == HGDI_ERROR) as i32, (last_error() == 0x5A5A) as i32);
        set_error(0x5A5A);
        let p = SelectObject(dc, core::ptr::null_mut());
        say!(t, "select NULL object: null {} HGDI_ERROR {} error unchanged {}", p.is_null() as i32, (p == HGDI_ERROR) as i32, (last_error() == 0x5A5A) as i32);
        let p = SelectObject(core::ptr::null_mut(), bb);
        say!(t, "select into NULL DC: null {} HGDI_ERROR {}", p.is_null() as i32, (p == HGDI_ERROR) as i32);
        say!(t, "after those: brush is BLACK_BRUSH {} pen is WHITE_PEN {}", (GetCurrentObject(dc, OBJ_BRUSH) == bb) as i32, (GetCurrentObject(dc, OBJ_PEN) == wp) as i32);
        say!(t, "DeleteDC {}", DeleteDC(dc));
    }
}
