//! SystemParametersInfoW with wrong sizes for SPI_GETNONCLIENTMETRICS
//! (cbSize 0, 4, 500, 504, 508; uiParam differing), an action number that is
//! not defined (0x7777, 0x1FFF), SPI_GETGRIDGRANULARITY (not supported: FALSE,
//! ERROR_INVALID_SPI_VALUE 1439), SPI_GETDESKWALLPAPER style call with a bad
//! buffer is not made; results and last errors.
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sz = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    // SAFETY: structures are as large as passed.
    unsafe {
        for (cb, param) in [(0u32, 0u32), (4, 4), (500, 500), (504, 504), (508, 508), (504, 0), (504, 500), (0, 504)] {
            let mut n: NONCLIENTMETRICSW = zeroed();
            n.cb_size = cb;
            set_error(0x1234);
            let ok = SystemParametersInfoW(0x29, param, &mut n as *mut _ as *mut c_void, 0);
            say!(t, "NONCLIENTMETRICS cbSize {cb} uiParam {param}: ok {ok} caption {}x{}", n.caption_width, n.caption_height);
        }
        let _ = sz;
        for a in [0x7777u32, 0x1FFF, 0x12, 0x2FFF, 0] {
            let mut b = [0u8; 64];
            set_error(0x1234);
            let ok = SystemParametersInfoW(a, 0, b.as_mut_ptr() as *mut c_void, 0);
            say!(t, "action {:#x}: ok {ok} last error {}", a, last_error());
        }
    }
}
