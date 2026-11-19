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

// ---- COM helpers (declarations from Learn's prototypes and the public headers named per use) ----

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
    GUID { d1, d2, d3, d4 }
}

/// "{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}" text of a GUID.
fn guid_text(g: &GUID) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let _ = write!(
        b,
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.d1, g.d2, g.d3, g.d4[0], g.d4[1], g.d4[2], g.d4[3], g.d4[4], g.d4[5], g.d4[6], g.d4[7]
    );
    b
}

const IID_IUnknown: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;

const CLSCTX_INPROC_SERVER: u32 = 1;
const CLSCTX_ALL: u32 = 0x17;
const COINIT_APARTMENTTHREADED: u32 = 2;
const COINIT_MULTITHREADED: u32 = 0;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const GUID, outer: *mut c_void, ctx: u32, iid: *const GUID, out: *mut *mut c_void) -> i32;
    fn CoTaskMemAlloc(n: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
}

/// The function pointer in slot `idx` of the vtable of COM object `obj`.
unsafe fn slot(obj: *mut c_void, idx: usize) -> usize {
    // SAFETY (caller): `obj` is a live COM interface pointer with at least idx+1 slots.
    unsafe { *(*(obj as *mut *mut usize)).add(idx) }
}

/// Calls vtable slot `idx` of `obj`: `vc!(obj, idx, (T1, T2) -> R, a1, a2)` (the types are the method's after `this`).
macro_rules! vc {
    ($obj:expr, $idx:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o: *mut c_void = $obj;
        // SAFETY: `o` is a live interface pointer; the slot has the signature given by the types.
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(slot(o, $idx));
            f(o $(, $a)*)
        }
    }};
}

fn qi(obj: *mut c_void, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: IUnknown::QueryInterface is slot 0.
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32 = core::mem::transmute(slot(obj, 0));
        f(obj, iid, &mut out)
    };
    (hr, out)
}
fn add_ref(obj: *mut c_void) -> u32 {
    // SAFETY: slot 1 is AddRef.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(obj, 1));
        f(obj)
    }
}
fn release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: slot 2 is Release.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(obj, 2));
        f(obj)
    }
}
fn create(clsid: &GUID, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { CoCreateInstance(clsid, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, iid, &mut out) };
    (hr, out)
}

// ---- Windows Firewall COM helpers (INetFwPolicy2, INetFwRules, INetFwRule: netfw.h; vtable slots counted after IDispatch's 7) ----
#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SysAllocString(s: *const u16) -> *mut u16;
    fn SysFreeString(s: *mut u16);
    fn SysStringLen(s: *const u16) -> u32;
}

const CLSID_NetFwPolicy2: GUID = guid(0xE2B3C97F, 0x6AE1, 0x41AC, [0x81, 0x7A, 0xF6, 0xF9, 0x21, 0x66, 0xD7, 0xDD]);
const IID_INetFwPolicy2: GUID = guid(0x98325047, 0xC671, 0x4174, [0x8D, 0x81, 0xDE, 0xFC, 0xD3, 0xF0, 0x31, 0x86]);
const CLSID_NetFwRule: GUID = guid(0x2C5BC43E, 0x3369, 0x4C33, [0xAB, 0x0C, 0xBE, 0x94, 0x69, 0x67, 0x7A, 0xF4]);
const IID_INetFwRule: GUID = guid(0xAF230D27, 0xBABA, 0x4E42, [0xAC, 0xED, 0xF5, 0x24, 0xF2, 0x2C, 0xFC, 0xE2]);

// INetFwRule slots
const R_NAME: usize = 7;
const R_DESCRIPTION: usize = 9;
const R_APPLICATION: usize = 11;
const R_PROTOCOL: usize = 15;
const R_LOCALPORTS: usize = 17;
const R_DIRECTION: usize = 27;
const R_ENABLED: usize = 33;
const R_GROUPING: usize = 35;
const R_PROFILES: usize = 37;
const R_EDGE: usize = 39;
const R_ACTION: usize = 41;

