//! CoInitializeEx(NULL, flags) on threads of their own: S_OK the first time
//! and S_FALSE when the thread is already initialized the same way;
//! RPC_E_CHANGED_MODE (0x80010106) when it was initialized with the other
//! threading model; COINIT_APARTMENTTHREADED (2), COINIT_MULTITHREADED (0)
//! with COINIT_DISABLE_OLE1DDE (4) and COINIT_SPEED_OVER_MEMORY (8); the
//! reserved parameter (non-NULL gives E_INVALIDARG); flag values that are
//! not defined; the apartment type CoGetApartmentType gives after each.
//! Chrome's threads use the multi-threaded model for workers and the apartment
//! model (through OleInitialize) for the UI thread (S3).
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
const COINIT_MULTITHREADED: u32 = 0;
const COINIT_APARTMENTTHREADED: u32 = 2;
const COINIT_DISABLE_OLE1DDE: u32 = 4;
const COINIT_SPEED_OVER_MEMORY: u32 = 8;
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
}

poly_wtest::main!(test);

struct Out {
    v: [u32; 40],
    n: usize,
}
impl Out {
    fn put(&mut self, v: u32) {
        if self.n < 40 {
            self.v[self.n] = v;
            self.n += 1;
        }
    }
}

unsafe fn kind() -> u32 {
    let (mut k, mut q) = (-9i32, -9i32);
    // SAFETY: plain call, writable outputs.
    let r = unsafe { CoGetApartmentType(&mut k, &mut q) };
    if r == 0 { k as u32 } else { 0xFFFF_0000 | (r as u32 & 0xFFFF) }
}

unsafe extern "system" fn one(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread.
    unsafe {
        let o = &mut *(p as *mut Out);
        let flags = o.v[0];
        o.n = 0;
        o.put(hr(CoInitializeEx(core::ptr::null_mut(), flags)));
        o.put(kind());
        o.put(hr(CoInitializeEx(core::ptr::null_mut(), flags)));
        let other = if flags & 2 != 0 { 0 } else { 2 };
        o.put(hr(CoInitializeEx(core::ptr::null_mut(), other)));
        CoUninitialize();
        CoUninitialize();
        o.put(kind());
        CoUninitialize();
        o.put(kind());
        0
    }
}

unsafe extern "system" fn reserved(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread.
    unsafe {
        let o = &mut *(p as *mut Out);
        o.n = 0;
        o.put(hr(CoInitializeEx(0x10usize as *mut c_void, 0)));
        o.put(kind());
        for flags in [1u32, 3, 0x10, 0x20, 0x100, 0xFFFF_FFFF] {
            let r = CoInitializeEx(core::ptr::null_mut(), flags);
            o.put(hr(r));
            if r >= 0 {
                o.put(kind());
                CoUninitialize();
            }
        }
        0
    }
}

fn test(t: &mut Test) {
    for (name, flags) in [("COINIT_APARTMENTTHREADED", COINIT_APARTMENTTHREADED), ("COINIT_MULTITHREADED", COINIT_MULTITHREADED), ("APARTMENTTHREADED | DISABLE_OLE1DDE", COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE), ("MULTITHREADED | DISABLE_OLE1DDE | SPEED_OVER_MEMORY", COINIT_MULTITHREADED | COINIT_DISABLE_OLE1DDE | COINIT_SPEED_OVER_MEMORY), ("APARTMENTTHREADED | SPEED_OVER_MEMORY", COINIT_APARTMENTTHREADED | COINIT_SPEED_OVER_MEMORY)] {
        let mut o = Out { v: [0; 40], n: 0 };
        o.v[0] = flags;
        on_thread(one, &mut o as *mut Out as *mut c_void);
        say!(t, "{name}: first {:#x}, apartment type {:#x}, again {:#x}, other model {:#x}, after two uninitialize {:#x}, after the third {:#x}", o.v[0], o.v[1], o.v[2], o.v[3], o.v[4], o.v[5]);
    }
    let mut o = Out { v: [0; 40], n: 0 };
    on_thread(reserved, &mut o as *mut Out as *mut c_void);
    say!(t, "reserved parameter not NULL: {:#x}, apartment afterwards {:#x} (0xffff.. means not initialized: low bits of the HRESULT)", o.v[0], o.v[1]);
    let mut b = Buf::<250>::new();
    for i in 2..o.n {
        let _ = write!(b, "{:#x} ", o.v[i]);
    }
    say!(t, "flags 1, 3, 0x10, 0x20, 0x100, 0xFFFFFFFF in turn (HRESULT and, when it worked, the apartment type): {}", b.as_str().trim_end());
}
