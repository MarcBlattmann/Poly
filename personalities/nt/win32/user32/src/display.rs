//! DPI awareness, system parameters and colours, display modes and
//! monitors (ADR 0172): the third GUI slice's `user32` calls about the
//! screen. One monitor at 96 DPI; the sheets are
//! `SetProcessDpiAwarenessContext.yaml` ... `EnumDisplayMonitors.yaml`.

use super::{H, SYS, SpinLock, call, info, set_last_error};
use alloc::vec::Vec;
use poly_win32k::guic;
use poly_win32k::metrics;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::spi;
use poly_win32k::user::{self as u};

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> usize;
    fn GetClipBox(hdc: usize, rect: *mut i32) -> i32;
    fn SaveDC(hdc: usize) -> i32;
    fn RestoreDC(hdc: usize, saved: i32) -> i32;
    fn IntersectClipRect(hdc: usize, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn PolyGdiDisplayOrigin(hdc: usize, out: *mut i32) -> i32;
}

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_INVALID_SPI_VALUE: u32 = 1439;
const PRIMARY_MONITOR: usize = 0x0001_0001;

// ----- DPI awareness contexts

fn awareness_of(ctx: isize) -> Option<u8> {
    guic::context_awareness(ctx as i64)
}

#[unsafe(no_mangle)]
pub extern "system" fn IsValidDpiAwarenessContext(ctx: isize) -> i32 {
    i32::from(awareness_of(ctx).is_some())
}

