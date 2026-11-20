//! DPI awareness above 96 DPI (ADR 0231): every request is converted between the calling
//! thread's units and win32k's physical pixels here, and a change of the monitor's scale
//! (`DisplayConfigSetDeviceInfo`) hands each window's thread what it must do.
//!
//! A thread's units follow its awareness context (`SetProcessDpiAwarenessContext.yaml`
//! `at_200_modes`, `SetThreadDpiAwarenessContext.yaml`): an unaware or GDI-scaled thread works
//! at 96 DPI, a system aware one at the system DPI the session started with, a per-monitor aware
//! one (v1, v2) at the monitor's DPI, which are physical pixels. The arithmetic is in
//! [`crate::dpiscale`].

use super::*;
use crate::dpiscale::{self, Units};

/// The `msg` of an [`Incoming`] with no window procedure that a scale change hands a
/// per-monitor aware top-level window's thread: `w` the new DPI and the old one above it, `l`
/// the suggested size (width, height above). `user32` runs
/// the window's `WM_DPICHANGED` sequence (`user::dpi_changed`). Not a window message.
pub const DPI_CHANGED: u32 = 0x5059_0002;

/// The `msg` of an [`Incoming`] that moves a window the system scales to where its visible frame
/// stays put (`dpiscale::virtualized_move`): `w` and `l` as [`pack_set_pos`], in the window's
/// units; `user32` calls `SetWindowPos` without a size, which sends `WM_MOVE`.
pub const DPI_MOVE: u32 = 0x5059_0003;

/// A window's rectangles in its own units, kept while they still map to the physical ones:
/// at 175 percent a system aware unit is smaller than a physical pixel, so the physical
/// rectangles alone could not give back what the window's thread set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Own {
    pub window_rel: Rect,
    pub client_rel: Rect,
    /// The physical rectangles they were mapped to.
    phys: (Rect, Rect),
    /// The monitor's DPI and the window's they are valid for.
    monitor: u32,
    units: u32,
}

/// Whether an awareness is one the system scales by bitmap (unaware, GDI-scaled, system aware).
fn virtualized(a: u8) -> bool {
    matches!(a, guic::AWARE_UNAWARE | guic::AWARE_GDI_SCALED | guic::AWARE_SYSTEM)
}

impl Wm {
    /// The monitor's DPI now.
    pub fn monitor_dpi(&self) -> u32 {
        self.monitor_dpi
    }

    /// The DPI of an awareness: what a thread or window of it works at.
    fn awareness_dpi(&self, a: u8) -> u32 {
        match a {
            guic::AWARE_SYSTEM => self.system_dpi,
            guic::AWARE_PER_MONITOR | guic::AWARE_PER_MONITOR_V2 => self.monitor_dpi,
            _ => 96,
        }
    }

    /// The units thread `key` works in.
    pub fn units_of(&self, key: Key) -> Units {
        Units::new(self.awareness_dpi(self.thread_awareness(key)), self.monitor_dpi)
    }

    /// `GetDpiForWindow`: the desktop is 96 (`GetDpiForWindow.yaml` `at_200_kinds`).
    pub(super) fn window_dpi(&self, h: u32) -> u32 {
        if h == self.desktop {
            return 96;
        }
        self.get(u64::from(h)).map_or(96, |w| self.awareness_dpi(w.dpi))
    }

    /// The units a window's rectangles are in for its own thread (the desktop: physical).
    fn window_units(&self, h: u32) -> u32 {
        if h == self.desktop || h == self.message {
            return self.monitor_dpi;
        }
        self.get(u64::from(h)).map_or(self.monitor_dpi, |w| self.awareness_dpi(w.dpi))
    }

    /// The units of a top-level window's surface buffer against physical pixels: a window the
    /// system scales draws in its own units and the compositor enlarges its buffer; (1, 1) for a
    /// window in physical pixels.
    pub fn buffer_scale(&self, h: u32) -> (u32, u32) {
        let units = self.window_units(h);
        if units == self.monitor_dpi { (1, 1) } else { (units, self.monitor_dpi) }
    }

