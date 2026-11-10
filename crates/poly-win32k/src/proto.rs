//! The records of Poly's win32k system call `PolyUserCall` (ADR 0128):
//! `user32.dll` encodes a [`Request`], the NT personality decodes it, runs
//! it against the session's [`crate::wm::Wm`] and encodes the [`Reply`].
//! Window procedures never run in the kernel: what needs them (sending a
//! message, the creation sequence) is driven by `user32` with requests in
//! between ([`crate::user`]).
//!
//! Handles are the 32-bit values `user32` hands out (`HWND`s, lower 32
//! bits significant); pointers (window procedures, `HINSTANCE`s, brushes)
//! are opaque 64-bit values the kernel only stores and returns.

use crate::frame::Rect;
use crate::wire::{Reader, Wire};
use alloc::vec::Vec;

impl Wire for Rect {
    fn put(&self, out: &mut Vec<u8>) {
        self.left.put(out);
        self.top.put(out);
        self.right.put(out);
        self.bottom.put(out);
    }
    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Some(Rect { left: i32::get(r)?, top: i32::get(r)?, right: i32::get(r)?, bottom: i32::get(r)? })
    }
}

crate::wire_enum! {
    /// A window class named by its atom or its name.
    pub enum ClassRef {
        1 => Atom { atom: u16 },
        2 => Name { name: Vec<u16> },
    }
}

impl Default for ClassRef {
    fn default() -> Self {
        ClassRef::Atom { atom: 0 }
    }
}

crate::wire_struct! {
    /// What `RegisterClassExW` registers.
    pub struct ClassSpec {
        pub name: Vec<u16>,
        pub style: u32,
        pub wndproc: u64,
        pub cls_extra: i32,
        pub wnd_extra: i32,
        pub instance: u64,
        pub icon: u64,
        pub cursor: u64,
        pub brush: u64,
        pub menu_name: u64,
        pub icon_sm: u64,
        pub unicode: bool,
    }
}

crate::wire_struct! {
    /// A registered class as `GetClassInfoExW` reports it.
    pub struct ClassInfo {
        pub atom: u16,
        pub name: Vec<u16>,
        pub style: u32,
        pub wndproc: u64,
        pub cls_extra: i32,
        pub wnd_extra: i32,
        pub instance: u64,
        pub icon: u64,
        pub cursor: u64,
        pub brush: u64,
        pub menu_name: u64,
        pub icon_sm: u64,
    }
}

crate::wire_struct! {
    /// What `CreateWindowExW` asks for (sizes as given).
    pub struct CreateSpec {
        pub ex_style: u32,
        pub class: ClassRef,
        pub style: u32,
        pub x: i32,
        pub y: i32,
        pub cx: i32,
        pub cy: i32,
        pub parent: u64,
        pub menu: u64,
        pub instance: u64,
    }
}

crate::wire_struct! {
    /// The window win32k made for a `CreateSpec`, before its creation
    /// messages.
    pub struct Created {
        pub hwnd: u32,
        pub wndproc: u64,
        pub unicode: bool,
        /// Send `WM_GETMINMAXINFO` first (a top-level window with a caption
        /// or thick frame).
        pub minmax: bool,
        /// `CW_USEDEFAULT` resolved; parent-client coordinates for a child.
        pub x: i32,
        pub y: i32,
        pub cx: i32,
        pub cy: i32,
        /// The extended style `CREATESTRUCT` shows.
        pub ex_style: u32,
        /// The class name (`CREATESTRUCT.lpszClass` points at a copy).
        pub class_name: Vec<u16>,
        /// The parent `CREATESTRUCT.hwndParent` shows.
        pub cs_parent: u64,
        /// The size was `CW_USEDEFAULT`: CREATESTRUCT shows -1 x -1
        /// (observed on the oracle).
        pub default_size: bool,
        /// The height asked, which CREATESTRUCT shows with a default width
        /// unless it is `CW_USEDEFAULT` too (-1 then; observed).
        pub asked_cy: i32,
    }
}

