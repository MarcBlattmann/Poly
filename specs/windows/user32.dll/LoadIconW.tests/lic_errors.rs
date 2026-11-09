//! LoadIconW failures: a module that has no such icon resource (the
//! executable itself, and a made-up module handle) gives NULL with
//! ERROR_RESOURCE_DATA_NOT_FOUND (1812) for every numeric id and string
//! name tried; a NULL module with an id or name that is not a system icon
//! gives ERROR_RESOURCE_TYPE_NOT_FOUND (1813), also for the string
//! "IDI_APPLICATION" and for the id 0 (a NULL name).
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
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

type HICON = *mut c_void;
/// winuser.h: system icon ids (IDI_*) and cursor ids (OCR_*).
const IDI_APPLICATION: usize = 32512;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn LoadIconW(inst: HINSTANCE, name: *const u16) -> HICON;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe {
        let null = core::ptr::null_mut();
        for (lbl, inst, name) in [
            ("own module, id 1", instance(), 1usize as *const u16),
            ("own module, id 0x7777", instance(), 0x7777usize as *const u16),
            ("own module, id 32512", instance(), 32512usize as *const u16),
            ("own module, name NOSUCH", instance(), wz!("NOSUCH").as_ptr()),
            ("NULL module, id 0x7777", null, 0x7777usize as *const u16),
            ("NULL module, id 0", null, core::ptr::null()),
            ("NULL module, name NOSUCH", null, wz!("NOSUCH").as_ptr()),
            ("NULL module, name IDI_APPLICATION", null, wz!("IDI_APPLICATION").as_ptr()),
            ("made-up module, id 1", 0x0bad_0000usize as HINSTANCE, 1usize as *const u16),
            ("made-up module, id 32512", 0x0bad_0000usize as HINSTANCE, 32512usize as *const u16),
        ] {
            set_error(0x1234);
            let h = LoadIconW(inst, name);
            say!(t, "{lbl}: {} error {}", hwnd_kind(h), last_error());
        }
    }
}
