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
// Opening the volume with NtCreateFile (\??\C:), the way the installers do before they check the disk: access masks, share modes, options.

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryVolumeInformationFile(h: HANDLE, iosb: *mut IoStatusBlock, buf: *mut c_void, len: u32, class: u32) -> i32;
}

poly_wtest::main!(test);

const FILE_OPEN: u32 = 1;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
const FILE_DIRECTORY_FILE: u32 = 1;

fn open(t: &mut Test, label: &str, name: &str, access: u32, share: u32, options: u32) -> HANDLE {
    let w = wide(name);
    let n = wstr_at(w.as_ptr(), 299).len();
    let us = UnicodeString::of(&w[..n]);
    let oa = ObjectAttributes::named(&us);
    let mut iosb = IoStatusBlock::new();
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid structures.
    let st = unsafe { NtCreateFile(&mut h, access, &oa, &mut iosb, core::ptr::null(), 0, share, FILE_OPEN, options, core::ptr::null(), 0) };
    say!(t, "{label}: status {:#010x} information {}", st as u32, if st >= 0 { iosb.information as i64 } else { -1 });
    if st >= 0 { h } else { core::ptr::null_mut() }
}

fn test(t: &mut Test) {
    let sync = FILE_SYNCHRONOUS_IO_NONALERT;
    // access 0 is not valid for NtCreateFile of a file object; the usual minimum is SYNCHRONIZE or FILE_READ_ATTRIBUTES
    for (name, access) in [
        ("0", 0u32),
        ("FILE_READ_ATTRIBUTES", 0x80),
        ("SYNCHRONIZE", 0x100000),
        ("FILE_READ_ATTRIBUTES|SYNCHRONIZE", 0x100080),
        ("READ_CONTROL|SYNCHRONIZE", 0x120000),
        ("FILE_READ_DATA|SYNCHRONIZE", 0x100001),
        ("FILE_WRITE_DATA|SYNCHRONIZE", 0x100002),
        ("GENERIC_READ|SYNCHRONIZE", 0x8010_0000),
        ("FILE_LIST_DIRECTORY|SYNCHRONIZE", 0x100001),
        ("FILE_TRAVERSE|SYNCHRONIZE", 0x100020),
        ("FILE_READ_EA|SYNCHRONIZE", 0x100008),
        ("MAXIMUM_ALLOWED", 0x0200_0000),
    ] {
        let h = open(t, alloc_label("\\??\\C: access ", name).as_str(),"\\??\\C:", access, 7, sync);
        if !h.is_null() {
            // SAFETY: valid handle.
            unsafe { NtClose(h) };
        }
    }
    // the name forms
    for name in ["\\??\\C:", "\\??\\C:\\", "\\??\\c:", "\\??\\GLOBALROOT", "\\??\\C:\\Windows", "\\??\\C:\\$Volume", "\\??\\Z:"] {
        let h = open(t, alloc_label("name ", name).as_str(), name, 0x100080, 7, sync);
        if !h.is_null() {
            // SAFETY: valid handle.
            unsafe { NtClose(h) };
        }
    }
    // the options
    let h = open(t, "\\??\\C: with FILE_DIRECTORY_FILE", "\\??\\C:", 0x100080, 7, sync | FILE_DIRECTORY_FILE);
    if !h.is_null() {
        // SAFETY: valid handle.
        unsafe { NtClose(h) };
    }
    let h = open(t, "\\??\\C: with FILE_NON_DIRECTORY_FILE", "\\??\\C:", 0x100080, 7, sync | FILE_NON_DIRECTORY_FILE);
    if !h.is_null() {
        // SAFETY: valid handle.
        unsafe { NtClose(h) };
    }
    // share modes with a read-attributes open
    for sh in [0u32, 1, 3, 7] {
        let h = open(t, alloc_label_n("\\??\\C: FILE_READ_ATTRIBUTES share ", sh).as_str(),"\\??\\C:", 0x100080, sh, sync);
        if !h.is_null() {
            // SAFETY: valid handle.
            unsafe { NtClose(h) };
        }
    }
    // device information of an opened volume (if any open was allowed)
    let h = open(t, "open for device information", "\\??\\C:", 0x100080, 7, sync);
    if !h.is_null() {
        let mut iosb = IoStatusBlock::new();
        let mut b = [0u8; 16];
        // SAFETY: valid buffers; class 4 = FileFsDeviceInformation.
        let st = unsafe { NtQueryVolumeInformationFile(h, &mut iosb, b.as_mut_ptr().cast(), 8, 4) };
        let dt = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let ch = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        say!(t, "FileFsDeviceInformation: status {:#x} device type {dt:#x} characteristics {ch:#x}", st as u32);
        // SAFETY: valid handle.
        unsafe { NtClose(h) };
    }
}

fn alloc_label(prefix: &str, name: &str) -> Buf<200> {
    let mut b = Buf::<200>::new();
    let _ = write!(b, "{prefix}{name}");
    b
}

fn alloc_label_n(prefix: &str, n: u32) -> Buf<200> {
    let mut b = Buf::<200>::new();
    let _ = write!(b, "{prefix}{n}");
    b
}

// recorded with the elevated (full) administrator token
