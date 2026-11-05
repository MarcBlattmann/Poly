//! GetSystemMetrics, the flag and count indices (mouse, buttons, wheels,
//! remote session, tablet, slate and dock state, ...) and indices that are
//! not defined in winuser.h: what each returns, and for the undefined ones
//! the last error (set to 0 before each call). GetSystemMetricsForDpi with
//! DPI 0, unusual DPIs and undefined indices.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

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

const SM_MOUSEPRESENT: i32 = 19;
const SM_SWAPBUTTON: i32 = 23;
const SM_CMOUSEBUTTONS: i32 = 43;
const SM_MOUSEWHEELPRESENT: i32 = 75;
const SM_ARRANGE: i32 = 56;
const SM_DBCSENABLED: i32 = 42;
const SM_DEBUG: i32 = 22;
const SM_MENUDROPALIGNMENT: i32 = 40;
const SM_PENWINDOWS: i32 = 41;
const SM_SECURE: i32 = 44;
const SM_CLEANBOOT: i32 = 67;
const SM_SHOWSOUNDS: i32 = 70;
const SM_SLOWMACHINE: i32 = 73;
const SM_MIDEASTENABLED: i32 = 74;
const SM_IMMENABLED: i32 = 82;
const SM_TABLETPC: i32 = 86;
const SM_MEDIACENTER: i32 = 87;
const SM_STARTER: i32 = 88;
const SM_SERVERR2: i32 = 89;
const SM_MOUSEHORIZONTALWHEELPRESENT: i32 = 91;
const SM_DIGITIZER: i32 = 94;
const SM_MAXIMUMTOUCHES: i32 = 95;
const SM_REMOTESESSION: i32 = 0x1000;
const SM_SHUTTINGDOWN: i32 = 0x2000;
const SM_REMOTECONTROL: i32 = 0x2001;
const SM_CONVERTIBLESLATEMODE: i32 = 0x2003;
const SM_SYSTEMDOCKED: i32 = 0x2004;
const SM_NETWORK: i32 = 63;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

const GSM_FLAGS: [(&str, i32); 29] = [
    ("SM_MOUSEPRESENT", 19),
    ("SM_DEBUG", 22),
    ("SM_SWAPBUTTON", 23),
    ("SM_MENUDROPALIGNMENT", 40),
    ("SM_PENWINDOWS", 41),
    ("SM_DBCSENABLED", 42),
    ("SM_CMOUSEBUTTONS", 43),
    ("SM_SECURE", 44),
    ("SM_ARRANGE", 56),
    ("SM_CLEANBOOT", 67),
    ("SM_SHOWSOUNDS", 70),
    ("SM_SLOWMACHINE", 73),
    ("SM_MIDEASTENABLED", 74),
    ("SM_MOUSEWHEELPRESENT", 75),
    ("SM_IMMENABLED", 82),
    ("SM_TABLETPC", 86),
    ("SM_MEDIACENTER", 87),
    ("SM_STARTER", 88),
    ("SM_SERVERR2", 89),
    ("SM_MOUSEHORIZONTALWHEELPRESENT", 91),
    ("SM_DIGITIZER", 94),
    ("SM_MAXIMUMTOUCHES", 95),
    ("SM_REMOTESESSION", 0x1000),
    ("SM_SHUTTINGDOWN", 0x2000),
    ("SM_REMOTECONTROL", 0x2001),
    ("SM_CARETBLINKINGENABLED", 0x2002),
    ("SM_CONVERTIBLESLATEMODE", 0x2003),
    ("SM_SYSTEMDOCKED", 0x2004),
    ("SM_NETWORK & 1", 63),
];

const GSM_UNDEFINED: [i32; 14] = [-1, -100, 24, 25, 26, 27, 64, 65, 66, 90, 96, 97, 0x1001, 0x2005];

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for &(name, index) in GSM_FLAGS.iter() {
        // SAFETY: plain query.
        let mut v = unsafe { GetSystemMetrics(index) };
        if index == 63 {
            v &= 1;
        }
        say!(t, "{name} ({index:#x}) = {v}");
    }
    for &index in GSM_UNDEFINED.iter() {
        set_error(0);
        // SAFETY: plain query.
        let v = unsafe { GetSystemMetrics(index) };
        say!(t, "undefined index {index} ({:#x}) = {v} error {}", index as u32, last_error());
    }
    set_error(0);
    // SAFETY: plain query.
    let v = unsafe { GetSystemMetrics(i32::MAX) };
    say!(t, "index 0x7fffffff = {v} error {}", last_error());
    for (index, dpi) in [(4, 0u32), (4, 1), (4, 48), (4, 1000), (64, 96), (0x1000, 96)] {
        set_error(0);
        // SAFETY: plain query.
        let v = unsafe { GetSystemMetricsForDpi(index, dpi) };
        say!(t, "ForDpi index {index:#x} dpi {dpi} = {v} error {}", last_error());
    }
}
