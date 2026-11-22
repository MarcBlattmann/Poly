//! The clipboard of a session (ADR 0172): one object per desktop with an
//! owner window, an open state, the formats in the order they were first
//! set, a sequence number and the format listeners.
//!
//! The rules come from the sheets under `specs/windows/user32.dll/`
//! (`OpenClipboard.yaml` ... `GetPriorityClipboardFormat.yaml`) and the
//! Learn pages "About the Clipboard" and "Clipboard Formats". Data is held
//! as bytes: `SetClipboardData` copies the caller's global block (or, for
//! `CF_BITMAP`, the packed DIB `user32` made of the bitmap) so that another
//! process can read it and the clipboard outlives the one that set it. The
//! block the caller passed stays its own handle: the setting process gets
//! it back from `GetClipboardData`; a reader in another process, and every
//! synthesized format, gets a new block made from the bytes.
//!
//! Message work (`WM_DESTROYCLIPBOARD`, `WM_RENDERFORMAT`,
//! `WM_RENDERALLFORMATS`) needs window procedures and is `user32`'s
//! ([`crate::user`]); this module only says to whom.

use alloc::vec::Vec;

// Formats (`winuser.h`).
pub const CF_TEXT: u32 = 1;
pub const CF_BITMAP: u32 = 2;
pub const CF_OEMTEXT: u32 = 7;
pub const CF_DIB: u32 = 8;
pub const CF_UNICODETEXT: u32 = 13;
pub const CF_LOCALE: u32 = 16;
pub const CF_DIBV5: u32 = 17;

/// `WM_CLIPBOARDUPDATE` (`winuser.h`).
pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;

// Win32 errors (`winerror.h`).
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_NOT_FOUND: u32 = 1168;
pub const ERROR_CLIPBOARD_NOT_OPEN: u32 = 1418;

/// The first registered format (`RegisterClipboardFormatW` gives
/// `0xC000..=0xFFFF`).
pub const FIRST_REGISTERED: u32 = 0xC000;
const MAX_REGISTERED: usize = 0x4000;
/// The longest registered name (`RegisterClipboardFormatW.yaml`: 255).
pub const MAX_NAME: usize = 255;

/// The ANSI and OEM code pages of the session (the oracle's).
pub const ANSI_CP: u32 = 1252;
pub const OEM_CP: u32 = 850;
/// The user default locale (`en-US`; `GetClipboardData.yaml`: `CF_LOCALE`
/// equals `GetUserDefaultLCID`).
pub const LOCALE: u32 = 0x0409;

crate::wire_enum! {
    /// One operation on the clipboard.
    pub enum ClipOp {
        1 => Open { hwnd: u32 },
        2 => Close {},
        /// A `Val` with the previous owner (`user32` sends it
        /// `WM_DESTROYCLIPBOARD`).
        3 => Empty {},
        /// `data` is `None` for delayed rendering. A rendering owner's set
        /// (during `WM_RENDERFORMAT`) does not count as a change.
        4 => Set { format: u32, handle: u64, data: Option<Vec<u8>> },
        5 => Get { format: u32 },
        /// The owner has answered `WM_RENDERFORMAT(format)`.
        6 => RenderDone { format: u32 },
        /// The window is being destroyed: a `Bool`, whether it owns the
        /// clipboard (then `user32` sends `WM_RENDERALLFORMATS`).
        7 => Destroying { hwnd: u32 },
        /// `WM_RENDERALLFORMATS` is over: delayed formats still unrendered
        /// are dropped and the owner goes.
        8 => RenderedAll { hwnd: u32 },
        9 => Available { format: u32 },
        10 => Count {},
        11 => Enum { prev: u32 },
        12 => Owner {},
        13 => OpenWindow {},
        14 => Sequence {},
        15 => Priority { list: Vec<u32> },
        16 => Listener { hwnd: u32, add: bool },
        17 => Register { name: Vec<u16> },
        18 => Name { id: u32 },
    }
}

crate::wire_enum! {
    /// The answer to a [`ClipOp`].
    pub enum ClipRes {
        1 => Ok {},
        2 => Err { code: u32 },
        3 => Val { value: u64 },
        /// The data of a format: `handle` the one the caller set (valid
        /// when `same` is true: the reader is the setting process), else
        /// `bytes` of format `from` (the format itself, or the one a
        /// synthesized format is made from) to turn into a new block.
        4 => Data { handle: u64, same: bool, from: u32, bytes: Vec<u8> },
        /// The data is promised: send `WM_RENDERFORMAT(format)` to `owner`,
        /// then ask again.
        5 => Render { owner: u32, format: u32 },
        6 => Name { name: Vec<u16> },
        /// Nothing there, no error (a delayed format nobody renders).
        7 => Nothing {},
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Content {
    /// Promised (`SetClipboardData(fmt, NULL)`), not rendered.
    Delayed,
    Bytes(Vec<u8>),
    /// Made by the close from another format's content.
    Synth(u32),
}

#[derive(Clone, Debug)]
struct Fmt {
    format: u32,
    content: Content,
    /// The caller's block (0: none) and its process.
    handle: u64,
    pid: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Open {
    hwnd: u32,
}

/// Materialized clipboard payload ready to bridge to the desktop compositor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BridgeSnapshot {
    /// Clipboard sequence number when this snapshot was taken.
    pub seq: u32,
    /// UTF-8 text (LF line endings, no trailing NUL), if a materialized text
    /// format is present.
    pub text: Option<Vec<u8>>,
    /// Packed `CF_DIB` bytes (40-byte `BITMAPINFOHEADER` + optional
    /// masks/palette + pixels), if a materialized image format is present.
    pub dib: Option<Vec<u8>>,
}

impl BridgeSnapshot {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.dib.is_none()
    }
}

/// Outcome of applying a desktop-bridge clipboard update into [`Clipboard`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeApply {
    /// Whether the Windows clipboard contents changed (listeners should get
    /// `WM_CLIPBOARDUPDATE`).
    pub changed: bool,
    /// Previous Windows clipboard owner window (nonzero: send
    /// `WM_DESTROYCLIPBOARD`).
    pub old_owner: u32,
}

