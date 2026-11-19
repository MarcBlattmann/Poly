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

// ---- file helpers (kernel32; prototypes from Learn) ----
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn RemoveDirectoryW(name: *const u16) -> BOOL;
    fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> BOOL;
    fn GetFileSize(h: HANDLE, hi: *mut u32) -> u32;
    fn GetFileAttributesW(name: *const u16) -> u32;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

/// The path of `name` in the directory of the running image (a fresh scratch directory per run on Windows), as UTF-16.
fn scratch(name: &str) -> [u16; 300] {
    let img = poly_wtest::image_path();
    let mut out = [0u16; 300];
    let mut last = 0;
    for (i, &c) in img.iter().enumerate() {
        if c == 0x5c {
            last = i + 1;
        }
    }
    let mut n = 0;
    for &c in &img[..last] {
        if n < 299 {
            out[n] = c;
            n += 1;
        }
    }
    for c in name.chars() {
        if n < 299 {
            out[n] = c as u32 as u16;
            n += 1;
        }
    }
    out
}

/// Reads a whole file (up to N bytes); `None` when it cannot be opened.
fn read_all<const N: usize>(path: &[u16]) -> Option<([u8; N], usize)> {
    let mut buf = [0u8; N];
    let mut got = 0u32;
    // SAFETY: valid NUL-terminated path and buffer.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_READ, 7, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            return None;
        }
        ReadFile(h, buf.as_mut_ptr(), N as u32, &mut got, core::ptr::null_mut());
        CloseHandle(h);
    }
    Some((buf, got as usize))
}

/// Writes a whole file; false on failure.
fn write_all(path: &[u16], data: &[u8]) -> bool {
    let mut got = 0u32;
    // SAFETY: valid NUL-terminated path and data.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            return false;
        }
        let ok = WriteFile(h, data.as_ptr(), data.len() as u32, &mut got, core::ptr::null_mut());
        CloseHandle(h);
        ok != 0 && got as usize == data.len()
    }
}

fn exists(path: &[u16]) -> bool {
    // SAFETY: valid NUL-terminated path.
    unsafe { GetFileAttributesW(path.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

fn hexdump(t: &mut Test, d: &[u8]) {
    for (i, ch) in d.chunks(32).enumerate() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "{:04x}:", i * 32);
        for x in ch {
            let _ = write!(b, " {:02x}", x);
        }
        t.line(b.as_str());
    }
}

// ---- registry helpers (advapi32; prototypes from Learn, constants from winreg.h / winnt.h) ----
type HKEY = usize;
const HKEY_CURRENT_USER: HKEY = 0x8000_0001;
const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002;
const HKEY_USERS: HKEY = 0x8000_0003;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const KEY_READ: u32 = 0x20019;
const REG_SZ: u32 = 1;
const REG_BINARY: u32 = 3;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(h: HKEY, sub: *const u16, res: u32, class: *const u16, opts: u32, sam: u32, sa: *const c_void, out: *mut HKEY, disp: *mut u32) -> i32;
    fn RegOpenKeyExW(h: HKEY, sub: *const u16, opts: u32, sam: u32, out: *mut HKEY) -> i32;
    fn RegCloseKey(h: HKEY) -> i32;
    fn RegSetValueExW(h: HKEY, name: *const u16, res: u32, ty: u32, data: *const u8, n: u32) -> i32;
    fn RegQueryValueExW(h: HKEY, name: *const u16, res: *mut u32, ty: *mut u32, data: *mut u8, n: *mut u32) -> i32;
    fn RegDeleteKeyW(h: HKEY, sub: *const u16) -> i32;
    fn RegDeleteTreeW(h: HKEY, sub: *const u16) -> i32;
    fn RegEnumValueA(h: HKEY, idx: u32, name: *mut u8, nlen: *mut u32, res: *mut u32, ty: *mut u32, data: *mut u8, dlen: *mut u32) -> i32;
    fn RegEnumValueW(h: HKEY, idx: u32, name: *mut u16, nlen: *mut u32, res: *mut u32, ty: *mut u32, data: *mut u8, dlen: *mut u32) -> i32;
    fn RegSaveKeyW(h: HKEY, file: *const u16, sa: *const c_void) -> i32;
    fn RegLoadKeyW(h: HKEY, sub: *const u16, file: *const u16) -> i32;
    fn RegUnLoadKeyW(h: HKEY, sub: *const u16) -> i32;
    fn OpenProcessToken(p: HANDLE, access: u32, out: *mut HANDLE) -> BOOL;
    fn LookupPrivilegeValueW(sys: *const u16, name: *const u16, luid: *mut u64) -> BOOL;
    fn AdjustTokenPrivileges(t: HANDLE, all: BOOL, new: *const u8, n: u32, old: *mut u8, retn: *mut u32) -> BOOL;
}

