//! RegisterClipboardFormatW failures: an empty name returns 0 and leaves the last error as it
//! was; NULL returns 0 with ERROR_INVALID_PARAMETER (87); a name of 256 units or more returns 0
//! with ERROR_MORE_DATA (234); 255 units still work; after a success the last error is not
//! reliable (not printed).
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
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    cl_clean();
    // SAFETY: plain calls.
    unsafe {
        set_error(0x1234);
        let r = RegisterClipboardFormatW(wz!("").as_ptr());
        say!(t, "empty name: {} err {:#x}", r, last_error());
        set_error(0x1234);
        let r = RegisterClipboardFormatW(core::ptr::null());
        say!(t, "NULL name: {} err {}", r, last_error());
        for (n, name) in
            [(254usize, "254 units"), (255, "255 units"), (256, "256 units"), (257, "257 units"), (300, "300 units")]
        {
            let mut long = [0x62u16; 301];
            long[n] = 0;
            set_error(0x1234);
            let r = RegisterClipboardFormatW(long.as_ptr());
            if r >= 0xC000 {
                say!(t, "{name}: registered");
            } else {
                say!(t, "{name}: {} err {}", r, last_error());
            }
        }
        // a name made of a single unit and of control characters
        let r = RegisterClipboardFormatW([0x01u16, 0x02, 0].as_ptr());
        say!(t, "control characters in the name: registered {}", r >= 0xC000);
        let r1 = RegisterClipboardFormatW([0x20u16, 0].as_ptr());
        say!(t, "a single space: registered {}", r1 >= 0xC000);
    }
    cl_clean();
    t.check("done", true, 0);
}
