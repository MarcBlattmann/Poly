//! EnumDisplayDevicesW(adapter name, 0, ..): the monitor attached to the primary
//! adapter: name "\\.\DISPLAY1\Monitor0", StateFlags ACTIVE|ATTACHED (0x3), TRUE;
//! index 1 gives FALSE; with EDD_GET_DEVICE_INTERFACE_NAME (1) the DeviceID is a
//! device interface path starting with \\?\DISPLAY# instead of the PnP id
//! (MONITOR\...); an adapter without a monitor (not attached) gives FALSE for
//! every index.
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

/// wingdi.h DISPLAY_DEVICEW
#[repr(C)]
#[derive(Clone, Copy)]
struct DISPLAY_DEVICEW {
    cb: u32,
    device_name: [u16; 32],
    device_string: [u16; 128],
    state_flags: u32,
    device_id: [u16; 128],
    device_key: [u16; 128],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumDisplayDevicesW(device: *const u16, index: u32, dd: *mut DISPLAY_DEVICEW, flags: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: the structures are writable.
    unsafe {
        let mut dd: DISPLAY_DEVICEW = zeroed();
        dd.cb = core::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        EnumDisplayDevicesW(core::ptr::null(), 0, &mut dd, 0);
        for flags in [0u32, 1] {
            for j in 0..3u32 {
                let mut md: DISPLAY_DEVICEW = zeroed();
                md.cb = core::mem::size_of::<DISPLAY_DEVICEW>() as u32;
                set_error(0x1234);
                let ok = EnumDisplayDevicesW(dd.device_name.as_ptr(), j, &mut md, flags);
                let (mut a, mut c) = (Buf::<300>::new(), Buf::<300>::new());
                md.device_name.iter().position(|&x| x == 0).map(|l| ascii_of(&md.device_name[..l], &mut a));
                md.device_id.iter().position(|&x| x == 0).map(|l| ascii_of(&md.device_id[..l], &mut c));
                let id = c.as_str();
                say!(
                    t,
                    "flags {flags} monitor {j}: ok {ok} last error {:#x} name '{}' flags {:#x} id starts with MONITOR\\ {} id starts with \\\\?\\DISPLAY# {}",
                    last_error(),
                    a.as_str(),
                    md.state_flags,
                    id.starts_with("MONITOR\\"),
                    id.starts_with("\\\\?\\DISPLAY#")
                );
            }
        }
    }
}
