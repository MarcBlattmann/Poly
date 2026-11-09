//! Raw input (ADR 0131, slice B): SendInput's keyboard and mouse events
//! (and later the desktop link's), the physical ("async") key state, each
//! thread's key state as of the input messages it retrieved, and the
//! input messages waiting in a thread's queue (SendInput.yaml,
//! GetKeyState.yaml, GetKeyboardState.yaml, GetAsyncKeyState.yaml).
//!
//! A keyboard event goes to the foreground thread: its focus window, or its
//! active window as a system key when no window has the focus. A mouse
//! event moves the cursor and goes to the capture window or the window
//! under the cursor. Input messages come after posted messages and before
//! WM_PAINT; a key-down not yet retrieved is coalesced with a repeat of the
//! same key, a mouse move with the next move, a wheel turn with the next.

use super::*;
use crate::kbd;
use crate::proto::InputEvent;

pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSKEYUP: u32 = 0x0105;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
/// The double-click messages (winuser.h): a button-down plus 2.
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONDBLCLK: u32 = 0x0206;
pub const WM_MBUTTONDBLCLK: u32 = 0x0209;
/// `CS_DBLCLKS` (winuser.h): the class wants double-click messages.
const CS_DBLCLKS: u32 = 0x0008;
/// `GetDoubleClickTime`, and `SM_CXDOUBLECLK` x `SM_CYDOUBLECLK`: the
/// rectangle centred on the first click that a second one must fall in.
const DOUBLE_CLICK_MS: u64 = 500;
const DOUBLE_CLICK_SIZE: (i32, i32) = (4, 4);
/// `QS_KEY`, `QS_MOUSEMOVE`, `QS_MOUSEBUTTON` (winuser.h).
pub const QS_KEY: u32 = 0x0001;
pub const QS_MOUSEMOVE: u32 = 0x0002;
pub const QS_MOUSEBUTTON: u32 = 0x0004;
/// `KEYEVENTF_*`, `MOUSEEVENTF_*` (winuser.h).
const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_UNICODE: u32 = 0x0004;
const KEYEVENTF_SCANCODE: u32 = 0x0008;
const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;
const VK_LBUTTON: usize = 0x01;
const VK_RBUTTON: usize = 0x02;
const VK_MBUTTON: usize = 0x04;
const VK_PACKET: u32 = 0xE7;
/// `MK_*` (winuser.h).
const MK_LBUTTON: u64 = 0x0001;
const MK_RBUTTON: u64 = 0x0002;
const MK_SHIFT: u64 = 0x0004;
const MK_CONTROL: u64 = 0x0008;
const MK_MBUTTON: u64 = 0x0010;
/// `Msg::private` of a mouse move a capture release synthesized.
const SYNTHETIC_MOVE: u32 = 0x4D4F_5645;

/// The key-state bytes of a session, a thread.
pub(super) fn blank_keys() -> Vec<u8> {
    alloc::vec![0u8; 256]
}

fn is_key(m: u32) -> bool {
    (WM_KEYDOWN..=0x0109).contains(&m)
}

fn is_mouse(m: u32) -> bool {
    (WM_MOUSEMOVE..=0x020E).contains(&m)
}

/// The side-specific code of a modifier from its scan code.
fn side_of(vk: u32, scan: u32, ext: bool) -> Option<usize> {
    match vk {
        kbd::VK_SHIFT => Some(if scan == 0x36 { kbd::VK_RSHIFT } else { kbd::VK_LSHIFT } as usize),
        kbd::VK_CONTROL => Some(if ext { kbd::VK_RCONTROL } else { kbd::VK_LCONTROL } as usize),
        kbd::VK_MENU => Some(if ext { kbd::VK_RMENU } else { kbd::VK_LMENU } as usize),
        _ => None,
    }
}

/// The generic code of a side-specific one.
fn generic_of(vk: u32) -> u32 {
    match vk {
        kbd::VK_LSHIFT | kbd::VK_RSHIFT => kbd::VK_SHIFT,
        kbd::VK_LCONTROL | kbd::VK_RCONTROL => kbd::VK_CONTROL,
        kbd::VK_LMENU | kbd::VK_RMENU => kbd::VK_MENU,
        v => v,
    }
}

