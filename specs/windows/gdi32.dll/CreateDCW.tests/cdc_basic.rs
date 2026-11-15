//! CreateDCW and CreateICW with different arguments: L"DISPLAY" with all
//! other arguments NULL, in other letter case, with a made-up device name,
//! with an empty driver name, a made-up driver name and a NULL driver name:
//! whether a DC comes back, its object type, the last error, DeleteDC once
//! and twice, and that two calls give different handles. For a DISPLAY DC
//! the default DC state (text colour, background colour and mode, ROP2, map
//! mode, whether the current brush, pen and font are the stock ones, the DC
//! origin, the clip box against the screen metrics) and the equality of its
//! capabilities with the screen DC's.
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

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

const GWLP_USERDATA: i32 = -21;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const WHITE_BRUSH: i32 = 0;
const BLACK_PEN: i32 = 7;
const OBJ_PEN: u32 = 1;
const OBJ_BRUSH: u32 = 2;
const OBJ_FONT: u32 = 6;
const OBJ_BITMAP: u32 = 7;
const BITSPIXEL: i32 = 12;
const PLANES: i32 = 14;
const LOGPIXELSX: i32 = 88;
const LOGPIXELSY: i32 = 90;
const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
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

const DC_TECHNOLOGY: i32 = 2;
const DC_NUMCOLORS: i32 = 24;
const DC_RASTERCAPS: i32 = 38;
const DC_COLORRES: i32 = 108;
const DC_DESKTOPVERTRES: i32 = 117;
const DC_DESKTOPHORZRES: i32 = 118;

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
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn GetTextColor(hdc: HDC) -> COLORREF;
    fn GetBkColor(hdc: HDC) -> COLORREF;
    fn GetBkMode(hdc: HDC) -> i32;
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
    fn CreateDCW(driver: *const u16, device: *const u16, port: *const u16, mode: *const c_void) -> HDC;
    fn CreateICW(driver: *const u16, device: *const u16, port: *const u16, mode: *const c_void) -> HDC;
    fn GetDCOrgEx(hdc: HDC, pt: *mut POINT) -> BOOL;
    fn GetROP2(hdc: HDC) -> i32;
    fn GetMapMode(hdc: HDC) -> i32;
}

poly_wtest::main!(test);

fn try_dc(t: &mut Test, what: &str, driver: *const u16, device: *const u16, ic: bool) {
    // SAFETY: the pointers are NUL-terminated strings or NULL.
    unsafe {
        set_error(0x5A5A);
        let dc = if ic {
            CreateICW(driver, device, core::ptr::null(), core::ptr::null())
        } else {
            CreateDCW(driver, device, core::ptr::null(), core::ptr::null())
        };
        let e = last_error();
        let k = if ic { "CreateICW" } else { "CreateDCW" };
        if dc.is_null() {
            say!(t, "{k} {what}: NULL (error unchanged {} / error {})", (e == 0x5A5A) as i32, if e == 0x5A5A { 0 } else { e });
        } else {
            let ty = GetObjectType(dc);
            let bpp = GetDeviceCaps(dc, BITSPIXEL);
            let d1 = DeleteDC(dc);
            let d2 = DeleteDC(dc);
            say!(t, "{k} {what}: DC type {ty} BITSPIXEL {bpp} ; DeleteDC {d1} again {d2}");
        }
    }
}

fn test(t: &mut Test) {
    let null = core::ptr::null();
    for ic in [false, true] {
        try_dc(t, "DISPLAY", wz!("DISPLAY").as_ptr(), null, ic);
        try_dc(t, "display (lower case)", wz!("display").as_ptr(), null, ic);
        try_dc(t, "DISPLAY with a made-up device name", wz!("NoSuchDevice").as_ptr(), wz!("NoSuchDevice").as_ptr(), ic);
        try_dc(t, "DISPLAY with device name \\\\.\\NoSuchDisplay", wz!("DISPLAY").as_ptr(), wz!("\\\\.\\NoSuchDisplay").as_ptr(), ic);
        try_dc(t, "empty driver name", wz!("").as_ptr(), null, ic);
        try_dc(t, "made-up driver name", wz!("NoSuchDriver").as_ptr(), null, ic);
        try_dc(t, "NULL driver and NULL device", null, null, ic);
    }
    // SAFETY: GDI objects of this test.
    unsafe {
        let a = CreateDCW(wz!("DISPLAY").as_ptr(), null, null, core::ptr::null());
        let b = CreateDCW(wz!("DISPLAY").as_ptr(), null, null, core::ptr::null());
        say!(t, "two DISPLAY DCs: both made {} different handles {}", (!a.is_null() && !b.is_null()) as i32, (a != b) as i32);
        DeleteDC(b);
        say!(t, "after deleting the second the first still answers: BITSPIXEL {}", GetDeviceCaps(a, BITSPIXEL));
        say!(t, "default state: text colour {:06x} bk colour {:06x} bk mode {} ROP2 {} map mode {}", GetTextColor(a), GetBkColor(a), GetBkMode(a), GetROP2(a), GetMapMode(a));
        say!(
            t,
            "current brush is WHITE_BRUSH {} pen is BLACK_PEN {} font is a font {} bitmap type {}",
            (GetCurrentObject(a, OBJ_BRUSH) == GetStockObject(WHITE_BRUSH)) as i32,
            (GetCurrentObject(a, OBJ_PEN) == GetStockObject(BLACK_PEN)) as i32,
            GetObjectType(GetCurrentObject(a, OBJ_FONT)),
            GetObjectType(GetCurrentObject(a, OBJ_BITMAP))
        );
        let mut org = POINT { x: 99, y: 99 };
        let r = GetDCOrgEx(a, &mut org);
        let mut clip = RECT::default();
        let c = GetClipBox(a, &mut clip);
        say!(t, "origin: {r} at {},{} ; clip box: type {c} left,top {},{} size equals the primary screen {} {}", org.x, org.y, clip.left, clip.top, (clip.right == GetSystemMetrics(SM_CXSCREEN)) as i32, (clip.bottom == GetSystemMetrics(SM_CYSCREEN)) as i32);
        let screen = GetDC(core::ptr::null_mut());
        let mut diff = Buf::<200>::new();
        for (name, i) in [
            ("BITSPIXEL", BITSPIXEL),
            ("PLANES", PLANES),
            ("LOGPIXELSX", LOGPIXELSX),
            ("LOGPIXELSY", LOGPIXELSY),
            ("HORZRES", HORZRES),
            ("VERTRES", VERTRES),
            ("NUMCOLORS", DC_NUMCOLORS),
            ("TECHNOLOGY", DC_TECHNOLOGY),
            ("RASTERCAPS", DC_RASTERCAPS),
            ("COLORRES", DC_COLORRES),
            ("DESKTOPHORZRES", DC_DESKTOPHORZRES),
            ("DESKTOPVERTRES", DC_DESKTOPVERTRES),
        ] {
            if GetDeviceCaps(a, i) != GetDeviceCaps(screen, i) {
                let _ = write!(diff, " {name}");
            }
        }
        say!(t, "capabilities that differ from GetDC(NULL):{}", if diff.n == 0 { " none" } else { diff.as_str() });
        ReleaseDC(core::ptr::null_mut(), screen);
        say!(t, "DeleteDC {}", DeleteDC(a));
    }
}