crate::wire_struct! {
    /// A window as win32k knows it.
    pub struct WinInfo {
        pub hwnd: u32,
        pub pid: u32,
        pub tid: u32,
        pub style: u32,
        pub ex_style: u32,
        /// Screen coordinates.
        pub window: Rect,
        pub client: Rect,
        /// Relative to the parent's client area (top-level: the screen).
        pub window_rel: Rect,
        pub client_rel: Rect,
        pub parent: u32,
        pub owner: u32,
        pub atom: u16,
        pub class_style: u32,
        pub class_brush: u64,
        pub wndproc: u64,
        pub unicode: bool,
        pub instance: u64,
        pub id: u64,
        pub text_len: u32,
        /// `WS_VISIBLE` on it and every ancestor.
        pub visible: bool,
        pub minimized: bool,
        pub maximized: bool,
        /// A minimized window that was maximized before: restoring it
        /// maximizes it again.
        pub restore_max: bool,
        pub own_dc: u64,
        pub class_dc: u64,
        /// A hidden overlapped window whose `WM_SIZE`/`WM_MOVE` wait for
        /// its first showing.
        pub size_move_pending: bool,
        pub desktop: bool,
        pub message_only: bool,
        /// DPI awareness the window was created under (`guic::AWARE_*`).
        pub dpi: u8,
        /// Between `CreateWindow` and `FinishCreate`.
        pub creating: bool,
        /// The input-context association (`ime::Assoc::code`).
        pub ime: u32,
        /// Not a layered window, or one that has had its attributes (or
        /// `UpdateLayeredWindow`) set: it paints.
        pub lay_ready: bool,
    }
}

crate::wire_struct! {
    /// A message as `MSG` carries it.
    pub struct Msg {
        pub hwnd: u32,
        pub msg: u32,
        pub w: u64,
        pub l: u64,
        pub time: u32,
        pub x: i32,
        pub y: i32,
        /// MSG's lPrivate: the UTF-16 unit of a VK_PACKET key message.
        pub private: u32,
    }
}

crate::wire_struct! {
    /// One SendInput event (winuser.h INPUT): `kind` INPUT_MOUSE (0) with
    /// dx, dy, mouseData, dwFlags; INPUT_KEYBOARD (1) with wVk (`a`),
    /// wScan (`b`), dwFlags.
    pub struct InputEvent {
        pub kind: u32,
        pub a: i32,
        pub b: i32,
        pub data: u32,
        pub flags: u32,
    }
}

crate::wire_struct! {
    /// A message another thread sent, to run here.
    pub struct Incoming {
        pub id: u64,
        pub hwnd: u32,
        pub msg: u32,
        pub w: u64,
        pub l: u64,
        pub wndproc: u64,
    }
}

