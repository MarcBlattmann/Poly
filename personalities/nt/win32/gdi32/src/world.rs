//! World transformations and the stretching blits (Learn: `SetGraphicsMode`,
//! `SetWorldTransform`, `GetWorldTransform`, `ModifyWorldTransform`,
//! `CombineTransform`, `SetStretchBltMode`, `StretchBlt`, `StretchDIBits`,
//! `SetDIBitsToDevice`): the export glue over `poly_gdi`.

use super::{H, gdi, set_last_error};
use alloc::vec::Vec;
use poly_gdi::bitmap::{Bitmap, Storage};
use poly_gdi::gdi::Gdi;
use poly_gdi::region::Rect;
use poly_gdi::xform::Xform;

const ERROR_INVALID_PARAMETER: u32 = 87;
// wingdi.h
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const DIB_PAL_COLORS: u32 = 1;

/// The caller's `XFORM` (`None`: a null pointer).
unsafe fn read_xform(p: *const u8) -> Option<Xform> {
    if p.is_null() {
        return None;
    }
    let mut b = [0u8; 24];
    // SAFETY: the caller's XFORM (24 bytes).
    unsafe { core::ptr::copy_nonoverlapping(p, b.as_mut_ptr(), 24) };
    Some(Xform::from_bytes(&b))
}

unsafe fn write_xform(p: *mut u8, x: Xform) {
    // SAFETY: the caller's XFORM (24 bytes).
    unsafe { core::ptr::copy_nonoverlapping(x.to_bytes().as_ptr(), p, 24) };
}

#[unsafe(no_mangle)]
pub extern "system" fn SetGraphicsMode(hdc: H, mode: i32) -> i32 {
    let (old, err) = gdi(|g| g.set_graphics_mode(hdc as u64, mode));
    if old == 0 && err != 0 {
        set_last_error(err);
    }
    old
}

