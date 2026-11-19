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

// ---- native registry helpers (prototypes from winternl.h / Learn "Zw*" pages; constants from wdm.h, ntstatus.h) ----
#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtCreateKey(h: *mut HANDLE, access: u32, oa: *const ObjectAttributes, title: u32, class: *const UnicodeString, opts: u32, disp: *mut u32) -> i32;
    fn NtOpenKeyEx(h: *mut HANDLE, access: u32, oa: *const ObjectAttributes, opts: u32) -> i32;
    fn NtOpenKey(h: *mut HANDLE, access: u32, oa: *const ObjectAttributes) -> i32;
    fn NtQueryValueKey(h: HANDLE, name: *const UnicodeString, class: u32, info: *mut u8, len: u32, res: *mut u32) -> i32;
    fn NtSetValueKey(h: HANDLE, name: *const UnicodeString, title: u32, ty: u32, data: *const u8, n: u32) -> i32;
    fn NtDeleteKey(h: HANDLE) -> i32;
    fn RtlFormatCurrentUserKeyPath(out: *mut UnicodeStringMut) -> i32;
    fn RtlFreeUnicodeString(s: *mut UnicodeStringMut);
}

#[repr(C)]
struct UnicodeStringMut {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

const KEY_QUERY_VALUE: u32 = 1;
const KEY_SET_VALUE: u32 = 2;
const KEY_CREATE_SUB_KEY: u32 = 4;
const KEY_READ_ACC: u32 = 0x20019;
const KEY_ALL_ACC: u32 = 0xF003F;
const DELETE_ACC: u32 = 0x10000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const OBJ_CI: u32 = 0x40;
const OBJ_OPENLINK: u32 = 0x100;

fn st(s: i32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    let _ = write!(b, "{:#010x}", s as u32);
    b
}

/// An object attributes block over `name` (kept alive by the caller) with the given attribute flags.
fn oa_of(name: &UnicodeString, attrs: u32, root: HANDLE) -> ObjectAttributes {
    ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: root,
        object_name: name,
        attributes: attrs,
        security_descriptor: core::ptr::null(),
        security_quality_of_service: core::ptr::null(),
    }
}

/// The NT path `\Registry\User\<sid>` of the current user as UTF-16 (from RtlFormatCurrentUserKeyPath).
fn user_root() -> ([u16; 200], usize) {
    let mut out = UnicodeStringMut { length: 0, maximum_length: 0, buffer: core::ptr::null_mut() };
    let mut p = [0u16; 200];
    let mut n = 0;
    // SAFETY: valid out structure; the buffer is freed with RtlFreeUnicodeString.
    unsafe {
        let s = RtlFormatCurrentUserKeyPath(&mut out);
        if s >= 0 {
            n = (out.length as usize / 2).min(199);
            core::ptr::copy_nonoverlapping(out.buffer, p.as_mut_ptr(), n);
            RtlFreeUnicodeString(&mut out);
        }
    }
    (p, n)
}

fn cat(a: &[u16], b: &str) -> ([u16; 300], usize) {
    let mut o = [0u16; 300];
    let mut n = 0;
    for &c in a {
        o[n] = c;
        n += 1;
    }
    for c in b.chars() {
        o[n] = c as u32 as u16;
        n += 1;
    }
    (o, n)
}

/// Opens the key `path` (UTF-16, NT form) with NtOpenKeyEx.
fn nt_open(path: &[u16], access: u32, attrs: u32, opts: u32) -> (i32, HANDLE) {
    let un = UnicodeString::of(path);
    let oa = oa_of(&un, attrs, core::ptr::null_mut());
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid structures.
    let s = unsafe { NtOpenKeyEx(&mut h, access, &oa, opts) };
    (s, h)
}

/// Creates (or opens) `path`; returns (status, handle, disposition).
fn nt_create(path: &[u16], access: u32, opts: u32) -> (i32, HANDLE, u32) {
    let un = UnicodeString::of(path);
    let oa = oa_of(&un, OBJ_CI, core::ptr::null_mut());
    let mut h: HANDLE = core::ptr::null_mut();
    let mut disp = 0u32;
    // SAFETY: valid structures.
    let s = unsafe { NtCreateKey(&mut h, access, &oa, 0, core::ptr::null(), opts, &mut disp) };
    (s, h, disp)
}

fn nt_close(h: HANDLE) -> i32 {
    // SAFETY: handle.
    unsafe { NtClose(h) }
}

fn nt_set(h: HANDLE, name: &str, ty: u32, data: &[u8]) -> i32 {
    let w = wide(name);
    let un = UnicodeString::of(&w[..wlen(&w)]);
    // SAFETY: valid buffers.
    unsafe { NtSetValueKey(h, &un, 0, ty, data.as_ptr(), data.len() as u32) }
}

fn wlen(w: &[u16]) -> usize {
    w.iter().position(|&c| c == 0).unwrap_or(w.len())
}

/// NtQueryValueKey of value `name`: (status, bytes written to the buffer by the call (`res`), the buffer)
fn nt_query(h: HANDLE, name: &str, class: u32, cap: u32) -> (i32, u32, [u8; 256]) {
    let w = wide(name);
    let un = UnicodeString::of(&w[..wlen(&w)]);
    let mut buf = [0xCCu8; 256];
    let mut res = 0xFFFF_FFFFu32;
    // SAFETY: valid buffers.
    let s = unsafe { NtQueryValueKey(h, &un, class, buf.as_mut_ptr(), cap.min(256), &mut res) };
    (s, res, buf)
}

