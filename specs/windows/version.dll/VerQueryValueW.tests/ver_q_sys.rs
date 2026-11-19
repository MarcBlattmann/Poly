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

// ---- file helpers (kernel32; prototypes from Learn) ----
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn RemoveDirectoryW(name: *const u16) -> BOOL;
    fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> BOOL;
    fn GetFileSize(h: HANDLE, hi: *mut u32) -> u32;
    fn GetFileAttributesW(name: *const u16) -> u32;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

/// The path of `name` in the directory of the running image (a fresh scratch directory per run on Windows), as UTF-16.
fn scratch(name: &str) -> [u16; 300] {
    let img = poly_wtest::image_path();
    let mut out = [0u16; 300];
    let mut last = 0;
    for (i, &c) in img.iter().enumerate() {
        if c == 0x5c {
            last = i + 1;
        }
    }
    let mut n = 0;
    for &c in &img[..last] {
        if n < 299 {
            out[n] = c;
            n += 1;
        }
    }
    for c in name.chars() {
        if n < 299 {
            out[n] = c as u32 as u16;
            n += 1;
        }
    }
    out
}

/// Reads a whole file (up to N bytes); `None` when it cannot be opened.
fn read_all<const N: usize>(path: &[u16]) -> Option<([u8; N], usize)> {
    let mut buf = [0u8; N];
    let mut got = 0u32;
    // SAFETY: valid NUL-terminated path and buffer.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_READ, 7, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            return None;
        }
        ReadFile(h, buf.as_mut_ptr(), N as u32, &mut got, core::ptr::null_mut());
        CloseHandle(h);
    }
    Some((buf, got as usize))
}

/// Writes a whole file; false on failure.
fn write_all(path: &[u16], data: &[u8]) -> bool {
    let mut got = 0u32;
    // SAFETY: valid NUL-terminated path and data.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            return false;
        }
        let ok = WriteFile(h, data.as_ptr(), data.len() as u32, &mut got, core::ptr::null_mut());
        CloseHandle(h);
        ok != 0 && got as usize == data.len()
    }
}

fn exists(path: &[u16]) -> bool {
    // SAFETY: valid NUL-terminated path.
    unsafe { GetFileAttributesW(path.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

fn hexdump(t: &mut Test, d: &[u8]) {
    for (i, ch) in d.chunks(32).enumerate() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "{:04x}:", i * 32);
        for x in ch {
            let _ = write!(b, " {:02x}", x);
        }
        t.line(b.as_str());
    }
}

#[link(name = "version", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileVersionInfoSizeW(file: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(file: *const u16, handle: u32, len: u32, data: *mut u8) -> BOOL;
    fn VerQueryValueW(block: *const u8, sub: *const u16, buf: *mut *const c_void, len: *mut u32) -> BOOL;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleFileNameW(m: HANDLE, buf: *mut u16, n: u32) -> u32;
}

poly_wtest::main!(test);

fn size_of(t: &mut Test, what: &str, path: *const u16, with_handle: bool) -> u32 {
    let mut h = 0x1234u32;
    clear_error();
    // SAFETY: valid pointers or NULL on purpose.
    let n = unsafe { GetFileVersionInfoSizeW(path, if with_handle { &mut h } else { core::ptr::null_mut() }) };
    say!(t, "size of {what}: result {} last error {} handle out {}", if n == 0 { "0" } else { "non-zero" }, if n == 0 { gle() } else { -2 }, if with_handle { i64::from(h) } else { -1 });
    n
}