    /// The desktop as thread `key` sees it.
    pub(super) fn desktop_reply(&self, key: Key) -> Reply {
        let u = self.units_of(key);
        let a = self.thread_awareness(key);
        let dpi = if matches!(a, guic::AWARE_UNAWARE | guic::AWARE_GDI_SCALED) { 96 } else { self.system_dpi };
        let w = self.display.work;
        Reply::Desktop {
            desktop: self.desktop,
            message: self.message,
            width: u.to_caller(self.screen.0),
            height: u.to_caller(self.screen.1),
            work: Rect::new(u.to_caller(w.left), u.to_caller(w.top), u.to_caller(w.right), u.to_caller(w.bottom)),
            interactive: self.display.interactive,
            dpi,
            monitor_dpi: u.caller,
            phys_width: self.screen.0,
            phys_height: self.screen.1,
            process_dpi: self.system_dpi,
            scale: self.scale_percent(),
        }
    }

    /// The monitor's scale in percent.
    pub fn scale_percent(&self) -> u32 {
        (self.monitor_dpi * 100).div_ceil(96).min(500)
    }

    /// A window's rectangles relative to its parent's client area, in units of `caller` DPI.
    fn rels_for(&self, h: u32, u: &Units) -> Option<(Rect, Rect)> {
        let w = self.get(u64::from(h))?;
        if let Some(o) = w.own
            && o.phys == (w.window_rel, w.client_rel)
            && o.monitor == self.monitor_dpi
            && o.units == u.caller
            && self.window_units(h) == u.caller
        {
            return Some((o.window_rel, o.client_rel));
        }
        Some((u.rect_to_caller(w.window_rel), u.rect_to_caller(w.client_rel)))
    }

    /// A window's client origin on the screen in the caller's units.
    fn client_origin_for(&self, h: u32, u: &Units) -> (i32, i32) {
        let Some(w) = self.get(u64::from(h)) else { return (0, 0) };
        let (px, py) = if w.parent == 0 { (0, 0) } else { self.client_origin_for(w.parent, u) };
        let (_, c) = self.rels_for(h, u).unwrap_or_default();
        (px + c.left, py + c.top)
    }

    /// What `GetWindowRect`, `GetClientRect` and the rest read of a window, in the caller's units.
    fn info_for(&self, mut i: WinInfo, u: &Units) -> WinInfo {
        let h = i.hwnd;
        let Some((wr, cr)) = self.rels_for(h, u) else { return i };
        let (ox, oy) = match self.get(u64::from(h)) {
            Some(w) if w.parent != 0 => self.client_origin_for(w.parent, u),
            _ => (0, 0),
        };
        let shift = |r: Rect| Rect::new(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy);
        i.window = shift(wr);
        i.client = shift(cr);
        i.window_rel = wr;
        i.client_rel = cr;
        if (i.min_x, i.min_y) != (-1, -1) {
            i.min_x = u.to_caller(i.min_x);
            i.min_y = u.to_caller(i.min_y);
        }
        i.menu_bar = u.size_to_caller(i.menu_bar);
        i
    }

