//! Mapping modes, origins, extents and the brush origin (Learn: `SetMapMode`, `SetWindowExtEx`,
//! `SetViewportExtEx`, `OffsetWindowOrgEx`, `OffsetViewportOrgEx`, `LPtoDP`, `DPtoLP`,
//! `SetBrushOrgEx`, "Mapping Modes and Translations"; `specs/windows/gdi32.dll/OffsetWindowOrgEx.yaml`),
//! and `FrameRgn` and `RoundRect` (`FrameRgn.yaml`, `RoundRect.yaml`): the export glue over
//! `poly_gdi`.

use super::{H, gdi, set_last_error};

/// Write a `POINT` (two `LONG`s) to the caller's pointer when it is not NULL.
unsafe fn put_point(p: *mut i32, v: (i32, i32)) {
    if !p.is_null() {
        // SAFETY: the caller's POINT.
        unsafe {
            p.write_unaligned(v.0);
            p.add(1).write_unaligned(v.1);
        }
    }
}

/// A `BOOL` from a result that carries the old value (written to `old`) or the error to set
/// (0: none).
unsafe fn answer(r: Result<(i32, i32), u32>, old: *mut i32) -> i32 {
    match r {
        Ok(v) => {
            // SAFETY: the caller's POINT/SIZE or NULL.
            unsafe { put_point(old, v) };
            1
        }
        Err(e) => {
            if e != 0 {
                set_last_error(e);
            }
            0
        }
    }
}

/// A getter's `BOOL`: the value to `out`, or `ERROR_INVALID_HANDLE` for a bad DC.
unsafe fn get(v: Option<(i32, i32)>, out: *mut i32) -> i32 {
    // SAFETY: the caller's POINT/SIZE or NULL.
    unsafe { answer(v.ok_or(super::ERROR_INVALID_HANDLE), out) }
}

/// `SetMapMode(hdc, iMode)`: the previous mode, 0 on failure.
#[unsafe(no_mangle)]
pub extern "system" fn SetMapMode(hdc: H, mode: i32) -> i32 {
    match gdi(|g| g.set_map_mode(hdc as u64, mode)) {
        Ok(old) => old,
        Err(e) => {
            if e != 0 {
                set_last_error(e);
            }
            0
        }
    }
}

/// `GetWindowOrgEx(hdc, lppoint)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowOrgEx(hdc: H, out: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { get(gdi(|g| g.origin(hdc as u64, false)), out) }
}

/// `GetViewportOrgEx(hdc, lppoint)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetViewportOrgEx(hdc: H, out: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { get(gdi(|g| g.origin(hdc as u64, true)), out) }
}

/// `OffsetWindowOrgEx(hdc, x, y, lppt)`: the window origin moves by (x, y); `lppt` gets the old
/// one.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OffsetWindowOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { answer(gdi(|g| g.offset_origin(hdc as u64, false, x, y)), old) }
}

/// `OffsetViewportOrgEx(hdc, x, y, lppt)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OffsetViewportOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { answer(gdi(|g| g.offset_origin(hdc as u64, true, x, y)), old) }
}

/// `GetWindowExtEx(hdc, lpsize)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowExtEx(hdc: H, out: *mut i32) -> i32 {
    // SAFETY: the caller's SIZE or NULL.
    unsafe { get(gdi(|g| g.extent(hdc as u64, false)), out) }
}

/// `GetViewportExtEx(hdc, lpsize)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetViewportExtEx(hdc: H, out: *mut i32) -> i32 {
    // SAFETY: the caller's SIZE or NULL.
    unsafe { get(gdi(|g| g.extent(hdc as u64, true)), out) }
}

/// `SetWindowExtEx(hdc, x, y, lpsz)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWindowExtEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's SIZE or NULL.
    unsafe { answer(gdi(|g| g.set_extent(hdc as u64, false, x, y)), old) }
}

/// `SetViewportExtEx(hdc, x, y, lpsz)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetViewportExtEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's SIZE or NULL.
    unsafe { answer(gdi(|g| g.set_extent(hdc as u64, true, x, y)), old) }
}

/// `LPtoDP` (`to_device`) / `DPtoLP` of the caller's `count` points.
unsafe fn convert(hdc: H, points: *mut i32, count: i32, to_device: bool) -> i32 {
    let n = usize::try_from(count).unwrap_or(0);
    let mut v = alloc::vec::Vec::with_capacity(n);
    for i in 0..n {
        // SAFETY: the caller's array of `count` POINTs.
        unsafe { v.push((points.add(2 * i).read_unaligned(), points.add(2 * i + 1).read_unaligned())) };
    }
    if !gdi(|g| g.convert_points(hdc as u64, to_device, &mut v)) {
        set_last_error(super::ERROR_INVALID_HANDLE);
        return 0;
    }
    for (i, p) in v.into_iter().enumerate() {
        // SAFETY: as above.
        unsafe { put_point(points.add(2 * i), p) };
    }
    1
}

/// `LPtoDP(hdc, lppt, c)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LPtoDP(hdc: H, points: *mut i32, count: i32) -> i32 {
    // SAFETY: the caller's points.
    unsafe { convert(hdc, points, count, true) }
}

/// `DPtoLP(hdc, lppt, c)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DPtoLP(hdc: H, points: *mut i32, count: i32) -> i32 {
    // SAFETY: the caller's points.
    unsafe { convert(hdc, points, count, false) }
}

/// `SetBrushOrgEx(hdc, x, y, lppt)`: where hatch patterns start; `lppt` gets the old origin.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetBrushOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { get(gdi(|g| g.set_brush_org(hdc as u64, x, y)), old) }
}

/// `GetBrushOrgEx(hdc, lppt)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetBrushOrgEx(hdc: H, out: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { get(gdi(|g| g.brush_org(hdc as u64)), out) }
}

/// `FrameRgn(hdc, hrgn, hbr, w, h)`.
#[unsafe(no_mangle)]
pub extern "system" fn FrameRgn(hdc: H, rgn: H, brush: H, w: i32, h: i32) -> i32 {
    let (ok, e) = gdi(|g| g.frame_rgn(hdc as u64, rgn as u64, brush as u64, w, h));
    if !ok && e != 0 {
        set_last_error(e);
    }
    i32::from(ok)
}

/// `RoundRect(hdc, left, top, right, bottom, width, height)`.
#[unsafe(no_mangle)]
pub extern "system" fn RoundRect(hdc: H, l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> i32 {
    let ok = gdi(|g| g.round_rect(hdc as u64, l, t, r, b, w, h));
    if !ok {
        set_last_error(super::ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}