/// A deferred bridge update waiting for a Windows thread to close the clipboard.
#[derive(Clone, Debug)]
struct PendingBridge {
    text: Option<Vec<u8>>,
    dib: Option<Vec<u8>>,
}

/// The clipboard.
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    open: Option<Open>,
    /// Threads that opened it (several only for `OpenClipboard(NULL)`).
    holders: Vec<(u32, u32)>,
    owner: u32,
    seq: u32,
    formats: Vec<Fmt>,
    dirty: bool,
    /// Set when a Windows app changed or materialized clipboard contents,
    /// signalling the desktop bridge to export a snapshot. Never set by
    /// [`Clipboard::set_from_bridge`], preventing echo loops.
    bridge_dirty: bool,
    /// A bridge update that arrived while a Windows thread held the clipboard
    /// open; applied on `Close` if the Windows thread did not write newer data.
    pending_bridge: Option<PendingBridge>,
    /// Registered listeners, oldest first.
    listeners: Vec<u32>,
    names: Vec<Vec<u16>>,
    /// Formats whose owner is answering `WM_RENDERFORMAT`.
    rendering: Vec<u32>,
}

fn err(code: u32) -> ClipRes {
    ClipRes::Err { code }
}

fn upcase_ascii(c: u16) -> u16 {
    if (0x61..=0x7A).contains(&c) { c - 0x20 } else { c }
}

fn is_text(f: u32) -> bool {
    matches!(f, CF_TEXT | CF_OEMTEXT | CF_UNICODETEXT)
}

impl Clipboard {
    pub fn new() -> Clipboard {
        Clipboard::default()
    }

    fn holds(&self, pid: u32, tid: u32) -> bool {
        self.holders.contains(&(pid, tid))
    }

    fn find(&self, format: u32) -> Option<usize> {
        self.formats.iter().position(|f| f.format == format)
    }

    /// The owner window, or 0.
    pub fn owner(&self) -> u32 {
        self.owner
    }

    /// The window the clipboard is open for (0: closed or open for NULL).
    pub fn open_window(&self) -> u32 {
        self.open.map_or(0, |o| o.hwnd)
    }

    /// The listener windows in the order their messages are posted (the
    /// most recently added first, `AddClipboardFormatListener.yaml`).
    pub fn listeners_newest_first(&self) -> Vec<u32> {
        self.listeners.iter().rev().copied().collect()
    }

    /// Whether `hwnd` is registered as a listener.
    pub fn is_listener(&self, hwnd: u32) -> bool {
        self.listeners.contains(&hwnd)
    }

    /// The current clipboard sequence number.
    pub fn sequence(&self) -> u32 {
        self.seq
    }

    /// Whether a Windows app changed or materialized clipboard contents since
    /// the last [`Clipboard::take_bridge_dirty`].
    pub fn bridge_dirty(&self) -> bool {
        self.bridge_dirty
    }

    /// Consume the bridge-dirty flag, returning whether a Windows app update
    /// is ready to export to the desktop compositor.
    pub fn take_bridge_dirty(&mut self) -> bool {
        core::mem::take(&mut self.bridge_dirty)
    }

    /// Resolve a format to its materialized bytes (following at most one
    /// `Content::Synth` hop). Returns `None` for missing or unrendered
    /// `Content::Delayed` formats without ever blocking.
    fn materialized(&self, format: u32) -> Option<(u32, &[u8])> {
        let i = self.find(format)?;
        match &self.formats[i].content {
            Content::Bytes(b) => Some((format, b.as_slice())),
            Content::Delayed => None,
            Content::Synth(src) => {
                let j = self.find(*src)?;
                match &self.formats[j].content {
                    Content::Bytes(b) => Some((*src, b.as_slice())),
                    Content::Delayed | Content::Synth(_) => None,
                }
            }
        }
    }

    /// Export materialized text and/or image contents for the desktop bridge.
    /// Delayed formats that have not been rendered are skipped without
    /// blocking; once rendered, a subsequent export includes them.
    pub fn export_bridge(&self) -> BridgeSnapshot {
        let text = [CF_UNICODETEXT, CF_TEXT, CF_OEMTEXT]
            .into_iter()
            .find_map(|f| self.materialized(f))
            .and_then(|(fmt, bytes)| to_utf8(fmt, bytes));
        let dib = [CF_DIB, CF_BITMAP, CF_DIBV5]
            .into_iter()
            .find_map(|f| self.materialized(f))
            .filter(|(_, bytes)| !bytes.is_empty())
            .and_then(|(fmt, bytes)| {
                if fmt == CF_DIBV5 {
                    crate::dib::from_v5(bytes)
                } else {
                    crate::dib::info(bytes).map(|_| bytes.to_vec())
                }
            });
        BridgeSnapshot { seq: self.seq, text, dib }
    }

    /// Replace the Windows clipboard with contents bridged from the desktop
    /// (Wayland/Linux/native apps). Sets `CF_UNICODETEXT` and/or `CF_DIB`,
    /// synthesizes standard companion formats (`CF_TEXT`, `CF_OEMTEXT`,
    /// `CF_LOCALE`, `CF_BITMAP`, `CF_DIBV5`), and bumps the sequence number.
    /// Never sets `bridge_dirty`, preventing echo loops back to the compositor.
    ///
    /// If a Windows thread currently holds the clipboard open, the update is
    /// deferred until `CloseClipboard` (unless the Windows thread itself
    /// writes newer data).
    pub fn set_from_bridge(&mut self, text: Option<Vec<u8>>, dib: Option<Vec<u8>>) -> BridgeApply {
        if self.open.is_some() {
            self.pending_bridge = Some(PendingBridge { text, dib });
            return BridgeApply { changed: false, old_owner: 0 };
        }
        self.apply_bridge(text, dib)
    }

