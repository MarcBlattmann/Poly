//! SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS): cbSize must be 504 and
//! uiParam must equal it; cbSize 0, 500 or a wrong uiParam (0, 4) fail with
//! ERROR_INVALID_PARAMETER and leave the structure alone; a NULL pvParam fails the
//! same way.
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
    fn SystemParametersInfoForDpi(action: u32, param: u32, pv: *mut c_void, ini: u32, dpi: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sz = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    // SAFETY: the structure is sz bytes.
    unsafe {
        for (cb, param) in [(sz, sz), (sz, 0), (0, sz), (sz - 4, sz - 4), (500, 500), (sz, 4), (sz + 4, sz + 4)] {
            let mut n: NONCLIENTMETRICSW = zeroed();
            n.cb_size = cb;
            n.caption_height = 0x55;
            set_error(0x1234);
            let ok = SystemParametersInfoForDpi(0x29, param, &mut n as *mut _ as *mut c_void, 0, 144);
            say!(t, "cbSize {cb} uiParam {param} dpi 144: ok {ok} last error {} caption height field {}", dp_e(ok, last_error()), n.caption_height);
        }
        set_error(0x1234);
        let ok = SystemParametersInfoForDpi(0x29, sz, core::ptr::null_mut(), 0, 144);
        say!(t, "NULL pvParam: ok {ok} last error {}", last_error());
        let mut n: NONCLIENTMETRICSW = zeroed();
        n.cb_size = sz;
        set_error(0x1234);
        let ok = SystemParametersInfoForDpi(0x29, sz, &mut n as *mut _ as *mut c_void, 0, 96);
        say!(t, "dpi 96 (the system DPI): ok {ok} last error {}", dp_e(ok, last_error()));
    }
}
