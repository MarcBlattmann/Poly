//! Activation, focus and the foreground in `user32` (ADR 0131, slice B):
//! the message sequences of GetActiveWindow.yaml, SetActiveWindow.yaml,
//! SetFocus.yaml, GetFocus.yaml, SetForegroundWindow.yaml,
//! GetForegroundWindow.yaml and EnableWindow.yaml over win32k's input
//! state (`crate::wm::input`).
//!
//! Activation from window X to Y of the thread: X WM_NCACTIVATE(0),
//! X WM_ACTIVATE(WA_INACTIVE), Y WM_WINDOWPOSCHANGING (brought to the top,
//! its owned windows and owner along), Y WM_WINDOWPOSCHANGED if the z
//! order changed, WM_ACTIVATEAPP(1) to the thread's top-level windows when
//! it had no active window, Y WM_NCACTIVATE(1), Y WM_ACTIVATE(WA_ACTIVE),
//! the old focus WM_KILLFOCUS, Y WM_SETFOCUS. Only an interactive session
//! activates windows when they are shown (session 0 activates nothing).

use super::*;
use crate::wm::input::*;

pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WM_ENABLE: u32 = 0x000A;
pub const WM_ACTIVATEAPP: u32 = 0x001C;
pub const WM_CANCELMODE: u32 = 0x001F;
pub const WM_NCACTIVATE: u32 = 0x0086;
/// `WA_INACTIVE` / `WA_ACTIVE` (winuser.h).
pub const WA_INACTIVE: u64 = 0;
pub const WA_ACTIVE: u64 = 1;
/// `WA_CLICKACTIVE` (winuser.h): an activation by a mouse click.
pub const WA_CLICKACTIVE: u64 = 2;
/// The next activation is a click's (`user::input`).
pub static CLICK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// The window `DestroyWindow` is hiding right now (0: none): its owner's
/// activation reports a z-order change (GetActiveWindow.yaml gaw_owner).
pub static DESTROYING: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// winerror.h.
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
/// `GWL_STYLE` (winuser.h).
const GWL_STYLE: i32 = -16;

/// The calling thread's input state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub active: u32,
    pub focus: u32,
    pub capture: u32,
    pub foreground: u32,
    /// The foreground window is one of this thread's (or the queue it is
    /// attached to).
    pub fg_mine: bool,
}

pub fn input<E: Env + ?Sized>(env: &E) -> Input {
    match env.call(Request::InputState {}) {
        Reply::Input { active, focus, capture, foreground, fg_mine } => {
            Input { active, focus, capture, foreground, fg_mine }
        }
        _ => Input::default(),
    }
}

/// Another thread's input state (tid 0: the foreground thread's).
pub fn thread_input<E: Env + ?Sized>(env: &E, tid: u32) -> Input {
    match env.call(Request::ThreadInput { tid }) {
        Reply::Input { active, focus, capture, foreground, fg_mine } => {
            Input { active, focus, capture, foreground, fg_mine }
        }
        _ => Input::default(),
    }
}

fn set<E: Env + ?Sized>(env: &E, active: u32, focus: u32, which: u8) {
    env.call(Request::SetInput { active, focus, capture: 0, which });
}

/// The top-level window above `h` (itself if it is one).
fn top_of<E: Env + ?Sized>(env: &E, h: u32) -> Option<WinInfo> {
    let mut i = info(env, u64::from(h))?;
    while i.style & WS_CHILD != 0 {
        i = info(env, u64::from(i.parent))?;
    }
    Some(i)
}

/// Whether `h` is in another thread's input queue than the caller's.
fn foreign<E: Env + ?Sized>(env: &E, i: &WinInfo) -> bool {
    i.tid != env.tid()
}

/// The minimized flag of WM_ACTIVATE / WM_NCACTIVATE (HIWORD 0x20).
fn min_bits(i: &WinInfo) -> u64 {
    if i.minimized { 0x20_0000 } else { 0 }
}

/// The thread's top-level windows (for WM_ACTIVATEAPP), topmost first.
fn thread_tops<E: Env + ?Sized>(env: &E) -> Vec<u32> {
    tops_of(env, env.tid())
}

