//! SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, SPI_GETICONMETRICS,
//! SPI_GETWORKAREA, SPI_GETANIMATION, SPI_GETHIGHCONTRAST) on the oracle: the
//! metrics equal GetSystemMetrics at the system DPI (border 1, scroll 17,
//! caption 36x22 buttons, menu 19, padded 4), the five message/menu/status/
//! caption fonts are Segoe UI 400 height -12, the work area is 0,24,1920,1080,
//! SPI_GETANIMATION fills cbSize and MinAnimate (1), the high-contrast flags
//! are the default scheme's; a success may leave the last error alone or
//! clear it (the output prints -1 for both).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

const SM_CXBORDER: i32 = 5;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CXVSCROLL: i32 = 2;
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

const SM_CYSIZE: i32 = 31;
const SM_CYSMSIZE: i32 = 53;
const SM_CYMENUSIZE: i32 = 55;
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

/// winuser.h LOGFONTW
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

/// winuser.h NONCLIENTMETRICSW
#[repr(C)]
#[derive(Clone, Copy)]
struct NONCLIENTMETRICSW {
    cb_size: u32,
    border_width: i32,
    scroll_width: i32,
    scroll_height: i32,
    caption_width: i32,
    caption_height: i32,
    caption_font: LOGFONTW,
    sm_caption_width: i32,
    sm_caption_height: i32,
    sm_caption_font: LOGFONTW,
    menu_width: i32,
    menu_height: i32,
    menu_font: LOGFONTW,
    status_font: LOGFONTW,
    message_font: LOGFONTW,
    padded_border_width: i32,
}

/// winuser.h ICONMETRICSW
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONMETRICSW {
    cb_size: u32,
    horz_spacing: i32,
    vert_spacing: i32,
    title_wrap: i32,
    font: LOGFONTW,
}

/// The last error of a call for printing: -1 when the call succeeded and
/// left it as it was (0x1234, set before) or cleared (0): a success does
/// not promise either.
fn dp_e(ok: i32, e: u32) -> i64 {
    if ok != 0 && (e == 0 || e == 0x1234) { -1 } else { e as i64 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sz = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    // SAFETY: structures are as large as passed.
    unsafe {
        let mut wa = RECT::default();
        set_error(0x1234);
        let ok = SystemParametersInfoW(0x30, 0, &mut wa as *mut _ as *mut c_void, 0);
        say!(t, "SPI_GETWORKAREA: ok {ok} work area {} last error {}", R(wa), dp_e(ok, last_error()));
        let mut n: NONCLIENTMETRICSW = zeroed();
        n.cb_size = sz;
        set_error(0x1234);
        let ok = SystemParametersInfoW(0x29, sz, &mut n as *mut _ as *mut c_void, 0);
        say!(t, "SPI_GETNONCLIENTMETRICS: ok {ok} last error {} border {} scroll {}x{} caption {}x{} small caption {}x{} menu {}x{} padded {}", dp_e(ok, last_error()), n.border_width, n.scroll_width, n.scroll_height, n.caption_width, n.caption_height, n.sm_caption_width, n.sm_caption_height, n.menu_width, n.menu_height, n.padded_border_width);
        say!(t, "  equal to GetSystemMetrics: border {} scroll {} caption height {} menu {} padded {} small caption {}", n.border_width == GetSystemMetrics(SM_CXBORDER), n.scroll_width == GetSystemMetrics(SM_CXVSCROLL), n.caption_height == GetSystemMetrics(SM_CYSIZE), n.menu_height == GetSystemMetrics(SM_CYMENUSIZE), n.padded_border_width == GetSystemMetrics(SM_CXPADDEDBORDER), n.sm_caption_height == GetSystemMetrics(SM_CYSMSIZE));
        for (name, f) in [("caption", &n.caption_font), ("small caption", &n.sm_caption_font), ("menu", &n.menu_font), ("status", &n.status_font), ("message", &n.message_font)] {
            let mut face = Buf::<300>::new();
            let l = f.face_name.iter().position(|&c| c == 0).unwrap_or(32);
            ascii_of(&f.face_name[..l], &mut face);
            say!(t, "  {name} font: height {} width {} weight {} italic {} charset {} quality {} pitch/family {:#x} face {}", f.height, f.width, f.weight, f.italic, f.char_set, f.quality, f.pitch_and_family, face.as_str());
        }
        let mut im: ICONMETRICSW = zeroed();
        im.cb_size = core::mem::size_of::<ICONMETRICSW>() as u32;
        set_error(0x1234);
        let ok = SystemParametersInfoW(0x2D, im.cb_size, &mut im as *mut _ as *mut c_void, 0);
        say!(t, "SPI_GETICONMETRICS (cbSize {}): ok {ok} last error {} horizontal {} vertical {} wrap {} font height {}", im.cb_size, dp_e(ok, last_error()), im.horz_spacing, im.vert_spacing, im.title_wrap, im.font.height);
        let mut an = [0u8; 8];
        an[..4].copy_from_slice(&8u32.to_le_bytes());
        set_error(0x1234);
        let ok = SystemParametersInfoW(0x48, 8, an.as_mut_ptr() as *mut c_void, 0);
        say!(t, "SPI_GETANIMATION: ok {ok} last error {:#x} cbSize {} MinAnimate {}", last_error(), u32::from_le_bytes([an[0], an[1], an[2], an[3]]), u32::from_le_bytes([an[4], an[5], an[6], an[7]]));
        let mut hc = [0u8; 16];
        hc[..4].copy_from_slice(&16u32.to_le_bytes());
        set_error(0x1234);
        let ok = SystemParametersInfoW(0x42, 16, hc.as_mut_ptr() as *mut c_void, 0);
        say!(t, "SPI_GETHIGHCONTRAST: ok {ok} last error {:#x} cbSize {} flags {:#x}", last_error(), u32::from_le_bytes([hc[0], hc[1], hc[2], hc[3]]), u32::from_le_bytes([hc[4], hc[5], hc[6], hc[7]]));
    }
}
