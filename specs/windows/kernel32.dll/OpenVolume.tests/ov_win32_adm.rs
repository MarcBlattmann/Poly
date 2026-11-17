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
// Opening the volume with CreateFileW (\\.\C:): access masks, share modes, name forms; what the handle can do.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(path: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn GetFileType(h: HANDLE) -> u32;
    fn GetVolumeInformationByHandleW(h: HANDLE, name: *mut u16, nlen: u32, serial: *mut u32, maxc: *mut u32, flags: *mut u32, fs: *mut u16, fslen: u32) -> BOOL;
    fn DeviceIoControl(h: HANDLE, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32, ret: *mut u32, ov: *mut c_void) -> BOOL;
    fn GetFileSizeEx(h: HANDLE, size: *mut i64) -> BOOL;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn SetFilePointerEx(h: HANDLE, dist: i64, newpos: *mut i64, method: u32) -> BOOL;
}

poly_wtest::main!(test);

const INVALID: usize = usize::MAX;
const OPEN_EXISTING: u32 = 3;

fn open(t: &mut Test, label: &str, name: &str, access: u32, share: u32) -> HANDLE {
    let w = wide(name);
    clear_error();
    // SAFETY: valid string.
    let h = unsafe { CreateFileW(w.as_ptr(), access, share, core::ptr::null(), OPEN_EXISTING, 0, core::ptr::null_mut()) };
    let ok = h as usize != INVALID;
    say!(t, "{label}: {} err {}", ok, if ok { -2 } else { gle() });
    if ok { h } else { core::ptr::null_mut() }
}

