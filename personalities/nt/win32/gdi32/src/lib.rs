//! Our `gdi32.dll` (ADR 0128, Decision 5): GDI draws in the app's process,
//! into DIB sections, compatible bitmaps and window surfaces it maps, with
//! per-process objects (DCs, bitmaps, brushes, pens, fonts, regions) and
//! Poly's font store through `poly-font`.
//!
//! **Not implemented yet.** The first slice's functions are exported as
//! stubs until its spec sheets arrive: each sets `ERROR_CALL_NOT_IMPLEMENTED`
//! and returns the failure value of its Learn page (`poly_win32k::exports::
//! GDI32_STUBS` is the list `cargo xtask win32` checks).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_gui::not_implemented_stubs;
use poly_win32_rt as _;

/// `CLR_INVALID` (`wingdi.h`).
const CLR_INVALID: usize = 0xFFFF_FFFF;

not_implemented_stubs! {
    /// `GetStockObject(i)`: NULL on failure.
    GetStockObject(i) => 0;
    /// `CreateCompatibleDC(hdc)`: NULL on failure.
    CreateCompatibleDC(hdc) => 0;
    /// `DeleteDC(hdc)`: zero on failure.
    DeleteDC(hdc) => 0;
    /// `SelectObject(hdc, h)`: NULL on failure (an object that is not a region).
    SelectObject(hdc, h) => 0;
    /// `DeleteObject(ho)`: zero on failure.
    DeleteObject(ho) => 0;
    /// `GetObjectW(h, c, pv)`: zero on failure.
    GetObjectW(h, c, pv) => 0;
    /// `GetObjectType(h)`: zero on failure.
    GetObjectType(h) => 0;
    /// `CreateSolidBrush(color)`: NULL on failure.
    CreateSolidBrush(color) => 0;
    /// `CreatePen(iStyle, cWidth, color)`: NULL on failure.
    CreatePen(style, width, color) => 0;
    /// `CreateCompatibleBitmap(hdc, cx, cy)`: NULL on failure.
    CreateCompatibleBitmap(hdc, cx, cy) => 0;
    /// `CreateDIBSection(hdc, pbmi, usage, ppvBits, hSection, offset)`: NULL on failure.
    CreateDIBSection(hdc, pbmi, usage, bits, section, offset) => 0;
    /// `CreateRectRgn(x1, y1, x2, y2)`: NULL on failure.
    CreateRectRgn(x1, y1, x2, y2) => 0;
    /// `SelectClipRgn(hdc, hrgn)`: `ERROR` (0) on failure.
    SelectClipRgn(hdc, hrgn) => 0;
    /// `BitBlt(hdc, x, y, cx, cy, hdcSrc, x1, y1, rop)`: zero on failure.
    BitBlt(hdc, x, y, cx, cy, src, x1, y1, rop) => 0;
    /// `PatBlt(hdc, x, y, w, h, rop)`: zero on failure.
    PatBlt(hdc, x, y, w, h, rop) => 0;
    /// `Rectangle(hdc, left, top, right, bottom)`: zero on failure.
    Rectangle(hdc, left, top, right, bottom) => 0;
    /// `MoveToEx(hdc, x, y, lppt)`: zero on failure.
    MoveToEx(hdc, x, y, lppt) => 0;
    /// `LineTo(hdc, x, y)`: zero on failure.
    LineTo(hdc, x, y) => 0;
    /// `SetPixel(hdc, x, y, color)`: -1 on failure.
    SetPixel(hdc, x, y, color) => CLR_INVALID;
    /// `GetPixel(hdc, x, y)`: `CLR_INVALID` on failure.
    GetPixel(hdc, x, y) => CLR_INVALID;
    /// `SetBkColor(hdc, color)`: `CLR_INVALID` on failure.
    SetBkColor(hdc, color) => CLR_INVALID;
    /// `SetTextColor(hdc, color)`: `CLR_INVALID` on failure.
    SetTextColor(hdc, color) => CLR_INVALID;
    /// `SetBkMode(hdc, mode)`: zero on failure.
    SetBkMode(hdc, mode) => 0;
    /// `CreateFontIndirectW(lplf)`: NULL on failure.
    CreateFontIndirectW(lplf) => 0;
    /// `TextOutW(hdc, x, y, lpString, c)`: zero on failure.
    TextOutW(hdc, x, y, string, c) => 0;
    /// `ExtTextOutW(hdc, x, y, options, lprect, lpString, c, lpDx)`: zero on failure.
    ExtTextOutW(hdc, x, y, options, rect, string, c, dx) => 0;
    /// `GetTextExtentPoint32W(hdc, lpString, c, psizl)`: zero on failure.
    GetTextExtentPoint32W(hdc, string, c, size) => 0;
    /// `GetDeviceCaps(hdc, index)`: zero (the function has no failure value).
    GetDeviceCaps(hdc, index) => 0;
    /// `GdiFlush()`: zero when not every call of the batch succeeded.
    GdiFlush() => 0;
}

/// The entry point (`DllMain`): nothing to set up yet.
#[unsafe(no_mangle)]
pub extern "system" fn gdi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
