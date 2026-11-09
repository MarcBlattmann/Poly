//! Our `gdi32.dll` (ADR 0128, Decision 5): GDI draws in the app's process.
//! The objects and the drawing are `poly-gdi`'s (one `Gdi` per process,
//! behind a lock); this is the export glue: arguments read from the
//! caller's memory, results and last errors written back. DIB sections'
//! pixels are pages from `VirtualAlloc` the caller addresses directly.
//!
//! Exports starting with `PolyGdi` are private, for our `user32.dll`
//! (window DCs, `FillRect`, `DrawTextW`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_gdi::gdi::{Gdi, Rect};
use poly_gdi::region::Region;
use poly_win32_gui::{SpinLock, set_last_error, wide};
use poly_win32_rt as _;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
}

const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
// winerror.h
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;

static GDI: SpinLock<Option<Gdi>> = SpinLock::new(None);

fn gdi<R>(f: impl FnOnce(&mut Gdi) -> R) -> R {
    GDI.with(|g| f(g.get_or_insert_with(Gdi::new)))
}

type H = usize;

unsafe fn read_rect(p: *const i32) -> Rect {
    // SAFETY: the caller passes a RECT.
    unsafe { Rect::new(*p, *p.add(1), *p.add(2), *p.add(3)) }
}

unsafe fn write_rect(p: *mut i32, r: Rect) {
    // SAFETY: the caller passes a writable RECT.
    unsafe {
        *p = r.left;
        *p.add(1) = r.top;
        *p.add(2) = r.right;
        *p.add(3) = r.bottom;
    }
}

// ----- objects

