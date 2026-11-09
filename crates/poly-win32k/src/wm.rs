//! win32k's state for one session (ADR 0128): window classes and atoms,
//! the window tree, per-thread message queues, sent messages between
//! threads, timers. The NT personality keeps one [`Wm`] and runs every
//! `PolyUserCall` [`Request`] against it with [`Wm::handle`]; the threads
//! to wake (their queue's event) come back in [`Effects`].
//!
//! Nothing here calls user code: window procedures run in `user32`, which
//! drives the message sequences ([`crate::user`]). Behaviour follows the
//! spec sheets of the first Phase 7 slice (`specs/windows/user32.dll/`).

use crate::frame::{FrameMetrics, Rect};
use crate::guic;
use crate::metrics::*;
use crate::proto::*;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

pub mod input;
pub mod paint;
pub mod raw;

// Win32 errors (`winerror.h`).
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
pub const ERROR_INVALID_MENU_HANDLE: u32 = 1401;
pub const ERROR_TLW_WITH_WSCHILD: u32 = 1406;
pub const ERROR_CANNOT_FIND_WND_CLASS: u32 = 1407;
pub const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
pub const ERROR_CLASS_DOES_NOT_EXIST: u32 = 1411;
pub const ERROR_CLASS_HAS_WINDOWS: u32 = 1412;
pub const ERROR_INVALID_INDEX: u32 = 1413;
pub const ERROR_INVALID_THREAD_ID: u32 = 1444;
pub const ERROR_MESSAGE_SYNC_ONLY: u32 = 1159;
pub const ERROR_NOT_ENOUGH_QUOTA: u32 = 1816;

// Queue status bits (`winuser.h`).
pub const QS_POSTMESSAGE: u32 = 0x0008;
pub const QS_TIMER: u32 = 0x0010;
pub const QS_PAINT: u32 = 0x0020;
pub const QS_SENDMESSAGE: u32 = 0x0040;

pub const WM_TIMER: u32 = 0x0113;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_PAINT: u32 = 0x000F;
pub const PM_REMOVE: u32 = 0x0001;

/// `CW_USEDEFAULT` (`winuser.h`).
pub const CW_USEDEFAULT: i32 = i32::MIN;
/// A queue holds this many posted messages (`PostMessageW.yaml`).
pub const QUEUE_LIMIT: usize = 10_000;
/// `USER_TIMER_MINIMUM` (`winuser.h`).
pub const TIMER_MINIMUM: u32 = 0x0A;

/// Messages whose parameters are pointers, which cannot be posted (Learn,
/// "PostMessage": below `WM_USER` such messages fail).
const POINTER_MESSAGES: &[u32] = &[
    0x0001, // WM_CREATE
    0x000C, // WM_SETTEXT
    0x000D, // WM_GETTEXT
    0x0024, // WM_GETMINMAXINFO
    0x0046, // WM_WINDOWPOSCHANGING
    0x0047, // WM_WINDOWPOSCHANGED
    0x004A, // WM_COPYDATA
    0x007C, // WM_STYLECHANGING
    0x007D, // WM_STYLECHANGED
    0x0081, // WM_NCCREATE
    0x0083, // WM_NCCALCSIZE
];

/// Who is calling, and when (milliseconds of the monotonic clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ctx {
    pub pid: u32,
    pub tid: u32,
    pub now: u64,
}

impl Ctx {
    fn key(&self) -> Key {
        (self.pid, self.tid)
    }
}

/// A thread: (process id, thread id).
pub type Key = (u32, u32);

/// What the kernel must do after a request.
#[derive(Debug, Default)]
pub struct Effects {
    /// Threads whose queue event to set.
    pub wake: Vec<Key>,
}

#[derive(Clone, Debug)]
struct Class {
    pid: u32,
    atom: u16,
    name: Vec<u16>,
    style: u32,
    wndproc: u64,
    cls_extra: i32,
    wnd_extra: i32,
    instance: u64,
    icon: u64,
    cursor: u64,
    brush: u64,
    menu_name: u64,
    icon_sm: u64,
    unicode: bool,
    extra: Vec<u8>,
    windows: u32,
    dc: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Creating,
    Normal,
}

#[derive(Clone, Debug)]
struct Window {
    thread: Key,
    class: usize,
    style: u32,
    ex_style: u32,
    window_rel: Rect,
    client_rel: Rect,
    parent: u32,
    owner: u32,
    children: Vec<u32>,
    text: Vec<u16>,
    wndproc: u64,
    unicode: bool,
    instance: u64,
    id: u64,
    userdata: u64,
    extra: Vec<u8>,
    state: State,
    minimized: bool,
    maximized: bool,
    normal: Rect,
    own_dc: u64,
    size_move_pending: bool,
    seq: u64,
    /// Update region, pending erase and frame (`paint`).
    paint: paint::Paint,
    /// The DPI awareness it was created under (`guic::AWARE_*`).
    dpi: u8,
    /// Window properties (`SetPropW`), keyed by atom or upper-cased name.
    props: Vec<(ClassRef, u64)>,
    /// Layered-window attributes (`WS_EX_LAYERED`, ADR 0172).
    lay: guic::Layered,
    /// The input-context association (`ime::Assoc::code`).
    ime: u32,
}

#[derive(Debug, Default)]
struct Queue {
    posted: VecDeque<Msg>,
    quit: Option<u32>,
    sent: VecDeque<u64>,
    receiving: Vec<u64>,
    changed: u32,
    /// The last message retrieved: time and position.
    last: (u32, i32, i32),
    /// Active, focus and capture window (`input`).
    input: input::InputState,
    /// Input messages waiting (`raw`).
    raw: VecDeque<Msg>,
    /// The thread's own key state after SetKeyboardState (`raw`).
    keys: Option<Vec<u8>>,
}

#[derive(Debug)]
struct Send {
    from: Key,
    to: Key,
    incoming: Incoming,
    result: Option<u64>,
    /// The sender has its result (after a `ReplyMessage`, before the
    /// receiver's procedure returned).
    collected: bool,
}

#[derive(Clone, Debug)]
struct Timer {
    thread: Key,
    hwnd: u32,
    id: u64,
    period: u64,
    due: u64,
    proc_: u64,
    flagged: bool,
}

/// The session's window manager.
#[derive(Debug)]
pub struct Wm {
    slots: Vec<Option<Window>>,
    gens: Vec<u16>,
    free: Vec<usize>,
    classes: Vec<Option<Class>>,
    atoms: BTreeMap<Vec<u16>, u16>,
    next_atom: u16,
    queues: BTreeMap<Key, Queue>,
    sends: BTreeMap<u64, Send>,
    next_send: u64,
    timers: Vec<Timer>,
    next_thread_timer: u64,
    screen: (i32, i32),
    /// The session's screen (ADR 0131).
    display: Display,
    desktop: u32,
    message: u32,
    seq: u64,
    cascade: i32,
    /// The foreground window (`input`).
    foreground: u32,
    /// The interactive session's window of another process (`input`).
    keeper: u32,
    /// AttachThreadInput: a thread to the queue whose input it shares.
    attached: BTreeMap<Key, Key>,
    /// The cursor position (screen coordinates).
    cursor: (i32, i32),
    /// The physical key state, keys pressed since GetAsyncKeyState last
    /// looked, the state as of the input messages retrieved (`raw`).
    async_keys: Vec<u8>,
    async_new: Vec<bool>,
    sync_keys: Vec<u8>,
    /// TrackMouseEvent's leave tracking: (window, flags) (`raw`).
    tracking: Vec<(u32, u32)>,
    /// TrackMouseEvent's hover tracking: (window, ms, since).
    hover: Option<(u32, u32, u64)>,
    /// The time of the last request (ms), for input from the desktop link.
    now: u64,
    /// The session's clipboard (ADR 0172).
    clip: crate::clip::Clipboard,
    /// Input contexts (`ime`).
    ime: crate::ime::Ime,
    /// DPI awareness: processes that set theirs, and threads' own (`guic`).
    proc_dpi: BTreeMap<u32, u8>,
    thread_dpi: BTreeMap<Key, u8>,
}

impl Default for Wm {
    fn default() -> Self {
        Wm::new()
    }
}

/// Upper-case a UTF-16 string for comparisons without regard to case
/// (ASCII and Latin-1 letters).
pub fn upcase(s: &[u16]) -> Vec<u16> {
    s.iter().map(|&c| upcase_char(c)).collect()
}

fn upcase_char(c: u16) -> u16 {
    match c {
        0x61..=0x7A => c - 0x20,
        0xE0..=0xFE if c != 0xF7 => c - 0x20,
        _ => c,
    }
}

const SYSTEM_DESKTOP_PID: u32 = 0;

impl Wm {
    pub fn new() -> Wm {
        let mut wm = Wm {
            slots: Vec::new(),
            gens: Vec::new(),
            free: Vec::new(),
            classes: Vec::new(),
            atoms: BTreeMap::new(),
            next_atom: 0xC000,
            queues: BTreeMap::new(),
            sends: BTreeMap::new(),
            next_send: 1,
            timers: Vec::new(),
            next_thread_timer: 0x7FEF,
            screen: DEFAULT_SCREEN,
            display: Display::SERVICE,
            desktop: 0,
            message: 0,
            seq: 0,
            cascade: 0,
            foreground: 0,
            keeper: 0,
            attached: BTreeMap::new(),
            cursor: (0, 0),
            async_keys: raw::blank_keys(),
            async_new: alloc::vec![false; 256],
            sync_keys: raw::blank_keys(),
            tracking: Vec::new(),
            hover: None,
            now: 0,
            clip: crate::clip::Clipboard::new(),
            ime: crate::ime::Ime::default(),
            proc_dpi: BTreeMap::new(),
            thread_dpi: BTreeMap::new(),
        };
        // Index 0 is never a window.
        wm.slots.push(None);
        wm.gens.push(0);
        // The system classes: found by every process (Learn: "About Window
        // Classes", system classes; `RegisterClassExW.yaml`: "Button" style
        // 0x8b, 8 extra bytes).
        for (name, style, extra) in [("Button", 0x8b, 8), ("Static", 0x88, 0), ("Edit", 0x88, 6), ("#32769", 0x08, 0)] {
            let name: Vec<u16> = name.encode_utf16().collect();
            let atom = wm.atom_for(&name);
            wm.classes.push(Some(Class {
                pid: SYSTEM_DESKTOP_PID,
                atom,
                name,
                style,
                wndproc: 0,
                cls_extra: 0,
                wnd_extra: extra,
                instance: 0,
                icon: 0,
                cursor: 0,
                brush: 0,
                menu_name: 0,
                icon_sm: 0,
                unicode: true,
                extra: Vec::new(),
                windows: 0,
                dc: 0,
            }));
        }
        let desktop_class = wm.classes.len() - 1;
        let (w, h) = wm.screen;
        wm.desktop = wm.alloc(Window::new(
            (0, 0),
            desktop_class,
            WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            Rect::new(0, 0, w, h),
        ));
        wm.message = wm.alloc(Window::new((0, 0), desktop_class, WS_POPUP, Rect::new(0, 0, 100, 100)));
        wm
    }

