#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
type DWORD = u32;

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

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; no characters outside the BMP).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| {
            let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
            l(x) == l(y)
        })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn clear_error() {
    // SAFETY: plain call.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a failed call, else -2 (success leaves it unspecified).
fn gle_if(ok: BOOL) -> i64 {
    if ok == 0 { gle() } else { -2 }
}
// OleLoadPicturePath (oleaut32 ordinal 424): a 1x1 GIF: picture type and size; the file content decides the format, not the name.

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OleLoadPicturePath(path: *const u16, unk: *mut c_void, reserved: u32, back: u32, iid: *const [u8; 16], pp: *mut *mut c_void) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const c_void, flags: u32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(path: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(path: *const u16) -> BOOL;
    fn GetCurrentDirectoryW(len: u32, buf: *mut u16) -> u32;
}

poly_wtest::main!(test);

const IID_IPicture: [u8; 16] = [0x80, 0x09, 0xf8, 0x7b, 0x32, 0xbf, 0x1a, 0x10, 0x8b, 0xbb, 0x00, 0xaa, 0x00, 0x30, 0x0c, 0xab];
const IID_IUnknown: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IDispatch: [u8; 16] = [0x00, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

// A 1x1 pixel, 24 bits per pixel Windows bitmap (BITMAPFILEHEADER + BITMAPINFOHEADER + one padded row).
const BMP: [u8; 58] = [
    0x42, 0x4d, 58, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, // file header
    40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0x13, 0x0b, 0, 0, 0x13, 0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // info header
    0xff, 0x00, 0x00, 0x00, // blue pixel + padding
];

/// "<current directory>\name" as text (the scratch directory of the run; never printed).
fn full(name: &str) -> Buf<600> {
    let mut cwd = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetCurrentDirectoryW(300, cwd.as_mut_ptr()) } as usize;
    let mut b = Buf::<600>::new();
    for &c in &cwd[..n] {
        let _ = write!(b, "{}", c as u8 as char);
    }
    let _ = write!(b, "\\{name}");
    b
}

fn vt(p: *mut c_void, slot: usize) -> usize {
    // SAFETY: COM interface pointer.
    unsafe { *(*(p as *const *const usize)).add(slot) }
}

fn write_file(name: &str, data: &[u8]) -> bool {
    let w = wide(name);
    // SAFETY: valid string; CREATE_ALWAYS = 2.
    let h = unsafe { CreateFileW(w.as_ptr(), 0x4000_0000, 0, core::ptr::null(), 2, 0x80, core::ptr::null_mut()) };
    if h as usize == usize::MAX {
        return false;
    }
    let mut n = 0u32;
    // SAFETY: valid handle and buffer.
    unsafe {
        WriteFile(h, data.as_ptr(), data.len() as u32, &mut n, core::ptr::null_mut());
        CloseHandle(h);
    }
    true
}

fn load(t: &mut Test, label: &str, path: &str, iid: &[u8; 16]) -> *mut c_void {
    let w = wide(path);
    let mut p: *mut c_void = 0x1234usize as *mut c_void;
    // SAFETY: valid string and out pointer.
    let hr = unsafe { OleLoadPicturePath(w.as_ptr(), core::ptr::null_mut(), 0, 0, iid, &mut p) };
    let state = if p as usize == 0x1234 { "untouched" } else if p.is_null() { "NULL" } else { "set" };
    say!(t, "{label}: hr={:#010x} out {state}", hr as u32);
    if hr >= 0 && !p.is_null() && p as usize != 0x1234 { p } else { core::ptr::null_mut() }
}

fn release(p: *mut c_void) {
    // SAFETY: IUnknown::Release.
    unsafe {
        let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vt(p, 2));
        f(p);
    }
}


// A 1x1 GIF89a image (header, logical screen with a 2-colour table, one image).
const GIF: [u8; 35] = [
    b'G', b'I', b'F', b'8', b'9', b'a', 1, 0, 1, 0, 0x80, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0, 0x3b,
];

fn test(t: &mut Test) {
    // SAFETY: COM init for this thread.
    unsafe { CoInitializeEx(core::ptr::null(), 2) };
    say!(t, "wrote the gif: {}", write_file("poly-pic.gif", &GIF));
    let pic = load(t, "poly-pic.gif", full("poly-pic.gif").as_str(), &IID_IPicture);
    if !pic.is_null() {
        // SAFETY: vtable slots of IPicture.
        unsafe {
            let get_type: extern "system" fn(*mut c_void, *mut i16) -> i32 = core::mem::transmute(vt(pic, 5));
            let get_w: extern "system" fn(*mut c_void, *mut i32) -> i32 = core::mem::transmute(vt(pic, 6));
            let get_h: extern "system" fn(*mut c_void, *mut i32) -> i32 = core::mem::transmute(vt(pic, 7));
            let (mut ty, mut w, mut h) = (-1i16, -1i32, -1i32);
            let r1 = get_type(pic, &mut ty);
            let r2 = get_w(pic, &mut w);
            let r3 = get_h(pic, &mut h);
            say!(t, "IPicture: type {ty} width {w} height {h} hr {r1} {r2} {r3}");
        }
        release(pic);
    }
    // a bitmap file named like a gif is still a bitmap (the content decides)
    say!(t, "wrote the bitmap as .gif: {}", write_file("poly-bmp.gif", &BMP));
    let b = load(t, "bitmap content in a .gif name", full("poly-bmp.gif").as_str(), &IID_IPicture);
    if !b.is_null() {
        release(b);
    }
    for n in ["poly-pic.gif", "poly-bmp.gif"] {
        let w = wide(n);
        // SAFETY: valid string.
        unsafe { DeleteFileW(w.as_ptr()) };
    }
}
