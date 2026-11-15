//! GetStockObject for every index 0..19 and some out-of-range ones: whether
//! a handle comes back, its GetObjectType class, whether two calls return
//! the same handle, which indexes share a handle, and the last error after
//! an index that is not defined. Then DeleteObject on every stock object
//! (return value, the object still there, the same handle on the next
//! call), twice.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HGDIOBJ = *mut c_void;
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

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: stock objects are process-wide and owned by the system; the
    // calls only query them and try to delete them.
    unsafe {
        let mut all: [HGDIOBJ; 20] = [core::ptr::null_mut(); 20];
        for i in 0..20usize {
            all[i] = GetStockObject(i as i32);
        }
        for i in 0..20usize {
            let h = all[i];
            let again = GetStockObject(i as i32);
            let mut same = Buf::<100>::new();
            for j in 0..i {
                if !h.is_null() && all[j] == h {
                    let _ = write!(same, " {j}");
                }
            }
            say!(
                t,
                "stock {i:2}: null={} type={} stable={} same handle as:{}",
                h.is_null() as i32,
                GetObjectType(h),
                (h == again) as i32,
                same.as_str()
            );
        }
        for idx in [-1, 20, 21, 22, 31, 32, 255, 256, 1000, i32::MAX, i32::MIN] {
            set_error(0x5A5A);
            let h = GetStockObject(idx);
            let e = last_error();
            let ty = GetObjectType(h);
            say!(t, "index {idx}: null={} type={ty} error unchanged={} (error {})", h.is_null() as i32, (e == 0x5A5A) as i32, if e == 0x5A5A { 0 } else { e });
        }
        for i in 0..20usize {
            let h = all[i];
            if h.is_null() {
                continue;
            }
            set_error(0x5A5A);
            let r1 = DeleteObject(h);
            let e1 = last_error();
            let ty = GetObjectType(h);
            let r2 = DeleteObject(h);
            let still = GetStockObject(i as i32) == h;
            say!(t, "delete stock {i:2}: first={r1} (error unchanged {}) type after={ty} second={r2} same handle after={}", (e1 == 0x5A5A) as i32, still as i32);
        }
    }
}