    /// Run one request of thread `ctx`: its coordinates are in the thread's units.
    pub fn handle(&mut self, ctx: Ctx, req: Request, fx: &mut Effects) -> Reply {
        self.queue(ctx.key());
        let u = self.units_of(ctx.key());
        if u.identity() {
            return self.handle_raw(ctx, req, fx);
        }
        // Rectangles a thread sets on a window of its own units are kept as it gave them.
        let keep = match &req {
            Request::SetRects { hwnd, window_rel, client_rel } if self.window_units(*hwnd) == u.caller => {
                self.rels_for(*hwnd, &u).map(|(w, c)| (*hwnd, window_rel.unwrap_or(w), client_rel.unwrap_or(c)))
            }
            _ => None,
        };
        // Replies whose value packs a point.
        let packed = match &req {
            Request::Cursor { .. } => 1,
            Request::MessageInfo {} => 2,
            _ => 0,
        };
        let req = self.request_to_phys(&u, req);
        let reply = self.handle_raw(ctx, req, fx);
        let reply = match (packed, reply) {
            (1, Reply::Value { value }) => Reply::Value {
                value: u64::from(u.to_caller(value as u32 as i32) as u32)
                    | (u64::from(u.to_caller((value >> 32) as u32 as i32) as u32) << 32),
            },
            (2, Reply::Value { value }) => {
                let x = u.to_caller(i32::from((value >> 32) as u16 as i16));
                let y = u.to_caller(i32::from((value >> 48) as u16 as i16));
                Reply::Value {
                    value: (value & 0xFFFF_FFFF) | (u64::from(x as u16) << 32) | (u64::from(y as u16) << 48),
                }
            }
            (_, r) => r,
        };
        if let Some((h, wr, cr)) = keep
            && matches!(reply, Reply::Ok {})
        {
            let monitor = self.monitor_dpi;
            if let Some(w) = self.get_mut(u64::from(h)) {
                w.own = Some(Own {
                    window_rel: wr,
                    client_rel: cr,
                    phys: (w.window_rel, w.client_rel),
                    monitor,
                    units: u.caller,
                });
            }
        }
        self.reply_to_caller(&u, reply)
    }

    /// [`Wm::handle`] for a request that only reads (ADR 0249), without changing anything: the
    /// same reply `handle` gives now, or `None` for every other request and for a thread whose
    /// first request this is (`handle` makes its queue). `user32` caches what `Window` answers
    /// and asks again once the window state changed ([`Wm`] users bump a generation on every
    /// change); answering here keeps the asking from counting as a change.
    pub fn handle_read(&self, ctx: Ctx, req: &Request) -> Option<Reply> {
        if !self.has_queue(ctx.key()) {
            return None;
        }
        let reply = match req {
            Request::Window { hwnd } => match self.info(*hwnd as u32).filter(|_| self.is_window(*hwnd)) {
                Some(info) => Reply::Window { info },
                None => Reply::Error { code: crate::wm::ERROR_INVALID_WINDOW_HANDLE },
            },
            Request::Desktop {} => self.desktop_reply(ctx.key()),
            Request::GetLong { hwnd, index, size } => self.get_long(*hwnd, *index, *size),
            Request::KeyboardState {} => Reply::Keys { keys: self.keys_of(ctx.key()) },
            _ => return None,
        };
        let u = self.units_of(ctx.key());
        Some(if u.identity() { reply } else { self.reply_to_caller(&u, reply) })
    }

    /// [`Wm::handle`] for a request whose coordinates are physical already.
    pub fn handle_phys(&mut self, ctx: Ctx, req: Request, fx: &mut Effects) -> Reply {
        self.handle_raw(ctx, req, fx)
    }

    fn request_to_phys(&self, u: &Units, req: Request) -> Request {
        let p = |v: i32| u.to_phys(v);
        let r = |x: Rect| u.rect_to_phys(x);
        match req {
            Request::CreateWindow { mut spec } => {
                if spec.x != i32::MIN {
                    spec.x = p(spec.x);
                    spec.y = p(spec.y);
                }
                if spec.cx != i32::MIN {
                    spec.cx = p(spec.cx);
                    spec.cy = p(spec.cy);
                }
                Request::CreateWindow { spec }
            }
            Request::SetRects { hwnd, window_rel, client_rel } => {
                Request::SetRects { hwnd, window_rel: window_rel.map(r), client_rel: client_rel.map(r) }
            }
            Request::SetState { hwnd, visible, minimized, maximized, normal, size_move_pending } => {
                Request::SetState { hwnd, visible, minimized, maximized, normal: normal.map(r), size_move_pending }
            }
            Request::SetMinPos { hwnd, x, y } if (x, y) != (-1, -1) => Request::SetMinPos { hwnd, x: p(x), y: p(y) },
            Request::Redraw { hwnd, rects, whole, flags } => {
                Request::Redraw { hwnd, rects: rects.into_iter().map(r).collect(), whole, flags }
            }
            Request::Caret { op, hwnd, a, b, tid } => Request::Caret { op, hwnd, a: p(a), b: p(b), tid },
            Request::Cursor { set, x, y } => Request::Cursor { set, x: p(x), y: p(y) },
            Request::WindowFromPoint { x, y, below } => Request::WindowFromPoint { x: p(x), y: p(y), below },
            Request::ScreenPixel { x, y } => Request::ScreenPixel { x: p(x), y: p(y) },
            Request::WindowRgn { hwnd, set, rects } => {
                Request::WindowRgn { hwnd, set, rects: rects.into_iter().map(|v| p(v as i32) as u32).collect() }
            }
            Request::MenuBar { hwnd, height } => Request::MenuBar { hwnd, height: p(height) },
            other => other,
        }
    }