/// A key going down or up in a key-state array (and its generic code;
/// the toggle bit flips at each press).
fn apply_key(keys: &mut [u8], vk: usize, side: Option<usize>, down: bool) {
    let set = |keys: &mut [u8], k: usize| {
        if k == 0 || k > 0xFF {
            return;
        }
        if down {
            if keys[k] & 0x80 == 0 {
                keys[k] ^= 0x01;
            }
            keys[k] |= 0x80;
        } else {
            keys[k] &= !0x80;
        }
    };
    set(keys, vk);
    if let Some(s) = side {
        set(keys, s);
        // The generic code stays down while either side is.
        if !down {
            let (a, b) = match vk as u32 {
                kbd::VK_SHIFT => (kbd::VK_LSHIFT, kbd::VK_RSHIFT),
                kbd::VK_CONTROL => (kbd::VK_LCONTROL, kbd::VK_RCONTROL),
                _ => (kbd::VK_LMENU, kbd::VK_RMENU),
            };
            if keys[a as usize] & 0x80 != 0 || keys[b as usize] & 0x80 != 0 {
                keys[vk] |= 0x80;
            }
        }
    }
}

impl Wm {
    /// The key state thread `key` sees (its own after SetKeyboardState,
    /// else the session's synchronous state).
    pub(super) fn keys_of(&self, key: Key) -> Vec<u8> {
        let k = self.input_key(key);
        match self.queues.get(&k).and_then(|q| q.keys.clone()) {
            Some(v) => v,
            None => self.sync_keys.clone(),
        }
    }

    pub(super) fn set_keys(&mut self, key: Key, keys: Vec<u8>) {
        let k = self.input_key(key);
        let mut v = keys;
        v.resize(256, 0);
        self.queue(k).keys = Some(v);
    }

    /// GetAsyncKeyState: 0x8000 while down, the low bit at the first call
    /// after a press.
    pub(super) fn async_key(&mut self, vk: u32) -> u16 {
        let i = vk as usize;
        if i == 0 || i > 0xFE {
            return 0;
        }
        let down = self.async_keys[i] & 0x80 != 0;
        let new = core::mem::take(&mut self.async_new[i]);
        (if down { 0x8000 } else { 0 }) | u16::from(new && down)
    }

    /// SendInput: the events, in order; the number inserted.
    pub(super) fn send_input(&mut self, ctx: Ctx, events: &[InputEvent], fx: &mut Effects) -> u32 {
        for e in events {
            match e.kind {
                0 => self.mouse_event(ctx, e, fx),
                1 => self.key_event(ctx, e, fx),
                _ => {}
            }
        }
        events.len() as u32
    }

