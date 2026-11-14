//! GetWindowRgn on the window's copy of larger elliptic regions: the box (one
//! pixel less than the rectangle given in both directions), one rectangle per
//! run of equal rows, a copy that is the same in the upper and the lower
//! half, and the rows that are mirrored left and right.
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
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type HGDIOBJ = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    wnd_proc: WNDPROC,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HINSTANCE,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: HBRUSH,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const GWLP_USERDATA: i32 = -21;
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

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Register a class with `proc_` (and no menu, icon or cursor).
fn register(name: &[u16], proc_: WNDPROC, style: u32, wnd_extra: i32, brush: HBRUSH) -> u16 {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        wnd_proc: proc_,
        cls_extra: 0,
        wnd_extra,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: brush,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW; the name is NUL-terminated and static.
    unsafe { RegisterClassExW(&wc) }
}

/// CreateWindowExW with this executable's instance.
fn create(
    ex: u32,
    class: &[u16],
    name: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    // SAFETY: NUL-terminated names; the parent is a window or NULL.
    unsafe {
        CreateWindowExW(
            ex,
            class.as_ptr(),
            if name.is_empty() { core::ptr::null() } else { name.as_ptr() },
            style,
            x,
            y,
            w,
            h,
            parent,
            menu as HMENU,
            instance(),
            core::ptr::null_mut(),
        )
    }
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// The rows of a small top-down 32-bit DIB as text: one character per
/// pixel from `key` (pixel value, char), '?' for others.
fn rows(t: &mut Test, bits: *mut u32, w: i32, h: i32, key: &[(u32, char)]) {
    for y in 0..h {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "  |");
        for x in 0..w {
            let v = px(bits, w, x, y);
            let c = key.iter().find(|k| k.0 == v).map(|k| k.1).unwrap_or('?');
            let _ = b.write_char(c);
        }
        let _ = b.write_str("|");
        t.line(b.as_str());
    }
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

/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RGNDATA64) -> u32;
}

poly_wtest::main!(test);

#[repr(C)]
struct RgnBig {
    size: u32,
    kind: u32,
    count: u32,
    rgn_size: u32,
    bound: RECT,
    rects: [RECT; 600],
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateEllipticRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateRoundRectRgn(l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> HRGN;
    #[link_name = "GetRegionData"]
    fn GetRegionDataBig(rgn: HRGN, n: u32, data: *mut RgnBig) -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetWindowRgn(hwnd: HWND, rgn: HRGN, redraw: BOOL) -> i32;
    fn GetWindowRgn(hwnd: HWND, rgn: HRGN) -> i32;
    fn GetWindowRgnBox(hwnd: HWND, rect: *mut RECT) -> i32;
}

/// The region's rectangles, each as "top:left-right" (right exclusive) or
/// "top..bottom:left-right" for a rectangle of several rows.
fn text_of(rgn: HRGN) -> (Buf<2400>, u32) {
    let mut d: RgnBig = zeroed();
    // SAFETY: d is large enough for the header and 600 rectangles.
    unsafe { GetRegionDataBig(rgn, core::mem::size_of::<RgnBig>() as u32, &mut d) };
    let mut out = Buf::<2400>::new();
    for i in 0..(d.count as usize).min(600) {
        let r = d.rects[i];
        if r.bottom == r.top + 1 {
            let _ = write!(out, " {}:{}-{}", r.top, r.left, r.right);
        } else {
            let _ = write!(out, " {}..{}:{}-{}", r.top, r.bottom, r.left, r.right);
        }
    }
    (out, d.count)
}

/// Print `label` and the text in lines of at most about 250 characters.
fn lines(t: &mut Test, label: &str, s: &Buf<2400>) {
    let txt = s.as_str();
    let bytes = txt.as_bytes();
    let mut i = 0;
    let mut first = true;
    while i < bytes.len() {
        let mut j = (i + 230).min(bytes.len());
        while j < bytes.len() && bytes[j] != b' ' {
            j += 1;
        }
        let mut line = Buf::<300>::new();
        let _ = write!(line, "{}{}", if first { label } else { "  .." }, &txt[i..j]);
        t.line(line.as_str());
        first = false;
        i = j;
    }
    if first {
        t.line(label);
    }
}

/// The rows of the region as (left, right) pairs, index = row, for rows
/// 0..n; (-1, -1) for a row without a rectangle.
fn rows_of(rgn: HRGN, n: usize, out: &mut [(i32, i32)]) {
    let mut d: RgnBig = zeroed();
    // SAFETY: d is large enough for the header and 600 rectangles.
    unsafe { GetRegionDataBig(rgn, core::mem::size_of::<RgnBig>() as u32, &mut d) };
    for y in 0..n.min(out.len()) {
        out[y] = (-1, -1);
        for i in 0..(d.count as usize).min(600) {
            let q = d.rects[i];
            if q.top <= y as i32 && (y as i32) < q.bottom {
                out[y] = (q.left, q.right);
            }
        }
    }
}

/// The window's copy of a region (made by `make`), after SetWindowRgn.
fn window_copy(w: HWND, make: &dyn Fn() -> HRGN) -> (HRGN, i32) {
    let src = make();
    // SAFETY: our window and regions; the window owns `src` after the call.
    unsafe {
        SetWindowRgn(w, src, 0);
        let copy = CreateRectRgn(0, 0, 0, 0);
        let ty = GetWindowRgn(w, copy);
        (copy, ty)
    }
}

fn new_window() -> HWND {
    let cls = wz!("GwrTest");
    register(cls, Some(DefWindowProcW), 0, 0, core::ptr::null_mut());
    create(WS_EX_TOOLWINDOW, cls, &[], WS_POPUP, 100, 100, 200, 150, core::ptr::null_mut(), 0)
}

fn test(t: &mut Test) {
    let w = new_window();
    // SAFETY: our window and regions.
    unsafe {
        for (wd, h) in [(20, 20), (31, 21), (50, 30), (64, 40), (90, 45), (24, 60), (130, 100), (101, 101), (45, 12), (12, 45)] {
            let (copy, ty) = window_copy(w, &|| CreateEllipticRgn(0, 0, wd, h));
            let mut bx = RECT::default();
            let by = GetWindowRgnBox(w, &mut bx);
            let mut rows = [(0i32, 0i32); 140];
            let n = (h - 1) as usize;
            rows_of(copy, n, &mut rows);
            let bw = wd - 1;
            let mut vsym = true;
            let mut hsym = 0;
            let mut grows = true;
            let mut full = 0;
            let mut missing = 0;
            for y in 0..n {
                if rows[y] != rows[n - 1 - y] {
                    vsym = false;
                }
                if rows[y].0 + rows[y].1 != bw {
                    hsym += 1;
                }
                if y > 0 && y < n / 2 && (rows[y].0 > rows[y - 1].0 || rows[y].1 < rows[y - 1].1) {
                    grows = false;
                }
                if rows[y] == (0, bw) {
                    full += 1;
                }
                if rows[y].0 < 0 {
                    missing += 1;
                }
            }
            let (_, cnt) = text_of(copy);
            say!(
                t,
                "ellipse 0,0,{wd},{h}: type {ty}, box {} (call {by} {}), {n} rows, rectangles {cnt}, upper = lower half {vsym}, rows not mirrored left/right {hsym}, rows widen towards the middle {grows}, full-width rows {full}, rows without a rectangle {missing}",
                R(bx),
                R(bx)
            );
            DeleteObject(copy);
        }
        DestroyWindow(w);
    }
}