/// Enables a privilege by name; returns (AdjustTokenPrivileges result, last error afterwards: 0 or 1300 when not held).
fn enable_priv(name: &str) -> (i32, i64) {
    let w = wide(name);
    let mut luid = 0u64;
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: valid pointers.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), 0x20, &mut tok) == 0 {
            return (-1, gle());
        }
        if LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut luid) == 0 {
            return (-2, gle());
        }
        // TOKEN_PRIVILEGES: count (4), LUID (8), attributes (4)
        let mut tp = [0u8; 16];
        tp[0] = 1;
        tp[4..12].copy_from_slice(&luid.to_le_bytes());
        tp[12] = 2; // SE_PRIVILEGE_ENABLED
        SetLastError(0xDEAD);
        let ok = AdjustTokenPrivileges(tok, 0, tp.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut());
        let e = gle();
        CloseHandle(tok);
        (ok, e)
    }
}

fn wlen(w: &[u16]) -> usize {
    w.iter().position(|&c| c == 0).unwrap_or(w.len())
}

fn set_sz(h: HKEY, name: &str, val: &str) -> i32 {
    let n = wide(name);
    let v = wide(val);
    let bytes = (wlen(&v) + 1) * 2;
    // SAFETY: valid buffers.
    unsafe { RegSetValueExW(h, n.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, bytes as u32) }
}
fn set_dw(h: HKEY, name: &str, val: u32) -> i32 {
    let n = wide(name);
    // SAFETY: valid buffers.
    unsafe { RegSetValueExW(h, n.as_ptr(), 0, REG_DWORD, val.to_le_bytes().as_ptr(), 4) }
}
/// (status, type, data length, first up to 64 data bytes)
fn get_val(h: HKEY, name: &str) -> (i32, u32, u32, [u8; 64]) {
    let n = wide(name);
    let mut ty = 0u32;
    let mut len = 64u32;
    let mut d = [0u8; 64];
    // SAFETY: valid buffers.
    let r = unsafe { RegQueryValueExW(h, n.as_ptr(), core::ptr::null_mut(), &mut ty, d.as_mut_ptr(), &mut len) };
    (r, ty, len, d)
}
fn data_text(d: &[u8], len: usize) -> Buf<200> {
    let mut b = Buf::<200>::new();
    let n = len.min(64) / 2;
    for i in 0..n {
        let c = u16::from_le_bytes([d[2 * i], d[2 * i + 1]]);
        if c == 0 {
            break;
        }
        let _ = write!(b, "{}", c as u8 as char);
    }
    b
}
fn create_key(parent: HKEY, sub: &str) -> (i32, HKEY, u32) {
    let w = wide(sub);
    let mut h: HKEY = 0;
    let mut disp = 0u32;
    // SAFETY: valid buffers.
    let r = unsafe { RegCreateKeyExW(parent, w.as_ptr(), 0, core::ptr::null(), 0, KEY_ALL_ACCESS, core::ptr::null(), &mut h, &mut disp) };
    (r, h, disp)
}
fn open_key(parent: HKEY, sub: &str, sam: u32) -> (i32, HKEY) {
    let w = wide(sub);
    let mut h: HKEY = 0;
    // SAFETY: valid buffers.
    let r = unsafe { RegOpenKeyExW(parent, w.as_ptr(), 0, sam, &mut h) };
    (r, h)
}
fn del_tree(parent: HKEY, sub: &str) -> i32 {
    let w = wide(sub);
    // SAFETY: valid buffer.
    unsafe { RegDeleteTreeW(parent, w.as_ptr()) }
}
fn del_key(parent: HKEY, sub: &str) -> i32 {
    let w = wide(sub);
    // SAFETY: valid buffer.
    unsafe { RegDeleteKeyW(parent, w.as_ptr()) }
}

/// Opens a key and closes it again; returns the open status.
fn key_status(parent: HKEY, sub: &str, sam: u32) -> i32 {
    let (r, h) = open_key(parent, sub, sam);
    if r == 0 {
        // SAFETY: handle just opened.
        unsafe { RegCloseKey(h) };
    }
    r
}

// recorded with the standard (filtered) token: the privileges are not held
poly_wtest::main!(test);

const SUB: &str = "Software\\PolySpecB4HiveStdUnlSrc";

fn test(t: &mut Test) {
    let _ = del_tree(HKEY_CURRENT_USER, SUB);
    let (a, e) = enable_priv("SeBackupPrivilege");
    say!(t, "enable SeBackupPrivilege: adjust {a} last error {e}");
    let (a, e) = enable_priv("SeRestorePrivilege");
    say!(t, "enable SeRestorePrivilege: adjust {a} last error {e}");
    let file = scratch("hive_std_unload.hiv");
    let (r, h, _) = create_key(HKEY_CURRENT_USER, SUB);
    say!(t, "create source key in HKCU: {r}");
    set_sz(h, "S", "v");
    // SAFETY: handle.
    unsafe { RegCloseKey(h) };
    let w = wide("PolySpecB4HiveStdU");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW of a name that is not loaded (HKLM): {r}");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_USERS, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW (HKU): {r}");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_CURRENT_USER, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW (HKEY_CURRENT_USER): {r}");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, core::ptr::null()) };
    say!(t, "RegUnLoadKeyW with a NULL name: {r}");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(0, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW with a NULL key handle: {r}");
    // SAFETY: valid buffers.
    unsafe { DeleteFileW(file.as_ptr()) };
    let _ = del_tree(HKEY_CURRENT_USER, SUB);
    say!(t, "source key left behind: {}", key_status(HKEY_CURRENT_USER, SUB, KEY_READ) == 0);
}