    fn key_event(&mut self, ctx: Ctx, e: &InputEvent, fx: &mut Effects) {
        let up = e.flags & KEYEVENTF_KEYUP != 0;
        let ext = e.flags & KEYEVENTF_EXTENDEDKEY != 0;
        let (vk, scan, unit) = if e.flags & KEYEVENTF_UNICODE != 0 {
            (VK_PACKET, 0u32, e.b as u32 & 0xFFFF)
        } else if e.flags & KEYEVENTF_SCANCODE != 0 {
            let scan = e.b as u32 & 0xFF;
            let code = if ext { 0xE000 | scan } else { scan };
            (generic_of(kbd::map_virtual_key(code, kbd::MAPVK_VSC_TO_VK_EX)), scan, 0)
        } else {
            let vk = e.a as u32 & 0xFF;
            let scan = e.b as u32 & 0xFF;
            // A side code given: the generic code with that side's scan.
            let (vk, scan) = match vk {
                kbd::VK_LSHIFT => (kbd::VK_SHIFT, if scan == 0 { 0x2A } else { scan }),
                kbd::VK_RSHIFT => (kbd::VK_SHIFT, if scan == 0 { 0x36 } else { scan }),
                kbd::VK_LCONTROL => (kbd::VK_CONTROL, if scan == 0 { 0x1D } else { scan }),
                kbd::VK_RCONTROL => (kbd::VK_CONTROL, if scan == 0 { 0x1D } else { scan }),
                v => (v, scan),
            };
            (vk, scan, 0)
        };
        let ext = ext || (e.flags & KEYEVENTF_SCANCODE == 0 && e.a as u32 & 0xFF == kbd::VK_RCONTROL);
        let i = vk as usize;
        let was_down = self.async_keys[i] & 0x80 != 0;
        let side = side_of(vk, scan, ext);
        let mut a = core::mem::take(&mut self.async_keys);
        apply_key(&mut a, i, side, !up);
        self.async_keys = a;
        if !up {
            self.async_new[i] = true;
            if let Some(s) = side {
                self.async_new[s] = true;
            }
        }
        // To the foreground thread's focus window, or its active window as
        // a system key.
        let fg = self.foreground();
        let Some(thread) = self.thread_of(fg) else { return };
        if thread.0 == 0 || thread == super::input::KEEPER {
            return;
        }
        let st = self.input_of(thread);
        let alt = self.async_keys[kbd::VK_MENU as usize] & 0x80 != 0;
        let (target, sys) = if st.focus != 0 { (st.focus, alt) } else { (st.active.max(fg), true) };
        let msg = match (up, sys) {
            (false, false) => WM_KEYDOWN,
            (true, false) => WM_KEYUP,
            (false, true) => WM_SYSKEYDOWN,
            (true, true) => WM_SYSKEYUP,
        };
        let mut l = 1u64 | (u64::from(scan) << 16) | if ext { 1 << 24 } else { 0 };
        if up {
            // The previous state: down unless an earlier key-up of the same
            // key (a VK_PACKET pair) released it already (SendInput.yaml).
            l |= 0x8000_0000 | if was_down { 1 << 30 } else { 0 };
        } else if was_down {
            l |= 1 << 30;
        }
        if alt && sys {
            l |= 1 << 29;
        }
        let key = self.input_key(thread);
        let cursor = self.cursor;
        let q = self.queue(key);
        // A key-down not yet retrieved counts a repeat of the same key.
        if !up
            && vk != VK_PACKET
            && let Some(last) = q.raw.back_mut()
            && last.msg == msg
            && last.w == u64::from(vk)
            && last.hwnd == target
        {
            let count = (last.l & 0xFFFF) + 1;
            last.l = (last.l & !0xFFFF) | (count & 0xFFFF);
            return;
        }
        q.raw.push_back(Msg {
            hwnd: target,
            msg,
            w: u64::from(vk),
            l,
            time: ctx.now as u32,
            x: cursor.0,
            y: cursor.1,
            private: unit,
        });
        q.changed |= QS_KEY;
        fx.wake.push(key);
    }

    fn mouse_event(&mut self, ctx: Ctx, e: &InputEvent, fx: &mut Effects) {
        let (w, h) = self.screen;
        let old = self.cursor;
        if e.flags & MOUSEEVENTF_MOVE != 0 {
            self.cursor = if e.flags & MOUSEEVENTF_ABSOLUTE != 0 {
                (((i64::from(e.a) * i64::from(w)) >> 16) as i32, ((i64::from(e.b) * i64::from(h)) >> 16) as i32)
            } else {
                (old.0 + e.a, old.1 + e.b)
            };
            self.cursor = (self.cursor.0.clamp(0, w - 1), self.cursor.1.clamp(0, h - 1));
        }
        let buttons: [(u32, usize, bool, u32); 6] = [
            (MOUSEEVENTF_LEFTDOWN, VK_LBUTTON, true, WM_LBUTTONDOWN),
            (MOUSEEVENTF_LEFTUP, VK_LBUTTON, false, WM_LBUTTONUP),
            (MOUSEEVENTF_RIGHTDOWN, VK_RBUTTON, true, WM_RBUTTONDOWN),
            (MOUSEEVENTF_RIGHTUP, VK_RBUTTON, false, WM_RBUTTONUP),
            (MOUSEEVENTF_MIDDLEDOWN, VK_MBUTTON, true, WM_MBUTTONDOWN),
            (MOUSEEVENTF_MIDDLEUP, VK_MBUTTON, false, WM_MBUTTONUP),
        ];
        let mut msgs: Vec<(u32, u64)> = Vec::new();
        if e.flags & MOUSEEVENTF_MOVE != 0 && self.cursor != old {
            msgs.push((WM_MOUSEMOVE, 0));
        }
        for (flag, vk, down, msg) in buttons {
            if e.flags & flag != 0 {
                let mut a = core::mem::take(&mut self.async_keys);
                apply_key(&mut a, vk, None, down);
                self.async_keys = a;
                if down {
                    self.async_new[vk] = true;
                }
                msgs.push((msg, 0));
            }
        }
        if e.flags & MOUSEEVENTF_WHEEL != 0 {
            msgs.push((WM_MOUSEWHEEL, u64::from(e.data as u16) << 16));
        }
        for (msg, extra) in msgs {
            self.queue_mouse(ctx, msg, extra, fx);
        }
    }

