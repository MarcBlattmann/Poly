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
pub const WM_IME_SETCONTEXT: u32 = 0x0281;
pub const WM_IME_NOTIFY: u32 = 0x0282;
/// `WM_IME_SETCONTEXT`'s lParam: ISC_SHOWUICOMPOSITIONWINDOW | ISC_SHOWUIGUIDELINE
/// | ISC_SHOWUIALLCANDIDATEWINDOW (`imm.h`).
const ISC_SHOWUIALL: u64 = 0xC000_000F;
const IMN_CLOSESTATUSWINDOW: u64 = 1;
const IMN_OPENSTATUSWINDOW: u64 = 2;

/// The input-method messages of an activation change (`ImmGetContext.yaml`):
/// the window losing it gets `WM_IME_SETCONTEXT(0)`, the one gaining it
/// `WM_IME_SETCONTEXT(1)`, then the old one `WM_IME_NOTIFY(IMN_CLOSESTATUSWINDOW)`
/// and the new one `WM_IME_NOTIFY(IMN_OPENSTATUSWINDOW)`.
fn ime_switch<E: Env + ?Sized>(env: &E, old: u32, new: u32) {
    if old != 0 {
        send(env, u64::from(old), WM_IME_SETCONTEXT, 0, ISC_SHOWUIALL);
    }
    if new != 0 {
        send(env, u64::from(new), WM_IME_SETCONTEXT, 1, ISC_SHOWUIALL);
    }
    if old != 0 {
        send(env, u64::from(old), WM_IME_NOTIFY, IMN_CLOSESTATUSWINDOW, 0);
    }
    if new != 0 {
        send(env, u64::from(new), WM_IME_NOTIFY, IMN_OPENSTATUSWINDOW, 0);
    }
}
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
/// from another thread of the process (else it does when no other thread
/// of ours has it). A thread that has lost the right to take the
/// foreground gets its active window without it
/// (GetForegroundWindow.yaml `foreground_right`). Returns the window that
/// was active (0 when none or when the thread was in the background).
pub fn activate<E: Env + ?Sized>(env: &E, y: u32, in_show: bool, foreground: bool) -> u32 {
    let stay_back = !may_take_foreground(env);
    activate_as(env, y, in_show, foreground, stay_back)
}

/// Whether the calling thread takes the foreground when a window becomes
/// its active window.
pub fn may_take_foreground<E: Env + ?Sized>(env: &E) -> bool {
    matches!(env.call(Request::MayTakeForeground {}), Reply::Bool { value: true })
}