fn test(t: &mut Test) {
    // the system's kernel32.dll: shape only (numbers differ between machines)
    let k = wide("C:\\Windows\\System32\\kernel32.dll");
    let mut h = 7u32;
    // SAFETY: valid pointers.
    let size = unsafe { GetFileVersionInfoSizeW(k.as_ptr(), &mut h) };
    say!(t, "kernel32.dll: size > 0 {}; size <= 16384 {}; handle out {h}", size > 0, size <= 16384);
    let mut kb = [0u8; 16384];
    // SAFETY: valid buffers.
    let ok = unsafe { GetFileVersionInfoW(k.as_ptr(), 0, size, kb.as_mut_ptr()) };
    say!(t, "kernel32.dll: GetFileVersionInfoW with the size from the first call: ok {ok}");
    // a stale handle value is ignored
    let mut kb2 = [0u8; 16384];
    // SAFETY: valid buffers.
    let ok2 = unsafe { GetFileVersionInfoW(k.as_ptr(), 0x55AA, size, kb2.as_mut_ptr()) };
    say!(t, "kernel32.dll: with a non-zero handle argument: ok {ok2}; same bytes {}", kb[..size as usize] == kb2[..size as usize]);
    // too small buffers
    let mut small = [0xEEu8; 64];
    clear_error();
    // SAFETY: valid buffers.
    let ok = unsafe { GetFileVersionInfoW(k.as_ptr(), 0, 64, small.as_mut_ptr()) };
    say!(t, "kernel32.dll: buffer length 64 (smaller than the size): ok {ok} last error {} first bytes written {}", gle_if(ok), small[0] != 0xEE);
    clear_error();
    // SAFETY: valid buffers.
    let ok = unsafe { GetFileVersionInfoW(k.as_ptr(), 0, 0, small.as_mut_ptr()) };
    say!(t, "kernel32.dll: buffer length 0: ok {ok} last error {}", gle_if(ok));
    // shape of the block
    let mut p: *const c_void = core::ptr::null();
    let mut n = 0u32;
    let root = wide("\\");
    // SAFETY: valid block.
    let ok = unsafe { VerQueryValueW(kb.as_ptr(), root.as_ptr(), &mut p, &mut n) };
    if ok != 0 && n == 52 {
        // SAFETY: pointer into the block, 52 bytes.
        let f = unsafe { core::slice::from_raw_parts(p as *const u32, 13) };
        say!(t, "kernel32.dll fixed info: signature ok {}; struct version {:#x}; os {:#x}; type {}", f[0] == 0xFEEF04BD, f[1], f[8], f[9]);
    } else {
        say!(t, "kernel32.dll root query: ok {ok} length {n}");
    }
    let tr = wide("\\VarFileInfo\\Translation");
    // SAFETY: valid block.
    let ok = unsafe { VerQueryValueW(kb.as_ptr(), tr.as_ptr(), &mut p, &mut n) };
    say!(t, "kernel32.dll translation query: ok {ok}; at least one pair {}; length multiple of 4 {}", n >= 4, n % 4 == 0);
    if ok != 0 && n >= 4 {
        // SAFETY: pointer into the block.
        let (lang, cp) = unsafe { (*(p as *const u16), *(p as *const u16).add(1)) };
        let mut sb = Buf::<100>::new();
        let _ = write!(sb, "\\StringFileInfo\\{lang:04X}{cp:04X}\\ProductName");
        let s = wide(sb.as_str());
        // SAFETY: valid block.
        let ok = unsafe { VerQueryValueW(kb.as_ptr(), s.as_ptr(), &mut p, &mut n) };
        say!(t, "kernel32.dll ProductName under the first translation: ok {ok}; non-empty {}", ok != 0 && n > 1);
        let mut sb = Buf::<100>::new();
        let _ = write!(sb, "\\StringFileInfo\\{lang:04X}{cp:04X}\\FileVersion");
        let s = wide(sb.as_str());
        // SAFETY: valid block.
        let ok = unsafe { VerQueryValueW(kb.as_ptr(), s.as_ptr(), &mut p, &mut n) };
        let txt = if ok != 0 { wstr_at(p as *const u16, 100) } else { &[] };
        say!(t, "kernel32.dll FileVersion: ok {ok}; starts with 10.0. {}", txt.len() > 5 && txt[0] == 0x31 && txt[1] == 0x30 && txt[2] == 0x2e && txt[3] == 0x30 && txt[4] == 0x2e);
    }
    // VerQueryValueW on garbage
    let zero = [0u8; 256];
    clear_error();
    // SAFETY: valid block.
    let ok = unsafe { VerQueryValueW(zero.as_ptr(), root.as_ptr(), &mut p, &mut n) };
    say!(t, "VerQueryValueW on a block of zero bytes: ok {ok} last error {}", gle_if(ok));
}
