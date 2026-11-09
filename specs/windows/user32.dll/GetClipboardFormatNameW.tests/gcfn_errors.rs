//! GetClipboardFormatNameW errors: predefined formats (CF_TEXT, CF_UNICODETEXT, CF_DIB), 0,
//! private formats and other numbers below 0xC000 give 0 with ERROR_INVALID_PARAMETER (87);
//! a number in the registered range that is no registered name gives 0 with ERROR_INVALID_HANDLE
//! (6); a negative buffer size gives 0 with ERROR_NOACCESS (998); the buffer is left alone on
//! every failure.
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
type BOOL = i32;
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

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
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

const CF_TEXT: u32 = 1;
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_OWNERDISPLAY: u32 = 0x0080;
const CF_PRIVATEFIRST: u32 = 0x0200;
/// Open the clipboard for `hwnd` (NULL: no owner window), retrying for up
/// to two seconds: another program on the desktop may hold it for a moment.
fn cl_open(hwnd: HWND) -> bool {
    for _ in 0..40 {
        // SAFETY: plain call.
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return true;
        }
        // SAFETY: plain call.
        unsafe { Sleep(50) };
    }
    false
}

/// Empty the clipboard and close it (open with retries, owner NULL).
fn cl_clean() {
    if cl_open(core::ptr::null_mut()) {
        // SAFETY: the clipboard is open.
        unsafe {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn Sleep(ms: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn OpenClipboard(hwnd: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn GetClipboardFormatNameW(format: u32, buf: *mut u16, max: i32) -> i32;
}

poly_wtest::main!(test);

fn try_name(f: u32, max: i32, t: &mut Test, label: &str) {
    let mut buf = [0x5A5Au16; 64];
    set_error(0x1234);
    // SAFETY: buf has 64 units; max <= 64.
    let n = unsafe { GetClipboardFormatNameW(f, buf.as_mut_ptr(), max) };
    let e = last_error();
    let touched = buf.iter().any(|&c| c != 0x5A5A);
    say!(t, "{label}: returns {n} err {e} buffer written {touched}");
}

fn test(t: &mut Test) {
    cl_clean();
    // SAFETY: plain calls.
    unsafe {
        let _ = RegisterClipboardFormatW(wz!("PolyTest Name Errors").as_ptr());
        try_name(CF_TEXT, 64, t, "CF_TEXT");
        try_name(CF_UNICODETEXT, 64, t, "CF_UNICODETEXT");
        try_name(CF_DIB, 64, t, "CF_DIB");
        try_name(CF_HDROP, 64, t, "CF_HDROP");
        try_name(0, 64, t, "0");
        try_name(CF_PRIVATEFIRST, 64, t, "private");
        try_name(CF_OWNERDISPLAY, 64, t, "CF_OWNERDISPLAY");
        try_name(0x1234, 64, t, "0x1234");
        try_name(0xBFFF, 64, t, "0xBFFF");
        try_name(0xFFFF, 64, t, "0xFFFF (not registered)");
        try_name(0xFFFF_FFFF, 64, t, "-1");
        try_name(0x1_0000, 64, t, "0x10000");
        let f = RegisterClipboardFormatW(wz!("PolyTest Name Errors").as_ptr());
        try_name(f, -1, t, "valid format, size -1");
        try_name(f, 0, t, "valid format, size 0");
        try_name(f, 1, t, "valid format, size 1");
    }
    cl_clean();
    t.check("done", true, 0);
}