#[unsafe(no_mangle)]
pub extern "system" fn GetStockObject(i: i32) -> H {
    gdi(|g| g.stock(i as u32)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateCompatibleDC(hdc: H) -> H {
    gdi(|g| g.create_compatible_dc(hdc as u64)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn DeleteDC(hdc: H) -> i32 {
    let (ok, err) = gdi(|g| g.delete_dc(hdc as u64));
    if !ok {
        set_last_error(err);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn DeleteObject(h: H) -> i32 {
    let (ok, freed) = gdi(|g| {
        let mem = g.dib_memory(h as u64);
        let ok = g.delete_object(h as u64);
        let gone = g.object_type(h as u64) == 0;
        (ok, if gone { mem } else { None })
    });
    if let Some((addr, _)) = freed {
        // SAFETY: pages this DLL allocated for the DIB section.
        unsafe { VirtualFree(addr as *mut u8, 0, MEM_RELEASE) };
    }
    if !ok {
        set_last_error(0);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn SelectObject(hdc: H, obj: H) -> H {
    match gdi(|g| g.select_object(hdc as u64, obj as u64)) {
        Ok(h) => h as H,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetObjectType(h: H) -> u32 {
    gdi(|g| g.object_type(h as u64))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentObject(hdc: H, kind: u32) -> H {
    gdi(|g| g.current_object(hdc as u64, kind)) as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetObjectW(h: H, size: i32, buf: *mut u8) -> i32 {
    let Some(bytes) = gdi(|g| g.object_bytes(h as u64)) else { return 0 };
    if buf.is_null() {
        return bytes.len() as i32;
    }
    let n = bytes.len().min(size.max(0) as usize);
    // SAFETY: the caller's buffer holds `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n) };
    n as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateSolidBrush(color: u32) -> H {
    gdi(|g| g.create_solid_brush(color)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn CreatePen(style: i32, width: i32, color: u32) -> H {
    gdi(|g| g.create_pen(style as u32, width, color)) as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateBitmap(w: i32, h: i32, _planes: u32, bpp: u32, bits: *const u8) -> H {
    let src = if bits.is_null() || w <= 0 || h <= 0 {
        None
    } else {
        let stride = (w as usize * if bpp == 1 { 1 } else { 32 }).div_ceil(16) * 2;
        // SAFETY: the caller passes rows of the bitmap's size.
        Some(unsafe { core::slice::from_raw_parts(bits, stride * h as usize) })
    };
    gdi(|g| g.create_bitmap(w, h, bpp as u16, src)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateCompatibleBitmap(hdc: H, w: i32, h: i32) -> H {
    gdi(|g| g.create_compatible_bitmap(hdc as u64, w, h)) as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDIBSection(
    _hdc: H,
    info: *const u8,
    _usage: u32,
    bits: *mut *mut u8,
    _section: H,
    _offset: u32,
) -> H {
    let fail = |code: u32| {
        if !bits.is_null() {
            // SAFETY: the caller's out-pointer.
            unsafe { *bits = core::ptr::null_mut() };
        }
        set_last_error(code);
        0
    };
    if info.is_null() {
        return fail(87);
    }
    // The header, and the colour table for 8 bits per pixel and below.
    // SAFETY: the caller passes a BITMAPINFO.
    let mut bytes: Vec<u8> = unsafe { core::slice::from_raw_parts(info, 40) }.to_vec();
    let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let bpp = u16::from_le_bytes([bytes[14], bytes[15]]);
    let used = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]) as usize;
    if (40..=124).contains(&size) && matches!(bpp, 1 | 4 | 8) {
        let n = if used == 0 { 1 << bpp } else { used.min(1 << bpp) };
        // SAFETY: a BITMAPINFO with that many colours.
        bytes = unsafe { core::slice::from_raw_parts(info, size + 4 * n) }.to_vec();
    }
    let (w, h, bpp, colors, len) = match Gdi::check_dib(&bytes) {
        Ok(v) => v,
        Err(code) => return fail(code),
    };
    // SAFETY: fresh zeroed pages.
    let mem = unsafe { VirtualAlloc(core::ptr::null_mut(), len.max(1), MEM_COMMIT_RESERVE, PAGE_READWRITE) };
    if mem.is_null() {
        return fail(8);
    }
    let handle = gdi(|g| g.create_dib(w, h, bpp, colors, mem as u64, len));
    if !bits.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { *bits = mem };
    }
    handle as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFontIndirectW(lf: *const u8) -> H {
    if lf.is_null() {
        return 0;
    }
    // SAFETY: a LOGFONTW (92 bytes).
    let b = unsafe { core::slice::from_raw_parts(lf, 92) };
    gdi(|g| g.create_font(b)) as H
}

// ----- regions

/// A (kind, error) result: the error, if any, becomes the last error.
fn kind_or_error((kind, err): (i32, u32)) -> i32 {
    if err != 0 {
        set_last_error(err);
    }
    kind
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> H {
    match gdi(|g| g.create_rect_rgn(Rect::new(l, t, r, b))) {
        Ok(h) => h as H,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateRectRgnIndirect(rect: *const i32) -> H {
    if rect.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { read_rect(rect) };
    CreateRectRgn(r.left, r.top, r.right, r.bottom)
}

#[unsafe(no_mangle)]
pub extern "system" fn SetRectRgn(rgn: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    i32::from(gdi(|g| g.set_rect_rgn(rgn as u64, Rect::new(l, t, r, b))))
}

#[unsafe(no_mangle)]
pub extern "system" fn CombineRgn(dst: H, src1: H, src2: H, mode: i32) -> i32 {
    kind_or_error(gdi(|g| g.combine_rgn(dst as u64, src1 as u64, src2 as u64, mode)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetRgnBox(rgn: H, rect: *mut i32) -> i32 {
    let Some((kind, r)) = gdi(|g| g.rgn_box(rgn as u64)) else { return 0 };
    if !rect.is_null() {
        // SAFETY: the caller's RECT.
        unsafe { write_rect(rect, r) };
    }
    kind
}

#[unsafe(no_mangle)]
pub extern "system" fn OffsetRgn(rgn: H, dx: i32, dy: i32) -> i32 {
    gdi(|g| g.offset_rgn(rgn as u64, dx, dy))
}

#[unsafe(no_mangle)]
pub extern "system" fn EqualRgn(a: H, b: H) -> i32 {
    gdi(|g| g.equal_rgn(a as u64, b as u64))
}

#[unsafe(no_mangle)]
pub extern "system" fn PtInRegion(rgn: H, x: i32, y: i32) -> i32 {
    match gdi(|g| g.pt_in_region(rgn as u64, x, y)) {
        Ok(inside) => i32::from(inside),
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RectInRegion(rgn: H, rect: *const i32) -> i32 {
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { read_rect(rect) };
    i32::from(gdi(|g| g.rect_in_region(rgn as u64, r)).unwrap_or(false))
}

/// `GetRegionData` (Learn): the size needed for a NULL buffer, `count`
/// when it was large enough, else 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetRegionData(rgn: H, count: u32, data: *mut u8) -> u32 {
    let Some(bytes) = gdi(|g| g.region_data(rgn as u64)) else { return 0 };
    if data.is_null() {
        return bytes.len() as u32;
    }
    if (count as usize) < bytes.len() {
        return 0;
    }
    // SAFETY: the caller's buffer holds `count` bytes, at least these.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
    count
}

// ----- clipping

#[unsafe(no_mangle)]
pub extern "system" fn SelectClipRgn(hdc: H, rgn: H) -> i32 {
    kind_or_error(gdi(|g| g.select_clip_rgn(hdc as u64, rgn as u64)))
}

#[unsafe(no_mangle)]
pub extern "system" fn ExtSelectClipRgn(hdc: H, rgn: H, mode: i32) -> i32 {
    kind_or_error(gdi(|g| g.ext_select_clip_rgn(hdc as u64, rgn as u64, mode)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClipRgn(hdc: H, rgn: H) -> i32 {
    kind_or_error(gdi(|g| g.clip_rgn(hdc as u64, rgn as u64)))
}

#[unsafe(no_mangle)]
pub extern "system" fn IntersectClipRect(hdc: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    kind_or_error(gdi(|g| g.clip_rect(hdc as u64, Rect::new(l, t, r, b), false)))
}

#[unsafe(no_mangle)]
pub extern "system" fn ExcludeClipRect(hdc: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    kind_or_error(gdi(|g| g.clip_rect(hdc as u64, Rect::new(l, t, r, b), true)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetRandomRgn(hdc: H, rgn: H, which: i32) -> i32 {
    gdi(|g| g.random_rgn(hdc as u64, rgn as u64, which))
}

/// `SetViewportOrgEx` (`viewport`) / `SetWindowOrgEx`.
unsafe fn set_origin(hdc: H, viewport: bool, x: i32, y: i32, old: *mut i32) -> i32 {
    let Some((ox, oy)) = gdi(|g| g.set_origin(hdc as u64, viewport, x, y)) else { return 0 };
    if !old.is_null() {
        // SAFETY: the caller's POINT.
        unsafe {
            *old = ox;
            *old.add(1) = oy;
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetViewportOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { set_origin(hdc, true, x, y, old) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWindowOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { set_origin(hdc, false, x, y, old) }
}

// ----- attributes

macro_rules! dc_get_set {
    ($($name:ident($($a:ident: $t:ty),*) -> $r:ty = $m:ident;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(hdc: H $(, $a: $t)*) -> $r {
            gdi(|g| g.$m(hdc as u64 $(, $a)*))
        }
    )*};
}

dc_get_set! {
    SetTextColor(c: u32) -> u32 = set_text_color;
    GetTextColor() -> u32 = text_color;
    SetBkColor(c: u32) -> u32 = set_bk_color;
    GetBkColor() -> u32 = bk_color;
    SetBkMode(m: i32) -> i32 = set_bk_mode;
    GetBkMode() -> i32 = bk_mode;
    SetTextAlign(a: u32) -> u32 = set_text_align;
    GetTextAlign() -> u32 = text_align;
    SetDCBrushColor(c: u32) -> u32 = set_dc_brush_color;
    SetDCPenColor(c: u32) -> u32 = set_dc_pen_color;
    SaveDC() -> i32 = save_dc;
    GetDeviceCaps(index: i32) -> i32 = device_caps;
}

#[unsafe(no_mangle)]
pub extern "system" fn RestoreDC(hdc: H, level: i32) -> i32 {
    i32::from(gdi(|g| g.restore_dc(hdc as u64, level)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MoveToEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    match gdi(|g| g.move_to(hdc as u64, x, y)) {
        Some((ox, oy)) => {
            if !old.is_null() {
                // SAFETY: the caller's POINT.
                unsafe {
                    *old = ox;
                    *old.add(1) = oy;
                }
            }
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClipBox(hdc: H, rect: *mut i32) -> i32 {
    let (kind, r) = gdi(|g| g.clip_box(hdc as u64));
    if kind == 0 {
        set_last_error(ERROR_INVALID_HANDLE);
    } else if !rect.is_null() {
        // SAFETY: the caller's RECT.
        unsafe { write_rect(rect, r) };
    }
    kind
}

// ----- drawing

#[unsafe(no_mangle)]
pub extern "system" fn SetPixel(hdc: H, x: i32, y: i32, c: u32) -> u32 {
    gdi(|g| g.set_pixel(hdc as u64, x, y, c))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetPixelV(hdc: H, x: i32, y: i32, c: u32) -> i32 {
    i32::from(SetPixel(hdc, x, y, c) != poly_gdi::gdi::CLR_INVALID)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetPixel(hdc: H, x: i32, y: i32) -> u32 {
    gdi(|g| g.get_pixel(hdc as u64, x, y))
}

#[unsafe(no_mangle)]
pub extern "system" fn PatBlt(hdc: H, x: i32, y: i32, w: i32, h: i32, rop: u32) -> i32 {
    let ok = gdi(|g| g.pat_blt(hdc as u64, x, y, w, h, rop));
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn BitBlt(dst: H, x: i32, y: i32, w: i32, h: i32, src: H, sx: i32, sy: i32, rop: u32) -> i32 {
    let (ok, err) = gdi(|g| g.bit_blt(dst as u64, x, y, w, h, src as u64, sx, sy, rop));
    if !ok {
        set_last_error(err);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn Rectangle(hdc: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    i32::from(gdi(|g| g.rectangle(hdc as u64, l, t, r, b)))
}

#[unsafe(no_mangle)]
pub extern "system" fn LineTo(hdc: H, x: i32, y: i32) -> i32 {
    i32::from(gdi(|g| g.line_to(hdc as u64, x, y)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GdiFlush() -> i32 {
    1
}

// ----- text

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TextOutW(hdc: H, x: i32, y: i32, text: *const u16, count: i32) -> i32 {
    // SAFETY: `count` characters at `text`.
    let s = unsafe { wide(text, count.max(0)) };
    let ok = gdi(|g| g.text_out(hdc as u64, x, y, &s));
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn ExtTextOutW(
    hdc: H,
    x: i32,
    y: i32,
    options: u32,
    rect: *const i32,
    text: *const u16,
    count: u32,
    _dx: *const i32,
) -> i32 {
    const ETO_OPAQUE: u32 = 2;
    if options & ETO_OPAQUE != 0 && !rect.is_null() {
        // SAFETY: the caller's RECT.
        let r = unsafe { read_rect(rect) };
        gdi(|g| {
            let bk = g.bk_color(hdc as u64);
            g.fill_rect_color(hdc as u64, r, bk)
        });
    }
    // SAFETY: `count` characters at `text`.
    unsafe { TextOutW(hdc, x, y, text, count as i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentPoint32W(hdc: H, text: *const u16, count: i32, size: *mut i32) -> i32 {
    // SAFETY: `count` characters at `text`.
    let s = unsafe { wide(text, count.max(0)) };
    match gdi(|g| g.text_extent(hdc as u64, &s)) {
        Some((cx, cy)) => {
            // SAFETY: the caller's SIZE.
            unsafe {
                *size = cx;
                *size.add(1) = cy;
            }
            1
        }
        None => {
            set_last_error(ERROR_INVALID_HANDLE);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextMetricsW(hdc: H, tm: *mut u8) -> i32 {
    match gdi(|g| g.text_metrics(hdc as u64)) {
        Some(m) => {
            let b = m.to_bytes();
            // SAFETY: the caller's TEXTMETRICW (60 bytes).
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), tm, b.len()) };
            1
        }
        None => {
            set_last_error(ERROR_INVALID_HANDLE);
            0
        }
    }
}

// ----- for user32

/// A DC for window `hwnd` (0: the screen); `persistent` for a class or
/// own DC.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiCreateDisplayDC(hwnd: H, persistent: i32) -> H {
    gdi(|g| g.create_display_dc(hwnd as u64, persistent != 0)) as H
}

/// `ReleaseDC`'s GDI part: 1 for a display DC.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiReleaseDisplayDC(hdc: H) -> i32 {
    i32::from(gdi(|g| g.release_display_dc(hdc as u64)))
}

/// A window's own or class DC goes with the window.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiDeleteDisplayDC(hdc: H) -> i32 {
    i32::from(gdi(|g| g.delete_dc(hdc as u64).0))
}

/// `count` RECTs at `p` (none for NULL).
unsafe fn read_rects(p: *const i32, count: u32) -> Vec<Rect> {
    if p.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller passes `count` RECTs.
    (0..count as usize).map(|i| unsafe { read_rect(p.add(4 * i)) }).collect()
}

/// What window DC `hdc` can draw on (user32's `GetDC`/`GetWindowDC`):
/// its device 0,0 is at (`x`, `y`) on the screen, and it reaches the
/// union of the `n_include` RECTs at `include` less the union of the
/// `n_exclude` RECTs at `exclude`, in screen coordinates. 1 on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSetDisplayGeometry(
    hdc: H,
    x: i32,
    y: i32,
    include: *const i32,
    n_include: u32,
    exclude: *const i32,
    n_exclude: u32,
) -> i32 {
    // SAFETY: the caller's arrays of RECTs.
    let (inc, exc) = unsafe { (read_rects(include, n_include), read_rects(exclude, n_exclude)) };
    i32::from(gdi(|g| g.set_display_geometry(hdc as u64, (x, y), &inc, &exc)))
}

/// Set window DC `hdc`'s system clip to the union of the `n` RECTs at
/// `rects` (device, i.e. client or window, coordinates), as `BeginPaint`
/// limits its DC to the update region: drawing and `GetClipBox` honour it
/// on top of the visible region and any clip region, `GetClipRgn` does not
/// see it. NULL `rects` removes it; the next `GetDC`/`GetWindowDC` of the
/// DC removes it too. 1 on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSetSystemClip(hdc: H, rects: *const i32, n: u32) -> i32 {
    let clip = if rects.is_null() {
        None
    } else {
        // SAFETY: the caller's array of RECTs.
        let rs = unsafe { read_rects(rects, n) };
        Some(solid_union(&rs))
    };
    i32::from(gdi(|g| g.set_system_clip(hdc as u64, clip)))
}

/// The union of rectangles, empty and inverted ones counting as nothing.
fn solid_union(rs: &[Rect]) -> Region {
    let solid: Vec<Rect> = rs.iter().filter(|r| !r.is_empty()).copied().collect();
    Region::from_rects(&solid)
}

/// Region `hrgn`'s rectangles in canonical band order (`GetRegionData`'s)
/// as (left, top, right, bottom) quadruples into `out`, at most `cap` of
/// them: the total count (`cap` 0 asks for it), -1 when `hrgn` is not a
/// region.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiGetRegionRects(hrgn: H, out: *mut i32, cap: u32) -> i32 {
    let Some(rects) = gdi(|g| g.region(hrgn as u64).map(|r| r.rects().to_vec())) else { return -1 };
    if !out.is_null() {
        for (i, r) in rects.iter().take(cap as usize).enumerate() {
            // SAFETY: the caller's buffer holds `cap` RECTs.
            unsafe { write_rect(out.add(4 * i), *r) };
        }
    }
    rects.len() as i32
}

/// Replace region `hrgn`'s contents by the union of the `n` RECTs at
/// `rects` (empty or inverted ones count as nothing): the new kind
/// (NULLREGION 1, SIMPLEREGION 2, COMPLEXREGION 3), ERROR 0 when `hrgn` is
/// not a region.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSetRegionRects(hrgn: H, rects: *const i32, n: u32) -> i32 {
    // SAFETY: the caller's array of RECTs (or NULL: none).
    let rs = unsafe { read_rects(rects, n) };
    let rgn = solid_union(&rs);
    gdi(|g| g.set_region(hrgn as u64, rgn))
}

/// The desktop link's surface DIBs (ADR 0131): per window, a top-down
/// 32-bpp DIB section of its surface's size that its DCs draw into:
/// (hwnd, bitmap, pixels, width, height).
type SurfaceDib = (H, u64, usize, u32, u32);
static SURFACES: SpinLock<Vec<SurfaceDib>> = SpinLock::new(Vec::new());

/// Point display DC `hdc` of its window at that window's surface DIB of
/// `width` x `height` (made, or remade for a new size); 0 x 0: draw
/// nowhere. 1 on success.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiAttachSurface(hdc: H, width: u32, height: u32) -> i32 {
    let Some(hwnd) = gdi(|g| g.display_hwnd(hdc as u64)) else { return 0 };
    if width == 0 || height == 0 {
        gdi(|g| g.set_surface(hdc as u64, None));
        return 1;
    }
    let hwnd = hwnd as H;
    let current = SURFACES.with(|s| s.iter().find(|e| e.0 == hwnd).copied());
    let bitmap = match current {
        Some((_, b, _, w, h)) if (w, h) == (width, height) => b,
        old => {
            let len = width as usize * height as usize * 4;
            // SAFETY: fresh zeroed pages.
            let mem = unsafe { VirtualAlloc(core::ptr::null_mut(), len, MEM_COMMIT_RESERVE, PAGE_READWRITE) };
            if mem.is_null() {
                return 0;
            }
            // White, as the compositor shows a surface before it is drawn.
            // SAFETY: `len` bytes just allocated.
            unsafe { core::ptr::write_bytes(mem, 0xFF, len) };
            let b = gdi(|g| g.create_dib(width as i32, -(height as i32), 32, Vec::new(), mem as u64, len));
            let b = u64::from(b);
            SURFACES.with(|s| {
                s.retain(|e| e.0 != hwnd);
                s.push((hwnd, b, mem as usize, width, height));
            });
            if let Some((_, ob, _, _, _)) = old {
                // DCs still pointing at it draw nowhere until attached again.
                DeleteObject(ob as H);
            }
            b
        }
    };
    gdi(|g| g.set_surface(hdc as u64, Some(bitmap as u32)));
    1
}

/// Window `hwnd`'s surface DIB: its pixels, width and height into
/// `out[0..3]`; 0 when it has none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSurface(hwnd: H, out: *mut u64) -> i32 {
    let Some((_, _, bits, w, h)) = SURFACES.with(|s| s.iter().find(|e| e.0 == hwnd).copied()) else { return 0 };
    if out.is_null() {
        return 0;
    }
    // SAFETY: the caller passes three writable u64s.
    unsafe {
        *out = bits as u64;
        *out.add(1) = u64::from(w);
        *out.add(2) = u64::from(h);
    }
    1
}

/// `FillRect` with a brush handle, or with a colour (`brush` 0).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiFillRect(hdc: H, rect: *const i32, brush: H, color: u32) -> i32 {
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { read_rect(rect) };
    let ok = gdi(|g| {
        if brush == 0 && color == poly_gdi::gdi::CLR_INVALID {
            g.fill_rect(hdc as u64, r, 0)
        } else if brush == 0 {
            g.fill_rect_color(hdc as u64, r, color)
        } else {
            g.fill_rect(hdc as u64, r, brush as u64)
        }
    });
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

/// `DrawTextW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiDrawText(
    hdc: H,
    text: *const u16,
    count: i32,
    rect: *mut i32,
    format: u32,
) -> i32 {
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's string (count -1: NUL-terminated) and RECT.
    let (s, mut r) = unsafe { (wide(text, count), read_rect(rect)) };
    let n = gdi(|g| g.draw_text(hdc as u64, &s, &mut r, format));
    // SAFETY: as above.
    unsafe { write_rect(rect, r) };
    n
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn gdi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
