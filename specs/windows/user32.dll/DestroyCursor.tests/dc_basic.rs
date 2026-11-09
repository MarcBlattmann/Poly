//! DestroyCursor on cursors the caller owns: TRUE for a cursor from
//! CreateCursor, from CreateIconIndirect (fIcon FALSE), a copy made by
//! CopyIcon and one loaded from a .cur file with LR_LOADFROMFILE; then FALSE
//! with ERROR_INVALID_CURSOR_HANDLE (1402) when called again and GetIconInfo
//! fails on the handle; the last error is not touched on success.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
}

const GWLP_USERDATA: i32 = -21;
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

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
}

type HCURSOR = *mut c_void;
/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONINFO {
    f_icon: BOOL,
    x_hotspot: u32,
    y_hotspot: u32,
    mask: HBITMAP,
    color: HBITMAP,
}

type HICON = *mut c_void;
const IMAGE_CURSOR: u32 = 2;
const LR_LOADFROMFILE: u32 = 0x0010;
/// winbase.h / winnt.h
const CREATE_ALWAYS: u32 = 2;

const FILE_ATTRIBUTE_NORMAL_L: u32 = 0x80;
const INVALID_HANDLE_VALUE_L: usize = usize::MAX;
/// "(lo-hi)" style result text of a BOOL.
fn ly_b(v: BOOL) -> &'static str {
    if v != 0 { "TRUE" } else { "FALSE" }
}

/// Width, height, planes, bits per pixel of a bitmap (0s for a non-bitmap).
fn ly_bm(h: HBITMAP) -> (i32, i32, u16, u16) {
    let mut b = BITMAP::default();
    // SAFETY: b is writable.
    let n = unsafe { GetObjectW(h, core::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut c_void) };
    if n == 0 { (0, 0, 0, 0) } else { (b.width, b.height, b.planes, b.bits_pixel) }
}

/// One line about an icon or cursor: GetIconInfo's fIcon, hotspot, and the
/// sizes of its bitmaps; the bitmaps are deleted. Returns false when
/// GetIconInfo failed (the line then has the last error).
fn ly_info(t: &mut Test, label: &str, h: HICON) -> bool {
    let mut ii = ICONINFO {
        f_icon: 7,
        x_hotspot: 0xAAAA,
        y_hotspot: 0xBBBB,
        mask: core::ptr::null_mut(),
        color: core::ptr::null_mut(),
    };
    set_error(0x1234);
    // SAFETY: ii is writable; the bitmaps it returns are deleted.
    unsafe {
        if GetIconInfo(h, &mut ii) == 0 {
            say!(t, "{label}: GetIconInfo FALSE error {}", last_error());
            return false;
        }
        let m = ly_bm(ii.mask);
        let c = ly_bm(ii.color);
        say!(
            t,
            "{label}: fIcon {} hotspot {},{} mask {}x{} planes {} bits {} ; color {}x{} planes {} bits {}{}",
            ii.f_icon,
            ii.x_hotspot,
            ii.y_hotspot,
            m.0,
            m.1,
            m.2,
            m.3,
            c.0,
            c.1,
            c.2,
            c.3,
            if ii.color.is_null() { " (no color bitmap)" } else { "" }
        );
        DeleteObject(ii.mask);
        if !ii.color.is_null() {
            DeleteObject(ii.color);
        }
    }
    true
}

/// A 16x16 32-bit .ico (or .cur with hotspot 3,4) image file's bytes: the
/// left half opaque red, the right half transparent; returns the length.
fn ly_ico16(cursor: bool, out: &mut [u8; 1200]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8]| {
        for &x in b {
            out[n] = x;
            n += 1;
        }
    };
    put(&[0, 0, if cursor { 2 } else { 1 }, 0, 1, 0]);
    put(&[16, 16, 0, 0]);
    if cursor {
        put(&[3, 0, 4, 0]);
    } else {
        put(&[1, 0, 32, 0]);
    }
    put(&(40u32 + 1024 + 64).to_le_bytes());
    put(&22u32.to_le_bytes());
    put(&40u32.to_le_bytes());
    put(&16i32.to_le_bytes());
    put(&32i32.to_le_bytes());
    put(&[1, 0, 32, 0]);
    put(&0u32.to_le_bytes());
    put(&(1024u32 + 64).to_le_bytes());
    put(&[0; 16]);
    for _y in 0..16 {
        for x in 0..16 {
            if x < 8 {
                put(&[0, 0, 255, 255]);
            } else {
                put(&[0, 0, 0, 0]);
            }
        }
    }
    for _y in 0..16 {
        put(&[0x00, 0xFF, 0, 0]);
    }
    n
}

