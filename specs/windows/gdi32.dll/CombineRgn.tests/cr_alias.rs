//! CombineRgn when the destination is also a source (dst = A op dst,
//! dst = dst op dst for each mode), RGN_COPY with a NULL or a deleted
//! second source, NULL or invalid handles in each position (the return
//! value, the last error, whether the destination changed), and the
//! destination's old contents being replaced (not merged).
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

const RGN_AND: i32 = 1;
const RGN_OR: i32 = 2;
const RGN_XOR: i32 = 3;
const RGN_DIFF: i32 = 4;
const RGN_COPY: i32 = 5;
/// The rectangles of a region, "l,t,r,b" joined with " + " (at most 64),
/// or "empty".
fn rgn_text(rgn: HRGN, out: &mut Buf<300>) {
    let mut d: RGNDATA64 = zeroed();
    // SAFETY: d is large enough for the header and 64 rectangles.
    let n = unsafe { GetRegionData(rgn, core::mem::size_of::<RGNDATA64>() as u32, &mut d) };
    if n == 0 {
        let _ = out.write_str("(no data)");
        return;
    }
    if d.count == 0 {
        let _ = out.write_str("empty");
        return;
    }
    for i in 0..(d.count as usize).min(64) {
        let r = d.rects[i];
        if i > 0 {
            let _ = out.write_str(" + ");
        }
        let _ = write!(out, "{},{},{},{}", r.left, r.top, r.right, r.bottom);
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
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn SetRectRgn(rgn: HRGN, l: i32, t: i32, r: i32, b: i32) -> BOOL;
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RGNDATA64) -> u32;
}

fn cr_text(rgn: HRGN) -> Buf<300> {
    let mut s = Buf::<300>::new();
    rgn_text(rgn, &mut s);
    s
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: regions of ours; invalid handles are what is observed.
    unsafe {
        for (name, mode) in [("AND", RGN_AND), ("OR", RGN_OR), ("XOR", RGN_XOR), ("DIFF", RGN_DIFF), ("COPY", RGN_COPY)]
        {
            let a = CreateRectRgn(0, 0, 20, 20);
            let d = CreateRectRgn(10, 10, 30, 30);
            let k = CombineRgn(d, a, d, mode);
            say!(t, "dst = A {name} dst: kind {k} [{}]", cr_text(d).as_str());
            let k = CombineRgn(d, d, d, mode);
            say!(t, "  then dst = dst {name} dst: kind {k} [{}]", cr_text(d).as_str());
            DeleteObject(a);
            DeleteObject(d);
        }
        let a = CreateRectRgn(0, 0, 20, 20);
        let b = CreateRectRgn(100, 100, 120, 120);
        let d = CreateRectRgn(50, 50, 60, 60);
        let k = CombineRgn(d, a, b, RGN_AND);
        say!(t, "old destination replaced: kind {k} [{}]", cr_text(d).as_str());
        let gone = CreateRectRgn(1, 1, 2, 2);
        DeleteObject(gone);
        let null = core::ptr::null_mut();
        let bad = 0x0bad_0bad_usize as HRGN;
        let cases: [(&str, HRGN, HRGN, HRGN, i32); 9] = [
            ("COPY with NULL second", d, a, null, RGN_COPY),
            ("COPY with deleted second", d, b, gone, RGN_COPY),
            ("COPY with bad second", d, a, bad, RGN_COPY),
            ("OR with NULL second", d, b, null, RGN_OR),
            ("AND with deleted second", d, a, gone, RGN_AND),
            ("NULL destination", null, a, b, RGN_OR),
            ("deleted destination", gone, a, b, RGN_OR),
            ("NULL first", d, null, b, RGN_OR),
            ("bad first, COPY", d, bad, null, RGN_COPY),
        ];
        for (label, dst, s1, s2, mode) in cases {
            SetRectRgn(d, 5, 5, 6, 6);
            set_error(0);
            let k = CombineRgn(dst, s1, s2, mode);
            say!(t, "{label}: kind {k} error {} destination now [{}]", last_error(), cr_text(d).as_str());
        }
        DeleteObject(a);
        DeleteObject(b);
        DeleteObject(d);
    }
}
