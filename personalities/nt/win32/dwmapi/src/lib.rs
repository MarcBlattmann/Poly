//! Our `dwmapi.dll` (ADR 0172): the calls a browser window makes of the
//! desktop window manager. Composition is always on; the per-window
//! attributes live in window properties of the window (`user32`'s
//! `SetPropW`), so every thread of the process sees them
//! (`DwmSetWindowAttribute.yaml`, `DwmGetWindowAttribute.yaml`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_gui::set_last_error;
use poly_win32_rt as _;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsWindow(hwnd: usize) -> i32;
    fn GetDesktopWindow() -> usize;
    fn GetWindowLongW(hwnd: usize, index: i32) -> i32;
    fn GetWindowRect(hwnd: usize, rect: *mut i32) -> i32;
    fn SetPropW(hwnd: usize, key: *const u16, data: usize) -> i32;
    fn GetPropW(hwnd: usize, key: *const u16) -> usize;
    fn SendMessageW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn Sleep(ms: u32);
}

const S_OK: i32 = 0;
const E_HANDLE: i32 = 0x8007_0006u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_INSUFFICIENT: i32 = 0x8007_007Au32 as i32;
const DWM_E_NOT_PRESENTING: i32 = 0x8898_0090u32 as i32;

const WS_CHILD: u32 = 0x4000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_EX_TOOLWINDOW: u32 = 0x80;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;

const WM_NCCALCSIZE: u32 = 0x83;
const WM_NCPAINT: u32 = 0x85;
const WM_WINDOWPOSCHANGED: u32 = 0x47;
const WM_DWMNCRENDERINGCHANGED: u32 = 0x31F;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DwmIsCompositionEnabled(enabled: *mut i32) -> i32 {
    if enabled.is_null() {
        set_last_error(0);
        return E_INVALIDARG;
    }
    // SAFETY: the caller's BOOL.
    unsafe { *enabled = 1 };
    S_OK
}

#[unsafe(no_mangle)]
pub extern "system" fn DwmFlush() -> i32 {
    // The next composition pass of a 60 Hz screen.
    // SAFETY: no preconditions.
    unsafe { Sleep(8) };
    S_OK
}

/// Always fails: only a process presenting through DXGI gets timing.
#[unsafe(no_mangle)]
pub extern "system" fn DwmGetCompositionTimingInfo(_hwnd: usize, _info: *mut u8) -> i32 {
    DWM_E_NOT_PRESENTING
}

fn style(hwnd: usize) -> u32 {
    // SAFETY: a user32 call on a window handle.
    unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 }
}

fn is_window(hwnd: usize) -> bool {
    // SAFETY: a user32 call on a window handle.
    hwnd != 0 && unsafe { IsWindow(hwnd) } != 0
}

