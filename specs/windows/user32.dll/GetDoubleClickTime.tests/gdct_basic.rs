//! GetDoubleClickTime (3 calls in a Chrome run): the value is the maximum
//! time in milliseconds between the two clicks of a double click: printed as
//! a class (within 200..5000: the range the system accepts), the same value
//! on repeated calls and from another thread, the double click rectangle
//! metrics are positive, and the last error is left alone by the call. No setter is called (the value is a
//! system-wide user setting).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
const INFINITE: u32 = 0xFFFF_FFFF;
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
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

const SM_CXDOUBLECLK: i32 = 36;
const SM_CYDOUBLECLK: i32 = 37;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDoubleClickTime() -> u32;
}

static OTHER: Global<u32> = Global::new(0);

unsafe extern "system" fn thread_proc(_p: *mut c_void) -> u32 {
    // SAFETY: plain call.
    *OTHER.get() = unsafe { GetDoubleClickTime() };
    0
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls throughout; the thread is ours.
    unsafe {
        set_error(0xBEEF);
        let a = GetDoubleClickTime();
        say!(t, "last error after the call: {:#x}", last_error());
        let b = GetDoubleClickTime();
        say!(t, "within 200..5000: {}", (200..=5000).contains(&a) as i32);
        say!(t, "equal on a second call: {}", (a == b) as i32);
        say!(t, "not 0 and not INFINITE: {}", (a != 0 && a != INFINITE) as i32);

        let mut id = 0u32;
        let th = CreateThread(core::ptr::null_mut(), 0, thread_proc, core::ptr::null_mut(), 0, &mut id);
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
        say!(t, "equal from another thread: {}", (*OTHER.get() == a) as i32);

        say!(t, "SM_CXDOUBLECLK is positive: {}", (GetSystemMetrics(SM_CXDOUBLECLK) > 0) as i32);
        say!(t, "SM_CYDOUBLECLK is positive: {}", (GetSystemMetrics(SM_CYDOUBLECLK) > 0) as i32);
        let mut sum = 0u32;
        for _ in 0..1000 {
            sum = sum.wrapping_add((GetDoubleClickTime() == a) as u32);
        }
        say!(t, "1000 further calls all equal: {}", (sum == 1000) as i32);
    }
}