    /// TrackMouseEvent (TrackMouseEvent.yaml): TME_LEAVE is kept until the
    /// cursor leaves the window (WM_MOUSELEAVE, posted at once when it is
    /// not over it); TME_QUERY answers flags | window << 32.
    pub(super) fn track_mouse(&mut self, ctx: Ctx, hwnd: u32, flags: u32, hover: u32, fx: &mut Effects) -> Reply {
        const TME_LEAVE: u32 = 0x2;
        const TME_QUERY: u32 = 0x4000_0000;
        const TME_CANCEL: u32 = 0x8000_0000;
        const TME_HOVER: u32 = 0x1;
        if flags & TME_QUERY != 0 {
            let leave = self.tracking.first().copied();
            let mut fl = leave.map_or(0, |t| t.1);
            let mut win = leave.map_or(0, |t| t.0);
            let mut ms = 0;
            if let Some((hw, h, _)) = self.hover {
                fl |= TME_HOVER;
                win = hw;
                ms = h;
            }
            return Reply::Value {
                value: u64::from(fl & 0xFFFF) | (u64::from(ms & 0xFFFF) << 16) | (u64::from(win) << 32),
            };
        }
        if flags & TME_CANCEL != 0 {
            // TME_CANCEL removes the kinds of tracking named with it
            // (TrackMouseEvent.yaml `cancel`; Learn: "the caller should also
            // specify the type of tracking that it wants to cancel").
            if flags & TME_HOVER != 0 && self.hover.is_some_and(|h| h.0 == hwnd) {
                self.hover = None;
            }
            if flags & TME_LEAVE != 0 {
                self.tracking.retain(|t| t.0 != hwnd);
            }
            return Reply::Ok {};
        }
        if flags & TME_HOVER != 0 {
            self.hover = Some((hwnd, if hover == u32::MAX { 400 } else { hover }, ctx.now));
        }
        if flags & TME_LEAVE != 0 {
            let (x, y) = self.cursor;
            let under = self.window_from_point(x, y, false);
            if under != hwnd {
                self.post_leave(ctx, hwnd, fx);
            } else if !self.tracking.iter().any(|t| t.0 == hwnd) {
                self.tracking.push((hwnd, TME_LEAVE));
            }
        }
        Reply::Ok {}
    }

    fn post_leave(&mut self, ctx: Ctx, hwnd: u32, fx: &mut Effects) {
        const WM_MOUSELEAVE: u32 = 0x02A3;
        if let Some(t) = self.thread_of(hwnd) {
            let cursor = self.cursor;
            let q = self.queue(t);
            q.posted.push_back(Msg {
                hwnd,
                msg: WM_MOUSELEAVE,
                w: 0,
                l: 0,
                time: ctx.now as u32,
                x: cursor.0,
                y: cursor.1,
                private: 0,
            });
            q.changed |= QS_POSTMESSAGE;
            fx.wake.push(t);
        }
    }

    /// The desktop link's pointer at screen point (x, y) (ADR 0131): a move.
    pub fn link_pointer(&mut self, x: i32, y: i32, fx: &mut Effects) {
        let (w, h) = self.screen;
        let ctx = Ctx { pid: 0, tid: 0, now: self.now };
        let e = InputEvent {
            kind: 0,
            a: ((i64::from(x) * 65536 + i64::from(w) - 1) / i64::from(w.max(1))) as i32,
            b: ((i64::from(y) * 65536 + i64::from(h) - 1) / i64::from(h.max(1))) as i32,
            data: 0,
            flags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
        };
        self.mouse_event(ctx, &e, fx);
    }

