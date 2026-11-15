//! The blinking caret on the interactive desktop (`caret.rs` has the calls). A shown caret is a
//! rectangle that is inverted in its owner window's client area (`DSTINVERT`, as Windows does) and
//! inverted again every `GetCaretBlinkTime` milliseconds by a timer of the owner's thread; in a
//! session without a desktop nothing is drawn and no timer runs (the sheets' observations are of
//! `GetGUIThreadInfo`, which is the model's).

use super::{H, SYS, SpinLock};
use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32k::caret as c;
use poly_win32k::user::Env as _;

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PatBlt(hdc: usize, x: i32, y: i32, w: i32, h: i32, rop: u32) -> i32;
}

/// Windows' default blink time on the oracle (GetCaretBlinkTime.yaml): 500 ms.
const DEFAULT_BLINK_MS: u32 = 500;
static BLINK_MS: AtomicU32 = AtomicU32::new(DEFAULT_BLINK_MS);
/// `DSTINVERT` (`wingdi.h`).
const DSTINVERT: u32 = 0x0055_0009;
/// The id of the blink timer (a system timer's in Windows; ours is an ordinary one nobody else uses).
const BLINK_TIMER: usize = 0xCA12;
const INFINITE: u32 = 0xFFFF_FFFF;

pub fn blink_time() -> u32 {
    BLINK_MS.load(Ordering::Relaxed)
}

pub fn set_blink_time(ms: u32) {
    BLINK_MS.store(ms, Ordering::Relaxed);
    // A running caret takes the new rate at once.
    let timer_on = STATE.with(|s| s.as_ref().map(|c| (c.hwnd, c.timer)));
    if let Some((hwnd, true)) = timer_on {
        start_timer(hwnd);
    }
}

/// The caret that is on show: its window, its rectangle (client coordinates) and whether it is
/// inverted on the screen at the moment.
struct Drawn {
    hwnd: H,
    rect: [i32; 4],
    on: bool,
    timer: bool,
}

static STATE: SpinLock<Option<Drawn>> = SpinLock::new(None);

fn interactive() -> bool {
    poly_win32k::user::screen(&SYS).interactive
}

/// Inverts `rect` of the client area of `hwnd`.
fn invert(hwnd: H, rect: [i32; 4]) {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    if w <= 0 || h <= 0 || super::info(hwnd).is_none() {
        return;
    }
    let dc = super::GetDC(hwnd);
    if dc == 0 {
        return;
    }
    // SAFETY: our gdi32 checks the DC.
    unsafe { PatBlt(dc, rect[0], rect[1], w, h, DSTINVERT) };
    super::ReleaseDC(hwnd, dc);
}

fn start_timer(hwnd: H) {
    let ms = blink_time();
    if ms == INFINITE || ms == 0 {
        super::KillTimer(hwnd, BLINK_TIMER);
        return;
    }
    super::SetTimer(hwnd, BLINK_TIMER, ms.max(10), blink_proc as usize);
    STATE.with(|s| {
        if let Some(c) = s {
            c.timer = true;
        }
    });
}

/// The timer's procedure: the caret goes off and on.
unsafe extern "system" fn blink_proc(_hwnd: H, _msg: u32, _id: usize, _tick: u32) {
    let shown = STATE.with(|s| {
        s.as_mut().map(|c| {
            c.on = !c.on;
            (c.hwnd, c.rect)
        })
    });
    if let Some((hwnd, rect)) = shown {
        invert(hwnd, rect);
    }
}

/// Takes the drawn caret off the screen (before its rectangle or window changes).
fn erase() {
    let was = STATE.with(|s| s.as_mut().and_then(|c| core::mem::replace(&mut c.on, false).then_some((c.hwnd, c.rect))));
    if let Some((hwnd, rect)) = was {
        invert(hwnd, rect);
    }
}

pub fn created(_hwnd: H) {
    if !interactive() {
        return;
    }
    erase();
    if let Some(c) = STATE.with(|s| s.take()) {
        super::KillTimer(c.hwnd, BLINK_TIMER);
    }
}

pub fn destroyed() {
    if !interactive() {
        return;
    }
    erase();
    if let Some(c) = STATE.with(|s| s.take()) {
        super::KillTimer(c.hwnd, BLINK_TIMER);
    }
}

/// Before `ShowCaret`, `HideCaret` or `SetCaretPos` changes the caret: it comes off the screen.
pub fn before_change() {
    if interactive() {
        erase();
    }
}

/// After the change: a shown caret is drawn at its place and blinks, a hidden one is not.
pub fn after_change(r: Option<c::Snapshot>) {
    if !interactive() {
        return;
    }
    let Some(s) = r else { return };
    let hwnd = s.hwnd as H;
    if !s.shown || hwnd == 0 {
        if let Some(old) = STATE.with(|st| st.take()) {
            super::KillTimer(old.hwnd, BLINK_TIMER);
        }
        return;
    }
    let rect = [s.x, s.y, s.x.wrapping_add(s.width), s.y.wrapping_add(s.height)];
    let had_timer = STATE.with(|st| st.as_ref().is_some_and(|c| c.timer && c.hwnd == hwnd));
    STATE.with(|st| *st = Some(Drawn { hwnd, rect, on: true, timer: had_timer }));
    invert(hwnd, rect);
    if !had_timer {
        start_timer(hwnd);
    }
}

/// The owner window painted over the caret: what is on the screen there is the application's again.
#[allow(dead_code)]
pub fn painted(hwnd: H) {
    STATE.with(|s| {
        if let Some(c) = s
            && c.hwnd == hwnd
        {
            c.on = false;
        }
    });
}