crate::wire_enum! {
    /// `user32` → kernel.
    pub enum Request {
        1 => Desktop {},
        2 => RegisterEvent { handle: u64 },
        3 => RegisterClass { spec: ClassSpec },
        4 => UnregisterClass { class: ClassRef, instance: u64 },
        5 => ClassInfo { class: ClassRef, instance: u64 },
        6 => ClassLong { hwnd: u64, index: i32 },
        7 => CreateWindow { spec: CreateSpec },
        8 => SetRects { hwnd: u32, window_rel: Option<Rect>, client_rel: Option<Rect> },
        9 => FinishCreate { hwnd: u32 },
        10 => Free { hwnd: u32 },
        11 => Window { hwnd: u64 },
        12 => Relatives { hwnd: u32 },
        13 => GetLong { hwnd: u64, index: i32, size: u8 },
        14 => SetLong { hwnd: u64, index: i32, size: u8, value: u64 },
        15 => SetStyle { hwnd: u32, which: i32, value: u32 },
        16 => GetText { hwnd: u64 },
        17 => SetText { hwnd: u32, text: Vec<u16> },
        18 => ZOrder { hwnd: u32, after: u64, apply: bool },
        19 => SetParent { hwnd: u32, parent: u64 },
        20 => SetState { hwnd: u32, visible: Option<bool>, minimized: Option<bool>, maximized: Option<bool>, normal: Option<Rect>, size_move_pending: Option<bool> },
        21 => FindWindow { class: Option<ClassRef>, title: Option<Vec<u16>>, parent: u64, after: u64 },
        22 => GetWindow { hwnd: u64, cmd: u32 },
        23 => Ancestor { hwnd: u64, flags: u32 },
        24 => GetParent { hwnd: u64 },
        25 => SetDc { hwnd: u32, class: bool, dc: u64 },
        26 => Post { hwnd: u64, msg: u32, w: u64, l: u64 },
        27 => PostThread { tid: u32, msg: u32, w: u64, l: u64 },
        28 => PostQuit { code: u32 },
        29 => Peek { filter: u64, min: u32, max: u32, flags: u32 },
        30 => SendStart { hwnd: u64, msg: u32, w: u64, l: u64 },
        31 => SendWait { id: u64 },
        32 => SendDone { id: u64, result: u64 },
        33 => Reply { result: u64 },
        34 => InSend {},
        35 => QueueStatus { flags: u32, clear: bool },
        36 => SetTimer { hwnd: u64, id: u64, elapse: u32, timer_proc: u64 },
        37 => KillTimer { hwnd: u64, id: u64 },
        38 => TimerProc { hwnd: u64, id: u64, timer_proc: u64 },
        39 => ClassName { hwnd: u64 },
        40 => IsChild { parent: u64, child: u64 },
        41 => Queue {},
        /// `GetMessageTime` / `GetMessagePos` of the last message retrieved:
        /// a `Value` with the time in the low 32 bits, x and y above.
        42 => MessageInfo {},
        /// The desktop link's surface of a top-level window, for its DCs to
        /// draw into: a `Value` with the width in the low 32 bits and the
        /// height above; 0 when it has none (no compositor, hidden, a child
        /// window). Answered by the NT personality, which keeps the
        /// surfaces ([`crate::surfaces`]).
        43 => Surface { hwnd: u32 },
        /// RedrawWindow's region work (`RDW_*` flags; `rects` in client
        /// coordinates, `whole` for all of the client area). Also
        /// InvalidateRect/Rgn and ValidateRect/Rgn.
        44 => Redraw { hwnd: u32, rects: Vec<Rect>, whole: bool, flags: u32 },
        /// What is pending for painting: an `Update`.
        45 => Update { hwnd: u32 },
        /// What `user32` did (`wm::paint::PAINTED_*`); `ferase` 0 keeps the
        /// window's fErase state, 1 sets it, 2 clears it.
        46 => Painted { hwnd: u32, what: u32, ferase: u8 },
        /// The windows to paint in WM_PAINT order: of the calling thread,
        /// within `root` and its descendants (0: all). A `Windows`.
        47 => PaintOrder { root: u32 },
        /// The calling thread's input state: an `Input`.
        48 => InputState {},
        /// Set the calling thread's active / focus / capture window and the
        /// foreground (`wm::input::SET_*` bits in `which`).
        49 => SetInput { active: u32, focus: u32, capture: u32, which: u8 },
        /// Thread `tid`'s input state (0: the foreground thread's).
        50 => ThreadInput { tid: u32 },
        /// Window `hwnd`'s thread becomes the foreground thread with it
        /// active and focused (SetForegroundWindow of another thread's window).
        51 => GiveForeground { hwnd: u32 },
        /// AttachThreadInput of the calling thread to thread `tid`.
        52 => AttachInput { tid: u32, attach: bool },
        /// The cursor position (screen coordinates), moved first when `set`
        /// (clamped to the screen): a `Value`, x in the low 32 bits, y above.
        53 => Cursor { set: bool, x: i32, y: i32 },
        /// SendInput: a `Value`, the number of events inserted.
        54 => SendInput { events: Vec<InputEvent> },
        /// The calling thread's key state (GetKeyboardState): a `Keys`.
        55 => KeyboardState {},
        /// SetKeyboardState for the calling thread.
        56 => SetKeyboardState { keys: Vec<u8> },
        /// GetAsyncKeyState: a `Value`.
        57 => AsyncKeyState { vk: u32 },
        /// WindowFromPoint (screen coordinates): a `Handle`; `below`: only top-level windows below this one in z order (0: all).
        58 => WindowFromPoint { x: i32, y: i32, below: u32 },
        /// TrackMouseEvent: `flags` TME_* (TME_QUERY answers a `Value`:
        /// flags | hwnd << 32 ... see `wm::raw`), `hover` ms.
        59 => TrackMouse { hwnd: u32, flags: u32, hover: u32 },
        /// The windows that were the calling queue's active window, the
        /// latest first (destroyed ones too): a `Windows`.
        60 => ActivationHistory {},
        /// Whether the calling process takes the foreground when it shows
        /// or creates a window (`wm::input`): a `Bool`.
        61 => MayTakeForeground {},
        /// `RegisterWindowMessageW`: the message number (0xC000..=0xFFFF) of a
        /// name, the same for every caller of the session and the same
        /// number as a class of that name has as its atom; an `Atom`.
        70 => RegisterMessage { name: Vec<u16> },
        /// A clipboard operation (ADR 0172): a `Clip`.
        100 => Clip { op: crate::clip::ClipOp },
        /// DPI awareness (`awareness` is `guic::AWARE_*`): without `set` a
        /// `Value` with the calling thread's; with `set` for the process an
        /// `Ok` (once) or for the thread a `Value` with the previous one.
        101 => Dpi { set: bool, process: bool, awareness: u8 },
        /// Layered attributes: `op` 0 set (a `Value` for the first set, else `Ok`), 1 get
        /// (a `Value`: key, alpha << 32, flags << 40), 2 whether `UpdateLayeredWindow`
        /// may be used (not after a set), 3 an `UpdateLayeredWindow` succeeded: `key` is
        /// its colour key, `alpha` the blend's constant alpha, `flags` the `ULW_*` flags
        /// with 0x100 for `AC_SRC_ALPHA`.
        103 => Layered { hwnd: u32, op: u8, key: u32, alpha: u8, flags: u32 },
        /// An input-context operation (ADR 0172): an `Ime`.
        105 => Ime { op: crate::ime::ImeOp },
        /// Set a window's input-context association (`ime::Assoc::code`): a `Value` with the previous code.
        106 => ImeAssoc { hwnd: u32, code: u32 },
        /// The system cursor (`IDC_*`) to show over a top-level window's surface.
        107 => CursorShape { hwnd: u32, idc: u16 },
        /// What the screen shows at a point: a `Value`, the `COLORREF`.
        108 => ScreenPixel { x: i32, y: i32 },
        /// `SetWindowRgn`: the window's region as left, top, right, bottom
        /// quadruples in window coordinates (`set` false: none): an `Ok`.
        109 => WindowRgn { hwnd: u32, set: bool, rects: Vec<u32> },
        /// The content `UpdateLayeredWindow` gave a layered window: `width` x
        /// `height` pixels (`B, G, R, A`, top-down) from the window's top-left
        /// (`width` 0: none): an `Ok`.
        110 => LayeredContent { hwnd: u32, width: u32, height: u32, pixels: Vec<u8> },
    }
}

