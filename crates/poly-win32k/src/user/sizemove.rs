//! The sizing and moving loop of `DefWindowProc` (`WM_SYSCOMMAND` with `SC_SIZE` or `SC_MOVE`;
//! ADR 0212): `WM_ENTERSIZEMOVE`, the mouse moves as `WM_SIZING` / `WM_MOVING` and
//! `SetWindowPos`, `WM_EXITSIZEMOVE`.
//!
//! Sources: the spec sheet `WM_ENTERSIZEMOVE.yaml` and its visible tests' recorded results
//! (`szm_*`), Microsoft Learn ("WM_ENTERSIZEMOVE", "WM_EXITSIZEMOVE", "WM_SIZING", "WM_MOVING",
//! "WM_SYSCOMMAND"). The order of the messages, in the sheet's words:
//!
//! - start: `WM_CAPTURECHANGED` for a move, `WM_GETMINMAXINFO`, `WM_ENTERSIZEMOVE`,
//!   `WM_NCMOUSELEAVE` (the non-client tracking ends with the capture);
//! - each mouse move: the rectangle (the window's at the start moved or sized by the distance the
//!   pointer went, an edge never past the track limits) goes to the window as `WM_SIZING` or
//!   `WM_MOVING`, which may change it; if it differs from the window's, `SetWindowPos` with the
//!   flags `0x80214`;
//! - the button's release (or Escape, which gives the window back its first rectangle):
//!   `WM_CAPTURECHANGED`, a `SetWindowPos` that changes nothing (`0x204`, `0x205` for a move: only
//!   `WM_WINDOWPOSCHANGING` and, for a size, `WM_GETMINMAXINFO` come of it), `WM_EXITSIZEMOVE`.

use super::*;
use crate::wm::raw::{WM_KEYDOWN, WM_MOUSEMOVE, WM_SYSKEYDOWN};

const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_SIZING: u32 = 0x0214;
const WM_MOVING: u32 = 0x0216;

const WM_EXITSIZEMOVE: u32 = 0x0232;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_LBUTTONUP: u32 = 0x0202;
/// The flags of the `SetWindowPos` of every step: `SWP_NOACTIVATE | SWP_NOZORDER |
/// SWP_NOOWNERZORDER` and the bit `0x80000` that the oracle's calls carry.
const STEP_FLAGS: u32 = 0x8_0214;
/// The closing call's flags: the same without the activation and the internal bit, and for a
/// move `SWP_NOSIZE` too.
const END_SIZE_FLAGS: u32 = 0x204;
const END_MOVE_FLAGS: u32 = 0x205;
const VK_ESCAPE: usize = 0x1B;
const VK_RETURN: usize = 0x0D;

/// The window rectangle a drag of the side or corner `edge` (`WMSZ_LEFT` 1 .. `WMSZ_BOTTOMRIGHT`
/// 8) by (`dx`, `dy`) makes of `start`; the width and height stay within `min` and `max` (the edge
/// opposite the dragged one stays where it is).
pub fn sized(start: Rect, edge: u64, dx: i32, dy: i32, min: (i32, i32), max: (i32, i32)) -> Rect {
    let (left, right, top, bottom) = match edge {
        1 => (true, false, false, false),
        2 => (false, true, false, false),
        3 => (false, false, true, false),
        4 => (true, false, true, false),
        5 => (false, true, true, false),
        6 => (false, false, false, true),
        7 => (true, false, false, true),
        8 => (false, true, false, true),
        _ => (false, false, false, false),
    };
    let mut r = start;
    if left {
        r.left += dx;
    }
    if right {
        r.right += dx;
    }
    if top {
        r.top += dy;
    }
    if bottom {
        r.bottom += dy;
    }
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    if left || right {
        let w = clamp(r.width(), min.0, max.0);
        if left {
            r.left = r.right - w;
        } else {
            r.right = r.left + w;
        }
    }
    if top || bottom {
        let h = clamp(r.height(), min.1, max.1);
        if top {
            r.top = r.bottom - h;
        } else {
            r.bottom = r.top + h;
        }
    }
    r
}

