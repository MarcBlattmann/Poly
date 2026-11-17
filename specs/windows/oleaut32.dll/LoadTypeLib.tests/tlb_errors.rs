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
// LoadTypeLib (oleaut32 ordinal 161): the failures (missing file, no type library in the module, bad resource number, empty name).

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadTypeLib(file: *const u16, pp: *mut *mut c_void) -> i32;
    fn SysFreeString(b: *mut u16);
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *const c_void, flags: u32) -> i32;
}

poly_wtest::main!(test);

#[repr(C)]
struct Tlibattr {
    guid: [u8; 16],
    lcid: u32,
    syskind: u32,
    major: u16,
    minor: u16,
    flags: u16,
}

fn vt(p: *mut c_void, slot: usize) -> usize {
    // SAFETY: `p` is a COM interface pointer; slot < vtable length.
    unsafe { *(*(p as *const *const usize)).add(slot) }
}

fn release(p: *mut c_void) {
    // SAFETY: IUnknown::Release.
    unsafe {
        let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vt(p, 2));
        f(p);
    }
}

fn guid_text(g: &[u8; 16]) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    let _ = write!(b, "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}", g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]);
    b
}

fn load(t: &mut Test, label: &str, path: &str) -> *mut c_void {
    let w = wide(path);
    let mut p: *mut c_void = 0x1234usize as *mut c_void;
    // SAFETY: valid string and out pointer.
    let hr = unsafe { LoadTypeLib(w.as_ptr(), &mut p) };
    let state = if p as usize == 0x1234 { "untouched" } else if p.is_null() { "NULL" } else { "set" };
    say!(t, "{label}: hr={:#010x} out {state}", hr as u32);
    if hr >= 0 && !p.is_null() && p as usize != 0x1234 { p } else { core::ptr::null_mut() }
}

fn describe(t: &mut Test, label: &str, lib: *mut c_void) {
    // ITypeLib: 3 GetTypeInfoCount, 7 GetLibAttr, 9 GetDocumentation, 12 ReleaseTLibAttr
    // SAFETY: vtable slots of ITypeLib.
    unsafe {
        let count: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vt(lib, 3));
        say!(t, "{label}: type info count > 0: {}", count(lib) > 0);
        let getattr: extern "system" fn(*mut c_void, *mut *mut Tlibattr) -> i32 = core::mem::transmute(vt(lib, 7));
        let relattr: extern "system" fn(*mut c_void, *mut Tlibattr) = core::mem::transmute(vt(lib, 12));
        let mut a: *mut Tlibattr = core::ptr::null_mut();
        let hr = getattr(lib, &mut a);
        if hr >= 0 && !a.is_null() {
            let at = &*a;
            say!(t, "{label}: guid {} version {}.{} lcid {:#x} flags {:#x}", guid_text(&at.guid).as_str(), at.major, at.minor, at.lcid, at.flags);
            relattr(lib, a);
        } else {
            say!(t, "{label}: GetLibAttr hr={:#x}", hr as u32);
        }
        let doc: extern "system" fn(*mut c_void, i32, *mut *mut u16, *mut *mut u16, *mut u32, *mut *mut u16) -> i32 = core::mem::transmute(vt(lib, 9));
        let mut name: *mut u16 = core::ptr::null_mut();
        let hr = doc(lib, -1, &mut name, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        if hr >= 0 && !name.is_null() {
            say!(t, "{label}: library name {}", units_text(wstr_at(name, 64)).as_str());
            SysFreeString(name);
        } else {
            say!(t, "{label}: GetDocumentation hr={:#x}", hr as u32);
        }
    }
}

fn test(t: &mut Test) {
    // SAFETY: COM init for this thread.
    unsafe { CoInitializeEx(core::ptr::null(), 2) };
    for (label, path) in [
        ("scrrun.dll\\2 (no such resource)", "C:\\Windows\\System32\\scrrun.dll\\2"),
        ("scrrun.dll\\0", "C:\\Windows\\System32\\scrrun.dll\\0"),
        ("scrrun.dll\\x (not a number)", "C:\\Windows\\System32\\scrrun.dll\\x"),
        ("kernel32.dll (no type library)", "C:\\Windows\\System32\\kernel32.dll"),
        ("a name that does not exist", "C:\\Windows\\no-such-file.tlb"),
        ("a directory", "C:\\Windows"),
        ("empty string", ""),
        ("a drive that does not exist", "Z:\\no.tlb"),
    ] {
        let lib = load(t, label, path);
        if !lib.is_null() {
            release(lib);
        }
    }
}
