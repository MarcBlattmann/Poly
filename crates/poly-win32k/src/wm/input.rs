//! Focus, activation, the foreground and input state (ADR 0131, slice B;
//! the z order and the hand-over of the activation: ADR 0176).
//!
//! Each thread's input queue has an active window, a focus window and a
//! capture window; one window of the session is the foreground window.
//! `AttachThreadInput` makes two threads share one queue's state
//! (GetFocus.yaml `attached`). The message sequences of activation run in
//! `user32` (`crate::user::focus`); this module keeps the state they
//! change. In the interactive session windows of other processes stand in
//! the z order as on the oracle's desktop (docs/AGENT_PIPELINE.md): the
//! runner's "keeper", which is in the foreground when no window of ours
//! is, a hidden window of its process, and a console window. Where
//! activation leaves our windows, the foreground goes to one of them
//! (GetActiveWindow.yaml `next_active`). What the calls do to the z order
//! and who takes the activation is decided by `crate::zorder`.

use super::*;
use crate::zorder;

/// A thread's input state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct InputState {
    pub active: u32,
    pub focus: u32,
    pub capture: u32,
}

/// `SetInput` bits.
pub const SET_ACTIVE: u8 = 1;
pub const SET_FOCUS: u8 = 2;
pub const SET_CAPTURE: u8 = 4;
/// Make the active window the foreground window too.
pub const SET_FOREGROUND: u8 = 8;
/// As [`SET_CAPTURE`], for a capture that ends with its window: no mouse
/// move is synthesized for it.
pub const SET_CAPTURE_QUIET: u8 = 16;
/// The calling process may take the foreground again (a successful
/// SetForegroundWindow on a window of its own: GetForegroundWindow.yaml
/// `foreground_right`).
pub const SET_FG_GRANT: u8 = 32;

/// The keeper's key: a thread of no process of ours.
pub(super) const KEEPER: Key = (0xFFFF_FFF0, 0xFFFF_FFF0);
/// The key of the other process's plain window (on the oracle a console
/// window): what a minimize hands the foreground to (GetActiveWindow.yaml
/// `next_active`).
pub(super) const CONSOLE: Key = (0xFFFF_FFF1, 0xFFFF_FFF1);

/// Whether a thread is one of the stand-ins for other processes' threads.
pub(super) fn foreign(key: Key) -> bool {
    key.0 >= KEEPER.0
}

impl Wm {
    /// The queue whose input state thread `key` sees.
    pub(super) fn input_key(&self, key: Key) -> Key {
        self.attached.get(&key).copied().unwrap_or(key)
    }

    pub(super) fn input_of(&self, key: Key) -> InputState {
        let k = self.input_key(key);
        self.queues.get(&k).map(|q| q.input).unwrap_or_default()
    }

    /// The thread a window belongs to.
    pub(super) fn thread_of(&self, h: u32) -> Option<Key> {
        self.get(u64::from(h)).map(|w| w.thread)
    }

    /// Whether `h` is a window of another process (the keeper, the hidden
    /// window of its process, the console window).
    pub fn is_foreign(&self, h: u32) -> bool {
        self.thread_of(h).is_some_and(foreign)
    }

    /// The foreground window, if it still exists.
    pub(super) fn foreground(&self) -> u32 {
        if self.is_window(u64::from(self.foreground)) { self.foreground } else { 0 }
    }

    pub(super) fn input_reply(&self, key: Key) -> Reply {
        let s = self.input_of(key);
        let fg = self.foreground();
        let fg_mine = fg != 0 && self.thread_of(fg).is_some_and(|t| self.input_key(t) == self.input_key(key));
        Reply::Input { active: s.active, focus: s.focus, capture: s.capture, foreground: fg, fg_mine }
    }