/// The loop of window `hwnd`'s move (`cmd` = `SC_MOVE`) or size by `edge` (`SC_SIZE`) that the press
/// at `l` (screen coordinates; 0: the cursor's place) started.
pub fn run<E: Env + ?Sized>(env: &E, hwnd: u64, cmd: u64, edge: u64, l: u64) {
    let Some(i) = info(env, hwnd) else { return };
    let moving = cmd == SC_MOVE;
    let start = if l == 0 {
        match env.call(Request::Cursor { set: false, x: 0, y: 0 }) {
            Reply::Value { value } => (value as u32 as i32, (value >> 32) as u32 as i32),
            _ => (0, 0),
        }
    } else {
        (l as u16 as i16 as i32, (l >> 16) as u16 as i16 as i32)
    };
    let orig = i.window;
    if moving {
        send(env, hwnd, WM_CAPTURECHANGED, 0, 0);
    }
    let mmi = ask_minmax(env, &i);
    let (min, max) = if metrics::track_limited(i.style) {
        ((mmi.min_track.x, mmi.min_track.y), (mmi.max_track.x, mmi.max_track.y))
    } else {
        ((0, 0), (i32::MAX, i32::MAX))
    };
    send(env, hwnd, WM_ENTERSIZEMOVE, 0, 0);
    let tracked = input::NC_TRACK.swap(0, core::sync::atomic::Ordering::Relaxed);
    if tracked != 0 {
        env.call(Request::Post { hwnd: u64::from(tracked), msg: WM_NCMOUSELEAVE, w: 0, l: 0 });
    }
    focus::set_capture(env, hwnd);
    let mut m = CMsg::default();
    let mut cancelled = false;
    loop {
        let got = get_message(env, &mut m, 0, 0, 0);
        if got <= 0 {
            if got == 0 {
                // WM_QUIT is the thread's to see after the loop.
                env.call(Request::Post { hwnd: 0, msg: WM_QUIT, w: m.w as u64, l: 0 });
            }
            break;
        }
        let Some(now) = info(env, hwnd) else { return };
        match m.message {
            WM_MOUSEMOVE => {
                let (dx, dy) = (m.pt.x - start.0, m.pt.y - start.1);
                step(env, hwnd, &now, orig, moving, edge, (dx, dy), min, max);
            }
            WM_LBUTTONUP => break,
            WM_KEYDOWN | WM_SYSKEYDOWN if m.w == VK_ESCAPE => {
                cancelled = true;
                break;
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if m.w == VK_RETURN => break,
            _ => {
                translate_message(env, &m);
                dispatch_message(env, &m);
            }
        }
    }
    if cancelled && let Some(now) = info(env, hwnd) {
        let (x, y, w, h) = (orig.left, orig.top, orig.width(), orig.height());
        if now.window != orig {
            set_window_pos(env, hwnd, 0, x, y, w, h, STEP_FLAGS);
        }
    }
    focus::release_capture(env);
    if let Some(now) = info(env, hwnd) {
        let r = now.window_rel;
        set_window_pos(
            env,
            hwnd,
            0,
            r.left,
            r.top,
            r.width(),
            r.height(),
            if moving { END_MOVE_FLAGS } else { END_SIZE_FLAGS },
        );
    }
    send(env, hwnd, WM_EXITSIZEMOVE, 0, 0);
}

/// One mouse move of the loop: the rectangle, `WM_SIZING` / `WM_MOVING`, `SetWindowPos`.
#[allow(clippy::too_many_arguments)]
fn step<E: Env + ?Sized>(
    env: &E,
    hwnd: u64,
    now: &WinInfo,
    orig: Rect,
    moving: bool,
    edge: u64,
    (dx, dy): (i32, i32),
    min: (i32, i32),
    max: (i32, i32),
) {
    let r = if moving {
        Rect::new(orig.left + dx, orig.top + dy, orig.right + dx, orig.bottom + dy)
    } else {
        sized(orig, edge, dx, dy, min, max)
    };
    let mut cr = CRect::from(r);
    send(env, hwnd, if moving { WM_MOVING } else { WM_SIZING }, if moving { 0 } else { edge }, ptr(&mut cr));
    let r = Rect::from(cr);
    if r != now.window {
        set_window_pos(env, hwnd, 0, r.left, r.top, r.width(), r.height(), STEP_FLAGS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Rect = Rect::new(200, 200, 620, 500);

    #[test]
    fn an_edge_moves_with_the_pointer() {
        let any = (0, i32::MAX);
        let r = |edge, dx, dy| sized(START, edge, dx, dy, (136, 39), (any.1, any.1));
        assert_eq!(r(2, 30, 7), Rect::new(200, 200, 650, 500));
        assert_eq!(r(1, 30, 7), Rect::new(230, 200, 620, 500));
        assert_eq!(r(3, 30, -20), Rect::new(200, 180, 620, 500));
        assert_eq!(r(6, 30, 25), Rect::new(200, 200, 620, 525));
        assert_eq!(r(8, 30, 25), Rect::new(200, 200, 650, 525));
        assert_eq!(r(4, -10, -10), Rect::new(190, 190, 620, 500));
    }

    #[test]
    fn the_track_limits_keep_the_opposite_edge() {
        let r = |edge, dx, dy| sized(START, edge, dx, dy, (136, 39), (1940, 1100));
        // Right edge dragged far left: 136 wide from the left edge.
        assert_eq!(r(2, -400, 0), Rect::new(200, 200, 336, 500));
        // Left edge dragged far right: 136 wide ending at the right edge.
        assert_eq!(r(1, 400, 0), Rect::new(484, 200, 620, 500));
        assert_eq!(r(6, 0, -400), Rect::new(200, 200, 620, 239));
        assert_eq!(r(2, 5000, 0), Rect::new(200, 200, 2140, 500));
    }
}