fn bstr(s: &str) -> *mut u16 {
    let w = wide(s);
    // SAFETY: valid NUL-terminated buffer.
    unsafe { SysAllocString(w.as_ptr()) }
}

/// Reads a BSTR out parameter into printable text and frees it.
fn bstr_text(b: *mut u16) -> Buf<200> {
    let mut o = Buf::<200>::new();
    if b.is_null() {
        let _ = write!(o, "(NULL)");
        return o;
    }
    // SAFETY: a BSTR the system returned.
    unsafe {
        let n = SysStringLen(b) as usize;
        let s = core::slice::from_raw_parts(b, n);
        let t = units_text(s);
        let _ = write!(o, "{}", t.as_str());
        SysFreeString(b);
    }
    o
}

fn get_bstr(rule: *mut c_void, slot: usize) -> (i32, Buf<200>) {
    let mut b: *mut u16 = core::ptr::null_mut();
    let hr = vc!(rule, slot, (*mut *mut u16) -> i32, &mut b);
    (hr, bstr_text(b))
}
fn put_bstr(rule: *mut c_void, slot: usize, s: &str) -> i32 {
    let b = bstr(s);
    let hr = vc!(rule, slot + 1, (*mut u16) -> i32, b);
    // SAFETY: freeing our own BSTR (the callee copies).
    unsafe { SysFreeString(b) };
    hr
}
fn get_long(rule: *mut c_void, slot: usize) -> (i32, i32) {
    let mut v = 0i32;
    let hr = vc!(rule, slot, (*mut i32) -> i32, &mut v);
    (hr, v)
}
fn put_long(rule: *mut c_void, slot: usize, v: i32) -> i32 {
    vc!(rule, slot + 1, (i32) -> i32, v)
}
fn get_bool(rule: *mut c_void, slot: usize) -> (i32, i16) {
    let mut v = 0x7777i16;
    let hr = vc!(rule, slot, (*mut i16) -> i32, &mut v);
    (hr, v)
}
fn put_bool(rule: *mut c_void, slot: usize, v: bool) -> i32 {
    vc!(rule, slot + 1, (i16) -> i32, if v { -1 } else { 0 })
}

/// A new INetFwRule object.
fn new_rule() -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { CoCreateInstance(&CLSID_NetFwRule, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_INetFwRule, &mut out) };
    (hr, out)
}

fn hx(hr: i32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    let _ = write!(b, "{:#010x}", hr as u32);
    b
}

poly_wtest::main!(test);

