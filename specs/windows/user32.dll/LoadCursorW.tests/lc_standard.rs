//! LoadCursorW(NULL, IDC_*) for every standard cursor id of winuser.h:
//! non-NULL, the same handle on a second call, distinct from the others;
//! GetIconInfo of each (fIcon, hotspot, mask and colour bitmap sizes and
//! depths); SM_CXCURSOR / SM_CYCURSOR.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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

type HCURSOR = *mut c_void;
const IDC_ARROW: usize = 32512;
const IDC_IBEAM: usize = 32513;
const IDC_WAIT: usize = 32514;
const IDC_CROSS: usize = 32515;
const IDC_UPARROW: usize = 32516;
const IDC_SIZENWSE: usize = 32642;
const IDC_SIZENESW: usize = 32643;
const IDC_SIZEWE: usize = 32644;
const IDC_SIZENS: usize = 32645;
const IDC_SIZEALL: usize = 32646;
const IDC_NO: usize = 32648;
const IDC_HAND: usize = 32649;
const IDC_APPSTARTING: usize = 32650;
const IDC_HELP: usize = 32651;
const SM_CXCURSOR: i32 = 13;
const SM_CYCURSOR: i32 = 14;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn LoadCursorW(instance: HINSTANCE, name: *const u16) -> HCURSOR;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
}

/// winuser.h
#[repr(C)]
struct ICONINFO_ {
    icon: BOOL,
    x_hotspot: u32,
    y_hotspot: u32,
    mask: HBITMAP,
    color: HBITMAP,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetIconInfo(icon: HCURSOR, info: *mut ICONINFO_) -> BOOL;
}

const IDC_SIZE_: usize = 32640; // winuser.h, obsolete: use IDC_SIZEALL
const IDC_ICON_: usize = 32641; // winuser.h, obsolete
const IDC_PIN_: usize = 32671; // winuser.h (WINVER >= 0x0606)
const IDC_PERSON_: usize = 32672; // winuser.h (WINVER >= 0x0606)

fn bitmap_text(bmp: HBITMAP, out: &mut Buf<300>) {
    if bmp.is_null() {
        let _ = out.write_str("none");
        return;
    }
    let mut b = BITMAP::default();
    // SAFETY: b is a writable BITMAP of the size given.
    unsafe { GetObjectW(bmp, core::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut c_void) };
    let _ = write!(out, "{}x{} {}bpp", b.width, b.height, b.bits_pixel);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain queries.
    unsafe {
        say!(t, "SM_CXCURSOR {} SM_CYCURSOR {}", GetSystemMetrics(SM_CXCURSOR), GetSystemMetrics(SM_CYCURSOR));
    }
    let ids: [(usize, &str); 18] = [
        (IDC_ARROW, "IDC_ARROW"),
        (IDC_IBEAM, "IDC_IBEAM"),
        (IDC_WAIT, "IDC_WAIT"),
        (IDC_CROSS, "IDC_CROSS"),
        (IDC_UPARROW, "IDC_UPARROW"),
        (IDC_SIZE_, "IDC_SIZE"),
        (IDC_ICON_, "IDC_ICON"),
        (IDC_SIZENWSE, "IDC_SIZENWSE"),
        (IDC_SIZENESW, "IDC_SIZENESW"),
        (IDC_SIZEWE, "IDC_SIZEWE"),
        (IDC_SIZENS, "IDC_SIZENS"),
        (IDC_SIZEALL, "IDC_SIZEALL"),
        (IDC_NO, "IDC_NO"),
        (IDC_HAND, "IDC_HAND"),
        (IDC_APPSTARTING, "IDC_APPSTARTING"),
        (IDC_HELP, "IDC_HELP"),
        (IDC_PIN_, "IDC_PIN"),
        (IDC_PERSON_, "IDC_PERSON"),
    ];
    let mut handles = [core::ptr::null_mut::<c_void>(); 18];
    for (i, &(id, name)) in ids.iter().enumerate() {
        set_error(0);
        // SAFETY: MAKEINTRESOURCE ids of the system cursors.
        let (h, h2) = unsafe {
            (LoadCursorW(core::ptr::null_mut(), id as *const u16), LoadCursorW(core::ptr::null_mut(), id as *const u16))
        };
        let err = last_error();
        handles[i] = h;
        let mut same_as = "none";
        for j in 0..i {
            if handles[j] == h && !h.is_null() {
                same_as = ids[j].1;
                break;
            }
        }
        let mut info = ICONINFO_ {
            icon: -1,
            x_hotspot: 0,
            y_hotspot: 0,
            mask: core::ptr::null_mut(),
            color: core::ptr::null_mut(),
        };
        // SAFETY: info is writable; h is a cursor or NULL.
        let ok = unsafe { GetIconInfo(h, &mut info) };
        let mut m = Buf::<300>::new();
        let mut c = Buf::<300>::new();
        bitmap_text(info.mask, &mut m);
        bitmap_text(info.color, &mut c);
        say!(
            t,
            "{name} ({id}): {} error {} second call same {} equals earlier {}; GetIconInfo {} fIcon {} hotspot {},{} mask {} color {}",
            if h.is_null() { "NULL" } else { "a cursor" },
            err,
            (h == h2) as i32,
            same_as,
            ok,
            info.icon,
            info.x_hotspot,
            info.y_hotspot,
            m.as_str(),
            c.as_str()
        );
        // SAFETY: GetIconInfo's bitmaps belong to the caller.
        unsafe {
            if !info.mask.is_null() {
                DeleteObject(info.mask);
            }
            if !info.color.is_null() {
                DeleteObject(info.color);
            }
        }
    }
}
