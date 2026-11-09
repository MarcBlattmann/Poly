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
                *close_notify = core::mem::take(&mut self.dirty);
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
        if format >= FIRST_REGISTERED && format <= 0xFFFF && (format - FIRST_REGISTERED) as usize >= self.names.len() {
            return err(ERROR_INVALID_HANDLE);
        }
        let content = match data {
            None => Content::Delayed,
            Some(b) => Content::Bytes(b),
        };
        let rendering = self.rendering.contains(&format);
        let slot = Fmt { format, content, handle, pid };
        match self.find(format) {
            Some(i) => self.formats[i] = slot,
            None => self.formats.push(slot),
        }
        if rendering {
            // The owner's answer to WM_RENDERFORMAT is not a change.
            self.rendering.retain(|&f| f != format);
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
        }
    }

    /// A thread ended: its open state ends.
    pub fn thread_gone(&mut self, pid: u32, tid: u32) {
        self.holders.retain(|&h| h != (pid, tid));
        if self.holders.is_empty() && self.open.is_some() {
            self.open = None;
            self.rendering.clear();
        }
    }
}

// ----- text conversion

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
}
