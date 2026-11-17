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

// ---- file helpers for the group D tests (kernel32 prototypes from Learn; constants from winbase.h/fileapi.h) ----
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct FILETIME {
    lo: u32,
    hi: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CopyFileW(src: *const u16, dst: *const u16, fail_if_exists: BOOL) -> BOOL;
    fn SetFileAttributesW(name: *const u16, attrs: u32) -> BOOL;
    fn GetFileTime(h: HANDLE, c: *mut FILETIME, a: *mut FILETIME, w: *mut FILETIME) -> BOOL;
    fn SetFileTime(h: HANDLE, c: *const FILETIME, a: *const FILETIME, w: *const FILETIME) -> BOOL;
}

const FILE_ATTRIBUTE_READONLY: u32 = 1;
const FILE_ATTRIBUTE_HIDDEN: u32 = 2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 4;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

fn attrs_of(p: &[u16]) -> u32 {
    // SAFETY: valid NUL-terminated path.
    unsafe { GetFileAttributesW(p.as_ptr()) }
}

/// FNV-1a hash and size of a file's contents (read in chunks), or None when it cannot be opened.
fn hash_file(p: &[u16]) -> Option<(u64, u64)> {
    let mut buf = [0u8; 65536];
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut total = 0u64;
    // SAFETY: valid NUL-terminated path and buffer.
    unsafe {
        let f = CreateFileW(p.as_ptr(), GENERIC_READ, 7, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if f as isize == -1 {
            return None;
        }
        loop {
            let mut got = 0u32;
            if ReadFile(f, buf.as_mut_ptr(), 65536, &mut got, core::ptr::null_mut()) == 0 || got == 0 {
                break;
            }
            for &b in &buf[..got as usize] {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            total += u64::from(got);
        }
        CloseHandle(f);
    }
    Some((h, total))
}

/// Writes `n` bytes of a pattern depending on `seed` (in 4 KiB pieces). False on failure.
fn make_file(p: &[u16], n: usize, seed: u8) -> bool {
    let mut piece = [0u8; 4096];
    for (i, b) in piece.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(31).wrapping_add(seed);
    }
    // SAFETY: valid NUL-terminated path and buffer.
    unsafe {
        let f = CreateFileW(p.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if f as isize == -1 {
            return false;
        }
        let mut left = n;
        while left > 0 {
            let k = if left > 4096 { 4096 } else { left };
            let mut got = 0u32;
            if WriteFile(f, piece.as_ptr(), k as u32, &mut got, core::ptr::null_mut()) == 0 {
                CloseHandle(f);
                return false;
            }
            left -= k;
        }
        CloseHandle(f);
    }
    true
}

/// Creation, last access and last write time of a file.
fn times_of(p: &[u16]) -> Option<[FILETIME; 3]> {
    let mut t = [FILETIME::default(); 3];
    // SAFETY: valid path and out pointers.
    unsafe {
        let f = CreateFileW(p.as_ptr(), GENERIC_READ, 7, core::ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, core::ptr::null_mut());
        if f as isize == -1 {
            return None;
        }
        let ok = GetFileTime(f, &mut t[0], &mut t[1], &mut t[2]);
        CloseHandle(f);
        if ok == 0 {
            return None;
        }
    }
    Some(t)
}

/// Sets creation, access and write time.
fn set_times(p: &[u16], c: FILETIME, a: FILETIME, w: FILETIME) -> bool {
    // SAFETY: valid path and pointers.
    unsafe {
        let f = CreateFileW(p.as_ptr(), GENERIC_WRITE, 7, core::ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, core::ptr::null_mut());
        if f as isize == -1 {
            return false;
        }
        let ok = SetFileTime(f, &c, &a, &w);
        CloseHandle(f);
        ok != 0
    }
}

fn set_attrs(p: &[u16], a: u32) -> bool {
    // SAFETY: valid NUL-terminated path.
    unsafe { SetFileAttributesW(p.as_ptr(), a) != 0 }
}

/// Deletes a file whatever its attributes (clears read-only first); ignores errors.
fn nuke(p: &[u16]) {
    // SAFETY: valid NUL-terminated path.
    unsafe {
        SetFileAttributesW(p.as_ptr(), FILE_ATTRIBUTE_NORMAL);
        DeleteFileW(p.as_ptr());
    }
}

/// CopyFileW with the result and the last error (-2 on success) as one text.
fn copy(src: &[u16], dst: &[u16], fail_if_exists: bool) -> (bool, i64) {
    clear_error();
    // SAFETY: valid NUL-terminated paths.
    let ok = unsafe { CopyFileW(src.as_ptr(), dst.as_ptr(), fail_if_exists as BOOL) };
    (ok != 0, gle_if(ok))
}

// ---- short and long path helpers (kernel32 prototypes from Learn) ----
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetShortPathNameW(src: *const u16, dst: *mut u16, len: u32) -> u32;
    fn GetLongPathNameW(src: *const u16, dst: *mut u16, len: u32) -> u32;
}

/// A path as text relative to the scratch directory's own form `base` (the part after it, if it starts with it): the machine's own
/// directory names are never printed; what is left is `<scratch>` + the rest.
fn rel_text(full: &[u16], base: &[u16]) -> Buf<400> {
    let mut b = Buf::<400>::new();
    if full.len() >= base.len() && eq_nocase(&full[..base.len()], base) {
        let _ = write!(b, "<scratch>{}", units_text(&full[base.len()..]).as_str());
    } else {
        let _ = write!(b, "(not under the scratch directory: {} units)", full.len());
    }
    b
}

/// Calls the function with a 300-unit buffer: (result, error or -2, the text).
fn call_path(f: unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32, src: &[u16], len: u32) -> (u32, i64, [u16; 300]) {
    let mut out = [0u16; 300];
    clear_error();
    // SAFETY: valid NUL-terminated source and a 300-unit buffer (len never exceeds 300).
    let n = unsafe { f(src.as_ptr(), out.as_mut_ptr(), len) };
    (n, if n == 0 { gle() } else { -2 }, out)
}

fn mk_dir(p: &[u16]) -> bool {
    // SAFETY: valid NUL-terminated path.
    unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null_mut()) != 0 }
}

