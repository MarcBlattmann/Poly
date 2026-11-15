//! PropVariantClear's checks: a NULL pointer (E_INVALIDARG 0x80070057), a vt
//! that is not defined (E_INVALIDARG; the value is left as it was), reserved
//! flag bits (VT_RESERVED 0x8000 and others), VT_BYREF values (not freed),
//! VT_ARRAY values with no array (a NULL SAFEARRAY pointer), VT_VECTOR with a
//! zero count, VT_VARIANT vectors of nested values, and the vts that are
//! valid in a PROPVARIANT but hold no pointer: VT_I1 .. VT_R8, VT_BOOL,
//! VT_FILETIME. The HRESULT of each is printed and the vt afterwards.
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

type HRESULT = i32;
const VT_EMPTY: u16 = 0;
const VT_I4: u16 = 3;
const VT_UI4: u16 = 19;
const VT_BSTR: u16 = 8;
const VT_LPWSTR: u16 = 31;
const VT_VECTOR: u16 = 0x1000;
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

fn hr(h: i32) -> u32 {
    h as u32
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemAlloc(size: usize) -> *mut c_void;
    fn PropVariantClear(pv: *mut PROPVARIANT) -> HRESULT;
}

poly_wtest::main!(test);

fn pv(vt: u16) -> PROPVARIANT {
    PROPVARIANT { vt, r1: 0, r2: 0, r3: 0, a: 0x0123_4567_89AB_CDEF, b: 0xFEDC_BA98_7654_3210 }
}

fn test(t: &mut Test) {
    // SAFETY: values without owned memory, or NULL pointers.
    unsafe {
        let r = PropVariantClear(core::ptr::null_mut());
        say!(t, "a NULL pointer: {:#x}", hr(r));
        for (name, vt) in [("VT_I2", 2u16), ("VT_R4", 4), ("VT_R8", 5), ("VT_CY", 6), ("VT_DATE", 7), ("VT_ERROR", 10), ("VT_BOOL", 11), ("VT_I1", 16), ("VT_UI1", 17), ("VT_UI2", 18), ("VT_I8", 20), ("VT_UI8", 21), ("VT_INT", 22), ("VT_UINT", 23), ("VT_FILETIME", 64)] {
            let mut v = pv(vt);
            let r = PropVariantClear(&mut v);
            say!(t, "{name}: returns {:#x}, vt now {}, value words {:#x} {:#x}", hr(r), v.vt, v.a, v.b);
        }
        // not defined
        for (name, vt) in [("vt 0x7F (not defined)", 0x7Fu16), ("vt 0x200 (not defined)", 0x200), ("VT_RESERVED 0x8000", 0x8000), ("vt 0xFFFF", 0xFFFF)] {
            let mut v = pv(vt);
            let r = PropVariantClear(&mut v);
            say!(t, "{name}: returns {:#x}, vt afterwards {:#x}, value words kept {}", hr(r), v.vt, v.a == 0x0123_4567_89AB_CDEF);
        }
        // by reference: nothing is freed, the pointer word may be anything
        let mut v = pv(VT_BSTR | 0x4000);
        v.a = 0;
        let r = PropVariantClear(&mut v);
        say!(t, "VT_BYREF | VT_BSTR with a NULL pointer: returns {:#x}, vt now {}", hr(r), v.vt);
        // vector, zero elements and a NULL array
        let mut v = pv(VT_VECTOR | VT_I4);
        v.a = 0;
        v.b = 0;
        let r = PropVariantClear(&mut v);
        say!(t, "VT_VECTOR | VT_I4 with no elements: returns {:#x}, vt now {}", hr(r), v.vt);
        let mut v = pv(VT_VECTOR | VT_LPWSTR);
        v.a = 0;
        v.b = 0;
        let r = PropVariantClear(&mut v);
        say!(t, "VT_VECTOR | VT_LPWSTR with no elements: returns {:#x}, vt now {}", hr(r), v.vt);
        // a vector of two strings owned by the value
        let arr = CoTaskMemAlloc(16) as *mut *mut u16;
        for i in 0..2 {
            let s = CoTaskMemAlloc(8) as *mut u16;
            *s = 0x61 + i as u16;
            *s.add(1) = 0;
            *arr.add(i) = s;
        }
        let mut v = pv(VT_VECTOR | VT_LPWSTR);
        v.a = 2;
        v.b = arr as u64;
        let r = PropVariantClear(&mut v);
        say!(t, "VT_VECTOR | VT_LPWSTR with two owned strings: returns {:#x}, vt now {}, words {:#x} {:#x}", hr(r), v.vt, v.a, v.b);
        // an array with no SAFEARRAY
        let mut v = pv(0x2000 | VT_I4);
        v.a = 0;
        let r = PropVariantClear(&mut v);
        say!(t, "VT_ARRAY | VT_I4 with a NULL SAFEARRAY: returns {:#x}, vt now {}", hr(r), v.vt);
        // consecutive clears of an array of values
        let mut vs = [pv(VT_I4), pv(VT_EMPTY), pv(VT_UI4)];
        let mut all = true;
        for v in vs.iter_mut() {
            all &= PropVariantClear(v) == 0 && v.vt == 0;
        }
        say!(t, "three plain values in an array: {all}");
    }
}
