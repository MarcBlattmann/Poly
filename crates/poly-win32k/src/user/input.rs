//! Input in `user32` (ADR 0131, slice B): what a retrieved mouse message
//! sends first (WM_NCHITTEST, WM_MOUSEACTIVATE, WM_SETCURSOR) and its
//! client coordinates, TranslateMessage over the keyboard layout
//! (`crate::kbd`), SendInput's events.

use super::*;
use crate::kbd;
use crate::wm::raw::{
    SYNTHETIC_MOVE, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
};

pub const WM_SETCURSOR: u32 = 0x0020;
/// `HTCLIENT`, `HTERROR` (winuser.h; HTERROR is -2, 16 bits in a hit code).
pub const HTCLIENT: u64 = 1;
const HTERROR: u64 = 0xFFFE;
/// `MA_ACTIVATE`, `MA_NOACTIVATE` (winuser.h).
const MA_NOACTIVATE: u64 = 3;
const MA_NOACTIVATEANDEAT: u64 = 4;

/// The window whose non-client mouse area is tracked as far as this process
/// knows (DefWindowProc handled its last WM_NCMOUSEMOVE, or the program asked
/// with TME_NONCLIENT); 0 for none (TrackMouseEvent.yaml `nonclient`,
/// SetCursorPos.yaml `caption`). The tracking itself is the window manager's
/// (`wm::pointer`): it ends it when the pointer is over another window, also
/// one of another process; this hint only saves a request on client moves.
pub static NC_TRACK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// `HTTRANSPARENT` (winuser.h): -1.
const HTTRANSPARENT: u64 = u64::MAX;

/// The vertical and horizontal wheel messages: they keep the screen position in `lParam` and get
/// no `WM_SETCURSOR`.
fn is_wheel(m: u32) -> bool {
    m == WM_MOUSEWHEEL || m == WM_MOUSEHWHEEL
}

/// Set when a move loop ended (`sizemove`): the mouse move its capture release makes is followed by
/// a second one (`WM_ENTERSIZEMOVE.yaml` `szm_caption`: two `WM_NCMOUSEMOVE` after
/// `WM_EXITSIZEMOVE`).
pub static AFTER_MOVE_LOOP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The window and screen place of the last mouse move whose hit was the caption (0: none).
static CAPTION_SPOT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn caption_spot(hwnd: u32, x: i32, y: i32) -> u64 {
    (u64::from(hwnd) << 32) | (u64::from(x as u16) << 16) | u64::from(y as u16) | (1 << 63)
}

fn is_button_down(m: u32) -> bool {
    matches!(m, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN)
}

