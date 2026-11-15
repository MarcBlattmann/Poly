//! GetPointerDevices (the pen and touch devices attached; a mouse is not one):
//! the two-step use (a NULL buffer to get the count, then a buffer); Chrome's
//! browser calls it 8 times in a run (S3), always for the count and the list.
//! On the oracle there is no pen or touch device. The size of one entry is
//! 1080 bytes (POINTER_DEVICE_INFO). With no devices: the count query gives
//! TRUE and 0, repeated calls agree, a call with a buffer and a count fails
//! (FALSE) and leaves the count as passed; a NULL count pointer fails with 87.
//! When devices exist (not on the oracle) the entries have a type of
//! POINTER_DEVICE_TYPE (1 integrated pen, 2 external pen, 3 touch, 4 touch
//! pad), a monitor and the number of contacts; the test prints them only
//! when the count is not 0, as booleans.
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
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

type HMONITOR = *mut c_void;
/// winuser.h (POINTER_DEVICE_INFO)
#[repr(C)]
#[derive(Clone, Copy)]
struct POINTER_DEVICE_INFO {
    display_orientation: u32,
    device: HANDLE,
    pointer_device_type: u32,
    monitor: HMONITOR,
    starting_cursor_id: u32,
    max_active_contacts: u16,
    product_string: [u16; 520],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetPointerDevices(count: *mut u32, devices: *mut POINTER_DEVICE_INFO) -> BOOL;
    fn GetPointerDevice(device: HANDLE, info: *mut POINTER_DEVICE_INFO) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: local buffers.
    unsafe {
        say!(t, "size of one POINTER_DEVICE_INFO: {}", core::mem::size_of::<POINTER_DEVICE_INFO>());
        let mut n = 0xAAu32;
        set_error(0xDEAD);
        let r = GetPointerDevices(&mut n, core::ptr::null_mut());
        say!(t, "count query: returns {r}, count {n} (the oracle has none), last error left alone {}", last_error() == 0xDEAD);
        let first = n;
        let mut same = true;
        for _ in 0..20 {
            let mut m = 0x55u32;
            same &= GetPointerDevices(&mut m, core::ptr::null_mut()) == 1 && m == first;
        }
        say!(t, "twenty more count queries agree: {same}");
        let mut m = 7u32;
        let r = GetPointerDevices(&mut m, core::ptr::null_mut());
        say!(t, "a count of 7 passed in with a NULL buffer: returns {r}, the count is replaced {}", m == first);
        // with a buffer
        let mut devs = [zeroed::<POINTER_DEVICE_INFO>(); 4];
        let mut c = 4u32;
        set_error(0xDEAD);
        let r = GetPointerDevices(&mut c, devs.as_mut_ptr());
        if first == 0 {
            say!(t, "with a buffer for 4 and no devices: returns {r}, count left as passed {}, buffer untouched {}", c == 4, devs[0].pointer_device_type == 0 && devs[0].display_orientation == 0);
        } else {
            say!(t, "with a buffer for 4: returns {r}, count is at most 4 and at least 1 {}", c >= 1 && c <= 4);
            for i in 0..(c as usize).min(4) {
                let d = &devs[i];
                say!(t, "device {i}: type in 1..=4 {}, has a monitor {}, product string not empty {}", (1..=4).contains(&d.pointer_device_type), !d.monitor.is_null(), d.product_string[0] != 0);
            }
        }
        let mut c = 0u32;
        let r = GetPointerDevices(&mut c, devs.as_mut_ptr());
        say!(t, "a count of 0 with a buffer: returns {r}, count {}", if first == 0 { c == 0 } else { c == first });
        set_error(0xDEAD);
        let r = GetPointerDevices(core::ptr::null_mut(), devs.as_mut_ptr());
        say!(t, "NULL count pointer: returns {r}, last error {}", last_error());
        set_error(0xDEAD);
        let r = GetPointerDevices(core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "both NULL: returns {r}, last error {}", last_error());
        // GetPointerDevice with a handle that is none
        let mut d: POINTER_DEVICE_INFO = zeroed();
        set_error(0xDEAD);
        let r = GetPointerDevice(0x1234usize as HANDLE, &mut d);
        say!(t, "GetPointerDevice with a garbage handle: returns {r}, last error {}", last_error());
        set_error(0xDEAD);
        let r = GetPointerDevice(core::ptr::null_mut(), &mut d);
        say!(t, "GetPointerDevice with NULL: returns {r}, last error {}", last_error());
    }
}
