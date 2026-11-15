//! Carets: `CreateCaret`, `DestroyCaret`, `ShowCaret`, `HideCaret`, `SetCaretPos`,
//! `GetCaretPos`, `SetCaretBlinkTime` and `GetCaretBlinkTime`. The state (one caret and one
//! position per input queue, the hide count) and the rules are win32k's (`poly_win32k::caret`);
//! this file gives them their Win32 shapes, works out the caret's size from the bitmap argument
//! and draws the blinking caret in the interactive session (below).
//!
//! Sources: Learn, "Carets", `CreateCaret` ("hBitmap: ... If this parameter is (HBITMAP)1, the
//! caret is gray. If this parameter is (HBITMAP)2, the caret is gray (halftone)"; zero width or
//! height: "the system's window border width or height"), and the sheets of
//! `specs/windows/user32.dll`.

use super::{H, SYS, call, set_last_error};
use poly_win32k::caret::{self as c, UNCHANGED};
use poly_win32k::metrics;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user::{self as u};

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetObjectType(h: usize) -> u32;
    fn GetObjectW(h: usize, size: i32, out: *mut u8) -> i32;
}

const OBJ_BITMAP: u32 = 7;
const ERROR_NOACCESS: u32 = 998;
const ERROR_REQUIRES_INTERACTIVE_WINDOWSTATION: u32 = 1459;
const SM_CXBORDER: i32 = 5;
const SM_CYBORDER: i32 = 6;

/// What a caret call returned: the reply's snapshot, or the call failed (the last error set unless it is to stay).
fn run(op: u8, hwnd: H, a: i32, b: i32) -> Option<c::Snapshot> {
    match call(Request::Caret { op, hwnd: hwnd as u32, a, b, tid: 0 }) {
        Reply::Caret { ok: true, hwnd, width, height, x, y, shown, .. } => {
            Some(c::Snapshot { hwnd, width, height, x, y, shown })
        }
        Reply::Caret { code, .. } => {
            if code != UNCHANGED {
                set_last_error(code);
            }
            None
        }
        _ => {
            set_last_error(87);
            None
        }
    }
}

/// The size of a caret made from `bitmap` (the width and height of the call otherwise).
fn caret_size(bitmap: usize, width: i32, height: i32) -> (i32, i32) {
    match bitmap {
        0 | 1 => {
            let screen = u::screen(&SYS);
            let border = |m| metrics::system_metric(m, &screen).unwrap_or(1);
            (if width == 0 { border(SM_CXBORDER) } else { width }, if height == 0 { border(SM_CYBORDER) } else { height })
        }
        // SAFETY: gdi32 looks the handle up; a handle that is none gives 0.
        h if unsafe { GetObjectType(h) } == OBJ_BITMAP => {
            let mut bm = [0i32; 8];
            // SAFETY: a BITMAP is 32 bytes at most.
            let n = unsafe { GetObjectW(h, 32, bm.as_mut_ptr() as *mut u8) };
            if n >= 12 { (bm[1], bm[2]) } else { (0, 0) }
        }
        // Handle 2 (the halftone caret) and a handle that is none: an empty rectangle.
        _ => (0, 0),
    }
}

/// `CreateCaret(hWnd, hBitmap, nWidth, nHeight)`.
#[unsafe(no_mangle)]
pub extern "system" fn CreateCaret(hwnd: H, bitmap: usize, width: i32, height: i32) -> i32 {
    if hwnd == 0 || super::info(hwnd).is_none() {
        set_last_error(1400);
        return 0;
    }
    let (w, h) = caret_size(bitmap, width, height);
    let ok = run(c::OP_CREATE, hwnd, w, h).is_some();
    if ok {
        super::caretdraw::created(hwnd);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyCaret() -> i32 {
    super::caretdraw::before_change();
    let ok = run(c::OP_DESTROY, 0, 0, 0).is_some();
    if ok {
        super::caretdraw::destroyed();
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowCaret(hwnd: H) -> i32 {
    super::caretdraw::before_change();
    let r = run(c::OP_SHOW, hwnd, 0, 0);
    super::caretdraw::after_change(r);
    i32::from(r.is_some())
}

#[unsafe(no_mangle)]
pub extern "system" fn HideCaret(hwnd: H) -> i32 {
    super::caretdraw::before_change();
    let r = run(c::OP_HIDE, hwnd, 0, 0);
    super::caretdraw::after_change(r);
    i32::from(r.is_some())
}

#[unsafe(no_mangle)]
pub extern "system" fn SetCaretPos(x: i32, y: i32) -> i32 {
    super::caretdraw::before_change();
    let r = run(c::OP_SET_POS, 0, x, y);
    super::caretdraw::after_change(r);
    i32::from(r.is_some())
}

/// `GetCaretPos(lpPoint)`: the queue's position, also without a caret.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCaretPos(pt: *mut i32) -> i32 {
    if pt.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    let Some(s) = run(c::OP_GET, 0, 0, 0) else { return 0 };
    // SAFETY: the caller's POINT.
    unsafe {
        *pt = s.x;
        *pt.add(1) = s.y;
    }
    1
}

/// `GetCaretBlinkTime`: the interactive session's setting (500 ms until `SetCaretBlinkTime`).
#[unsafe(no_mangle)]
pub extern "system" fn GetCaretBlinkTime() -> u32 {
    super::caretdraw::blink_time()
}

/// `SetCaretBlinkTime`: only an interactive window station has a setting to change
/// (SetCaretBlinkTime.yaml `non_interactive`: 1459 and the time stays).
#[unsafe(no_mangle)]
pub extern "system" fn SetCaretBlinkTime(ms: u32) -> i32 {
    if !u::screen(&SYS).interactive {
        set_last_error(ERROR_REQUIRES_INTERACTIVE_WINDOWSTATION);
        return 0;
    }
    super::caretdraw::set_blink_time(ms);
    1
}
