//! SystemParametersInfoForDpi with SPI_GETICONMETRICS and SPI_GETICONTITLELOGFONT
//! at several DPIs (spacing 75/94/113/150, font height -12/-15/-18/-24), and
//! with actions it does not support (SPI_GETWORKAREA, SPI_GETBEEP, GETMOUSE,
//! ICONHORIZONTALSPACING, GETWHEELSCROLLLINES, GETFONTSMOOTHING): FALSE with
//! ERROR_INVALID_PARAMETER for DPIs other than 0; DPI 0 forwards any action to
//! SystemParametersInfoW.
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
    fn SystemParametersInfoForDpi(action: u32, param: u32, pv: *mut c_void, ini: u32, dpi: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: buffers are large enough for the actions.
    unsafe {
        for d in [96u32, 120, 144, 192, 288] {
            let mut i2: ICONMETRICSW = zeroed();
            i2.cb_size = core::mem::size_of::<ICONMETRICSW>() as u32;
            set_error(0x1234);
            let ok = SystemParametersInfoForDpi(0x2D, i2.cb_size, &mut i2 as *mut _ as *mut c_void, 0, d);
            say!(t, "ICONMETRICS dpi {d}: ok {ok} last error {} horizontal {} vertical {} wrap {} font height {}", dp_e(ok, last_error()), i2.horz_spacing, i2.vert_spacing, i2.title_wrap, i2.font.height);
            let mut lf: LOGFONTW = zeroed();
            set_error(0x1234);
            let ok = SystemParametersInfoForDpi(0x1F, core::mem::size_of::<LOGFONTW>() as u32, &mut lf as *mut _ as *mut c_void, 0, d);
            say!(t, "ICONTITLELOGFONT dpi {d}: ok {ok} height {} weight {}", lf.height, lf.weight);
        }
        for (name, a) in [("GETWORKAREA", 0x30u32), ("GETBEEP", 1), ("GETMOUSE", 3), ("ICONHORIZONTALSPACING", 0xD), ("GETWHEELSCROLLLINES", 0x68), ("GETFONTSMOOTHING", 0x4A), ("GETMENUSHOWDELAY", 0x6A), ("GETCARETWIDTH", 0x2006)] {
            for d in [96u32, 144, 0] {
                let mut v = [0u8; 32];
                set_error(0x1234);
                let ok = SystemParametersInfoForDpi(a, 0, v.as_mut_ptr() as *mut c_void, 0, d);
                say!(t, "{name} dpi {d}: ok {ok} last error {} first value {}", last_error(), u32::from_le_bytes([v[0], v[1], v[2], v[3]]));
            }
        }
    }
}