poly_wtest::main!(test);

fn try_open(t: &mut Test, what: &str, path: &str, access: u32, attrs: u32, opts: u32) -> HANDLE {
    let w = wide(path);
    let (s, h) = nt_open(&w[..wlen(&w)], access, attrs, opts);
    say!(t, "{what}: status {} handle {}", st(s).as_str(), if h.is_null() { "NULL" } else { "set" });
    h
}

fn test(t: &mut Test) {
    let h = try_open(t, "HKLM\\SOFTWARE\\Microsoft, KEY_READ, case-insensitive", "\\Registry\\Machine\\SOFTWARE\\Microsoft", KEY_READ_ACC, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    let h = try_open(t, "same path in other case, case-insensitive", "\\registry\\machine\\software\\MICROSOFT", KEY_READ_ACC, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    let h = try_open(t, "same path in other case, no OBJ_CASE_INSENSITIVE", "\\registry\\machine\\software\\MICROSOFT", KEY_READ_ACC, 0, 0);
    if !h.is_null() {
        nt_close(h);
    }
    let h = try_open(t, "exact case, no OBJ_CASE_INSENSITIVE", "\\Registry\\Machine\\SOFTWARE\\Microsoft", KEY_READ_ACC, 0, 0);
    if !h.is_null() {
        nt_close(h);
    }
    try_open(t, "a key that does not exist", "\\Registry\\Machine\\SOFTWARE\\PolySpecB4NoSuchKey", KEY_READ_ACC, OBJ_CI, 0);
    try_open(t, "a parent that does not exist", "\\Registry\\Machine\\SOFTWARE\\PolySpecB4NoSuchKey\\Deeper", KEY_READ_ACC, OBJ_CI, 0);
    try_open(t, "empty name", "", KEY_READ_ACC, OBJ_CI, 0);
    let h = try_open(t, "\\Registry itself", "\\Registry", KEY_READ_ACC, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    let h = try_open(t, "\\Registry\\Machine", "\\Registry\\Machine", KEY_READ_ACC, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    try_open(t, "no leading backslash", "Registry\\Machine\\SOFTWARE", KEY_READ_ACC, OBJ_CI, 0);
    try_open(t, "a trailing backslash", "\\Registry\\Machine\\SOFTWARE\\Microsoft\\", KEY_READ_ACC, OBJ_CI, 0);
    try_open(t, "a Win32 path form (HKEY_LOCAL_MACHINE\\SOFTWARE)", "HKEY_LOCAL_MACHINE\\SOFTWARE", KEY_READ_ACC, OBJ_CI, 0);
    let h = try_open(t, "access MAXIMUM_ALLOWED", "\\Registry\\Machine\\SOFTWARE\\Microsoft", MAXIMUM_ALLOWED, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    let h = try_open(t, "access 0", "\\Registry\\Machine\\SOFTWARE\\Microsoft", 0, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    let h = try_open(t, "KEY_ALL_ACCESS on HKLM\\SOFTWARE\\Microsoft (elevated: allowed)", "\\Registry\\Machine\\SOFTWARE\\Microsoft", KEY_ALL_ACC, OBJ_CI, 0);
    if !h.is_null() {
        nt_close(h);
    }
    // the current user's key
    let (root, n) = user_root();
    say!(t, "RtlFormatCurrentUserKeyPath gave a path: {}", n > 0);
    let h = try_open_w(t, "the current user's key", &root[..n], KEY_READ_ACC);
    if !h.is_null() {
        // relative open through a root directory handle
        let name = wide("Software");
        let un = UnicodeString::of(&name[..wlen(&name)]);
        let oa = oa_of(&un, OBJ_CI, h);
        let mut h2: HANDLE = core::ptr::null_mut();
        // SAFETY: valid structures.
        let s = unsafe { NtOpenKeyEx(&mut h2, KEY_READ_ACC, &oa, 0) };
        say!(t, "relative open of Software below the user's key (RootDirectory): status {}", st(s).as_str());
        if s >= 0 {
            nt_close(h2);
        }
        nt_close(h);
    }
    // NtOpenKey and NtOpenKeyEx agree
    let w = wide("\\Registry\\Machine\\SOFTWARE\\Microsoft");
    let un = UnicodeString::of(&w[..wlen(&w)]);
    let oa = oa_of(&un, OBJ_CI, core::ptr::null_mut());
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid structures.
    let s = unsafe { NtOpenKey(&mut h, KEY_READ_ACC, &oa) };
    say!(t, "NtOpenKey of the same key: status {}", st(s).as_str());
    if s >= 0 {
        nt_close(h);
    }
}

fn try_open_w(t: &mut Test, what: &str, path: &[u16], access: u32) -> HANDLE {
    let (s, h) = nt_open(path, access, OBJ_CI, 0);
    say!(t, "{what}: status {} handle {}", st(s).as_str(), if h.is_null() { "NULL" } else { "set" });
    h
}
