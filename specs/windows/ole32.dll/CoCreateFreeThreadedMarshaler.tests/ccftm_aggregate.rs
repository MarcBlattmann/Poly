//! CoCreateFreeThreadedMarshaler with an outer unknown (aggregation, as an
//! object that is free-threaded uses it): the object returned is the inner
//! unknown whose QueryInterface(IMarshal) answers; QueryInterface of other
//! interfaces on the inner unknown fail with E_NOINTERFACE 0x80004002; the
//! inner object's AddRef and Release are separate from the outer's; the
//! outer object is not called by the creation (no QueryInterface/AddRef/
//! Release on it during the call). Chrome's aggregates are made as the
//! last step of a COM object's construction (S3).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const IID_IUNKNOWN: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IDROPTARGET: GUID = GUID { d1: 0x0000_0122, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
fn hr(h: i32) -> u32 {
    h as u32
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoCreateFreeThreadedMarshaler(outer: *mut c_void, out: *mut *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

const IID_IMARSHAL2: GUID = GUID { d1: 3, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

#[repr(C)]
struct Vt {
    qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add: unsafe extern "system" fn(*mut c_void) -> u32,
    rel: unsafe extern "system" fn(*mut c_void) -> u32,
}
#[repr(C)]
struct Outer {
    vtbl: *const Vt,
    calls: core::cell::Cell<i32>,
}
unsafe extern "system" fn o_qi(this: *mut c_void, _: *const GUID, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: `this` is an Outer.
    unsafe {
        let o = &*(this as *const Outer);
        o.calls.set(o.calls.get() + 1);
        *out = core::ptr::null_mut();
    }
    E_NOINTERFACE
}
unsafe extern "system" fn o_add(this: *mut c_void) -> u32 {
    // SAFETY: `this` is an Outer.
    unsafe {
        let o = &*(this as *const Outer);
        o.calls.set(o.calls.get() + 1);
    }
    2
}
unsafe extern "system" fn o_rel(this: *mut c_void) -> u32 {
    // SAFETY: `this` is an Outer.
    unsafe {
        let o = &*(this as *const Outer);
        o.calls.set(o.calls.get() + 1);
    }
    1
}
static OVT: Vt = Vt { qi: o_qi, add: o_add, rel: o_rel };

unsafe fn vt(p: *mut c_void) -> &'static Vt {
    // SAFETY: a COM object.
    unsafe { &**(p as *const *const Vt) }
}

fn test(t: &mut Test) {
    // SAFETY: COM objects used within their lifetime.
    unsafe {
        let outer = Outer { vtbl: &OVT, calls: core::cell::Cell::new(0) };
        let po = &outer as *const Outer as *mut c_void;
        let mut inner: *mut c_void = 5usize as *mut c_void;
        let r = CoCreateFreeThreadedMarshaler(po, &mut inner);
        say!(t, "with an outer unknown: {:#x}, inner object {}, the outer was called {} time(s)", hr(r), !inner.is_null(), outer.calls.get());
        let mut m: *mut c_void = core::ptr::null_mut();
        let r = (vt(inner).qi)(inner, &IID_IMARSHAL2, &mut m);
        say!(t, "QueryInterface(IMarshal) on the inner unknown: {:#x}, an object {}, the outer was called {} time(s)", hr(r), !m.is_null(), outer.calls.get());
        let mut u: *mut c_void = core::ptr::null_mut();
        let r = (vt(inner).qi)(inner, &IID_IUNKNOWN, &mut u);
        say!(t, "QueryInterface(IUnknown) on the inner unknown: {:#x}, the same pointer as the inner {}", hr(r), u == inner);
        let mut d: *mut c_void = 5usize as *mut c_void;
        let r = (vt(inner).qi)(inner, &IID_IDROPTARGET, &mut d);
        say!(t, "QueryInterface(IDropTarget): {:#x}, result NULL {}", hr(r), d.is_null());
        // the inner counts are its own: three references held (inner, m, u), release them
        let r1 = (vt(m).rel)(m);
        let r2 = (vt(u).rel)(u);
        let r3 = (vt(inner).rel)(inner);
        say!(t, "references left after releasing m, u and the inner: {r1} {r2} {r3}, the outer was called {} time(s) in all", outer.calls.get());
        // a non-IUnknown request on aggregation: the inner must be IUnknown; call twice for two objects
        let mut a: *mut c_void = core::ptr::null_mut();
        let mut b: *mut c_void = core::ptr::null_mut();
        CoCreateFreeThreadedMarshaler(po, &mut a);
        CoCreateFreeThreadedMarshaler(po, &mut b);
        say!(t, "two aggregations make different inner objects: {}", a != b);
        (vt(a).rel)(a);
        (vt(b).rel)(b);
    }
}