fn rm_dir(p: &[u16]) {
    // SAFETY: valid NUL-terminated path.
    unsafe { RemoveDirectoryW(p.as_ptr()) };
}

/// Concatenates UTF-16 parts into a NUL-terminated buffer.
fn cat(parts: &[&[u16]]) -> [u16; 300] {
    let mut o = [0u16; 300];
    let mut n = 0;
    for p in parts {
        let l = wstr_at(p.as_ptr(), p.len()).len();
        for &c in &p[..l] {
            if n < 299 {
                o[n] = c;
                n += 1;
            }
        }
    }
    o
}

// ---- ReadDirectoryChangesW helpers (kernel32 prototypes from Learn; FILE_NOTIFY_* from winnt.h) ----
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: HANDLE,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ReadDirectoryChangesW(h: HANDLE, buf: *mut u8, len: u32, subtree: BOOL, filter: u32, bytes: *mut u32, ov: *mut Overlapped, completion: *const c_void) -> BOOL;
    fn CreateEventW(sa: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn ResetEvent(h: HANDLE) -> BOOL;
    fn GetOverlappedResult(h: HANDLE, ov: *mut Overlapped, bytes: *mut u32, wait: BOOL) -> BOOL;
    fn CancelIo(h: HANDLE) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn MoveFileW(from: *const u16, to: *const u16) -> BOOL;
    fn CreateThread(sa: *const c_void, stack: usize, start: unsafe extern "system" fn(*mut c_void) -> u32, param: *mut c_void, flags: u32, id: *mut u32) -> HANDLE;
    fn Sleep(ms: u32);
}

const FILE_LIST_DIRECTORY: u32 = 1;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const NOTIFY_FILE_NAME: u32 = 1;
const NOTIFY_DIR_NAME: u32 = 2;
const NOTIFY_ATTRIBUTES: u32 = 4;
const NOTIFY_SIZE: u32 = 8;
const NOTIFY_LAST_WRITE: u32 = 0x10;
const NOTIFY_ALL: u32 = 0x1FF;

fn open_dir(path: &[u16], overlapped: bool, access: u32, backup: bool) -> HANDLE {
    let mut flags = if backup { FILE_FLAG_BACKUP_SEMANTICS } else { 0 };
    if overlapped {
        flags |= FILE_FLAG_OVERLAPPED;
    }
    // SAFETY: valid NUL-terminated path.
    unsafe { CreateFileW(path.as_ptr(), access, 7, core::ptr::null_mut(), OPEN_EXISTING, flags, core::ptr::null_mut()) }
}

fn bad(h: HANDLE) -> bool {
    h as isize == -1 || h.is_null()
}

struct Watch {
    h: HANDLE,
    ov: Overlapped,
    buf: [u8; 4096],
}

