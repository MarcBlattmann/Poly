//! CoTaskMemFree across threads: blocks made on one thread and freed on
//! others (a producer fills a table, consumers free the slots), strings from
//! StringFromCLSID freed by another thread, a PROPVARIANT string cleared by
//! a thread other than the one that allocated it, and all of it without any
//! COM initialization on any thread and again with each thread in its own
//! apartment (the allocator does not depend on apartments).
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

/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const GUID_DEVINTERFACE_USB_DEVICE: GUID = GUID { d1: 0xA5DCBF10, d2: 0x6530, d3: 0x11D2, d4: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED] };
type HRESULT = i32;
const COINIT_MULTITHREADED: u32 = 0;
const VT_LPWSTR: u16 = 31;
/// propidl.h (PROPVARIANT on 64-bit Windows: 24 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
struct PROPVARIANT {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
    a: u64,
    b: u64,
}

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
    fn CoTaskMemAlloc(size: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
    fn StringFromCLSID(clsid: *const GUID, out: *mut *mut u16) -> HRESULT;
    fn PropVariantClear(pv: *mut PROPVARIANT) -> HRESULT;
}

poly_wtest::main!(test);

struct Table {
    slots: [usize; 128],
    com: u32,
}

unsafe extern "system" fn consumer(p: *mut c_void) -> u32 {
    // SAFETY: the table outlives the thread; each consumer frees a disjoint range.
    unsafe {
        let (tab, part) = *(p as *const (usize, usize));
        let tab = &mut *(tab as *mut Table);
        if tab.com != 0 {
            CoInitializeEx(core::ptr::null_mut(), COINIT_MULTITHREADED);
        }
        let mut ok = 1u32;
        for i in (part * 32)..(part * 32 + 32) {
            let b = tab.slots[i] as *mut u8;
            // the producer wrote its index into every block
            if b.is_null() || *b != i as u8 {
                ok = 0;
            }
            if i % 2 == 0 {
                CoTaskMemFree(b as *mut c_void);
            } else {
                // odd slots hold a string from StringFromCLSID (index in the first character)
                CoTaskMemFree(b as *mut c_void);
            }
            tab.slots[i] = 0;
        }
        if tab.com != 0 {
            CoUninitialize();
        }
        ok
    }
}

fn run(t: &mut Test, com: u32) {
    // SAFETY: blocks from the COM allocator, each freed once by one consumer.
    unsafe {
        let mut tab = Table { slots: [0; 128], com };
        for i in 0..128 {
            if i % 2 == 0 {
                let b = CoTaskMemAlloc(50 + i) as *mut u8;
                core::ptr::write_bytes(b, i as u8, 50 + i);
                tab.slots[i] = b as usize;
            } else {
                let mut s: *mut u16 = core::ptr::null_mut();
                StringFromCLSID(&GUID_DEVINTERFACE_USB_DEVICE, &mut s);
                // overwrite the first character with the index to check it later (a byte store)
                *(s as *mut u8) = i as u8;
                tab.slots[i] = s as usize;
            }
        }
        let args: [(usize, usize); 4] = [(&mut tab as *mut Table as usize, 0), (&mut tab as *mut Table as usize, 1), (&mut tab as *mut Table as usize, 2), (&mut tab as *mut Table as usize, 3)];
        let mut hs = [core::ptr::null_mut::<c_void>(); 4];
        for (i, h) in hs.iter_mut().enumerate() {
            *h = CreateThread(core::ptr::null_mut(), 0, consumer, &args[i] as *const (usize, usize) as *mut c_void, 0, core::ptr::null_mut());
        }
        let mut all = true;
        for h in hs {
            WaitForSingleObject(h, 30_000);
            let mut code = 0u32;
            GetExitCodeThread(h, &mut code);
            CloseHandle(h);
            all &= code == 1;
        }
        let empty = tab.slots.iter().all(|&s| s == 0);
        say!(t, "{} 128 blocks and strings freed by four consumer threads, each block seen intact: {all}, table emptied {empty}", if com != 0 { "with COM on every consumer:" } else { "without COM:" });
    }
}

fn test(t: &mut Test) {
    run(t, 0);
    run(t, 1);
    // a PROPVARIANT string allocated here and cleared by another thread
    unsafe extern "system" fn clear_there(p: *mut c_void) -> u32 {
        // SAFETY: the variant outlives the thread.
        unsafe {
            let v = &mut *(p as *mut PROPVARIANT);
            let r = PropVariantClear(v);
            (r == 0 && v.vt == 0) as u32
        }
    }
    // SAFETY: the string is from CoTaskMemAlloc, cleared once.
    unsafe {
        let s = CoTaskMemAlloc(20) as *mut u16;
        *s = 0x41;
        *s.add(1) = 0;
        let mut v = PROPVARIANT { vt: VT_LPWSTR, r1: 0, r2: 0, r3: 0, a: s as u64, b: 0 };
        let ok = on_thread(clear_there, &mut v as *mut PROPVARIANT as *mut c_void);
        say!(t, "a PROPVARIANT string cleared by another thread: {ok}");
    }
}