/// A mouse message being retrieved (PM_REMOVE): the hit test and cursor
/// messages to its window first; client coordinates for a client hit, the
/// WM_NC* message otherwise (About Mouse Input). `false`: the message is
/// eaten (MA_NOACTIVATEANDEAT).
pub fn mouse_retrieved<E: Env + ?Sized>(env: &E, m: &mut CMsg) -> bool {
    let h = m.hwnd as u64;
    let Some(i) = info(env, h) else { return true };
    let (sx, sy) = (m.l as u16 as i16 as i32, (m.l >> 16) as u16 as i16 as i32);
    let screen_l = m.l as u64;
    let st0 = focus::input(env);
    // A disabled top-level window: the hit is HTERROR, WM_SETCURSOR is all
    // it gets; no WM_NCHITTEST, no activation, the mouse message is dropped
    // (EnableWindow.yaml `mouse_disabled_popup`).
    if i.style & WS_DISABLED != 0 && i.style & WS_CHILD == 0 && st0.capture != i.hwnd && !is_wheel(m.message) {
        send(env, h, WM_SETCURSOR, h, HTERROR | (u64::from(m.message) << 16));
        return false;
    }
    if st0.capture == i.hwnd {
        // Captured: client coordinates of the capture window, no hit test
        // and no WM_SETCURSOR (SetCapture.yaml).
        if !is_wheel(m.message) {
            m.l = (makelparam(sx - i.client.left, sy - i.client.top)) as usize;
        }
        return true;
    }
    // A mouse move over the non-client area uses the geometric hit test
    // directly (SetCursorPos.yaml: no WM_NCHITTEST there). Client moves and
    // button events send WM_NCHITTEST to the window procedure.
    // The move a capture release makes (`Msg::private`) asks the window like a button message
    // does (`WM_ENTERSIZEMOVE.yaml`: WM_NCHITTEST over the border after a sizing loop).
    let synthetic = m.private == SYNTHETIC_MOVE;
    let hit = if m.message == WM_MOUSEMOVE && !synthetic {
        let geometric = def_window_proc(env, h, WM_NCHITTEST, 0, screen_l);
        // A move over the caption is remembered: the press that follows at the same place
        // reuses its hit (`WM_ENTERSIZEMOVE.yaml` `szm_caption`: no WM_NCHITTEST at the press).
        CAPTION_SPOT.store(
            if geometric == frame::HTCAPTION as u64 { caption_spot(i.hwnd, sx, sy) } else { 0 },
            core::sync::atomic::Ordering::Relaxed,
        );
        if geometric == HTCLIENT { send(env, h, WM_NCHITTEST, 0, screen_l) } else { geometric }
    } else if is_button_down(m.message)
        && CAPTION_SPOT.swap(0, core::sync::atomic::Ordering::Relaxed) == caption_spot(i.hwnd, sx, sy)
    {
        frame::HTCAPTION as u64
    } else if synthetic && def_window_proc(env, h, WM_NCHITTEST, 0, screen_l) == frame::HTCAPTION as u64 {
        // Over the caption it is not asked either (`szm_caption`, `szm_clamp`).
        frame::HTCAPTION as u64
    } else {
        send(env, h, WM_NCHITTEST, 0, screen_l)
    };
    if synthetic && AFTER_MOVE_LOOP.swap(false, core::sync::atomic::Ordering::Relaxed) {
        env.call(Request::Cursor { set: true, x: sx, y: sy });
    }
    // HTTRANSPARENT: the move belongs to the window below; the window manager
    // makes it again for that one (WM_MOUSELEAVE.yaml `windows_that_do_not_cover`).
    if hit == HTTRANSPARENT && m.message == WM_MOUSEMOVE {
        env.call(Request::TrackMouse { hwnd: i.hwnd, flags: crate::wm::pointer::TRACK_TRANSPARENT, hover: 0 });
        return false;
    }
    // Moving from the tracked non-client area into the client area ends the
    // tracking: WM_NCMOUSELEAVE follows the move. Over another window the
    // window manager has ended it already.
    if m.message == WM_MOUSEMOVE {
        let tracked = NC_TRACK.load(core::sync::atomic::Ordering::Relaxed);
        if tracked != 0 && (tracked != i.hwnd || hit == HTCLIENT) {
            NC_TRACK.store(0, core::sync::atomic::Ordering::Relaxed);
            if tracked == i.hwnd {
                env.call(Request::TrackMouse { hwnd: tracked, flags: crate::wm::pointer::TRACK_NC_END, hover: 0 });
            }
        }
    }
    let mut eat = false;
    if is_button_down(m.message) {
        let top = top_level_of(env, &i);
        let st = focus::input(env);
        let child = i.style & WS_CHILD != 0;
        // A press on a child, also of the active window, or on a top-level
        // window that is not the active one (EnableWindow.yaml
        // `click_sequence`): the active top-level window itself gets neither
        // message.
        if child || top != st.active {
            // WM_PARENTNOTIFY(button) to each ancestor, the nearest first.
            let mut cur = i.clone();
            while cur.style & WS_CHILD != 0 && cur.ex_style & WS_EX_NOPARENTNOTIFY == 0 {
                let Some(p) = info(env, u64::from(cur.parent)) else { break };
                let pl = makelparam(sx - p.client.left, sy - p.client.top);
                send(env, u64::from(p.hwnd), 0x0210, u64::from(m.message), pl);
                cur = p;
            }
            // To the window pressed; DefWindowProc passes it up to the
            // top-level window.
            let r = send(env, h, WM_MOUSEACTIVATE, u64::from(top), (hit & 0xFFFF) | (u64::from(m.message) << 16));
            if top != st.active && r != MA_NOACTIVATE && r != MA_NOACTIVATEANDEAT {
                // A click takes the foreground whatever the thread's right to it.
                let how = focus::How { foreground: true, click: true, ..focus::How::default() };
                focus::activate_with(env, top, how);
                // The window raised under the cursor gets a move after the
                // button messages queued behind this one
                // (WindowFromPoint.yaml wfp_activate: WM_NCHITTEST,
                // WM_SETCURSOR, WM_MOUSEMOVE after the WM_LBUTTONUP). In the
                // interactive session the window manager makes it when the
                // activation changed the window structure (`wm::pointer`;
                // WM_CAPTURECHANGED.yaml `button`: no move for a window that
                // stays where it is).
                if !screen(env).interactive
                    && let Reply::Value { value } = env.call(Request::Cursor { set: false, x: 0, y: 0 })
                {
                    env.call(Request::Cursor { set: true, x: value as u32 as i32, y: (value >> 32) as u32 as i32 });
                }
            }
            eat = r == MA_NOACTIVATEANDEAT;
        }
    }
    if !is_wheel(m.message) {
        send(env, h, WM_SETCURSOR, h, (hit & 0xFFFF) | (u64::from(m.message) << 16));
    }
    if eat {
        return false;
    }
    if is_wheel(m.message) {
        return true;
    }
    if hit == HTCLIENT {
        let (cx, cy) = (sx - i.client.left, sy - i.client.top);
        m.l = (makelparam(cx, cy)) as usize;
    } else {
        // WM_NCMOUSEMOVE .. WM_NCMBUTTONDBLCLK: the hit code, screen coordinates.
        m.message = m.message - WM_MOUSEMOVE + 0x00A0;
        m.w = (hit & 0xFFFF) as usize;
    }
    true
}

