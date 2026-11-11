//! Input in `user32` (ADR 0131, slice B): what a retrieved mouse message
//! sends first (WM_NCHITTEST, WM_MOUSEACTIVATE, WM_SETCURSOR) and its
//! client coordinates, TranslateMessage over the keyboard layout
//! (`crate::kbd`), SendInput's events.

use super::*;
use crate::kbd;
use crate::wm::raw::{WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN};

pub const WM_SETCURSOR: u32 = 0x0020;
/// `HTCLIENT`, `HTERROR` (winuser.h; HTERROR is -2, 16 bits in a hit code).
pub const HTCLIENT: u64 = 1;
const HTERROR: u64 = 0xFFFE;
/// `MA_ACTIVATE`, `MA_NOACTIVATE` (winuser.h).
const MA_NOACTIVATE: u64 = 3;
const MA_NOACTIVATEANDEAT: u64 = 4;

/// The window DefWindowProc tracks the non-client mouse area of (it
/// handled the last WM_NCMOUSEMOVE); 0 for none (TrackMouseEvent.yaml
/// `nonclient`, SetCursorPos.yaml `caption`).
pub static NC_TRACK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// `WM_NCMOUSELEAVE` (winuser.h).
const WM_NCMOUSELEAVE: u32 = 0x02A2;

fn is_button_down(m: u32) -> bool {
    matches!(m, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN)
}

/// A mouse message being retrieved (PM_REMOVE): the hit test and cursor
/// messages to its window first; client coordinates for a client hit, the
/// WM_NC* message otherwise (About Mouse Input). `false`: the message is
/// eaten (MA_NOACTIVATEANDEAT).
pub fn mouse_retrieved<E: Env + ?Sized>(env: &E, m: &mut CMsg) -> bool {
    let h = m.hwnd;
    let Some(i) = info(env, h) else { return true };
    let (sx, sy) = (m.l as u16 as i16 as i32, (m.l >> 16) as u16 as i16 as i32);
    let screen_l = m.l;
    let st0 = focus::input(env);
    // A disabled top-level window: the hit is HTERROR, WM_SETCURSOR is all
    // it gets; no WM_NCHITTEST, no activation, the mouse message is dropped
    // (EnableWindow.yaml `mouse_disabled_popup`).
    if i.style & WS_DISABLED != 0 && i.style & WS_CHILD == 0 && st0.capture != i.hwnd && m.message != WM_MOUSEWHEEL {
        send(env, h, WM_SETCURSOR, h, HTERROR | (u64::from(m.message) << 16));
        return false;
    }
    if st0.capture == i.hwnd {
        // Captured: client coordinates of the capture window, no hit test
        // and no WM_SETCURSOR (SetCapture.yaml).
        if m.message != WM_MOUSEWHEEL {
            m.l = makelparam(sx - i.client.left, sy - i.client.top);
        }
        return true;
    }
    // A mouse move over the non-client area uses the geometric hit test
    // directly (SetCursorPos.yaml: no WM_NCHITTEST there). Client moves and
    // button events send WM_NCHITTEST to the window procedure.
    let hit = if m.message == WM_MOUSEMOVE {
        let geometric = def_window_proc(env, h, WM_NCHITTEST, 0, screen_l);
        if geometric == HTCLIENT { send(env, h, WM_NCHITTEST, 0, screen_l) } else { geometric }
    } else {
        send(env, h, WM_NCHITTEST, 0, screen_l)
    };
    // Moving from the tracked non-client area into the client area (or
    // another window) ends the tracking: WM_NCMOUSELEAVE follows the move.
    if m.message == WM_MOUSEMOVE {
        let tracked = NC_TRACK.load(core::sync::atomic::Ordering::Relaxed);
        if tracked != 0 && (tracked != i.hwnd || hit == HTCLIENT) {
            NC_TRACK.store(0, core::sync::atomic::Ordering::Relaxed);
            env.call(Request::Post { hwnd: u64::from(tracked), msg: WM_NCMOUSELEAVE, w: 0, l: 0 });
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
                focus::CLICK.store(true, core::sync::atomic::Ordering::Relaxed);
                // A click takes the foreground whatever the thread's right to it.
                focus::activate_as(env, top, false, true, false);
                focus::CLICK.store(false, core::sync::atomic::Ordering::Relaxed);
                // The window raised under the cursor gets a move after the
                // button messages queued behind this one
                // (WindowFromPoint.yaml wfp_activate: WM_NCHITTEST,
                // WM_SETCURSOR, WM_MOUSEMOVE after the WM_LBUTTONUP).
                if let Reply::Value { value } = env.call(Request::Cursor { set: false, x: 0, y: 0 }) {
                    env.call(Request::Cursor { set: true, x: value as u32 as i32, y: (value >> 32) as u32 as i32 });
                }
            }
            eat = r == MA_NOACTIVATEANDEAT;
        }
    }
    if m.message != WM_MOUSEWHEEL {
        send(env, h, WM_SETCURSOR, h, (hit & 0xFFFF) | (u64::from(m.message) << 16));
    }
    if eat {
        return false;
    }
    if m.message == WM_MOUSEWHEEL {
        return true;
    }
    if hit == HTCLIENT {
        let (cx, cy) = (sx - i.client.left, sy - i.client.top);
        m.l = makelparam(cx, cy);
    } else {
        // WM_NCMOUSEMOVE .. WM_NCMBUTTONDBLCLK: the hit code, screen coordinates.
        m.message = m.message - WM_MOUSEMOVE + 0x00A0;
        m.w = hit & 0xFFFF;
    }
    true
}

