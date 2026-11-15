//! Carets (spec-win-apps-3: `CreateCaret`, `DestroyCaret`, `ShowCaret`, `HideCaret`,
//! `SetCaretPos`, `GetCaretPos`).
//!
//! A caret belongs to an input queue: one per queue, owned by a window of the queue's thread,
//! with a size, a hide count and a position. The position outlives the caret (the next caret
//! of the queue starts there). Everything here is plain data and rules; the window checks
//! (does the window exist, is it the desktop, is it the caller's) are the window manager's
//! (`crate::wm`), which calls [`CaretQueue`].
//!
//! The rules are those of the sheets `specs/windows/user32.dll/*Caret*.yaml`:
//! - a new caret is hidden (hide count 1); `ShowCaret` lowers the count by one and never
//!   below 0 (a shown caret ignores further calls), `HideCaret` raises it;
//! - the rectangle `GetGUIThreadInfo` reports is `(x, y, x + width, y + height)`, with wrapping
//!   arithmetic (a caret at `0x7FFFFFFF` has a negative right edge);
//! - destroying the owner window destroys the caret, not the position.

/// `CreateCaret`: the window, the size, and what `user32` made of the bitmap argument.
pub const OP_CREATE: u8 = 1;
pub const OP_DESTROY: u8 = 2;
pub const OP_SHOW: u8 = 3;
pub const OP_HIDE: u8 = 4;
pub const OP_SET_POS: u8 = 5;
/// `GetCaretPos` of the calling queue, and `GetGUIThreadInfo`'s caret of queue of thread `tid`.
pub const OP_GET: u8 = 6;
pub const OP_QUERY: u8 = 7;

/// The error code a reply carries when the call fails and leaves the last error as it was.
pub const UNCHANGED: u32 = 0xFFFF_FFFF;

/// The caret of one queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caret {
    pub hwnd: u32,
    pub width: i32,
    pub height: i32,
    /// 0: shown. A new caret starts at 1.
    pub hide: u32,
}

/// What an input queue has of a caret: its caret, if any, and the position it was last at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaretQueue {
    pub caret: Option<Caret>,
    pub x: i32,
    pub y: i32,
}

/// What a caret call tells (a reply of `Request::Caret`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The owner window (0: none).
    pub hwnd: u32,
    /// The size and the position of the caret rectangle's corner.
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub shown: bool,
}

impl CaretQueue {
    pub fn snapshot(&self) -> Snapshot {
        match self.caret {
            Some(c) => Snapshot { hwnd: c.hwnd, width: c.width, height: c.height, x: self.x, y: self.y, shown: c.hide == 0 },
            None => Snapshot { x: self.x, y: self.y, ..Snapshot::default() },
        }
    }

    /// `CreateCaret`: replaces the queue's caret with a hidden one at the queue's position.
    pub fn create(&mut self, hwnd: u32, width: i32, height: i32) {
        self.caret = Some(Caret { hwnd, width, height, hide: 1 });
    }

    /// `DestroyCaret`: whether there was a caret.
    pub fn destroy(&mut self) -> bool {
        self.caret.take().is_some()
    }

    /// `ShowCaret`: lowers the hide count, never below 0. Whether there is a caret.
    pub fn show(&mut self) -> bool {
        match self.caret.as_mut() {
            Some(c) => {
                c.hide = c.hide.saturating_sub(1);
                true
            }
            None => false,
        }
    }

    /// `HideCaret`: raises the hide count. Whether there is a caret.
    pub fn hide(&mut self) -> bool {
        match self.caret.as_mut() {
            Some(c) => {
                c.hide = c.hide.saturating_add(1);
                true
            }
            None => false,
        }
    }

    /// `SetCaretPos`: stores the position as given. Whether there is a caret.
    pub fn set_pos(&mut self, x: i32, y: i32) -> bool {
        if self.caret.is_none() {
            return false;
        }
        self.x = x;
        self.y = y;
        true
    }

    /// The window `hwnd` is gone: its caret goes, the position stays.
    pub fn window_gone(&mut self, hwnd: u32) {
        if self.caret.is_some_and(|c| c.hwnd == hwnd) {
            self.caret = None;
        }
    }
}

impl Snapshot {
    /// The caret rectangle `GetGUIThreadInfo` reports: left, top, right, bottom (all zeros without a caret).
    pub fn rect(&self) -> [i32; 4] {
        if self.hwnd == 0 {
            return [0; 4];
        }
        [self.x, self.y, self.x.wrapping_add(self.width), self.y.wrapping_add(self.height)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_caret_is_hidden_and_starts_at_the_old_position() {
        let mut q = CaretQueue::default();
        assert!(!q.set_pos(20, 30), "no caret yet");
        q.create(7, 2, 16);
        assert!(!q.snapshot().shown);
        assert!(q.set_pos(20, 30));
        assert!(q.destroy());
        assert!(!q.destroy());
        assert_eq!((q.snapshot().x, q.snapshot().y), (20, 30));
        q.create(8, 1, 1);
        assert_eq!(q.snapshot().rect(), [20, 30, 21, 31]);
    }

    #[test]
    fn the_hide_count_never_goes_below_shown() {
        let mut q = CaretQueue::default();
        q.create(1, 2, 2);
        q.show();
        assert!(q.snapshot().shown);
        q.show();
        q.show();
        q.hide();
        assert!(!q.snapshot().shown, "extra ShowCaret calls are lost");
        q.show();
        assert!(q.snapshot().shown);
        q.hide();
        q.hide();
        q.hide();
        q.show();
        q.show();
        assert!(!q.snapshot().shown);
        q.show();
        assert!(q.snapshot().shown);
    }

    #[test]
    fn the_rectangle_wraps_and_the_window_takes_the_caret_along() {
        let mut q = CaretQueue::default();
        q.create(5, 4, 4);
        q.set_pos(0x7FFF_FFFF, i32::MIN);
        assert_eq!(q.snapshot().rect(), [0x7FFF_FFFF, i32::MIN, i32::MIN + 3, i32::MIN + 4]);
        q.window_gone(6);
        assert!(q.caret.is_some());
        q.window_gone(5);
        assert_eq!(q.snapshot().rect(), [0; 4]);
        assert_eq!(q.snapshot().x, 0x7FFF_FFFF, "the position stays");
    }
}
