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
// WinHttpSetOption / WinHttpQueryOption on a session handle (no network traffic: nothing is sent).

#[link(name = "winhttp", kind = "raw-dylib")]
unsafe extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> HANDLE;
    fn WinHttpConnect(session: HANDLE, server: *const u16, port: u16, reserved: u32) -> HANDLE;
    fn WinHttpOpenRequest(connect: HANDLE, verb: *const u16, object: *const u16, version: *const u16, referrer: *const u16, accept: *mut *const u16, flags: u32) -> HANDLE;
    fn WinHttpCloseHandle(h: HANDLE) -> BOOL;
    fn WinHttpSetOption(h: HANDLE, option: u32, buf: *mut c_void, len: u32) -> BOOL;
    fn WinHttpQueryOption(h: HANDLE, option: u32, buf: *mut c_void, len: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

/// Sets a 4-byte option, then queries it; prints both results.
fn u32_option(t: &mut Test, label: &str, h: HANDLE, opt: u32, value: u32) {
    let mut v = value;
    clear_error();
    // SAFETY: valid buffer of 4 bytes.
    let ok = unsafe { WinHttpSetOption(h, opt, (&mut v as *mut u32).cast(), 4) };
    let e = gle_if(ok);
    let mut out = [0xAAu8; 16];
    let mut n = 16u32;
    clear_error();
    // SAFETY: valid buffers.
    let q = unsafe { WinHttpQueryOption(h, opt, out.as_mut_ptr().cast(), &mut n) };
    let qe = gle_if(q);
    let got = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
    say!(t, "{label}: set {value:#x} -> {ok} (error {e}); query -> {q} (error {qe}), length {n}, value {}", if q != 0 { got as i64 } else { -1 });
}

fn test(t: &mut Test) {
    let agent = wide("polyspecb4");
    // SAFETY: valid strings.
    let s = unsafe { WinHttpOpen(agent.as_ptr(), 0, core::ptr::null(), core::ptr::null(), 0) };
    say!(t, "session opened: {}", !s.is_null());
    // session handle options
    u32_option(t, "REDIRECT_POLICY NEVER", s, 88, 0);
    u32_option(t, "REDIRECT_POLICY DISALLOW_HTTPS_TO_HTTP", s, 88, 1);
    u32_option(t, "REDIRECT_POLICY ALWAYS", s, 88, 2);
    u32_option(t, "REDIRECT_POLICY 3", s, 88, 3);
    u32_option(t, "SECURE_PROTOCOLS TLS1.2", s, 84, 0x800);
    u32_option(t, "SECURE_PROTOCOLS TLS1.2|1.3", s, 84, 0x2800);
    u32_option(t, "SECURE_PROTOCOLS 0", s, 84, 0);
    u32_option(t, "DECOMPRESSION ALL", s, 118, 3);
    u32_option(t, "DECOMPRESSION GZIP", s, 118, 1);
    u32_option(t, "DECOMPRESSION 0", s, 118, 0);
    u32_option(t, "DECOMPRESSION 4", s, 118, 4);
    u32_option(t, "CONNECT_RETRIES 7", s, 4, 7);
    u32_option(t, "MAX_CONNS_PER_SERVER 12", s, 73, 12);
    u32_option(t, "MAX_CONNS_PER_1_0_SERVER 3", s, 74, 3);
    u32_option(t, "UNSAFE_HEADER_PARSING 1", s, 110, 1);
    u32_option(t, "AUTOLOGON_POLICY 1", s, 77, 1);
    u32_option(t, "AUTOLOGON_POLICY 3", s, 77, 3);
    u32_option(t, "MAX_HTTP_AUTOMATIC_REDIRECTS 5", s, 89, 5);
    u32_option(t, "MAX_RESPONSE_HEADER_SIZE 65536", s, 91, 65536);
    u32_option(t, "MAX_RESPONSE_DRAIN_SIZE 1000", s, 92, 1000);
    u32_option(t, "CONNECT_TIMEOUT 5000", s, 3, 5000);
    u32_option(t, "SEND_TIMEOUT 6000", s, 5, 6000);
    u32_option(t, "RECEIVE_TIMEOUT 7000", s, 6, 7000);
    u32_option(t, "RESOLVE_TIMEOUT 1000", s, 2, 1000);
    u32_option(t, "RECEIVE_RESPONSE_TIMEOUT 9000", s, 7, 9000);
    u32_option(t, "ENABLE_FEATURE 1", s, 79, 1);
    u32_option(t, "CODEPAGE 1252", s, 68, 1252);
    u32_option(t, "HTTP_VERSION", s, 59, 0x0001_0001);
    u32_option(t, "HANDLE_TYPE (read only)", s, 9, 1);
    u32_option(t, "SECURITY_FLAGS on a session", s, 31, 0x3300);
    // wrong sizes and unknown options
    {
        let mut v = 1u32;
        for (label, opt, len) in [("REDIRECT_POLICY, length 0", 88u32, 0u32), ("REDIRECT_POLICY, length 3", 88, 3), ("REDIRECT_POLICY, length 8", 88, 8), ("SECURE_PROTOCOLS, length 2", 84, 2), ("unknown option 0xFFFF", 0xFFFF, 4), ("option 0", 0, 4), ("unknown option 200", 200, 4)] {
            clear_error();
            // SAFETY: valid buffer; the sizes under test are at most 8, the buffer holds 4 (never read beyond for len > 4 by a correct call).
            let ok = unsafe {
                let mut big = [0u8; 16];
                big[..4].copy_from_slice(&v.to_le_bytes());
                WinHttpSetOption(s, opt, big.as_mut_ptr().cast(), len)
            };
            say!(t, "{label}: result {ok}, error {}", gle_if(ok));
            v += 1;
        }
        clear_error();
        // SAFETY: NULL buffer with length 0 is the case under test.
        let ok = unsafe { WinHttpSetOption(s, 88, core::ptr::null_mut(), 0) };
        say!(t, "REDIRECT_POLICY, NULL buffer length 0: result {ok}, error {}", gle_if(ok));
        clear_error();
        // SAFETY: NULL buffer with length 4 is the case under test.
        let ok = unsafe { WinHttpSetOption(s, 88, core::ptr::null_mut(), 4) };
        say!(t, "REDIRECT_POLICY, NULL buffer length 4: result {ok}, error {}", gle_if(ok));
    }
    // query with a short buffer
    {
        let mut out = [0u8; 16];
        let mut n = 2u32;
        clear_error();
        // SAFETY: valid buffers.
        let q = unsafe { WinHttpQueryOption(s, 88, out.as_mut_ptr().cast(), &mut n) };
        say!(t, "query REDIRECT_POLICY with length 2: result {q}, error {}, length now {n}", gle_if(q));
        n = 0;
        clear_error();
        // SAFETY: NULL buffer, length 0.
        let q = unsafe { WinHttpQueryOption(s, 88, core::ptr::null_mut(), &mut n) };
        say!(t, "query REDIRECT_POLICY with NULL buffer: result {q}, error {}, length now {n}", gle_if(q));
        n = 16;
        clear_error();
        // SAFETY: valid buffers.
        let q = unsafe { WinHttpQueryOption(s, 9, out.as_mut_ptr().cast(), &mut n) };
        say!(t, "query HANDLE_TYPE of the session: result {q}, length {n}, value {}", u32::from_le_bytes([out[0], out[1], out[2], out[3]]));
    }
    // invalid handles
    {
        let mut v = 1u32;
        clear_error();
        // SAFETY: invalid handle values are the case under test.
        let ok = unsafe { WinHttpSetOption(core::ptr::null_mut(), 88, (&mut v as *mut u32).cast(), 4) };
        say!(t, "NULL handle, REDIRECT_POLICY: result {ok}, error {}", gle_if(ok));
        clear_error();
        // SAFETY: as above.
        let ok = unsafe { WinHttpSetOption(0x1234usize as HANDLE, 88, (&mut v as *mut u32).cast(), 4) };
        say!(t, "bad handle, REDIRECT_POLICY: result {ok}, error {}", gle_if(ok));
    }
    // SAFETY: closing our own handle.
    unsafe { WinHttpCloseHandle(s) };
}