/// The top-level windows of thread `tid`, topmost first (GetActiveWindow.yaml gaw_owner).
pub fn tops_of<E: Env + ?Sized>(env: &E, tid: u32) -> Vec<u32> {
    let d = desktop(env);
    let mut out = Vec::new();
    let mut h = match env.call(Request::GetWindow { hwnd: u64::from(d), cmd: 5 }) {
        Reply::Handle { hwnd } => hwnd,
        _ => 0,
    };
    while h != 0 {
        if info(env, u64::from(h)).is_some_and(|i| i.tid == tid && !i.message_only) {
            out.push(h);
        }
        h = match env.call(Request::GetWindow { hwnd: u64::from(h), cmd: 2 }) {
            Reply::Handle { hwnd } => hwnd,
            _ => 0,
        };
    }
    out
}

/// Bring `y` to the top with its owned windows above it and its owner
/// below (`changed`: also send WM_WINDOWPOSCHANGED when the z order moved).
fn raise<E: Env + ?Sized>(env: &E, y: &WinInfo, changed: bool, force: bool) {
    let h = u64::from(y.hwnd);
    let owned = match env.call(Request::Relatives { hwnd: y.hwnd }) {
        Reply::Relatives { owned, .. } => owned,
        _ => Vec::new(),
    };
    for &o in &owned {
        let mut wp =
            WindowPos { hwnd: u64::from(o), flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE, ..WindowPos::default() };
        send(env, u64::from(o), WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
    }
    let mut wp = WindowPos { hwnd: h, flags: SWP_NOMOVE | SWP_NOSIZE, ..WindowPos::default() };
    send(env, h, WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
    // A hidden window changes no z order the screen shows: no
    // WM_WINDOWPOSCHANGED (SetActiveWindow.yaml `hidden`).
    let changed = changed && y.style & WS_VISIBLE != 0;
    if y.owner != 0 {
        let mut ow = WindowPos {
            hwnd: u64::from(y.owner),
            flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            ..WindowPos::default()
        };
        send(env, u64::from(y.owner), WM_WINDOWPOSCHANGING, 0, ptr(&mut ow));
    }
    let moved = matches!(
        env.call(Request::ZOrder { hwnd: y.hwnd, after: HWND_TOP, apply: false }),
        Reply::Bool { value: true }
    );
    let force = force || {
        let d = DESTROYING.load(core::sync::atomic::Ordering::Relaxed);
        d != 0 && owned.contains(&d)
    };
    if moved || force {
        if moved {
            env.call(Request::ZOrder { hwnd: y.hwnd, after: HWND_TOP, apply: true });
        }
        if changed {
            // Owned windows moved along above it first.
            for &o in &owned {
                let Some(oi) = info(env, u64::from(o)) else { continue };
                let hidden = if oi.style & WS_VISIBLE == 0 { SWP_NOREDRAW } else { 0 };
                let mut od = WindowPos {
                    hwnd: u64::from(o),
                    x: oi.window_rel.left,
                    y: oi.window_rel.top,
                    cx: oi.window_rel.width(),
                    cy: oi.window_rel.height(),
                    flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE | hidden,
                    ..WindowPos::default()
                };
                send(env, u64::from(o), WM_WINDOWPOSCHANGED, 0, ptr(&mut od));
            }
            let mut done = WindowPos {
                hwnd: h,
                x: y.window_rel.left,
                y: y.window_rel.top,
                cx: y.window_rel.width(),
                cy: y.window_rel.height(),
                flags: SWP_NOMOVE | SWP_NOSIZE | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE,
                ..WindowPos::default()
            };
            send(env, h, WM_WINDOWPOSCHANGED, 0, ptr(&mut done));
        }
    }
}

/// Activate top-level window `y` of the calling thread; `in_show`: from
/// inside ShowWindow / SetWindowPos (which sends the final
/// WM_WINDOWPOSCHANGED itself); `foreground`: it may take the foreground
/// (else it does when no other thread of ours has it). Returns the window
/// that was active (0 when none or when the thread was in the background).
pub fn activate<E: Env + ?Sized>(env: &E, y: u32, in_show: bool, foreground: bool) -> u32 {
    let Some(yi) = info(env, u64::from(y)) else { return 0 };
    let st = input(env);
    let x = st.active;
    let fg_thread_other = st.foreground != 0
        && !st.fg_mine
        && info(env, u64::from(st.foreground)).is_some_and(|f| f.pid == yi.pid && f.pid != 0);
    let background = !foreground && fg_thread_other;
    if x == y && (st.fg_mine || background) {
        return x;
    }
    let xi = if x != 0 && x != y { info(env, u64::from(x)) } else { None };
    if let Some(xi) = &xi {
        send(env, u64::from(x), WM_NCACTIVATE, 0, u64::from(y));
        send(env, u64::from(x), WM_ACTIVATE, WA_INACTIVE | min_bits(xi), u64::from(y));
    }
    // A background input queue may update its active/focus window without
    // taking the foreground. Its attempted raise sends WINDOWPOSCHANGING,
    // but does not report a z-order change with WINDOWPOSCHANGED
    // (SetActiveWindow.yaml / SetFocus.yaml `background`).
    // An owned window activated by SetActiveWindow / SetFocus reports its z order as changed
    // even at the top already (SetActiveWindow.yaml saw_special `O`, SetForegroundWindow.yaml
    // sfw_owned); SetForegroundWindow, clicks and shows do not.
    raise(env, &yi, !in_show && !background, !foreground && yi.owner != 0);
    set(env, y, 0, SET_ACTIVE | if background { 0 } else { SET_FOREGROUND });
    if x == 0 {
        for t in thread_tops(env) {
            send(env, u64::from(t), WM_ACTIVATEAPP, 1, 0);
        }
    }
    let yi = info(env, u64::from(y)).unwrap_or(yi);
    let other = if xi.is_some() { u64::from(x) } else { 0 };
    send(env, u64::from(y), WM_NCACTIVATE, u64::from(!background) | min_bits(&yi), other);
    let state = if CLICK.swap(false, core::sync::atomic::Ordering::Relaxed) { WA_CLICKACTIVE } else { WA_ACTIVE };
    send(env, u64::from(y), WM_ACTIVATE, state | min_bits(&yi), other);
    let old_focus = st.focus;
    if yi.minimized {
        if old_focus != 0 {
            send(env, u64::from(old_focus), WM_KILLFOCUS, 0, 0);
        }
        set(env, 0, 0, SET_FOCUS);
    } else {
        if old_focus != 0 && old_focus != y {
            send(env, u64::from(old_focus), WM_KILLFOCUS, u64::from(y), 0);
        }
        set(env, 0, y, SET_FOCUS);
        send(env, u64::from(y), WM_SETFOCUS, u64::from(old_focus), 0);
    }
    if background { 0 } else { x }
}

/// Whether the foreground is another thread's of our process (then a
/// thread shows windows without activating them: GetFocus.yaml gf_threads).
pub fn in_background<E: Env + ?Sized>(env: &E, pid: u32) -> bool {
    let st = input(env);
    st.foreground != 0 && !st.fg_mine && info(env, u64::from(st.foreground)).is_some_and(|f| f.pid == pid && pid != 0)
}

/// The thread's windows lose the activation to a window of another
/// thread or process (`to`, 0: the keeper): WM_NCACTIVATE(0),
/// WM_ACTIVATE(WA_INACTIVE), WM_ACTIVATEAPP(0) to every top-level window
/// of the thread, WM_KILLFOCUS (unless `focus_first` killed it already).
pub fn deactivate_away<E: Env + ?Sized>(env: &E, focus_done: bool) {
    let st = input(env);
    if st.active != 0
        && let Some(xi) = info(env, u64::from(st.active))
    {
        send(env, u64::from(st.active), WM_NCACTIVATE, 0, 0);
        send(env, u64::from(st.active), WM_ACTIVATE, WA_INACTIVE | min_bits(&xi), 0);
    }
    if st.active != 0 {
        for t in thread_tops(env) {
            send(env, u64::from(t), WM_ACTIVATEAPP, 0, 0);
        }
    }
    if !focus_done && st.focus != 0 {
        send(env, u64::from(st.focus), WM_KILLFOCUS, 0, 0);
    }
    let fg = if st.fg_mine { SET_FOREGROUND } else { 0 };
    set(env, 0, 0, SET_ACTIVE | SET_FOCUS | fg);
}

/// The calling thread (which owns a visible top-level window) just
/// attached its input to thread `other`, the foreground thread: that
/// thread's active window is deactivated and its focus taken away
/// (GetCapture.yaml gc_thread: A gets WM_NCACTIVATE(0), WM_ACTIVATE,
/// WM_ACTIVATEAPP(0), WM_KILLFOCUS before the capture moves). A thread with
/// no window of its own (ReleaseCapture.yaml rc_nocapture) changes nothing.
pub fn attached_to_foreground<E: Env + ?Sized>(env: &E, other: u32) {
    // The newer thread's input state is the shared one (GetFocus.yaml gf_threads keeps
    // thread T's when the main thread attaches to it).
    if env.tid() <= other || !screen(env).interactive {
        return;
    }
    if !tops_of(env, env.tid()).into_iter().any(|t| info(env, u64::from(t)).is_some_and(|i| i.visible)) {
        return;
    }
    let st = thread_input(env, other);
    if st.active == 0 || !st.fg_mine {
        return;
    }
    let Some(xi) = info(env, u64::from(st.active)) else { return };
    send(env, u64::from(st.active), WM_NCACTIVATE, 0, 0);
    send(env, u64::from(st.active), WM_ACTIVATE, WA_INACTIVE | min_bits(&xi), 0);
    for t in tops_of(env, other) {
        send(env, u64::from(t), WM_ACTIVATEAPP, 0, 0);
    }
    if st.focus != 0 {
        send(env, u64::from(st.focus), WM_KILLFOCUS, 0, 0);
    }
    // The queue is shared now: its active and focus windows are cleared.
    set(env, 0, 0, SET_ACTIVE | SET_FOCUS);
}

/// After window `h` was hidden or is being destroyed (interactive): if it
/// was the active window the activation goes to its owner, if that can
/// take it, else away; if it had the focus (a child), the focus goes to
/// its parent (SetFocus.yaml `hide_focused`, GetActiveWindow.yaml).
pub fn after_hide<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    if !screen(env).interactive {
        return;
    }
    let st = input(env);
    if st.active == i.hwnd {
        let owner = (i.owner != 0).then(|| info(env, u64::from(i.owner))).flatten();
        match owner {
            Some(o) if o.visible && o.style & WS_DISABLED == 0 && o.tid == env.tid() => {
                activate(env, o.hwnd, false, true);
            }
            _ => deactivate_away(env, false),
        }
    } else if st.focus != 0 && i.style & WS_CHILD != 0 && is_within(env, st.focus, i.hwnd) {
        let parent = i.parent;
        send(env, u64::from(st.focus), WM_KILLFOCUS, u64::from(parent), 0);
        set(env, 0, parent, SET_FOCUS);
        send(env, u64::from(parent), WM_SETFOCUS, u64::from(st.focus), 0);
    }
}

/// Whether `h` is `root` or below it.
fn is_within<E: Env + ?Sized>(env: &E, h: u32, root: u32) -> bool {
    let mut cur = h;
    for _ in 0..64 {
        if cur == root {
            return true;
        }
        match info(env, u64::from(cur)) {
            Some(i) if i.style & WS_CHILD != 0 => cur = i.parent,
            _ => return false,
        }
    }
    false
}

/// A window `root` goes (hidden by DestroyWindow): a window below it that
/// holds the capture is sent WM_CANCELMODE and loses the capture with
/// WM_CAPTURECHANGED (lParam NULL). The capture of `root` itself ends
/// without a message (SetCapture.yaml `destroy`).
pub fn release_capture_below<E: Env + ?Sized>(env: &E, root: u32) {
    let st = input(env);
    if st.capture == 0 || st.capture == root || !is_within(env, st.capture, root) {
        return;
    }
    send(env, u64::from(st.capture), WM_CANCELMODE, 0, 0);
    env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: SET_CAPTURE });
    send(env, u64::from(st.capture), 0x0215, 0, 0); // WM_CAPTURECHANGED
}