impl Watch {
    fn new(dir: &[u16]) -> Option<Watch> {
        let h = open_dir(dir, true, FILE_LIST_DIRECTORY, true);
        if bad(h) {
            return None;
        }
        // SAFETY: plain call.
        let ev = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
        Some(Watch { h, ov: Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: ev }, buf: [0; 4096] })
    }

    /// Starts an asynchronous read of changes; returns (accepted, last error or -2).
    fn arm(&mut self, subtree: bool, filter: u32, len: u32) -> (bool, i64) {
        // SAFETY: valid event; the buffer outlives the operation (the Watch does).
        unsafe { ResetEvent(self.ov.event) };
        self.buf = [0; 4096];
        self.ov.internal = 0;
        self.ov.internal_high = 0;
        clear_error();
        let mut got = 0u32;
        // SAFETY: valid handle, buffer and OVERLAPPED that live as long as the operation.
        let ok = unsafe { ReadDirectoryChangesW(self.h, self.buf.as_mut_ptr(), len, subtree as BOOL, filter, &mut got, &mut self.ov, core::ptr::null()) };
        (ok != 0, gle_if(ok))
    }

    /// Waits for the operation (ms); true when it completed.
    fn wait(&mut self, ms: u32) -> bool {
        // SAFETY: valid event.
        unsafe { WaitForSingleObject(self.ov.event, ms) == 0 }
    }

    /// The completed operation's byte count and error (-2 on success).
    fn result(&mut self) -> (u32, i64) {
        let mut n = 0u32;
        clear_error();
        // SAFETY: valid handle and OVERLAPPED.
        let ok = unsafe { GetOverlappedResult(self.h, &mut self.ov, &mut n, 0) };
        (n, gle_if(ok))
    }

    /// Prints the records of the buffer as `action name` lines under `tag`.
    fn show(&self, t: &mut Test, tag: &str, n: u32) {
        let mut off = 0usize;
        let mut idx = 0;
        while (n as usize) >= off + 12 && off + 12 <= 4096 {
            let next = u32::from_le_bytes([self.buf[off], self.buf[off + 1], self.buf[off + 2], self.buf[off + 3]]) as usize;
            let action = u32::from_le_bytes([self.buf[off + 4], self.buf[off + 5], self.buf[off + 6], self.buf[off + 7]]);
            let nl = u32::from_le_bytes([self.buf[off + 8], self.buf[off + 9], self.buf[off + 10], self.buf[off + 11]]) as usize;
            let mut name = [0u16; 128];
            let k = (nl / 2).min(128);
            for i in 0..k {
                name[i] = u16::from_le_bytes([self.buf[off + 12 + 2 * i], self.buf[off + 13 + 2 * i]]);
            }
            say!(t, "{tag}: record {idx}: action {action}, name '{}'", units_text(&name[..k]).as_str());
            idx += 1;
            if next == 0 {
                break;
            }
            off += next;
        }
        if idx == 0 {
            say!(t, "{tag}: no records");
        }
    }

    fn cancel(&mut self) {
        // SAFETY: valid handle.
        unsafe { CancelIo(self.h) };
        // SAFETY: valid event; waits (at most 2 s) for the cancelled operation to finish.
        unsafe { WaitForSingleObject(self.ov.event, 2000) };
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: handles of this Watch.
        unsafe {
            CloseHandle(self.h);
            CloseHandle(self.ov.event);
        }
    }
}
// ReadDirectoryChangesW: errors of bad handles and arguments, buffers that are too small, the synchronous form.
poly_wtest::main!(test);

struct Mk {
    path: [u16; 300],
}

unsafe extern "system" fn maker(p: *mut c_void) -> u32 {
    // SAFETY: the parameter is the `Mk` the test passed and keeps alive until the thread ended.
    let m = unsafe { &*(p as *const Mk) };
    // SAFETY: plain call.
    unsafe { Sleep(400) };
    write_all(&m.path, b"made by the thread");
    0
}