/// [`activate`]; with `stay_back` the window becomes the thread's active
/// window without taking the foreground from the thread that has it
/// (GetFocus.yaml `background_focus`: WM_ACTIVATEAPP(1) when the thread had
/// no active window, WM_NCACTIVATE with wParam 0, WM_ACTIVATE(1),
/// WM_SETFOCUS).
pub fn activate_as<E: Env + ?Sized>(env: &E, y: u32, in_show: bool, foreground: bool, stay_back: bool) -> u32 {
    if !screen(env).interactive {
        // Session 0: SetActiveWindow and SetFocus ask without SWP_NOACTIVATE.
        let ask = if in_show { SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE } else { SWP_NOMOVE | SWP_NOSIZE };
        return activate_local(env, y, in_show, ask);
    }
    let Some(yi) = info(env, u64::from(y)) else { return 0 };
    let st = input(env);
    let x = st.active;
    let fg_thread_other = st.foreground != 0
        && !st.fg_mine
        && info(env, u64::from(st.foreground)).is_some_and(|f| f.pid == yi.pid && f.pid != 0);
    let behind_sibling = !foreground && fg_thread_other;
    let background = behind_sibling || (stay_back && !st.fg_mine && st.foreground != 0);
    if x == y && (st.fg_mine || background) {
        return x;
    }
    let xi = if x != 0 && x != y { info(env, u64::from(x)) } else { None };
    if let Some(xi) = &xi {
        send(env, u64::from(x), WM_NCACTIVATE, min_bits(xi), u64::from(y));
        send(env, u64::from(x), WM_ACTIVATE, WA_INACTIVE | min_bits(xi), u64::from(y));
    }
    // Behind another thread of the process the attempted raise sends
    // WINDOWPOSCHANGING, but no z-order change is reported with
    // WINDOWPOSCHANGED (SetActiveWindow.yaml / SetFocus.yaml `background`);
    // a thread behind another process's window reports it (GetFocus.yaml
    // `background_focus`: 0x1803 when it moved).
    // An owned window activated by SetActiveWindow / SetFocus reports its z order as changed
    // even at the top already (SetActiveWindow.yaml saw_special `O`, SetForegroundWindow.yaml
    // sfw_owned); SetForegroundWindow, clicks and shows do not.
    raise(env, &yi, !in_show && !behind_sibling, !foreground && yi.owner != 0);
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
        ime_switch(env, if xi.is_some() { x } else { 0 }, y);
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
/// thread or process: WM_NCACTIVATE(0), WM_ACTIVATE(WA_INACTIVE),
/// WM_ACTIVATEAPP(0) to every top-level window of the thread, WM_KILLFOCUS
/// (unless `focus_done`: it was killed already). The foreground goes back
/// to where the thread took it from.
pub fn deactivate_away<E: Env + ?Sized>(env: &E, focus_done: bool) {
    deactivate(env, focus_done, false, true);
}

/// As [`deactivate_away`]; with `names_next` the foreground is left alone:
/// the caller gives it to a window it names. `cancel_capture`: the
/// thread's capture ends with its activation.
fn deactivate<E: Env + ?Sized>(env: &E, focus_done: bool, names_next: bool, cancel_capture: bool) {
    let st = input(env);
    // The thread's capture ends with its activation (SetCapture.yaml
    // `hide_nobody_takes_over`: WM_CANCELMODE and WM_CAPTURECHANGED(NULL)
    // before the deactivation messages).
    if st.capture != 0 && cancel_capture {
        send(env, u64::from(st.capture), WM_CANCELMODE, 0, 0);
        if input(env).capture == st.capture {
            env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: SET_CAPTURE });
            send(env, u64::from(st.capture), WM_CAPTURECHANGED, 0, 0);
        }
    }
    if st.active != 0
        && let Some(xi) = info(env, u64::from(st.active))
    {
        // No minimized bit in WM_NCACTIVATE when no window is named
        // (gaw_leave_a: wParam 0, WM_ACTIVATE's HIWORD 32).
        send(env, u64::from(st.active), WM_NCACTIVATE, 0, 0);
        send(env, u64::from(st.active), WM_ACTIVATE, WA_INACTIVE | min_bits(&xi), 0);
    }
    if st.active != 0 {
        // Every top-level window of the thread, from the top of the z order
        // down as it is at that moment, hidden ones too (a window just
        // hidden is on top, a destroyed one keeps its place, a minimized
        // one is the last of its band: GetActiveWindow.yaml `next_active`).
        for t in thread_tops(env) {
            send(env, u64::from(t), WM_ACTIVATEAPP, 0, 0);
        }
    }
    if !focus_done && st.focus != 0 {
        send(env, u64::from(st.focus), WM_KILLFOCUS, 0, 0);
    }
    if st.active != 0 {
        ime_switch(env, st.active, 0);
    }
    let fg = if st.fg_mine && !names_next { SET_FOREGROUND } else { 0 };
    set(env, 0, 0, SET_ACTIVE | SET_FOCUS | fg);
}

/// Whether top-level window `h` of the calling thread can take the
/// activation from `from` (session 0): visible, enabled, not minimized,
/// not `WS_EX_NOACTIVATE`.
fn can_take<E: Env + ?Sized>(env: &E, h: u32, from: u32) -> bool {
    h != from
        && info(env, u64::from(h)).is_some_and(|w| {
            w.tid == env.tid()
                && w.style & WS_CHILD == 0
                && !w.desktop
                && !w.message_only
                && w.visible
                && !w.minimized
                && w.style & WS_DISABLED == 0
                && w.ex_style & WS_EX_NOACTIVATE == 0
        })
}

