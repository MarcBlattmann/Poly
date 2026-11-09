//! Layered windows and window regions (ADR 0172): `SetLayeredWindowAttributes`,
//! `GetLayeredWindowAttributes`, `UpdateLayeredWindow`, `SetWindowRgn`,
//! `GetWindowRgn` and `GetWindowRgnBox`. The attributes belong to the kernel
//! (`poly_win32k::guic::Layered`); the regions of this process's windows are
//! kept here as rectangle lists (the system owns the region a caller passes:
//! its handle is deleted at once).

use super::{H, SYS, SpinLock, call, info, set_last_error};
use alloc::vec::Vec;
use poly_win32k::frame::Rect;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user::{self as u};

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetObjectType(h: usize) -> u32;
    fn GetObjectW(h: usize, size: i32, out: *mut u8) -> i32;
    fn GetCurrentObject(hdc: usize, kind: u32) -> usize;
    fn DeleteObject(h: usize) -> i32;
    fn BitBlt(dst: usize, x: i32, y: i32, w: i32, h: i32, src: usize, sx: i32, sy: i32, rop: u32) -> i32;
    fn PolyGdiGetRegionRects(rgn: usize, out: *mut i32, cap: u32) -> i32;
    fn PolyGdiSetRegionRects(rgn: usize, rects: *const i32, n: u32) -> i32;
}

const ERROR_GEN_FAILURE: u32 = 31;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const OBJ_DC: u32 = 3;
const OBJ_REGION: u32 = 8;
const OBJ_MEMDC: u32 = 10;
const OBJ_BITMAP: u32 = 7;
const SRCCOPY: u32 = 0x00CC_0020;

fn layered(hwnd: H, op: u8, key: u32, alpha: u8, flags: u32) -> Reply {
    call(Request::Layered { hwnd: hwnd as u32, op, key, alpha, flags })
}