/// `TrackMouseEvent` with `TME_NONCLIENT` (TrackMouseEvent.yaml
/// `nonclient`): `TME_LEAVE` asks for `WM_NCMOUSELEAVE`. With the cursor
/// outside the window's non-client area the message is posted at once;
/// over it, DefWindowProc's tracking of that area (`NC_TRACK`) ends with it.
/// `TME_CANCEL` takes the tracking away.
pub fn track_nonclient<E: Env + ?Sized>(env: &E, hwnd: u32, flags: u32) {
    const TME_LEAVE: u32 = 0x2;
    const TME_CANCEL: u32 = 0x8000_0000;
    use core::sync::atomic::Ordering::Relaxed;
    if flags & TME_CANCEL != 0 {
        if flags & TME_LEAVE != 0 && NC_TRACK.load(Relaxed) == hwnd {
            NC_TRACK.store(0, Relaxed);
        }
        return;
    }
    if flags & TME_LEAVE == 0 {
        return;
    }
    let (x, y) = match env.call(Request::Cursor { set: false, x: 0, y: 0 }) {
        Reply::Value { value } => (value as u32 as i32, (value >> 32) as u32 as i32),
        _ => (0, 0),
    };
    let under = match env.call(Request::WindowFromPoint { x, y, below: 0 }) {
        Reply::Handle { hwnd } => hwnd,
        _ => 0,
    };
    let in_nonclient = under == hwnd && {
        let hit = def_window_proc(env, u64::from(hwnd), WM_NCHITTEST, 0, makelparam(x, y));
        hit > HTCLIENT
    };
    if in_nonclient {
        NC_TRACK.store(hwnd, Relaxed);
    } else {
        env.call(Request::Post { hwnd: u64::from(hwnd), msg: WM_NCMOUSELEAVE, w: 0, l: 0 });
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
                env.call(Request::Post { hwnd: m.hwnd, msg, w: u64::from(m.private), l: m.l });
                return true;
            }
            let keys = key_state(env);
            for c in kbd::translate_key(m.message, m.w, m.l, &keys, dead).iter() {
                env.call(Request::Post { hwnd: m.hwnd, msg: c.msg, w: c.wparam, l: c.lparam });
            }
            true
        }
        WM_KEYUP | WM_SYSKEYUP => true,
        _ => false,
    }
}