    fn apply_bridge(&mut self, text: Option<Vec<u8>>, dib: Option<Vec<u8>>) -> BridgeApply {
        let dib = dib.filter(|d| crate::dib::info(d).is_some());
        if text.is_none() && dib.is_none() && self.formats.is_empty() && self.owner == 0 {
            return BridgeApply { changed: false, old_owner: 0 };
        }
        let old_owner = core::mem::take(&mut self.owner);
        self.formats.clear();
        self.rendering.clear();
        self.seq = self.seq.wrapping_add(1);
        if let Some(utf8) = text {
            let utf16 = from_utf8(&utf8);
            self.formats.push(Fmt { format: CF_UNICODETEXT, content: Content::Bytes(utf16), handle: 0, pid: 0 });
            self.seq = self.seq.wrapping_add(1);
        }
        if let Some(dib_bytes) = dib {
            self.formats.push(Fmt { format: CF_DIB, content: Content::Bytes(dib_bytes), handle: 0, pid: 0 });
            self.seq = self.seq.wrapping_add(1);
        }
        self.synthesize();
        self.dirty = false;
        self.bridge_dirty = false;
        BridgeApply { changed: true, old_owner }
    }

    /// Run `op` for thread (`pid`, `tid`). `close_notify` is set when a
    /// close changed something (the caller posts `WM_CLIPBOARDUPDATE` to
    /// [`Clipboard::listeners_newest_first`]).
    pub fn run(&mut self, pid: u32, tid: u32, op: ClipOp, close_notify: &mut bool) -> ClipRes {
        match op {
            ClipOp::Open { hwnd } => self.do_open(pid, tid, hwnd),
            ClipOp::Close {} => {
                if !self.holds(pid, tid) {
                    return err(ERROR_CLIPBOARD_NOT_OPEN);
                }
                self.synthesize();
                self.open = None;
                self.holders.clear();
                self.rendering.clear();
                let changed = core::mem::take(&mut self.dirty);
                if changed {
                    // The Windows app wrote newer data: supersede any deferred
                    // bridge update and export to the compositor.
                    self.pending_bridge = None;
                    self.bridge_dirty = true;
                    *close_notify = true;
                } else if let Some(pending) = self.pending_bridge.take() {
                    let applied = self.apply_bridge(pending.text, pending.dib);
                    *close_notify = applied.changed;
                } else {
                    *close_notify = false;
                }
                ClipRes::Ok {}
            }
            ClipOp::Empty {} => {
                if !self.holds(pid, tid) {
                    return err(ERROR_CLIPBOARD_NOT_OPEN);
                }
                let old = self.owner;
                self.formats.clear();
                self.rendering.clear();
                self.owner = self.open_window();
                self.seq = self.seq.wrapping_add(1);
                self.dirty = true;
                ClipRes::Val { value: u64::from(old) }
            }
            ClipOp::Set { format, handle, data } => self.do_set(pid, tid, format, handle, data),
            ClipOp::Get { format } => self.do_get(pid, tid, format),
            ClipOp::RenderDone { format } => {
                self.rendering.retain(|&f| f != format);
                ClipRes::Ok {}
            }
            ClipOp::Destroying { hwnd } => ClipRes::Val { value: u64::from(hwnd != 0 && self.owner == hwnd) },
            ClipOp::RenderedAll { hwnd } => {
                if self.owner == hwnd {
                    self.owner = 0;
                    self.formats.retain(|f| f.content != Content::Delayed);
                    // Synthesized formats whose source went go too.
                    loop {
                        let n = self.formats.len();
                        let gone: Vec<u32> = self
                            .formats
                            .iter()
                            .filter_map(|f| match f.content {
                                Content::Synth(s) if self.find(s).is_none() => Some(f.format),
                                _ => None,
                            })
                            .collect();
                        self.formats.retain(|f| !gone.contains(&f.format));
                        if self.formats.len() == n {
                            break;
                        }
                    }
                }
                ClipRes::Ok {}
            }
            ClipOp::Available { format } => ClipRes::Val { value: u64::from(self.find(format).is_some()) },
            ClipOp::Count {} => ClipRes::Val { value: self.formats.len() as u64 },
            ClipOp::Enum { prev } => {
                if !self.holds(pid, tid) {
                    return err(ERROR_CLIPBOARD_NOT_OPEN);
                }
                let next = if prev == 0 {
                    self.formats.first().map(|f| f.format)
                } else {
                    self.find(prev).and_then(|i| self.formats.get(i + 1)).map(|f| f.format)
                };
                ClipRes::Val { value: u64::from(next.unwrap_or(0)) }
            }
            ClipOp::Owner {} => ClipRes::Val { value: u64::from(self.owner) },
            ClipOp::OpenWindow {} => ClipRes::Val { value: u64::from(self.open_window()) },
            ClipOp::Sequence {} => ClipRes::Val { value: u64::from(self.seq) },
            ClipOp::Priority { list } => {
                if self.formats.is_empty() {
                    return ClipRes::Val { value: 0 };
                }
                let hit = list.iter().find(|&&f| f != 0 && f != u32::MAX && self.find(f).is_some());
                ClipRes::Val { value: u64::from(hit.copied().unwrap_or(u32::MAX)) }
            }
            ClipOp::Listener { hwnd, add } => {
                let at = self.listeners.iter().position(|&l| l == hwnd);
                match (add, at) {
                    (true, Some(_)) | (false, None) => err(ERROR_INVALID_PARAMETER),
                    (true, None) => {
                        self.listeners.push(hwnd);
                        ClipRes::Ok {}
                    }
                    (false, Some(i)) => {
                        self.listeners.remove(i);
                        ClipRes::Ok {}
                    }
                }
            }
            ClipOp::Register { name } => self.register(&name),
            ClipOp::Name { id } => {
                if id == u32::MAX {
                    return err(ERROR_INVALID_HANDLE);
                }
                if !(FIRST_REGISTERED..=0xFFFF).contains(&id) {
                    return err(ERROR_INVALID_PARAMETER);
                }
                match self.names.get((id - FIRST_REGISTERED) as usize) {
                    Some(n) => ClipRes::Name { name: n.clone() },
                    None => err(ERROR_INVALID_HANDLE),
                }
            }
        }
    }