#[unsafe(no_mangle)]
pub extern "system" fn GetGraphicsMode(hdc: H) -> i32 {
    gdi(|g| g.graphics_mode(hdc as u64))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWorldTransform(hdc: H, xf: *const u8) -> i32 {
    // SAFETY: the caller's XFORM or NULL.
    let Some(x) = (unsafe { read_xform(xf) }) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    i32::from(gdi(|g| g.set_world_transform(hdc as u64, x)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWorldTransform(hdc: H, xf: *mut u8) -> i32 {
    if xf.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    match gdi(|g| g.world_transform(hdc as u64)) {
        Some(x) => {
            // SAFETY: the caller's XFORM.
            unsafe { write_xform(xf, x) };
            1
        }
        None => 0,
    }
}

/// `ModifyWorldTransform`: the `XFORM` is ignored (and may be NULL) for
/// `MWT_IDENTITY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ModifyWorldTransform(hdc: H, xf: *const u8, mode: u32) -> i32 {
    // SAFETY: the caller's XFORM or NULL.
    let x = unsafe { read_xform(xf) };
    let x = match (x, mode) {
        (Some(x), _) => x,
        (None, poly_gdi::xform::MWT_IDENTITY) => Xform::IDENTITY,
        (None, _) => {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    };
    i32::from(gdi(|g| g.modify_world_transform(hdc as u64, x, mode)))
}

/// `CombineTransform(out, first, second)`: the three may be the same.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CombineTransform(out: *mut u8, a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the caller's XFORMs or NULL.
    let (Some(a), Some(b)) = (unsafe { read_xform(a) }, unsafe { read_xform(b) }) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    if out.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's XFORM.
    unsafe { write_xform(out, a.then(&b)) };
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn SetStretchBltMode(hdc: H, mode: i32) -> i32 {
    gdi(|g| g.set_stretch_mode(hdc as u64, mode))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetStretchBltMode(hdc: H) -> i32 {
    gdi(|g| g.stretch_mode(hdc as u64))
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn StretchBlt(
    dst: H,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    src: H,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    rop: u32,
) -> i32 {
    let (ok, err) = gdi(|g| g.stretch_blt(dst as u64, (x, y, w, h), src as u64, (sx, sy, sw, sh), rop));
    if !ok && err != 0 {
        set_last_error(err);
    }
    i32::from(ok)
}

/// A caller's packed DIB (`BITMAPINFO` and bits) as a bitmap over the
/// bits, `rows` scan lines of it (all for `None`), with the full image's
/// height. `None` for a format Poly does not draw.
unsafe fn caller_dib(info: *const u8, bits: *const u8, usage: u32, rows: Option<u32>) -> Option<(Bitmap, i32)> {
    if info.is_null() || bits.is_null() {
        return None;
    }
    // SAFETY: the caller's BITMAPINFOHEADER (at least 40 bytes).
    let head = unsafe { core::slice::from_raw_parts(info, 40) };
    let rd32 = |o: usize| u32::from_le_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
    let size = rd32(0) as usize;
    let bpp = u16::from_le_bytes([head[14], head[15]]);
    let compression = rd32(16);
    let used = rd32(32) as usize;
    // BI_BITFIELDS with the standard masks reads like BI_RGB for 32 bits.
    if !(compression == BI_RGB || (compression == BI_BITFIELDS && bpp == 32)) || !(40..=124).contains(&size) {
        return None;
    }
    let ncolors = if bpp <= 8 { if used == 0 { 1usize << bpp } else { used.min(1 << bpp) } } else { 0 };
    let mut bytes = head.to_vec();
    bytes[16..20].copy_from_slice(&BI_RGB.to_le_bytes());
    // The colours follow the header (and the 3 masks of a 40-byte BI_BITFIELDS header).
    let table = if usage == DIB_PAL_COLORS { 2 * ncolors } else { 4 * ncolors };
    // SAFETY: the colour table the header announces.
    let colors_raw = unsafe { core::slice::from_raw_parts(info.add(size), table) };
    if usage == DIB_PAL_COLORS {
        // Indexes into the default palette.
        for c in colors_raw.chunks_exact(2) {
            let i = u32::from(u16::from_le_bytes([c[0], c[1]]));
            bytes.extend_from_slice(&poly_gdi::bitmap::Rgb::from_colorref(0x0100_0000 | i).quad().to_le_bytes());
        }
    } else {
        bytes.extend_from_slice(colors_raw);
    }
    bytes.truncate(40 + 4 * ncolors);
    bytes[0..4].copy_from_slice(&40u32.to_le_bytes());
    let (w, h, bpp, colors, len) = Gdi::check_dib(&bytes).ok()?;
    let full = h.abs();
    let stride = Bitmap::dib_stride(w, bpp);
    let lines = rows.map_or(full, |n| (n as i32).min(full));
    let len = rows.map_or(len, |_| stride * lines.max(0) as usize);
    let bmp = Bitmap {
        width: w,
        height: lines,
        bpp,
        stride,
        top_down: h < 0,
        dib: true,
        colors,
        storage: Storage::External(bits as u64, len),
    };
    Some((bmp, full))
}

/// `StretchDIBits`: the scan lines copied (the source height), 0 on
/// failure. A bottom-up DIB's source rectangle counts from its lower-left
/// corner, a top-down one's from its upper-left corner (Learn).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn StretchDIBits(
    hdc: H,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    bits: *const u8,
    info: *const u8,
    usage: u32,
    rop: u32,
) -> i32 {
    // SAFETY: the caller's BITMAPINFO and bits.
    let Some((bmp, full)) = (unsafe { caller_dib(info, bits, usage, None) }) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let (sl, sr) = if sw < 0 { (sx + sw, sx) } else { (sx, sx + sw) };
    let (st, sb) = if sh < 0 { (sy + sh, sy) } else { (sy, sy + sh) };
    let s = if bmp.top_down { Rect::new(sl, st, sr, sb) } else { Rect::new(sl, full - sb, sr, full - st) };
    let flip = ((w < 0) != (sw < 0), (h < 0) != (sh < 0));
    if !gdi(|g| g.dib_to_device(hdc as u64, (x, y, w.abs(), h.abs()), &bmp, s, flip, rop)) {
        return 0;
    }
    sh.abs()
}

/// `SetDIBitsToDevice`: `lines` scan lines from `start` of a DIB (for a
/// bottom-up one scan line 0 is the bottom row) into the DC, not
/// stretched; the source origin (`sx`, `sy`) is the image's lower-left
/// corner for a bottom-up DIB. The scan lines set, 0 on failure.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn SetDIBitsToDevice(
    hdc: H,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    sx: i32,
    sy: i32,
    start: u32,
    lines: u32,
    bits: *const u8,
    info: *const u8,
    usage: u32,
) -> i32 {
    // SAFETY: the caller's BITMAPINFO and `lines` scan lines of bits.
    let Some((bmp, full)) = (unsafe { caller_dib(info, bits, usage, Some(lines)) }) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let (w, h) = (w.min(i32::MAX as u32) as i32, h.min(i32::MAX as u32) as i32);
    // The rows given: scan lines `start..start + lines`. In the full
    // image's top-down rows they begin at `first`; the bitmap holds them
    // from its row 0.
    let first = if bmp.top_down { start as i32 } else { full - start as i32 - bmp.height };
    let top = if bmp.top_down { sy } else { full - sy - h };
    let s = Rect::new(sx, top - first, sx + w, top - first + h);
    if !gdi(|g| g.dib_to_device(hdc as u64, (x, y, w, h), &bmp, s, (false, false), 0x00CC_0020)) {
        return 0;
    }
    bmp.height
}

/// Views of file mappings that DIB sections live in (`CreateDIBSection`
/// with a section): (the bits' address, the view's base).
pub(crate) static SECTION_VIEWS: super::SpinLock<Vec<(u64, u64)>> = super::SpinLock::new(Vec::new());
