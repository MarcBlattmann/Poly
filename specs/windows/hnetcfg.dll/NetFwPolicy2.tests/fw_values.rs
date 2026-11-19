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

fn try_long(t: &mut Test, rule: *mut c_void, what: &str, slot: usize, vals: &[i32]) {
    for &v in vals {
        let hr = put_long(rule, slot, v);
        let (_, back) = get_long(rule, slot);
        say!(t, "{what} = {v}: put {} then get {back}", hx(hr).as_str());
    }
}

fn try_str(t: &mut Test, rule: *mut c_void, what: &str, slot: usize, vals: &[&str]) {
    for v in vals {
        let hr = put_bstr(rule, slot, v);
        let (_, back) = get_bstr(rule, slot);
        say!(t, "{what} = '{v}': put {} then get '{}'", hx(hr).as_str(), back.as_str());
    }
}

fn test(t: &mut Test) {
    // SAFETY: plain call.
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    let (_, rule) = new_rule();
    try_long(t, rule, "Protocol", R_PROTOCOL, &[6, 17, 1, 58, 256, 300, -1, 0, 255]);
    try_long(t, rule, "Direction", R_DIRECTION, &[1, 2, 0, 3, -1]);
    try_long(t, rule, "Action", R_ACTION, &[0, 1, 2, 3]);
    try_long(t, rule, "Profiles", R_PROFILES, &[1, 2, 4, 7, 0x7FFF_FFFF, 8, 0, -1]);
    try_long(t, rule, "EdgeTraversal(slot)", R_EDGE, &[0, 1]);
    // ports depend on the protocol
    put_long(rule, R_PROTOCOL, 256);
    try_str(t, rule, "LocalPorts with protocol ANY", R_LOCALPORTS, &["5353"]);
    put_long(rule, R_PROTOCOL, 17);
    try_str(t, rule, "LocalPorts with protocol UDP", R_LOCALPORTS, &["5353", "80,443", "5000-5100", "*", "RPC", "abc", "70000", "0", "", "5353,", "1-"]);
    put_long(rule, R_PROTOCOL, 1);
    try_str(t, rule, "LocalPorts with protocol ICMP", R_LOCALPORTS, &["5353"]);
    put_long(rule, R_PROTOCOL, 17);
    try_str(t, rule, "Name", R_NAME, &["PolySpecB4 name", ""]);
    try_str(t, rule, "Grouping", R_GROUPING, &["PolySpec B4", "@missing.dll,-1"]);
    try_str(t, rule, "ApplicationName", R_APPLICATION, &["C:\\PolySpecB4\\a.exe", "%ProgramFiles%\\PolySpecB4\\a.exe", "relative.exe"]);
    release(rule);

    // Add: what is required
    let (hr, pol) = create(&CLSID_NetFwPolicy2, &IID_INetFwPolicy2);
    if hr < 0 || pol.is_null() {
        say!(t, "no policy object: {}", hx(hr).as_str());
        return;
    }
    let mut rules: *mut c_void = core::ptr::null_mut();
    let _ = vc!(pol, 18, (*mut *mut c_void) -> i32, &mut rules);
    for nm in ["PolySpecB4 minimal", "PolySpecB4 bad"] {
        for _ in 0..3 {
            let b = bstr(nm);
            let _ = vc!(rules, 9, (*mut u16) -> i32, b);
            // SAFETY: our BSTR.
            unsafe { SysFreeString(b) };
        }
    }
    let mut n0 = 0i32;
    let _ = vc!(rules, 7, (*mut i32) -> i32, &mut n0);
    // no name
    let (_, r) = new_rule();
    put_long(r, R_PROTOCOL, 17);
    put_bool(r, R_ENABLED, true);
    let hr = vc!(rules, 8, (*mut c_void) -> i32, r);
    say!(t, "Add of a rule without a name: {}", hx(hr).as_str());
    release(r);
    // NULL rule
    let hr = vc!(rules, 8, (*mut c_void) -> i32, core::ptr::null_mut());
    say!(t, "Add(NULL): {}", hx(hr).as_str());
    // a minimal named rule
    let (_, r) = new_rule();
    put_bstr(r, R_NAME, "PolySpecB4 minimal");
    let hr = vc!(rules, 8, (*mut c_void) -> i32, r);
    say!(t, "Add of a rule with only a name: {}", hx(hr).as_str());
    let mut n1 = 0i32;
    let _ = vc!(rules, 7, (*mut i32) -> i32, &mut n1);
    say!(t, "the count grew by: {}", n1 - n0);
    // a change to the rule object after Add
    put_bstr(r, R_DESCRIPTION, "changed after add");
    let (hr, found) = {
        let b = bstr("PolySpecB4 minimal");
        let mut out: *mut c_void = core::ptr::null_mut();
        let hr = vc!(rules, 10, (*mut u16, *mut *mut c_void) -> i32, b, &mut out);
        // SAFETY: our BSTR.
        unsafe { SysFreeString(b) };
        (hr, out)
    };
    say!(t, "Item of the minimal rule: {}", hx(hr).as_str());
    if !found.is_null() {
        say!(t, "  Description after a put on the added object: '{}'", get_bstr(found, R_DESCRIPTION).1.as_str());
        say!(t, "  Protocol {:?} Direction {:?} Action {:?} Enabled {:?} Profiles {:?}", get_long(found, R_PROTOCOL), get_long(found, R_DIRECTION), get_long(found, R_ACTION), get_bool(found, R_ENABLED), get_long(found, R_PROFILES));
        say!(t, "  the same object as the one added: {}", found == r);
        release(found);
    }
    // an invalid combination: ports for ICMP
    let (_, r2) = new_rule();
    put_bstr(r2, R_NAME, "PolySpecB4 bad");
    put_long(r2, R_PROTOCOL, 1);
    put_bstr(r2, R_LOCALPORTS, "80");
    let hr = vc!(rules, 8, (*mut c_void) -> i32, r2);
    say!(t, "Add of an ICMP rule with ports: {}", hx(hr).as_str());
    release(r2);
    let b = bstr("PolySpecB4 minimal");
    let hr = vc!(rules, 9, (*mut u16) -> i32, b);
    // SAFETY: our BSTR.
    unsafe { SysFreeString(b) };
    say!(t, "Remove: {}", hx(hr).as_str());
    let b = bstr("PolySpecB4 bad");
    let hr = vc!(rules, 9, (*mut u16) -> i32, b);
    // SAFETY: our BSTR.
    unsafe { SysFreeString(b) };
    say!(t, "Remove of the bad one (if it was added): {}", hx(hr).as_str());
    let hr = vc!(rules, 9, (*mut u16) -> i32, core::ptr::null_mut());
    say!(t, "Remove(NULL): {}", hx(hr).as_str());
    let mut n2 = 0i32;
    let _ = vc!(rules, 7, (*mut i32) -> i32, &mut n2);
    say!(t, "the count is back to the start: {}", n2 == n0);
    release(r);
    release(rules);
    release(pol);
}
