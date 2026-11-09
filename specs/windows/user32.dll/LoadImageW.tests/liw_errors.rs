//! LoadImageW failures and bitmaps: an unknown system id with a NULL module
//! gives NULL with ERROR_RESOURCE_TYPE_NOT_FOUND (1813) as an icon and
//! ERROR_RESOURCE_NAME_NOT_FOUND (1814) as a cursor or bitmap; with the
//! executable's own module (it has no resources) ERROR_RESOURCE_DATA_NOT_FOUND
//! (1812) for numeric ids and names; a NULL name gives 1813; an unknown image
//! type (7, 0xFFFFFFFF) gives NULL and leaves the last error unchanged;
//! LR_LOADFROMFILE with a NULL name gives ERROR_FILE_NOT_FOUND (2) and with an
//! empty name 2; the system bitmaps OBM_CLOSE, UPARROW, DNARROW, RGARROW,
//! LFARROW, REDUCE, ZOOM, RESTORE, CHECK, SIZE and BTNCORNERS load without
//! LR_SHARED as bitmaps (GDI object type 7) of fixed sizes (42x23, 17x17 x4,
//! 17x19, 20x18, 17x19, 15x15 1-bit, 17x17, 13x13) at 96 DPI; a size asked
//! for does not change them; they are not found with LR_SHARED.
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SIZE {
    cx: i32,
    cy: i32,
}

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

type HICON = *mut c_void;
/// winuser.h: LoadImage types and flags.
const IMAGE_BITMAP: u32 = 0;

const IMAGE_ICON: u32 = 1;
const IMAGE_CURSOR: u32 = 2;
const LR_LOADFROMFILE: u32 = 0x0010;
const LR_SHARED: u32 = 0x8000;
/// winuser.h: system icon ids (IDI_*) and cursor ids (OCR_*).
const IDI_APPLICATION: usize = 32512;

/// Width, height, planes, bits per pixel of a bitmap (0s for a non-bitmap).
fn ly_bm(h: HBITMAP) -> (i32, i32, u16, u16) {
    let mut b = BITMAP::default();
    // SAFETY: b is writable.
    let n = unsafe { GetObjectW(h, core::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut c_void) };
    if n == 0 { (0, 0, 0, 0) } else { (b.width, b.height, b.planes, b.bits_pixel) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn LoadImageW(inst: HINSTANCE, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> HANDLE;
    fn LoadIconW(inst: HINSTANCE, name: *const u16) -> HICON;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: system resources.
    unsafe {
        let null = core::ptr::null_mut();
        for (n, bid) in [
            ("CLOSE", 32754usize),
            ("UPARROW", 32753),
            ("DNARROW", 32752),
            ("RGARROW", 32751),
            ("LFARROW", 32750),
            ("REDUCE", 32749),
            ("ZOOM", 32748),
            ("RESTORE", 32747),
            ("CHECK", 32760),
            ("SIZE", 32766),
            ("BTNCORNERS", 32758),
        ] {
            set_error(0x1234);
            let h = LoadImageW(null, bid as *const u16, IMAGE_BITMAP, 0, 0, 0);
            if h.is_null() {
                say!(t, "OBM_{n}: NULL error {}", last_error());
            } else {
                let b = ly_bm(h as HBITMAP);
                say!(
                    t,
                    "OBM_{n}: bitmap {}x{} planes {} bits {} object type {}",
                    b.0,
                    b.1,
                    b.2,
                    b.3,
                    GetObjectType(h as HGDIOBJ)
                );
                DeleteObject(h as HGDIOBJ);
            }
        }
        let h = LoadImageW(null, 32754usize as *const u16, IMAGE_BITMAP, 20, 20, 0);
        if !h.is_null() {
            let b = ly_bm(h as HBITMAP);
            say!(t, "OBM_CLOSE asked for 20x20: bitmap {}x{}", b.0, b.1);
            DeleteObject(h as HGDIOBJ);
        }
        let h = LoadImageW(null, 32754usize as *const u16, IMAGE_BITMAP, 0, 0, LR_SHARED);
        say!(t, "OBM_CLOSE with LR_SHARED: {}", hwnd_kind(h));
        // errors
        let id = IDI_APPLICATION as *const u16;
        set_error(0x1234);
        let h = LoadImageW(null, 0x7777usize as *const u16, IMAGE_ICON, 0, 0, LR_SHARED);
        say!(t, "NULL module, id 0x7777 icon: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, 0x7777usize as *const u16, IMAGE_CURSOR, 0, 0, LR_SHARED);
        say!(t, "NULL module, id 0x7777 cursor: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, 0x7777usize as *const u16, IMAGE_BITMAP, 0, 0, 0);
        say!(t, "NULL module, id 0x7777 bitmap: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(instance(), 0x7777usize as *const u16, IMAGE_ICON, 0, 0, 0);
        say!(t, "own module, id 0x7777 icon: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(instance(), 0x7777usize as *const u16, IMAGE_BITMAP, 0, 0, 0);
        say!(t, "own module, id 0x7777 bitmap: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(instance(), wz!("NOSUCHICON").as_ptr(), IMAGE_ICON, 0, 0, 0);
        say!(t, "own module, string name icon: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, id, 7, 0, 0, 0);
        say!(t, "image type 7: {} error {:#x}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, id, 0xFFFF_FFFF, 0, 0, LR_SHARED);
        say!(t, "image type 0xFFFFFFFF: {} error {:#x}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, core::ptr::null(), IMAGE_ICON, 0, 0, 0);
        say!(t, "NULL name: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, core::ptr::null(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE);
        say!(t, "NULL name with LR_LOADFROMFILE: {} error {:#x}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, wz!("").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE);
        say!(t, "empty file name: {} error {}", hwnd_kind(h), last_error());
        // LoadIconW with the own module
        set_error(0x1234);
        let h = LoadIconW(instance(), 0x7777usize as *const u16);
        say!(t, "LoadIconW(own module, 0x7777): {} error {}", hwnd_kind(h), last_error());
        let _ = id;
    }
}
