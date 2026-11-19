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

// ---- Toolhelp helpers (tlhelp32.h: PROCESSENTRY32W, TH32CS_*; prototypes from Learn) ----
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessEntry32W {
    size: u32,
    usage: u32,
    pid: u32,
    heap_id: usize,
    module_id: u32,
    threads: u32,
    parent: u32,
    pri: i32,
    flags: u32,
    exe: [u16; 260],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Process32FirstW(snap: HANDLE, entry: *mut ProcessEntry32W) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn Process32NextW(snap: HANDLE, entry: *mut ProcessEntry32W) -> BOOL;
}

const TH32CS_SNAPPROCESS: u32 = 2;
const TH32CS_SNAPTHREAD: u32 = 4;
const TH32CS_SNAPMODULE: u32 = 8;

fn new_entry(size: u32) -> ProcessEntry32W {
    ProcessEntry32W { size, usage: 0, pid: 0, heap_id: 0, module_id: 0, threads: 0, parent: 0, pri: 0, flags: 0, exe: [0; 260] }
}

#[derive(Clone, Copy)]
struct Row {
    pid: u32,
    parent: u32,
    threads: u32,
    name: [u16; 64],
}

struct Snap {
    rows: [Row; 640],
    n: usize,
}

/// Walks a snapshot with Process32FirstW/NextW (entry size 568).
fn walk(snap: HANDLE) -> Snap {
    let mut s = Snap { rows: [Row { pid: 0, parent: 0, threads: 0, name: [0; 64] }; 640], n: 0 };
    let mut e = new_entry(core::mem::size_of::<ProcessEntry32W>() as u32);
    // SAFETY: valid snapshot handle and entry of the documented size.
    let mut ok = unsafe { Process32FirstW(snap, &mut e) };
    while ok != 0 && s.n < 640 {
        let nm = wstr_at(e.exe.as_ptr(), 260);
        let mut name = [0u16; 64];
        let k = if nm.len() > 63 { 63 } else { nm.len() };
        name[..k].copy_from_slice(&nm[..k]);
        s.rows[s.n] = Row { pid: e.pid, parent: e.parent, threads: e.threads, name };
        s.n += 1;
        // SAFETY: as above.
        ok = unsafe { Process32NextW(snap, &mut e) };
    }
    s
}

fn find(s: &Snap, pid: u32) -> Option<Row> {
    s.rows[..s.n].iter().copied().find(|r| r.pid == pid)
}

fn name_is(r: &Row, text: &str) -> bool {
    let w = wide(text);
    let n = wstr_at(w.as_ptr(), 300).len();
    let m = wstr_at(r.name.as_ptr(), 64).len();
    eq_nocase(&r.name[..m], &w[..n])
}

/// The file name part of this image's path, as UTF-16.
fn image_file_name() -> ([u16; 64], usize) {
    let img = poly_wtest::image_path();
    let mut last = 0;
    for (i, &c) in img.iter().enumerate() {
        if c == 0x5c {
            last = i + 1;
        }
    }
    let mut o = [0u16; 64];
    let tail = &img[last..];
    let k = if tail.len() > 63 { 63 } else { tail.len() };
    o[..k].copy_from_slice(&tail[..k]);
    (o, k)
}

fn name_is_image(r: &Row) -> bool {
    let (n, k) = image_file_name();
    let m = wstr_at(r.name.as_ptr(), 64).len();
    eq_nocase(&r.name[..m], &n[..k])
}
// Process32NextW: wrong sizes, a Next on a fresh snapshot, bad and closed handles.
poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain call.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    // wrong sizes
    let mut e7 = new_entry(0);
    clear_error();
    // SAFETY: valid snapshot and entry.
    let ok = unsafe { Process32NextW(snap, &mut e7) };
    say!(t, "Process32NextW with dwSize 0 on a fresh snapshot: result {}, last error {}", ok != 0, gle_if(ok));
    let mut a = new_entry(568);
    // SAFETY: valid snapshot and entry.
    unsafe { Process32FirstW(snap, &mut a) };
    let mut e8 = new_entry(100);
    clear_error();
    // SAFETY: valid snapshot and entry.
    let ok = unsafe { Process32NextW(snap, &mut e8) };
    say!(t, "Process32NextW with dwSize 100 in the middle of the list: result {}, last error {}", ok != 0, gle_if(ok));
    let mut b = new_entry(568);
    // SAFETY: valid snapshot and entry.
    unsafe { Process32NextW(snap, &mut b) };
    say!(t, "Next after First gives another entry: {}", b.pid != a.pid);

    // a fresh snapshot: Next without First
    // SAFETY: plain call.
    let snap2 = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let mut e4 = new_entry(568);
    clear_error();
    // SAFETY: valid snapshot and entry.
    let ok = unsafe { Process32NextW(snap2, &mut e4) };
    say!(t, "Process32NextW on a fresh snapshot: result {}, last error {}, pid {}", ok != 0, gle_if(ok), e4.pid);
    let mut e9 = new_entry(568);
    // SAFETY: valid snapshot and entry.
    let ok = unsafe { Process32NextW(snap2, &mut e9) };
    say!(t, "and the next one: {}, a different pid: {}", ok != 0, e9.pid != e4.pid);
    // SAFETY: handle from the call above.
    unsafe { CloseHandle(snap2) };

    for (name, h) in [("NULL", core::ptr::null_mut::<c_void>()), ("-1", -1isize as *mut c_void)] {
        let mut e = new_entry(568);
        clear_error();
        // SAFETY: invalid handles are the case under test.
        let ok = unsafe { Process32NextW(h, &mut e) };
        say!(t, "Process32NextW with handle {name}: result {}, last error {}", ok != 0, gle_if(ok));
    }
    // SAFETY: handle from the first call.
    unsafe { CloseHandle(snap) };
    let mut e = new_entry(568);
    clear_error();
    // SAFETY: a closed handle is the case under test.
    let ok = unsafe { Process32NextW(snap, &mut e) };
    say!(t, "Process32NextW on a closed handle: result {}, last error {}", ok != 0, gle_if(ok));
}