#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDpiAwarenessContext(ctx: isize) -> i32 {
    // An invalid value is refused after the "already set" check (255 is none).
    let a = awareness_of(ctx).unwrap_or(255);
    match call(Request::Dpi { set: true, process: true, awareness: a }) {
        Reply::Ok {} => 1,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

fn thread_awareness() -> u8 {
    match call(Request::Dpi { set: false, process: false, awareness: 0 }) {
        Reply::Value { value } => value as u8,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDpiAwarenessContext() -> isize {
    guic::context_handle(thread_awareness()) as isize
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDpiAwarenessContextForProcess(_process: usize) -> isize {
    GetThreadDpiAwarenessContext()
}

#[unsafe(no_mangle)]
pub extern "system" fn SetThreadDpiAwarenessContext(ctx: isize) -> isize {
    let Some(a) = awareness_of(ctx) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    match call(Request::Dpi { set: true, process: false, awareness: a }) {
        Reply::Value { value } => guic::context_handle(value as u8) as isize,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowDpiAwarenessContext(hwnd: H) -> isize {
    match info(hwnd) {
        Some(i) => guic::context_handle(i.dpi) as isize,
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> i32 {
    match (awareness_of(a), awareness_of(b)) {
        (Some(x), Some(y)) => i32::from(x == y),
        _ => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetAwarenessFromDpiAwarenessContext(ctx: isize) -> i32 {
    match awareness_of(ctx) {
        Some(a) => guic::dpi_awareness(a),
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            -1
        }
    }
}

/// `IsProcessDPIAware`: whether the calling thread is not unaware.
#[unsafe(no_mangle)]
pub extern "system" fn IsProcessDPIAware() -> i32 {
    i32::from(guic::dpi_awareness(thread_awareness()) != 0)
}

/// `SetProcessDPIAware`: system awareness, once.
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDPIAware() -> i32 {
    i32::from(matches!(call(Request::Dpi { set: true, process: true, awareness: guic::AWARE_SYSTEM }), Reply::Ok {}))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDpiForProcess(_process: usize) -> u32 {
    96
}

/// Windows whose non-client scaling was enabled (the per-monitor v1 cascade
/// runs once per window).
static NCDPI_DONE: SpinLock<Vec<u32>> = SpinLock::new(Vec::new());

/// `EnableNonClientDpiScaling`: only inside the creation of a top-level
/// window of a per-monitor-aware thread; in a per-monitor (v1) process the
/// frame is recomputed at once (`EnableNonClientDpiScaling.yaml`).
#[unsafe(no_mangle)]
pub extern "system" fn EnableNonClientDpiScaling(hwnd: H) -> i32 {
    let Some(i) = info(hwnd) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    let aware = matches!(i.dpi, guic::AWARE_PER_MONITOR | guic::AWARE_PER_MONITOR_V2);
    if !aware || i.style & metrics::WS_CHILD != 0 || !i.creating {
        return 0;
    }
    if i.dpi == guic::AWARE_PER_MONITOR {
        let first = NCDPI_DONE.with(|d| {
            if d.contains(&i.hwnd) {
                false
            } else {
                d.push(i.hwnd);
                true
            }
        });
        if first {
            u::set_window_pos(
                &SYS,
                hwnd as u64,
                0,
                0,
                0,
                0,
                0,
                u::SWP_FRAMECHANGED | u::SWP_NOMOVE | u::SWP_NOSIZE | u::SWP_NOZORDER | u::SWP_NOACTIVATE,
            );
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustWindowRectExForDpi(
    rect: *mut i32,
    style: u32,
    menu: i32,
    ex: u32,
    dpi: u32,
) -> i32 {
    if dpi == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    unsafe {
        let r = poly_win32k::frame::Rect::new(*rect, *rect.add(1), *rect.add(2), *rect.add(3));
        super::put_rect(rect, metrics::adjust_window_rect_dpi(r, style, menu != 0, ex, dpi));
    }
    1
}

// ----- system colours

#[unsafe(no_mangle)]
pub extern "system" fn GetSysColor(index: i32) -> u32 {
    match guic::sys_color(index) {
        Some(c) => c,
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

static BRUSHES: SpinLock<[usize; 31]> = SpinLock::new([0; 31]);

#[unsafe(no_mangle)]
pub extern "system" fn GetSysColorBrush(index: i32) -> usize {
    let Some(color) = guic::sys_color(index) else { return 0 };
    BRUSHES.with(|b| {
        let slot = &mut b[index as usize];
        if *slot == 0 {
            // SAFETY: no preconditions.
            *slot = unsafe { CreateSolidBrush(color) };
        }
        *slot
    })
}

// ----- SystemParametersInfo

fn put32(p: *mut u8, at: usize, v: i32) {
    // SAFETY: the callers check the structure is large enough.
    unsafe { core::ptr::write_unaligned(p.add(at) as *mut i32, v) };
}

fn read32(p: *const u8, at: usize) -> u32 {
    // SAFETY: the callers read within the structure's first field or checked size.
    unsafe { core::ptr::read_unaligned(p.add(at) as *const u32) }
}

/// A `LOGFONTW` of the message font ("Segoe UI", 400, DEFAULT_CHARSET).
fn put_logfont(p: *mut u8, at: usize, height: i32) {
    put32(p, at, height);
    for k in 1..5 {
        put32(p, at + 4 * k, 0);
    }
    put32(p, at + 16, 400);
    // SAFETY: 92 bytes at `at` are inside the structure.
    unsafe {
        core::ptr::write_bytes(p.add(at + 20), 0, 72);
        *p.add(at + 23) = 1; // lfCharSet
        for (k, c) in "Segoe UI".encode_utf16().enumerate() {
            core::ptr::write_unaligned(p.add(at + 28 + 2 * k) as *mut u16, c);
        }
    }
}

/// The font height at `dpi` (`-DPI * 12 / 96` rounded).
fn font_height(dpi: u32) -> i32 {
    -(((i64::from(dpi) * 12 + 48) / 96) as i32)
}

fn ncm(p: *mut u8, size: u32, dpi: u32) {
    let sc = |v: i64| ((v * i64::from(dpi) + 48) / 96) as i32;
    let cx_size = match dpi {
        1 => 38,
        1000 => 377,
        _ => metrics::system_metric_for_dpi(30, dpi, &metrics::Display::SERVICE).unwrap_or(36),
    };
    put32(p, 4, metrics::frame_for_dpi(dpi) - 3);
    put32(p, 8, sc(17));
    put32(p, 12, sc(17));
    put32(p, 16, cx_size);
    put32(p, 20, sc(22));
    put_logfont(p, 24, font_height(dpi));
    put32(p, 116, sc(22));
    put32(p, 120, sc(22));
    put_logfont(p, 124, font_height(dpi));
    put32(p, 216, sc(19));
    put32(p, 220, sc(19));
    for at in [224, 316, 408] {
        put_logfont(p, at, font_height(dpi));
    }
    if size >= 504 {
        put32(p, 500, metrics::padded_for_dpi(dpi));
    }
}

fn iconmetrics(p: *mut u8, dpi: u32) {
    let sc = ((75 * i64::from(dpi) + 48) / 96) as i32;
    put32(p, 4, sc);
    put32(p, 8, sc);
    put32(p, 12, 1);
    put_logfont(p, 16, font_height(dpi));
}

/// Simple getters: (action, values) of the oracle's settings.
const SPI_VALUES: &[(u32, &[i32])] = &[
    (0x0001, &[1]),           // SPI_GETBEEP
    (0x0003, &[6, 10, 1]),    // SPI_GETMOUSE
    (0x0005, &[1]),           // SPI_GETBORDER
    (0x000A, &[31]),          // SPI_GETKEYBOARDSPEED
    (0x000D, &[75]),          // SPI_ICONHORIZONTALSPACING
    (0x0010, &[0]),           // SPI_GETSCREENSAVEACTIVE
    (0x0016, &[1]),           // SPI_GETKEYBOARDDELAY
    (0x0018, &[75]),          // SPI_ICONVERTICALSPACING
    (0x0019, &[1]),           // SPI_GETICONTITLEWRAP
    (0x001B, &[0]),           // SPI_GETMENUDROPALIGNMENT
    (0x0026, &[1]),           // SPI_GETDRAGFULLWINDOWS
    (0x0044, &[0]),           // SPI_GETKEYBOARDPREF
    (0x0046, &[0]),           // SPI_GETSCREENREADER
    (0x004A, &[1]),           // SPI_GETFONTSMOOTHING
    (0x005E, &[0]),           // SPI_GETMOUSETRAILS
    (0x005F, &[0]),           // SPI_GETSNAPTODEFBUTTON
    (0x0062, &[4]),           // SPI_GETMOUSEHOVERWIDTH
    (0x0064, &[4]),           // SPI_GETMOUSEHOVERHEIGHT
    (0x0066, &[400]),         // SPI_GETMOUSEHOVERTIME
    (0x0068, &[3]),           // SPI_GETWHEELSCROLLLINES
    (0x006A, &[400]),         // SPI_GETMENUSHOWDELAY
    (0x006C, &[3]),           // SPI_GETWHEELSCROLLCHARS
    (0x006E, &[1]),           // SPI_GETSHOWIMEUI
    (0x0070, &[10]),          // SPI_GETMOUSESPEED
    (0x1000, &[0]),           // SPI_GETACTIVEWINDOWTRACKING
    (0x1002, &[1]),           // SPI_GETMENUANIMATION
    (0x1004, &[1]),           // SPI_GETCOMBOBOXANIMATION
    (0x1006, &[1]),           // SPI_GETLISTBOXSMOOTHSCROLLING
    (0x1008, &[1]),           // SPI_GETGRADIENTCAPTIONS
    (0x100A, &[0]),           // SPI_GETKEYBOARDCUES
    (0x100C, &[0]),           // SPI_GETACTIVEWNDTRKZORDER
    (0x100E, &[1]),           // SPI_GETHOTTRACKING
    (0x1012, &[1]),           // SPI_GETMENUFADE
    (0x1014, &[1]),           // SPI_GETSELECTIONFADE
    (0x1016, &[1]),           // SPI_GETTOOLTIPANIMATION
    (0x1018, &[1]),           // SPI_GETTOOLTIPFADE
    (0x101A, &[1]),           // SPI_GETCURSORSHADOW
    (0x1022, &[1]),           // SPI_GETFLATMENU
    (0x1024, &[1]),           // SPI_GETDROPSHADOW
    (0x103E, &[1]),           // SPI_GETUIEFFECTS
    (0x1040, &[0]),           // SPI_GETDISABLEOVERLAPPEDCONTENT
    (0x1042, &[1]),           // SPI_GETCLIENTAREAANIMATION
    (0x2000, &[0x7FFF_FFFF]), // SPI_GETFOREGROUNDLOCKTIMEOUT
    (0x2002, &[0]),           // SPI_GETACTIVEWNDTRKTIMEOUT
    (0x2004, &[7]),           // SPI_GETFOREGROUNDFLASHCOUNT
    (0x2006, &[1]),           // SPI_GETCARETWIDTH
    (0x2008, &[1200]),        // SPI_GETMOUSECLICKLOCKTIME
    (0x200A, &[2]),           // SPI_GETFONTSMOOTHINGTYPE
    (0x200C, &[1200]),        // SPI_GETFONTSMOOTHINGCONTRAST
    (0x200E, &[1]),           // SPI_GETFOCUSBORDERWIDTH
    (0x2010, &[1]),           // SPI_GETFOCUSBORDERHEIGHT
    (0x2016, &[5]),           // SPI_GETMESSAGEDURATION
];

/// `SystemParametersInfoW` for the read-only getters (`SystemParametersInfoW.yaml`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemParametersInfoW(action: u32, param: u32, pv: *mut u8, _ini: u32) -> i32 {
    match action {
        0x0030 => {
            if pv.is_null() {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            // SAFETY: the caller's RECT.
            unsafe { super::put_rect(pv as *mut i32, u::screen(&SYS).work) };
            1
        }
        0x0029 => {
            // SAFETY: cbSize is the structure's first field.
            let size = if pv.is_null() { 0 } else { read32(pv, 0) };
            if size != 500 && size != 504 {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            ncm(pv, size, 96);
            1
        }
        0x002D => {
            // SAFETY: as above.
            if pv.is_null() || read32(pv, 0) != 108 {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            iconmetrics(pv, 96);
            1
        }
        0x001F => {
            if pv.is_null() || param != 92 {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            put_logfont(pv, 0, -12);
            1
        }
        0x0048 => {
            // SAFETY: as above.
            if pv.is_null() || read32(pv, 0) != 8 {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            put32(pv, 4, 1);
            1
        }
        0x0042 => {
            // SAFETY: as above.
            if pv.is_null() || read32(pv, 0) != 16 {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            put32(pv, 4, 0x7E);
            // SAFETY: the 8-byte scheme pointer of the structure.
            unsafe { core::ptr::write_unaligned(pv.add(8) as *mut u64, 0) };
            1
        }
        0x0012 | 0 => {
            set_last_error(ERROR_INVALID_SPI_VALUE);
            0
        }
        _ => match SPI_VALUES.iter().find(|e| e.0 == action) {
            Some(&(_, vals)) => {
                // Simple getters write through pvParam (SPI_ICONHORIZONTALSPACING
                // and the other uiParam-based ones also through uiParam 0).
                if pv.is_null() {
                    set_last_error(ERROR_INVALID_PARAMETER);
                    return 0;
                }
                for (k, &v) in vals.iter().enumerate() {
                    put32(pv, 4 * k, v);
                }
                1
            }
            // The queries the sheet did not observe: what Learn documents
            // as the defaults (`poly_win32k::spi`, ADR 0155); anything
            // else is no action (87).
            None => {
                // SAFETY: cbSize is the structure's first field.
                let cb = if spi::has_cb_size(action) && !pv.is_null() { read32(pv, 0) } else { 0 };
                let w = u::screen(&SYS).work;
                match spi::query(action, param, cb, [w.left, w.top, w.right, w.bottom]) {
                    spi::Outcome::Bytes(bytes) if !pv.is_null() => {
                        // SAFETY: the caller's buffer for this action holds the answer.
                        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), pv, bytes.len()) };
                        1
                    }
                    spi::Outcome::Error(code) => {
                        set_last_error(code);
                        0
                    }
                    _ => {
                        set_last_error(ERROR_INVALID_PARAMETER);
                        0
                    }
                }
            }
        },
    }
}

/// `SystemParametersInfoForDpi`: the three DPI-dependent actions at `dpi`,
/// everything else only with DPI 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemParametersInfoForDpi(
    action: u32,
    param: u32,
    pv: *mut u8,
    ini: u32,
    dpi: u32,
) -> i32 {
    if dpi == 0 {
        // SAFETY: forwarded unchanged.
        return unsafe { SystemParametersInfoW(action, param, pv, ini) };
    }
    let bad = |_: ()| {
        set_last_error(ERROR_INVALID_PARAMETER);
        0
    };
    match action {
        0x0029 => {
            if !pv.is_null() && param == 504 && read32(pv, 0) == 0 {
                // A size of 0 clears the buffer the caller named.
                // SAFETY: the caller's NONCLIENTMETRICSW of `param` bytes.
                unsafe { core::ptr::write_bytes(pv, 0, 504) };
            }
            if pv.is_null() || param != 504 || read32(pv, 0) != 504 {
                return bad(());
            }
            ncm(pv, 504, dpi);
            1
        }
        0x002D => {
            if pv.is_null() || read32(pv, 0) != 108 {
                return bad(());
            }
            iconmetrics(pv, dpi);
            1
        }
        0x001F => {
            if pv.is_null() || param != 92 {
                return bad(());
            }
            put_logfont(pv, 0, font_height(dpi));
            1
        }
        _ => bad(()),
    }
}

/// `GetAutoRotationState`: no auto-rotation hardware.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAutoRotationState(state: *mut u32) -> i32 {
    if state.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *state = 0 };
    1
}

// ----- display modes and devices

fn wstr_eq(p: *const u16, s: &str) -> bool {
    if p.is_null() {
        return false;
    }
    // SAFETY: a NUL-terminated name.
    let w = unsafe { poly_win32_gui::wide(p, -1) };
    let lower = |c: u16| if (0x41..=0x5A).contains(&c) { c + 0x20 } else { c };
    w.len() == s.len() && w.iter().zip(s.encode_utf16()).all(|(&a, b)| lower(a) == lower(b))
}

const DM_FIELDS_CURRENT: u32 = 0x207C_00A0;
const DM_FIELDS_MODE: u32 = 0x007C_0080;

fn put_name(p: *mut u8, at: usize, s: &str, max: usize) {
    // SAFETY: `max` units at `at` are inside the caller's structure.
    unsafe {
        core::ptr::write_bytes(p.add(at), 0, max * 2);
        for (k, c) in s.encode_utf16().take(max - 1).enumerate() {
            core::ptr::write_unaligned(p.add(at + 2 * k) as *mut u16, c);
        }
    }
}

fn modes() -> Vec<(i32, i32)> {
    let d = u::screen(&SYS);
    let mut v: Vec<(i32, i32)> =
        [(800, 600), (1024, 768), (1280, 720), (1280, 1024), (1366, 768), (1600, 900), (1920, 1080)]
            .into_iter()
            .filter(|&(w, h)| w <= d.width && h <= d.height)
            .collect();
    if !v.contains(&(d.width, d.height)) {
        v.push((d.width, d.height));
    }
    v.sort_unstable();
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplaySettingsW(device: *const u16, mode: u32, dm: *mut u8) -> i32 {
    if dm.is_null() {
        return 0;
    }
    // dmSize is written whatever happens.
    // SAFETY: a DEVMODEW of the caller (at least up to dmSize's offset).
    unsafe { core::ptr::write_unaligned(dm.add(68) as *mut u16, 188) };
    if !device.is_null() && !wstr_eq(device, "\\\\.\\display1") {
        return 0;
    }
    let d = u::screen(&SYS);
    let (w, h, fields, name, spec, freq) = match mode {
        0xFFFF_FFFF | 0xFFFF_FFFE => (d.width, d.height, DM_FIELDS_CURRENT, "CDD", 0x401u16, 60),
        0xFFFF_FFFD => {
            // SAFETY: the DEVMODEW fields written below are inside it.
            unsafe {
                put_name(dm, 0, "", 32);
                core::ptr::write_unaligned(dm.add(64) as *mut u16, 0);
                core::ptr::write_unaligned(dm.add(72) as *mut u32, 0x58_0000);
                core::ptr::write_bytes(dm.add(76), 0, 16);
                core::ptr::write_unaligned(dm.add(168) as *mut u32, 0);
                core::ptr::write_unaligned(dm.add(172) as *mut u32, 0);
                core::ptr::write_unaligned(dm.add(176) as *mut u32, 0);
                core::ptr::write_unaligned(dm.add(180) as *mut u32, 0);
                core::ptr::write_unaligned(dm.add(184) as *mut u32, 64);
            }
            return 1;
        }
        i => match modes().get(i as usize) {
            Some(&(w, h)) => (w, h, DM_FIELDS_MODE, "cdd", 0x401u16, 60),
            None => return 0,
        },
    };
    put_name(dm, 0, name, 32);
    // SAFETY: the DEVMODEW fields written below are inside it.
    unsafe {
        core::ptr::write_unaligned(dm.add(64) as *mut u16, spec);
        core::ptr::write_unaligned(dm.add(66) as *mut u16, 0x0401);
        core::ptr::write_unaligned(dm.add(70) as *mut u16, 0);
        core::ptr::write_unaligned(dm.add(72) as *mut u32, fields);
        // dmPosition (0, 0), dmDisplayOrientation, dmDisplayFixedOutput.
        core::ptr::write_bytes(dm.add(76), 0, 16);
        core::ptr::write_unaligned(dm.add(168) as *mut u32, 32);
        core::ptr::write_unaligned(dm.add(172) as *mut u32, w as u32);
        core::ptr::write_unaligned(dm.add(176) as *mut u32, h as u32);
        core::ptr::write_unaligned(dm.add(180) as *mut u32, 0);
        core::ptr::write_unaligned(dm.add(184) as *mut u32, freq);
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplaySettingsExW(device: *const u16, mode: u32, dm: *mut u8, _flags: u32) -> i32 {
    // SAFETY: forwarded.
    unsafe { EnumDisplaySettingsW(device, mode, dm) }
}

const DD_STRUCT_SIZE: usize = 840;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplayDevicesW(device: *const u16, index: u32, dd: *mut u8, flags: u32) -> i32 {
    if dd.is_null() {
        return 0;
    }
    // SAFETY: cb is the structure's first field.
    let cb = read32(dd, 0) as usize;
    let fill = |name: &str, string: &str, state: u32, id: &str, key: &str| {
        // DISPLAY_DEVICEW: cb, DeviceName[32], DeviceString[128], StateFlags,
        // DeviceID[128], DeviceKey[128]; a smaller cb stops the copy.
        if cb >= 4 + 64 {
            put_name(dd, 4, name, 32);
        }
        if cb >= 4 + 64 + 256 {
            put_name(dd, 68, string, 128);
        }
        if cb >= 4 + 64 + 256 + 4 {
            put32(dd, 324, state as i32);
        }
        if cb >= 328 + 256 {
            put_name(dd, 328, id, 128);
        }
        if cb >= 328 + 512 {
            put_name(dd, 584, key, 128);
        }
    };
    let _ = DD_STRUCT_SIZE;
    if device.is_null() {
        if index != 0 {
            return 0;
        }
        fill(
            "\\\\.\\DISPLAY1",
            "Poly Display Adapter",
            0x5,
            "PCI\\VEN_1AF4&DEV_1050",
            "\\Registry\\Machine\\System\\CurrentControlSet\\Control\\Video\\{00000000-0000-0000-0000-000000000000}\\0000",
        );
        return 1;
    }
    if !wstr_eq(device, "\\\\.\\display1") || index != 0 {
        return 0;
    }
    let id = if flags & 1 != 0 {
        "\\\\?\\DISPLAY#POLY0001#1&0#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
    } else {
        "MONITOR\\POLY0001\\{4d36e96e-e325-11ce-bfc1-08002be10318}\\0000"
    };
    fill("\\\\.\\DISPLAY1\\Monitor0", "Generic Monitor", 0x3, id, "");
    1
}

// ----- monitors

type MonitorEnumProc = unsafe extern "system" fn(usize, usize, *const i32, isize) -> i32;

fn isect(a: [i32; 4], b: [i32; 4]) -> Option<[i32; 4]> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplayMonitors(
    hdc: usize,
    clip: *const i32,
    f: Option<MonitorEnumProc>,
    data: isize,
) -> i32 {
    let Some(f) = f else { return 0 };
    let d = u::screen(&SYS);
    let screen = [0, 0, d.width, d.height];
    let clip = if clip.is_null() {
        None
    } else {
        // SAFETY: the caller's RECT.
        Some(unsafe { [*clip, *clip.add(1), *clip.add(2), *clip.add(3)] })
    };
    if hdc == 0 {
        let r = match clip {
            Some(c) => isect(screen, c),
            None => Some(screen),
        };
        return match r {
            None => 1,
            // SAFETY: the callback is the caller's.
            Some(r) => i32::from(unsafe { f(PRIMARY_MONITOR, 0, r.as_ptr(), data) } != 0),
        };
    }
    let mut org = [0i32; 2];
    // SAFETY: two i32 out.
    if unsafe { PolyGdiDisplayOrigin(hdc, org.as_mut_ptr()) } == 0 {
        return 0;
    }
    let mut cb = [0i32; 4];
    // SAFETY: a RECT out.
    if unsafe { GetClipBox(hdc, cb.as_mut_ptr()) } == 0 {
        return 1;
    }
    // The monitor in the DC's coordinates, cut by the DC's clip and the caller's rectangle.
    let mon = [screen[0] - org[0], screen[1] - org[1], screen[2] - org[0], screen[3] - org[1]];
    let mut r = isect(mon, cb);
    if let (Some(x), Some(c)) = (r, clip) {
        r = isect(x, c);
    }
    let Some(r) = r else { return 1 };
    // SAFETY: our gdi32; the DC's clip is the rectangle during the call.
    unsafe {
        let saved = SaveDC(hdc);
        IntersectClipRect(hdc, r[0], r[1], r[2], r[3]);
        let go = f(PRIMARY_MONITOR, hdc, r.as_ptr(), data);
        RestoreDC(hdc, saved);
        i32::from(go != 0)
    }
}

// ----- the per-monitor DPI functions of `api-ms-win-shcore-scaling-*` (user32 exports them too)

const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;

/// `GetDpiForMonitor`: 96 x 96 for the effective and angular types; the raw
/// type is not supported; the type is checked before the monitor.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDpiForMonitor(monitor: usize, kind: u32, dpi_x: *mut u32, dpi_y: *mut u32) -> i32 {
    if kind > 2 {
        set_last_error(0xA0);
        return 0x8007_00A0u32 as i32;
    }
    if monitor != PRIMARY_MONITOR {
        set_last_error(6);
        return 0x8007_0006u32 as i32;
    }
    if dpi_x.is_null() || dpi_y.is_null() {
        set_last_error(0x1E7);
        return 0x8007_01E7u32 as i32;
    }
    let v = if kind == 2 { 0 } else { 96 };
    // SAFETY: the caller's outputs.
    unsafe {
        *dpi_x = v;
        *dpi_y = v;
    }
    if kind == 2 {
        set_last_error(0x32);
        0x8007_0032u32 as i32
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetScaleFactorForMonitor(monitor: usize, scale: *mut i32) -> i32 {
    if scale.is_null() || monitor != PRIMARY_MONITOR {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *scale = 100 };
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDpiAwareness(value: u32) -> i32 {
    let ctx = match value {
        0 => -1,
        1 => -2,
        2 => -3,
        _ => return E_INVALIDARG,
    };
    if SetProcessDpiAwarenessContext(ctx) != 0 { 0 } else { E_ACCESSDENIED }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessDpiAwareness(process: usize, value: *mut u32) -> i32 {
    if value.is_null() {
        return E_INVALIDARG;
    }
    if process != 0 && process != usize::MAX {
        // SAFETY: the caller's output.
        unsafe { *value = u32::MAX };
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *value = guic::dpi_awareness(thread_awareness()) as u32 };
    0
}