crate::wire_enum! {
    /// kernel → `user32`.
    pub enum Reply {
        1 => Ok {},
        /// A Win32 error for the caller's last error.
        2 => Error { code: u32 },
        3 => Value { value: u64 },
        4 => Desktop { desktop: u32, message: u32, width: i32, height: i32, work: Rect, interactive: bool },
        5 => Atom { atom: u16 },
        6 => Class { info: ClassInfo },
        7 => Created { created: Created },
        8 => Window { info: WinInfo },
        9 => Relatives { children: Vec<u32>, owned: Vec<u32> },
        10 => Text { text: Vec<u16> },
        11 => Handle { hwnd: u32 },
        12 => Message { msg: Msg },
        13 => Incoming { incoming: Incoming },
        /// Nothing to retrieve; the earliest timer falls due in this many
        /// milliseconds (`None`: no timer).
        14 => Empty { next_timer_ms: Option<u64> },
        15 => SameThread { wndproc: u64, unicode: bool, hwnd: u32 },
        16 => Started { id: u64 },
        17 => Pending {},
        18 => Done { result: u64 },
        /// Queue status; the earliest timer not yet seen due, in milliseconds.
        19 => Bits { now: u32, changed: u32, next_timer_ms: Option<u64> },
        20 => Bool { value: bool },
        21 => Update { rects: Vec<Rect>, erase: bool, frame: bool, frame_whole: bool, internal: bool, ferase: bool, visible: bool },
        22 => Windows { hwnds: Vec<u32> },
        23 => Input { active: u32, focus: u32, capture: u32, foreground: u32, fg_mine: bool },
        24 => Keys { keys: Vec<u8> },
        100 => Clip { res: crate::clip::ClipRes },
        105 => Ime { res: crate::ime::ImeRes },
    }
}

