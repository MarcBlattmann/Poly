//! A region's lifetime: DeleteObject returns 1; afterwards CombineRgn with
//! the handle as a source fails (ERROR 0, ERROR_INVALID_HANDLE) and a
//! second DeleteObject returns 0; GetRgnBox of NULL and of a stock brush;
//! 1000 regions made and deleted in turn.
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

const WHITE_BRUSH: i32 = 0;
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
const RGN_COPY: i32 = 5;
const OBJ_REGION: u32 = 8;
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
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn GetRgnBox(rgn: HRGN, rect: *mut RECT) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: regions and a DC of ours; invalid handles are what is observed.
    unsafe {
        let rgn = CreateRectRgn(0, 0, 10, 10);
        say!(t, "DeleteObject: {}", DeleteObject(rgn));
        let mut bx = RECT { left: 7, top: 7, right: 7, bottom: 7 };
        let dst = CreateRectRgn(0, 0, 1, 1);
        set_error(0);
        say!(
            t,
            "  CombineRgn(dst, deleted, NULL, RGN_COPY) {} error {}",
            CombineRgn(dst, rgn, core::ptr::null_mut(), RGN_COPY),
            last_error()
        );
        set_error(0);
        say!(t, "  second DeleteObject {}", DeleteObject(rgn));
        set_error(0);
        say!(t, "GetRgnBox(NULL) {} error {} box {}", GetRgnBox(core::ptr::null_mut(), &mut bx), last_error(), R(bx));
        set_error(0);
        say!(t, "GetRgnBox(a brush) {} error {}", GetRgnBox(GetStockObject(WHITE_BRUSH), &mut bx), last_error());
        DeleteObject(dst);
        let mut ok = 0;
        for i in 0..1000 {
            let r = CreateRectRgn(i, i, i + 5, i + 5);
            if !r.is_null() && GetObjectType(r) == OBJ_REGION && DeleteObject(r) != 0 {
                ok += 1;
            }
        }
        t.expect("1000 regions made and deleted", ok, 1000);
    }
}