    fn reply_to_caller(&self, u: &Units, reply: Reply) -> Reply {
        let c = |v: i32| u.to_caller(v);
        match reply {
            Reply::Window { info } => Reply::Window { info: self.info_for(info, u) },
            Reply::Created { mut created } => {
                created.x = c(created.x);
                created.y = c(created.y);
                created.cx = u.size_to_caller(created.cx);
                created.cy = u.size_to_caller(created.cy);
                Reply::Created { created }
            }
            Reply::Update { rects, erase, frame, frame_whole, internal, ferase, visible } => Reply::Update {
                rects: rects.into_iter().map(|r| u.rect_to_caller(r)).collect(),
                erase,
                frame,
                frame_whole,
                internal,
                ferase,
                visible,
            },
            Reply::Message { mut msg } => {
                msg.x = c(msg.x);
                msg.y = c(msg.y);
                // Mouse messages carry a point: client (0x200..) or screen (non-client, wheel).
                if (0x00A0..=0x00AD).contains(&msg.msg) || (0x0200..=0x020E).contains(&msg.msg) {
                    msg.l = u.lparam_to_caller(msg.l);
                }
                Reply::Message { msg }
            }
            Reply::Incoming { mut incoming } => {
                if incoming.wndproc == 0 && incoming.msg == DESKTOP_SET_POS {
                    let (x, y, cx, cy) = unpack_set_pos(incoming.w, incoming.l);
                    let r = Rect::new(c(x), c(y), c(x) + u.size_to_caller(cx), c(y) + u.size_to_caller(cy));
                    (incoming.w, incoming.l) = pack_set_pos(r);
                }
                Reply::Incoming { incoming }
            }
            Reply::Caret { ok, code, hwnd, width, height, x, y, shown } => Reply::Caret {
                ok,
                code,
                hwnd,
                width: u.size_to_caller(width),
                height: u.size_to_caller(height),
                x: c(x),
                y: c(y),
                shown,
            },
            other => other,
        }
    }