    /// The desktop link's button (evdev code: BTN_LEFT 0x110, BTN_RIGHT
    /// 0x111, BTN_MIDDLE 0x112).
    pub fn link_button(&mut self, button: u16, pressed: bool, fx: &mut Effects) {
        let flags = match (button, pressed) {
            (0x110, true) => MOUSEEVENTF_LEFTDOWN,
            (0x110, false) => MOUSEEVENTF_LEFTUP,
            (0x111, true) => MOUSEEVENTF_RIGHTDOWN,
            (0x111, false) => MOUSEEVENTF_RIGHTUP,
            (0x112, true) => MOUSEEVENTF_MIDDLEDOWN,
            (0x112, false) => MOUSEEVENTF_MIDDLEUP,
            _ => return,
        };
        let ctx = Ctx { pid: 0, tid: 0, now: self.now };
        self.mouse_event(ctx, &InputEvent { kind: 0, a: 0, b: 0, data: 0, flags }, fx);
    }

    /// The desktop link's key (an evdev code, which for the main block is
    /// the PC set 1 scan code; the cursor and editing keys are extended).
    pub fn link_key(&mut self, code: u16, pressed: bool, fx: &mut Effects) {
        let (scan, ext) = match code {
            96 => (0x1C, true),  // KEY_KPENTER
            97 => (0x1D, true),  // KEY_RIGHTCTRL
            98 => (0x35, true),  // KEY_KPSLASH
            100 => (0x38, true), // KEY_RIGHTALT
            102 => (0x47, true), // KEY_HOME
            103 => (0x48, true), // KEY_UP
            104 => (0x49, true), // KEY_PAGEUP
            105 => (0x4B, true), // KEY_LEFT
            106 => (0x4D, true), // KEY_RIGHT
            107 => (0x4F, true), // KEY_END
            108 => (0x50, true), // KEY_DOWN
            109 => (0x51, true), // KEY_PAGEDOWN
            110 => (0x52, true), // KEY_INSERT
            111 => (0x53, true), // KEY_DELETE
            125 => (0x5B, true), // KEY_LEFTMETA
            126 => (0x5C, true), // KEY_RIGHTMETA
            c if c < 0x59 => (u32::from(c), false),
            _ => return,
        };
        let mut flags = KEYEVENTF_SCANCODE;
        if ext {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !pressed {
            flags |= KEYEVENTF_KEYUP;
        }
        let ctx = Ctx { pid: 0, tid: 0, now: self.now };
        self.key_event(ctx, &InputEvent { kind: 1, a: 0, b: scan as i32, data: 0, flags }, fx);
    }

    /// The MK_* keys and buttons down now.
    fn mouse_keys(&self) -> u64 {
        let k = &self.async_keys;
        let b = |vk: usize, mk: u64| if k[vk] & 0x80 != 0 { mk } else { 0 };
        b(VK_LBUTTON, MK_LBUTTON)
            | b(VK_RBUTTON, MK_RBUTTON)
            | b(kbd::VK_SHIFT as usize, MK_SHIFT)
            | b(kbd::VK_CONTROL as usize, MK_CONTROL)
            | b(VK_MBUTTON, MK_MBUTTON)
    }

    pub(super) fn queue_mouse(&mut self, ctx: Ctx, msg: u32, extra: u64, fx: &mut Effects) {
        let (x, y) = self.cursor;
        let target = self.mouse_target(x, y);
        if target == 0 {
            // The cursor is over no window: every tracked window was left
            // (TrackMouseEvent.yaml `leave`), the hover is over.
            if msg == WM_MOUSEMOVE {
                self.end_tracking(ctx, 0, (x, y), fx);
            }
            return;
        }
        let Some(thread) = self.thread_of(target) else { return };
        if thread.0 == 0 || thread == super::input::KEEPER {
            // The desktop, or a window of no process of ours.
            if msg == WM_MOUSEMOVE {
                self.end_tracking(ctx, 0, (x, y), fx);
            }
            return;
        }
        let keys = self.mouse_keys();
        let key = self.input_key(thread);
        let msg = self.double_click(ctx, target, msg, (x, y));
        let l = u64::from(x as u16) | (u64::from(y as u16) << 16);
        let q = self.queue(key);
        // A move or wheel turn not yet retrieved is replaced or summed.
        if let Some(last) = q.raw.back_mut()
            && last.hwnd == target
            && last.msg == msg
        {
            if msg == WM_MOUSEMOVE {
                last.l = l;
                last.w = keys;
                last.x = x;
                last.y = y;
                return;
            }
            if msg == WM_MOUSEWHEEL {
                let sum = (last.w >> 16) as u16 as i16 as i32 + (extra >> 16) as u16 as i16 as i32;
                last.w = (u64::from(sum as u16) << 16) | keys;
                return;
            }
        }
        q.raw.push_back(Msg { hwnd: target, msg, w: keys | extra, l, time: ctx.now as u32, x, y, private: 0 });
        q.changed |= if msg == WM_MOUSEMOVE { QS_MOUSEMOVE } else { QS_MOUSEBUTTON };
        fx.wake.push(key);
        // Tracked windows the cursor left get WM_MOUSELEAVE after the move's
        // messages (TrackMouseEvent.yaml `leave`).
        if msg == WM_MOUSEMOVE {
            self.end_tracking(ctx, target, (x, y), fx);
        }
    }

    /// A move of the cursor to `at`, over window `target` (0: none): the
    /// windows whose client area it left get WM_MOUSELEAVE after the move's
    /// messages (leaving for the window's own frame counts: the tracking is
    /// of the client area; TrackMouseEvent.yaml `leave`, `nonclient`), and a
    /// hover that was not over the new place ends.
    fn end_tracking(&mut self, ctx: Ctx, target: u32, at: (i32, i32), fx: &mut Effects) {
        let outside = |wm: &Wm, h: u32| h != target || !wm.in_client(h, at);
        if !self.tracking.is_empty() {
            let left: Vec<u32> = self.tracking.iter().filter(|t| outside(self, t.0)).map(|t| t.0).collect();
            self.tracking.retain(|t| !left.contains(&t.0));
            for h in left {
                self.queue_leave(ctx, h, fx);
            }
        }
        if let Some((hw, _, _)) = self.hover
            && outside(self, hw)
        {
            self.hover = None;
        }
    }

    /// Whether screen point `at` is in window `h`'s client area.
    fn in_client(&self, h: u32, at: (i32, i32)) -> bool {
        let Some(w) = self.get(u64::from(h)) else { return false };
        let (ox, oy) = self.client_origin(h);
        let c = w.client_rel;
        at.0 >= ox && at.1 >= oy && at.0 < ox + c.width() && at.1 < oy + c.height()
    }

    /// A button-down that is the second of a double click on a window whose
    /// class has `CS_DBLCLKS` becomes its double-click message
    /// (WindowFromPoint.yaml `double_click`): the same button on the same
    /// window within `GetDoubleClickTime` and the double-click rectangle
    /// around the first click (1 pixel to the right is one, 3 pixels or 2
    /// down is not); the click after a double click starts afresh. Any other
    /// message is returned as it is.
    fn double_click(&mut self, ctx: Ctx, target: u32, msg: u32, at: (i32, i32)) -> u32 {
        if !matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
            return msg;
        }
        let wants = self.get(u64::from(target)).is_some_and(|w| {
            self.classes.get(w.class).and_then(|c| c.as_ref()).is_some_and(|c| c.style & CS_DBLCLKS != 0)
        });
        if wants
            && let Some((m, h, x, y, t)) = self.last_click
            && m == msg
            && h == target
            && ctx.now.saturating_sub(t) <= DOUBLE_CLICK_MS
            && (x - DOUBLE_CLICK_SIZE.0 / 2..x + DOUBLE_CLICK_SIZE.0 / 2).contains(&at.0)
            && (y - DOUBLE_CLICK_SIZE.1 / 2..y + DOUBLE_CLICK_SIZE.1 / 2).contains(&at.1)
        {
            self.last_click = None;
            return msg + 2;
        }
        self.last_click = Some((msg, target, at.0, at.1, ctx.now));
        msg
    }

