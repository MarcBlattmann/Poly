//! EnumDisplaySettingsW(NULL, i) for i = 0, 1, 2, ...: TRUE for each mode until
//! FALSE past the last (no error set); the modes are 32-bit entries sorted by
//! size (first 800x600 at 60 Hz) and the current mode (1920x1080, 32 bits, 60 Hz)
//! is one of them; dmFields for a listed mode lacks the position bit; the
//! device name of a listed mode is "cdd" in lower case.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
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

/// wingdi.h DEVMODEW (the display variant of the union).
#[repr(C)]
#[derive(Clone, Copy)]
struct DEVMODEW {
    device_name: [u16; 32],
    spec_version: u16,
    driver_version: u16,
    size: u16,
    driver_extra: u16,
    fields: u32,
    pos_x: i32,
    pos_y: i32,
    display_orientation: u32,
    display_fixed_output: u32,
    color: i16,
    duplex: i16,
    y_resolution: i16,
    tt_option: i16,
    collate: i16,
    form_name: [u16; 32],
    log_pixels: u16,
    bits_per_pel: u32,
    pels_width: u32,
    pels_height: u32,
    display_flags: u32,
    display_frequency: u32,
    icm_method: u32,
    icm_intent: u32,
    media_type: u32,
    dither_type: u32,
    reserved1: u32,
    reserved2: u32,
    panning_width: u32,
    panning_height: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumDisplaySettingsW(device: *const u16, mode: u32, dm: *mut DEVMODEW) -> BOOL;
}

poly_wtest::main!(test);

fn ws(buf: &[u16], out: &mut Buf<300>) {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    ascii_of(&buf[..n], out);
}

fn fresh() -> DEVMODEW {
    let mut dm: DEVMODEW = zeroed();
    dm.size = core::mem::size_of::<DEVMODEW>() as u16;
    dm.pels_width = 0xDEAD;
    dm.pos_x = 0x77;
    dm
}

fn show_dm(t: &mut Test, label: &str, ok: BOOL, dm: &DEVMODEW) {
    say!(
        t,
        "{label}: ok {} last error {:#x} specVersion {:#x} dmSize {} dmDriverExtra {} dmFields {:#x}",
        ok,
        last_error(),
        dm.spec_version,
        dm.size,
        dm.driver_extra,
        dm.fields
    );
    say!(
        t,
        "   position {},{} orientation {} fixed output {} bits {} {}x{} flags {:#x} frequency {}",
        dm.pos_x,
        dm.pos_y,
        dm.display_orientation,
        dm.display_fixed_output,
        dm.bits_per_pel,
        dm.pels_width,
        dm.pels_height,
        dm.display_flags,
        dm.display_frequency
    );
}

fn test(t: &mut Test) {
    // SAFETY: the structures are writable.
    unsafe {
        let mut cur = fresh();
        EnumDisplaySettingsW(core::ptr::null(), 0xFFFF_FFFF, &mut cur);
        let mut n = 0u32;
        let mut ended = 1;
        let mut found_current = false;
        let mut sorted = true;
        let mut prev = (0u32, 0u32);
        let mut first = fresh();
        let mut all32 = true;
        for i in 0..5000 {
            let mut dm = fresh();
            set_error(0x1234);
            ended = EnumDisplaySettingsW(core::ptr::null(), i, &mut dm);
            if ended == 0 {
                say!(t, "past the last mode: FALSE, last error {:#x}", last_error());
                break;
            }
            if n == 0 {
                first = dm;
            }
            if dm.pels_width == cur.pels_width
                && dm.pels_height == cur.pels_height
                && dm.bits_per_pel == cur.bits_per_pel
                && dm.display_frequency == cur.display_frequency
            {
                found_current = true;
            }
            if (dm.pels_width, dm.pels_height) < prev {
                sorted = false;
            }
            prev = (dm.pels_width, dm.pels_height);
            if dm.bits_per_pel != 32 {
                all32 = false;
            }
            n += 1;
        }
        t.check("more than 0 modes", n > 0, n as i64);
        say!(
            t,
            "enumeration ended with FALSE: {} ; current mode among the modes: {} ; sorted by size: {} ; all 32 bits: {}",
            ended == 0,
            found_current,
            sorted,
            all32
        );
        let mut name = Buf::<300>::new();
        ws(&first.device_name, &mut name);
        show_dm(t, "mode 0", 1, &first);
        say!(t, "   device name '{}'", name.as_str());
    }
}