const IID_IDispatch: GUID = guid(0x00020400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_INetFwPolicy: GUID = guid(0xD46D2478, 0x9AC9, 0x4008, [0x9D, 0xC7, 0x55, 0x63, 0xCE, 0x55, 0x36, 0xCC]);

fn test(t: &mut Test) {
    // SAFETY: plain call.
    let hr = unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_MULTITHREADED) };
    say!(t, "CoInitializeEx (MTA): {}", hx(hr).as_str());
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers; CLSCTX_ALL.
    let hr = unsafe { CoCreateInstance(&CLSID_NetFwPolicy2, core::ptr::null_mut(), CLSCTX_ALL, &IID_INetFwPolicy2, &mut out) };
    say!(t, "CoCreateInstance(NetFwPolicy2, CLSCTX_ALL, INetFwPolicy2): {}", hx(hr).as_str());
    let pol = out;
    if pol.is_null() {
        return;
    }
    let (hr, d) = qi(pol, &IID_IDispatch);
    say!(t, "QueryInterface(IDispatch): {}", hx(hr).as_str());
    release(d);
    let (hr, u) = qi(pol, &IID_IUnknown);
    say!(t, "QueryInterface(IUnknown): {}", hx(hr).as_str());
    release(u);
    let (hr, o) = qi(pol, &IID_INetFwPolicy);
    say!(t, "QueryInterface(INetFwPolicy, the older interface): {}", hx(hr).as_str());
    release(o);
    let (hr, o) = qi(pol, &guid(0x12345678, 0, 0, [0, 0, 0, 0, 0, 0, 0, 1]));
    say!(t, "QueryInterface(an unknown IID): {} object {}", hx(hr).as_str(), if o.is_null() { "NULL" } else { "set" });
    // profile types and the enabled state per profile (values differ between machines: shapes only)
    let (hr, prof) = get_long(pol, 7);
    say!(t, "get_CurrentProfileTypes: {} exactly one of domain 1, private 2, public 4 or a combination: {}", hx(hr).as_str(), prof > 0 && prof & !7 == 0);
    for p in [1i32, 2, 4] {
        let mut v = 0x7777i16;
        let hr = vc!(pol, 8, (i32, *mut i16) -> i32, p, &mut v);
        say!(t, "get_FirewallEnabled(profile {p}): {} is a VARIANT_BOOL (0 or -1): {}", hx(hr).as_str(), v == 0 || v == -1);
    }
    for p in [0i32, 3, 8, 0x7FFF_FFFF, -1] {
        let mut v = 0x7777i16;
        let hr = vc!(pol, 8, (i32, *mut i16) -> i32, p, &mut v);
        say!(t, "get_FirewallEnabled(profile {p}): {} value written {}", hx(hr).as_str(), v != 0x7777);
    }
    let hr = vc!(pol, 7, (*mut i32) -> i32, core::ptr::null_mut());
    say!(t, "get_CurrentProfileTypes(NULL): {}", hx(hr).as_str());
    let hr = vc!(pol, 18, (*mut *mut c_void) -> i32, core::ptr::null_mut());
    say!(t, "get_Rules(NULL): {}", hx(hr).as_str());
    // the rules object: shape
    let mut rules: *mut c_void = core::ptr::null_mut();
    let _ = vc!(pol, 18, (*mut *mut c_void) -> i32, &mut rules);
    let (hr, d) = qi(rules, &IID_IDispatch);
    say!(t, "Rules: QueryInterface(IDispatch): {}", hx(hr).as_str());
    release(d);
    let mut n = -5i32;
    let _ = vc!(rules, 7, (*mut i32) -> i32, &mut n);
    say!(t, "Rules.Count is positive: {}", n > 0);
    let hr = vc!(rules, 7, (*mut i32) -> i32, core::ptr::null_mut());
    say!(t, "Rules.Count(NULL): {}", hx(hr).as_str());
    let b = bstr("PolySpecB4 no such rule");
    let mut item: *mut c_void = 1 as *mut c_void;
    let hr = vc!(rules, 10, (*mut u16, *mut *mut c_void) -> i32, b, &mut item);
    // SAFETY: our BSTR.
    unsafe { SysFreeString(b) };
    say!(t, "Rules.Item(missing): {} out pointer set to NULL: {}", hx(hr).as_str(), item.is_null());
    let hr = vc!(rules, 10, (*mut u16, *mut *mut c_void) -> i32, core::ptr::null_mut(), &mut item);
    say!(t, "Rules.Item(NULL name): {}", hx(hr).as_str());
    // enumerator
    let mut en: *mut c_void = core::ptr::null_mut();
    let hr = vc!(rules, 11, (*mut *mut c_void) -> i32, &mut en);
    say!(t, "Rules._NewEnum: {} object {}", hx(hr).as_str(), if en.is_null() { "NULL" } else { "set" });
    release(en);
    release(rules);
    // a second policy object sees the same rules
    let (hr, p2) = create(&CLSID_NetFwPolicy2, &IID_INetFwPolicy2);
    say!(t, "a second NetFwPolicy2: {} a different object: {}", hx(hr).as_str(), p2 != pol);
    release(p2);
    // aggregation is not supported
    let mut agg: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers; a bogus outer pointer.
    let hr = unsafe { CoCreateInstance(&CLSID_NetFwPolicy2, pol, CLSCTX_INPROC_SERVER, &IID_INetFwPolicy2, &mut agg) };
    say!(t, "CoCreateInstance with an outer unknown: {}", hx(hr).as_str());
    release(agg);
    release(pol);
}
