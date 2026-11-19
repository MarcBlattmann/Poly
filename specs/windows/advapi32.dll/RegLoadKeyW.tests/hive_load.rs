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

poly_wtest::main!(test);

const SUB: &str = "Software\\PolySpecB4HiveLoadSrc";
const LOADED: &str = "PolySpecB4HiveLoad";

fn cleanup() {
    let _ = del_tree(HKEY_CURRENT_USER, SUB);
    let w = wide(LOADED);
    let w2 = wide("PolySpecB4HiveLoad2");
    // SAFETY: valid buffers.
    unsafe {
        RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr());
        RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w2.as_ptr());
        RegUnLoadKeyW(HKEY_USERS, w.as_ptr());
    }
}

fn test(t: &mut Test) {
    cleanup();
    let (a, e) = enable_priv("SeBackupPrivilege");
    say!(t, "enable SeBackupPrivilege: adjust {a} last error {e}");
    let (a, e) = enable_priv("SeRestorePrivilege");
    say!(t, "enable SeRestorePrivilege: adjust {a} last error {e}");
    let file = scratch("src.hiv");
    let (_, h, _) = create_key(HKEY_CURRENT_USER, SUB);
    set_sz(h, "S", "text value");
    set_dw(h, "D", 42);
    let (_, hs, _) = create_key(h, "Sub");
    set_sz(hs, "X", "inner");
    // SAFETY: handle.
    unsafe { RegCloseKey(hs) };
    // SAFETY: valid buffers.
    let r = unsafe { RegSaveKeyW(h, file.as_ptr(), core::ptr::null()) };
    say!(t, "RegSaveKeyW: {r}");
    // SAFETY: handle.
    unsafe { RegCloseKey(h) };
    let w = wide(LOADED);

    // load into HKLM, read, write
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr(), file.as_ptr()) };
    say!(t, "RegLoadKeyW into HKLM: {r}");
    let (r, hl) = open_key(HKEY_LOCAL_MACHINE, LOADED, KEY_ALL_ACCESS);
    say!(t, "open loaded hive: {r}");
    let (r, ty, len, d) = get_val(hl, "S");
    say!(t, "loaded value S: {r} type {ty} len {len} text '{}'", data_text(&d, len as usize).as_str());
    let (r, ty, _, d) = get_val(hl, "D");
    say!(t, "loaded value D: {r} type {ty} data {}", u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
    let (r, hs) = open_key(hl, "Sub", KEY_READ);
    say!(t, "loaded subkey Sub: {r}");
    let (r, ty, len, d) = get_val(hs, "X");
    say!(t, "loaded value Sub\\X: {r} type {ty} text '{}'", data_text(&d, len as usize).as_str());
    // SAFETY: handle.
    unsafe { RegCloseKey(hs) };
    say!(t, "the saved key's own name is not part of the loaded tree (open PolySpecB4HiveLoadSrc): {}", key_status(hl, "PolySpecB4HiveLoadSrc", KEY_READ));
    say!(t, "write into the loaded hive: {}", set_dw(hl, "N", 7));
    let (r, hn, disp) = create_key(hl, "NewSub");
    say!(t, "create a subkey in the loaded hive: {r} new {}", disp == 1);
    // SAFETY: handle.
    unsafe { RegCloseKey(hn) };

    // loading again
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr(), file.as_ptr()) };
    say!(t, "RegLoadKeyW again, same name, same file: {r}");
    let w2 = wide("PolySpecB4HiveLoad2");
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w2.as_ptr(), file.as_ptr()) };
    say!(t, "RegLoadKeyW again, other name, same file: {r}");
    say!(t, "the second name exists: {}", key_status(HKEY_LOCAL_MACHINE, "PolySpecB4HiveLoad2", KEY_READ));
    // unload with an open handle
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW with a handle still open: {r}");
    say!(t, "the loaded key is still there: {}", key_status(HKEY_LOCAL_MACHINE, LOADED, KEY_READ));
    // SAFETY: handle.
    unsafe { RegCloseKey(hl) };
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW after closing: {r}");
    say!(t, "open after unload: {}", key_status(HKEY_LOCAL_MACHINE, LOADED, KEY_READ));
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW again (not loaded): {r}");
    say!(t, "log files beside the hive: LOG1 {}; LOG2 {}", exists(&scratch("src.hiv.LOG1")), exists(&scratch("src.hiv.LOG2")));

    // reload into HKEY_USERS: the changes persisted
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_USERS, w.as_ptr(), file.as_ptr()) };
    say!(t, "RegLoadKeyW into HKEY_USERS: {r}");
    let (r, hl) = open_key(HKEY_USERS, LOADED, KEY_READ);
    let (r2, ty, _, d) = get_val(hl, "N");
    say!(t, "reloaded: open {r}; value N written earlier: {r2} type {ty} data {}", u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
    say!(t, "reloaded: subkey NewSub created earlier: {}", key_status(hl, "NewSub", KEY_READ));
    // SAFETY: handle.
    unsafe { RegCloseKey(hl) };
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_USERS, w.as_ptr()) };
    say!(t, "RegUnLoadKeyW from HKEY_USERS: {r}");

    // a hive loaded in HKLM is not visible in HKU and vice versa
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr(), file.as_ptr()) };
    say!(t, "load into HKLM again: {r}; visible in HKU: {}", key_status(HKEY_USERS, LOADED, KEY_READ));
    // SAFETY: valid buffer.
    unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr()) };

    // bad arguments
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_CURRENT_USER, w.as_ptr(), file.as_ptr()) };
    say!(t, "RegLoadKeyW into HKEY_CURRENT_USER: {r}");
    let junk = scratch("junk.hiv");
    write_all(&junk, b"this is not a registry hive file, only text");
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr(), junk.as_ptr()) };
    say!(t, "RegLoadKeyW of a file that is not a hive: {r}; key exists {}", key_status(HKEY_LOCAL_MACHINE, LOADED, KEY_READ));
    let empty = wide("");
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, empty.as_ptr(), file.as_ptr()) };
    say!(t, "RegLoadKeyW with an empty key name: {r}");
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, core::ptr::null(), file.as_ptr()) };
    say!(t, "RegLoadKeyW with a NULL key name: {r}");
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr(), core::ptr::null()) };
    say!(t, "RegLoadKeyW with a NULL file: {r}");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, wide("PolySpecB4NeverLoaded").as_ptr()) };
    say!(t, "RegUnLoadKeyW of a name that was never loaded: {r}");
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, wide("SOFTWARE\\Microsoft").as_ptr()) };
    say!(t, "RegUnLoadKeyW of an ordinary key (not a loaded hive root): {r}");

    // a missing file is created as an empty hive
    let nofile = scratch("no-such.hiv");
    say!(t, "no-such.hiv exists before: {}", exists(&nofile));
    // SAFETY: valid buffers.
    let r = unsafe { RegLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr(), nofile.as_ptr()) };
    say!(t, "RegLoadKeyW of a missing file: {r}; the file exists now: {}", exists(&nofile));
    let (r, hl) = open_key(HKEY_LOCAL_MACHINE, LOADED, KEY_READ);
    say!(t, "the new hive opens: {r}; value S in it: {}", get_val(hl, "S").0);
    // SAFETY: handle.
    unsafe { RegCloseKey(hl) };
    // SAFETY: valid buffer.
    let r = unsafe { RegUnLoadKeyW(HKEY_LOCAL_MACHINE, w.as_ptr()) };
    say!(t, "unload it: {r}");

    cleanup();
    for n in ["src.hiv", "src.hiv.LOG1", "src.hiv.LOG2", "junk.hiv", "no-such.hiv", "no-such.hiv.LOG1", "no-such.hiv.LOG2"] {
        // SAFETY: valid buffer.
        unsafe { DeleteFileW(scratch(n).as_ptr()) };
    }
    say!(t, "loaded names left behind: {} {}", key_status(HKEY_LOCAL_MACHINE, LOADED, KEY_READ) == 0, key_status(HKEY_USERS, LOADED, KEY_READ) == 0);
    say!(t, "source key left behind: {}", key_status(HKEY_CURRENT_USER, SUB, KEY_READ) == 0);
}