/// What becomes of the activation when a window goes (`crate::zorder::Next`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing changes.
    Stay,
    /// This window of the thread takes it.
    Take(u32),
    /// The thread loses it: the foreground goes to that window of another
    /// thread or process (0: back to where the thread took it from).
    Away(u32),
}

/// `crate::zorder::Cause` on the wire.
pub const GOES_HIDDEN: u8 = 0;
pub const GOES_DESTROYED: u8 = 1;
pub const GOES_MINIMIZED: u8 = 2;

/// Who takes the activation when top-level window `hwnd` of the calling
/// thread is hidden, destroyed or minimized (`cause`): win32k decides by
/// the thread's activation history and the z order (ADR 0176,
/// `crate::zorder::successor`; GetActiveWindow.yaml `next_active`,
/// `minimize_inactive`; GetForegroundWindow.yaml `handover_same_thread`,
/// `nobody_takes_over`, `minimize_foreground`).
pub fn next_active<E: Env + ?Sized>(env: &E, hwnd: u32, cause: u8) -> Next {
    match env.call(Request::NextActive { hwnd, cause }) {
        Reply::Next { kind: 1, hwnd } => Next::Take(hwnd),
        Reply::Next { kind: 2, hwnd } => Next::Away(hwnd),
        _ => Next::Stay,
    }
}

/// Window `i` gave way (hidden, destroyed or minimized): the activation
/// goes where `next` says. A window of the thread that takes over becomes
/// the foreground window when the thread had the foreground, and only
/// then.
pub fn hand_on<E: Env + ?Sized>(env: &E, next: Next, focus_done: bool) {
    let st = input(env);
    match next {
        Next::Stay => {}
        Next::Take(n) => {
            activate_as(env, n, false, true, !st.fg_mine);
        }
        Next::Away(0) => deactivate_away(env, focus_done),
        Next::Away(to) => {
            // The thread's windows are deactivated (if it had an active
            // one), then that window gets the foreground.
            if st.active != 0 {
                deactivate(env, focus_done, true, true);
            }
            env.call(Request::GiveForeground { hwnd: to });
        }
    }
}

