#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
type DWORD = u32;

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

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; no characters outside the BMP).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| {
            let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
            l(x) == l(y)
        })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn clear_error() {
    // SAFETY: plain call.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a failed call, else -2 (success leaves it unspecified).
fn gle_if(ok: BOOL) -> i64 {
    if ok == 0 { gle() } else { -2 }
}
// CoGetCallContext outside any call, on a thread that never initialised COM: every IID answers RPC_E_CALL_COMPLETE and leaves the output alone.

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoGetCallContext(riid: *const [u8; 16], pp: *mut *mut c_void) -> i32;
    fn CoInitializeEx(reserved: *const c_void, flags: u32) -> i32;
    fn CoUninitialize();
}

poly_wtest::main!(test);

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

const CLS: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IUnknown: [u8; 16] = guid(0, 0, 0, CLS);
const IID_IServerSecurity: [u8; 16] = guid(0x13E, 0, 0, CLS);
const IID_IContextCallback: [u8; 16] = guid(0x1da, 0, 0, CLS);
const IID_IObjContext: [u8; 16] = guid(0x1c6, 0, 0, CLS);
const IID_ICallFactory: [u8; 16] = guid(0x1c9, 0, 0, CLS);
const IID_IMarshal: [u8; 16] = guid(3, 0, 0, CLS);
const IID_ISecurityCallContext: [u8; 16] = guid(0xCAFC823E, 0xB441, 0x11D1, [0xB8, 0x2B, 0, 0, 0xF8, 0x75, 0x7E, 0x2A]);
const IID_IDispatch: [u8; 16] = guid(0x20400, 0, 0, CLS);
const GUID_NULL: [u8; 16] = [0; 16];

fn ask(t: &mut Test, label: &str, iid: &[u8; 16]) {
    let mut p: *mut c_void = 0x1234usize as *mut c_void;
    // SAFETY: valid pointers.
    let hr = unsafe { CoGetCallContext(iid, &mut p) };
    let state = if p as usize == 0x1234 { "untouched" } else if p.is_null() { "NULL" } else { "set" };
    say!(t, "{label}: hr={:#010x} out {state}", hr as u32);
    if p as usize != 0x1234 && !p.is_null() {
        // SAFETY: an interface pointer: release through its vtable (slot 2).
        unsafe {
            let vt = *(p as *const *const usize);
            let rel: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
            rel(p);
        }
    }
}

fn all(t: &mut Test, tag: &str) {
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} IServerSecurity");
    ask(t, b.as_str(), &IID_IServerSecurity);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} IUnknown");
    ask(t, b.as_str(), &IID_IUnknown);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} IContextCallback");
    ask(t, b.as_str(), &IID_IContextCallback);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} IObjContext");
    ask(t, b.as_str(), &IID_IObjContext);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} ICallFactory");
    ask(t, b.as_str(), &IID_ICallFactory);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} ISecurityCallContext");
    ask(t, b.as_str(), &IID_ISecurityCallContext);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} IMarshal");
    ask(t, b.as_str(), &IID_IMarshal);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} IDispatch");
    ask(t, b.as_str(), &IID_IDispatch);
    let mut b = Buf::<80>::new();
    let _ = write!(b, "{tag} GUID_NULL");
    ask(t, b.as_str(), &GUID_NULL);
}

fn test(t: &mut Test) {
    all(t, "no COM:");
}
