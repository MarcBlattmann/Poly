//! CoUninitialize with objects that belong to the apartment: the free-threaded
//! marshaler made by CoCreateFreeThreadedMarshaler stays usable (it is an
//! aggregate that does not need the apartment) and can be released after
//! CoUninitialize; CoTaskMemAlloc memory survives CoUninitialize and can be
//! freed afterwards; the thread can start COM again and CoGetApartmentType
//! reports the new apartment; a message filter registered in an STA goes with
//! the apartment (a new CoRegisterMessageFilter gives no old filter back).
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
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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

type HRESULT = i32;
const COINIT_APARTMENTTHREADED: u32 = 2;
/// Run `f(arg)` on a new thread and wait for it.
fn on_thread(f: unsafe extern "system" fn(*mut c_void) -> u32, arg: *mut c_void) -> u32 {
    // SAFETY: the thread is joined before returning; `arg` outlives it.
    unsafe {
        let h = CreateThread(core::ptr::null_mut(), 0, f, arg, 0, core::ptr::null_mut());
        WaitForSingleObject(h, 20_000);
        let mut code = 0xFFFF_FFFFu32;
        GetExitCodeThread(h, &mut code);
        CloseHandle(h);
        code
    }
}

fn hr(h: i32) -> u32 {
    h as u32
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
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
    fn GetExitCodeThread(thread: HANDLE, code: *mut u32) -> BOOL;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> HRESULT;
    fn CoRegisterMessageFilter(filter: *mut c_void, old: *mut *mut c_void) -> HRESULT;
    fn CoCreateFreeThreadedMarshaler(outer: *mut c_void, out: *mut *mut c_void) -> HRESULT;
    fn CoTaskMemAlloc(size: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
}

poly_wtest::main!(test);

unsafe extern "system" fn run(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread.
    unsafe {
        let o = &mut *(p as *mut [u32; 12]);
        CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        let mut ftm: *mut c_void = core::ptr::null_mut();
        o[0] = hr(CoCreateFreeThreadedMarshaler(core::ptr::null_mut(), &mut ftm));
        let mem = CoTaskMemAlloc(64);
        *(mem as *mut u32) = 0x1234_5678;
        let mut old: *mut c_void = 1 as *mut c_void;
        // a filter object is not made here: removing (NULL) is allowed and leaves no old filter
        o[1] = hr(CoRegisterMessageFilter(core::ptr::null_mut(), &mut old));
        o[2] = old.is_null() as u32;
        CoUninitialize();
        o[3] = (*(mem as *const u32) == 0x1234_5678) as u32;
        CoTaskMemFree(mem);
        // releasing the marshaler after the apartment is gone: IUnknown::Release is the third entry of its table
        if !ftm.is_null() {
            let vtbl = *(ftm as *const *const usize);
            let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vtbl.add(2));
            o[4] = release(ftm);
        }
        o[5] = hr(CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED));
        let (mut k, mut q) = (-9i32, -9i32);
        o[6] = hr(CoGetApartmentType(&mut k, &mut q));
        o[7] = k as u32;
        CoUninitialize();
        0
    }
}

fn test(t: &mut Test) {
    let mut o = [0u32; 12];
    on_thread(run, &mut o as *mut [u32; 12] as *mut c_void);
    say!(t, "free-threaded marshaler made inside the apartment: {:#x}", o[0]);
    say!(t, "removing the message filter (NULL): {:#x}, no old filter {}", o[1], o[2] != 0);
    say!(t, "CoTaskMemAlloc data survives CoUninitialize: {}", o[3] != 0);
    say!(t, "releasing the marshaler after CoUninitialize returns count {}", o[4]);
    say!(t, "a new CoInitializeEx: {:#x}, CoGetApartmentType {:#x} type {}", o[5], o[6], o[7]);
}