    /// The session's screen: the interactive session of the slice B oracle
    /// (the kernel's `poly.win32k.session=interactive`) or a compositor's
    /// output.
    pub fn set_display(&mut self, d: Display) {
        self.display = d;
        self.set_screen(d.width, d.height);
        if d.interactive {
            self.make_keeper();
            // Where the oracle's runner parks it before each test.
            self.cursor = (1500.min(d.width - 1), 600.min(d.height - 1));
        }
    }

    pub fn display(&self) -> Display {
        self.display
    }

    /// The output's size (from the desktop link when a compositor greets).
    pub fn set_screen(&mut self, w: i32, h: i32) {
        self.screen = (w, h);
        self.display.width = w;
        self.display.height = h;
        let d = self.desktop;
        if let Some(win) = self.get_mut(u64::from(d)) {
            win.window_rel = Rect::new(0, 0, w, h);
            win.client_rel = win.window_rel;
        }
    }

    pub fn screen(&self) -> (i32, i32) {
        self.screen
    }

    pub fn desktop(&self) -> u32 {
        self.desktop
    }

    fn atom_for(&mut self, name: &[u16]) -> u16 {
        let key = upcase(name);
        if let Some(&a) = self.atoms.get(&key) {
            return a;
        }
        let a = self.next_atom;
        self.next_atom = self.next_atom.wrapping_add(1).max(0xC000);
        self.atoms.insert(key, a);
        a
    }

    // ----- handles

