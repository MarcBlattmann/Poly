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

pub mod dpi;
pub mod input;
pub mod paint;
pub mod raw;

// Win32 errors (`winerror.h`).
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_MORE_DATA: u32 = 234;
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

/// The `msg` of an [`Incoming`] with no window procedure (`wndproc` 0) that
/// the compositor's resize of a window hands its thread
/// ([`Wm::configure_from_desktop`]): `user32` calls `SetWindowPos` for the
/// window with the rectangle [`unpack_set_pos`] reads from `w` and `l`, and
/// the app never sees the message itself. Not a window message (above
/// 0xFFFF, where none is).
pub const DESKTOP_SET_POS: u32 = 0x5059_0001;

/// The sender of the desktop's requests: no thread (process 0 is the
/// system's desktop, thread 0 none of it).
const DESKTOP_SENDER: Key = (SYSTEM_DESKTOP_PID, 0);

/// `w` and `l` of a [`DESKTOP_SET_POS`]: the left and top, the width and
/// height, 32 bits each, the first in the low half.
pub fn pack_set_pos(r: Rect) -> (u64, u64) {
    let pair = |a: i32, b: i32| u64::from(a as u32) | (u64::from(b as u32) << 32);
    (pair(r.left, r.top), pair(r.width(), r.height()))
}

/// The left, top, width and height of a [`DESKTOP_SET_POS`].
pub fn unpack_set_pos(w: u64, l: u64) -> (i32, i32, i32, i32) {
    (w as u32 as i32, (w >> 32) as u32 as i32, l as u32 as i32, (l >> 32) as u32 as i32)
}

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
    /// Layered-window attributes (`WS_EX_LAYERED`, ADR 0172).
    lay: guic::Layered,
    /// The input-context association (`ime::Assoc::code`).
    ime: u32,
    /// The window region (`SetWindowRgn`), in window coordinates.
    region: Option<crate::rgn::Rgn>,
    /// A top-level window that lies below the windows of other processes:
    /// minimizing the active window puts it at the bottom of the z order,
    /// raising it brings it back (ShowWindow.yaml `z_order`). The list
    /// order alone cannot say that for a process's only window.
    below_others: bool,
    /// Minimized while it was maximized: restoring it maximizes it again
    /// (ShowWindow.yaml `restore`).
    restore_max: bool,
    /// `ptMinPosition` of its window placement (`WinInfo::min_x`).
    min_pos: (i32, i32),
    /// Its thread's default IME window in the interactive session: in the
    /// z order it follows the thread's active window (`input`, ADR 0176).
    helper: bool,
    /// The menu bar's height above the client area, inside the surface.
    menu_bar: i32,
    /// Its rectangles in its own units while they map to the physical ones (`dpi`, ADR 0231).
    own: Option<dpi::Own>,
    /// The DPI its frame stays measured at after a scale change until `DefWindowProc` handles
    /// `WM_DPICHANGED` (0: its DPI).
    frame_pin: u32,
    /// What `WM_DPICHANGED` suggested last (`dpiscale::suggest`).
    suggest: Option<crate::dpiscale::SuggestBase>,
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
    /// The windows that were active in this queue, the latest first (also
    /// ones destroyed since): who takes the activation when the active
    /// window goes (GetForegroundWindow.yaml `handover_same_thread`).
    activated: Vec<u32>,
    /// Input messages waiting (`raw`).
    raw: VecDeque<Msg>,
    /// The thread's own key state after SetKeyboardState (`raw`).
    keys: Option<Vec<u8>>,
    /// The queue's caret and caret position (`caret`).
    caret: crate::caret::CaretQueue,
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

impl Wm {
    /// One line per thread queue that holds anything, for a wait report (`poly.nt.waits`, ADR
    /// 0155 step 12): the posted messages (`msg@hwnd`, first eight), the `changed` wake bits,
    /// the sends waiting and the input messages. A thread that waits on its queue event while
    /// its queue holds a message is a lost wakeup.
    pub fn queue_debug_lines(&self) -> Vec<alloc::string::String> {
        self.queues
            .iter()
            .filter(|(_, q)| !q.posted.is_empty() || !q.sent.is_empty() || !q.raw.is_empty() || q.changed != 0)
            .map(|(k, q)| {
                let posted: Vec<alloc::string::String> =
                    q.posted.iter().take(8).map(|m| alloc::format!("{:#x}@{:#x}", m.msg, m.hwnd)).collect();
                alloc::format!(
                    "queue p{} t{}: posted {} [{}] changed {:#x} sent {} receiving {} raw {} quit {:?}",
                    k.0,
                    k.1,
                    q.posted.len(),
                    posted.join(" "),
                    q.changed,
                    q.sent.len(),
                    q.receiving.len(),
                    q.raw.len(),
                    q.quit
                )
            })
            .collect()
    }
}

