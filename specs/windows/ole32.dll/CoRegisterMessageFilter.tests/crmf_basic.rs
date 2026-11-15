//! CoRegisterMessageFilter(filter, &old): per apartment-threaded thread. The
//! first registration returns S_OK and no old filter; the old filter comes
//! back (AddRef'ed by the call: the caller releases it) when another one is
//! registered; NULL removes the filter; the filter is AddRef'ed by the
//! registration and released at removal; the old pointer may be NULL; a
//! thread without COM gets CO_E_NOT_SUPPORTED-class errors (here the HRESULT
//! is printed); an MTA thread gets CO_E_NOT_SUPPORTED 0x80004021; the same
//! filter twice. Chrome's COM code calls it once with NULL (S3).
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

type HRESULT = i32;
const S_OK: i32 = 0;
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
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoRegisterMessageFilter(filter: *mut c_void, old: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

#[repr(C)]
struct IMessageFilterVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    handle_incoming_call: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut c_void) -> u32,
    retry_rejected_call: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) -> u32,
    message_pending: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) -> u32,
}
#[repr(C)]
struct Filter {
    vtbl: *const IMessageFilterVtbl,
    refs: core::cell::Cell<i32>,
}
unsafe extern "system" fn f_qi(this: *mut c_void, _iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: COM passes a writable out pointer.
    unsafe {
        *out = this;
        (*(this as *const Filter)).refs.set((*(this as *const Filter)).refs.get() + 1);
    }
    S_OK
}
unsafe extern "system" fn f_add(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a Filter.
    unsafe {
        let f = &*(this as *const Filter);
        f.refs.set(f.refs.get() + 1);
        f.refs.get() as u32
    }
}
unsafe extern "system" fn f_rel(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a Filter.
    unsafe {
        let f = &*(this as *const Filter);
        f.refs.set(f.refs.get() - 1);
        f.refs.get() as u32
    }
}
unsafe extern "system" fn f_in(_: *mut c_void, _: u32, _: *mut c_void, _: u32, _: *mut c_void) -> u32 {
    0
}
unsafe extern "system" fn f_retry(_: *mut c_void, _: *mut c_void, _: u32, _: u32) -> u32 {
    0xFFFF_FFFF
}
unsafe extern "system" fn f_pending(_: *mut c_void, _: *mut c_void, _: u32, _: u32) -> u32 {
    1
}
static FVTBL: IMessageFilterVtbl = IMessageFilterVtbl { query_interface: f_qi, add_ref: f_add, release: f_rel, handle_incoming_call: f_in, retry_rejected_call: f_retry, message_pending: f_pending };

struct Out {
    v: [u32; 24],
    n: usize,
}
impl Out {
    fn put(&mut self, v: u32) {
        if self.n < 24 {
            self.v[self.n] = v;
            self.n += 1;
        }
    }
}

unsafe extern "system" fn sta(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread; the filters live on this stack.
    unsafe {
        let o = &mut *(p as *mut Out);
        CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        let a = Filter { vtbl: &FVTBL, refs: core::cell::Cell::new(1) };
        let b = Filter { vtbl: &FVTBL, refs: core::cell::Cell::new(1) };
        let pa = &a as *const Filter as *mut c_void;
        let pb = &b as *const Filter as *mut c_void;
        let mut old: *mut c_void = 5usize as *mut c_void;
        o.put(hr(CoRegisterMessageFilter(pa, &mut old))); // 0
        o.put(old.is_null() as u32); // 1: no old filter
        o.put(a.refs.get() as u32); // 2: AddRef'ed
        o.put(hr(CoRegisterMessageFilter(pb, &mut old))); // 3
        o.put((old == pa) as u32); // 4: the old filter comes back
        o.put(a.refs.get() as u32); // 5: reference to the old one handed to the caller
        o.put(b.refs.get() as u32); // 6
        o.put(hr(CoRegisterMessageFilter(pb, core::ptr::null_mut()))); // 7: same filter, no old pointer wanted
        o.put(b.refs.get() as u32); // 8
        let mut old2: *mut c_void = 5usize as *mut c_void;
        o.put(hr(CoRegisterMessageFilter(core::ptr::null_mut(), &mut old2))); // 9: remove
        o.put((old2 == pb) as u32); // 10
        o.put(b.refs.get() as u32); // 11
        let mut old3: *mut c_void = 5usize as *mut c_void;
        o.put(hr(CoRegisterMessageFilter(core::ptr::null_mut(), &mut old3))); // 12: remove again
        o.put(old3.is_null() as u32); // 13
        CoUninitialize();
        0
    }
}

unsafe extern "system" fn mta(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread.
    unsafe {
        let o = &mut *(p as *mut Out);
        CoInitializeEx(core::ptr::null_mut(), COINIT_MULTITHREADED);
        let mut old: *mut c_void = 5usize as *mut c_void;
        o.put(hr(CoRegisterMessageFilter(core::ptr::null_mut(), &mut old)));
        let a = Filter { vtbl: &FVTBL, refs: core::cell::Cell::new(1) };
        o.put(hr(CoRegisterMessageFilter(&a as *const Filter as *mut c_void, core::ptr::null_mut())));
        CoUninitialize();
        0
    }
}

fn test(t: &mut Test) {
    let mut o = Out { v: [0; 24], n: 0 };
    on_thread(sta, &mut o as *mut Out as *mut c_void);
    say!(t, "first filter: {:#x}, no old filter {}, references held {}", o.v[0], o.v[1], o.v[2]);
    say!(t, "second filter: {:#x}, the old one comes back {}, its references {}, the new one's {}", o.v[3], o.v[4], o.v[5], o.v[6]);
    say!(t, "the same filter again without an old pointer: {:#x}, its references {}", o.v[7], o.v[8]);
    say!(t, "NULL removes it: {:#x}, the old filter comes back {}, its references {}", o.v[9], o.v[10], o.v[11]);
    say!(t, "NULL again: {:#x}, no old filter {}", o.v[12], o.v[13]);
    let mut o = Out { v: [0; 24], n: 0 };
    on_thread(mta, &mut o as *mut Out as *mut c_void);
    say!(t, "an MTA thread: removing {:#x}, registering {:#x}", o.v[0], o.v[1]);
}
