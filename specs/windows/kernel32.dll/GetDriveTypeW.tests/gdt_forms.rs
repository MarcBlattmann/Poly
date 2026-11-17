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
// GetDriveTypeW: NT-style prefixes, UNC names, device names, odd strings, the NULL and empty arguments.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetCurrentDirectoryW(len: u32, buf: *mut u16) -> u32;
    fn SetCurrentDirectoryW(path: *const u16) -> BOOL;
}

poly_wtest::main!(test);

fn gdt(t: &mut Test, label: &str, s: &str) {
    let w = wide(s);
    clear_error();
    // SAFETY: NUL-terminated buffer.
    let r = unsafe { GetDriveTypeW(w.as_ptr()) };
    say!(t, "{label} => {r}");
}

fn test(t: &mut Test) {
    gdt(t, "\\\\?\\C:\\", "\\\\?\\C:\\");
    gdt(t, "\\\\?\\C: no backslash", "\\\\?\\C:");
    gdt(t, "\\\\.\\C:\\", "\\\\.\\C:\\");
    gdt(t, "\\\\.\\C: no backslash", "\\\\.\\C:");
    gdt(t, "\\??\\C:\\", "\\??\\C:\\");
    gdt(t, "\\\\?\\GLOBALROOT\\", "\\\\?\\GLOBALROOT\\");
    gdt(t, "\\ (root of the current drive)", "\\");
    gdt(t, "\\Windows (rooted, no drive)", "\\Windows");
    gdt(t, "Windows (relative)", "Windows");
    gdt(t, "empty string", "");
    gdt(t, "a space", " ");
    gdt(t, "\\\\no-such-host-xyz\\share\\", "\\\\no-such-host-xyz\\share\\");
    gdt(t, "CONOUT$", "CONOUT$");
    gdt(t, "NUL", "NUL");
    // NULL = the root of the current directory's drive
    clear_error();
    // SAFETY: NULL is documented.
    let r = unsafe { GetDriveTypeW(core::ptr::null()) };
    say!(t, "NULL => {r}");
    // after changing the directory the NULL form follows it
    let mut cwd = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetCurrentDirectoryW(300, cwd.as_mut_ptr()) };
    say!(t, "cwd is on a drive letter: {}", n >= 3 && cwd[1] == 0x3a);
    let sys = wide("C:\\Windows");
    // SAFETY: valid string.
    let ok = unsafe { SetCurrentDirectoryW(sys.as_ptr()) };
    // SAFETY: NULL is documented.
    let r2 = unsafe { GetDriveTypeW(core::ptr::null()) };
    say!(t, "after chdir C:\\Windows: ok={ok} NULL => {r2}");
    // SAFETY: valid string.
    unsafe { SetCurrentDirectoryW(cwd.as_ptr()) };
}