fn is_desktop(hwnd: usize) -> bool {
    // SAFETY: a user32 call without preconditions.
    hwnd == unsafe { GetDesktopWindow() }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DwmExtendFrameIntoClientArea(hwnd: usize, _margins: *const i32) -> i32 {
    if !is_window(hwnd) {
        set_last_error(1400);
        return E_INVALIDARG;
    }
    if is_desktop(hwnd) || style(hwnd) & WS_CHILD != 0 {
        return E_INVALIDARG;
    }
    S_OK
}

// ----- window attributes, kept as properties of the window

fn key(attr: u32) -> [u16; 12] {
    // "PolyDwm" and two decimal digits.
    let mut k = [0u16; 12];
    for (i, c) in "PolyDwm".encode_utf16().enumerate() {
        k[i] = c;
    }
    k[7] = 0x30 + (attr / 10 % 10) as u16;
    k[8] = 0x30 + (attr % 10) as u16;
    k
}

/// The stored value of an attribute of the window, if any was set.
fn stored(hwnd: usize, attr: u32) -> Option<u32> {
    // SAFETY: a NUL-terminated key.
    let v = unsafe { GetPropW(hwnd, key(attr).as_ptr()) };
    (v != 0).then(|| (v - 1) as u32)
}

fn store(hwnd: usize, attr: u32, value: u32) {
    // SAFETY: a NUL-terminated key.
    unsafe { SetPropW(hwnd, key(attr).as_ptr(), value as usize + 1) };
}

const NCRENDERING_ENABLED: u32 = 1;
const NCRENDERING_POLICY: u32 = 2;
const CAPTION_BUTTON_BOUNDS: u32 = 5;
const EXTENDED_FRAME_BOUNDS: u32 = 9;
const CLOAK: u32 = 13;
const CLOAKED: u32 = 14;
const PASSIVE_UPDATE_MODE: u32 = 16;
const USE_IMMERSIVE_DARK_MODE: u32 = 20;
const WINDOW_CORNER_PREFERENCE: u32 = 33;
const BORDER_COLOR: u32 = 34;
const CAPTION_COLOR: u32 = 35;
const TEXT_COLOR: u32 = 36;
const VISIBLE_FRAME_BORDER_THICKNESS: u32 = 37;
const SYSTEMBACKDROP_TYPE: u32 = 38;

/// Attribute numbers that exist at all.
fn exists(attr: u32) -> bool {
    (1..=40).contains(&attr)
}

fn write_only(attr: u32) -> bool {
    matches!(attr, 2 | 3 | 4 | 6 | 7 | 10 | 11 | 12 | 13 | 17 | 34 | 35 | 36 | 39)
}

fn round_trip(attr: u32) -> bool {
    matches!(attr, 16 | 20 | 33 | 38)
}

/// Whether non-client rendering is in effect: the policy, else the style.
fn nc_enabled(hwnd: usize) -> bool {
    match stored(hwnd, NCRENDERING_POLICY) {
        Some(1) => false,
        Some(2) => true,
        Some(0) | None => style(hwnd) & WS_CAPTION == WS_CAPTION,
        Some(_) => false,
    }
}

#[repr(C)]
struct WindowPos {
    hwnd: usize,
    after: usize,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
}

#[repr(C)]
struct NcCalcSize {
    rgrc: [[i32; 4]; 3],
    pos: *mut WindowPos,
}

/// The message cascade of a change of the non-client rendering state
/// (`DwmSetWindowAttribute.yaml` `ncrendering_policy`).
fn nc_changed(hwnd: usize, enabled: bool) {
    let mut r = [0i32; 4];
    // SAFETY: a RECT out.
    unsafe { GetWindowRect(hwnd, r.as_mut_ptr()) };
    let pos = |flags| WindowPos { hwnd, after: 0, x: r[0], y: r[1], cx: r[2] - r[0], cy: r[3] - r[1], flags };
    let mut p1 = pos(0x1837);
    let mut p2 = pos(0x0100_1837);
    let mut calc1 = NcCalcSize { rgrc: [r, r, r], pos: &mut p1 };
    let mut calc2 = NcCalcSize { rgrc: [r, r, r], pos: &mut p2 };
    // SAFETY: messages to a window with structures that outlive the calls.
    unsafe {
        SendMessageW(hwnd, WM_NCCALCSIZE, 1, &mut calc1 as *mut NcCalcSize as isize);
        SendMessageW(hwnd, WM_NCPAINT, 1, 0);
        SendMessageW(hwnd, WM_WINDOWPOSCHANGED, 0, &mut p1 as *mut WindowPos as isize);
        SendMessageW(hwnd, WM_NCCALCSIZE, 1, &mut calc2 as *mut NcCalcSize as isize);
        SendMessageW(hwnd, WM_NCPAINT, 1, 0);
        SendMessageW(hwnd, WM_DWMNCRENDERINGCHANGED, usize::from(enabled), 0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DwmSetWindowAttribute(hwnd: usize, attr: u32, value: *const u8, size: u32) -> i32 {
    if !exists(attr) {
        return E_INVALIDARG;
    }
    if !is_window(hwnd) || is_desktop(hwnd) {
        set_last_error(6);
        return E_HANDLE;
    }
    if value.is_null() {
        return E_INVALIDARG;
    }
    if size == 0 {
        set_last_error(0);
        return E_INVALIDARG;
    }
    if matches!(attr, CAPTION_BUTTON_BOUNDS | EXTENDED_FRAME_BOUNDS) && size < 16 {
        return E_INSUFFICIENT;
    }
    let colour = matches!(attr, BORDER_COLOR | CAPTION_COLOR | TEXT_COLOR);
    if size < 4 || (colour && size != 4) || !(write_only(attr) || round_trip(attr)) {
        return E_INVALIDARG;
    }
    // SAFETY: at least four bytes.
    let v = unsafe { core::ptr::read_unaligned(value as *const u32) };
    let v = match attr {
        PASSIVE_UPDATE_MODE | USE_IMMERSIVE_DARK_MODE | CLOAK => u32::from(v != 0),
        WINDOW_CORNER_PREFERENCE => {
            if v > 4 {
                return E_INVALIDARG;
            }
            v
        }
        BORDER_COLOR | CAPTION_COLOR | TEXT_COLOR => {
            let default = v == 0xFFFF_FFFE && attr != TEXT_COLOR;
            if v != 0xFFFF_FFFF && !default && v > 0x00FF_FFFF {
                return E_INVALIDARG;
            }
            v
        }
        _ => v,
    };
    if attr == NCRENDERING_POLICY {
        let before = nc_enabled(hwnd);
        store(hwnd, attr, v);
        let after = nc_enabled(hwnd);
        if before != after && v <= 2 {
            nc_changed(hwnd, after);
        }
        return S_OK;
    }
    store(hwnd, attr, v);
    S_OK
}

fn put_rect(out: *mut u8, b: [i32; 4]) -> i32 {
    // SAFETY: a RECT's worth of bytes at `out`.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr() as *const u8, out, 16) };
    S_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DwmGetWindowAttribute(hwnd: usize, attr: u32, out: *mut u8, size: u32) -> i32 {
    if !exists(attr) {
        return E_INVALIDARG;
    }
    let valid = is_window(hwnd);
    let child_or_desktop = valid && (is_desktop(hwnd) || style(hwnd) & WS_CHILD != 0);
    let readable = matches!(attr, 1 | 5 | 9 | 14 | 15 | 16 | 19 | 20 | 33 | 37 | 38 | 40);
    if !valid {
        set_last_error(6);
        return E_HANDLE;
    }
    if child_or_desktop && !matches!(attr, CLOAKED | VISIBLE_FRAME_BORDER_THICKNESS) {
        if !readable {
            return E_INVALIDARG;
        }
        set_last_error(6);
        return E_HANDLE;
    }
    if size < 4 || out.is_null() || (attr == VISIBLE_FRAME_BORDER_THICKNESS && size != 4) {
        return E_INVALIDARG;
    }
    let rect = matches!(attr, CAPTION_BUTTON_BOUNDS | EXTENDED_FRAME_BOUNDS);
    if (rect && size < 16) || (attr == 40 && size == 4) {
        return E_INSUFFICIENT;
    }
    let put = |v: u32| {
        // SAFETY: at least four bytes.
        unsafe { core::ptr::write_unaligned(out as *mut u32, v) };
        S_OK
    };
    match attr {
        NCRENDERING_ENABLED => put(u32::from(nc_enabled(hwnd))),
        CLOAKED => put(stored(hwnd, CLOAK).unwrap_or(0)),
        15 | 19 | 40 => put(0),
        PASSIVE_UPDATE_MODE | USE_IMMERSIVE_DARK_MODE | WINDOW_CORNER_PREFERENCE | SYSTEMBACKDROP_TYPE => {
            put(stored(hwnd, attr).unwrap_or(0))
        }
        VISIBLE_FRAME_BORDER_THICKNESS => put(1),
        EXTENDED_FRAME_BOUNDS => {
            let mut r = [0i32; 4];
            // SAFETY: a RECT out.
            unsafe { GetWindowRect(hwnd, r.as_mut_ptr()) };
            let s = style(hwnd);
            let inset = if s & WS_CAPTION == WS_CAPTION {
                7
            } else if s & WS_POPUP != 0 && s & WS_THICKFRAME != 0 {
                6
            } else {
                0
            };
            put_rect(out, [r[0] + inset, r[1], r[2] - inset, r[3] - inset])
        }
        CAPTION_BUTTON_BOUNDS => {
            let mut r = [0i32; 4];
            // SAFETY: a RECT out.
            unsafe { GetWindowRect(hwnd, r.as_mut_ptr()) };
            let s = style(hwnd);
            // SAFETY: a user32 call on a window handle.
            let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };
            let w = r[2] - r[0];
            let b: [i32; 4] = if s & WS_CAPTION == WS_CAPTION {
                if s & (WS_MINIMIZEBOX | WS_MAXIMIZEBOX) != 0 && ex & WS_EX_TOOLWINDOW == 0 {
                    [w - 153, 0, w - 7, 30]
                } else if s & WS_SYSMENU != 0 {
                    [w - 56, 0, w - 7, 30]
                } else {
                    [w - 7, 0, w - 7, 30]
                }
            } else if s & WS_THICKFRAME != 0 {
                [w - 6, 0, w - 6, 29]
            } else {
                [0, 0, 0, 0]
            };
            put_rect(out, b)
        }
        _ => E_INVALIDARG,
    }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dwmapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