    pub(super) fn set_input(&mut self, key: Key, active: u32, focus: u32, capture: u32, which: u8) {
        let k = self.input_key(key);
        if which & SET_FG_GRANT != 0 {
            self.fg_lost.remove(&key.0);
        }
        if which & SET_ACTIVE != 0 {
            let q = self.queue(k);
            let changed = q.input.active != active;
            q.input.active = active;
            Self::note_active(q, active);
            if changed && active != 0 {
                self.helper_follows(k, false);
            }
        }
        if which & SET_FOCUS != 0 {
            let q = self.queue(k);
            let changed = q.input.focus != focus;
            q.input.focus = focus;
            if changed && focus != 0 {
                self.helper_follows(k, true);
            }
        }
        if which & (SET_CAPTURE | SET_CAPTURE_QUIET) != 0 {
            self.queue(k).input.capture = capture;
        }
        if which & SET_FOREGROUND != 0 {
            if active != 0 {
                self.take_foreground(k, active);
            } else {
                self.hand_foreground_over(Some(k));
            }
        }
    }

    // ----- the z order (ADR 0176, `crate::zorder`)

    /// The desktop's top-level windows from the top down, as the model
    /// sees them.
    pub(super) fn tops(&self) -> Vec<zorder::Top> {
        let Some(d) = self.get(u64::from(self.desktop)) else { return Vec::new() };
        d.children.iter().filter_map(|&h| self.top_of(h)).collect()
    }

    pub(super) fn top_of(&self, h: u32) -> Option<zorder::Top> {
        let w = self.get(u64::from(h))?;
        let mut flags = 0;
        for (on, bit) in [
            (w.ex_style & WS_EX_TOPMOST != 0, zorder::TOPMOST),
            (w.style & WS_VISIBLE != 0, zorder::VISIBLE),
            (w.style & WS_DISABLED != 0, zorder::DISABLED),
            (w.minimized || w.style & WS_MINIMIZE != 0, zorder::MINIMIZED),
            (w.ex_style & WS_EX_NOACTIVATE != 0, zorder::NOACTIVATE),
            (w.ex_style & WS_EX_TOOLWINDOW != 0, zorder::TOOL),
            (w.helper, zorder::HELPER),
        ] {
            if on {
                flags |= bit;
            }
        }
        Some(zorder::Top { hwnd: h, queue: self.input_key(w.thread), flags, owner: w.owner })
    }

    /// The window of the foreground process that a window of thread `key`
    /// which is not topmost cannot be put above: `key`'s process has lost
    /// the right to take the foreground and another process has it
    /// (GetForegroundWindow.yaml `foreground_right`; Learn, SetWindowPos:
    /// "To use SetWindowPos to bring a window to the top, the process that
    /// owns the window must have SetForegroundWindow permission").
    pub(super) fn barrier(&self, key: Key) -> Option<u32> {
        if !self.fg_lost.contains(&key.0) {
            return None;
        }
        let fg = self.foreground();
        if fg == 0 || !self.is_foreign(fg) {
            None
        } else if fg == self.keeper {
            Some(self.keeper_aux)
        } else {
            Some(fg)
        }
    }

    /// Give the desktop's children a new order.
    pub(super) fn set_order(&mut self, order: Vec<u32>) {
        let d = self.desktop;
        if let Some(dw) = self.get_mut(u64::from(d)) {
            dw.children = order;
        }
    }

    /// The default IME window of queue `k`'s thread (0: none).
    pub(super) fn helper_of(&self, k: Key) -> u32 {
        let Some(d) = self.get(u64::from(self.desktop)) else { return 0 };
        d.children
            .iter()
            .copied()
            .find(|&h| self.get(u64::from(h)).is_some_and(|w| w.helper && self.input_key(w.thread) == k))
            .unwrap_or(0)
    }

    /// The thread's default IME window follows its active window, which
    /// owns it: on a change of the focus (`to_top`) it goes on top of that
    /// window's band; when only the active window changed, directly above
    /// it if it was below (GetFocus.yaml `ime_window`).
    fn helper_follows(&mut self, k: Key, to_top: bool) {
        if !self.display.interactive {
            return;
        }
        let helper = self.helper_of(k);
        let active = self.input_of(k).active;
        if helper == 0 || active == 0 || active == helper {
            return;
        }
        let Some(a) = self.get(u64::from(active)) else { return };
        if a.parent != self.desktop {
            return;
        }
        let topmost = a.ex_style & WS_EX_TOPMOST;
        let Some(thread) = self.thread_of(helper) else { return };
        if let Some(w) = self.get_mut(u64::from(helper)) {
            w.owner = active;
            w.ex_style = (w.ex_style & !WS_EX_TOPMOST) | topmost;
        }
        let list = self.tops();
        let order = if to_top {
            zorder::helper_to_top(&list, helper, self.barrier(thread))
        } else {
            zorder::helper_above_owner(&list, helper, active)
        };
        self.set_order(order);
    }

