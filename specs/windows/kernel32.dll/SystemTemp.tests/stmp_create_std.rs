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
// C:\Windows\SystemTemp: creating a directory (like the installers' Google<pid>_<n>), a file inside it, and removing both.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateDirectoryW(path: *const u16, sa: *const c_void) -> BOOL;
    fn RemoveDirectoryW(path: *const u16) -> BOOL;
    fn CreateFileW(path: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(path: *const u16) -> BOOL;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> BOOL;
}

poly_wtest::main!(test);

const CREATE_NEW: u32 = 1;
const INVALID: usize = usize::MAX;

fn path(suffix: &str) -> [u16; 300] {
    // a name that is unique to this run but whose text is not printed
    let mut b = Buf::<200>::new();
    // SAFETY: plain call.
    let pid = unsafe { GetCurrentProcessId() };
    let _ = write!(b, "C:\\Windows\\SystemTemp\\polyspec{pid}{suffix}");
    wide(b.as_str())
}

fn test(t: &mut Test) {
    let dir = path("_dir");
    clear_error();
    // SAFETY: valid string.
    let ok = unsafe { CreateDirectoryW(dir.as_ptr(), core::ptr::null()) };
    say!(t, "CreateDirectoryW(SystemTemp\\sub) = {ok} err {}", gle_if(ok));
    if ok != 0 {
        let mut b = Buf::<300>::new();
        let d = wstr_at(dir.as_ptr(), 299);
        for &c in d {
            let _ = write!(b, "{}", c as u8 as char);
        }
        let _ = write!(b, "\\inner.txt");
        let f = wide(b.as_str());
        clear_error();
        // SAFETY: valid string.
        let h = unsafe { CreateFileW(f.as_ptr(), 0x4000_0000, 0, core::ptr::null(), CREATE_NEW, 0x80, core::ptr::null_mut()) };
        let fok = h as usize != INVALID;
        say!(t, "create a file inside it = {fok} err {}", if fok { -2 } else { gle() });
        if fok {
            let mut n = 0u32;
            // SAFETY: valid handle and buffer.
            let w = unsafe { WriteFile(h, b"hello".as_ptr(), 5, &mut n, core::ptr::null_mut()) };
            say!(t, "write to it = {w} bytes {n}");
            // SAFETY: valid handle.
            unsafe { CloseHandle(h) };
            // SAFETY: valid string.
            let del = unsafe { DeleteFileW(f.as_ptr()) };
            say!(t, "delete it = {del}");
        }
        // SAFETY: valid string.
        let a = unsafe { GetFileAttributesW(dir.as_ptr()) };
        say!(t, "the new directory is a directory: {}", a != 0xFFFF_FFFF && a & 0x10 != 0);
        // SAFETY: valid string.
        let rd = unsafe { RemoveDirectoryW(dir.as_ptr()) };
        say!(t, "RemoveDirectoryW = {rd}");
    }
    // a file directly in SystemTemp
    let file = path("_file.tmp");
    clear_error();
    // SAFETY: valid string.
    let h = unsafe { CreateFileW(file.as_ptr(), 0x4000_0000, 0, core::ptr::null(), CREATE_NEW, 0x80, core::ptr::null_mut()) };
    let fok = h as usize != INVALID;
    say!(t, "CreateFileW(CREATE_NEW) in SystemTemp = {fok} err {}", if fok { -2 } else { gle() });
    if fok {
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
        // SAFETY: valid string.
        let del = unsafe { DeleteFileW(file.as_ptr()) };
        say!(t, "DeleteFileW = {del}");
    }
    // opening an existing file in the directory for reading: none is known; a missing name inside:
    let missing = path("_missing");
    clear_error();
    // SAFETY: valid string.
    let h = unsafe { CreateFileW(missing.as_ptr(), 0x8000_0000, 7, core::ptr::null(), 3, 0x80, core::ptr::null_mut()) };
    say!(t, "open a missing name for reading = {} err {}", h as usize != INVALID, if h as usize != INVALID { -2 } else { gle() });
    // the user's profile temp is writable (control)
    let _ = MoveFileExW as usize;
}

// recorded with the standard (filtered) administrator token