/// `HWND_MESSAGE` (`winuser.h`: `(HWND)-3`).
pub const HWND_MESSAGE: u64 = (-3i64) as u64;
/// `HWND_TOP`, `HWND_BOTTOM`, `HWND_TOPMOST`, `HWND_NOTOPMOST` (`winuser.h`).
pub const HWND_TOP: u64 = 0;
pub const HWND_BOTTOM: u64 = 1;
pub const HWND_TOPMOST: u64 = (-1i64) as u64;
pub const HWND_NOTOPMOST: u64 = (-2i64) as u64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{decode, encode};
    use alloc::vec;

    #[test]
    fn requests_and_replies_round_trip() {
        let r = Request::CreateWindow {
            spec: CreateSpec {
                ex_style: 0x100,
                class: ClassRef::Name { name: vec![65, 66] },
                style: 0xcf0000,
                x: -5,
                y: 7,
                cx: 300,
                cy: 200,
                parent: 0,
                menu: 0,
                instance: 0x1_4000_0000,
            },
        };
        assert_eq!(decode::<Request>(&encode(&r)), Some(r));
        let p = Reply::Window { info: WinInfo { hwnd: 0x10010, window: Rect::new(1, 2, 3, 4), ..WinInfo::default() } };
        assert_eq!(decode::<Reply>(&encode(&p)), Some(p));
        for r in [
            Request::Ime { op: crate::ime::ImeOp::Create {} },
            Request::Clip { op: crate::clip::ClipOp::Set { format: 13, handle: 5, data: Some(vec![1, 2]) } },
            Request::Dpi { set: true, process: false, awareness: 2 },
            Request::Layered { hwnd: 1, op: 0, key: 2, alpha: 3, flags: 3 },
            Request::WindowRgn { hwnd: 1, set: true, rects: vec![1, 2, 3, 4] },
            Request::LayeredContent { hwnd: 1, width: 1, height: 1, pixels: vec![1, 2, 3, 4] },
        ] {
            assert_eq!(decode::<Request>(&encode(&r)), Some(r));
        }
        let r = Reply::Ime { res: crate::ime::ImeRes::Val { value: 7 } };
        assert_eq!(decode::<Reply>(&encode(&r)), Some(r));
        let e = Reply::Empty { next_timer_ms: Some(5) };
        assert_eq!(decode::<Reply>(&encode(&e)), Some(e));
    }
}