    /// A window that owns a default IME window goes away: the helper is
    /// owned by its thread's active window, else by the thread's top
    /// window (ImmGetDefaultIMEWnd.yaml `lifetime`: it survives the
    /// thread's first window).
    pub(super) fn rehome_helpers(&mut self, gone: u32) {
        let list = self.tops();
        for t in list.iter().filter(|t| t.is(zorder::HELPER) && t.owner == gone) {
            let active = self.input_of(t.queue).active;
            let next = if active != 0 && active != gone {
                active
            } else {
                list.iter()
                    .find(|o| o.queue == t.queue && o.hwnd != gone && !o.is(zorder::HELPER))
                    .map_or(0, |o| o.hwnd)
            };
            let topmost = self.get(u64::from(next)).map_or(0, |w| w.ex_style & WS_EX_TOPMOST);
            if let Some(w) = self.get_mut(u64::from(t.hwnd)) {
                w.owner = next;
                w.ex_style = (w.ex_style & !WS_EX_TOPMOST) | topmost;
            }
            if next != 0 {
                let order = zorder::helper_above_owner(&self.tops(), t.hwnd, next);
                self.set_order(order);
            }
        }
    }

    /// Who takes the activation when window `going` of the calling thread
    /// is hidden, destroyed or minimized (`cause` 0, 1, 2): `kind` 0
    /// nothing changes, 1 window `hwnd` of the thread takes it, 2 the
    /// thread loses it (`hwnd`: the window that gets the foreground, 0: it
    /// returns to where the thread took it from).
    pub(super) fn next_active(&self, key: Key, going: u32, cause: u8) -> Reply {
        let k = self.input_key(key);
        let list = self.tops();
        let history = self.queues.get(&k).map(|q| q.activated.clone()).unwrap_or_default();
        let scene = zorder::Scene {
            list: &list,
            history: &history,
            queue: k,
            active: self.input_of(k).active,
            foreground: self.foreground(),
        };
        let cause = match cause {
            0 => zorder::Cause::Hide,
            1 => zorder::Cause::Destroy,
            _ => zorder::Cause::Minimize,
        };
        let (kind, hwnd) = match zorder::successor(&scene, going, cause) {
            zorder::Next::Stay => (0, 0),
            zorder::Next::Take(h) => (1, h),
            zorder::Next::Away(h) => (2, h.unwrap_or(0)),
        };
        Reply::Next { kind, hwnd }
    }

    // ----- the foreground

    /// The window that gets the foreground when the foreground window of
    /// queue `from` goes (`None`: its queue is gone) and no window of its
    /// thread takes the activation: a visible, enabled, activatable
    /// top-level window of another thread of ours whose thread has no
    /// active window at that moment, wherever it is in the z order
    /// (GetForegroundWindow.yaml `destroy_foreground`,
    /// `other_thread_not_taken`); else the window of another process that
    /// had the foreground before a thread of ours took it
    /// (`nobody_takes_over`: "K, or the window the thread took it from").
    fn next_foreground(&self, from: Option<Key>) -> u32 {
        let Some(d) = self.get(u64::from(self.desktop)) else { return 0 };
        let back = if self.is_window(u64::from(self.fg_return)) { self.fg_return } else { self.keeper };
        d.children
            .iter()
            .copied()
            .find(|&h| {
                self.get(u64::from(h)).is_some_and(|w| {
                    h != self.message
                        && !foreign(w.thread)
                        && !w.helper
                        && self.visible(h)
                        && w.style & (WS_DISABLED | WS_MINIMIZE) == 0
                        && !w.minimized
                        && w.ex_style & WS_EX_NOACTIVATE == 0
                        && from.is_none_or(|k| self.input_key(w.thread) != self.input_key(k))
                        && self.input_of(w.thread).active == 0
                })
            })
            .unwrap_or(back)
    }