fn test(t: &mut Test) {
    let dir = scratch("rd_dir2");
    let fa = scratch("rd_dir2\\rd_t.txt");
    let fx = scratch("rd_dir2\\rd_long_name_to_overflow.txt");
    let ff = scratch("rd_file.txt");
    nuke(&fa);
    nuke(&fx);
    nuke(&ff);
    rm_dir(&dir);
    say!(t, "make the directory: {}", mk_dir(&dir));
    let mut buf = [0u8; 4096];
    let mut got = 0u32;

    // an ordinary file handle
    write_all(&ff, b"x");
    let hf = open_dir(&ff, false, GENERIC_READ, false);
    if !bad(hf) {
        clear_error();
        // SAFETY: valid handle and buffer.
        let ok = unsafe { ReadDirectoryChangesW(hf, buf.as_mut_ptr(), 4096, 0, NOTIFY_FILE_NAME, &mut got, core::ptr::null_mut(), core::ptr::null()) };
        say!(t, "a handle of a file: result {}, last error {}", ok != 0, gle_if(ok));
        // SAFETY: handle opened above.
        unsafe { CloseHandle(hf) };
    }
    nuke(&ff);

    // a directory opened without backup semantics
    let hn = open_dir(&dir, false, FILE_LIST_DIRECTORY, false);
    say!(t, "open a directory without FILE_FLAG_BACKUP_SEMANTICS: {}, last error {}", !bad(hn), if bad(hn) { gle() } else { -2 });
    if !bad(hn) {
        // SAFETY: handle opened above.
        unsafe { CloseHandle(hn) };
    }
    // a directory handle without FILE_LIST_DIRECTORY
    let hr = open_dir(&dir, false, FILE_READ_ATTRIBUTES, true);
    if !bad(hr) {
        clear_error();
        // SAFETY: valid handle and buffer.
        let ok = unsafe { ReadDirectoryChangesW(hr, buf.as_mut_ptr(), 4096, 0, NOTIFY_FILE_NAME, &mut got, core::ptr::null_mut(), core::ptr::null()) };
        say!(t, "a directory handle without the list right: result {}, last error {}", ok != 0, gle_if(ok));
        // SAFETY: handle opened above.
        unsafe { CloseHandle(hr) };
    }
    // NULL and invalid handles
    for (name, h) in [("NULL", core::ptr::null_mut::<c_void>()), ("-1", -1isize as *mut c_void)] {
        clear_error();
        // SAFETY: invalid handles are the case under test.
        let ok = unsafe { ReadDirectoryChangesW(h, buf.as_mut_ptr(), 4096, 0, NOTIFY_FILE_NAME, &mut got, core::ptr::null_mut(), core::ptr::null()) };
        say!(t, "handle {name}: result {}, last error {}", ok != 0, gle_if(ok));
    }

    // arguments that are refused
    for (label, filter) in [("filter 0", 0u32), ("an undefined filter bit only", 0x8000_0000)] {
        if let Some(mut w) = Watch::new(&dir) {
            let (ok, e) = w.arm(false, filter, 4096);
            say!(t, "{label}: accepted {ok}, last error {e}");
            if ok {
                w.cancel();
            }
        }
    }
    // buffers that are small: the call is accepted; the completion reports an overflow
    for len in [0u32, 3, 16, 40] {
        if let Some(mut w) = Watch::new(&dir) {
            let (ok, e) = w.arm(false, NOTIFY_FILE_NAME, len);
            say!(t, "buffer length {len}: accepted {ok}, last error {e}");
            if ok {
                write_all(&fx, b"x");
                if w.wait(3000) {
                    let (n, e) = w.result();
                    say!(t, "  a change whose record is 12 + 2 * 28 = 68 bytes: completed, bytes {n}, result error {e}");
                } else {
                    say!(t, "  a change: no completion within 3 s");
                    w.cancel();
                }
                nuke(&fx);
            }
        }
    }

    // all filter bits and subtree
    if let Some(mut w) = Watch::new(&dir) {
        let (ok, _) = w.arm(true, NOTIFY_ALL, 4096);
        write_all(&fa, b"x");
        let done = w.wait(5000);
        let (n, e) = w.result();
        say!(t, "all filter bits: armed {ok}, completed {done}, result error {e}, bytes > 0 {}", n > 0);
        w.show(t, "all", n);
    }
    nuke(&fa);

    // the synchronous form: the call returns when something changed
    let hs = open_dir(&dir, false, FILE_LIST_DIRECTORY, true);
    let m = Mk { path: fa };
    let mut tid = 0u32;
    // SAFETY: the thread only reads `m`, which outlives it (we wait below).
    let th = unsafe { CreateThread(core::ptr::null(), 0, maker, &m as *const Mk as *mut c_void, 0, &mut tid) };
    clear_error();
    let mut got = 0u32;
    // SAFETY: valid handle and buffer, synchronous call.
    let ok = unsafe { ReadDirectoryChangesW(hs, buf.as_mut_ptr(), 4096, 0, NOTIFY_FILE_NAME, &mut got, core::ptr::null_mut(), core::ptr::null()) };
    say!(t, "synchronous call: result {}, last error {}, bytes {}", ok != 0, gle_if(ok), if got >= 12 { "at least one record" } else { "none" });
    if ok != 0 && got >= 12 {
        let action = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let nl = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
        let mut name = [0u16; 64];
        for i in 0..(nl / 2).min(64) {
            name[i] = u16::from_le_bytes([buf[12 + 2 * i], buf[13 + 2 * i]]);
        }
        say!(t, "its first record: action {action}, name '{}'", units_text(&name[..(nl / 2).min(64)]).as_str());
    }
    // SAFETY: handles of this test.
    unsafe {
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
        CloseHandle(hs);
    }
    nuke(&fa);
    rm_dir(&dir);
    say!(t, "clean: {}", !exists(&dir));
}
