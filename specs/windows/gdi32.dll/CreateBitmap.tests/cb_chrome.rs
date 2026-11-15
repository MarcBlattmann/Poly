//! Chrome's icon construction: CreateBitmap makes the 1-bit AND mask of an
//! icon, a 32-bit DIB section is the colour bitmap, and CreateIconIndirect
//! builds the icon. GetIconInfo then returns copies (other handles; their
//! sizes and bit depths are printed, and the hotspot an icon gets), our own
//! bitmaps stay valid and are deleted by us, the copies are deleted by us,
//! DestroyIcon works once. Also a monochrome icon from a double-height mask
//! only (no colour bitmap) and a cursor with a hotspot. The GDI object count
//! is compared with the one before the test.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// wingdi.h (one RGBQUAD as a u32)
#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 1],
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

const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A memory DC with a 32-bit top-down (or bottom-up) DIB section selected:
/// (dc, bitmap, pixels). Pixels are 0x00RRGGBB, row by row from the top
/// for top-down.
fn dib32(w: i32, h: i32, top_down: bool) -> (HDC, HBITMAP, *mut u32) {
    let info = BITMAPINFO {
        header: BITMAPINFOHEADER {
            size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            width: w,
            height: if top_down { -h } else { h },
            planes: 1,
            bit_count: 32,
            compression: BI_RGB,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(dc, bmp);
        (dc, bmp, bits as *mut u32)
    }
}

/// Set every pixel of a w x h 32-bit DIB to v.
fn fill_bits(bits: *mut u32, w: i32, h: i32, v: u32) {
    for i in 0..(w * h) as usize {
        // SAFETY: inside the DIB.
        unsafe { *bits.add(i) = v };
    }
}

type HICON = *mut c_void;
/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONINFO {
    is_icon: BOOL,
    x_hotspot: u32,
    y_hotspot: u32,
    mask: HBITMAP,
    color: HBITMAP,
}

const GR_GDIOBJECTS: u32 = 0;
/// The GDI object count of this process (only differences are printed).
fn gdi_count() -> u32 {
    // SAFETY: the pseudo handle of this process.
    unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcess() -> HANDLE;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetGuiResources(process: HANDLE, flags: u32) -> u32;
    fn CreateIconIndirect(info: *const ICONINFO) -> HICON;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(icon: HICON) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
}

poly_wtest::main!(test);

fn describe(t: &mut Test, what: &str, bmp: HBITMAP) {
    let mut bm = BITMAP::default();
    // SAFETY: bmp is a bitmap or NULL; bm is writable.
    let (r, ty) = unsafe { (GetObjectW(bmp, core::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut c_void), GetObjectType(bmp)) };
    say!(t, "  {what}: type {ty} read {r} {}x{} bytes/row {} planes {} bpp {}", bm.width, bm.height, bm.width_bytes, bm.planes, bm.bits_pixel);
}

fn info(t: &mut Test, what: &str, icon: HICON, ours_mask: HBITMAP, ours_color: HBITMAP) {
    let mut ii: ICONINFO = zeroed();
    // SAFETY: icon is a live icon; ii is writable; the returned bitmaps are
    // ours to delete.
    unsafe {
        let r = GetIconInfo(icon, &mut ii);
        say!(t, "{what}: GetIconInfo {r}: is_icon {} hotspot {},{} mask null {} color null {} mask is ours {} color is ours {}", ii.is_icon, ii.x_hotspot, ii.y_hotspot, ii.mask.is_null() as i32, ii.color.is_null() as i32, (ii.mask == ours_mask) as i32, (ii.color == ours_color && !ii.color.is_null()) as i32);
        describe(t, "mask copy", ii.mask);
        if !ii.color.is_null() {
            describe(t, "colour copy", ii.color);
        }
        let a = if ii.mask.is_null() { 1 } else { DeleteObject(ii.mask) };
        let b = if ii.color.is_null() { 1 } else { DeleteObject(ii.color) };
        say!(t, "  deleted the copies: {a} {b}");
    }
}

fn test(t: &mut Test) {
    // SAFETY: GDI objects of this test; the arrays have the sizes the
    // bitmaps need (16x16 at 1 bit: 32 bytes; 16x32: 64 bytes).
    unsafe {
        let before = gdi_count();
        let mut mask_bits = [0u8; 32];
        for y in 0..16 {
            mask_bits[2 * y] = 0xF0;
            mask_bits[2 * y + 1] = 0x0F;
        }
        let mask = CreateBitmap(16, 16, 1, 1, mask_bits.as_ptr() as *const c_void);
        let (dc, color, bits) = dib32(16, 16, true);
        fill_bits(bits, 16, 16, 0xFF20_4080);
        // The colour bitmap must not be selected into a DC when the icon is built.
        let dummy = CreateBitmap(1, 1, 1, 1, core::ptr::null());
        SelectObject(dc, dummy);
        DeleteDC(dc);
        describe(t, "our mask", mask);
        describe(t, "our colour bitmap", color);

        let ii = ICONINFO { is_icon: 1, x_hotspot: 0, y_hotspot: 0, mask, color };
        let icon = CreateIconIndirect(&ii);
        say!(t, "CreateIconIndirect: null {}", icon.is_null() as i32);
        info(t, "icon", icon, mask, color);
        say!(t, "our bitmaps still valid: mask type {} colour type {}", GetObjectType(mask), GetObjectType(color));
        say!(t, "DestroyIcon {} ; again {}", DestroyIcon(icon), DestroyIcon(icon));
        say!(t, "our bitmaps after DestroyIcon: mask type {} colour type {}", GetObjectType(mask), GetObjectType(color));

        // A cursor made from the same bitmaps, hotspot 3,4.
        let cur = CreateIconIndirect(&ICONINFO { is_icon: 0, x_hotspot: 3, y_hotspot: 4, mask, color });
        info(t, "cursor", cur, mask, color);
        say!(t, "DestroyIcon(cursor) {}", DestroyIcon(cur));

        // A monochrome icon: the mask has twice the height (AND part over XOR part).
        let mut tall = [0u8; 64];
        for y in 0..16 {
            tall[2 * y] = 0xFF;
            tall[2 * y + 1] = 0xFF;
            tall[32 + 2 * y] = 0x0F;
            tall[32 + 2 * y + 1] = 0xF0;
        }
        let mono = CreateBitmap(16, 32, 1, 1, tall.as_ptr() as *const c_void);
        let mi = CreateIconIndirect(&ICONINFO { is_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: mono, color: core::ptr::null_mut() });
        say!(t, "monochrome icon: null {}", mi.is_null() as i32);
        if !mi.is_null() {
            info(t, "monochrome icon", mi, mono, core::ptr::null_mut());
            say!(t, "DestroyIcon {}", DestroyIcon(mi));
        }
        say!(t, "DestroyIcon(NULL) {}", DestroyIcon(core::ptr::null_mut()));
        say!(t, "GetIconInfo(NULL) {}", GetIconInfo(core::ptr::null_mut(), &mut zeroed::<ICONINFO>()));

        for b in [mask, color, dummy, mono] {
            say!(t, "DeleteObject {}", DeleteObject(b));
        }
        say!(t, "GDI objects back to the count before: {}", (gdi_count() == before) as i32);
    }
}
