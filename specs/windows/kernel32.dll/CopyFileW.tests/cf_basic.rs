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
// CopyFileW: the result, the copy's contents, the fail-if-exists flag, the errors of bad names.
poly_wtest::main!(test);

fn test(t: &mut Test) {
    let src = scratch("cf_src.bin");
    let dst = scratch("cf_dst.bin");
    let other = scratch("cf_other.bin");
    let nodir = scratch("cf_nodir\\x.bin");
    let missing = scratch("cf_missing.bin");
    let dir = scratch("cf_dir");
    nuke(&src);
    nuke(&dst);
    nuke(&other);
    // SAFETY: valid paths.
    unsafe {
        RemoveDirectoryW(dir.as_ptr());
    }

    say!(t, "make source (10000 bytes): {}", make_file(&src, 10000, 1));
    let hs = hash_file(&src);

    let (ok, e) = copy(&src, &dst, true);
    say!(t, "copy to a new name: result {ok}, last error {e}");
    say!(t, "the copy exists with the same size and contents: {}", hash_file(&dst) == hs);
    say!(t, "the source still exists unchanged: {}", hash_file(&src) == hs);

    // fail if exists
    make_file(&other, 300, 9);
    let ho = hash_file(&other);
    let (ok, e) = copy(&other, &dst, true);
    say!(t, "fail-if-exists TRUE, destination exists: result {ok}, last error {e}");
    say!(t, "the destination is unchanged: {}", hash_file(&dst) == hs);

    // overwrite
    let (ok, e) = copy(&other, &dst, false);
    say!(t, "fail-if-exists FALSE, destination exists: result {ok}, last error {e}");
    say!(t, "the destination now has the new contents and size: {}", hash_file(&dst) == ho);

    // errors
    let (ok, e) = copy(&missing, &dst, false);
    say!(t, "missing source: result {ok}, last error {e}");
    say!(t, "the destination was not touched by the failed copy: {}", hash_file(&dst) == ho);
    let (ok, e) = copy(&src, &nodir, false);
    say!(t, "destination in a directory that does not exist: result {ok}, last error {e}");
    let empty = [0u16; 4];
    let (ok, e) = copy(&empty, &dst, false);
    say!(t, "empty source name: result {ok}, last error {e}");
    let (ok, e) = copy(&src, &empty, false);
    say!(t, "empty destination name: result {ok}, last error {e}");

    // a directory
    // SAFETY: valid path.
    unsafe {
        CreateDirectoryW(dir.as_ptr(), core::ptr::null_mut());
    }
    let (ok, e) = copy(&dir, &dst, false);
    say!(t, "source is a directory: result {ok}, last error {e}");
    let (ok, e) = copy(&src, &dir, false);
    say!(t, "destination is an existing directory: result {ok}, last error {e}");
    let (ok, e) = copy(&src, &dir, true);
    say!(t, "destination is an existing directory, fail-if-exists: result {ok}, last error {e}");

    // the same file
    let (ok, e) = copy(&src, &src, false);
    say!(t, "source and destination are the same file: result {ok}, last error {e}");
    say!(t, "the source is intact after that: {}", hash_file(&src) == hs);
    // the same file written with another case
    let mut up = src;
    for c in up.iter_mut() {
        if (0x61..=0x7a).contains(c) {
            *c -= 32;
        }
    }
    let (ok, e) = copy(&src, &up, false);
    say!(t, "the same file with the name in other case: result {ok}, last error {e}");
    say!(t, "the source is intact after that: {}", hash_file(&src) == hs);

    // an empty file and a big one (more than the copy buffer)
    let z = scratch("cf_zero.bin");
    let zd = scratch("cf_zero2.bin");
    make_file(&z, 0, 0);
    let (ok, e) = copy(&z, &zd, true);
    say!(t, "empty source: result {ok}, last error {e}; the copy is empty: {}", hash_file(&zd) == Some((0xcbf2_9ce4_8422_2325, 0)));
    let big = scratch("cf_big.bin");
    let bigd = scratch("cf_big2.bin");
    make_file(&big, 3_000_000, 5);
    let (ok, e) = copy(&big, &bigd, true);
    say!(t, "3000000-byte source: result {ok}, last error {e}; same contents: {}", hash_file(&big) == hash_file(&bigd));

    // a name with spaces, dots and non-ASCII letters, and the \\?\ form
    let sp = scratch("cf name with spaces. and dots.bin");
    let (ok, e) = copy(&src, &sp, true);
    say!(t, "name with spaces and dots: result {ok}, last error {e}; same contents: {}", hash_file(&sp) == hs);
    let mut ex = [0u16; 300];
    let pre = wide("\\\\?\\");
    ex[..4].copy_from_slice(&pre[..4]);
    let tail = scratch("cf_ex.bin");
    let n = wstr_at(tail.as_ptr(), 300).len();
    ex[4..4 + n].copy_from_slice(&tail[..n]);
    let (ok, e) = copy(&src, &ex, true);
    say!(t, "destination with the \\\\?\\ prefix: result {ok}, last error {e}; same contents: {}", hash_file(&tail) == hs);
    let uni = scratch("cf_\u{e9}\u{4e2d}.bin");
    let (ok, e) = copy(&src, &uni, true);
    say!(t, "non-ASCII name: result {ok}, last error {e}; same contents: {}", hash_file(&uni) == hs);

    for p in [&src, &dst, &other, &z, &zd, &big, &bigd, &sp, &tail, &uni] {
        nuke(p);
    }
    // SAFETY: valid path.
    unsafe {
        RemoveDirectoryW(dir.as_ptr());
    }
    say!(t, "clean: {}", !exists(&src) && !exists(&dst));
}