    fn do_open(&mut self, pid: u32, tid: u32, hwnd: u32) -> ClipRes {
        match self.open {
            None => {
                self.open = Some(Open { hwnd });
                self.holders = alloc::vec![(pid, tid)];
                ClipRes::Ok {}
            }
            Some(o) => {
                let mine = self.holds(pid, tid);
                if mine && o.hwnd == hwnd {
                    ClipRes::Ok {}
                } else if !mine && o.hwnd == 0 && hwnd == 0 {
                    self.holders.push((pid, tid));
                    ClipRes::Ok {}
                } else {
                    err(ERROR_ACCESS_DENIED)
                }
            }
        }
    }

    fn do_set(&mut self, pid: u32, tid: u32, format: u32, handle: u64, data: Option<Vec<u8>>) -> ClipRes {
        let _ = (pid, tid);
        if format == 0 || self.open.is_none() {
            return err(ERROR_CLIPBOARD_NOT_OPEN);
        }
        if (FIRST_REGISTERED..=0xFFFF).contains(&format) && (format - FIRST_REGISTERED) as usize >= self.names.len() {
            return err(ERROR_INVALID_HANDLE);
        }
        let content = match data {
            None => Content::Delayed,
            Some(b) => Content::Bytes(b),
        };
        let materialized = matches!(&content, Content::Bytes(_));
        let rendering = self.rendering.contains(&format);
        let slot = Fmt { format, content, handle, pid };
        match self.find(format) {
            Some(i) => self.formats[i] = slot,
            None => self.formats.push(slot),
        }
        if rendering {
            // The owner's answer to WM_RENDERFORMAT is not a change for Win32
            // listeners/sequence, but a newly materialized format can now be
            // bridged to the compositor without blocking.
            self.rendering.retain(|&f| f != format);
            if materialized {
                self.bridge_dirty = true;
            }
        } else {
            self.seq = self.seq.wrapping_add(1);
            self.dirty = true;
        }
        // A synthesized format is replaced by an explicit set.
        ClipRes::Ok {}
    }

    fn do_get(&mut self, pid: u32, tid: u32, format: u32) -> ClipRes {
        if !self.holds(pid, tid) {
            return err(ERROR_CLIPBOARD_NOT_OPEN);
        }
        let Some(i) = self.find(format) else { return err(ERROR_NOT_FOUND) };
        let f = self.formats[i].clone();
        match f.content {
            Content::Bytes(b) => {
                if f.handle != 0 && f.pid == pid {
                    ClipRes::Data { handle: f.handle, same: true, from: format, bytes: Vec::new() }
                } else {
                    ClipRes::Data { handle: 0, same: false, from: format, bytes: b }
                }
            }
            Content::Delayed => self.render_or_nothing(format),
            Content::Synth(from) => match self.find(from).map(|j| self.formats[j].content.clone()) {
                Some(Content::Bytes(b)) => ClipRes::Data { handle: 0, same: false, from, bytes: b },
                Some(Content::Delayed) => self.render_or_nothing(from),
                Some(Content::Synth(_)) | None => err(ERROR_NOT_FOUND),
            },
        }
    }

    fn render_or_nothing(&mut self, format: u32) -> ClipRes {
        if self.owner == 0 || self.rendering.contains(&format) {
            self.rendering.retain(|&f| f != format);
            return ClipRes::Nothing {};
        }
        self.rendering.push(format);
        ClipRes::Render { owner: self.owner, format }
    }

    fn register(&mut self, name: &[u16]) -> ClipRes {
        if name.is_empty() {
            return ClipRes::Val { value: 0 };
        }
        if name.len() > MAX_NAME {
            return err(ERROR_MORE_DATA);
        }
        let key: Vec<u16> = name.iter().map(|&c| upcase_ascii(c)).collect();
        for (i, n) in self.names.iter().enumerate() {
            if n.len() == key.len() && n.iter().zip(&key).all(|(&a, &b)| upcase_ascii(a) == b) {
                return ClipRes::Val { value: u64::from(FIRST_REGISTERED) + i as u64 };
            }
        }
        if self.names.len() >= MAX_REGISTERED {
            return err(ERROR_ACCESS_DENIED);
        }
        self.names.push(name.to_vec());
        ClipRes::Val { value: u64::from(FIRST_REGISTERED) + (self.names.len() - 1) as u64 }
    }

