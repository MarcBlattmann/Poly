//! Input in `user32` (ADR 0131, slice B): what a retrieved mouse message
//! sends first (WM_NCHITTEST, WM_MOUSEACTIVATE, WM_SETCURSOR) and its
//! client coordinates, TranslateMessage over the keyboard layout
//! (`crate::kbd`), SendInput's events.

use super::*;
use crate::kbd;
use crate::wm::raw::{WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN};

pub const WM_SETCURSOR: u32 = 0x0020;
/// `HTCLIENT` (winuser.h).
pub const HTCLIENT: u64 = 1;
/// `MA_ACTIVATE`, `MA_NOACTIVATE` (winuser.h).
const MA_NOACTIVATE: u64 = 3;
const MA_NOACTIVATEANDEAT: u64 = 4;

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
    if st0.capture == i.hwnd {
        // Captured: client coordinates of the capture window, no hit test
        // and no WM_SETCURSOR (SetCapture.yaml).
        if m.message != WM_MOUSEWHEEL {
            m.l = makelparam(sx - i.client.left, sy - i.client.top);
        }
        return true;
    }
    // A client mouse move reuses DefWindowProc's geometric hit test instead
    // of calling the app's WM_NCHITTEST procedure. A button-down asks the app
    // first, so it can return HTTRANSPARENT or a non-client code.
    let hit = if m.message == WM_MOUSEMOVE {
        def_window_proc(env, h, WM_NCHITTEST, 0, screen_l)
    } else {
        send(env, h, WM_NCHITTEST, 0, screen_l)
    };
    let mut eat = false;
    if is_button_down(m.message) {
        let top = top_level_of(env, &i);
        let st = focus::input(env);
        if top != st.active {
            if i.style & WS_CHILD != 0 {
                // WM_PARENTNOTIFY(button) to the parent first.
                let p = info(env, u64::from(i.parent));
                let pl = p.map_or(0, |p| makelparam(sx - p.client.left, sy - p.client.top));
                send(env, u64::from(i.parent), 0x0210, u64::from(m.message), pl);
            }
            let r = send(env, h, WM_MOUSEACTIVATE, u64::from(top), (hit & 0xFFFF) | (u64::from(m.message) << 16));
            if r != MA_NOACTIVATE && r != MA_NOACTIVATEANDEAT {
                focus::CLICK.store(true, core::sync::atomic::Ordering::Relaxed);
                focus::activate(env, top, false, true);
                focus::CLICK.store(false, core::sync::atomic::Ordering::Relaxed);
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