    /// Marks the move just queued for thread `key` as the one a capture
    /// release synthesized.
    pub(super) fn mark_synthetic_move(&mut self, key: Key) {
        let k = self.input_key(key);
        if let Some(last) = self.queue(k).raw.back_mut()
            && last.msg == WM_MOUSEMOVE
        {
            last.private = SYNTHETIC_MOVE;
        }
    }

    /// Removes a synthesized move that was not retrieved yet.
    pub(super) fn drop_synthetic_move(&mut self, key: Key) {
        let k = self.input_key(key);
        let q = self.queue(k);
        q.raw.retain(|m| !(m.msg == WM_MOUSEMOVE && m.private == SYNTHETIC_MOVE));
        if q.raw.iter().all(|m| m.msg != WM_MOUSEMOVE) {
            q.changed &= !QS_MOUSEMOVE;
        }
    }

    /// WM_MOUSELEAVE as an input message, after what is queued.
    fn queue_leave(&mut self, ctx: Ctx, hwnd: u32, fx: &mut Effects) {
        const WM_MOUSELEAVE: u32 = 0x02A3;
        let Some(t) = self.thread_of(hwnd) else { return };
        let key = self.input_key(t);
        let cursor = self.cursor;
        let q = self.queue(key);
        q.raw.push_back(Msg {
            hwnd,
            msg: WM_MOUSELEAVE,
            w: 0,
            l: 0,
            time: ctx.now as u32,
            x: cursor.0,
            y: cursor.1,
            private: 0,
        });
        fx.wake.push(key);
    }

