//! CreateRectRgn with large coordinates: around 2^27 (134217727), the
//! largest values that still make a region, i32::MIN/MAX: whether a region
//! is made, and the box it has (whether coordinates are clamped); and
//! SetRectRgn on an existing region with the same values.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HGDIOBJ = *mut c_void;
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

type HRGN = *mut c_void;
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn GetRgnBox(rgn: HRGN, rect: *mut RECT) -> i32;
    fn SetRectRgn(rgn: HRGN, l: i32, t: i32, r: i32, b: i32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cases: [(&str, i32, i32, i32, i32); 10] = [
        ("2^27-1 square", 0, 0, 0x07FF_FFFF, 0x07FF_FFFF),
        ("2^27 square", 0, 0, 0x0800_0000, 0x0800_0000),
        ("-2^27 to 2^27-1", -0x0800_0000, -0x0800_0000, 0x07FF_FFFF, 0x07FF_FFFF),
        ("-2^27-1 left", -0x0800_0001, 0, 10, 10),
        ("2^28 right", 0, 0, 0x1000_0000, 10),
        ("i32::MAX right", 0, 0, i32::MAX, 10),
        ("i32::MIN left", i32::MIN, 0, 10, 10),
        ("MIN..MAX", i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        ("1000000 square", -1_000_000, -1_000_000, 1_000_000, 1_000_000),
        ("65536 square", 0, 0, 65536, 65536),
    ];
    for (label, l, tp, r, b) in cases {
        set_error(0);
        // SAFETY: a new region, deleted after use.
        unsafe {
            let rgn = CreateRectRgn(l, tp, r, b);
            let err = last_error();
            if rgn.is_null() {
                say!(t, "{label}: NULL error {err}");
                continue;
            }
            let mut bx = RECT::default();
            let kind = GetRgnBox(rgn, &mut bx);
            say!(t, "{label}: region, kind {kind} box {}", R(bx));
            DeleteObject(rgn);
        }
    }
    // SAFETY: a region of ours, deleted at the end.
    unsafe {
        let rgn = CreateRectRgn(1, 2, 3, 4);
        for (label, l, tp, r, b) in [
            ("2^27 square", 0, 0, 0x0800_0000, 0x0800_0000),
            ("MIN..MAX", i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            ("swapped", 30, 40, 10, 20),
            ("empty", 5, 5, 5, 9),
        ] {
            set_error(0);
            let ok = SetRectRgn(rgn, l, tp, r, b);
            let mut bx = RECT::default();
            let kind = GetRgnBox(rgn, &mut bx);
            say!(t, "SetRectRgn {label}: {ok} error {} kind {kind} box {}", last_error(), R(bx));
        }
        DeleteObject(rgn);
    }
}
