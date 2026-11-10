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
/// As [`SET_CAPTURE`], for a capture that ends with its window: no mouse
/// move is synthesized for it.
pub const SET_CAPTURE_QUIET: u8 = 16;
/// The calling thread destroyed its foreground window: the next window it
/// creates may take the foreground back.
pub const SET_FG_GRANT: u8 = 32;

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
            Self::note_active(q, active);
        }
        if which & SET_FG_GRANT != 0 {
            self.fg_grant.insert(key);
        }
        let q = self.queue(k);
        if which & SET_FOCUS != 0 {
            q.input.focus = focus;
        }
        if which & (SET_CAPTURE | SET_CAPTURE_QUIET) != 0 {
            q.input.capture = capture;
        }
        if which & SET_FOREGROUND != 0 {
            if active != 0 {
                self.take_foreground(k, active);
            } else {
                self.hand_foreground_over(Some(k));
            }
        }
    }

    /// The window that gets the foreground when the foreground window of
    /// queue `from` goes (`None`: its queue is gone): the topmost visible,
    /// enabled, activatable top-level window of another queue, which is
    /// another thread of our process or the keeper (GetForegroundWindow.yaml
    /// `destroy_foreground`; GetActiveWindow.yaml `hide_active`: not a
    /// window of the same thread, B).
    fn next_foreground(&self, from: Option<Key>) -> u32 {
        let Some(d) = self.get(u64::from(self.desktop)) else { return 0 };
        // A window of another thread of ours takes it, wherever it is in the
        // z order, if its thread has no active window at that moment
        // (GetForegroundWindow.yaml `destroy_foreground`,
        // `other_thread_not_taken`); else the window of another process.
        d.children
            .iter()
            .copied()
            .find(|&h| {
                self.get(u64::from(h)).is_some_and(|w| {
                    h != self.message
                        && h != self.keeper
                        && self.visible(h)
                        && w.style & (WS_DISABLED | WS_MINIMIZE) == 0
                        && !w.minimized
                        && w.ex_style & WS_EX_NOACTIVATE == 0
                        && from.is_none_or(|k| self.input_key(w.thread) != self.input_key(k))
                        && self.input_of(w.thread).active == 0
                })
            })
            .unwrap_or(self.keeper)
    }

    /// Nobody of the calling queue is in the foreground any more: another
    /// thread's window takes it with its active and focus (or the keeper).
    fn hand_foreground_over(&mut self, from: Option<Key>) {
        let next = self.next_foreground(from);
        if next != 0 && next != self.keeper {
            self.give_foreground(next);
        } else {
            self.foreground = self.keeper;
            self.raise_keeper();
        }
    }

    /// The keeper took the foreground: as any activated window it comes to the
    /// top of its band, above our topmost windows (GetActiveWindow.yaml
    /// `show_again`: "a window of another process (the runner keeper) was
    /// activated and raised above A meanwhile"; GetFocus.yaml `restore`).
    pub(super) fn raise_keeper(&mut self) {
        let (d, k) = (self.desktop, self.keeper);
        if k == 0 {
            return;
        }
        // Every window of ours lies below the other process's window now:
        // raising one is a move in the z order (GetFocus.yaml `show`:
        // 0x10001843 for a window shown again after the activation left).
        let ours: Vec<u32> = self.get(u64::from(d)).map(|w| w.children.clone()).unwrap_or_default();
        for h in ours {
            if h != k
                && let Some(w) = self.get_mut(u64::from(h))
            {
                w.below_others = true;
            }
        }
        if let Some(dw) = self.get_mut(u64::from(d))
            && let Some(i) = dw.children.iter().position(|&c| c == k)
            && i != 0
        {
            dw.children.remove(i);
            dw.children.insert(0, k);
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
            q.activated.clear();
        }
        self.foreground = fg;
        self.fg_seen.insert(k.0);
        self.fg_grant.remove(&k);
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
    }

    /// Whether thread `key` takes the foreground when it creates a visible
    /// window (Learn: SetForegroundWindow's conditions, as far as the
    /// sheets' tests show them: gaw_zhide, gaw_leave_a, gfw_xthread): its
    /// process has the foreground window, or never had one yet (it was
    /// started by the foreground process), or the thread destroyed its
    /// window while that was the foreground window and has not had the
    /// foreground since. A thread that lost the foreground otherwise (its
    /// window was hidden or minimized), and a new thread of a process that
    /// lost it, get a window that is active without the foreground:
    /// SetForegroundWindow, or ShowWindow of a hidden window, gets it back.
    pub(super) fn may_take_foreground(&self, key: Key) -> bool {
        let fg = self.foreground();
        fg == 0
            || self.thread_of(fg).is_some_and(|t| t.0 == key.0)
            || !self.fg_seen.contains(&key.0)
            || self.fg_grant.contains(&key)
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