    /// TME_HOVER: WM_MOUSEHOVER once the cursor rested over the window for
    /// the hover time (keys, client coordinates); the hover tracking ends.
    pub(super) fn check_hover(&mut self, ctx: Ctx, fx: &mut Effects) {
        const WM_MOUSEHOVER: u32 = 0x02A1;
        let Some((hwnd, ms, since)) = self.hover else { return };
        if ctx.now < since + u64::from(ms) {
            return;
        }
        self.hover = None;
        let (x, y) = self.cursor;
        let Some(t) = self.thread_of(hwnd) else { return };
        let (ox, oy) = self.client_origin(hwnd);
        let keys = self.mouse_keys();
        let l = u64::from((x - ox) as u16) | (u64::from((y - oy) as u16) << 16);
        let key = self.input_key(t);
        let q = self.queue(key);
        q.posted.push_back(Msg { hwnd, msg: WM_MOUSEHOVER, w: keys, l, time: ctx.now as u32, x, y, private: 0 });
        fx.wake.push(key);
    }

    /// When the next hover message is due (ms from now), for the waits.
    pub(super) fn hover_due(&self, now: u64) -> Option<u64> {
        self.hover.map(|(_, ms, since)| (since + u64::from(ms)).saturating_sub(now))
    }

    /// Where a mouse event at (x, y) goes: the capture window of the
    /// foreground thread, else the window under the point.
    pub(super) fn mouse_target(&self, x: i32, y: i32) -> u32 {
        if let Some(t) = self.thread_of(self.foreground()) {
            let cap = self.input_of(t).capture;
            if cap != 0 && self.is_window(u64::from(cap)) {
                return cap;
            }
        }
        self.window_from_point(x, y, true)
    }

    /// The visible window at a screen point: the topmost top-level window
    /// containing it, then its deepest visible child (a disabled child
    /// gives its parent when `skip_disabled`).
    pub(super) fn window_from_point(&self, x: i32, y: i32, skip_disabled: bool) -> u32 {
        self.window_from_point_below(x, y, skip_disabled, 0)
    }