    /// Nobody of the calling queue is in the foreground any more: another
    /// thread's window takes it with its active and focus, or it goes back
    /// to the other process's window.
    fn hand_foreground_over(&mut self, from: Option<Key>) {
        let next = self.next_foreground(from);
        if next != 0 && !self.is_foreign(next) {
            self.give_foreground(next);
        } else {
            self.foreground = next;
            self.raise_foreign(next);
        }
    }

    /// A window of another process took the foreground: as any activated
    /// window it comes to the top of its band (GetActiveWindow.yaml
    /// `z_order_moves`: the keeper "then lies above the test's topmost
    /// windows"), and with the keeper the hidden window of its process:
    /// every window of ours that is not topmost lies below one of them
    /// now, and raising it is a move in the z order (GetFocus.yaml
    /// `ime_window` (e): "the other process's windows were raised
    /// meanwhile").
    pub(super) fn raise_foreign(&mut self, h: u32) {
        if h == 0 || !self.is_foreign(h) {
            return;
        }
        let order = zorder::raise(&self.tops(), h, None);
        self.set_order(order);
        if h == self.keeper && self.keeper_aux != 0 {
            let order = zorder::raise(&self.tops(), self.keeper_aux, None);
            self.set_order(order);
        }
    }

    /// A new foreground window: the thread that had the foreground (another
    /// queue) has no active or focus window any more (GetFocus.yaml
    /// gf_threads: thread T sees NULL once A is in the foreground). A
    /// thread of ours that takes it from another process's window
    /// remembers where it goes back to, and its process may take the
    /// foreground from then on (GetForegroundWindow.yaml `foreground_right`).
    fn take_foreground(&mut self, k: Key, fg: u32) {
        let old_fg = self.foreground();
        if let Some(old) = self.thread_of(old_fg)
            && self.input_key(old) != k
            && let Some(q) = self.queues.get_mut(&self.input_key(old))
        {
            q.input.active = 0;
            q.input.focus = 0;
            q.activated.clear();
        }
        if foreign(k) {
            self.raise_foreign(fg);
        } else {
            if self.is_foreign(old_fg) {
                self.fg_return = old_fg;
            }
            self.fg_lost.remove(&k.0);
        }
        self.foreground = fg;
    }

    /// `SetForegroundWindow` of another thread's window: that thread's
    /// active and focus window, and the foreground.
    pub(super) fn give_foreground(&mut self, h: u32) {
        let Some(t) = self.thread_of(h) else { return };
        let k = self.input_key(t);
        self.take_foreground(k, h);
        let q = self.queue(k);
        q.input.active = h;
        q.input.focus = h;
        Self::note_active(q, h);
        self.helper_follows(k, true);
    }

    /// A thread of ours handed the foreground to window `h` of another
    /// process (its own SetForegroundWindow, or a minimize that found that
    /// window): its process has lost the right to take the foreground by
    /// activating a window (GetForegroundWindow.yaml `foreground_right`,
    /// `minimize_foreground`).
    pub(super) fn handed_away(&mut self, from: Key, h: u32) {
        if self.is_foreign(h) && !foreign(from) {
            self.fg_lost.insert(from.0);
        }
    }

    /// Whether thread `key` takes the foreground when one of its windows
    /// becomes its active window (created visible, shown with activation,
    /// restored, SetActiveWindow): from the start of the program, and still
    /// after its foreground window was hidden or destroyed and the
    /// foreground fell back to another process; not after the foreground
    /// was handed to another process's window by a minimize or by the
    /// thread's own SetForegroundWindow, until a SetForegroundWindow on a
    /// window of its own gives the right back (GetForegroundWindow.yaml
    /// `foreground_right`).
    pub(super) fn may_take_foreground(&self, key: Key) -> bool {
        let fg = self.foreground();
        fg == 0 || self.thread_of(fg).is_some_and(|t| t.0 == key.0) || !self.fg_lost.contains(&key.0)
    }