    fn alloc(&mut self, w: Window) -> u32 {
        let idx = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(None);
                self.gens.push(0);
                self.slots.len() - 1
            }
        };
        self.gens[idx] = self.gens[idx].wrapping_add(1).max(1);
        self.slots[idx] = Some(w);
        (u32::from(self.gens[idx]) << 16) | idx as u32
    }

    fn index(&self, h: u64) -> Option<usize> {
        let h = h as u32;
        let idx = (h & 0xFFFF) as usize;
        let generation = (h >> 16) as u16;
        (idx > 0 && idx < self.slots.len() && self.gens[idx] == generation && self.slots[idx].is_some()).then_some(idx)
    }

    /// Whether `h` names a window (`IsWindow`).
    pub fn is_window(&self, h: u64) -> bool {
        self.index(h).is_some()
    }

    fn get(&self, h: u64) -> Option<&Window> {
        self.index(h).and_then(|i| self.slots[i].as_ref())
    }

    fn get_mut(&mut self, h: u64) -> Option<&mut Window> {
        let i = self.index(h)?;
        self.slots[i].as_mut()
    }

    /// The window's client origin on the screen.
    fn client_origin(&self, h: u32) -> (i32, i32) {
        let Some(w) = self.get(u64::from(h)) else { return (0, 0) };
        if w.parent == 0 {
            return (w.client_rel.left, w.client_rel.top);
        }
        let (px, py) = self.client_origin(w.parent);
        (px + w.client_rel.left, py + w.client_rel.top)
    }

    fn parent_origin(&self, h: u32) -> (i32, i32) {
        match self.get(u64::from(h)) {
            Some(w) if w.parent != 0 => self.client_origin(w.parent),
            _ => (0, 0),
        }
    }

    /// `WS_VISIBLE` on the window and all its ancestors.
    fn visible(&self, h: u32) -> bool {
        let mut h = h;
        loop {
            let Some(w) = self.get(u64::from(h)) else { return false };
            if w.style & WS_VISIBLE == 0 {
                return false;
            }
            if w.parent == 0 || w.parent == self.desktop {
                return h != self.message;
            }
            h = w.parent;
        }
    }

    /// The desktop's children, topmost first (the desktop link's
    /// candidates for a surface).
    pub fn top_levels(&self) -> Vec<u32> {
        self.get(u64::from(self.desktop)).map_or_else(Vec::new, |d| d.children.clone())
    }

    /// A window's layered attributes: (colour key, alpha, flags, any
    /// `SetLayeredWindowAttributes`, `UpdateLayeredWindow` used), for a
    /// window with `WS_EX_LAYERED`.
    pub fn layered_of(&self, h: u32) -> Option<(u32, u8, u32, bool, bool)> {
        let w = self.get(u64::from(h))?;
        (w.ex_style & guic::WS_EX_LAYERED != 0).then_some((
            w.lay.key,
            w.lay.alpha,
            w.lay.flags,
            w.lay.attrs,
            w.lay.update,
        ))
    }

    /// A window's text.
    pub fn text(&self, h: u32) -> Option<&[u16]> {
        self.get(u64::from(h)).map(|w| w.text.as_slice())
    }

    /// `WinInfo` of a window.
    pub fn info(&self, h: u32) -> Option<WinInfo> {
        let w = self.get(u64::from(h))?;
        let class = self.classes[w.class].as_ref();
        let (ox, oy) = self.parent_origin(h);
        let shift = |r: Rect| Rect::new(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy);
        Some(WinInfo {
            hwnd: h,
            pid: w.thread.0,
            tid: w.thread.1,
            style: w.style,
            ex_style: w.ex_style,
            window: shift(w.window_rel),
            client: shift(w.client_rel),
            window_rel: w.window_rel,
            client_rel: w.client_rel,
            parent: w.parent,
            owner: w.owner,
            atom: class.map_or(0, |c| c.atom),
            class_style: class.map_or(0, |c| c.style),
            class_brush: class.map_or(0, |c| c.brush),
            wndproc: w.wndproc,
            unicode: w.unicode,
            instance: w.instance,
            id: w.id,
            text_len: w.text.len() as u32,
            visible: self.visible(h),
            minimized: w.minimized,
            maximized: w.maximized,
            own_dc: w.own_dc,
            class_dc: class.map_or(0, |c| c.dc),
            size_move_pending: w.size_move_pending,
            desktop: h == self.desktop,
            message_only: w.parent == self.message,
            dpi: if h == self.desktop { guic::AWARE_PER_MONITOR } else { w.dpi },
            creating: w.state == State::Creating,
            ime: w.ime,
            lay_ready: w.ex_style & guic::WS_EX_LAYERED == 0 || w.lay.attrs || w.lay.update,
        })
    }

    fn queue(&mut self, key: Key) -> &mut Queue {
        self.queues.entry(key).or_default()
    }

    // ----- classes

    fn find_class(&self, pid: u32, class: &ClassRef) -> Option<usize> {
        let matches = |c: &Class| match class {
            ClassRef::Atom { atom } => c.atom == *atom,
            ClassRef::Name { name } => upcase(&c.name) == upcase(name),
        };
        let local = self.classes.iter().position(|c| c.as_ref().is_some_and(|c| c.pid == pid && matches(c)));
        local.or_else(|| {
            self.classes.iter().position(|c| c.as_ref().is_some_and(|c| c.pid == SYSTEM_DESKTOP_PID && matches(c)))
        })
    }

    fn class_info(c: &Class) -> ClassInfo {
        ClassInfo {
            atom: c.atom,
            name: c.name.clone(),
            style: c.style,
            wndproc: c.wndproc,
            cls_extra: c.cls_extra,
            wnd_extra: c.wnd_extra,
            instance: c.instance,
            icon: c.icon,
            cursor: c.cursor,
            brush: c.brush,
            menu_name: c.menu_name,
            icon_sm: c.icon_sm,
        }
    }

    fn register_class(&mut self, ctx: Ctx, s: ClassSpec) -> Reply {
        let exists = self
            .classes
            .iter()
            .any(|c| c.as_ref().is_some_and(|c| c.pid == ctx.pid && upcase(&c.name) == upcase(&s.name)));
        if exists {
            return err(ERROR_CLASS_ALREADY_EXISTS);
        }
        let atom = self.atom_for(&s.name);
        let c = Class {
            pid: ctx.pid,
            atom,
            name: s.name,
            style: s.style,
            wndproc: s.wndproc,
            cls_extra: s.cls_extra,
            wnd_extra: s.wnd_extra,
            instance: s.instance,
            icon: s.icon,
            cursor: s.cursor,
            brush: s.brush,
            menu_name: s.menu_name,
            icon_sm: s.icon_sm,
            unicode: s.unicode,
            extra: vec![0; s.cls_extra.max(0) as usize],
            windows: 0,
            dc: 0,
        };
        match self.classes.iter().position(Option::is_none) {
            Some(i) => self.classes[i] = Some(c),
            None => self.classes.push(Some(c)),
        }
        Reply::Atom { atom }
    }

    fn unregister_class(&mut self, ctx: Ctx, class: ClassRef) -> Reply {
        let pos = self.classes.iter().position(|c| {
            c.as_ref().is_some_and(|c| {
                c.pid == ctx.pid
                    && match &class {
                        ClassRef::Atom { atom } => c.atom == *atom,
                        ClassRef::Name { name } => upcase(&c.name) == upcase(name),
                    }
            })
        });
        let Some(i) = pos else {
            return match class {
                ClassRef::Atom { atom: 0 } => err(ERROR_INVALID_PARAMETER),
                ClassRef::Atom { .. } => err(ERROR_INVALID_HANDLE),
                ClassRef::Name { .. } => err(ERROR_CLASS_DOES_NOT_EXIST),
            };
        };
        if self.classes[i].as_ref().is_some_and(|c| c.windows > 0) {
            return err(ERROR_CLASS_HAS_WINDOWS);
        }
        self.classes[i] = None;
        Reply::Ok {}
    }

    fn class_long(&self, hwnd: u64, index: i32) -> Reply {
        let Some(w) = self.get(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let Some(c) = self.classes[w.class].as_ref() else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let v = match index {
            -32 => u64::from(c.atom),
            -26 => u64::from(c.style),
            -24 => c.wndproc,
            -20 => c.cls_extra as u64,
            -18 => c.wnd_extra as u64,
            -16 => c.instance,
            -14 => c.icon,
            -12 => c.cursor,
            -10 => c.brush,
            -8 => c.menu_name,
            -34 => c.icon_sm,
            i if i >= 0 && (i as usize) + 8 <= c.extra.len() => read_le(&c.extra, i as usize, 8),
            _ => return err(ERROR_INVALID_INDEX),
        };
        Reply::Value { value: v }
    }

    // ----- windows

    fn create(&mut self, ctx: Ctx, s: CreateSpec) -> Reply {
        let Some(class) = self.find_class(ctx.pid, &s.class) else { return err(ERROR_CANNOT_FIND_WND_CLASS) };
        let child = s.style & WS_CHILD != 0;
        let (parent, owner) = if s.parent == HWND_MESSAGE {
            (self.message, 0)
        } else if child {
            if s.parent == 0 {
                return err(ERROR_TLW_WITH_WSCHILD);
            }
            if !self.is_window(s.parent) {
                return err(ERROR_INVALID_WINDOW_HANDLE);
            }
            (s.parent as u32, 0)
        } else if s.parent == 0 {
            (self.desktop, 0)
        } else {
            if !self.is_window(s.parent) {
                return err(ERROR_INVALID_WINDOW_HANDLE);
            }
            (self.desktop, self.root(s.parent as u32))
        };
        if !child && s.menu != 0 {
            // No menus exist yet, so no value is a menu handle.
            return err(ERROR_INVALID_MENU_HANDLE);
        }
        // A message-only window of style 0 is overlapped too (its client
        // area is inside a caption frame, ClientToScreen.yaml cts_states).
        let overlapped = !child && s.style & WS_POPUP == 0 && (parent == self.desktop || parent == self.message);
        let (mut x, mut y, mut cx, mut cy) = (s.x, s.y, s.cx, s.cy);
        let default_size = overlapped && s.cx == CW_USEDEFAULT;
        if overlapped {
            let default_pos = x == CW_USEDEFAULT;
            if default_pos {
                let step = 26 * (self.cascade % 8);
                self.cascade += 1;
                x = step;
                y = step;
            }
            if cx == CW_USEDEFAULT {
                if default_pos {
                    cx = self.screen.0 * 3 / 4;
                    cy = self.screen.1 * 3 / 4 - 21;
                } else {
                    cx = self.screen.0 * 3 / 4 - x.max(0) / 2;
                    cy = self.screen.1 * 3 / 4 - y.max(0) / 2;
                }
            }
        } else {
            if x == CW_USEDEFAULT {
                x = 0;
                y = 0;
            }
            if cx == CW_USEDEFAULT {
                cx = 0;
                cy = 0;
            }
        }
        let mut ex = s.ex_style;
        if child
            && self
                .get(u64::from(parent))
                .is_some_and(|p| p.ex_style & (WS_EX_LAYOUTRTL | WS_EX_NOINHERITLAYOUT) == WS_EX_LAYOUTRTL)
        {
            ex |= WS_EX_LAYOUTRTL;
        }
        if s.style & (WS_DLGFRAME | WS_THICKFRAME) != 0 || ex & WS_EX_DLGMODALFRAME != 0 {
            ex |= WS_EX_WINDOWEDGE;
        } else {
            ex &= !WS_EX_WINDOWEDGE;
        }
        let (wndproc, unicode, extra, class_name) = {
            let c = self.classes[class].as_mut().expect("found");
            c.windows += 1;
            (c.wndproc, c.unicode, c.wnd_extra.max(0) as usize, c.name.clone())
        };
        let minmax = parent == self.desktop && (has_caption(s.style) || s.style & WS_THICKFRAME != 0);
        // An overlapped window has a caption: WS_CAPTION is added and the
        // frame counts it from WM_NCCALCSIZE on (AdjustWindowRectEx.yaml's
        // round trip), though CREATESTRUCT shows the style as given.
        let stored = if overlapped { s.style | WS_CAPTION } else { s.style };
        let mut win = Window::new(ctx.key(), class, stored, Rect::default());
        win.ex_style = ex;
        win.parent = parent;
        win.owner = owner;
        win.wndproc = wndproc;
        win.unicode = unicode;
        win.instance = s.instance;
        win.id = if child { s.menu } else { 0 };
        win.extra = vec![0; extra];
        win.state = State::Creating;
        win.dpi =
            if child { self.get(u64::from(parent)).map_or(0, |p| p.dpi) } else { self.thread_awareness(ctx.key()) };
        self.seq += 1;
        win.seq = self.seq;
        let hwnd = self.alloc(win);
        // Top-level windows go on top; children below their siblings
        // (`SetWindowPos.yaml`: GW_CHILD is the first created).
        // A top-level window that is not topmost goes below the topmost band.
        let at = self.band_start(parent, ex & WS_EX_TOPMOST != 0);
        let p = self.get_mut(u64::from(parent)).expect("valid parent");
        if child {
            p.children.push(hwnd);
        } else {
            p.children.insert(at, hwnd);
        }
        self.queue(ctx.key());
        Reply::Created {
            created: Created {
                hwnd,
                wndproc,
                unicode,
                minmax,
                x,
                y,
                cx,
                cy,
                ex_style: ex,
                class_name,
                cs_parent: s.parent,
                default_size,
                asked_cy: s.cy,
            },
        }
    }

    /// The top-level window of `h`'s chain of parents (`GA_ROOT`).
    fn root(&self, h: u32) -> u32 {
        let mut h = h;
        while let Some(w) = self.get(u64::from(h)) {
            if w.parent == 0 || w.parent == self.desktop || w.parent == self.message {
                return h;
            }
            h = w.parent;
        }
        h
    }

    fn finish_create(&mut self, hwnd: u32) -> Reply {
        let desktop = self.desktop;
        let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        w.state = State::Normal;
        let top = w.parent == desktop;
        if top {
            w.style |= WS_CLIPSIBLINGS;
            if w.style & (WS_POPUP | WS_CHILD) == 0 {
                w.style |= WS_CAPTION;
                w.ex_style |= WS_EX_WINDOWEDGE;
                if w.style & WS_VISIBLE == 0 {
                    w.size_move_pending = true;
                }
            }
        }
        match self.info(hwnd) {
            Some(info) => Reply::Window { info },
            None => err(ERROR_INVALID_WINDOW_HANDLE),
        }
    }

    /// Unlink and free a window whose `WM_NCDESTROY` has run (or whose
    /// creation failed): its posted messages and timers go with it.
    fn free_window(&mut self, hwnd: u32) {
        let Some(idx) = self.index(u64::from(hwnd)) else { return };
        let w = self.slots[idx].take().expect("valid");
        self.free.push(idx);
        if let Some(p) = self.get_mut(u64::from(w.parent)) {
            p.children.retain(|&c| c != hwnd);
        }
        if let Some(c) = self.classes[w.class].as_mut() {
            c.windows = c.windows.saturating_sub(1);
        }
        self.forget_input(hwnd);
        self.clip.window_gone(hwnd);
        for q in self.queues.values_mut() {
            q.posted.retain(|m| m.hwnd != hwnd);
        }
        self.timers.retain(|t| t.hwnd != hwnd);
        // Children not destroyed with it (another thread's) are freed too.
        for c in w.children {
            self.free_window(c);
        }
        for i in 1..self.slots.len() {
            if let Some(o) = self.slots[i].as_mut()
                && o.owner == hwnd
            {
                o.owner = 0;
            }
        }
    }

    fn relatives(&self, hwnd: u32) -> Reply {
        let Some(w) = self.get(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let mut owned: Vec<(u64, u32)> = Vec::new();
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(o) = s
                && o.owner == hwnd
            {
                owned.push((o.seq, (u32::from(self.gens[i]) << 16) | i as u32));
            }
        }
        owned.sort_by_key(|o| core::cmp::Reverse(o.0));
        Reply::Relatives { children: w.children.clone(), owned: owned.into_iter().map(|o| o.1).collect() }
    }

    fn get_long(&self, hwnd: u64, index: i32, size: u8) -> Reply {
        let Some(w) = self.get(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let v = match index {
            -4 => w.wndproc,
            -6 => w.instance,
            -8 => {
                if w.parent != self.desktop && w.parent != 0 {
                    u64::from(w.parent)
                } else {
                    u64::from(w.owner)
                }
            }
            -12 => w.id,
            -16 => u64::from(w.style),
            -20 => u64::from(w.ex_style),
            -21 => w.userdata,
            -2 => 0,
            crate::user::NORMAL_RECT_INDEX => {
                let r = w.normal;
                u64::from(r.left as u16)
                    | (u64::from(r.top as u16) << 16)
                    | (u64::from(r.width() as u16) << 32)
                    | (u64::from(r.height() as u16) << 48)
            }
            i if i >= 0 && (i as usize) + usize::from(size) <= w.extra.len() => {
                read_le(&w.extra, i as usize, usize::from(size))
            }
            _ => return err(ERROR_INVALID_INDEX),
        };
        Reply::Value { value: v }
    }

    fn set_long(&mut self, hwnd: u64, index: i32, size: u8, value: u64) -> Reply {
        let desktop = self.desktop;
        let root_owner = if index == -8 && value != 0 && self.is_window(value) { self.root(value as u32) } else { 0 };
        let Some(w) = self.get_mut(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let old = match index {
            -4 => core::mem::replace(&mut w.wndproc, value),
            -6 => core::mem::replace(&mut w.instance, value),
            -8 => {
                if w.parent == desktop {
                    u64::from(core::mem::replace(&mut w.owner, root_owner))
                } else {
                    u64::from(w.parent)
                }
            }
            -12 => {
                if w.style & WS_CHILD == 0 && value != 0 {
                    return err(ERROR_INVALID_MENU_HANDLE);
                }
                core::mem::replace(&mut w.id, value)
            }
            -16 => u64::from(core::mem::replace(&mut w.style, value as u32)),
            -20 => {
                let new = guic::filter_ex_style(w.style, w.ex_style, value as u32, &mut w.lay);
                u64::from(core::mem::replace(&mut w.ex_style, new))
            }
            -21 => core::mem::replace(&mut w.userdata, value),
            -2 => 0,
            i if i >= 0 && (i as usize) + usize::from(size) <= w.extra.len() => {
                let at = i as usize;
                let old = read_le(&w.extra, at, usize::from(size));
                w.extra[at..at + usize::from(size)].copy_from_slice(&value.to_le_bytes()[..usize::from(size)]);
                old
            }
            _ => return err(ERROR_INVALID_INDEX),
        };
        Reply::Value { value: old }
    }

    fn set_style(&mut self, hwnd: u32, which: i32, value: u32) -> Reply {
        let desktop = self.desktop;
        let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let stored = if which == -16 {
            let v = if w.parent == desktop { value | WS_CLIPSIBLINGS } else { value };
            w.style = v;
            v
        } else {
            let new = guic::filter_ex_style(w.style, w.ex_style, value, &mut w.lay);
            w.ex_style = new;
            new
        };
        Reply::Value { value: u64::from(stored) }
    }

    /// Where a window that is (`topmost`) or is not in the topmost band goes
    /// at the top of its band among `parent`'s children: only the desktop's
    /// children have bands (the windows with WS_EX_TOPMOST lead the list).
    fn band_start(&self, parent: u32, topmost: bool) -> usize {
        if parent != self.desktop || topmost {
            return 0;
        }
        self.get(u64::from(parent)).map_or(0, |p| {
            p.children
                .iter()
                .take_while(|&&c| self.get(u64::from(c)).is_some_and(|k| k.ex_style & WS_EX_TOPMOST != 0))
                .count()
        })
    }

    fn z_order(&mut self, hwnd: u32, after: u64, apply: bool) -> Reply {
        let Some(w) = self.get(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let parent = w.parent;
        let in_topmost_band = w.ex_style & WS_EX_TOPMOST != 0;
        let Some(p) = self.get(u64::from(parent)) else { return Reply::Bool { value: false } };
        let old = p.children.clone();
        let mut new: Vec<u32> = old.iter().copied().filter(|&c| c != hwnd).collect();
        match after {
            HWND_TOP | HWND_TOPMOST | HWND_NOTOPMOST => {
                // Owned windows stay above their owner (Learn: "An owned
                // window is always above its owner in the z-order").
                let owned: Vec<u32> =
                    new.iter().copied().filter(|&c| self.get(u64::from(c)).is_some_and(|o| o.owner == hwnd)).collect();
                new.retain(|c| !owned.contains(c));
                // Top of its band: a window outside the topmost band goes
                // below the topmost windows (HWND_TOPMOST / HWND_NOTOPMOST
                // leave the style to the caller).
                let band = if parent == self.desktop && !in_topmost_band {
                    new.iter()
                        .take_while(|&&c| self.get(u64::from(c)).is_some_and(|k| k.ex_style & WS_EX_TOPMOST != 0))
                        .count()
                } else {
                    0
                };
                new.insert(band, hwnd);
                for (i, o) in owned.into_iter().enumerate() {
                    new.insert(band + i, o);
                }
            }
            HWND_BOTTOM => new.push(hwnd),
            a => match new.iter().position(|&c| u64::from(c) == a & 0xFFFF_FFFF) {
                Some(i) => new.insert(i + 1, hwnd),
                None => return Reply::Bool { value: false },
            },
        }
        let changed = new != old;
        if apply && changed {
            self.get_mut(u64::from(parent)).expect("valid").children = new;
        }
        Reply::Bool { value: changed }
    }

    fn set_parent(&mut self, hwnd: u32, parent: u64) -> Reply {
        let new_parent = if parent == 0 { self.desktop } else { parent as u32 };
        if !self.is_window(u64::from(new_parent)) {
            return err(ERROR_INVALID_WINDOW_HANDLE);
        }
        let Some(w) = self.get(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let old = w.parent;
        // Windows of different DPI awareness kinds cannot be parent and
        // child (GetWindowDpiAwarenessContext.yaml set_parent).
        if new_parent != self.desktop
            && let Some(np) = self.get(u64::from(new_parent))
            && guic::kind(np.dpi) != guic::kind(w.dpi)
        {
            return err(guic::ERROR_INVALID_STATE);
        }
        if let Some(p) = self.get_mut(u64::from(old)) {
            p.children.retain(|&c| c != hwnd);
        }
        self.get_mut(u64::from(new_parent)).expect("valid").children.insert(0, hwnd);
        self.get_mut(u64::from(hwnd)).expect("valid").parent = new_parent;
        Reply::Handle { hwnd: old }
    }

    fn find_window(
        &self,
        ctx: Ctx,
        class: Option<ClassRef>,
        title: Option<Vec<u16>>,
        parent: u64,
        after: u64,
    ) -> Reply {
        let p = if parent == 0 {
            self.desktop
        } else if parent == HWND_MESSAGE {
            self.message
        } else {
            parent as u32
        };
        let Some(pw) = self.get(u64::from(p)) else { return Reply::Handle { hwnd: 0 } };
        let class_atom = match &class {
            None => None,
            Some(c) => match self.find_class(ctx.pid, c) {
                Some(i) => Some(self.classes[i].as_ref().map_or(0, |c| c.atom)),
                None => return Reply::Handle { hwnd: 0 },
            },
        };
        let title = title.map(|t| upcase(&t));
        let start = if after == 0 {
            0
        } else {
            match pw.children.iter().position(|&c| u64::from(c) == after & 0xFFFF_FFFF) {
                Some(i) => i + 1,
                None => return Reply::Handle { hwnd: 0 },
            }
        };
        let mut candidates: Vec<u32> = pw.children[start..].to_vec();
        if parent == 0 && after == 0 {
            // Observed: FindWindowW also finds a message-only window when
            // no top-level window matches.
            if let Some(m) = self.get(u64::from(self.message)) {
                candidates.extend(m.children.iter().copied());
            }
        }
        for c in candidates {
            let Some(w) = self.get(u64::from(c)) else { continue };
            if w.state != State::Normal {
                continue;
            }
            if let Some(a) = class_atom
                && self.classes[w.class].as_ref().map_or(0, |c| c.atom) != a
            {
                continue;
            }
            if let Some(t) = &title
                && upcase(&w.text) != *t
            {
                continue;
            }
            return Reply::Handle { hwnd: c };
        }
        Reply::Handle { hwnd: 0 }
    }

    fn get_window(&self, hwnd: u64, cmd: u32) -> Reply {
        let Some(w) = self.get(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let h = hwnd as u32;
        let siblings = self.get(u64::from(w.parent)).map(|p| p.children.as_slice()).unwrap_or(&[]);
        let pos = siblings.iter().position(|&c| c == h);
        let r = match cmd {
            0 => siblings.first().copied().unwrap_or(0),
            1 => siblings.last().copied().unwrap_or(0),
            2 => pos.and_then(|i| siblings.get(i + 1)).copied().unwrap_or(0),
            3 => pos.and_then(|i| i.checked_sub(1)).and_then(|i| siblings.get(i)).copied().unwrap_or(0),
            4 => w.owner,
            5 => w.children.first().copied().unwrap_or(0),
            _ => 0,
        };
        Reply::Handle { hwnd: r }
    }

    fn ancestor(&self, hwnd: u64, flags: u32) -> Reply {
        let Some(w) = self.get(hwnd) else { return Reply::Handle { hwnd: 0 } };
        let h = hwnd as u32;
        let r = match flags {
            1 => w.parent,
            2 => {
                if h == self.desktop {
                    h
                } else {
                    self.root(h)
                }
            }
            3 => {
                let mut r = self.root(h);
                loop {
                    let p = self.parent_of(r);
                    if p == 0 || p == self.desktop {
                        break r;
                    }
                    r = self.root(p);
                }
            }
            _ => 0,
        };
        Reply::Handle { hwnd: r }
    }

    /// `GetParent`: a child's parent, a popup's owner, else NULL.
    fn parent_of(&self, h: u32) -> u32 {
        let Some(w) = self.get(u64::from(h)) else { return 0 };
        if h == self.desktop {
            0
        } else if w.style & WS_CHILD != 0 {
            w.parent
        } else if w.style & WS_POPUP != 0 {
            w.owner
        } else {
            0
        }
    }

    fn is_child(&self, parent: u64, child: u64) -> bool {
        let mut h = child as u32;
        while let Some(w) = self.get(u64::from(h)) {
            if w.style & WS_CHILD == 0 {
                return false;
            }
            if u64::from(w.parent) == parent & 0xFFFF_FFFF {
                return true;
            }
            h = w.parent;
        }
        false
    }

    // ----- messages

    #[allow(clippy::too_many_arguments)]
    /// `SetPropW` (`op` 1), `GetPropW` (0), `RemovePropW` (2) and the count
    /// of properties (3): a `Value` (0 when there is none).
    fn prop_op(&mut self, hwnd: u64, op: u8, key: ClassRef, value: u64) -> Reply {
        let key = match key {
            ClassRef::Name { name } => ClassRef::Name { name: upcase(&name) },
            a => a,
        };
        let Some(w) = self.get_mut(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let at = w.props.iter().position(|p| p.0 == key);
        match op {
            0 => Reply::Value { value: at.map_or(0, |i| w.props[i].1) },
            1 => {
                match at {
                    Some(i) => w.props[i].1 = value,
                    None => w.props.push((key, value)),
                }
                Reply::Ok {}
            }
            2 => Reply::Value { value: at.map_or(0, |i| w.props.remove(i).1) },
            _ => Reply::Value { value: w.props.len() as u64 },
        }
    }

    /// Layered-window attributes (`SetLayeredWindowAttributes`, `GetLayeredWindowAttributes`
    /// and the mark `UpdateLayeredWindow` leaves; `Layered.yaml` sheets): see [`guic::Layered`].
    fn layered_op(&mut self, hwnd: u32, op: u8, key: u32, alpha: u8, flags: u32) -> Reply {
        let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        if w.ex_style & guic::WS_EX_LAYERED == 0 {
            return err(ERROR_INVALID_PARAMETER);
        }
        let l = &mut w.lay;
        match op {
            0 => {
                if flags & !3 != 0 {
                    return err(ERROR_INVALID_PARAMETER);
                }
                if flags & 1 != 0 {
                    l.key = key;
                }
                if flags & 2 != 0 {
                    l.alpha = alpha;
                }
                l.flags = flags;
                let first = !l.attrs && !l.update;
                l.attrs = true;
                // The first attributes make the window paint.
                if first { Reply::Value { value: 1 } } else { Reply::Ok {} }
            }
            1 => {
                if !l.attrs {
                    return err(0);
                }
                Reply::Value { value: u64::from(l.key) | u64::from(l.alpha) << 32 | u64::from(l.flags) << 40 }
            }
            _ => {
                if l.attrs {
                    return err(ERROR_INVALID_PARAMETER);
                }
                l.update = true;
                Reply::Ok {}
            }
        }
    }

    /// A thread's DPI awareness: its own, else its process's, else unaware.
    pub(crate) fn thread_awareness(&self, key: Key) -> u8 {
        self.thread_dpi.get(&key).or_else(|| self.proc_dpi.get(&key.0)).copied().unwrap_or(guic::AWARE_UNAWARE)
    }

    /// DPI awareness (`SetProcessDpiAwarenessContext`, `SetThreadDpiAwarenessContext`,
    /// `GetThreadDpiAwarenessContext`; ADR 0172): the process may set it
    /// once; a thread's own wins over the process's.
    fn dpi_op(&mut self, ctx: Ctx, set: bool, process: bool, awareness: u8) -> Reply {
        let prev = self.thread_awareness(ctx.key());
        if !set {
            return Reply::Value { value: u64::from(prev) };
        }
        if process && self.proc_dpi.contains_key(&ctx.pid) {
            return err(ERROR_ACCESS_DENIED);
        }
        if awareness > guic::AWARE_GDI_SCALED {
            return err(ERROR_INVALID_PARAMETER);
        }
        if process {
            self.proc_dpi.insert(ctx.pid, awareness);
            // The windows made before report the process's awareness now
            // (SetProcessDpiAwarenessContext.yaml mixed_observations).
            let desktop = self.desktop;
            for (i, slot) in self.slots.iter_mut().enumerate() {
                if let Some(w) = slot
                    && w.thread.0 == ctx.pid
                    && (u32::from(self.gens[i]) << 16 | i as u32) != desktop
                {
                    w.dpi = awareness;
                }
            }
            Reply::Ok {}
        } else {
            self.thread_dpi.insert(ctx.key(), awareness);
            Reply::Value { value: u64::from(prev) }
        }
    }

    /// One clipboard operation (ADR 0172): the window checks here, the rules
    /// in [`crate::clip`], `WM_CLIPBOARDUPDATE` posted to the listeners when a
    /// close changed something.
    fn clip_op(&mut self, ctx: Ctx, op: crate::clip::ClipOp, fx: &mut Effects) -> Reply {
        use crate::clip::{ClipOp, ClipRes, ERROR_CLIPBOARD_NOT_OPEN, WM_CLIPBOARDUPDATE};
        match &op {
            ClipOp::Open { hwnd } if *hwnd != 0 && !self.is_window(u64::from(*hwnd)) => {
                return Reply::Clip { res: ClipRes::Err { code: ERROR_INVALID_WINDOW_HANDLE } };
            }
            ClipOp::Listener { hwnd, .. } => {
                if !self.is_window(u64::from(*hwnd)) {
                    return Reply::Clip { res: ClipRes::Err { code: ERROR_INVALID_WINDOW_HANDLE } };
                }
                if *hwnd == self.desktop {
                    // The desktop window: the call returns 5 and leaves the
                    // last error (AddClipboardFormatListener.yaml).
                    return Reply::Clip { res: ClipRes::Val { value: 5 } };
                }
            }
            _ => {}
        }
        let _ = ERROR_CLIPBOARD_NOT_OPEN;
        let mut notify = false;
        let res = self.clip.run(ctx.pid, ctx.tid, op, &mut notify);
        if notify {
            for l in self.clip.listeners_newest_first() {
                if let Some(to) = self.get(u64::from(l)).map(|w| w.thread) {
                    self.post(ctx, to, l, WM_CLIPBOARDUPDATE, 0, 0, fx);
                }
            }
        }
        Reply::Clip { res }
    }

    fn post(&mut self, ctx: Ctx, to: Key, hwnd: u32, msg: u32, w: u64, l: u64, fx: &mut Effects) -> Reply {
        let cursor = self.cursor;
        if POINTER_MESSAGES.contains(&msg) {
            return err(ERROR_MESSAGE_SYNC_ONLY);
        }
        let q = self.queue(to);
        if q.posted.len() >= QUEUE_LIMIT {
            return err(ERROR_NOT_ENOUGH_QUOTA);
        }
        q.posted.push_back(Msg { hwnd, msg, w, l, time: ctx.now as u32, x: cursor.0, y: cursor.1, private: 0 });
        q.changed |= QS_POSTMESSAGE;
        fx.wake.push(to);
        Reply::Ok {}
    }

    /// Post a message generated by the desktop link to the window's own
    /// thread. Unlike `PostMessageW`, this is trusted compositor input.
    pub fn post_system_message(&mut self, hwnd: u32, msg: u32, now: u64, fx: &mut Effects) -> bool {
        let Some(window) = self.get(u64::from(hwnd)) else { return false };
        let to = window.thread;
        let ctx = Ctx { pid: to.0, tid: to.1, now };
        matches!(self.post(ctx, to, hwnd, msg, 0, 0, fx), Reply::Ok {})
    }

    /// Apply a compositor-moved or resized client rectangle to a top-level
    /// window's Win32 geometry. `client` is in screen coordinates, as the
    /// desktop link's `Configure` record is.
    pub fn configure_from_desktop(&mut self, hwnd: u32, client: Rect, frame: FrameMetrics) -> bool {
        if self.get(u64::from(hwnd)).is_none() {
            return false;
        }
        let (ox, oy) = self.parent_origin(hwnd);
        let outer = frame.window_from_surface(client);
        let window_rel = Rect::new(outer.left - ox, outer.top - oy, outer.right - ox, outer.bottom - oy);
        let client_rel = Rect::new(client.left - ox, client.top - oy, client.right - ox, client.bottom - oy);
        let Some(window) = self.get_mut(u64::from(hwnd)) else { return false };
        window.window_rel = window_rel;
        window.client_rel = client_rel;
        if !window.maximized && !window.minimized {
            window.normal = window_rel;
        }
        true
    }

    /// Mark the caller's timers that fell due (once each) in `changed`.
    fn refresh_timers(&mut self, key: Key, now: u64) {
        let mut new = false;
        for t in self.timers.iter_mut().filter(|t| t.thread == key) {
            if t.due <= now && !t.flagged {
                t.flagged = true;
                new = true;
            }
        }
        if new {
            self.queue(key).changed |= QS_TIMER;
        }
    }

    fn status_now(&self, key: Key, now: u64) -> u32 {
        let mut bits = 0;
        if let Some(q) = self.queues.get(&key) {
            if !q.posted.is_empty() || q.quit.is_some() {
                bits |= QS_POSTMESSAGE;
            }
            if !q.sent.is_empty() {
                bits |= QS_SENDMESSAGE;
            }
        }
        if self.timers.iter().any(|t| t.thread == key && t.due <= now) {
            bits |= QS_TIMER;
        }
        bits | self.paint_status(key) | self.raw_status(key)
    }

    fn peek(&mut self, ctx: Ctx, filter: u64, min: u32, max: u32, flags: u32) -> Reply {
        self.check_hover(ctx, &mut Effects::default());
        let cursor = self.cursor;
        let key = ctx.key();
        let thread_only = filter as u32 == u32::MAX;
        if filter != 0 && !thread_only && !self.is_window(filter) {
            return err(ERROR_INVALID_WINDOW_HANDLE);
        }
        self.refresh_timers(key, ctx.now);
        self.queue(key).changed = 0;
        // Messages other threads sent come first, whatever the filter.
        if let Some(id) = self.queue(key).sent.pop_front() {
            self.queue(key).receiving.push(id);
            let incoming = self.sends.get(&id).map(|s| s.incoming.clone()).unwrap_or_default();
            return Reply::Incoming { incoming };
        }
        let qs = flags >> 16;
        let want = |bit: u32| qs == 0 || qs & bit != 0;
        let remove = flags & PM_REMOVE != 0;
        let in_range = |m: u32| (min == 0 && max == 0) || (min <= m && m <= max);
        let filter_hwnd = filter as u32;
        let matches =
            |m: &Msg| in_range(m.msg) && if thread_only { m.hwnd == 0 } else { filter == 0 || m.hwnd == filter_hwnd };
        if want(QS_POSTMESSAGE) {
            let q = self.queue(key);
            if let Some(i) = q.posted.iter().position(matches) {
                let m = if remove { q.posted.remove(i).expect("found") } else { q.posted[i].clone() };
                return Reply::Message { msg: m };
            }
            if let Some(code) = q.quit {
                if remove {
                    q.quit = None;
                }
                return Reply::Message {
                    msg: Msg {
                        hwnd: 0,
                        msg: WM_QUIT,
                        w: u64::from(code),
                        l: 0,
                        time: ctx.now as u32,
                        x: cursor.0,
                        y: cursor.1,
                        private: 0,
                    },
                };
            }
        }
        if (want(raw::QS_KEY) || want(raw::QS_MOUSEMOVE) || want(raw::QS_MOUSEBUTTON))
            && let Some(m) = self.raw_message(key, if thread_only { u64::MAX } else { filter }, &in_range, remove)
        {
            return Reply::Message { msg: m };
        }
        if want(QS_PAINT)
            && in_range(WM_PAINT)
            && let Some(m) = self.paint_message(ctx, filter, thread_only)
        {
            return Reply::Message { msg: m };
        }
        if want(QS_TIMER) && in_range(WM_TIMER) {
            let due = self
                .timers
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.thread == key
                        && t.due <= ctx.now
                        && if thread_only { t.hwnd == 0 } else { filter == 0 || t.hwnd == filter_hwnd }
                })
                .min_by_key(|(_, t)| t.due)
                .map(|(i, _)| i);
            if let Some(i) = due {
                let t = &mut self.timers[i];
                let m = Msg {
                    hwnd: t.hwnd,
                    msg: WM_TIMER,
                    w: t.id,
                    l: t.proc_,
                    time: ctx.now as u32,
                    x: cursor.0,
                    y: cursor.1,
                    private: 0,
                };
                if remove {
                    t.due = ctx.now + t.period;
                    t.flagged = false;
                }
                return Reply::Message { msg: m };
            }
        }
        let next = self.timers.iter().filter(|t| t.thread == key).map(|t| t.due.saturating_sub(ctx.now)).min();
        let next = match (next, self.hover_due(ctx.now)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Reply::Empty { next_timer_ms: next }
    }

    fn send_start(&mut self, ctx: Ctx, hwnd: u64, msg: u32, w: u64, l: u64, fx: &mut Effects) -> Reply {
        let Some(win) = self.get(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let to = win.thread;
        let (wndproc, unicode) = (win.wndproc, win.unicode);
        // The desktop and the message window have no thread of their own:
        // their default procedure runs in the sender.
        if to == ctx.key() || to.0 == SYSTEM_DESKTOP_PID || to == input::KEEPER {
            return Reply::SameThread { wndproc, unicode, hwnd: hwnd as u32 };
        }
        let id = self.next_send;
        self.next_send += 1;
        let incoming = Incoming { id, hwnd: hwnd as u32, msg, w, l, wndproc };
        self.sends.insert(id, Send { from: ctx.key(), to, incoming, result: None, collected: false });
        let q = self.queue(to);
        q.sent.push_back(id);
        q.changed |= QS_SENDMESSAGE;
        fx.wake.push(to);
        Reply::Started { id }
    }

    fn send_wait(&mut self, ctx: Ctx, id: u64) -> Reply {
        match self.sends.get(&id) {
            None => return Reply::Done { result: 0 },
            Some(s) if s.result.is_some() => {
                // The receiver may still be running the procedure after a
                // `ReplyMessage`; the record goes when it is done.
                let r = s.result.expect("checked");
                let running = self.queues.get(&s.to).is_some_and(|q| q.receiving.contains(&id));
                if running {
                    self.sends.get_mut(&id).expect("present").collected = true;
                } else {
                    self.sends.remove(&id);
                }
                return Reply::Done { result: r };
            }
            Some(_) => {}
        }
        let key = ctx.key();
        if let Some(next) = self.queue(key).sent.pop_front() {
            self.queue(key).receiving.push(next);
            let incoming = self.sends.get(&next).map(|s| s.incoming.clone()).unwrap_or_default();
            return Reply::Incoming { incoming };
        }
        Reply::Pending {}
    }

    fn send_done(&mut self, ctx: Ctx, id: u64, result: u64, fx: &mut Effects) {
        let q = self.queue(ctx.key());
        if let Some(i) = q.receiving.iter().rposition(|&r| r == id) {
            q.receiving.remove(i);
        }
        if let Some(s) = self.sends.get_mut(&id) {
            if s.result.is_none() {
                s.result = Some(result);
                fx.wake.push(s.from);
            } else if s.collected {
                // Replied earlier and the sender already has its result.
                self.sends.remove(&id);
            }
        }
    }

    fn reply(&mut self, ctx: Ctx, result: u64, fx: &mut Effects) -> bool {
        let Some(&id) = self.queue(ctx.key()).receiving.last() else { return false };
        match self.sends.get_mut(&id) {
            Some(s) if s.result.is_none() => {
                s.result = Some(result);
                fx.wake.push(s.from);
                true
            }
            _ => false,
        }
    }

    // ----- timers

    fn set_timer(&mut self, ctx: Ctx, hwnd: u64, id: u64, elapse: u32, proc_: u64) -> Reply {
        let period = u64::from(elapse.clamp(TIMER_MINIMUM, 0x7FFF_FFFF));
        if hwnd != 0 {
            let Some(w) = self.get(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
            let thread = w.thread;
            let h = hwnd as u32;
            match self.timers.iter_mut().find(|t| t.hwnd == h && t.id == id) {
                Some(t) => {
                    t.period = period;
                    t.due = ctx.now + period;
                    t.proc_ = proc_;
                    t.flagged = false;
                }
                None => self.timers.push(Timer {
                    thread,
                    hwnd: h,
                    id,
                    period,
                    due: ctx.now + period,
                    proc_,
                    flagged: false,
                }),
            }
            return Reply::Value { value: if id == 0 { 1 } else { id } };
        }
        let key = ctx.key();
        if id != 0
            && let Some(t) = self.timers.iter_mut().find(|t| t.hwnd == 0 && t.thread == key && t.id == id)
        {
            t.period = period;
            t.due = ctx.now + period;
            t.proc_ = proc_;
            t.flagged = false;
            return Reply::Value { value: id };
        }
        let id = self.next_thread_timer;
        self.next_thread_timer += 1;
        self.timers.push(Timer { thread: key, hwnd: 0, id, period, due: ctx.now + period, proc_, flagged: false });
        Reply::Value { value: id }
    }

    fn kill_timer(&mut self, ctx: Ctx, hwnd: u64, id: u64) -> Reply {
        if hwnd != 0 && !self.is_window(hwnd) {
            return err(ERROR_INVALID_WINDOW_HANDLE);
        }
        let h = hwnd as u32;
        let pos = self.timers.iter().position(|t| {
            if hwnd != 0 { t.hwnd == h && t.id == id } else { t.hwnd == 0 && t.thread.0 == ctx.pid && t.id == id }
        });
        match pos {
            Some(i) => {
                self.timers.remove(i);
                Reply::Bool { value: true }
            }
            None => Reply::Bool { value: false },
        }
    }

    // ----- threads and processes

    /// A thread ended: its windows, queue and timers go; senders waiting
    /// on it get 0 (`IsWindow.yaml`: windows of a thread that has ended are
    /// gone).
    pub fn thread_exited(&mut self, pid: u32, tid: u32, fx: &mut Effects) {
        let key = (pid, tid);
        let mine: Vec<u32> = (1..self.slots.len())
            .filter(|&i| self.slots[i].as_ref().is_some_and(|w| w.thread == key))
            .map(|i| (u32::from(self.gens[i]) << 16) | i as u32)
            .collect();
        for h in mine {
            if h != self.desktop && h != self.message {
                self.free_window(h);
            }
        }
        self.timers.retain(|t| t.thread != key);
        self.clip.thread_gone(pid, tid);
        self.ime.thread_gone(key);
        if let Some(q) = self.queues.remove(&key) {
            for id in q.sent.iter().chain(q.receiving.iter()) {
                if let Some(s) = self.sends.get_mut(id) {
                    s.result.get_or_insert(0);
                    fx.wake.push(s.from);
                }
            }
        }
        self.sends.retain(|_, s| s.from != key);
    }

    /// A process ended: its threads' state and its classes go.
    pub fn process_exited(&mut self, pid: u32, fx: &mut Effects) {
        self.clip.process_gone(pid);
        let threads: Vec<u32> = self.queues.keys().filter(|k| k.0 == pid).map(|k| k.1).collect();
        for t in threads {
            self.thread_exited(pid, t, fx);
        }
        let owned: Vec<u32> = (1..self.slots.len())
            .filter(|&i| self.slots[i].as_ref().is_some_and(|w| w.thread.0 == pid))
            .map(|i| (u32::from(self.gens[i]) << 16) | i as u32)
            .collect();
        for h in owned {
            self.free_window(h);
        }
        for c in &mut self.classes {
            if c.as_ref().is_some_and(|c| c.pid == pid) {
                *c = None;
            }
        }
    }

    /// Whether a thread has something its event should report: posted or
    /// sent messages, a quit request, a reply to its send.
    pub fn has_pending(&self, key: Key) -> bool {
        self.queues
            .get(&key)
            .is_some_and(|q| !q.posted.is_empty() || !q.sent.is_empty() || q.quit.is_some() || !q.raw.is_empty())
            || self.sends.values().any(|s| s.from == key && s.result.is_some())
    }

    /// The thread that owns a queue with this id, if one has a queue.
    pub fn thread_with_queue(&self, tid: u32) -> Option<Key> {
        self.queues.keys().find(|k| k.1 == tid).copied()
    }

    // ----- the request dispatcher

    /// Run one request of thread `ctx`.
    pub fn handle(&mut self, ctx: Ctx, req: Request, fx: &mut Effects) -> Reply {
        self.queue(ctx.key());
        self.now = self.now.max(ctx.now);
        match req {
            Request::Desktop {} => Reply::Desktop {
                desktop: self.desktop,
                message: self.message,
                width: self.screen.0,
                height: self.screen.1,
                work: self.display.work,
                interactive: self.display.interactive,
            },
            Request::RegisterEvent { .. } | Request::Queue {} => Reply::Ok {},
            Request::RegisterClass { spec } => self.register_class(ctx, spec),
            Request::UnregisterClass { class, .. } => self.unregister_class(ctx, class),
            Request::ClassInfo { class, .. } => match self.find_class(ctx.pid, &class) {
                Some(i) => Reply::Class { info: Self::class_info(self.classes[i].as_ref().expect("found")) },
                None => err(ERROR_CLASS_DOES_NOT_EXIST),
            },
            Request::ClassLong { hwnd, index } => self.class_long(hwnd, index),
            Request::ClassName { hwnd } => match self.get(hwnd) {
                Some(w) => {
                    Reply::Text { text: self.classes[w.class].as_ref().map(|c| c.name.clone()).unwrap_or_default() }
                }
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::CreateWindow { spec } => self.create(ctx, spec),
            Request::SetRects { hwnd, window_rel, client_rel } => {
                let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
                if let Some(r) = window_rel {
                    w.window_rel = r;
                }
                if let Some(r) = client_rel {
                    w.client_rel = r;
                    // What is invalid stays inside the client area.
                    let client = crate::rgn::Rgn::rect(Rect::new(0, 0, r.width().max(0), r.height().max(0)));
                    w.paint.upd = w.paint.upd.intersect(&client);
                }
                Reply::Ok {}
            }
            Request::FinishCreate { hwnd } => self.finish_create(hwnd),
            Request::Free { hwnd } => {
                if hwnd != self.desktop && hwnd != self.message {
                    self.free_window(hwnd);
                }
                Reply::Ok {}
            }
            Request::Window { hwnd } => match self.info(hwnd as u32).filter(|_| self.is_window(hwnd)) {
                Some(info) => Reply::Window { info },
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::Relatives { hwnd } => self.relatives(hwnd),
            Request::GetLong { hwnd, index, size } => self.get_long(hwnd, index, size),
            Request::SetLong { hwnd, index, size, value } => self.set_long(hwnd, index, size, value),
            Request::SetStyle { hwnd, which, value } => self.set_style(hwnd, which, value),
            Request::GetText { hwnd } => match self.get(hwnd) {
                Some(w) => Reply::Text { text: w.text.clone() },
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::SetText { hwnd, text } => match self.get_mut(u64::from(hwnd)) {
                Some(w) => {
                    w.text = text;
                    Reply::Ok {}
                }
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::ZOrder { hwnd, after, apply } => self.z_order(hwnd, after, apply),
            Request::SetParent { hwnd, parent } => self.set_parent(hwnd, parent),
            Request::SetState { hwnd, visible, minimized, maximized, normal, size_move_pending } => {
                let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
                let was_visible = w.style & WS_VISIBLE != 0;
                if let Some(v) = visible {
                    if v {
                        w.style |= WS_VISIBLE;
                    } else {
                        w.style &= !WS_VISIBLE;
                    }
                }
                match visible {
                    Some(false) if was_visible => self.clear_paint_tree(hwnd),
                    Some(true) if !was_visible => self.shown(hwnd, fx),
                    _ => {}
                }
                let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
                if let Some(m) = minimized {
                    w.minimized = m;
                    if m {
                        w.style |= WS_MINIMIZE;
                    } else {
                        w.style &= !WS_MINIMIZE;
                    }
                }
                if let Some(m) = maximized {
                    w.maximized = m;
                    if m {
                        w.style |= WS_MAXIMIZE;
                    } else {
                        w.style &= !WS_MAXIMIZE;
                    }
                }
                if let Some(r) = normal {
                    w.normal = r;
                }
                if let Some(p) = size_move_pending {
                    w.size_move_pending = p;
                }
                Reply::Ok {}
            }
            Request::FindWindow { class, title, parent, after } => self.find_window(ctx, class, title, parent, after),
            Request::GetWindow { hwnd, cmd } => self.get_window(hwnd, cmd),
            Request::Ancestor { hwnd, flags } => self.ancestor(hwnd, flags),
            Request::GetParent { hwnd } => {
                if !self.is_window(hwnd) {
                    return err(ERROR_INVALID_WINDOW_HANDLE);
                }
                Reply::Handle { hwnd: self.parent_of(hwnd as u32) }
            }
            Request::IsChild { parent, child } => Reply::Bool { value: self.is_child(parent, child) },
            Request::SetDc { hwnd, class, dc } => {
                let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
                if class {
                    let c = w.class;
                    if let Some(c) = self.classes[c].as_mut() {
                        c.dc = dc;
                    }
                } else {
                    w.own_dc = dc;
                }
                Reply::Ok {}
            }
            Request::Clip { op } => self.clip_op(ctx, op, fx),
            Request::Layered { hwnd, op, key, alpha, flags } => self.layered_op(hwnd, op, key, alpha, flags),
            Request::Ime { op } => Reply::Ime { res: self.ime.run(ctx.key(), op) },
            Request::ImeAssoc { hwnd, code } => match self.get_mut(u64::from(hwnd)) {
                Some(w) => Reply::Value { value: u64::from(core::mem::replace(&mut w.ime, code)) },
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::CursorShape { .. } | Request::ScreenPixel { .. } => Reply::Ok {},
            Request::RegisterMessage { name } => Reply::Atom { atom: self.atom_for(&name) },
            Request::Prop { hwnd, op, key, value } => self.prop_op(hwnd, op, key, value),
            Request::Dpi { set, process, awareness } => self.dpi_op(ctx, set, process, awareness),
            Request::Post { hwnd, msg, w, l } => {
                if hwnd == 0 {
                    return self.post(ctx, ctx.key(), 0, msg, w, l, fx);
                }
                let Some(win) = self.get(hwnd) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
                let to = win.thread;
                self.post(ctx, to, hwnd as u32, msg, w, l, fx)
            }
            Request::PostThread { tid, msg, w, l } => match self.thread_with_queue(tid) {
                Some(to) if tid != 0 => self.post(ctx, to, 0, msg, w, l, fx),
                _ => err(ERROR_INVALID_THREAD_ID),
            },
            Request::PostQuit { code } => {
                let q = self.queue(ctx.key());
                q.quit = Some(code);
                q.changed |= QS_POSTMESSAGE;
                Reply::Ok {}
            }
            Request::Peek { filter, min, max, flags } => {
                let r = self.peek(ctx, filter, min, max, flags);
                if let Reply::Message { msg } = &r {
                    self.queue(ctx.key()).last = (msg.time, msg.x, msg.y);
                }
                r
            }
            // The surfaces are the NT personality's (`crate::surfaces`).
            Request::Surface { .. } => Reply::Value { value: 0 },
            Request::Redraw { hwnd, rects, whole, flags } => {
                if !self.is_window(u64::from(hwnd)) {
                    return err(ERROR_INVALID_WINDOW_HANDLE);
                }
                let area = if whole { None } else { Some(crate::rgn::Rgn::from_rects(&rects)) };
                self.redraw(hwnd, area, flags, fx);
                Reply::Ok {}
            }
            Request::Update { hwnd } => self.update_of(hwnd),
            Request::Painted { hwnd, what, ferase } => {
                self.painted(hwnd, what, ferase);
                Reply::Ok {}
            }
            Request::InputState {} => self.input_reply(ctx.key()),
            Request::SetInput { active, focus, capture, which } => {
                let had_capture = self.input_of(ctx.key()).capture;
                self.set_input(ctx.key(), active, focus, capture, which);
                // Releasing the capture synthesizes a move to the window under
                // the cursor (SetCapture.yaml ordering).
                if which & input::SET_CAPTURE != 0 && capture == 0 && had_capture != 0 {
                    self.queue_mouse(ctx, raw::WM_MOUSEMOVE, 0, fx);
                    self.mark_synthetic_move(ctx.key());
                } else if which & input::SET_CAPTURE != 0 && capture != 0 {
                    // A new capture takes the unretrieved synthesized move
                    // with it (ReleaseCapture.yaml rc_nocapture: no captured
                    // WM_MOUSEMOVE before the next release).
                    self.drop_synthetic_move(ctx.key());
                }
                Reply::Ok {}
            }
            Request::ThreadInput { tid } => self.thread_input(ctx.pid, tid),
            Request::GiveForeground { hwnd } => {
                self.give_foreground(hwnd);
                Reply::Ok {}
            }
            Request::AttachInput { tid, attach } => self.attach_input(ctx.key(), tid, attach),
            Request::SendInput { events } => Reply::Value { value: u64::from(self.send_input(ctx, &events, fx)) },
            Request::KeyboardState {} => Reply::Keys { keys: self.keys_of(ctx.key()) },
            Request::SetKeyboardState { keys } => {
                self.set_keys(ctx.key(), keys);
                Reply::Ok {}
            }
            Request::AsyncKeyState { vk } => Reply::Value { value: u64::from(self.async_key(vk)) },
            Request::WindowFromPoint { x, y, below } => {
                let h = self.window_from_point_below(x, y, true, below);
                // Nothing of ours there: another process's window.
                let h = if h == self.desktop && self.keeper != 0 { self.keeper } else { h };
                Reply::Handle { hwnd: h }
            }
            Request::TrackMouse { hwnd, flags, hover } => self.track_mouse(ctx, hwnd, flags, hover, fx),
            Request::Cursor { set, x, y } => {
                if set {
                    self.cursor = (x.clamp(0, self.screen.0 - 1), y.clamp(0, self.screen.1 - 1));
                    // A move to the window under it, even to the same point
                    // (SetCursorPos.yaml).
                    self.queue_mouse(ctx, raw::WM_MOUSEMOVE, 0, fx);
                }
                Reply::Value { value: u64::from(self.cursor.0 as u32) | (u64::from(self.cursor.1 as u32) << 32) }
            }
            Request::PaintOrder { root } => {
                Reply::Windows { hwnds: self.paint_order(ctx.key(), (root != 0).then_some(root)) }
            }
            Request::MessageInfo {} => {
                let (t, x, y) = self.queue(ctx.key()).last;
                Reply::Value { value: u64::from(t) | (u64::from(x as u16) << 32) | (u64::from(y as u16) << 48) }
            }
            Request::SendStart { hwnd, msg, w, l } => self.send_start(ctx, hwnd, msg, w, l, fx),
            Request::SendWait { id } => self.send_wait(ctx, id),
            Request::SendDone { id, result } => {
                self.send_done(ctx, id, result, fx);
                Reply::Ok {}
            }
            Request::Reply { result } => Reply::Bool { value: self.reply(ctx, result, fx) },
            Request::InSend {} => Reply::Bool { value: !self.queue(ctx.key()).receiving.is_empty() },
            Request::QueueStatus { flags, clear } => {
                let key = ctx.key();
                self.refresh_timers(key, ctx.now);
                let now = self.status_now(key, ctx.now);
                let next_timer_ms = self
                    .timers
                    .iter()
                    .filter(|t| t.thread == key && !t.flagged)
                    .map(|t| t.due.saturating_sub(ctx.now))
                    .min();
                let q = self.queue(key);
                let changed = q.changed;
                if clear {
                    q.changed &= !flags;
                }
                Reply::Bits { now: now & flags, changed: changed & flags, next_timer_ms }
            }
            Request::SetTimer { hwnd, id, elapse, timer_proc } => self.set_timer(ctx, hwnd, id, elapse, timer_proc),
            Request::KillTimer { hwnd, id } => self.kill_timer(ctx, hwnd, id),
            Request::TimerProc { hwnd, id, timer_proc } => Reply::Bool {
                value: timer_proc != 0
                    && self
                        .timers
                        .iter()
                        .any(|t| u64::from(t.hwnd) == hwnd & 0xFFFF_FFFF && t.id == id && t.proc_ == timer_proc),
            },
        }
    }
}

impl Window {
    fn new(thread: Key, class: usize, style: u32, rect: Rect) -> Window {
        Window {
            thread,
            class,
            style,
            ex_style: 0,
            window_rel: rect,
            client_rel: rect,
            parent: 0,
            owner: 0,
            children: Vec::new(),
            text: Vec::new(),
            wndproc: 0,
            unicode: true,
            instance: 0,
            id: 0,
            userdata: 0,
            extra: Vec::new(),
            state: State::Normal,
            minimized: false,
            maximized: false,
            normal: Rect::default(),
            own_dc: 0,
            size_move_pending: false,
            seq: 0,
            paint: paint::Paint::default(),
            dpi: 0,
            props: Vec::new(),
            lay: guic::Layered::default(),
            ime: u32::MAX,
        }
    }
}

fn err(code: u32) -> Reply {
    Reply::Error { code }
}

fn read_le(b: &[u8], at: usize, size: usize) -> u64 {
    let mut v = [0u8; 8];
    v[..size].copy_from_slice(&b[at..at + size]);
    u64::from_le_bytes(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Ctx = Ctx { pid: 7, tid: 1, now: 0 };
    const B: Ctx = Ctx { pid: 7, tid: 2, now: 0 };

    fn name(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn reg(wm: &mut Wm, n: &str) -> u16 {
        let spec = ClassSpec { name: name(n), wndproc: 0x1000, unicode: true, ..ClassSpec::default() };
        match wm.handle(A, Request::RegisterClass { spec }, &mut Effects::default()) {
            Reply::Atom { atom } => atom,
            r => panic!("{r:?}"),
        }
    }

    fn create(wm: &mut Wm, ctx: Ctx, n: &str, style: u32, parent: u64, menu: u64) -> u32 {
        let spec = CreateSpec {
            class: ClassRef::Name { name: name(n) },
            style,
            cx: 10,
            cy: 10,
            parent,
            menu,
            ..CreateSpec::default()
        };
        match wm.handle(ctx, Request::CreateWindow { spec }, &mut Effects::default()) {
            Reply::Created { created } => {
                wm.handle(ctx, Request::FinishCreate { hwnd: created.hwnd }, &mut Effects::default());
                created.hwnd
            }
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn classes_register_once_and_keep_their_atom() {
        let mut wm = Wm::new();
        let a = reg(&mut wm, "MyClass");
        assert!(a >= 0xC000);
        let spec = ClassSpec { name: name("MYCLASS"), ..ClassSpec::default() };
        assert_eq!(
            wm.handle(A, Request::RegisterClass { spec }, &mut Effects::default()),
            err(ERROR_CLASS_ALREADY_EXISTS)
        );
        let un =
            |wm: &mut Wm, c| wm.handle(A, Request::UnregisterClass { class: c, instance: 0 }, &mut Effects::default());
        assert_eq!(un(&mut wm, ClassRef::Name { name: name("myclass") }), Reply::Ok {});
        assert_eq!(un(&mut wm, ClassRef::Name { name: name("myclass") }), err(ERROR_CLASS_DOES_NOT_EXIST));
        assert_eq!(un(&mut wm, ClassRef::Atom { atom: a }), err(ERROR_INVALID_HANDLE));
        assert_eq!(reg(&mut wm, "MyClass"), a);
        // System classes are found, not unregistered.
        assert!(matches!(
            wm.handle(A, Request::ClassInfo { class: ClassRef::Name { name: name("button") }, instance: 0 }, &mut Effects::default()),
            Reply::Class { info } if info.style == 0x8b && info.wnd_extra == 8
        ));
        assert_eq!(un(&mut wm, ClassRef::Name { name: name("Button") }), err(ERROR_CLASS_DOES_NOT_EXIST));
    }

    #[test]
    fn handles_change_on_reuse_and_children_go_last() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let p = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        wm.handle(A, Request::Free { hwnd: p }, &mut Effects::default());
        let q = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        assert_ne!(p, q);
        assert!(!wm.is_window(u64::from(p)));
        let a = create(&mut wm, A, "C", WS_CHILD, u64::from(q), 1);
        let b = create(&mut wm, A, "C", WS_CHILD, u64::from(q), 2);
        assert_eq!(wm.get_window(u64::from(q), 5), Reply::Handle { hwnd: a });
        assert_eq!(wm.get_window(u64::from(a), 2), Reply::Handle { hwnd: b });
        assert_eq!(
            wm.handle(A, Request::ZOrder { hwnd: a, after: HWND_TOP, apply: true }, &mut Effects::default()),
            Reply::Bool { value: false }
        );
        assert_eq!(
            wm.handle(A, Request::ZOrder { hwnd: a, after: u64::from(b), apply: true }, &mut Effects::default()),
            Reply::Bool { value: true }
        );
        assert_eq!(wm.get_window(u64::from(q), 5), Reply::Handle { hwnd: b });
        // Top-level: the newest first.
        let r = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        assert_eq!(wm.find_window(A, Some(ClassRef::Name { name: name("c") }), None, 0, 0), Reply::Handle { hwnd: r });
    }

    #[test]
    fn posted_messages_quit_and_timers_in_their_order() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let h = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        let mut fx = Effects::default();
        let at = |now| Ctx { now, ..A };
        wm.handle(A, Request::SetTimer { hwnd: u64::from(h), id: 1, elapse: 10, timer_proc: 0 }, &mut fx);
        wm.handle(A, Request::Post { hwnd: u64::from(h), msg: 0x401, w: 1, l: 0 }, &mut fx);
        wm.handle(A, Request::PostQuit { code: 5 }, &mut fx);
        wm.handle(A, Request::Post { hwnd: 0, msg: 0x402, w: 2, l: 0 }, &mut fx);
        let next = |wm: &mut Wm, now| match wm.handle(
            at(now),
            Request::Peek { filter: 0, min: 0, max: 0, flags: PM_REMOVE },
            &mut Effects::default(),
        ) {
            Reply::Message { msg } => (msg.msg, msg.w),
            r => panic!("{r:?}"),
        };
        assert_eq!(next(&mut wm, 50), (0x401, 1));
        assert_eq!(next(&mut wm, 50), (0x402, 2));
        assert_eq!(next(&mut wm, 50), (WM_QUIT, 5));
        assert_eq!(next(&mut wm, 50), (WM_TIMER, 1));
        assert!(matches!(
            wm.handle(at(50), Request::Peek { filter: 0, min: 0, max: 0, flags: PM_REMOVE }, &mut Effects::default()),
            Reply::Empty { next_timer_ms: Some(10) }
        ));
    }

    #[test]
    fn desktop_moves_update_win32_geometry_and_close_is_posted_to_its_thread() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let hwnd = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        let client = Rect::new(200, 300, 584, 561);
        assert!(wm.configure_from_desktop(hwnd, client, FrameMetrics::POLY));
        let info = wm.info(hwnd).unwrap();
        assert_eq!(info.client, client);
        assert_eq!(info.window, Rect::new(199, 275, 585, 562));
        assert_eq!(info.window_rel, info.window);

        let mut fx = Effects::default();
        assert!(wm.post_system_message(hwnd, crate::user::WM_CLOSE, 77, &mut fx));
        assert_eq!(fx.wake, [(A.pid, A.tid)]);
        let Reply::Message { msg } =
            wm.handle(Ctx { now: 77, ..A }, Request::Peek { filter: 0, min: 0, max: 0, flags: PM_REMOVE }, &mut fx)
        else {
            panic!("the compositor close request was not queued")
        };
        assert_eq!((msg.hwnd, msg.msg, msg.time), (hwnd, crate::user::WM_CLOSE, 77));
    }

    #[test]
    fn a_send_to_another_thread_waits_for_its_result() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let h = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        let mut fx = Effects::default();
        let Reply::Started { id } =
            wm.handle(B, Request::SendStart { hwnd: u64::from(h), msg: 0x400, w: 0, l: 0 }, &mut fx)
        else {
            panic!()
        };
        assert_eq!(fx.wake, [(7, 1)]);
        assert_eq!(wm.handle(B, Request::SendWait { id }, &mut fx), Reply::Pending {});
        let Reply::Bits { now, .. } = wm.handle(A, Request::QueueStatus { flags: 0xff, clear: false }, &mut fx) else {
            panic!()
        };
        assert_eq!(now, QS_SENDMESSAGE);
        assert!(matches!(
            wm.handle(A, Request::Peek { filter: 0, min: 0, max: 0, flags: 0 }, &mut fx),
            Reply::Incoming { .. }
        ));
        assert_eq!(wm.handle(A, Request::InSend {}, &mut fx), Reply::Bool { value: true });
        assert_eq!(wm.handle(A, Request::Reply { result: 77 }, &mut fx), Reply::Bool { value: true });
        assert_eq!(wm.handle(B, Request::SendWait { id }, &mut fx), Reply::Done { result: 77 });
        wm.handle(A, Request::SendDone { id, result: 99 }, &mut fx);
        assert_eq!(wm.handle(A, Request::InSend {}, &mut fx), Reply::Bool { value: false });
    }
}
