//! SystemParametersInfoW and the last error: calls that do not clear it before the first SystemParametersInfoW,
//! which then does (once).
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
type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
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

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
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

type HCURSOR = *mut c_void;
type HKL = *mut c_void;
const IDC_ARROW: usize = 32512;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDesktopWindow() -> HWND;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSysColor(index: i32) -> u32;
    fn LoadCursorW(instance: HINSTANCE, name: *const u16) -> HCURSOR;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn GetKeyboardLayout(thread: u32) -> HKL;
}

poly_wtest::main!(test);

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
}

fn spi(t: &mut Test, name: &str, action: u32) {
    let mut b = [0u8; 16];
    b[..4].copy_from_slice(&0xA5A5_A5A5u32.to_le_bytes());
    set_error(0x1234);
    // SAFETY: b is 16 bytes, enough for an int.
    let ok = unsafe { SystemParametersInfoW(action, 0, b.as_mut_ptr() as *mut c_void, 0) };
    say!(t, "{name} ({:#x}): ok {ok} last error {:#x} value {:#x}", action, last_error(), u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
}

fn probe(t: &mut Test, what: &str, f: impl FnOnce()) {
    set_error(0x1234);
    f();
    say!(t, "after {what}: last error {:#x}", last_error());
}

fn test(t: &mut Test) {
    probe(t, "GetCursorPos", || { let mut p = POINT::default(); unsafe { GetCursorPos(&mut p) }; });
    probe(t, "GetDesktopWindow", || { unsafe { GetDesktopWindow() }; });
    probe(t, "GetSysColor(5)", || { unsafe { GetSysColor(5) }; });
    probe(t, "GetKeyboardLayout(0)", || { unsafe { GetKeyboardLayout(0) }; });
    probe(t, "LoadCursorW(IDC_ARROW)", || { unsafe { LoadCursorW(core::ptr::null_mut(), 32512 as *const u16) }; });
    spi(t, "GETSNAPTODEFBUTTON", 0x5F);
    spi(t, "GETSNAPTODEFBUTTON", 0x5F);
    probe(t, "GetSystemMetrics(1)", || { unsafe { GetSystemMetrics(1) }; });
}
