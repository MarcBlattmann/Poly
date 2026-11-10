//! CreateRectRgn's regions through GetRegionData with buffers of every
//! size: what is returned (the bytes needed when the buffer is NULL, the
//! bytes written when it is large enough, 0 when it is too small) and what
//! the header says (dwSize 32, RDH_RECTANGLES 1, nCount, nRgnSize, rcBound).
//! Regions with one, none, two and three rectangles; bad regions.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HBRUSH = *mut c_void;
type HGDIOBJ = *mut c_void;
type BOOL = i32;
type COLORREF = u32;
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

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
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

type HRGN = *mut c_void;
/// wingdi.h: RGNDATAHEADER followed by up to 64 rectangles.
#[repr(C)]
struct RGNDATA64 {
    size: u32,
    kind: u32,
    count: u32,
    rgn_size: u32,
    bound: RECT,
    rects: [RECT; 64],
}

const RGN_OR: i32 = 2;
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
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RGNDATA64) -> u32;
}

fn probe(t: &mut Test, label: &str, rgn: HRGN) {
    let sizes: [i64; 14] = [-1, 0, 1, 16, 31, 32, 33, 47, 48, 49, 64, 80, 100, 1056];
    for size in sizes {
        let mut d: RGNDATA64 = zeroed();
        set_error(0);
        // SAFETY: d holds 1056 bytes, the largest size tried; a NULL buffer is allowed with size 0.
        let n = unsafe {
            if size < 0 {
                GetRegionData(rgn, 0, core::ptr::null_mut())
            } else {
                GetRegionData(rgn, size as u32, &mut d)
            }
        };
        let e = last_error();
        if size < 0 {
            say!(t, "{label}: NULL buffer: returned {n} error {e}");
        } else {
            say!(
                t,
                "{label}: buffer {size}: returned {n} error {e} header size {} type {} count {} rgnsize {} bound {} first {}",
                d.size,
                d.kind,
                d.count,
                d.rgn_size,
                R(d.bound),
                R(d.rects[0])
            );
        }
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: regions of ours, all deleted at the end.
    unsafe {
        let one = CreateRectRgn(10, 20, 110, 70);
        probe(t, "one rectangle", one);
        let empty = CreateRectRgn(7, 7, 7, 9);
        probe(t, "empty", empty);
        let a = CreateRectRgn(0, 0, 10, 10);
        let b = CreateRectRgn(20, 30, 40, 50);
        let two = CreateRectRgn(0, 0, 0, 0);
        let k = CombineRgn(two, a, b, RGN_OR);
        say!(t, "two disjoint rectangles: CombineRgn kind {k}");
        probe(t, "two rectangles", two);
        let c = CreateRectRgn(100, 100, 150, 120);
        let three = CreateRectRgn(0, 0, 0, 0);
        CombineRgn(three, two, c, RGN_OR);
        probe(t, "three rectangles", three);
        // NULL, a brush and a deleted region.
        set_error(0);
        let n = GetRegionData(core::ptr::null_mut(), 0, core::ptr::null_mut());
        say!(t, "NULL region, NULL buffer: returned {n} error {}", last_error());
        let br = CreateSolidBrush(rgb(1, 2, 3));
        set_error(0);
        let n = GetRegionData(br, 0, core::ptr::null_mut());
        say!(t, "a brush handle, NULL buffer: returned {n} error {}", last_error());
        DeleteObject(br);
        let gone = CreateRectRgn(1, 1, 5, 5);
        DeleteObject(gone);
        set_error(0);
        let n = GetRegionData(gone, 0, core::ptr::null_mut());
        say!(t, "deleted region, NULL buffer: returned {n} error {}", last_error());
        for r in [one, empty, a, b, two, c, three] {
            DeleteObject(r);
        }
    }
}
