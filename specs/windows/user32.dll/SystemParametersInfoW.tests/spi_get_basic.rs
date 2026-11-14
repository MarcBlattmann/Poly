//! The first 31 of the read-only SPI_GET* actions of the sheet's getters rule and the seven profile-dependent ones: the
//! values returned (pointer to an int/BOOL/UINT) and the last error (unchanged
//! after a warm-up call). Seven of the settings depend on the user profile
//! (the oracle's three profiles differ in them): for those the test prints only
//! whether the value is in the range the setting allows. The others had the
//! same value in all three profiles and are printed as they are.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // The first call of a process may clear the last error (a desktop session); one call first
    // makes the lines below the same in every session.
    let mut warm = [0u8; 16];
    // SAFETY: warm is 16 bytes, enough for an int.
    unsafe { SystemParametersInfoW(0x0A, 0, warm.as_mut_ptr() as *mut c_void, 0) };
    // (name, action, max): max < 0: print the value; max >= 0: print only whether it is within 0..=max,
    // or (max = 0x1F00) whether it has no bits outside 0x1F.
    let list: &[(&str, u32, i64)] = &[
        ("GETBEEP", 0x01, -1), ("GETMOUSE", 0x03, -1), ("GETBORDER", 0x05, -1), ("GETKEYBOARDSPEED", 0x0A, -1),
        ("GETSCREENSAVEACTIVE", 0x10, 1), ("GETKEYBOARDDELAY", 0x16, -1), ("GETICONTITLEWRAP", 0x19, -1),
        ("GETMENUDROPALIGNMENT", 0x1B, 1), ("GETDRAGFULLWINDOWS", 0x26, -1), ("GETSNAPTODEFBUTTON", 0x5F, -1),
        ("GETMOUSEHOVERWIDTH", 0x62, -1), ("GETMOUSEHOVERHEIGHT", 0x64, -1), ("GETMOUSEHOVERTIME", 0x66, -1),
        ("GETWHEELSCROLLLINES", 0x68, -1), ("GETMENUSHOWDELAY", 0x6A, -1), ("GETWHEELSCROLLCHARS", 0x6C, -1),
        ("GETSHOWIMEUI", 0x6E, -1), ("GETMOUSESPEED", 0x70, -1), ("GETSCREENREADER", 0x46, -1), ("GETFONTSMOOTHING", 0x4A, -1),
        ("GETKEYBOARDPREF", 0x44, -1), ("GETFONTSMOOTHINGTYPE", 0x200A, -1), ("GETFONTSMOOTHINGCONTRAST", 0x200C, -1),
        ("GETFOCUSBORDERWIDTH", 0x200E, -1), ("GETFOCUSBORDERHEIGHT", 0x2010, -1), ("GETCARETWIDTH", 0x2006, -1),
        ("GETACTIVEWINDOWTRACKING", 0x1000, -1), ("GETMENUANIMATION", 0x1002, -1), ("GETCOMBOBOXANIMATION", 0x1004, -1),
        ("GETLISTBOXSMOOTHSCROLLING", 0x1006, -1), ("GETGRADIENTCAPTIONS", 0x1008, -1), ("GETCURSORSHADOW", 0x101A, 1),
        ("GETDROPSHADOW", 0x1024, 1), ("GETCONTACTVISUALIZATION", 0x2018, 2), ("GETGESTUREVISUALIZATION", 0x201A, 7936),
        ("GETMOUSEWHEELROUTING", 0x201C, 2),
    ];
    for &(name, a, max) in list {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&0xA5A5_A5A5u32.to_le_bytes());
        set_error(0x1234);
        // SAFETY: b is 16 bytes, enough for an int.
        let ok = unsafe { SystemParametersInfoW(a, 0, b.as_mut_ptr() as *mut c_void, 0) };
        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if max < 0 {
            say!(t, "SPI_{name} ({:#x}): ok {ok} last error {:#x} value {:#x}", a, last_error(), v);
        } else if max == 0x1F00 {
            say!(t, "SPI_{name} ({:#x}): ok {ok} last error {:#x} only flag bits 0..4 set: {}", a, last_error(), v & !0x1F == 0);
        } else {
            say!(t, "SPI_{name} ({:#x}): ok {ok} last error {:#x} value within 0..={max}: {}", a, last_error(), (v as i64) <= max);
        }
    }
}
