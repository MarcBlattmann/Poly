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

// ---- helpers for the small process/thread queries (kernel32 prototypes from Learn; psapi.h, winnt.h constants) ----
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessMemoryCountersEx {
    cb: u32,
    page_fault_count: u32,
    peak_working_set: usize,
    working_set: usize,
    quota_peak_paged: usize,
    quota_paged: usize,
    quota_peak_nonpaged: usize,
    quota_nonpaged: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHandleCount(process: HANDLE, count: *mut u32) -> BOOL;
    fn K32GetProcessMemoryInfo(process: HANDLE, counters: *mut ProcessMemoryCountersEx, cb: u32) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn CreateEventW(sa: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut u8;
    fn VirtualFree(addr: *mut c_void, size: usize, ty: u32) -> BOOL;
    fn GetThreadLocale() -> u32;
    fn SetThreadLocale(lcid: u32) -> BOOL;
    fn GetUserDefaultLCID() -> u32;
    fn GetUserDefaultLangID() -> u16;
    fn CreateThread(sa: *const c_void, stack: usize, start: unsafe extern "system" fn(*mut c_void) -> u32, param: *mut c_void, flags: u32, id: *mut u32) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_VM_READ: u32 = 0x10;
const SYNCHRONIZE: u32 = 0x0010_0000;
const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 4;
const MIB: usize = 1024 * 1024;

fn handle_count(h: HANDLE) -> (bool, i64, u32) {
    let mut n = 0xFFFF_FFFFu32;
    clear_error();
    // SAFETY: valid out pointer.
    let ok = unsafe { GetProcessHandleCount(h, &mut n) };
    (ok != 0, gle_if(ok), n)
}

fn mem_info(h: HANDLE, cb: u32) -> (bool, i64, ProcessMemoryCountersEx) {
    let mut c = ProcessMemoryCountersEx { cb: 0, page_fault_count: 0, peak_working_set: 0, working_set: 0, quota_peak_paged: 0, quota_paged: 0, quota_peak_nonpaged: 0, quota_nonpaged: 0, pagefile_usage: 0, peak_pagefile_usage: 0, private_usage: 0 };
    c.cb = cb;
    clear_error();
    // SAFETY: the buffer is the 80-byte EX structure; `cb` is only claimed (never above 80 in the tests).
    let ok = unsafe { K32GetProcessMemoryInfo(h, &mut c, cb) };
    (ok != 0, gle_if(ok), c)
}

fn bad_handle(h: HANDLE) -> bool {
    h as isize == -1 || h.is_null()
}
// K32GetProcessMemoryInfo: the counters of the own process and how they follow an allocation.
poly_wtest::main!(test);

fn test(t: &mut Test) {
    say!(t, "size of the extended structure: {}", core::mem::size_of::<ProcessMemoryCountersEx>());
    // SAFETY: plain call.
    let me = unsafe { GetCurrentProcess() };
    let (ok, e, c0) = mem_info(me, 80);
    say!(t, "extended structure (cb 80): result {ok}, last error {e}, cb kept {}", c0.cb == 80);
    say!(t, "working set > 0: {}, peak >= working set: {}, pagefile usage > 0: {}, peak pagefile >= pagefile: {}, private usage > 0: {}", c0.working_set > 0, c0.peak_working_set >= c0.working_set, c0.pagefile_usage > 0, c0.peak_pagefile_usage >= c0.pagefile_usage, c0.private_usage > 0);
    say!(t, "page fault count > 0: {}", c0.page_fault_count > 0);
    let (ok, e, c1) = mem_info(me, 72);
    say!(t, "basic structure (cb 72): result {ok}, last error {e}, the same working set order of magnitude: {}, private usage field left alone: {}", c1.working_set > 0, c1.private_usage == 0);

    // allocate and touch 16 MiB
    // SAFETY: plain call.
    let p = unsafe { VirtualAlloc(core::ptr::null_mut(), 16 * MIB, MEM_COMMIT_RESERVE, PAGE_READWRITE) };
    say!(t, "VirtualAlloc 16 MiB: {}", !p.is_null());
    let (_, _, c2) = mem_info(me, 80);
    say!(t, "after the allocation: private usage grew by at least 16 MiB: {}, pagefile usage grew by at least 16 MiB: {}", c2.private_usage >= c0.private_usage + 16 * MIB, c2.pagefile_usage >= c0.pagefile_usage + 16 * MIB);
    if !p.is_null() {
        let mut i = 0;
        while i < 16 * MIB {
            // SAFETY: inside the committed block.
            unsafe { *p.add(i) = 1 };
            i += 4096;
        }
    }
    let (_, _, c3) = mem_info(me, 80);
    say!(t, "after touching every page: working set grew by at least 14 MiB: {}, page faults grew by at least 3000: {}, peak working set >= working set: {}", c3.working_set >= c2.working_set + 14 * MIB, c3.page_fault_count >= c2.page_fault_count + 3000, c3.peak_working_set >= c3.working_set);
    // SAFETY: block from VirtualAlloc.
    unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
    let (_, _, c4) = mem_info(me, 80);
    say!(t, "after the release: private usage fell by at least 15 MiB: {}, the peak working set did not fall: {}, peak pagefile usage did not fall: {}", c4.private_usage + 15 * MIB <= c3.private_usage, c4.peak_working_set >= c3.peak_working_set, c4.peak_pagefile_usage >= c3.peak_pagefile_usage);
}
