//! Focus, activation, the foreground and input state (ADR 0131, slice B).
//!
//! Each thread's input queue has an active window, a focus window and a
//! capture window; one window of the session is the foreground window.
//! `AttachThreadInput` makes two threads share one queue's state
//! (GetFocus.yaml `attached`). The message sequences of activation run in
//! `user32` (`crate::user::focus`); this module keeps the state they
//! change. In the interactive session a window of another process, the
//! oracle's runner "keeper" (docs/AGENT_PIPELINE.md), is in the
//! foreground when no window of ours is: where activation leaves our
//! windows, the foreground goes to it (GetActiveWindow.yaml).

use super::*;

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

/// The keeper's key: a thread of no process of ours.
pub(super) const KEEPER: Key = (0xFFFF_FFF0, 0xFFFF_FFF0);

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
        let q = self.queue(k);
        if which & SET_ACTIVE != 0 {
            q.input.active = active;
        }
        if which & SET_FOCUS != 0 {
            q.input.focus = focus;
        }
        if which & SET_CAPTURE != 0 {
            q.input.capture = capture;
        }
        if which & SET_FOREGROUND != 0 {
            self.take_foreground(k, if active != 0 { active } else { self.keeper });
        }
    }

    /// A new foreground window: the thread that had the foreground (another
    /// queue) has no active or focus window any more (GetFocus.yaml
    /// gf_threads: thread T sees NULL once A is in the foreground).
    fn take_foreground(&mut self, k: Key, fg: u32) {
        if let Some(old) = self.thread_of(self.foreground())
            && self.input_key(old) != k
            && let Some(q) = self.queues.get_mut(&self.input_key(old))
        {
            q.input.active = 0;
            q.input.focus = 0;
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
            self.foreground = self.keeper;
        }
    }

    /// The interactive session's keeper window (a 16x16 topmost popup of
    /// another process at the screen's bottom-right corner), in the
    /// foreground.
    pub(super) fn make_keeper(&mut self) {
        if self.keeper != 0 {
            return;
        }
        let Some(class) = self.get(u64::from(self.desktop)).map(|d| d.class) else { return };
        let (w, h) = self.screen;
        let mut k = Window::new(KEEPER, class, WS_POPUP | WS_VISIBLE, Rect::new(w - 16, h - 16, w, h));
        k.ex_style = WS_EX_TOPMOST;
        k.parent = self.desktop;
        let hwnd = self.alloc(k);
        let d = self.desktop;
        if let Some(dw) = self.get_mut(u64::from(d)) {
            dw.children.insert(0, hwnd);
        }
        self.keeper = hwnd;
        self.foreground = hwnd;
    }
}
