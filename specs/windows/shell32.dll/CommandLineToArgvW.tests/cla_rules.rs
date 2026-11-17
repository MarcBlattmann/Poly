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
// CommandLineToArgvW: the quoting rules for arguments after the first, the special first argument, separators.

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CommandLineToArgvW(cmd: *const u16, argc: *mut i32) -> *mut *mut u16;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(h: *mut c_void) -> *mut c_void;
}

poly_wtest::main!(test);

fn show(t: &mut Test, label: &str, s: &str) {
    let w = wide(s);
    let mut argc = -77i32;
    // SAFETY: valid NUL-terminated string and out pointer.
    let argv = unsafe { CommandLineToArgvW(w.as_ptr(), &mut argc) };
    if argv.is_null() {
        say!(t, "{label}: NULL argc {argc}");
        return;
    }
    let mut b = Buf::<700>::new();
    let _ = write!(b, "{label}: argc {argc} [");
    for i in 0..argc.max(0) as usize {
        // SAFETY: argv has argc entries.
        let p = unsafe { *argv.add(i) };
        let u = wstr_at(p, 200);
        let _ = write!(b, "<{}>", units_text(u).as_str());
    }
    // SAFETY: argv[argc] exists (the array ends with a NULL pointer).
    let term = unsafe { (*argv.add(argc.max(0) as usize)).is_null() };
    let _ = write!(b, "] ends with NULL pointer {term}");
    t.line(b.as_str());
    // SAFETY: the block was allocated for us; freeing it with LocalFree is the documented way.
    unsafe { LocalFree(argv.cast()) };
}

fn test(t: &mut Test) {
    // arguments after the first: documented rules (backslashes before a quote)
    show(t, "plain", r#"p a b c"#);
    show(t, "quoted", r#"p "a b" c"#);
    show(t, "quote in word", r#"p a"b c"d e"#);
    show(t, "2 backslashes", r#"p a\\b"#);
    show(t, "backslash quote", r#"p a\"b"#);
    show(t, "2 backslashes quote", r#"p a\\"b c" d"#);
    show(t, "3 backslashes quote", r#"p a\\\"b c"#);
    show(t, "4 backslashes quote", r#"p "a\\\\" b"#);
    show(t, "backslashes at end of quoted", r#"p "C:\a b\" c"#);
    show(t, "backslashes no quote", r#"p \\\a \b"#);
    show(t, "empty quoted", r#"p "" a"#);
    show(t, "two empty", r#"p "" """#);
    show(t, "double quote in quoted", r#"p "a""b" c"#);
    show(t, "triple quote", r#"p """a" c"#);
    show(t, "quote pair after word", r#"p a"" b"#);
    show(t, "unterminated", r#"p "abc def"#);
    show(t, "unterminated backslash", r#"p "abc\"#);
    show(t, "unc path", r#"p \\server\share\ x"#);
    show(t, "flag with equals", r#"p --installerdata="C:\Windows\SystemTemp\a b\x.tmp" --x"#);
    show(t, "tabs", "p a\tb\t\tc");
    show(t, "many spaces", "p    a     b   ");
    show(t, "tab in quotes", "p \"a\tb\" c");
    show(t, "trailing space", "p a ");
    show(t, "long arg", r#"p aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa b"#);
    // the first argument
    show(t, "argv0 plain", r#"C:\a\b.exe x"#);
    show(t, "argv0 quoted", r#""C:\Program Files\a b.exe" x"#);
    show(t, "argv0 backslash quote", r#""C:\a\" b" x"#);
    show(t, "argv0 unquoted backslash quote", r#"C:\a\"b c d"#);
    show(t, "argv0 quote in middle", r#"C:\a"b c".exe x"#);
    show(t, "argv0 only", r#"C:\a\b.exe"#);
    show(t, "argv0 quoted no space after", r#""a b"x y"#);
    show(t, "argv0 tab", "a\tb");
    show(t, "argv0 unterminated", r#""a b"#);
    show(t, "argv0 empty quoted", r#""" x"#);
    show(t, "argv0 two quotes then arg", r#""""x y"#);
}