/// `TrackMouseEvent` with `TME_NONCLIENT` (TrackMouseEvent.yaml
/// `nonclient`): `TME_LEAVE` asks for `WM_NCMOUSELEAVE`. With the cursor
/// outside the window's non-client area the window manager posts it at once;
/// over it, the tracking of that area ends with it. `TME_CANCEL` takes the
/// tracking away.
pub fn track_nonclient<E: Env + ?Sized>(env: &E, hwnd: u32, flags: u32) {
    const TME_LEAVE: u32 = 0x2;
    const TME_CANCEL: u32 = 0x8000_0000;
    use core::sync::atomic::Ordering::Relaxed;
    if flags & TME_LEAVE == 0 {
        return;
    }
    env.call(Request::TrackMouse { hwnd, flags, hover: 0 });
    if flags & TME_CANCEL != 0 {
        if NC_TRACK.load(Relaxed) == hwnd {
            NC_TRACK.store(0, Relaxed);
        }
    } else {
        NC_TRACK.store(hwnd, Relaxed);
    }
}

fn top_level_of<E: Env + ?Sized>(env: &E, i: &WinInfo) -> u32 {
    let mut cur = i.clone();
    while cur.style & WS_CHILD != 0 {
        match info(env, u64::from(cur.parent)) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur.hwnd
}

/// The calling thread's key state.
pub fn key_state<E: Env + ?Sized>(env: &E) -> [u8; 256] {
    let mut out = [0u8; 256];
    if let Reply::Keys { keys } = env.call(Request::KeyboardState {}) {
        for (o, k) in out.iter_mut().zip(keys) {
            *o = k;
        }
    }
    out
}

/// `TranslateMessage`: the character messages of a key-down, from the
/// layout and the thread's key state (dead keys carried in `dead`).
pub fn translate<E: Env + ?Sized>(env: &E, m: &CMsg, dead: &mut kbd::DeadState) -> bool {
    match m.message {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if m.w == 0xE7 {
                // VK_PACKET: the unit SendInput gave (KEYEVENTF_UNICODE).
                let msg = if m.message == WM_KEYDOWN { WM_CHAR } else { WM_SYSCHAR };
                env.call(Request::Post { hwnd: m.hwnd as u64, msg, w: u64::from(m.private), l: m.l as u64 });
                return true;
            }
            let keys = key_state(env);
            for c in kbd::translate_key(m.message, m.w as u64, m.l as u64, &keys, dead).iter() {
                env.call(Request::Post { hwnd: m.hwnd as u64, msg: c.msg, w: c.wparam, l: c.lparam });
            }
            true
        }
        WM_KEYUP | WM_SYSKEYUP => true,
        _ => false,
    }
}
