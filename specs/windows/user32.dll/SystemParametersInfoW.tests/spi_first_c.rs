//! SystemParametersInfoW and the last error: GetSystemMetrics comes first and clears it; the later
//! SystemParametersInfoW calls leave it alone, also in a second thread.
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

static TH_OUT: Global<[u32; 3]> = Global::new([0; 3]);

unsafe extern "system" fn th_main(_p: *mut c_void) -> u32 {
    let mut b = [0u8; 16];
    for i in 0..3 {
        set_error(0x1234);
        // SAFETY: b is 16 bytes.
        unsafe { SystemParametersInfoW(if i == 1 { 3 } else { 0x16 }, 0, b.as_mut_ptr() as *mut c_void, 0) };
        TH_OUT.get()[i] = last_error();
    }
    0
}

fn run_thread(t: &mut Test) {
    let mut tid = 0u32;
    // SAFETY: the thread procedure above; the handle is closed.
    unsafe {
        let th = CreateThread(core::ptr::null_mut(), 0, th_main, core::ptr::null_mut(), 0, &mut tid);
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
    }
    let o = TH_OUT.get();
    say!(t, "second thread, three calls: last errors {:#x} {:#x} {:#x}", o[0], o[1], o[2]);
}

fn test(t: &mut Test) {
    probe(t, "GetSystemMetrics(0)", || { unsafe { GetSystemMetrics(0) }; });
    probe(t, "GetSystemMetrics(1)", || { unsafe { GetSystemMetrics(1) }; });
    spi(t, "GETMOUSEHOVERTIME", 0x66);
    spi(t, "GETMENUSHOWDELAY", 0x6A);
    run_thread(t);
}