fn test(t: &mut Test) {
    // access masks on \\.\C:
    for (name, access) in [
        ("0 (no access)", 0u32),
        ("FILE_READ_ATTRIBUTES", 0x80),
        ("SYNCHRONIZE", 0x100000),
        ("READ_CONTROL", 0x20000),
        ("FILE_READ_DATA", 1),
        ("GENERIC_READ", 0x8000_0000),
        ("GENERIC_WRITE", 0x4000_0000),
        ("GENERIC_READ|GENERIC_WRITE", 0xC000_0000),
    ] {
        let mut b = Buf::<100>::new();
        let _ = write!(b, "\\\\.\\C: access {name}, share 7");
        let h = open(t, b.as_str(), "\\\\.\\C:", access, 7);
        if !h.is_null() {
            // SAFETY: valid handle.
            unsafe { CloseHandle(h) };
        }
    }
    // share modes with the query-only open
    for sh in [0u32, 1, 2, 3] {
        let mut b = Buf::<100>::new();
        let _ = write!(b, "\\\\.\\C: access 0, share {sh}");
        let h = open(t, b.as_str(), "\\\\.\\C:", 0, sh);
        if !h.is_null() {
            // SAFETY: valid handle.
            unsafe { CloseHandle(h) };
        }
    }
    // name forms
    for name in ["\\\\.\\C:", "\\\\.\\c:", "\\\\?\\C:", "\\\\.\\C:\\", "\\\\.\\Z:", "C:", "C:\\"] {
        let mut b = Buf::<100>::new();
        let _ = write!(b, "name {name} access 0");
        let h = open(t, b.as_str(), name, 0, 7);
        if !h.is_null() {
            // SAFETY: valid handle.
            unsafe { CloseHandle(h) };
        }
    }
    // what can be done with a handle opened with no access
    let h = open(t, "query handle", "\\\\.\\C:", 0, 7);
    if !h.is_null() {
        // SAFETY: valid handle.
        let ft = unsafe { GetFileType(h) };
        say!(t, "GetFileType = {ft} (1 disk, 2 char, 3 pipe)");
        let mut fs = [0u16; 40];
        // SAFETY: valid buffers.
        let ok = unsafe { GetVolumeInformationByHandleW(h, core::ptr::null_mut(), 0, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), fs.as_mut_ptr(), 40) };
        say!(t, "GetVolumeInformationByHandleW ok={ok} fs={}", units_text(wstr_at(fs.as_ptr(), 40)).as_str());
        let mut buf = [0u8; 64];
        let mut ret = 0u32;
        clear_error();
        // SAFETY: valid buffers; IOCTL_STORAGE_GET_DEVICE_NUMBER.
        let ok = unsafe { DeviceIoControl(h, 0x002D_1080, core::ptr::null(), 0, buf.as_mut_ptr().cast(), 64, &mut ret, core::ptr::null_mut()) };
        let dt = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        say!(t, "IOCTL_STORAGE_GET_DEVICE_NUMBER ok={ok} err {} bytes {ret} device type {}", gle_if(ok), if ok != 0 { i64::from(dt) } else { -1 });
        let mut ext = [0u8; 128];
        clear_error();
        // SAFETY: valid buffers; IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS.
        let ok = unsafe { DeviceIoControl(h, 0x0056_0000, core::ptr::null(), 0, ext.as_mut_ptr().cast(), 128, &mut ret, core::ptr::null_mut()) };
        let n = u32::from_le_bytes([ext[0], ext[1], ext[2], ext[3]]);
        say!(t, "IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS ok={ok} err {} extents {}", gle_if(ok), if ok != 0 { i64::from(n) } else { -1 });
        let mut nv = [0u8; 128];
        clear_error();
        // SAFETY: valid buffers; FSCTL_GET_NTFS_VOLUME_DATA.
        let ok = unsafe { DeviceIoControl(h, 0x0009_0064, core::ptr::null(), 0, nv.as_mut_ptr().cast(), 128, &mut ret, core::ptr::null_mut()) };
        let rec = u32::from_le_bytes([nv[44], nv[45], nv[46], nv[47]]);
        say!(t, "FSCTL_GET_NTFS_VOLUME_DATA ok={ok} err {} bytes per cluster {}", gle_if(ok), if ok != 0 { i64::from(rec) } else { -1 });
        let mut size = 0i64;
        clear_error();
        // SAFETY: valid pointer.
        let ok = unsafe { GetFileSizeEx(h, &mut size) };
        say!(t, "GetFileSizeEx ok={ok} err {}", gle_if(ok));
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
    }
    // a read handle can read the boot sector
    let h = open(t, "read handle", "\\\\.\\C:", 0x8000_0000, 3);
    if !h.is_null() {
        let mut sec = [0u8; 4096];
        let mut got = 0u32;
        clear_error();
        // SAFETY: valid buffers.
        let ok = unsafe { ReadFile(h, sec.as_mut_ptr(), 512, &mut got, core::ptr::null_mut()) };
        say!(t, "ReadFile 512 bytes at offset 0: ok={ok} got={got} err {} NTFS signature: {}", gle_if(ok), &sec[3..11] == b"NTFS    ");
        clear_error();
        // SAFETY: valid buffers.
        let ok = unsafe { ReadFile(h, sec.as_mut_ptr(), 100, &mut got, core::ptr::null_mut()) };
        say!(t, "ReadFile 100 bytes (not a sector multiple): ok={ok} err {}", gle_if(ok));
        let mut nv = [0u8; 128];
        let mut ret = 0u32;
        clear_error();
        // SAFETY: valid buffers; FSCTL_GET_NTFS_VOLUME_DATA.
        let ok = unsafe { DeviceIoControl(h, 0x0009_0064, core::ptr::null(), 0, nv.as_mut_ptr().cast(), 128, &mut ret, core::ptr::null_mut()) };
        let rec = u32::from_le_bytes([nv[44], nv[45], nv[46], nv[47]]);
        say!(t, "FSCTL_GET_NTFS_VOLUME_DATA on the read handle ok={ok} err {} bytes per cluster {}", gle_if(ok), if ok != 0 { i64::from(rec) } else { -1 });
        let mut np = 0i64;
        // SAFETY: valid pointer.
        let ok = unsafe { SetFilePointerEx(h, 0, &mut np, 0) };
        say!(t, "SetFilePointerEx ok={ok}");
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
    }
}

// recorded with the elevated (full) administrator token
