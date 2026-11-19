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

// ---- PendingFileRenameOperations helpers (advapi32/kernel32 prototypes from Learn; constants from winreg.h/winnt.h) ----
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, opts: u32, sam: u32, out: *mut isize) -> i32;
    fn RegQueryValueExW(key: isize, name: *const u16, res: *mut u32, ty: *mut u32, data: *mut u8, len: *mut u32) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, res: u32, ty: u32, data: *const u8, len: u32) -> i32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MoveFileExW(src: *const u16, dst: *const u16, flags: u32) -> BOOL;
}

const HKLM: isize = 0x8000_0002u32 as i32 as isize;
const KEY_QUERY_VALUE: u32 = 1;
const KEY_SET_VALUE: u32 = 2;
const REG_MULTI_SZ: u32 = 7;
const MOVEFILE_REPLACE_EXISTING: u32 = 1;
const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 4;
const PFRO_N: usize = 24576;

struct Pfro {
    data: [u8; PFRO_N],
    n: usize,
    present: bool,
    ty: u32,
}

fn sm_key() -> [u16; 300] {
    wide("SYSTEM\\CurrentControlSet\\Control\\Session Manager")
}

/// Reads the value (present = false when the value does not exist).
fn pfro_read() -> Pfro {
    let mut p = Pfro { data: [0; PFRO_N], n: 0, present: false, ty: 0 };
    let k = sm_key();
    let name = wide("PendingFileRenameOperations");
    let mut h: isize = 0;
    // SAFETY: valid arguments.
    unsafe {
        if RegOpenKeyExW(HKLM, k.as_ptr(), 0, KEY_QUERY_VALUE, &mut h) != 0 {
            return p;
        }
        let mut len = PFRO_N as u32;
        let mut ty = 0u32;
        let r = RegQueryValueExW(h, name.as_ptr(), core::ptr::null_mut(), &mut ty, p.data.as_mut_ptr(), &mut len);
        if r == 0 {
            p.present = true;
            p.n = len as usize;
            p.ty = ty;
        }
        RegCloseKey(h);
    }
    p
}

/// Puts the value back exactly as it was; returns whether the registry call succeeded.
fn pfro_restore(orig: &Pfro) -> bool {
    let k = sm_key();
    let name = wide("PendingFileRenameOperations");
    let mut h: isize = 0;
    // SAFETY: valid arguments.
    unsafe {
        if RegOpenKeyExW(HKLM, k.as_ptr(), 0, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut h) != 0 {
            return false;
        }
        let r = if orig.present {
            RegSetValueExW(h, name.as_ptr(), 0, orig.ty, orig.data.as_ptr(), orig.n as u32)
        } else {
            let r = RegDeleteValueW(h, name.as_ptr());
            if r == 2 { 0 } else { r }
        };
        RegCloseKey(h);
        r == 0
    }
}

fn same(a: &Pfro, b: &Pfro) -> bool {
    a.present == b.present && a.ty == b.ty && a.n == b.n && a.data[..a.n] == b.data[..b.n]
}

