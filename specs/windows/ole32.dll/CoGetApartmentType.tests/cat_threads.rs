//! CoGetApartmentType from several threads at once: each thread sees its own
//! apartment (two STA threads and an MTA thread at the same time), the value
//! does not change while the thread keeps its apartment, calls made from a
//! window procedure of an STA thread (during a sent message) report the
//! thread's apartment, and a thread's apartment type is the same on every call
//! (a hundred calls). The first STA thread of the process is the main STA
//! (type 3).
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
    fn Sleep(ms: u32);
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> HRESULT;
}

poly_wtest::main!(test);

struct Shared {
    ready: core::sync::atomic::AtomicU32,
    go: core::sync::atomic::AtomicU32,
    kinds: [core::sync::atomic::AtomicU32; 4],
}

static SH: Shared = Shared {
    ready: core::sync::atomic::AtomicU32::new(0),
    go: core::sync::atomic::AtomicU32::new(0),
    kinds: [core::sync::atomic::AtomicU32::new(99), core::sync::atomic::AtomicU32::new(99), core::sync::atomic::AtomicU32::new(99), core::sync::atomic::AtomicU32::new(99)],
};

unsafe extern "system" fn member(p: *mut c_void) -> u32 {
    use core::sync::atomic::Ordering::SeqCst;
    // SAFETY: statics shared with the main thread; COM on this thread only.
    unsafe {
        let idx = p as usize;
        let flags = if idx == 2 { COINIT_MULTITHREADED } else { COINIT_APARTMENTTHREADED };
        CoInitializeEx(core::ptr::null_mut(), flags);
        SH.ready.fetch_add(1, SeqCst);
        while SH.go.load(SeqCst) == 0 {
            Sleep(5);
        }
        let mut same = true;
        let (mut first, mut q) = (-9i32, -9i32);
        CoGetApartmentType(&mut first, &mut q);
        for _ in 0..100 {
            let mut k = -9i32;
            CoGetApartmentType(&mut k, &mut q);
            same &= k == first;
        }
        SH.kinds[idx].store(first as u32 | if same { 0 } else { 0x100 }, SeqCst);
        // hold the apartment until everyone has looked
        SH.ready.fetch_add(10, SeqCst);
        while SH.go.load(SeqCst) < 2 {
            Sleep(5);
        }
        CoUninitialize();
        0
    }
}

fn test(t: &mut Test) {
    use core::sync::atomic::Ordering::SeqCst;
    // SAFETY: our threads, joined before the end.
    unsafe {
        // the first STA thread must be alone for a moment to be the main STA
        let mut hs = [core::ptr::null_mut::<c_void>(); 3];
        hs[0] = CreateThread(core::ptr::null_mut(), 0, member, 0 as *mut c_void, 0, core::ptr::null_mut());
        while SH.ready.load(SeqCst) < 1 {
            Sleep(5);
        }
        hs[1] = CreateThread(core::ptr::null_mut(), 0, member, 1 as *mut c_void, 0, core::ptr::null_mut());
        hs[2] = CreateThread(core::ptr::null_mut(), 0, member, 2 as *mut c_void, 0, core::ptr::null_mut());
        while SH.ready.load(SeqCst) < 3 {
            Sleep(5);
        }
        SH.go.store(1, SeqCst);
        while SH.ready.load(SeqCst) < 33 {
            Sleep(5);
        }
        say!(t, "first STA thread: type {} (3 is the main STA), constant over 100 calls {}", SH.kinds[0].load(SeqCst) & 0xFF, SH.kinds[0].load(SeqCst) < 0x100);
        say!(t, "second STA thread while the first lives: type {} (0 is an STA), constant {}", SH.kinds[1].load(SeqCst) & 0xFF, SH.kinds[1].load(SeqCst) < 0x100);
        say!(t, "MTA thread: type {} (1 is the MTA), constant {}", SH.kinds[2].load(SeqCst) & 0xFF, SH.kinds[2].load(SeqCst) < 0x100);
        let (mut k, mut q) = (-9i32, -9i32);
        let r = CoGetApartmentType(&mut k, &mut q);
        say!(t, "this thread, which never initialized, while an MTA exists: returns {:#x}, type {k}", hr(r));
        SH.go.store(2, SeqCst);
        for h in hs {
            WaitForSingleObject(h, 10_000);
            CloseHandle(h);
        }
    }
}
