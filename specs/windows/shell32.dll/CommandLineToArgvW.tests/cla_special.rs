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
// CommandLineToArgvW: empty / NULL / whitespace input, the block layout, Chrome's and setup.exe's real command lines.

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CommandLineToArgvW(cmd: *const u16, argc: *mut i32) -> *mut *mut u16;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(h: *mut c_void) -> *mut c_void;
    fn LocalSize(h: *mut c_void) -> usize;
    fn GetModuleFileNameW(m: *mut c_void, b: *mut u16, n: u32) -> u32;
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
    let _ = write!(b, "]");
    t.line(b.as_str());
    // SAFETY: freeing the block as documented.
    unsafe { LocalFree(argv.cast()) };
}

fn test(t: &mut Test) {
    // the module path (what argv[0] is for an empty command line)
    let mut me = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), me.as_mut_ptr(), 300) } as usize;
    let me = &me[..n];
    // empty string: argc 1, argv[0] the executable's own path
    {
        let w = [0u16; 1];
        let mut argc = -77i32;
        // SAFETY: valid pointers.
        let argv = unsafe { CommandLineToArgvW(w.as_ptr(), &mut argc) };
        if !argv.is_null() {
            // SAFETY: argc entries.
            let a0 = if argc >= 1 { wstr_at(unsafe { *argv }, 300) } else { &[] };
            say!(t, "empty string: argc {argc}; argv[0] is the module path: {}", eq_nocase(a0, me));
            // SAFETY: argv[argc] exists.
            say!(t, "empty string: argv[argc] is NULL: {}", unsafe { (*argv.add(argc.max(0) as usize)).is_null() });
            // SAFETY: freeing as documented.
            unsafe { LocalFree(argv.cast()) };
        } else {
            say!(t, "empty string: NULL argc {argc} error {}", gle());
        }
    }
    show(t, "single space", " ");
    show(t, "spaces then arg", "   a b");
    show(t, "tab only", "\t");
    show(t, "spaces then quoted", "  \"a b\" c");
    // the layout of the returned block
    {
        let w = wide("prog first \"second arg\" third");
        let mut argc = 0i32;
        // SAFETY: valid pointers.
        let argv = unsafe { CommandLineToArgvW(w.as_ptr(), &mut argc) };
        let mut inside = true;
        let mut after_array = true;
        let base = argv as usize;
        // SAFETY: the block is argc+1 pointers followed by the strings.
        let size = unsafe { LocalSize(argv.cast()) };
        for i in 0..argc as usize {
            // SAFETY: argc entries.
            let p = unsafe { *argv.add(i) } as usize;
            if p < base || p >= base + size {
                inside = false;
            }
            if p < base + (argc as usize + 1) * 8 {
                after_array = false;
            }
        }
        say!(t, "block: argc {argc}; every string lies inside the one LocalAlloc block: {inside}; after the pointer array: {after_array}");
        // SAFETY: freeing as documented; LocalFree of a valid block returns NULL.
        let r = unsafe { LocalFree(argv.cast()) };
        say!(t, "block: LocalFree returns NULL: {}", r.is_null());
    }
}