/// `SetFocus` (SetFocus.yaml).
pub fn set_focus<E: Env + ?Sized>(env: &E, hwnd: u64) -> u64 {
    let st = input(env);
    if hwnd == 0 {
        if st.focus != 0 {
            send(env, u64::from(st.focus), WM_KILLFOCUS, 0, 0);
            set(env, 0, 0, SET_FOCUS);
        }
        return u64::from(st.focus);
    }
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if foreign(env, &i) && !(st.active != 0 && top_of(env, i.hwnd).is_some_and(|t| t.hwnd == st.active)) {
        env.set_last_error(ERROR_ACCESS_DENIED);
        return 0;
    }
    // The window and every window up to its top-level must be enabled.
    let mut cur = i.clone();
    loop {
        if cur.style & WS_DISABLED != 0 {
            env.set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        if cur.style & WS_CHILD == 0 {
            break;
        }
        match info(env, u64::from(cur.parent)) {
            Some(p) => cur = p,
            None => break,
        }
    }
    let top = cur.hwnd;
    let mut activated = false;
    if top != st.active || (!st.fg_mine && st.active == 0) {
        activate(env, top, false, false);
        activated = true;
    }
    let st = input(env);
    let old = st.focus;
    if old == i.hwnd && !activated {
        return u64::from(old);
    }
    if old != 0 {
        send(env, u64::from(old), WM_KILLFOCUS, u64::from(i.hwnd), 0);
    }
    set(env, 0, i.hwnd, SET_FOCUS);
    send(env, hwnd, WM_SETFOCUS, u64::from(old), 0);
    u64::from(old)
}

/// `SetActiveWindow` (SetActiveWindow.yaml).
pub fn set_active_window<E: Env + ?Sized>(env: &E, hwnd: u64) -> u64 {
    let st = input(env);
    if hwnd == 0 {
        return 0;
    }
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if foreign(env, &i) {
        return 0;
    }
    if i.style & WS_CHILD != 0 {
        return u64::from(st.active);
    }
    u64::from(activate(env, i.hwnd, false, false))
}

/// `SetForegroundWindow` (SetForegroundWindow.yaml).
pub fn set_foreground_window<E: Env + ?Sized>(env: &E, hwnd: u64) -> bool {
    let Some(i) = (hwnd != 0).then(|| info(env, hwnd)).flatten() else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    if i.style & WS_CHILD != 0 || i.desktop {
        return false;
    }
    let st = input(env);
    if st.foreground == i.hwnd {
        return true;
    }
    if foreign(env, &i) {
        deactivate_away(env, false);
        env.call(Request::GiveForeground { hwnd: i.hwnd });
        return true;
    }
    activate(env, i.hwnd, false, true);
    true
}

/// `EnableWindow` (EnableWindow.yaml): whether the window was disabled.
pub fn enable_window<E: Env + ?Sized>(env: &E, hwnd: u64, enable: bool) -> bool {
    let Some(i) = (hwnd != 0).then(|| info(env, hwnd)).flatten() else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return false;
    };
    let was_disabled = i.style & WS_DISABLED != 0;
    if enable {
        if was_disabled {
            env.call(Request::SetStyle { hwnd: i.hwnd, which: GWL_STYLE, value: i.style & !WS_DISABLED });
            send(env, hwnd, WM_ENABLE, 1, 0);
        }
    } else {
        send(env, hwnd, WM_CANCELMODE, 0, 0);
        let st = input(env);
        // Disabling the capture window releases the capture (SetCapture.yaml).
        if st.capture == i.hwnd {
            env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: SET_CAPTURE });
            send(env, hwnd, 0x0215, 0, 0); // WM_CAPTURECHANGED
        }
        if st.focus == i.hwnd {
            send(env, hwnd, WM_KILLFOCUS, 0, 0);
            set(env, 0, 0, SET_FOCUS);
        }
        if !was_disabled {
            env.call(Request::SetStyle { hwnd: i.hwnd, which: GWL_STYLE, value: i.style | WS_DISABLED });
            send(env, hwnd, WM_ENABLE, 0, 0);
        }
    }
    was_disabled
}