    /// `h` is the queue's active window now: the head of its history. A
    /// queue that has no active window any more starts a new history: the
    /// next window to be active took the activation from another thread or
    /// process and has no predecessor (GetForegroundWindow.yaml
    /// `handover_same_thread`).
    fn note_active(q: &mut Queue, h: u32) {
        if h == 0 {
            q.activated.clear();
            return;
        }
        q.activated.retain(|&x| x != h);
        q.activated.insert(0, h);
        q.activated.truncate(32);
    }

    /// The input state of thread `tid` of process `pid` (`GetGUIThreadInfo`);
    /// tid 0: the foreground thread's.
    pub(super) fn thread_input(&self, pid: u32, tid: u32) -> Reply {
        let key = if tid == 0 {
            match self.thread_of(self.foreground()) {
                Some(k) => k,
                None => return Reply::Input { active: 0, focus: 0, capture: 0, foreground: 0, fg_mine: false },
            }
        } else {
            match self.queues.keys().find(|k| k.1 == tid) {
                Some(&k) => k,
                None => (pid, tid),
            }
        };
        self.input_reply(key)
    }

    pub(super) fn attach_input(&mut self, key: Key, tid: u32, attach: bool) -> Reply {
        let Some(&other) = self.queues.keys().find(|k| k.1 == tid) else { return Reply::Bool { value: false } };
        if attach {
            self.attached.insert(key, other);
        } else {
            self.attached.remove(&key);
        }
        Reply::Bool { value: true }
    }

    /// Whether `h` is the keeper window.
    pub fn is_keeper(&self, h: u32) -> bool {
        h != 0 && h == self.keeper
    }

    /// A window going away: no thread keeps it as its active, focus or
    /// capture window.
    pub(super) fn forget_input(&mut self, h: u32) {
        for q in self.queues.values_mut() {
            if q.input.active == h {
                q.input.active = 0;
            }
            if q.input.focus == h {
                q.input.focus = 0;
            }
            if q.input.capture == h {
                q.input.capture = 0;
            }
        }
        if self.foreground == h {
            self.hand_foreground_over(None);
        }
    }

    /// The other processes' windows of the interactive session (the
    /// oracle's desktop, GetActiveWindow.yaml `building_tests`): the
    /// runner's keeper K (a 16x16 topmost popup at the screen's
    /// bottom-right corner, in the foreground when a test starts), a
    /// hidden window of the keeper's process that is raised with it, and
    /// the top visible window that is not topmost, a console window of
    /// another process (beside the keeper).
    pub(super) fn make_keeper(&mut self) {
        if self.keeper != 0 {
            return;
        }
        let Some(class) = self.get(u64::from(self.desktop)).map(|d| d.class) else { return };
        let (w, h) = self.screen;
        self.other = self.add_foreign(CONSOLE, class, WS_POPUP | WS_VISIBLE, 0, Rect::new(w - 48, h - 16, w - 16, h));
        self.keeper_aux = self.add_foreign(KEEPER, class, WS_POPUP, 0, Rect::new(0, 0, 0, 0));
        self.keeper =
            self.add_foreign(KEEPER, class, WS_POPUP | WS_VISIBLE, WS_EX_TOPMOST, Rect::new(w - 16, h - 16, w, h));
        self.foreground = self.keeper;
        self.fg_return = self.keeper;
    }

    /// A window of another process, on top of its band.
    fn add_foreign(&mut self, key: Key, class: usize, style: u32, ex: u32, rect: Rect) -> u32 {
        let mut k = Window::new(key, class, style, rect);
        k.ex_style = ex;
        k.parent = self.desktop;
        let hwnd = self.alloc(k);
        let at = self.band_start(self.desktop, ex & WS_EX_TOPMOST != 0);
        let d = self.desktop;
        if let Some(dw) = self.get_mut(u64::from(d)) {
            dw.children.insert(at, hwnd);
        }
        hwnd
    }
}