    /// As `window_from_point`, but only top-level windows below `below` in
    /// z order count (0: all), for a window that answered HTTRANSPARENT.
    pub(super) fn window_from_point_below(&self, x: i32, y: i32, skip_disabled: bool, below: u32) -> u32 {
        let mut tops = self.get(u64::from(self.desktop)).map(|d| d.children.clone()).unwrap_or_default();
        if below != 0 {
            match tops.iter().position(|&t| t == below) {
                Some(i) => tops.drain(..=i).for_each(drop),
                None => tops.clear(),
            }
        }
        let inside = |w: &Window, ox: i32, oy: i32| {
            let r = w.window_rel;
            x >= r.left + ox && x < r.right + ox && y >= r.top + oy && y < r.bottom + oy
        };
        for t in tops {
            let Some(w) = self.get(u64::from(t)) else { continue };
            if w.style & WS_VISIBLE == 0 || !inside(w, 0, 0) {
                continue;
            }
            let mut cur = t;
            while let Some(cw) = self.get(u64::from(cur)) {
                let (ox, oy) = self.client_origin(cur);
                // Children are clipped to the client area: a point over the
                // frame belongs to the window, not to a child reaching over it.
                let c = cw.client_rel;
                if x < ox || y < oy || x >= ox + c.width() || y >= oy + c.height() {
                    break;
                }
                let next =
                    cw.children.iter().copied().find(|&c| {
                        self.get(u64::from(c)).is_some_and(|k| k.style & WS_VISIBLE != 0 && inside(k, ox, oy))
                    });
                match next {
                    Some(c)
                        if !(skip_disabled && self.get(u64::from(c)).is_some_and(|k| k.style & WS_DISABLED != 0)) =>
                    {
                        cur = c
                    }
                    _ => break,
                }
            }
            return cur;
        }
        self.desktop
    }

    /// The input message thread `key` gets next (PM_REMOVE takes it and
    /// updates the thread's key state).
    pub(super) fn raw_message(
        &mut self,
        key: Key,
        filter: u64,
        in_range: &dyn Fn(u32) -> bool,
        remove: bool,
    ) -> Option<Msg> {
        let k = self.input_key(key);
        let q = self.queues.get_mut(&k)?;
        let i = q.raw.iter().position(|m| in_range(m.msg) && (filter == 0 || u64::from(m.hwnd) == filter))?;
        let m = if remove { q.raw.remove(i)? } else { q.raw[i].clone() };
        if remove {
            self.retrieved(k, &m);
        }
        Some(m)
    }

    /// An input message retrieved: the thread's (and the session's
    /// synchronous) key state follows it.
    fn retrieved(&mut self, key: Key, m: &Msg) {
        let change: Option<(usize, Option<usize>, bool)> = if is_key(m.msg) {
            let vk = m.w as u32;
            let scan = ((m.l >> 16) & 0xFF) as u32;
            let ext = m.l & (1 << 24) != 0;
            let down = m.msg == WM_KEYDOWN || m.msg == WM_SYSKEYDOWN;
            Some((vk as usize & 0xFF, side_of(vk, scan, ext), down))
        } else if is_mouse(m.msg) {
            match m.msg {
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => Some((VK_LBUTTON, None, true)),
                WM_LBUTTONUP => Some((VK_LBUTTON, None, false)),
                WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => Some((VK_RBUTTON, None, true)),
                WM_RBUTTONUP => Some((VK_RBUTTON, None, false)),
                WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => Some((VK_MBUTTON, None, true)),
                WM_MBUTTONUP => Some((VK_MBUTTON, None, false)),
                _ => None,
            }
        } else {
            None
        };
        if let Some((vk, side, down)) = change {
            let mut s = core::mem::take(&mut self.sync_keys);
            apply_key(&mut s, vk, side, down);
            self.sync_keys = s;
            if let Some(own) = self.queues.get_mut(&key).and_then(|q| q.keys.as_mut()) {
                apply_key(own, vk, side, down);
            }
        }
        if let Some(q) = self.queues.get_mut(&key) {
            q.last = (m.time, m.x, m.y);
        }
    }

    pub(super) fn raw_status(&self, key: Key) -> u32 {
        let k = self.input_key(key);
        let Some(q) = self.queues.get(&k) else { return 0 };
        q.raw.iter().fold(0, |b, m| {
            b | if is_key(m.msg) {
                QS_KEY
            } else if m.msg == WM_MOUSEMOVE {
                QS_MOUSEMOVE
            } else {
                QS_MOUSEBUTTON
            }
        })
    }
}
