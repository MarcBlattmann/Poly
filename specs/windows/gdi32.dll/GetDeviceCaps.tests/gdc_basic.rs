//! GetDeviceCaps on a screen DC (GetDC(NULL)): the facts that hold for
//! every 32-bit display (bit depth, planes, NUMCOLORS, palette sizes,
//! TECHNOLOGY, the RASTERCAPS flags one by one, the constant counters),
//! the size values as relations to GetSystemMetrics (no absolute sizes),
//! the answers for indexes that are not defined (return value and whether
//! the last error changes), and for a NULL DC, a made-up DC handle, a
//! deleted DC and a bitmap handle passed as a DC.
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
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const BITSPIXEL: i32 = 12;
const PLANES: i32 = 14;
const LOGPIXELSX: i32 = 88;
const LOGPIXELSY: i32 = 90;
const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
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

const DC_DRIVERVERSION: i32 = 0;
const DC_TECHNOLOGY: i32 = 2;
const DC_HORZSIZE: i32 = 4;
const DC_VERTSIZE: i32 = 6;
const DC_NUMBRUSHES: i32 = 16;
const DC_NUMPENS: i32 = 18;
const DC_NUMMARKERS: i32 = 20;
const DC_NUMFONTS: i32 = 22;
const DC_NUMCOLORS: i32 = 24;
const DC_PDEVICESIZE: i32 = 26;
const DC_CLIPCAPS: i32 = 36;
const DC_RASTERCAPS: i32 = 38;
const DC_ASPECTX: i32 = 40;
const DC_ASPECTY: i32 = 42;
const DC_ASPECTXY: i32 = 44;
const DC_SIZEPALETTE: i32 = 104;
const DC_NUMRESERVED: i32 = 106;
const DC_COLORRES: i32 = 108;
const DC_PHYSICALWIDTH: i32 = 110;
const DC_SCALINGFACTORX: i32 = 114;
const DC_SCALINGFACTORY: i32 = 115;
const DC_VREFRESH: i32 = 116;
const DC_DESKTOPVERTRES: i32 = 117;
const DC_DESKTOPHORZRES: i32 = 118;
const DC_BLTALIGNMENT: i32 = 119;
const RC_BITBLT_FLAG: i32 = 0x0001;
const RC_BANDING_FLAG: i32 = 0x0002;
const RC_SCALING_FLAG: i32 = 0x0004;
const RC_BITMAP64_FLAG: i32 = 0x0008;
const RC_GDI20_OUTPUT_FLAG: i32 = 0x0010;
const RC_DI_BITMAP_FLAG: i32 = 0x0080;
const RC_PALETTE_FLAG: i32 = 0x0100;
const RC_DIBTODEV_FLAG: i32 = 0x0200;
const RC_STRETCHBLT_FLAG: i32 = 0x0800;
const RC_FLOODFILL_FLAG: i32 = 0x1000;
const RC_STRETCHDIB_FLAG: i32 = 0x2000;
const RC_OP_DX_OUTPUT_FLAG: i32 = 0x4000;
const RC_DEVBITS_FLAG: i32 = 0x8000;
const TECH_RASDISPLAY: i32 = 1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HBITMAP;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: one screen DC and one memory DC of this test; the fake handles
    // are only passed to a function that validates its handle.
    unsafe {
        let dc = GetDC(core::ptr::null_mut());
        say!(t, "screen DC: null {} type {}", dc.is_null() as i32, GetObjectType(dc));
        say!(t, "BITSPIXEL {} PLANES {} COLORRES {} NUMCOLORS {}", GetDeviceCaps(dc, BITSPIXEL), GetDeviceCaps(dc, PLANES), GetDeviceCaps(dc, DC_COLORRES), GetDeviceCaps(dc, DC_NUMCOLORS));
        say!(t, "SIZEPALETTE {} NUMRESERVED {} TECHNOLOGY is DT_RASDISPLAY {}", GetDeviceCaps(dc, DC_SIZEPALETTE), GetDeviceCaps(dc, DC_NUMRESERVED), (GetDeviceCaps(dc, DC_TECHNOLOGY) == TECH_RASDISPLAY) as i32);
        say!(t, "NUMBRUSHES {} NUMPENS {} NUMFONTS {} NUMMARKERS {}", GetDeviceCaps(dc, DC_NUMBRUSHES), GetDeviceCaps(dc, DC_NUMPENS), GetDeviceCaps(dc, DC_NUMFONTS), GetDeviceCaps(dc, DC_NUMMARKERS));
        say!(t, "CLIPCAPS {} PDEVICESIZE {} BLTALIGNMENT {} SCALINGFACTORX {} SCALINGFACTORY {}", GetDeviceCaps(dc, DC_CLIPCAPS), GetDeviceCaps(dc, DC_PDEVICESIZE), GetDeviceCaps(dc, DC_BLTALIGNMENT), GetDeviceCaps(dc, DC_SCALINGFACTORX), GetDeviceCaps(dc, DC_SCALINGFACTORY));
        say!(t, "LOGPIXELSX equals LOGPIXELSY {} ; ASPECTX equals ASPECTY {} ; ASPECTXY nonzero {}", (GetDeviceCaps(dc, LOGPIXELSX) == GetDeviceCaps(dc, LOGPIXELSY)) as i32, (GetDeviceCaps(dc, DC_ASPECTX) == GetDeviceCaps(dc, DC_ASPECTY)) as i32, (GetDeviceCaps(dc, DC_ASPECTXY) != 0) as i32);
        let rc = GetDeviceCaps(dc, DC_RASTERCAPS);
        for (name, bit) in [
            ("RC_BITBLT", RC_BITBLT_FLAG),
            ("RC_BANDING", RC_BANDING_FLAG),
            ("RC_SCALING", RC_SCALING_FLAG),
            ("RC_BITMAP64", RC_BITMAP64_FLAG),
            ("RC_GDI20_OUTPUT", RC_GDI20_OUTPUT_FLAG),
            ("RC_DI_BITMAP", RC_DI_BITMAP_FLAG),
            ("RC_PALETTE", RC_PALETTE_FLAG),
            ("RC_DIBTODEV", RC_DIBTODEV_FLAG),
            ("RC_STRETCHBLT", RC_STRETCHBLT_FLAG),
            ("RC_FLOODFILL", RC_FLOODFILL_FLAG),
            ("RC_STRETCHDIB", RC_STRETCHDIB_FLAG),
            ("RC_OP_DX_OUTPUT", RC_OP_DX_OUTPUT_FLAG),
            ("RC_DEVBITS", RC_DEVBITS_FLAG),
        ] {
            say!(t, "RASTERCAPS {name}: {}", ((rc & bit) != 0) as i32);
        }
        say!(t, "HORZRES equals SM_CXSCREEN {} ; VERTRES equals SM_CYSCREEN {}", (GetDeviceCaps(dc, HORZRES) == GetSystemMetrics(SM_CXSCREEN)) as i32, (GetDeviceCaps(dc, VERTRES) == GetSystemMetrics(SM_CYSCREEN)) as i32);
        say!(t, "DESKTOPHORZRES equals HORZRES {} ; DESKTOPVERTRES equals VERTRES {} ; PHYSICALWIDTH is zero {}", (GetDeviceCaps(dc, DC_DESKTOPHORZRES) == GetDeviceCaps(dc, HORZRES)) as i32, (GetDeviceCaps(dc, DC_DESKTOPVERTRES) == GetDeviceCaps(dc, VERTRES)) as i32, (GetDeviceCaps(dc, DC_PHYSICALWIDTH) == 0) as i32);
        say!(t, "HORZSIZE positive {} VERTSIZE positive {} VREFRESH nonnegative {}", (GetDeviceCaps(dc, DC_HORZSIZE) > 0) as i32, (GetDeviceCaps(dc, DC_VERTSIZE) > 0) as i32, (GetDeviceCaps(dc, DC_VREFRESH) >= 0) as i32);
        say!(t, "DRIVERVERSION {:#x}", GetDeviceCaps(dc, DC_DRIVERVERSION));

        for idx in [-1, 1, 3, 5, 7, 9, 13, 15, 100, 122, 123, 124, 200, 399, 400, 401, 1000, 65536, i32::MAX, i32::MIN] {
            set_error(0x5A5A);
            let v = GetDeviceCaps(dc, idx);
            let e = last_error();
            say!(t, "index {idx}: value {v} error unchanged {} (error {})", (e == 0x5A5A) as i32, if e == 0x5A5A { 0 } else { e });
        }

        let mem = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateCompatibleBitmap(dc, 4, 4);
        let fake = 0x00123456usize as HDC;
        let bad: [(&str, HDC); 4] = [("NULL", core::ptr::null_mut()), ("made-up handle", fake), ("bitmap handle", bmp), ("deleted DC", mem)];
        DeleteDC(mem);
        for (name, h) in bad {
            set_error(0x5A5A);
            let v = GetDeviceCaps(h, BITSPIXEL);
            let e = last_error();
            let w = GetDeviceCaps(h, HORZRES);
            say!(t, "{name} as DC: BITSPIXEL {v} HORZRES {w} error unchanged {} (error {})", (e == 0x5A5A) as i32, if e == 0x5A5A { 0 } else { e });
        }
        DeleteObject(bmp);
        say!(t, "ReleaseDC {}", ReleaseDC(core::ptr::null_mut(), dc));
    }
}