/// Write a file in the current directory (the scratch directory); true when written.
fn ly_write_file(name: &[u16], data: &[u8]) -> bool {
    // SAFETY: plain file creation in the scratch directory.
    unsafe {
        let f = CreateFileW(
            name.as_ptr(),
            0x4000_0000,
            0,
            core::ptr::null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL_L,
            core::ptr::null_mut(),
        );
        if f as usize == INVALID_HANDLE_VALUE_L {
            return false;
        }
        let mut w = 0u32;
        let ok = WriteFile(f, data.as_ptr() as *const c_void, data.len() as u32, &mut w, core::ptr::null_mut());
        CloseHandle(f);
        ok != 0 && w as usize == data.len()
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const c_void,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn WriteFile(file: HANDLE, buf: *const c_void, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DestroyCursor(cursor: HCURSOR) -> BOOL;
    fn LoadImageW(inst: HINSTANCE, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> HANDLE;
    fn CreateIconIndirect(info: *const ICONINFO) -> HICON;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn CreateCursor(
        inst: HINSTANCE,
        x_hot: i32,
        y_hot: i32,
        w: i32,
        h: i32,
        and_bits: *const c_void,
        xor_bits: *const c_void,
    ) -> HCURSOR;
    fn CopyIcon(icon: HICON) -> HICON;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: cursors created here are destroyed; others are system shared.
    unsafe {
        let null = core::ptr::null_mut();
        let a = CreateCursor(
            instance(),
            1,
            1,
            32,
            32,
            [0xFFu8; 128].as_ptr() as *const c_void,
            [0u8; 128].as_ptr() as *const c_void,
        );
        set_error(0x1234);
        let r = DestroyCursor(a);
        say!(t, "DestroyCursor(CreateCursor): {} error {:#x}", ly_b(r), last_error());
        set_error(0x1234);
        let r = DestroyCursor(a);
        say!(t, "again: {} error {}", ly_b(r), last_error());
        ly_info(t, "GetIconInfo(destroyed)", a as HICON);
        // a cursor file
        let mut cb = [0u8; 1200];
        let nc = ly_ico16(true, &mut cb);
        ly_write_file(wz!("ly_d.cur"), &cb[..nc]);
        let f = LoadImageW(null, wz!("ly_d.cur").as_ptr(), IMAGE_CURSOR, 0, 0, LR_LOADFROMFILE) as HCURSOR;
        set_error(0x1234);
        let r = DestroyCursor(f);
        say!(t, "DestroyCursor(cursor loaded from a file): {} error {:#x}", ly_b(r), last_error());
        DeleteFileW(wz!("ly_d.cur").as_ptr());
        // CreateIconIndirect cursor and CopyIcon
        let mb = CreateBitmap(16, 32, 1, 1, [0u8; 64].as_ptr() as *const c_void);
        let ii = ICONINFO { f_icon: 0, x_hotspot: 2, y_hotspot: 2, mask: mb, color: null };
        let ci = CreateIconIndirect(&ii);
        let cp = CopyIcon(ci);
        say!(t, "copy of a created cursor: {}", hwnd_kind(cp));
        ly_info(t, "   copy", cp);
        set_error(0x1234);
        let r = DestroyCursor(ci);
        say!(t, "DestroyCursor(CreateIconIndirect cursor): {} error {:#x}", ly_b(r), last_error());
        set_error(0x1234);
        let r = DestroyCursor(cp);
        say!(t, "DestroyCursor(copy): {} error {:#x}", ly_b(r), last_error());
        DeleteObject(mb);
    }
}