/// Prints the strings added after `before`, one per line, with the scratch directory shown as <scratch>.
/// Returns whether the old content is a prefix of the new one (without its final terminator).
fn show_added(t: &mut Test, tag: &str, before: &Pfro, after: &Pfro) -> bool {
    if !after.present {
        say!(t, "{tag}: value absent after");
        return false;
    }
    let keep = if before.present && before.n >= 4 { before.n - 2 } else { 0 };
    let prefix_ok = after.n >= keep && after.data[..keep] == before.data[..keep];
    say!(t, "{tag}: type {}; the old strings are unchanged and first: {prefix_ok}", after.ty);
    let img = poly_wtest::image_path();
    let mut last = 0;
    for (i, &c) in img.iter().enumerate() {
        if c == 0x5c {
            last = i + 1;
        }
    }
    let dir = &img[..last];
    // the strings after `keep`
    let mut units = [0u16; 4096];
    let mut nu = 0;
    let mut i = keep;
    while i + 1 < after.n && nu < 4096 {
        units[nu] = u16::from_le_bytes([after.data[i], after.data[i + 1]]);
        nu += 1;
        i += 2;
    }
    if nu > 0 {
        nu -= 1; // the list's own final terminator
    }
    let mut s = 0;
    let mut idx = 0;
    while s < nu {
        let mut e = s;
        while e < nu && units[e] != 0 {
            e += 1;
        }
        if e == nu {
            break;
        }
        let str_ = &units[s..e];
        // replace the scratch directory (after an optional `!` and `\??\`)
        let mut out = Buf::<700>::new();
        let mut j = 0;
        if str_.len() >= 2 && str_[0] == 0x2a && (0x30..=0x39).contains(&str_[1]) {
            let _ = write!(out, "*{}", (str_[1] as u8) as char);
            j = 2;
        }
        if str_.get(j) == Some(&0x21) {
            j += 1;
            let _ = write!(out, "!");
        } else if false {
            let _ = write!(out, "!");
            j = 1;
        }
        let nt = [0x5cu16, 0x3f, 0x3f, 0x5c];
        if str_[j..].len() >= 4 && str_[j..j + 4] == nt {
            let _ = write!(out, "\\??\\");
            j += 4;
        }
        let rest = &str_[j..];
        if rest.len() >= dir.len() && eq_nocase(&rest[..dir.len()], dir) {
            let _ = write!(out, "<scratch>\\{}", units_text(&rest[dir.len()..]).as_str());
        } else {
            let _ = write!(out, "{}", units_text(rest).as_str());
        }
        say!(t, "{tag} added[{idx}]: {}", out.as_str());
        idx += 1;
        s = e + 1;
    }
    prefix_ok
}
// MoveFileExW(file, new name, MOVEFILE_DELAY_UNTIL_REBOOT [| REPLACE_EXISTING]): the pairs added to PendingFileRenameOperations.
poly_wtest::main!(test);

fn test(t: &mut Test) {
    let orig = pfro_read();
    let a = scratch("mvr_a.tmp");
    let b = scratch("mvr_b.tmp");
    let c = scratch("mvr_c.tmp");
    let d = scratch("mvr_d.tmp");
    write_all(&a, b"abc");
    write_all(&c, b"abc");

    clear_error();
    // SAFETY: valid NUL-terminated paths.
    let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    say!(t, "rename a to b at reboot: result {}, last error {}", ok != 0, gle_if(ok));
    say!(t, "a still exists: {}; b exists: {}", exists(&a), exists(&b));
    let after1 = pfro_read();
    show_added(t, "rename", &orig, &after1);

    // SAFETY: valid paths.
    let ok = unsafe { MoveFileExW(c.as_ptr(), d.as_ptr(), MOVEFILE_DELAY_UNTIL_REBOOT | MOVEFILE_REPLACE_EXISTING) };
    say!(t, "rename c to d at reboot with REPLACE_EXISTING: result {}", ok != 0);
    let after2 = pfro_read();
    show_added(t, "rename+replace", &orig, &after2);

    // a destination in a directory that does not exist: accepted, only recorded
    let nodir = scratch("mvr_nodir\\x.tmp");
    // SAFETY: valid paths.
    let ok = unsafe { MoveFileExW(c.as_ptr(), nodir.as_ptr(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    say!(t, "destination in a missing directory: result {}, last error {}", ok != 0, gle_if(ok));

    // a relative source name is recorded as a full path
    let rel = wide("mvr_rel_missing.tmp");
    write_all(&scratch("mvr_rel.tmp"), b"x");
    // SAFETY: valid path.
    let ok = unsafe { MoveFileExW(rel.as_ptr(), core::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    say!(t, "a source that does not exist (relative name): result {}, last error {}", ok != 0, gle_if(ok));
    let after3 = pfro_read();
    say!(t, "the value is longer than after the two renames: {}", after3.n > after2.n);

    let restored = pfro_restore(&orig);
    let now = pfro_read();
    say!(t, "restored, the value is as it was (bytes, kind, presence): {} {}", restored, same(&orig, &now));
    // SAFETY: valid paths.
    unsafe {
        DeleteFileW(a.as_ptr());
        DeleteFileW(c.as_ptr());
        DeleteFileW(scratch("mvr_rel.tmp").as_ptr());
    }
}