fn fail(r: Reply) -> i32 {
    if let Reply::Error { code } = r {
        set_last_error(code);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn SetLayeredWindowAttributes(hwnd: H, key: u32, alpha: u8, flags: u32) -> i32 {
    if hwnd > u32::MAX as usize {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    match layered(hwnd, 0, key, alpha, flags) {
        Reply::Ok {} => 1,
        r => fail(r),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLayeredWindowAttributes(
    hwnd: H,
    key: *mut u32,
    alpha: *mut u8,
    flags: *mut u32,
) -> i32 {
    if hwnd > u32::MAX as usize {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    match layered(hwnd, 1, 0, 0, 0) {
        Reply::Value { value } => {
            // SAFETY: the caller's optional outputs.
            unsafe {
                if !key.is_null() {
                    *key = value as u32;
                }
                if !alpha.is_null() {
                    *alpha = (value >> 32) as u8;
                }
                if !flags.is_null() {
                    *flags = (value >> 40) as u32;
                }
            }
            1
        }
        // A layered window without attributes fails without an error.
        Reply::Error { code: 0 } => 0,
        r => fail(r),
    }
}

/// `UpdateLayeredWindow(hwnd, hdcDst, pptDst, psize, hdcSrc, pptSrc, crKey,
/// pblend, dwFlags)`: the rectangle and the content (`UpdateLayeredWindow.yaml`).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn UpdateLayeredWindow(
    hwnd: H,
    _dst: usize,
    ppt_dst: *const i32,
    psize: *const i32,
    src: usize,
    ppt_src: *const i32,
    _key: u32,
    pblend: *const u8,
    flags: u32,
) -> i32 {
    if hwnd > u32::MAX as usize || info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    // Not layered, or attributes already set: 87.
    match layered(hwnd, 2, 0, 0, 0) {
        Reply::Ok {} => {}
        r => return fail(r),
    }
    if flags & !0x7 != 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if flags & 2 != 0 && pblend.is_null() {
        set_last_error(ERROR_GEN_FAILURE);
        return 0;
    }
    let size = if psize.is_null() {
        None
    } else {
        // SAFETY: the caller's SIZE.
        Some(unsafe { (*psize, *psize.add(1)) })
    };
    if let Some((cx, cy)) = size {
        if cx < 0 || cy < 0 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        if cx == 0 || cy == 0 {
            set_last_error(ERROR_GEN_FAILURE);
            return 0;
        }
    }
    // The source: NULL, a screen DC, or a memory DC holding a real bitmap.
    if src != 0 {
        // SAFETY: gdi32 checks handles.
        let kind = unsafe { GetObjectType(src) };
        let usable = match kind {
            OBJ_DC => true,
            OBJ_MEMDC => {
                // SAFETY: gdi32 checks handles; a BITMAP's worth of space.
                unsafe {
                    let bmp = GetCurrentObject(src, OBJ_BITMAP);
                    let mut bm = [0u8; 32];
                    bmp != 0
                        && GetObjectW(bmp, 32, bm.as_mut_ptr()) > 0
                        && !(i32::from_le_bytes([bm[4], bm[5], bm[6], bm[7]]) == 1
                            && i32::from_le_bytes([bm[8], bm[9], bm[10], bm[11]]) == 1
                            && u16::from_le_bytes([bm[18], bm[19]]) == 1)
                }
            }
            _ => false,
        };
        if !usable {
            set_last_error(ERROR_GEN_FAILURE);
            return 0;
        }
    }
    let Some(i) = info(hwnd) else { return 0 };
    let (mut x, mut y) = (i.window.left, i.window.top);
    let (mut cx, mut cy) = (i.window.width(), i.window.height());
    let mut flag = u::SWP_NOZORDER | u::SWP_NOACTIVATE;
    if !ppt_dst.is_null() {
        // SAFETY: the caller's POINT.
        unsafe {
            x = *ppt_dst;
            y = *ppt_dst.add(1);
        }
    } else {
        flag |= u::SWP_NOMOVE;
    }
    if let Some((w, h)) = size {
        cx = w;
        cy = h;
    } else {
        flag |= u::SWP_NOSIZE;
    }
    u::set_window_pos(&SYS, hwnd as u64, 0, x, y, cx, cy, flag);
    // The content: the source's pixels into the window's surface.
    if src != 0 && i.style & poly_win32k::metrics::WS_CHILD == 0 {
        let (sx, sy) = if ppt_src.is_null() {
            (0, 0)
        } else {
            // SAFETY: the caller's POINT.
            unsafe { (*ppt_src, *ppt_src.add(1)) }
        };
        let dc = super::GetWindowDC(hwnd);
        if dc != 0 {
            // SAFETY: our gdi32.
            unsafe { BitBlt(dc, 0, 0, cx, cy, src, sx, sy, SRCCOPY) };
            super::ReleaseDC(hwnd, dc);
        }
    }
    1
}

// ----- window regions

/// The regions of this process's windows: (window, rectangles in window coordinates).
static REGIONS: SpinLock<Vec<(u32, Vec<Rect>)>> = SpinLock::new(Vec::new());

fn rects_of(rgn: usize) -> Option<Vec<Rect>> {
    // SAFETY: gdi32 checks the handle; `n` RECTs of space.
    unsafe {
        let n = PolyGdiGetRegionRects(rgn, core::ptr::null_mut(), 0);
        if n < 0 {
            return None;
        }
        let mut flat = alloc::vec![0i32; n as usize * 4];
        PolyGdiGetRegionRects(rgn, flat.as_mut_ptr(), n as u32);
        Some(flat.chunks_exact(4).map(|c| Rect::new(c[0], c[1], c[2], c[3])).collect())
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowRgn(hwnd: H, rgn: usize, redraw: i32) -> i32 {
    let Some(i) = info(hwnd) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    let mut new = None;
    if rgn != 0 {
        // SAFETY: gdi32 checks the handle.
        if unsafe { GetObjectType(rgn) } != OBJ_REGION {
            return 0;
        }
        new = rects_of(rgn);
        // The system owns the region now: the caller's handle is gone.
        // SAFETY: gdi32 checks the handle.
        unsafe { DeleteObject(rgn) };
    }
    let removing = new.is_none();
    REGIONS.with(|r| {
        r.retain(|e| e.0 != i.hwnd);
        if let Some(v) = new {
            r.push((i.hwnd, v));
        }
    });
    // A new frame: WM_WINDOWPOSCHANGING, WM_NCCALCSIZE, WM_WINDOWPOSCHANGED
    // with the flags the oracle sent (0x1001837, plus SWP_NOREDRAW).
    let mut flags = 0x0100_1837u32;
    if redraw == 0 {
        flags |= u::SWP_NOREDRAW;
    }
    if removing && redraw != 0 {
        u::FRAME_REDRAW.store(true, core::sync::atomic::Ordering::Relaxed);
    }
    u::set_window_pos(&SYS, hwnd as u64, 0, 0, 0, 0, 0, flags);
    u::FRAME_REDRAW.store(false, core::sync::atomic::Ordering::Relaxed);
    1
}

fn region_of_window(hwnd: H) -> Result<Option<Vec<Rect>>, ()> {
    let Some(i) = info(hwnd) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return Err(());
    };
    Ok(REGIONS.with(|r| r.iter().find(|e| e.0 == i.hwnd).map(|e| e.1.clone())))
}

fn kind_of(rects: &[Rect]) -> i32 {
    match rects.len() {
        0 => 1,
        1 => 2,
        _ => 3,
    }
}

fn bounds(rects: &[Rect]) -> Rect {
    let mut it = rects.iter();
    let Some(first) = it.next() else { return Rect::new(0, 0, 0, 0) };
    it.fold(*first, |a, r| {
        Rect::new(a.left.min(r.left), a.top.min(r.top), a.right.max(r.right), a.bottom.max(r.bottom))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowRgn(hwnd: H, rgn: usize) -> i32 {
    let Ok(r) = region_of_window(hwnd) else { return 0 };
    let Some(rects) = r else { return 0 };
    if rgn == 0 {
        return 0;
    }
    // SAFETY: gdi32 checks the handle.
    if unsafe { GetObjectType(rgn) } != OBJ_REGION {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    let flat: Vec<i32> = rects.iter().flat_map(|r| [r.left, r.top, r.right, r.bottom]).collect();
    // SAFETY: `rects.len()` RECTs.
    unsafe { PolyGdiSetRegionRects(rgn, flat.as_ptr(), rects.len() as u32) };
    kind_of(&rects)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowRgnBox(hwnd: H, rect: *mut i32) -> i32 {
    let Ok(r) = region_of_window(hwnd) else { return 0 };
    let Some(rects) = r else { return 0 };
    if rect.is_null() {
        return 0;
    }
    let b = if rects.is_empty() {
        // An empty region: the window's negated origin, four times.
        let o = info(hwnd).map_or((0, 0), |i| (i.window.left, i.window.top));
        Rect::new(-o.0, -o.1, -o.0, -o.1)
    } else {
        bounds(&rects)
    };
    // SAFETY: the caller's RECT.
    unsafe { super::put_rect(rect, b) };
    kind_of(&rects)
}