    /// CloseClipboard's synthesis (`CloseClipboard.yaml`): `CF_LOCALE` and
    /// the missing text formats after text, the missing image formats after
    /// a bitmap or DIB; one sequence number per added format.
    fn synthesize(&mut self) {
        let add = |this: &mut Clipboard, format: u32, from: u32| {
            this.formats.push(Fmt { format, content: Content::Synth(from), handle: 0, pid: 0 });
            this.seq = this.seq.wrapping_add(1);
            this.dirty = true;
        };
        let have = |this: &Clipboard, f: u32| this.find(f).is_some();
        // Formats made from are the ones set, not the ones made here.
        let orig: Vec<u32> = self.formats.iter().map(|f| f.format).collect();
        let was = |f: u32| orig.contains(&f);
        if [CF_TEXT, CF_OEMTEXT, CF_UNICODETEXT].iter().any(|&f| have(self, f)) {
            if !have(self, CF_LOCALE) {
                // CF_LOCALE has its own content: the user locale.
                self.formats.push(Fmt {
                    format: CF_LOCALE,
                    content: Content::Bytes(LOCALE.to_le_bytes().to_vec()),
                    handle: 0,
                    pid: 0,
                });
                self.seq = self.seq.wrapping_add(1);
                self.dirty = true;
            }
            for f in [CF_TEXT, CF_OEMTEXT, CF_UNICODETEXT] {
                if have(self, f) {
                    continue;
                }
                let from = match f {
                    CF_TEXT => [CF_UNICODETEXT, CF_OEMTEXT],
                    CF_OEMTEXT => [CF_UNICODETEXT, CF_TEXT],
                    _ => [CF_TEXT, CF_OEMTEXT],
                }
                .into_iter()
                .find(|&s| was(s) && is_text(s))
                .unwrap_or(CF_UNICODETEXT);
                add(self, f, from);
            }
        }
        let image = [CF_BITMAP, CF_DIB, CF_DIBV5].into_iter().find(|&f| was(f));
        if let Some(src) = image {
            for f in [CF_BITMAP, CF_DIB, CF_DIBV5] {
                if !have(self, f) {
                    add(self, f, src);
                }
            }
        }
    }

    /// A window is gone (its `WM_NCDESTROY` ran): it no longer holds the
    /// clipboard open, owns it or listens (`OpenClipboard.yaml`).
    pub fn window_gone(&mut self, hwnd: u32) {
        if self.open.is_some_and(|o| o.hwnd == hwnd && hwnd != 0) {
            self.open = None;
            self.holders.clear();
            self.rendering.clear();
            if let Some(pending) = self.pending_bridge.take() {
                self.apply_bridge(pending.text, pending.dib);
            }
        }
        if self.owner == hwnd {
            self.owner = 0;
        }
        self.listeners.retain(|&l| l != hwnd);
    }

    /// A process ended: its threads' open states end.
    pub fn process_gone(&mut self, pid: u32) {
        self.holders.retain(|h| h.0 != pid);
        if self.holders.is_empty() && self.open.is_some() {
            self.open = None;
            self.rendering.clear();
            if let Some(pending) = self.pending_bridge.take() {
                self.apply_bridge(pending.text, pending.dib);
            }
        }
    }

    /// A thread ended: its open state ends.
    pub fn thread_gone(&mut self, pid: u32, tid: u32) {
        self.holders.retain(|&h| h != (pid, tid));
        if self.holders.is_empty() && self.open.is_some() {
            self.open = None;
            self.rendering.clear();
            if let Some(pending) = self.pending_bridge.take() {
                self.apply_bridge(pending.text, pending.dib);
            }
        }
    }
}

// ----- text conversion