/// Where a window goes in its parent's list ([`Wm::z_plan`]).
struct ZPlan {
    parent: u32,
    was_below: bool,
    old: Vec<u32>,
    new: Vec<u32>,
    below: bool,
    raised: Vec<u32>,
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
    /// The keyboard layout of the desktop (ADR 0209): German without one.
    layout: crate::kbd::Layout,
    /// The session's screen (ADR 0131).
    display: Display,
    desktop: u32,
    message: u32,
    seq: u64,
    cascade: i32,
    /// The foreground window (`input`).
    foreground: u32,
    /// The interactive session's windows of other processes (`input`): the
    /// keeper, the hidden window of its process, the console window.
    keeper: u32,
    keeper_aux: u32,
    other: u32,
    /// The window of another process the foreground goes back to when a
    /// thread of ours gives it up: the one it was taken from.
    fg_return: u32,
    /// AttachThreadInput: a thread to the queue whose input it shares.
    attached: BTreeMap<Key, Key>,
    /// The right to take the foreground by activating a window (`input`):
    /// the processes that had the foreground at some time (one that never
    /// had it may take it: the foreground process started it), and the
    /// input queues that have the right since.
    fg_seen: alloc::collections::BTreeSet<u32>,
    fg_right: alloc::collections::BTreeSet<Key>,
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
    /// The last button-down that may start a double click: (message,
    /// window, x, y, time) (`raw`).
    last_click: Option<(u32, u32, i32, i32, u64)>,
    /// The time of the last request (ms), for input from the desktop link.
    now: u64,
    /// The session's clipboard (ADR 0172).
    clip: crate::clip::Clipboard,
    /// Input contexts (`ime`).
    ime: crate::ime::Ime,
    /// DPI awareness: processes that set theirs, and threads' own (`guic`).
    proc_dpi: BTreeMap<u32, u8>,
    /// Processes whose manifest declares Windows 8 or later
    /// (`Request::LayeredChildren`): they may create layered child windows.
    layered_children: alloc::collections::BTreeSet<u32>,
    thread_dpi: BTreeMap<Key, u8>,
    /// The system DPI: 96 times the desktop link's units (ADR 0219), or the session's scale
    /// (ADR 0231).
    system_dpi: u32,
    /// The monitor's DPI now: the system DPI until the scale changes (ADR 0231).
    monitor_dpi: u32,
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
            layout: crate::kbd::Layout::German,
            display: Display::SERVICE,
            desktop: 0,
            message: 0,
            seq: 0,
            cascade: 0,
            foreground: 0,
            keeper: 0,
            keeper_aux: 0,
            other: 0,
            fg_return: 0,
            attached: BTreeMap::new(),
            fg_seen: alloc::collections::BTreeSet::new(),
            fg_right: alloc::collections::BTreeSet::new(),
            cursor: (0, 0),
            async_keys: raw::blank_keys(),
            async_new: alloc::vec![false; 256],
            sync_keys: raw::blank_keys(),
            tracking: Vec::new(),
            hover: None,
            last_click: None,
            now: 0,
            clip: crate::clip::Clipboard::new(),
            ime: crate::ime::Ime::default(),
            proc_dpi: BTreeMap::new(),
            layered_children: alloc::collections::BTreeSet::new(),
            thread_dpi: BTreeMap::new(),
            system_dpi: 96,
            monitor_dpi: 96,
        };
        // Index 0 is never a window.
        wm.slots.push(None);
        wm.gens.push(0);
        // The system classes: found by every process (Learn: "About Window
        // Classes", system classes; `RegisterClassExW.yaml`: "Button" style
        // 0x8b, 8 extra bytes).
        // The procedures are `dlg::sysproc` numbers `user32` runs; the desktop's (last) has none.
        // Styles and extra bytes of the classes `Button` and the desktop are observed
        // (`RegisterClassExW.yaml`); the others are `CS_DBLCLKS | CS_PARENTDC`-like guesses
        // that no sheet contradicts.
        for (name, style, extra, wndproc) in [
            ("Button", 0x8b, 8, crate::dlg::sysproc::BUTTON),
            ("Static", 0x88, 0, crate::dlg::sysproc::STATIC),
            ("Edit", 0x88, 6, crate::dlg::sysproc::EDIT),
            ("ListBox", 0x88, 8, crate::dlg::sysproc::LISTBOX),
            ("ComboBox", 0x88, 8, crate::dlg::sysproc::COMBOBOX),
            ("ScrollBar", 0x08, 8, crate::dlg::sysproc::SCROLLBAR),
            ("ComboLBox", 0x808, 8, crate::dlg::sysproc::COMBOLBOX),
            ("#32770", 0x808, 30, crate::dlg::sysproc::DIALOG),
            ("#32769", 0x08, 0, 0),
        ] {
            let name: Vec<u16> = name.encode_utf16().collect();
            let atom = wm.atom_for(&name);
            wm.classes.push(Some(Class {
                pid: SYSTEM_DESKTOP_PID,
                atom,
                name,
                style,
                wndproc,
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
    /// The system DPI: 96, or 96 times the desktop link's units when Windows apps live in output
    /// pixels (ADR 0219).
    pub fn set_system_dpi(&mut self, dpi: u32) {
        self.system_dpi = dpi.max(1);
        self.monitor_dpi = self.system_dpi;
    }

    pub fn system_dpi(&self) -> u32 {
        self.system_dpi
    }

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

    /// A window's layered state, if it has `WS_EX_LAYERED`.
    pub fn layered(&self, h: u32) -> Option<guic::Layered> {
        let w = self.get(u64::from(h))?;
        (w.ex_style & guic::WS_EX_LAYERED != 0).then_some(w.lay)
    }

    /// Whether window `h` has a region (`SetWindowRgn`) that leaves out
    /// the screen point: nothing of the window is drawn or hit there
    /// (`SetWindowRgn.yaml` `shape_hit_test`).
    pub fn region_excludes(&self, h: u32, x: i32, y: i32) -> bool {
        let Some(w) = self.get(u64::from(h)) else { return false };
        let Some(rgn) = &w.region else { return false };
        let (ox, oy) = self.parent_origin(h);
        !rgn.contains(x - ox - w.window_rel.left, y - oy - w.window_rel.top)
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
            restore_max: w.restore_max,
            min_x: w.min_pos.0,
            min_y: w.min_pos.1,
            own_dc: w.own_dc,
            class_dc: class.map_or(0, |c| c.dc),
            size_move_pending: w.size_move_pending,
            desktop: h == self.desktop,
            message_only: w.parent == self.message,
            dpi: if h == self.desktop { guic::AWARE_PER_MONITOR } else { w.dpi },
            dpi_value: self.window_dpi(h),
            frame_dpi: if w.frame_pin != 0 { w.frame_pin } else { self.window_dpi(h) },
            creating: w.state == State::Creating,
            ime: w.ime,
            lay_ready: w.ex_style & guic::WS_EX_LAYERED == 0 || w.lay.attrs || w.lay.update,
            menu_bar: w.menu_bar,
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

    /// `SetClassLongPtrW`: the class's value is replaced and the old one answered. The icon, cursor, brush, menu name,
    /// window procedure, instance and style are the class's own fields; an offset into the class's extra bytes sets
    /// eight of them. The atom and the sizes cannot be set (`ERROR_INVALID_INDEX`).
    fn set_class_long(&mut self, hwnd: u64, index: i32, value: u64) -> Reply {
        let Some(class) = self.get(hwnd).map(|w| w.class) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let Some(c) = self.classes[class].as_mut() else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let old = match index {
            -26 => core::mem::replace(&mut c.style, value as u32) as u64,
            -24 => core::mem::replace(&mut c.wndproc, value),
            -16 => core::mem::replace(&mut c.instance, value),
            -14 => core::mem::replace(&mut c.icon, value),
            -12 => core::mem::replace(&mut c.cursor, value),
            -10 => core::mem::replace(&mut c.brush, value),
            -8 => core::mem::replace(&mut c.menu_name, value),
            -34 => core::mem::replace(&mut c.icon_sm, value),
            i if i >= 0 && (i as usize) + 8 <= c.extra.len() => {
                let old = read_le(&c.extra, i as usize, 8);
                c.extra[i as usize..i as usize + 8].copy_from_slice(&value.to_le_bytes());
                old
            }
            _ => return err(ERROR_INVALID_INDEX),
        };
        Reply::Value { value: old }
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
        // A layered child window needs a manifest that declares Windows 8
        // support: without one it is not created, and no last error is set
        // (SetLayeredWindowAttributes.yaml `children`; Learn, "Layered
        // Windows": Windows 8 supports them for child windows).
        if child && s.ex_style & guic::WS_EX_LAYERED != 0 && !self.layered_children.contains(&ctx.pid) {
            return err(0);
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
        // WS_EX_WINDOWEDGE is computed, not kept (ShowWindow.yaml
        // `tiled_styles`): a popup has it with a dialog or sizing frame
        // unless it has a static edge.
        let edge = if s.style & (WS_POPUP | WS_CHILD) == WS_POPUP {
            crate::metrics::popup_window_edge(s.style, ex)
        } else {
            s.style & (WS_DLGFRAME | WS_THICKFRAME) != 0 || ex & WS_EX_DLGMODALFRAME != 0
        };
        if edge {
            ex |= WS_EX_WINDOWEDGE;
        } else {
            ex &= !WS_EX_WINDOWEDGE;
        }
        let (wndproc, unicode, extra, class_name) = {
            let c = self.classes[class].as_mut().expect("found");
            c.windows += 1;
            // A window of a system class made through an `A` function is an ANSI window.
            let unicode = c.unicode && !(s.ansi && c.pid == SYSTEM_DESKTOP_PID);
            (c.wndproc, unicode, c.wnd_extra.max(0) as usize, c.name.clone())
        };
        // WM_GETMINMAXINFO at creation only for a window that is not
        // WS_POPUP or has a sizing frame (ShowWindow.yaml
        // `show_minimized_from_hidden`).
        let minmax = parent == self.desktop && crate::metrics::track_limited(s.style);
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
        // A top-level window's `id` is its menu handle (user32 checks it is one).
        win.id = s.menu;
        win.extra = vec![0; extra];
        win.state = State::Creating;
        win.dpi =
            if child { self.get(u64::from(parent)).map_or(0, |p| p.dpi) } else { self.thread_awareness(ctx.key()) };
        // The thread's default IME window (class "IME", owned by the
        // thread's first window) takes part in the z order of the
        // interactive session: it is in its owner's band and is made
        // directly below it (GetFocus.yaml `ime_window`).
        let helper = self.display.interactive
            && parent == self.desktop
            && owner != 0
            && class_name.len() == 3
            && upcase(&class_name) == [73, 77, 69];
        if helper && self.get(u64::from(owner)).is_some_and(|o| o.ex_style & WS_EX_TOPMOST != 0) {
            win.ex_style |= WS_EX_TOPMOST;
        }
        win.helper = helper;
        let topmost = win.ex_style & WS_EX_TOPMOST != 0;
        self.seq += 1;
        win.seq = self.seq;
        let hwnd = self.alloc(win);
        // Top-level windows go on top of their band (the topmost windows
        // lead the list); children below their siblings
        // (`SetWindowPos.yaml`: GW_CHILD is the first created).
        let at = if parent == self.desktop {
            let mut flags = if topmost { crate::zorder::TOPMOST } else { 0 };
            if helper {
                flags |= crate::zorder::HELPER;
            }
            let new = crate::zorder::Top { hwnd, queue: self.input_key(ctx.key()), flags, owner };
            let list = self.tops();
            // The text services' window lies directly above the default IME window that owns it.
            let ime_ui = self.display.interactive
                && owner != 0
                && upcase(&class_name) == upcase(&"MSCTFIME UI".encode_utf16().collect::<Vec<u16>>());
            let slot = match list.iter().position(|t| t.hwnd == owner) {
                Some(i) if ime_ui => i,
                _ => crate::zorder::new_slot(&list, &new, self.barrier(ctx.key())),
            };
            // `slot` counts the windows the z rules see; the text services' windows lie among them.
            let real = self.get(u64::from(self.desktop)).map(|d| d.children.clone()).unwrap_or_default();
            match list.get(slot).and_then(|t| real.iter().position(|&c| c == t.hwnd)) {
                Some(i) => i,
                None => real.len(),
            }
        } else {
            0
        };
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

    /// The default IME window of thread `tid`: its top-level window of
    /// class "IME" (ImmGetDefaultIMEWnd.yaml), 0 if it has none.
    fn default_ime(&self, tid: u32) -> u32 {
        let Some(d) = self.get(u64::from(self.desktop)) else { return 0 };
        d.children
            .iter()
            .copied()
            .find(|&h| {
                self.get(u64::from(h)).is_some_and(|w| {
                    w.thread.1 == tid && self.classes[w.class].as_ref().is_some_and(|c| upcase(&c.name) == [73, 77, 69])
                })
            })
            .unwrap_or(0)
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
        self.rehome_helpers(hwnd);
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
            q.caret.window_gone(hwnd);
        }
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
            -12 => core::mem::replace(&mut w.id, value),
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

    /// Where `SetWindowPos` puts a window among its siblings (`apply`:
    /// do it), and whether that is a move. Top-level windows follow the
    /// model of `crate::zorder` (ADR 0176): the top and the bottom are
    /// those of the window's band, owned windows come along above a raised
    /// owner, and a thread without the foreground right stays below the
    /// foreground process's windows.
    /// The window `hwnd` would lie directly after if it were put at `after`: what a
    /// `WM_WINDOWPOSCHANGING` of a top-level window reports as `hwndInsertAfter` (0: the top).
    /// `HWND_TOP` of a window that is not topmost comes after the topmost ones, `HWND_BOTTOM`
    /// after the last window (ShowWindow.yaml `chrome_show`, SetWindowPos.yaml `chrome_resize`).
    fn z_report(&mut self, hwnd: u32, after: u64) -> Reply {
        let Some(w) = self.get(u64::from(hwnd)) else { return Reply::Handle { hwnd: 0 } };
        if w.parent != self.desktop || hwnd == self.desktop {
            return Reply::Handle { hwnd: after as u32 };
        }
        if self.is_ime_ui(hwnd) {
            // It lies directly above its owner: the window above it now.
            let list = self.get(u64::from(self.desktop)).map(|d| d.children.clone()).unwrap_or_default();
            let at = list.iter().position(|&c| c == hwnd);
            return Reply::Handle { hwnd: at.and_then(|i| i.checked_sub(1)).map_or(0, |i| list[i]) };
        }
        let Some(new) = self.z_plan(hwnd, after).map(|p| p.new) else { return Reply::Handle { hwnd: after as u32 } };
        let at = new.iter().position(|&c| c == hwnd);
        Reply::Handle { hwnd: at.and_then(|i| i.checked_sub(1)).map_or(0, |i| new[i]) }
    }

    fn z_order(&mut self, hwnd: u32, after: u64, apply: bool) -> Reply {
        let plan = match self.z_plan(hwnd, after) {
            Some(p) => p,
            None if self.get(u64::from(hwnd)).is_none() => return err(ERROR_INVALID_WINDOW_HANDLE),
            None => return Reply::Bool { value: false },
        };
        let (parent, was_below) = (plan.parent, plan.was_below);
        let (new, below, raised, old) = (plan.new, plan.below, plan.raised, plan.old);
        // A process's only window in a session without other processes'
        // windows (session 0): the list alone cannot say that it went to
        // the bottom (ShowWindow.yaml `z_order`).
        let changed = new != old || below != was_below;
        if apply && changed {
            let new = if parent == self.desktop { self.with_ui(new) } else { new };
            self.get_mut(u64::from(parent)).expect("valid").children = new;
            if let Some(w) = self.get_mut(u64::from(hwnd)) {
                w.below_others = below;
            }
            for o in raised {
                if let Some(w) = self.get_mut(u64::from(o)) {
                    w.below_others = false;
                }
            }
        }
        Reply::Bool { value: changed }
    }

    /// Where `hwnd` would go for `after`: the new child list of its parent and what goes with it.
    fn z_plan(&self, hwnd: u32, after: u64) -> Option<ZPlan> {
        let w = self.get(u64::from(hwnd))?;
        let parent = w.parent;
        let thread = w.thread;
        let was_below = w.below_others;
        let p = self.get(u64::from(parent))?;
        let top_level = parent == self.desktop;
        let old: Vec<u32> = if top_level {
            p.children.iter().copied().filter(|&c| !self.is_ime_ui(c)).collect()
        } else {
            p.children.clone()
        };
        let mut raised: Vec<u32> = Vec::new();
        let (new, below) = match after {
            HWND_TOP | HWND_TOPMOST | HWND_NOTOPMOST => {
                // Owned windows stay above their owner (Learn: "An owned
                // window is always above its owner in the z-order");
                // HWND_TOPMOST / HWND_NOTOPMOST leave the style to the caller.
                if top_level {
                    let list = self.tops();
                    raised = list.iter().filter(|t| t.owner == hwnd).map(|t| t.hwnd).collect();
                    (crate::zorder::raise(&list, hwnd, self.barrier(thread)), false)
                } else {
                    let mut new: Vec<u32> = old.iter().copied().filter(|&c| c != hwnd).collect();
                    new.insert(0, hwnd);
                    (new, false)
                }
            }
            HWND_BOTTOM => {
                if top_level {
                    (crate::zorder::sink(&self.tops(), hwnd), true)
                } else {
                    let mut new: Vec<u32> = old.iter().copied().filter(|&c| c != hwnd).collect();
                    new.push(hwnd);
                    (new, false)
                }
            }
            a => {
                let mut new: Vec<u32> = old.iter().copied().filter(|&c| c != hwnd).collect();
                let i = new.iter().position(|&c| u64::from(c) == a & 0xFFFF_FFFF)?;
                let below = top_level && self.get(u64::from(new[i])).is_some_and(|k| k.below_others);
                new.insert(i + 1, hwnd);
                (new, below)
            }
        };
        Some(ZPlan { parent, was_below, old, new, below, raised })
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
            // The desktop, the parent of the message-only windows and the other processes' stand-ins
            // (the desktop's class) are no windows a search finds (FindWindowExW.yaml `parent`).
            if c == self.desktop || c == self.message {
                continue;
            }
            let Some(w) = self.get(u64::from(c)) else { continue };
            if self.get(u64::from(self.desktop)).is_some_and(|d| d.class == w.class) {
                continue;
            }
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

    /// Layered-window attributes (`SetLayeredWindowAttributes`, `GetLayeredWindowAttributes`
    /// and the mark `UpdateLayeredWindow` leaves; the sheets of those calls): see [`guic::Layered`].
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
            // UpdateLayeredWindow is refused once attributes are set.
            2 => {
                if l.attrs {
                    return err(ERROR_INVALID_PARAMETER);
                }
                Reply::Ok {}
            }
            // A successful UpdateLayeredWindow: the window shows its
            // content from now on, composed as the call said.
            _ => {
                if l.attrs {
                    return err(ERROR_INVALID_PARAMETER);
                }
                l.update = true;
                l.ulw_flags = flags & 7;
                l.ulw_src_alpha = flags & 0x100 != 0;
                l.ulw_key = key;
                l.ulw_alpha = alpha;
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
            // The process's own (`GetDpiAwarenessContextForProcess`, `GetProcessDpiAwareness`).
            let a = if process { self.proc_dpi.get(&ctx.pid).copied().unwrap_or(guic::AWARE_UNAWARE) } else { prev };
            return Reply::Value { value: u64::from(a) };
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

    #[allow(clippy::too_many_arguments)]
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
        self.post_system_message_with(hwnd, msg, 0, now, fx)
    }

    /// [`Wm::post_system_message`] with a `wParam`: the shell's minimize, maximize and restore
    /// are `WM_SYSCOMMAND` with `SC_MINIMIZE`, `SC_MAXIMIZE`, `SC_RESTORE` (Learn,
    /// "WM_SYSCOMMAND": what the window menu and the caption buttons send), which the window's
    /// procedure handles or `DefWindowProc` turns into the matching `ShowWindow`.
    pub fn post_system_message_with(&mut self, hwnd: u32, msg: u32, w: u64, now: u64, fx: &mut Effects) -> bool {
        let Some(window) = self.get(u64::from(hwnd)) else { return false };
        let to = window.thread;
        let ctx = Ctx { pid: to.0, tid: to.1, now };
        matches!(self.post(ctx, to, hwnd, msg, w, 0, fx), Reply::Ok {})
    }

    /// Apply a compositor-moved or resized client rectangle to a top-level
    /// window's Win32 geometry. `client` is in screen coordinates, as the
    /// desktop link's `Configure` record is. `surface` holds the menu bar (if
    /// any) above the client area. A window that draws its own
    /// caption (ADR 0183 addendum: `WS_CAPTION` and a client area its
    /// `WM_NCCALCSIZE` answer put at the top) keeps the borders that answer
    /// left around its client area; Poly's frame is around the others.
    ///
    /// A new **size** is not applied here: the window's thread is handed
    /// [`DESKTOP_SET_POS`] (as a message sent from no thread), which makes
    /// `user32` call `SetWindowPos` with the new window rectangle there, as
    /// Windows' sizing loop does in the window's thread (Learn,
    /// "WM_SIZING", "WM_WINDOWPOSCHANGED"): the app hears
    /// `WM_WINDOWPOSCHANGING`, `WM_NCCALCSIZE`, `WM_WINDOWPOSCHANGED` and
    /// `WM_SIZE` and draws the new size. A newer size for the same window
    /// replaces one its thread has not taken yet.
    pub fn configure_from_desktop(&mut self, hwnd: u32, surface: Rect, frame: FrameMetrics, fx: &mut Effects) -> bool {
        let Some(w) = self.get(u64::from(hwnd)) else { return false };
        let bar = w.menu_bar;
        // The surface holds the menu bar above the client area.
        let client = Rect::new(surface.left, (surface.top + bar).min(surface.bottom), surface.right, surface.bottom);
        let own_caption = w.style & crate::metrics::WS_CAPTION == crate::metrics::WS_CAPTION
            && !crate::surfaces::poly_frames(w.style, w.window_rel, w.client_rel);
        let (old_w, old_c) = (w.window_rel, w.client_rel);
        let (ox, oy) = self.parent_origin(hwnd);
        let outer = if own_caption {
            Rect::new(
                client.left - (old_c.left - old_w.left),
                client.top - (old_c.top - old_w.top),
                client.right + (old_w.right - old_c.right),
                client.bottom + (old_w.bottom - old_c.bottom),
            )
        } else {
            frame.window_from_surface(surface)
        };
        let window_rel = Rect::new(outer.left - ox, outer.top - oy, outer.right - ox, outer.bottom - oy);
        let client_rel = Rect::new(client.left - ox, client.top - oy, client.right - ox, client.bottom - oy);
        let resized = (client_rel.width(), client_rel.height()) != (old_c.width(), old_c.height());
        if resized && !w.maximized && !w.minimized && w.thread.0 != SYSTEM_DESKTOP_PID {
            let to = w.thread;
            self.desktop_set_pos(hwnd, to, window_rel, fx);
            return true;
        }
        let Some(window) = self.get_mut(u64::from(hwnd)) else { return false };
        window.window_rel = window_rel;
        window.client_rel = client_rel;
        if !window.maximized && !window.minimized {
            window.normal = window_rel;
        }
        true
    }

    /// Hand thread `to` a [`DESKTOP_SET_POS`] for window `hwnd` (`r`: the
    /// window rectangle relative to its parent), or update the one it has
    /// not taken yet.
    fn desktop_set_pos(&mut self, hwnd: u32, to: Key, r: Rect, fx: &mut Effects) {
        let (w, l) = pack_set_pos(r);
        let waiting = self.queues.get(&to).map(|q| q.sent.clone()).unwrap_or_default();
        for id in waiting {
            if let Some(s) = self.sends.get_mut(&id)
                && s.from == DESKTOP_SENDER
                && s.incoming.hwnd == hwnd
                && s.incoming.msg == DESKTOP_SET_POS
            {
                (s.incoming.w, s.incoming.l) = (w, l);
                return;
            }
        }
        let id = self.next_send;
        self.next_send += 1;
        let incoming = Incoming { id, hwnd, msg: DESKTOP_SET_POS, w, l, wndproc: 0 };
        self.sends.insert(id, Send { from: DESKTOP_SENDER, to, incoming, result: None, collected: false });
        let q = self.queue(to);
        q.sent.push_back(id);
        q.changed |= QS_SENDMESSAGE;
        fx.wake.push(to);
    }

    /// The desktop's keyboard layout (its settings' name, ADR 0209): keys
    /// become virtual keys, and `user32` characters, by it.
    pub fn set_layout(&mut self, layout: crate::kbd::Layout) {
        self.layout = layout;
    }

    /// The keyboard layout keys are read with.
    pub fn layout(&self) -> crate::kbd::Layout {
        self.layout
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
        if to == ctx.key() || to.0 == SYSTEM_DESKTOP_PID || input::foreign(to) {
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
        if self.sends.get(&id).is_some_and(|s| s.from == DESKTOP_SENDER) {
            // Nobody waits for the desktop's requests.
            self.sends.remove(&id);
            return;
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
        self.fg_right.remove(&key);
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
        self.sends.retain(|_, s| s.from != key && !(s.from == DESKTOP_SENDER && s.to == key));
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

    /// Run one request of thread `ctx`, in physical pixels (`dpi::handle` converts).
    fn handle_raw(&mut self, ctx: Ctx, req: Request, fx: &mut Effects) -> Reply {
        self.queue(ctx.key());
        self.now = self.now.max(ctx.now);
        match req {
            Request::Desktop {} => self.desktop_reply(ctx.key()),
            Request::RegisterEvent { .. } | Request::Queue {} => Reply::Ok {},
            Request::RegisterClass { spec } => self.register_class(ctx, spec),
            Request::UnregisterClass { class, .. } => self.unregister_class(ctx, class),
            Request::ClassInfo { class, .. } => match self.find_class(ctx.pid, &class) {
                Some(i) => Reply::Class { info: Self::class_info(self.classes[i].as_ref().expect("found")) },
                None => err(ERROR_CLASS_DOES_NOT_EXIST),
            },
            Request::ClassLong { hwnd, index } => self.class_long(hwnd, index),
            Request::SetClassLong { hwnd, index, value } => self.set_class_long(hwnd, index, value),
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
            Request::ZReport { hwnd, after } => self.z_report(hwnd, after),
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
                let mut minimizing = false;
                if let Some(m) = minimized {
                    if m && !w.minimized {
                        w.restore_max = w.maximized;
                        minimizing = true;
                    } else if !m {
                        w.restore_max = false;
                    }
                    w.minimized = m;
                    if m {
                        w.style |= WS_MINIMIZE;
                    } else {
                        w.style &= !WS_MINIMIZE;
                    }
                }
                if minimizing && was_visible && visible != Some(true) {
                    self.minimized_paint(hwnd);
                }
                let Some(w) = self.get_mut(u64::from(hwnd)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
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
            // Window stations and desktops are objects of the NT personality's (ADR 0192).
            Request::Winsta { .. } => Reply::Error { code: 87 },
            Request::Layered { hwnd, op, key, alpha, flags } => self.layered_op(hwnd, op, key, alpha, flags),
            Request::Ime { op } => Reply::Ime { res: self.ime.run(ctx.key(), op) },
            Request::ImeAssoc { hwnd, code } => match self.get_mut(u64::from(hwnd)) {
                Some(w) => Reply::Value { value: u64::from(core::mem::replace(&mut w.ime, code)) },
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::CursorShape { .. }
            | Request::ScreenPixel { .. }
            | Request::LayeredContent { .. }
            | Request::SurfaceAlpha { .. }
            | Request::MoveSize { .. } => Reply::Ok {},
            // The NT personality answers it (it owns the file handles); without it, no desktop.
            Request::ShellOpen { .. } => Reply::Error { code: 50 },
            Request::KeyboardLayout {} => Reply::Value { value: u64::from(self.layout.id()) },
            Request::LayeredChildren {} => {
                self.layered_children.insert(ctx.pid);
                Reply::Ok {}
            }
            Request::WindowRgn { hwnd, set, rects } => match self.get_mut(u64::from(hwnd)) {
                Some(w) => {
                    w.region = set.then(|| {
                        let rs: Vec<Rect> = rects
                            .chunks_exact(4)
                            .map(|c| Rect::new(c[0] as i32, c[1] as i32, c[2] as i32, c[3] as i32))
                            .collect();
                        crate::rgn::Rgn::from_rects(&rs)
                    });
                    Reply::Ok {}
                }
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::Dpi { set, process, awareness } => self.dpi_op(ctx, set, process, awareness),
            Request::SetScale { step } => self.set_scale(step, fx),
            Request::DpiFrame { hwnd } => self.dpi_frame(hwnd),
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
            Request::Caret { op, hwnd, a, b, tid } => self.caret_op(ctx, op, hwnd, a, b, tid),
            Request::MayTakeForeground {} => Reply::Bool { value: self.may_take_foreground(ctx.key()) },
            Request::ActivationHistory {} => {
                let k = self.input_key(ctx.key());
                Reply::Windows { hwnds: self.queues.get(&k).map(|q| q.activated.clone()).unwrap_or_default() }
            }
            Request::SetInput { active, focus, capture, which } => {
                if which & input::SET_CAPTURE_MOVE != 0 {
                    // The capture release's mouse move, after WM_CAPTURECHANGED.
                    self.queue_mouse(ctx, raw::WM_MOUSEMOVE, 0, fx);
                    self.mark_synthetic_move(ctx.key());
                    return Reply::Ok {};
                }
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
                self.handed_away(ctx.key(), hwnd);
                Reply::Ok {}
            }
            Request::NextActive { hwnd, cause } => self.next_active(ctx.key(), hwnd, cause),
            Request::SetMinPos { hwnd, x, y } => match self.get_mut(u64::from(hwnd)) {
                Some(w) => {
                    w.min_pos = (x, y);
                    Reply::Ok {}
                }
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::MenuBar { hwnd, height } => match self.get_mut(u64::from(hwnd)) {
                Some(w) => {
                    w.menu_bar = height.clamp(0, 256);
                    Reply::Ok {}
                }
                None => err(ERROR_INVALID_WINDOW_HANDLE),
            },
            Request::DefaultIme { tid } => {
                let tid = if tid == 0 { ctx.tid } else { tid };
                Reply::Handle { hwnd: self.default_ime(tid) }
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
            Request::ThreadHasWindows { tid } => {
                Reply::Bool { value: tid != 0 && self.slots.iter().flatten().any(|w| w.thread.1 == tid) }
            }
            Request::RegisterMessage { name } => {
                // Learn: the string is at most 255 characters; the number is in 0xC000..=0xFFFF.
                // Over that: ERROR_MORE_DATA; the empty string and "#1234" (an integer atom) give
                // 0 and no error (RegisterWindowMessageW.yaml `length`, `special`).
                let integer_atom = name.first() == Some(&(b'#' as u16))
                    && name.len() > 1
                    && name[1..].iter().all(|&c| (u16::from(b'0')..=u16::from(b'9')).contains(&c));
                if name.len() > 255 {
                    err(ERROR_MORE_DATA)
                } else if name.is_empty() || integer_atom {
                    Reply::Atom { atom: 0 }
                } else {
                    Reply::Atom { atom: self.atom_for(&name) }
                }
            }
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
            lay: guic::Layered::default(),
            ime: u32::MAX,
            region: None,
            below_others: false,
            restore_max: false,
            min_pos: (-1, -1),
            helper: false,
            menu_bar: 0,
            own: None,
            frame_pin: 0,
            suggest: None,
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

    /// The thread's IME windows after its top-level window is destroyed (GetWindow.yaml `ime_windows`):
    /// the walk is ime, ctf, holder; the default IME window has no owner.
    #[test]
    fn ime_windows_after_the_active_window_is_destroyed() {
        let mut wm = Wm::new();
        wm.set_display(crate::metrics::Display::INTERACTIVE);
        for n in ["HidC", "MainC", "IME", "MSCTFIME UI"] {
            reg(&mut wm, n);
        }
        let popup = WS_POPUP | WS_DISABLED;
        let hid = create(&mut wm, A, "HidC", popup, 0, 0);
        let ime = create_owned(&mut wm, "IME", popup, hid);
        let main = create(&mut wm, A, "MainC", WS_OVERLAPPEDWINDOW_TEST, 0, 0);
        wm.handle(A, Request::SetInput { active: main, focus: main, capture: 0, which: 3 }, &mut Effects::default());
        let ctf = create_owned(&mut wm, "MSCTFIME UI", popup, ime);
        let walk = |wm: &Wm| -> Vec<u32> {
            wm.get(u64::from(wm.desktop))
                .unwrap()
                .children
                .iter()
                .copied()
                .filter(|&h| [hid, ime, main, ctf].contains(&h))
                .collect()
        };
        assert_eq!(walk(&wm), vec![ctf, ime, main, hid], "active: ctf ime main hid");
        wm.handle(A, Request::SetInput { active: 0, focus: 0, capture: 0, which: 3 }, &mut Effects::default());
        wm.handle(A, Request::Free { hwnd: main }, &mut Effects::default());
        assert_eq!(wm.get(u64::from(ime)).unwrap().owner, 0, "the IME window has no owner");
        assert_eq!(walk(&wm), vec![ime, ctf, hid], "destroyed: ime ctf hid");
    }

    fn create_owned(wm: &mut Wm, n: &str, style: u32, owner: u32) -> u32 {
        let spec = CreateSpec {
            class: ClassRef::Name { name: name(n) },
            style,
            cx: 0,
            cy: 0,
            parent: u64::from(owner),
            ..CreateSpec::default()
        };
        match wm.handle(A, Request::CreateWindow { spec }, &mut Effects::default()) {
            Reply::Created { created } => {
                wm.handle(A, Request::FinishCreate { hwnd: created.hwnd }, &mut Effects::default());
                created.hwnd
            }
            r => panic!("{r:?}"),
        }
    }

    const WS_OVERLAPPEDWINDOW_TEST: u32 = 0x00CF_0000 | WS_VISIBLE;

    #[test]
    fn window_messages_share_one_number_per_name() {
        let mut wm = Wm::new();
        let reg = |wm: &mut Wm, ctx: Ctx, s: &str| {
            wm.handle(ctx, Request::RegisterMessage { name: s.encode_utf16().collect() }, &mut Effects::default())
        };
        let Reply::Atom { atom: a } = reg(&mut wm, A, "Chrome_MessageWindow") else { panic!() };
        assert!((0xC000..=0xFFFF).contains(&a));
        // Another process, another case: the same number.
        assert_eq!(reg(&mut wm, B, "CHROME_MESSAGEWINDOW"), Reply::Atom { atom: a });
        let Reply::Atom { atom: b } = reg(&mut wm, A, "TaskbarCreated") else { panic!() };
        assert_ne!(a, b);
        assert_eq!(reg(&mut wm, A, ""), Reply::Atom { atom: 0 });
        assert_eq!(reg(&mut wm, A, "#1234"), Reply::Atom { atom: 0 });
        assert_eq!(reg(&mut wm, A, &"x".repeat(256)), err(ERROR_MORE_DATA));
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
    fn the_surface_holds_the_menu_bar_above_the_client_area() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let hwnd = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        assert!(matches!(wm.handle(A, Request::MenuBar { hwnd, height: 20 }, &mut Effects::default()), Reply::Ok {}));
        assert_eq!(wm.info(hwnd).unwrap().menu_bar, 20);
        // The window has that size already (a new size is the thread's to apply).
        let (window_rel, client_rel) = (Some(Rect::new(0, 0, 386, 307)), Some(Rect::new(1, 45, 385, 306)));
        wm.handle(A, Request::SetRects { hwnd, window_rel, client_rel }, &mut Effects::default());
        // The compositor moves the surface; the client area is what is below the bar.
        let surface = Rect::new(200, 300, 584, 581);
        assert!(wm.configure_from_desktop(hwnd, surface, FrameMetrics::POLY, &mut Effects::default()));
        let info = wm.info(hwnd).unwrap();
        assert_eq!(info.client, Rect::new(200, 320, 584, 581));
        assert_eq!(info.window, Rect::new(199, 275, 585, 582));
        // A window that is gone has no bar to set.
        assert!(matches!(
            wm.handle(A, Request::MenuBar { hwnd: 0x7777, height: 20 }, &mut Effects::default()),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn desktop_moves_update_win32_geometry_and_close_is_posted_to_its_thread() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let hwnd = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        let (window_rel, client_rel) = (Some(Rect::new(0, 0, 386, 287)), Some(Rect::new(1, 25, 385, 286)));
        wm.handle(A, Request::SetRects { hwnd, window_rel, client_rel }, &mut Effects::default());
        let client = Rect::new(200, 300, 584, 561);
        assert!(wm.configure_from_desktop(hwnd, client, FrameMetrics::POLY, &mut Effects::default()));
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

    /// A window that draws its own caption keeps its own borders when the compositor moves it
    /// (ADR 0183 addendum): Chrome's 900x500 window with an 884x492 client area from its top.
    #[test]
    fn a_desktop_move_keeps_the_borders_of_a_window_that_draws_its_own_caption() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let hwnd = create(&mut wm, A, "C", 0x06CF_0000, 0, 0);
        let mut fx = Effects::default();
        let window_rel = Some(Rect::new(40, 40, 940, 540));
        let client_rel = Some(Rect::new(48, 40, 932, 532));
        wm.handle(A, Request::SetRects { hwnd, window_rel, client_rel }, &mut fx);
        assert!(wm.configure_from_desktop(hwnd, Rect::new(148, 140, 1032, 632), FrameMetrics::POLY, &mut fx));
        let info = wm.info(hwnd).unwrap();
        assert_eq!(info.client, Rect::new(148, 140, 1032, 632));
        assert_eq!(info.window, Rect::new(140, 140, 1040, 640));
        assert!(!crate::surfaces::poly_frames(info.style, info.window_rel, info.client_rel));
    }

    /// A resize by the window's border (ADR 0155 addendum "browsing"): the compositor's new size
    /// is the window's thread's to apply with `SetWindowPos`; win32k hands it over as a message
    /// sent from no thread, a newer size replaces one not taken yet, and nobody waits for it.
    #[test]
    fn a_desktop_resize_is_handed_to_the_windows_thread() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let hwnd = create(&mut wm, A, "C", 0x06CF_0000, 0, 0);
        let mut fx = Effects::default();
        let window_rel = Some(Rect::new(40, 40, 940, 540));
        let client_rel = Some(Rect::new(48, 40, 932, 532));
        wm.handle(A, Request::SetRects { hwnd, window_rel, client_rel }, &mut fx);
        let mut fx = Effects::default();
        assert!(wm.configure_from_desktop(hwnd, Rect::new(48, 40, 1032, 632), FrameMetrics::POLY, &mut fx));
        assert_eq!(fx.wake, [(A.pid, A.tid)]);
        // Not applied yet: the thread does it.
        assert_eq!(wm.info(hwnd).unwrap().client, Rect::new(48, 40, 932, 532));
        // A newer size before the thread looked replaces it.
        assert!(wm.configure_from_desktop(hwnd, Rect::new(48, 40, 1132, 652), FrameMetrics::POLY, &mut fx));
        let peek = Request::Peek { filter: 0, min: 0, max: 0, flags: PM_REMOVE };
        let Reply::Incoming { incoming } = wm.handle(A, peek.clone(), &mut fx) else { panic!("no set-pos") };
        assert_eq!((incoming.hwnd, incoming.msg, incoming.wndproc), (hwnd, DESKTOP_SET_POS, 0));
        // The window rectangle keeps the window's own borders around the new client area.
        assert_eq!(unpack_set_pos(incoming.w, incoming.l), (40, 40, 1100, 620));
        assert!(!matches!(wm.handle(A, peek, &mut fx), Reply::Incoming { .. }), "one hand-over");
        wm.handle(A, Request::SendDone { id: incoming.id, result: 0 }, &mut fx);
        assert!(wm.sends.is_empty(), "nobody waits for it");
        // A move alone is applied at once.
        let mut fx = Effects::default();
        assert!(wm.configure_from_desktop(hwnd, Rect::new(148, 140, 1032, 632), FrameMetrics::POLY, &mut fx));
        assert!(fx.wake.is_empty());
        assert_eq!(wm.info(hwnd).unwrap().client, Rect::new(148, 140, 1032, 632));
    }

    /// The desktop's wheel (`FromCompositor::Axis`) reaches the window under the pointer as
    /// `WM_MOUSEWHEEL`, the delta in the high word of `wParam`; turns not yet retrieved add up.
    #[test]
    fn the_desktops_wheel_reaches_the_window_under_the_pointer() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let hwnd = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        let mut fx = Effects::default();
        let rect = Some(Rect::new(100, 100, 500, 400));
        wm.handle(A, Request::SetRects { hwnd, window_rel: rect, client_rel: rect }, &mut fx);
        let shown = Request::SetState {
            hwnd,
            visible: Some(true),
            minimized: None,
            maximized: None,
            normal: None,
            size_move_pending: None,
        };
        wm.handle(A, shown, &mut fx);
        wm.link_pointer(200, 150, &mut fx);
        let mut fx = Effects::default();
        wm.link_wheel(-120, &mut fx);
        wm.link_wheel(-120, &mut fx);
        wm.link_wheel(0, &mut fx);
        assert_eq!(fx.wake.first(), Some(&(A.pid, A.tid)));
        let mut wheel = None;
        while let Reply::Message { msg } =
            wm.handle(A, Request::Peek { filter: 0, min: 0, max: 0, flags: PM_REMOVE }, &mut fx)
        {
            if msg.msg == crate::wm::raw::WM_MOUSEWHEEL {
                assert!(wheel.is_none(), "one message for both turns");
                wheel = Some(msg);
            }
        }
        let msg = wheel.expect("no WM_MOUSEWHEEL");
        assert_eq!((msg.hwnd, (msg.w >> 16) as u16 as i16, msg.x, msg.y), (hwnd, -240, 200, 150));
    }

    /// The desktop's keyboard layout (ADR 0209): `user32` asks for it, and keys are read with it.
    #[test]
    fn keys_are_read_with_the_desktops_layout() {
        let mut wm = Wm::new();
        reg(&mut wm, "C");
        let _ = create(&mut wm, A, "C", WS_POPUP, 0, 0);
        let mut fx = Effects::default();
        let ask = |wm: &mut Wm| match wm.handle(A, Request::KeyboardLayout {}, &mut Effects::default()) {
            Reply::Value { value } => crate::kbd::Layout::from_id(value),
            r => panic!("{r:?}"),
        };
        assert_eq!(ask(&mut wm), crate::kbd::Layout::German);
        wm.set_layout(crate::kbd::Layout::Us);
        assert_eq!(ask(&mut wm), crate::kbd::Layout::Us);
        // The key at Y's US place (scan 0x15) is Y, not the German Z.
        wm.link_key(0x15, true, &mut fx);
        assert_eq!(wm.async_keys[0x59] & 0x80, 0x80);
        assert_eq!(wm.async_keys[0x5A] & 0x80, 0);
    }

    #[test]
    fn set_pos_rectangles_round_trip() {
        let (w, l) = pack_set_pos(Rect::new(-8, -3, 1000, 600));
        assert_eq!(unpack_set_pos(w, l), (-8, -3, 1008, 603));
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
