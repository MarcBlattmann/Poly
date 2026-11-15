//! OleInitialize and OleUninitialize on a thread that has no COM state, run
//! on threads of their own (each starts uninitialized): S_OK the first time and
//! S_FALSE for each nested call; the reserved parameter must be NULL
//! (E_INVALIDARG); OleInitialize makes the thread a single-threaded apartment
//! (CoGetApartmentType gives APTTYPE_MAINSTA 3 for the first thread that
//! initializes); a thread already initialized as multi-threaded gets
//! RPC_E_CHANGED_MODE; an apartment thread already initialized by
//! CoInitializeEx still gets S_OK (OLE is a state of its own); the count of
//! OleInitialize calls must be matched by OleUninitialize calls before the
//! apartment goes. Chrome's UI thread calls OleInitialize once and OleUninitialize at
//! the end (S3).
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
const S_OK: i32 = 0;
const CO_E_NOTINITIALIZED: i32 = 0x8004_01F0u32 as i32;
const COINIT_MULTITHREADED: u32 = 0;
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
    fn OleInitialize(reserved: *mut c_void) -> HRESULT;
    fn OleUninitialize();
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> HRESULT;
}

poly_wtest::main!(test);

struct Out {
    lines: [(u32, u32); 24],
    n: usize,
}
impl Out {
    fn put(&mut self, a: u32, b: u32) {
        if self.n < 24 {
            self.lines[self.n] = (a, b);
            self.n += 1;
        }
    }
}

unsafe fn apt() -> (i32, i32, i32) {
    let (mut k, mut q) = (-9, -9);
    // SAFETY: plain call, writable outputs.
    let r = unsafe { CoGetApartmentType(&mut k, &mut q) };
    (r, k, q)
}

unsafe extern "system" fn nested(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread; COM calls on its own thread.
    unsafe {
        let o = &mut *(p as *mut Out);
        let (r, ..) = apt();
        o.put(1, hr(r)); // before: not initialized
        o.put(2, hr(OleInitialize(core::ptr::null_mut())));
        let (r, k, _) = apt();
        o.put(3, hr(r));
        o.put(4, k as u32);
        o.put(5, hr(OleInitialize(core::ptr::null_mut())));
        o.put(6, hr(OleInitialize(core::ptr::null_mut())));
        OleUninitialize();
        OleUninitialize();
        let (r, ..) = apt();
        o.put(7, hr(r)); // still initialized: one level left
        OleUninitialize();
        let (r, ..) = apt();
        o.put(8, hr(r)); // gone
        0
    }
}

unsafe extern "system" fn badarg(p: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe {
        let o = &mut *(p as *mut Out);
        o.put(1, hr(OleInitialize(0x10usize as *mut c_void)));
        let (r, ..) = apt();
        o.put(2, hr(r)); // not initialized by the failed call
        0
    }
}

unsafe extern "system" fn after_mta(p: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe {
        let o = &mut *(p as *mut Out);
        o.put(1, hr(CoInitializeEx(core::ptr::null_mut(), COINIT_MULTITHREADED)));
        o.put(2, hr(OleInitialize(core::ptr::null_mut())));
        let (_, k, _) = apt();
        o.put(3, k as u32);
        CoUninitialize();
        0
    }
}

unsafe extern "system" fn after_sta(p: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe {
        let o = &mut *(p as *mut Out);
        o.put(1, hr(CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED)));
        o.put(2, hr(OleInitialize(core::ptr::null_mut())));
        let (_, k, _) = apt();
        o.put(3, k as u32);
        OleUninitialize();
        let (r, ..) = apt();
        o.put(4, hr(r)); // the CoInitializeEx level is left
        CoUninitialize();
        let (r, ..) = apt();
        o.put(5, hr(r));
        0
    }
}

fn show(t: &mut Test, what: &str, o: &Out, names: &[&str]) {
    for i in 0..o.n {
        let (idx, v) = o.lines[i];
        say!(t, "{what}: {} = {v:#x}", names[idx as usize - 1]);
    }
}

fn test(t: &mut Test) {
    let mut o = Out { lines: [(0, 0); 24], n: 0 };
    on_thread(nested, &mut o as *mut Out as *mut c_void);
    show(t, "nesting", &o, &["apartment type before (CO_E_NOTINITIALIZED)", "first OleInitialize", "apartment type call afterwards", "apartment type (0 STA, 1 MTA, 3 MAINSTA)", "second OleInitialize", "third OleInitialize", "apartment after two OleUninitialize (still initialized: S_OK)", "apartment after the last OleUninitialize (CO_E_NOTINITIALIZED)"]);
    let mut o = Out { lines: [(0, 0); 24], n: 0 };
    on_thread(badarg, &mut o as *mut Out as *mut c_void);
    show(t, "reserved parameter", &o, &["OleInitialize(non-NULL)", "apartment afterwards"]);
    let mut o = Out { lines: [(0, 0); 24], n: 0 };
    on_thread(after_mta, &mut o as *mut Out as *mut c_void);
    show(t, "after CoInitializeEx(MTA)", &o, &["CoInitializeEx", "OleInitialize", "apartment type"]);
    let mut o = Out { lines: [(0, 0); 24], n: 0 };
    on_thread(after_sta, &mut o as *mut Out as *mut c_void);
    show(t, "after CoInitializeEx(STA)", &o, &["CoInitializeEx", "OleInitialize", "apartment type", "apartment after OleUninitialize (S_OK: the CoInitializeEx level is left)", "apartment after CoUninitialize"]);
}