/// Convert a Windows clipboard text format (`CF_UNICODETEXT`, `CF_TEXT`, or
/// `CF_OEMTEXT`) into UTF-8 bytes (no trailing NUL, `\r\n` normalized to
/// `\n`). Returns `None` for non-text formats.
pub fn to_utf8(format: u32, bytes: &[u8]) -> Option<Vec<u8>> {
    let units: Vec<u16> = match format {
        CF_UNICODETEXT => {
            bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect()
        }
        CF_TEXT | CF_OEMTEXT => {
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            let slice = &bytes[..end];
            let cp = poly_nls::codepage::CodePage::get(if format == CF_OEMTEXT { OEM_CP } else { ANSI_CP });
            match cp.and_then(|c| c.decode(slice, false).ok()) {
                Some(u) => u,
                None => slice.iter().map(|&x| u16::from(x)).collect(),
            }
        }
        _ => return None,
    };
    let mut s = alloc::string::String::with_capacity(units.len());
    for ch in core::char::decode_utf16(units) {
        s.push(ch.unwrap_or('\u{FFFD}'));
    }
    // Normalize CRLF -> LF for Linux/Wayland apps.
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Convert UTF-8 bytes (from Wayland/Linux) into NUL-terminated UTF-16LE
/// bytes for `CF_UNICODETEXT`, normalizing bare `\n` (not preceded by `\r`)
/// to `\r\n`.
pub fn from_utf8(utf8: &[u8]) -> Vec<u8> {
    let text = alloc::string::String::from_utf8_lossy(utf8);
    let mut out = Vec::with_capacity((text.len() + 1) * 2);
    let mut prev_cr = false;
    let mut buf = [0u16; 2];
    for ch in text.chars() {
        if ch == '\n' && !prev_cr {
            out.extend_from_slice(&(b'\r' as u16).to_le_bytes());
        }
        for u in ch.encode_utf16(&mut buf) {
            out.extend_from_slice(&u.to_le_bytes());
        }
        prev_cr = ch == '\r';
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Convert text between the clipboard's text formats. `bytes` is the
/// content of format `from`; the result is the content of format `to`.
/// A block without a terminating NUL has its last unit replaced by NUL
/// (`GetClipboardData.yaml` `text_termination`); the converted size
/// includes the terminating NUL. Conversion follows the code pages (ANSI
/// 1252, OEM 850); line feeds are not changed.
pub fn convert_text(from: u32, to: u32, bytes: &[u8]) -> Vec<u8> {
    let units: Vec<u16> = if from == CF_UNICODETEXT {
        let mut u: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        terminate(&mut u);
        u
    } else {
        let mut b = bytes.to_vec();
        terminate(&mut b);
        let cp = poly_nls::codepage::CodePage::get(if from == CF_OEMTEXT { OEM_CP } else { ANSI_CP });
        match cp.and_then(|c| c.decode(&b, false).ok()) {
            Some(u) => u,
            None => b.iter().map(|&x| u16::from(x)).collect(),
        }
    };
    if to == CF_UNICODETEXT {
        return units.iter().flat_map(|u| u.to_le_bytes()).collect();
    }
    let cp = poly_nls::codepage::CodePage::get(if to == CF_OEMTEXT { OEM_CP } else { ANSI_CP });
    match cp {
        Some(c) => c.encode(&units, b'?').0,
        None => units.iter().map(|&u| if u < 0x100 { u as u8 } else { b'?' }).collect(),
    }
}

fn terminate<T: Copy + Default + PartialEq>(v: &mut Vec<T>) {
    if v.last().copied() != Some(T::default()) {
        match v.last_mut() {
            Some(l) => *l = T::default(),
            None => v.push(T::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn run(c: &mut Clipboard, op: ClipOp) -> ClipRes {
        let mut n = false;
        c.run(1, 1, op, &mut n)
    }

    fn set(c: &mut Clipboard, f: u32, d: &[u8]) {
        assert_eq!(run(c, ClipOp::Set { format: f, handle: 0x1000, data: Some(d.to_vec()) }), ClipRes::Ok {});
    }

    fn formats(c: &mut Clipboard) -> Vec<u32> {
        let mut out = vec![];
        let mut p = 0;
        loop {
            let ClipRes::Val { value } = run(c, ClipOp::Enum { prev: p }) else { panic!() };
            if value == 0 {
                return out;
            }
            out.push(value as u32);
            p = value as u32;
        }
    }

    fn seq(c: &mut Clipboard) -> u64 {
        let ClipRes::Val { value } = run(c, ClipOp::Sequence {}) else { panic!() };
        value
    }

    #[test]
    fn unicode_text_gets_locale_text_oem_at_close() {
        let mut c = Clipboard::new();
        run(&mut c, ClipOp::Open { hwnd: 0x10 });
        run(&mut c, ClipOp::Empty {});
        let s0 = seq(&mut c);
        set(&mut c, CF_UNICODETEXT, &[72, 0, 0, 0]);
        assert_eq!(formats(&mut c), vec![CF_UNICODETEXT]);
        assert_eq!(run(&mut c, ClipOp::Get { format: CF_TEXT }), ClipRes::Err { code: ERROR_NOT_FOUND });
        let mut n = false;
        assert_eq!(c.run(1, 1, ClipOp::Close {}, &mut n), ClipRes::Ok {});
        assert!(n);
        assert_eq!(seq(&mut c) - s0, 4, "set +1, close +3");
        run(&mut c, ClipOp::Open { hwnd: 0 });
        assert_eq!(formats(&mut c), vec![CF_UNICODETEXT, CF_LOCALE, CF_TEXT, CF_OEMTEXT]);
    }

    #[test]
    fn synthesis_orders() {
        for (set_f, want) in [
            (vec![CF_TEXT], vec![CF_TEXT, CF_LOCALE, CF_OEMTEXT, CF_UNICODETEXT]),
            (vec![CF_OEMTEXT], vec![CF_OEMTEXT, CF_LOCALE, CF_TEXT, CF_UNICODETEXT]),
            (vec![CF_TEXT, CF_UNICODETEXT], vec![CF_TEXT, CF_UNICODETEXT, CF_LOCALE, CF_OEMTEXT]),
            (vec![CF_BITMAP], vec![CF_BITMAP, CF_DIB, CF_DIBV5]),
            (vec![CF_DIB], vec![CF_DIB, CF_BITMAP, CF_DIBV5]),
            (vec![0x201], vec![0x201]),
            (
                vec![CF_UNICODETEXT, CF_BITMAP],
                vec![CF_UNICODETEXT, CF_BITMAP, CF_LOCALE, CF_TEXT, CF_OEMTEXT, CF_DIB, CF_DIBV5],
            ),
        ] {
            let mut c = Clipboard::new();
            run(&mut c, ClipOp::Open { hwnd: 0 });
            for f in set_f {
                set(&mut c, f, &[0, 0]);
            }
            run(&mut c, ClipOp::Close {});
            run(&mut c, ClipOp::Open { hwnd: 0 });
            assert_eq!(formats(&mut c), want);
        }
    }

    #[test]
    fn open_rules() {
        let mut c = Clipboard::new();
        let mut n = false;
        assert_eq!(c.run(1, 1, ClipOp::Open { hwnd: 5 }, &mut n), ClipRes::Ok {});
        assert_eq!(c.run(1, 1, ClipOp::Open { hwnd: 5 }, &mut n), ClipRes::Ok {});
        assert_eq!(c.run(1, 1, ClipOp::Open { hwnd: 6 }, &mut n), ClipRes::Err { code: ERROR_ACCESS_DENIED });
        assert_eq!(c.run(1, 2, ClipOp::Open { hwnd: 0 }, &mut n), ClipRes::Err { code: ERROR_ACCESS_DENIED });
        assert_eq!(c.run(1, 2, ClipOp::Empty {}, &mut n), ClipRes::Err { code: ERROR_CLIPBOARD_NOT_OPEN });
        assert_eq!(c.run(1, 1, ClipOp::Close {}, &mut n), ClipRes::Ok {});
        assert_eq!(c.run(1, 1, ClipOp::Close {}, &mut n), ClipRes::Err { code: ERROR_CLIPBOARD_NOT_OPEN });
        // Open for NULL is shared.
        assert_eq!(c.run(1, 1, ClipOp::Open { hwnd: 0 }, &mut n), ClipRes::Ok {});
        assert_eq!(c.run(1, 2, ClipOp::Open { hwnd: 0 }, &mut n), ClipRes::Ok {});
        assert_eq!(c.run(1, 2, ClipOp::Close {}, &mut n), ClipRes::Ok {});
        assert_eq!(c.run(1, 1, ClipOp::Close {}, &mut n), ClipRes::Err { code: ERROR_CLIPBOARD_NOT_OPEN });
    }

    #[test]
    fn delayed_render_cycle() {
        let mut c = Clipboard::new();
        run(&mut c, ClipOp::Open { hwnd: 0x10 });
        run(&mut c, ClipOp::Empty {});
        assert_eq!(run(&mut c, ClipOp::Set { format: CF_UNICODETEXT, handle: 0, data: None }), ClipRes::Ok {});
        let s = seq(&mut c);
        assert_eq!(
            run(&mut c, ClipOp::Get { format: CF_UNICODETEXT }),
            ClipRes::Render { owner: 0x10, format: CF_UNICODETEXT }
        );
        // The owner answers inside the message.
        assert_eq!(
            run(&mut c, ClipOp::Set { format: CF_UNICODETEXT, handle: 0x2000, data: Some(vec![65, 0, 0, 0]) }),
            ClipRes::Ok {}
        );
        assert_eq!(seq(&mut c), s, "a render is no change");
        run(&mut c, ClipOp::RenderDone { format: CF_UNICODETEXT });
        assert!(matches!(
            run(&mut c, ClipOp::Get { format: CF_UNICODETEXT }),
            ClipRes::Data { same: true, handle: 0x2000, .. }
        ));
    }

    #[test]
    fn listeners_and_names() {
        let mut c = Clipboard::new();
        assert_eq!(run(&mut c, ClipOp::Listener { hwnd: 3, add: true }), ClipRes::Ok {});
        assert_eq!(run(&mut c, ClipOp::Listener { hwnd: 3, add: true }), ClipRes::Err { code: 87 });
        assert_eq!(run(&mut c, ClipOp::Listener { hwnd: 4, add: true }), ClipRes::Ok {});
        assert_eq!(c.listeners_newest_first(), vec![4, 3]);
        assert_eq!(run(&mut c, ClipOp::Listener { hwnd: 9, add: false }), ClipRes::Err { code: 87 });
        let a = run(&mut c, ClipOp::Register { name: "Poly A".encode_utf16().collect() });
        let b = run(&mut c, ClipOp::Register { name: "POLY a".encode_utf16().collect() });
        assert_eq!(a, b);
        assert_eq!(run(&mut c, ClipOp::Name { id: 0xC000 }), ClipRes::Name { name: "Poly A".encode_utf16().collect() });
        assert_eq!(run(&mut c, ClipOp::Name { id: 0xFFFF }), ClipRes::Err { code: ERROR_INVALID_HANDLE });
        assert_eq!(run(&mut c, ClipOp::Name { id: 1 }), ClipRes::Err { code: ERROR_INVALID_PARAMETER });
        assert_eq!(run(&mut c, ClipOp::Name { id: u32::MAX }), ClipRes::Err { code: ERROR_INVALID_HANDLE });
    }

    #[test]
    fn text_conversion() {
        // é in ANSI 1252 is 0xE9, in OEM 850 0x82; the euro is 0x80 / "?".
        let uni: Vec<u8> = [0x48u16, 0xE9, 0x20AC, 0].iter().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(convert_text(CF_UNICODETEXT, CF_TEXT, &uni), vec![0x48, 0xE9, 0x80, 0]);
        assert_eq!(convert_text(CF_UNICODETEXT, CF_OEMTEXT, &uni), vec![0x48, 0x82, b'?', 0]);
        assert_eq!(convert_text(CF_OEMTEXT, CF_TEXT, &[0x82, 0]), vec![0xE9, 0]);
        // No terminator: the last unit becomes NUL.
        let ab: Vec<u8> = [0x41u16, 0x42].iter().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(convert_text(CF_UNICODETEXT, CF_UNICODETEXT, &ab), vec![0x41, 0, 0, 0]);
        assert_eq!(convert_text(CF_TEXT, CF_UNICODETEXT, b"AB"), vec![0x41, 0, 0, 0]);
    }

    #[test]
    fn utf8_text_bridge_handles_ascii_unicode_surrogates_and_newlines() {
        // ASCII + Unicode BMP + supplementary plane (surrogate pair: U+1F30D 🌍).
        let src = "Hello 🌍 café\nLine 2\n";
        let utf16 = from_utf8(src.as_bytes());
        // Must end with UTF-16LE NUL and have CRLF line endings.
        assert!(utf16.ends_with(&[0, 0]));
        let units: Vec<u16> = utf16.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert!(units.contains(&0xD83C) && units.contains(&0xDF0D), "surrogate pair encoded");
        assert_eq!(to_utf8(CF_UNICODETEXT, &utf16).unwrap(), src.as_bytes());

        // Already-CRLF input must not double into \r\r\n.
        let crlf = b"one\r\ntwo\nthree\r";
        let w = from_utf8(crlf);
        let back = to_utf8(CF_UNICODETEXT, &w).unwrap();
        assert_eq!(back, b"one\ntwo\nthree\r");
        // Verify exact UTF-16 sequence has single \r\n for both line breaks.
        let units: Vec<u16> = w.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let expected: Vec<u16> = "one\r\ntwo\r\nthree\r\0".encode_utf16().collect();
        assert_eq!(units, expected);

        // Empty string preserves empty string (NUL-terminated [0, 0]).
        let empty = from_utf8(b"");
        assert_eq!(empty, [0, 0]);
        assert_eq!(to_utf8(CF_UNICODETEXT, &empty).unwrap(), b"");

        // ANSI (CP1252) and OEM (CP850) conversion to UTF-8.
        assert_eq!(to_utf8(CF_TEXT, &[0x48, 0xE9, 0x80, b'\r', b'\n', 0]).unwrap(), "Hé€\n".as_bytes());
        assert_eq!(to_utf8(CF_OEMTEXT, &[0x48, 0x82, 0]).unwrap(), "Hé".as_bytes());
        assert!(to_utf8(CF_DIB, &[1, 2, 3]).is_none());
    }

    #[test]
    fn bridge_export_import_delayed_and_loop_prevention() {
        let mut c = Clipboard::new();
        // Windows app sets Unicode text + closes: bridge_dirty becomes true.
        run(&mut c, ClipOp::Open { hwnd: 0x10 });
        run(&mut c, ClipOp::Empty {});
        set(&mut c, CF_UNICODETEXT, &from_utf8(b"win\ntext"));
        run(&mut c, ClipOp::Close {});
        assert!(c.take_bridge_dirty());
        assert!(!c.bridge_dirty());
        let snap = c.export_bridge();
        assert_eq!(snap.text.as_deref(), Some(&b"win\ntext"[..]));
        assert!(snap.dib.is_none());

        // Bridge import from Wayland sets CF_UNICODETEXT + synthesizes CF_TEXT/CF_OEMTEXT
        // and CF_DIB + synthesizes CF_BITMAP/CF_DIBV5, bumps seq, reports old owner,
        // and does NOT set bridge_dirty (no echo loop!).
        let s0 = c.sequence();
        let dib = crate::dib::make32(2, 1, &[1, 2, 3, 255, 4, 5, 6, 255]);
        let applied = c.set_from_bridge(Some(b"from wayland\nline2".to_vec()), Some(dib.clone()));
        assert_eq!(applied, BridgeApply { changed: true, old_owner: 0x10 });
        assert!(c.sequence() > s0);
        assert!(!c.bridge_dirty(), "bridge import must not echo back");
        assert_eq!(c.owner(), 0);
        run(&mut c, ClipOp::Open { hwnd: 0 });
        assert_eq!(formats(&mut c), vec![CF_UNICODETEXT, CF_DIB, CF_LOCALE, CF_TEXT, CF_OEMTEXT, CF_BITMAP, CF_DIBV5]);
        let ClipRes::Data { bytes, .. } = run(&mut c, ClipOp::Get { format: CF_UNICODETEXT }) else { panic!() };
        assert_eq!(to_utf8(CF_UNICODETEXT, &bytes).unwrap(), b"from wayland\nline2");
        let ClipRes::Data { from, bytes, .. } = run(&mut c, ClipOp::Get { format: CF_TEXT }) else { panic!() };
        assert_eq!(from, CF_UNICODETEXT);
        assert_eq!(convert_text(from, CF_TEXT, &bytes), b"from wayland\r\nline2\0");
        let ClipRes::Data { bytes: got_dib, .. } = run(&mut c, ClipOp::Get { format: CF_DIB }) else { panic!() };
        assert_eq!(got_dib, dib);
        run(&mut c, ClipOp::Close {});
        assert!(!c.bridge_dirty(), "reading and closing does not mark bridge dirty");

        // Clearing from bridge empties clipboard without echo; second clear is a no-op.
        let cleared = c.set_from_bridge(None, None);
        assert!(cleared.changed);
        assert!(!c.bridge_dirty());
        assert!(c.export_bridge().is_empty());
        assert!(!c.set_from_bridge(None, None).changed);

        // Delayed rendering: never blocks export; skipped until materialized, then exported.
        run(&mut c, ClipOp::Open { hwnd: 0x20 });
        run(&mut c, ClipOp::Empty {});
        assert_eq!(run(&mut c, ClipOp::Set { format: CF_UNICODETEXT, handle: 0, data: None }), ClipRes::Ok {});
        run(&mut c, ClipOp::Close {});
        assert!(c.take_bridge_dirty());
        assert!(c.export_bridge().is_empty(), "unrendered delayed format is skipped cleanly");
        // Another thread triggers render, owner renders it.
        run(&mut c, ClipOp::Open { hwnd: 0 });
        assert_eq!(
            run(&mut c, ClipOp::Get { format: CF_UNICODETEXT }),
            ClipRes::Render { owner: 0x20, format: CF_UNICODETEXT }
        );
        set(&mut c, CF_UNICODETEXT, &from_utf8(b"rendered"));
        assert!(c.take_bridge_dirty(), "materializing a delayed format notifies bridge");
        assert_eq!(c.export_bridge().text.as_deref(), Some(&b"rendered"[..]));
        run(&mut c, ClipOp::RenderDone { format: CF_UNICODETEXT });
        run(&mut c, ClipOp::Close {});

        // Deferred bridge update while open for reading applies cleanly on Close without echo.
        run(&mut c, ClipOp::Open { hwnd: 0 });
        let deferred = c.set_from_bridge(Some(b"deferred".to_vec()), None);
        assert!(!deferred.changed);
        let mut notify = false;
        assert_eq!(c.run(1, 1, ClipOp::Close {}, &mut notify), ClipRes::Ok {});
        assert!(notify, "listeners notified once deferred bridge update applies");
        assert!(!c.bridge_dirty(), "deferred bridge update must not echo");
        assert_eq!(c.export_bridge().text.as_deref(), Some(&b"deferred"[..]));
    }
}