/// Session 0 (ShowWindow.yaml `activation_ask`, `destroy`, sw_activation):
/// make `y` the thread's active window. No window is visible there and
/// nothing takes the foreground. The window that was active gets
/// WM_NCACTIVATE(0, lParam = `y`) and WM_ACTIVATE(WA_INACTIVE). A window
/// that takes the activation from another one is asked to come to the top
/// (WM_WINDOWPOSCHANGING with `ask`: SWP_NOACTIVATE | SWP_NOMOVE |
/// SWP_NOSIZE, without SWP_NOACTIVATE for SetActiveWindow), and outside a
/// SetWindowPos of its own (`in_swp`) it is moved there, with
/// WM_WINDOWPOSCHANGED when it moved; the thread's first active window is
/// not asked and its thread's windows get WM_ACTIVATEAPP(1). Then `y`
/// WM_NCACTIVATE with wParam 0 (nothing is drawn active), WM_ACTIVATE(1),
/// the focus messages. Returns the window that was active.
pub fn activate_local<E: Env + ?Sized>(env: &E, y: u32, in_swp: bool, ask: u32) -> u32 {
    let st = input(env);
    let x = st.active;
    if x == y {
        return x;
    }
    let Some(yi) = info(env, u64::from(y)) else { return 0 };
    let h = u64::from(y);
    let xi = if x != 0 { info(env, u64::from(x)) } else { None };
    if let Some(xi) = &xi {
        send(env, u64::from(x), WM_NCACTIVATE, min_bits(xi), h);
        send(env, u64::from(x), WM_ACTIVATE, WA_INACTIVE | min_bits(xi), h);
    }
    if x != 0 {
        let mut wp = WindowPos { hwnd: h, flags: ask, ..WindowPos::default() };
        send(env, h, WM_WINDOWPOSCHANGING, 0, ptr(&mut wp));
    }
    if !in_swp {
        let moved =
            matches!(env.call(Request::ZOrder { hwnd: y, after: HWND_TOP, apply: true }), Reply::Bool { value: true });
        if moved && x != 0 && yi.style & WS_VISIBLE != 0 {
            let mut done = WindowPos {
                hwnd: h,
                x: yi.window_rel.left,
                y: yi.window_rel.top,
                cx: yi.window_rel.width(),
                cy: yi.window_rel.height(),
                flags: ask | SWP_NOCLIENTSIZE | SWP_NOCLIENTMOVE,
                ..WindowPos::default()
            };
            send(env, h, WM_WINDOWPOSCHANGED, 0, ptr(&mut done));
        }
    }
    set(env, y, 0, SET_ACTIVE);
    if x == 0 {
        for t in thread_tops(env) {
            send(env, u64::from(t), WM_ACTIVATEAPP, 1, 0);
        }
    }
    let yi = info(env, h).unwrap_or(yi);
    let other = if xi.is_some() { u64::from(x) } else { 0 };
    send(env, h, WM_NCACTIVATE, min_bits(&yi), other);
    send(env, h, WM_ACTIVATE, WA_ACTIVE | min_bits(&yi), other);
    let old_focus = st.focus;
    if yi.minimized {
        if old_focus != 0 {
            send(env, u64::from(old_focus), WM_KILLFOCUS, 0, 0);
        }
        set(env, 0, 0, SET_FOCUS);
    } else {
        if old_focus != 0 && old_focus != y {
            send(env, u64::from(old_focus), WM_KILLFOCUS, h, 0);
        }
        ime_switch(env, if xi.is_some() { x } else { 0 }, y);
        set(env, 0, y, SET_FOCUS);
        send(env, h, WM_SETFOCUS, u64::from(old_focus), 0);
    }
    x
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

/// After top-level window `i` was hidden or is being destroyed. If it was
/// the active window and the call hands the activation on (`hands_on`: not
/// SetWindowPos with SWP_NOACTIVATE), another window of the thread takes
/// it ([`next_active`]) or it leaves the thread. Session 0 keeps a hidden
/// window active; only destroying it moves the activation, to the topmost
/// window of the thread that can have it (ShowWindow.yaml `hide`, `destroy`).
pub fn after_hide<E: Env + ?Sized>(env: &E, i: &WinInfo, hands_on: bool, destroying: bool) {
    if !(hands_on || destroying) || input(env).active != i.hwnd {
        return;
    }
    if screen(env).interactive {
        let next = next_active(env, i.hwnd, if destroying { GOES_DESTROYED } else { GOES_HIDDEN });
        // The active window gives way in any case: with nobody to take
        // over the thread is left without an active window.
        hand_on(env, if next == Next::Stay { Next::Away(0) } else { next }, false);
    } else if destroying {
        let next = thread_tops(env).into_iter().find(|&t| can_take(env, t, i.hwnd));
        match next {
            Some(n) => {
                activate_local(env, n, false, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
            }
            // Nobody takes it: no message at all (sw_activation).
            None => set(env, 0, 0, SET_ACTIVE | SET_FOCUS),
        }
    } else {
        // The hidden window stays the active one; the focus in it ends
        // (sw_activation: WM_KILLFOCUS(other = NULL) after the hide).
        let st = input(env);
        if st.focus != 0 && is_within(env, st.focus, i.hwnd) {
            send(env, u64::from(st.focus), WM_KILLFOCUS, 0, 0);
            set(env, 0, 0, SET_FOCUS);
        }
    }
}

/// A child was hidden: if it has the focus itself, the focus goes to its
/// parent (GetFocus.yaml `hide_focused_child`; the focus in a descendant of
/// the hidden window stays where it is).
pub fn child_hidden<E: Env + ?Sized>(env: &E, i: &WinInfo) {
    focus_to_parent(env, i.hwnd);
}

/// The focus leaves window `h`, which has it, for its parent:
/// WM_KILLFOCUS (other = the parent), WM_SETFOCUS (other = `h`).
pub fn focus_to_parent<E: Env + ?Sized>(env: &E, h: u32) {
    let st = input(env);
    if st.focus != h || h == 0 {
        return;
    }
    let Some(i) = info(env, u64::from(h)) else { return };
    if i.style & WS_CHILD == 0 {
        return;
    }
    let parent = i.parent;
    send(env, u64::from(h), WM_KILLFOCUS, u64::from(parent), 0);
    set(env, 0, parent, SET_FOCUS);
    send(env, u64::from(parent), WM_SETFOCUS, u64::from(h), 0);
}

/// The capture of `root` or of a window below it ends without a message
/// (the window is being destroyed: SetCapture.yaml `destroy`).
pub fn drop_capture_within<E: Env + ?Sized>(env: &E, root: u32) {
    let st = input(env);
    if st.capture != 0 && is_within(env, st.capture, root) {
        env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: SET_CAPTURE_QUIET });
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

/// `WM_CAPTURECHANGED` (winuser.h).
const WM_CAPTURECHANGED: u32 = 0x0215;

/// `SetCapture` (SetCapture.yaml): the previous capture window of the
/// thread's input. A window of another thread is refused (NULL, no error,
/// nothing changes); a hidden, disabled or child window can have the
/// capture; the previous capture window gets WM_CAPTURECHANGED (lParam the
/// new window) before the call returns; SetCapture(NULL) releases it
/// (lParam NULL).
pub fn set_capture<E: Env + ?Sized>(env: &E, hwnd: u64) -> u64 {
    let st = input(env);
    let set = |h: u32| {
        env.call(Request::SetInput { active: 0, focus: 0, capture: h, which: SET_CAPTURE });
    };
    if hwnd == 0 {
        if st.capture != 0 {
            set(0);
            send(env, u64::from(st.capture), WM_CAPTURECHANGED, 0, 0);
        }
        return u64::from(st.capture);
    }
    let Some(i) = info(env, hwnd) else {
        env.set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if i.tid != env.tid() {
        return 0;
    }
    set(i.hwnd);
    if st.capture != 0 {
        send(env, u64::from(st.capture), WM_CAPTURECHANGED, 0, u64::from(i.hwnd));
    }
    u64::from(st.capture)
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
    // The focus going to a child of the active window sends no activation
    // message; the thread's default IME window is put on top of the band
    // then, above hidden windows made before (GetFocus.yaml `ime_window`;
    // win32k does it with the focus change).
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

/// `SetForegroundWindow` (SetForegroundWindow.yaml;
/// GetForegroundWindow.yaml `foreground_right`).
///
/// On a window of the thread it gives the foreground and the right to
/// take it back. A window that already is the thread's active window
/// while another process has the foreground gets WM_NCACTIVATE(1, lParam
/// = itself), WM_WINDOWPOSCHANGING(0x3) and, when it moved,
/// WM_WINDOWPOSCHANGED(0x1803). On a window of another thread or process
/// the thread's windows are deactivated and that window gets the
/// foreground (the thread's process loses the right when it is another
/// process's).
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
        deactivate(env, false, true, false);
        env.call(Request::GiveForeground { hwnd: i.hwnd });
        return true;
    }
    env.call(Request::SetInput { active: 0, focus: 0, capture: 0, which: SET_FG_GRANT });
    if st.active == i.hwnd && !st.fg_mine {
        send(env, hwnd, WM_NCACTIVATE, 1 | min_bits(&i), hwnd);
        raise(env, &i, true, false);
        set(env, i.hwnd, 0, SET_ACTIVE | SET_FOREGROUND);
        return true;
    }
    activate_as(env, i.hwnd, false, true, false);
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