    /// `DisplayConfigSetDeviceInfo`'s scale step (ADR 0231): the monitor's DPI changes, the
    /// windows the system scales keep their size in their units and their visible frame's place,
    /// and the per-monitor aware ones are told (`WM_DPICHANGED`, through their threads).
    pub(super) fn set_scale(&mut self, step: i32, fx: &mut Effects) -> Reply {
        let max = dpiscale::max_step(self.screen.0.max(1) as u32, self.screen.1.max(1) as u32);
        if step < 0 || step as usize > max {
            return err(ERROR_INVALID_PARAMETER);
        }
        let new = dpiscale::dpi_of(dpiscale::STEPS[step as usize]);
        let old = self.monitor_dpi;
        if new == old {
            return Reply::Ok {};
        }
        // Every window's rectangles in its units before the change.
        let all: Vec<u32> = (1..self.slots.len())
            .filter(|&i| self.slots[i].is_some())
            .map(|i| u32::from(self.gens[i]) << 16 | i as u32)
            .filter(|&h| h != self.desktop && h != self.message)
            .collect();
        let mut kept = Vec::new();
        for &h in &all {
            let Some(w) = self.get(u64::from(h)) else { continue };
            let units = self.window_units(h);
            if virtualized(w.dpi) {
                if let Some(r) = self.rels_for(h, &Units::new(units, old)) {
                    kept.push((h, units, r));
                }
            } else if let Some(w) = self.get_mut(u64::from(h)) {
                // A per-monitor aware frame stays at the old DPI until DefWindowProc handles
                // WM_DPICHANGED.
                w.frame_pin = old;
            }
        }
        self.monitor_dpi = new;
        for (h, units, (wr, cr)) in kept {
            let u = Units::new(units, new);
            let monitor = self.monitor_dpi;
            if let Some(w) = self.get_mut(u64::from(h)) {
                w.window_rel = u.rect_to_phys(wr);
                w.client_rel = u.rect_to_phys(cr);
                w.own =
                    Some(Own { window_rel: wr, client_rel: cr, phys: (w.window_rel, w.client_rel), monitor, units });
            }
        }
        // The top-level windows, topmost first, each told through its thread.
        for h in self.top_levels() {
            let Some(w) = self.get(u64::from(h)) else { continue };
            if w.parent != self.desktop || h == self.keeper {
                continue;
            }
            let to = w.thread;
            let (style, ex, a) = (w.style, w.ex_style, w.dpi);
            if virtualized(a) {
                let units = self.window_units(h);
                let Some((wr, _)) = self.rels_for(h, &Units::new(units, new)) else { continue };
                let inv = (crate::metrics::hidden_frame_dpi(style, ex, units) - 1).max(0);
                let (x, y) = dpiscale::virtualized_move(wr.left, wr.top, inv, old, new);
                if (x, y) != (wr.left, wr.top) {
                    let (pw, pl) = pack_set_pos(Rect::new(x, y, x + wr.width(), y + wr.height()));
                    self.system_send(h, to, DPI_MOVE, pw, pl, fx);
                }
            } else {
                let size = (w.window_rel.width(), w.window_rel.height());
                let (cx, cy) = self.dpi_suggest(h, size, new, old);
                let l = u64::from(cx as u32) | (u64::from(cy as u32) << 32);
                self.system_send(h, to, DPI_CHANGED, u64::from(new) | (u64::from(old) << 32), l, fx);
            }
        }
        Reply::Ok {}
    }

    /// `DefWindowProc`'s `WM_DPICHANGED`: the frame is measured at the window's DPI again.
    pub(super) fn dpi_frame(&mut self, hwnd: u32) -> Reply {
        match self.get_mut(u64::from(hwnd)) {
            Some(w) => {
                w.frame_pin = 0;
                Reply::Ok {}
            }
            None => err(ERROR_INVALID_WINDOW_HANDLE),
        }
    }

    /// The size `WM_DPICHANGED` suggests for window `hwnd` going from `old` to `new` DPI (its
    /// current size in physical pixels), remembered for the next change.
    pub fn dpi_suggest(&mut self, hwnd: u32, size: (i32, i32), new: u32, old: u32) -> (i32, i32) {
        let Some(w) = self.get_mut(u64::from(hwnd)) else { return dpiscale::scaled_size(size, new, old) };
        let (s, b) = dpiscale::suggest(w.suggest, size, new, old);
        w.suggest = Some(b);
        s
    }

    /// Queue a message from no thread for window `hwnd`'s thread `to`.
    fn system_send(&mut self, hwnd: u32, to: Key, msg: u32, w: u64, l: u64, fx: &mut Effects) {
        let id = self.next_send;
        self.next_send += 1;
        let incoming = Incoming { id, hwnd, msg, w, l, wndproc: 0, data: None };
        self.sends.insert(id, Send { from: DESKTOP_SENDER, to, incoming, result: None, collected: false });
        let q = self.queue(to);
        q.sent.push_back(id);
        q.changed |= QS_SENDMESSAGE;
        fx.wake.push(to);
    }
}
