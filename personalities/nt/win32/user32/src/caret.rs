//! The caret (Learn: "Carets"): one per thread, owned by a window of that thread, hidden until
//! `ShowCaret`; `GetGUIThreadInfo` shows its window and rectangle (GetCaretBlinkTime.yaml `chrome`).

use super::{H, SYS, bad_window, info};
use alloc::vec::Vec;
use poly_win32_gui::{SpinLock, set_last_error};
use poly_win32k::user::Env;

const ERROR_ACCESS_DENIED: u32 = 5;

#[derive(Clone, Copy)]
struct Caret {
    hwnd: usize,
    width: i32,
    height: i32,
    /// `ShowCaret` calls minus `HideCaret` calls, from the caret being made hidden.
    shown: i32,
}

/// A thread's caret and where it lies: the position outlives the caret (`GetCaretPos` after
/// `DestroyCaret` still answers the last one).
struct Slot {
    tid: u32,
    caret: Option<Caret>,
    pos: (i32, i32),
}

static SLOTS: SpinLock<Vec<Slot>> = SpinLock::new(Vec::new());

fn with_slot<R>(f: impl FnOnce(&mut Slot) -> R) -> R {
    let tid = SYS.tid();
    SLOTS.with(|v| {
        let i = match v.iter().position(|s| s.tid == tid) {
            Some(i) => i,
            None => {
                v.push(Slot { tid, caret: None, pos: (0, 0) });
                v.len() - 1
            }
        };
        f(&mut v[i])
    })
}

/// What `GetGUIThreadInfo` shows of thread `tid`'s caret: its window, the rectangle in that
/// window's client coordinates and `GUI_CARETBLINKING` while it is shown.
pub fn of_thread(tid: u32) -> (usize, [i32; 4], u32) {
    SLOTS.with(|v| match v.iter().find(|s| s.tid == tid).and_then(|s| s.caret.map(|c| (c, s.pos))) {
        Some((c, (x, y))) => (c.hwnd, [x, y, x + c.width, y + c.height], u32::from(c.shown > 0)),
        None => (0, [0; 4], 0),
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateCaret(hwnd: H, bitmap: H, width: i32, height: i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if i.tid != SYS.tid() {
        set_last_error(ERROR_ACCESS_DENIED);
        return 0;
    }
    // A caret made from a bitmap takes its size; the gray caret handles 1 and 2 keep the given size.
    let _ = bitmap;
    let (w, h) = (width.max(1), height.max(1));
    with_slot(|s| s.caret = Some(Caret { hwnd, width: w, height: h, shown: 0 }));
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyCaret() -> i32 {
    with_slot(|s| i32::from(s.caret.take().is_some()))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetCaretPos(x: i32, y: i32) -> i32 {
    with_slot(|s| {
        if s.caret.is_none() {
            set_last_error(ERROR_ACCESS_DENIED);
            return 0;
        }
        s.pos = (x, y);
        1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCaretPos(pt: *mut i32) -> i32 {
    if pt.is_null() {
        set_last_error(87);
        return 0;
    }
    let (x, y) = with_slot(|s| s.pos);
    // SAFETY: the caller's POINT.
    unsafe {
        *pt = x;
        *pt.add(1) = y;
    }
    1
}

/// `ShowCaret` (`delta` 1) and `HideCaret` (`delta` -1) for window `hwnd` (NULL: the caret's own).
fn adjust(hwnd: H, delta: i32) -> i32 {
    with_slot(|s| match &mut s.caret {
        Some(c) if hwnd == 0 || hwnd == c.hwnd => {
            c.shown += delta;
            1
        }
        _ => {
            set_last_error(ERROR_ACCESS_DENIED);
            0
        }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowCaret(hwnd: H) -> i32 {
    adjust(hwnd, 1)
}

#[unsafe(no_mangle)]
pub extern "system" fn HideCaret(hwnd: H) -> i32 {
    adjust(hwnd, -1)
}
