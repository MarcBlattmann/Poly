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
// C:\Windows\SystemTemp: the directory exists, its attributes and security descriptor, what a caller may list and open.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn GetWindowsDirectoryW(buf: *mut u16, len: u32) -> u32;
    fn CreateFileW(path: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn FindFirstFileW(path: *const u16, data: *mut u8) -> HANDLE;
    fn FindClose(h: HANDLE) -> BOOL;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileSecurityW(path: *const u16, info: u32, sd: *mut u8, len: u32, needed: *mut u32) -> BOOL;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(sd: *const u8, rev: u32, info: u32, out: *mut *mut u16, len: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const INVALID: usize = usize::MAX;

fn open_dir(t: &mut Test, label: &str, access: u32) {
    let p = wide("C:\\Windows\\SystemTemp");
    clear_error();
    // SAFETY: valid string.
    let h = unsafe { CreateFileW(p.as_ptr(), access, 7, core::ptr::null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, core::ptr::null_mut()) };
    let ok = h as usize != INVALID;
    say!(t, "open directory with {label}: {} err {}", ok, if ok { -2 } else { gle() });
    if ok {
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
    }
}

fn test(t: &mut Test) {
    let p = wide("C:\\Windows\\SystemTemp");
    // is the Windows directory C:\Windows?
    let mut w = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetWindowsDirectoryW(w.as_mut_ptr(), 300) };
    say!(t, "Windows directory is C:\\Windows: {}", eq_nocase(&w[..n as usize], &wide("C:\\Windows")[..10]));
    // SAFETY: valid string.
    let a = unsafe { GetFileAttributesW(p.as_ptr()) };
    say!(t, "attributes: invalid={} directory={} attrs&0x1f1={:#x}", a == 0xFFFF_FFFF, a & FILE_ATTRIBUTE_DIRECTORY != 0, a & 0x1f1);
    open_dir(t, "FILE_READ_ATTRIBUTES (0x80)", 0x80);
    open_dir(t, "READ_CONTROL (0x20000)", 0x20000);
    open_dir(t, "FILE_LIST_DIRECTORY (1)", 1);
    open_dir(t, "FILE_ADD_FILE (2)", 2);
    open_dir(t, "FILE_ADD_SUBDIRECTORY (4)", 4);
    open_dir(t, "FILE_TRAVERSE (0x20)", 0x20);
    open_dir(t, "DELETE (0x10000)", 0x10000);
    open_dir(t, "WRITE_DAC (0x40000)", 0x40000);
    open_dir(t, "GENERIC_READ", 0x8000_0000);
    // listing
    let pat = wide("C:\\Windows\\SystemTemp\\*");
    let mut data = [0u8; 600];
    clear_error();
    // SAFETY: valid buffers.
    let h = unsafe { FindFirstFileW(pat.as_ptr(), data.as_mut_ptr()) };
    let ok = h as usize != INVALID;
    say!(t, "FindFirstFileW on SystemTemp\\*: {} err {}", ok, if ok { -2 } else { gle() });
    if ok {
        // SAFETY: valid handle.
        unsafe { FindClose(h) };
    }
    // the security descriptor: owner, group, DACL as SDDL
    for (name, info) in [("owner|group|dacl", 7u32), ("dacl", 4), ("owner", 1)] {
        let mut sd = [0u8; 2048];
        let mut need = 0u32;
        clear_error();
        // SAFETY: valid buffers.
        let ok = unsafe { GetFileSecurityW(p.as_ptr(), info, sd.as_mut_ptr(), 2048, &mut need) };
        if ok == 0 {
            say!(t, "GetFileSecurityW {name}: failed err {}", gle());
            continue;
        }
        let mut out: *mut u16 = core::ptr::null_mut();
        let mut l = 0u32;
        // SAFETY: valid buffers.
        let ok = unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(sd.as_ptr(), 1, info, &mut out, &mut l) };
        if ok != 0 {
            let s = wstr_at(out, 800);
            say!(t, "SDDL {name}: {}", units_text(s).as_str());
            // SAFETY: LocalAlloc'd by the call.
            unsafe { LocalFree(out.cast()) };
        } else {
            say!(t, "SDDL {name}: conversion failed err {}", gle());
        }
    }
}

// recorded with the elevated (full) administrator token
