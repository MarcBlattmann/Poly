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

// ---- event log helpers (advapi32; prototypes from Learn, EVENTLOGRECORD layout from winnt.h) ----
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterEventSourceA(server: *const u8, name: *const u8) -> HANDLE;
    fn RegisterEventSourceW(server: *const u16, name: *const u16) -> HANDLE;
    fn DeregisterEventSource(h: HANDLE) -> BOOL;
    fn ReportEventA(h: HANDLE, ty: u16, cat: u16, id: u32, sid: *const c_void, nstr: u16, dsz: u32, strs: *const *const u8, data: *const c_void) -> BOOL;
    fn OpenEventLogW(server: *const u16, name: *const u16) -> HANDLE;
    fn OpenEventLogA(server: *const u8, name: *const u8) -> HANDLE;
    fn CloseEventLog(h: HANDLE) -> BOOL;
    fn ReadEventLogW(h: HANDLE, flags: u32, off: u32, buf: *mut u8, n: u32, read: *mut u32, need: *mut u32) -> BOOL;
}

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
/// UTF-16 text at `o` up to the NUL, as printable text; returns (text, offset after the NUL).
fn rd_wstr(b: &[u8], mut o: usize) -> (Buf<160>, usize) {
    let mut s = Buf::<160>::new();
    while o + 1 < b.len() && rd16(b, o) != 0 {
        let c = rd16(b, o);
        if (0x20..0x7f).contains(&c) {
            let _ = write!(s, "{}", c as u8 as char);
        } else {
            let _ = write!(s, "<{c:04x}>");
        }
        o += 2;
    }
    (s, o + 2)
}

/// Prints the newest `max` records of the Application log written by `source` (newest first).
/// Returns how many were found.
fn dump_newest(t: &mut Test, source: &str, max: usize) -> usize {
    dump_in(t, "Application", source, max)
}

/// Same, reading through the log named `log` (an unknown name opens the Application log).
fn dump_in(t: &mut Test, log: &str, source: &str, max: usize) -> usize {
    let app = wide(log);
    // SAFETY: valid string.
    let lg = unsafe { OpenEventLogW(core::ptr::null(), app.as_ptr()) };
    if lg.is_null() {
        say!(t, "OpenEventLogW(Application) failed {}", gle());
        return 0;
    }
    let mut found = 0;
    let mut buf = [0u8; 32768];
    let mut rounds = 0;
    while found < max && rounds < 6 {
        rounds += 1;
        let (mut rd, mut need) = (0u32, 0u32);
        // SAFETY: valid buffers; EVENTLOG_SEQUENTIAL_READ | EVENTLOG_BACKWARDS_READ.
        let ok = unsafe { ReadEventLogW(lg, 0x1 | 0x8, 0, buf.as_mut_ptr(), 32768, &mut rd, &mut need) };
        if ok == 0 {
            break;
        }
        let mut o = 0usize;
        while o < rd as usize && found < max {
            let len = rd32(&buf, o) as usize;
            let rec = &buf[o..o + len];
            let (name, after) = rd_wstr(rec, 56);
            if name.as_str() == source {
                let (comp, _) = rd_wstr(rec, after);
                let id = rd32(rec, 20);
                let ty = rd16(rec, 24);
                let ns = rd16(rec, 26);
                let cat = rd16(rec, 28);
                let closing = rd32(rec, 32);
                let so = rd32(rec, 36) as usize;
                let sidl = rd32(rec, 40);
                let dl = rd32(rec, 48) as usize;
                let dof = rd32(rec, 52) as usize;
                let mut b = Buf::<400>::new();
                let _ = write!(b, "record: id {id:#x} type {ty} category {cat} strings {ns} user sid length {sidl} closing record {closing} data length {dl} computer name non-empty {}", !comp.as_str().is_empty());
                let mut p = so;
                for _ in 0..ns {
                    let (s, n) = rd_wstr(rec, p);
                    let _ = write!(b, " | '{}'", s.as_str());
                    p = n;
                }
                if dl > 0 {
                    let _ = write!(b, " | data");
                    for x in &rec[dof..dof + dl.min(16)] {
                        let _ = write!(b, " {x:02x}");
                    }
                }
                t.line(b.as_str());
                found += 1;
            }
            o += len;
            if len == 0 {
                break;
            }
        }
    }
    // SAFETY: handle.
    unsafe { CloseEventLog(lg) };
    found
}

poly_wtest::main!(test);

fn reg(t: &mut Test, what: &str, server: *const u8, name: *const u8) -> HANDLE {
    clear_error();
    // SAFETY: valid strings or NULL on purpose.
    let h = unsafe { RegisterEventSourceA(server, name) };
    say!(t, "RegisterEventSourceA {what}: handle {} last error {}", if h.is_null() { "NULL" } else { "set" }, if h.is_null() { gle() } else { -2 });
    h
}

fn test(t: &mut Test) {
    let h1 = reg(t, "NULL server, a new name", core::ptr::null(), b"PolySpecB4RegA\0".as_ptr());
    let h2 = reg(t, "the same name again", core::ptr::null(), b"PolySpecB4RegA\0".as_ptr());
    say!(t, "the two handles differ: {}", h1 != h2);
    let h3 = reg(t, "empty server string", b"\0".as_ptr(), b"PolySpecB4RegA\0".as_ptr());
    let h4 = reg(t, "the same name in other case", core::ptr::null(), b"polyspecb4rega\0".as_ptr());
    let h5 = reg(t, "a name with a space and a backslash", core::ptr::null(), b"PolySpecB4 Reg\\A\0".as_ptr());
    let h6 = reg(t, "a name with a non-ASCII byte (0xE9)", core::ptr::null(), b"PolySpecB4Reg\xe9\0".as_ptr());
    let long = [b'L'; 300];
    let mut ln = [0u8; 301];
    ln[..300].copy_from_slice(&long);
    let h7 = reg(t, "a 300 character name", core::ptr::null(), ln.as_ptr());
    for (i, h) in [h1, h2, h3, h4, h5, h6, h7].into_iter().enumerate() {
        if h.is_null() {
            continue;
        }
        clear_error();
        // SAFETY: handle.
        let ok = unsafe { DeregisterEventSource(h) };
        say!(t, "DeregisterEventSource of handle {i}: {ok} last error {}", gle_if(ok));
    }
    clear_error();
    // SAFETY: handle already closed.
    let ok = unsafe { DeregisterEventSource(h1) };
    say!(t, "DeregisterEventSource of a closed handle: {ok} last error {}", gle_if(ok));
    // the handle is usable for ReportEventA before it is closed and the log gets nothing for a name that is not written
    let n = dump_newest(t, "PolySpecB4RegA", 3);
    say!(t, "events written by this test for the source: {n}");
}
