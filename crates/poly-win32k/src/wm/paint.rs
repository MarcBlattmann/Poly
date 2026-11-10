//! Painting (ADR 0131, slice B): each window's update region, its pending
//! erase and frame, and which window gets the next `WM_PAINT`.
//!
//! Only an interactive session paints (the slice A oracle's session 0 kept
//! no update regions, InvalidateRect.yaml `not_covered`). The rules are the
//! slice B sheets': InvalidateRect / ValidateRect / RedrawWindow change the
//! region (client coordinates, clipped to the client area); invalidating a
//! parent without `WS_CLIPCHILDREN` invalidates the visible children under
//! the area too, which then also have their frame and erase pending; a
//! hidden window has nothing invalid; `WM_PAINT` comes after posted
//! messages and before timers, windows of later-created top-level windows
//! first, a parent before its children, later-created children first.
//! Sending the erase and frame messages is `user32`'s
//! (`crate::user`); this module answers what is pending and records what
//! was done.

use super::*;
use crate::rgn::Rgn;

/// `RDW_*` (winuser.h).
pub const RDW_INVALIDATE: u32 = 0x0001;
pub const RDW_INTERNALPAINT: u32 = 0x0002;
pub const RDW_ERASE: u32 = 0x0004;
pub const RDW_VALIDATE: u32 = 0x0008;
pub const RDW_NOINTERNALPAINT: u32 = 0x0010;
pub const RDW_NOERASE: u32 = 0x0020;
pub const RDW_NOCHILDREN: u32 = 0x0040;
pub const RDW_ALLCHILDREN: u32 = 0x0080;
pub const RDW_UPDATENOW: u32 = 0x0100;
pub const RDW_ERASENOW: u32 = 0x0200;
pub const RDW_FRAME: u32 = 0x0400;
pub const RDW_NOFRAME: u32 = 0x0800;

/// `Painted` bits: what `user32` did.
pub const PAINTED_ERASE: u32 = 1;
pub const PAINTED_FRAME: u32 = 2;
pub const PAINTED_VALIDATE: u32 = 4;
/// The pending frame is a part of the window (WM_NCPAINT gets a region).
pub const PAINTED_FRAME_PARTIAL: u32 = 8;

/// A window's painting state.
#[derive(Clone, Debug)]
pub(super) struct Paint {
    /// The update region, client coordinates.
    pub upd: Rgn,
    pub erase: bool,
    /// `WM_NCPAINT` pending, for all of the window (`whole`) or a part.
    pub frame: bool,
    pub frame_whole: bool,
    /// RDW_INTERNALPAINT: a `WM_PAINT` with nothing invalid.
    pub internal: bool,
    /// What BeginPaint reports as fErase when no erase is pending: the last
    /// erase's outcome (1 when the procedure returned 0); 0 for a window
    /// that was never erased (EndPaint.yaml ep_outside: a hidden window).
    pub ferase: bool,
}

impl Default for Paint {
    fn default() -> Self {
        Paint { upd: Rgn::new(), erase: false, frame: false, frame_whole: false, internal: false, ferase: false }
    }
}

impl Wm {
    fn client_size(&self, h: u32) -> (i32, i32) {
        self.get(u64::from(h)).map_or((0, 0), |w| (w.client_rel.width().max(0), w.client_rel.height().max(0)))
    }

    /// Whether `h` is to get a `WM_PAINT`.
    pub(super) fn needs_paint(&self, h: u32) -> bool {
        self.visible(h) && self.get(u64::from(h)).is_some_and(|w| !w.paint.upd.is_empty() || w.paint.internal)
    }

    fn thread_needs_paint(&self, key: Key) -> bool {
        (1..self.slots.len()).any(|i| {
            let h = (u32::from(self.gens[i]) << 16) | i as u32;
            self.slots[i].as_ref().is_some_and(|w| w.thread == key) && self.needs_paint(h)
        })
    }

    /// RedrawWindow's work on `h` (and its children as `flags` say): `area`
    /// in client coordinates, `None` for the whole client area.
    pub(super) fn redraw(&mut self, h: u32, area: Option<Rgn>, flags: u32, fx: &mut Effects) {
        if !self.display.interactive || !self.visible(h) {
            return;
        }
        // The child of a minimized window: InvalidateRect leaves nothing
        // invalid (GetUpdateRect.yaml `minimized`).
        if flags & RDW_INVALIDATE != 0 && self.below_minimized(h) {
            return;
        }
        let (cw, ch) = self.client_size(h);
        let client = Rgn::rect(Rect::new(0, 0, cw, ch));
        let whole = area.is_none();
        let area = area.map_or_else(|| client.clone(), |a| a.intersect(&client));
        let Some(w) = self.get(u64::from(h)) else { return };
        let thread = w.thread;
        let had = self.needs_paint(h);
        let minimized = w.minimized;
        let clip_children = w.style & WS_CLIPCHILDREN != 0;
        let w = self.get_mut(u64::from(h)).expect("found");
        let p = &mut w.paint;
        if flags & RDW_INVALIDATE != 0 {
            p.upd = p.upd.union(&area);
            if flags & RDW_ERASE != 0 && (!area.is_empty() || minimized) {
                p.erase = true;
            }
            if flags & RDW_FRAME != 0 {
                p.frame = true;
                p.frame_whole = true;
            }
            if minimized {
                // A minimized window has no client area, yet it gets a
                // WM_PAINT (GetUpdateRect.yaml `minimized`).
                p.internal = true;
            }
        } else if flags & RDW_VALIDATE != 0 {
            p.upd = if whole { Rgn::new() } else { p.upd.subtract(&area) };
            if p.upd.is_empty() {
                p.erase = false;
                p.frame = false;
                if whole {
                    p.internal = false;
                }
            }
            if flags & RDW_NOERASE != 0 {
                p.erase = false;
            }
            if flags & RDW_NOFRAME != 0 {
                p.frame = false;
            }
        }
        if flags & RDW_INTERNALPAINT != 0 {
            p.internal = true;
        }
        if flags & RDW_NOINTERNALPAINT != 0 {
            p.internal = false;
        }
        let children = flags & RDW_ALLCHILDREN != 0 || (flags & RDW_NOCHILDREN == 0 && !clip_children);
        if children && flags & (RDW_INVALIDATE | RDW_VALIDATE) != 0 {
            let kids = self.get(u64::from(h)).map(|w| w.children.clone()).unwrap_or_default();
            for c in kids {
                let Some(cw) = self.get(u64::from(c)) else { continue };
                if cw.style & WS_VISIBLE == 0 {
                    continue;
                }
                let (wr, cr) = (cw.window_rel, cw.client_rel);
                let under = area.intersect(&Rgn::rect(wr));
                if under.is_empty() {
                    continue;
                }
                let covers_window = under.covers(wr);
                let child_area = under.offset(-cr.left, -cr.top);
                // Invalidated through its parent, a child also has its
                // frame and erase pending (InvalidateRect.yaml ir_children).
                let child_flags = if flags & RDW_INVALIDATE != 0 { flags | RDW_ERASE | RDW_FRAME } else { flags };
                self.redraw(c, Some(child_area), child_flags & !RDW_ALLCHILDREN | (flags & RDW_ALLCHILDREN), fx);
                if flags & RDW_INVALIDATE != 0
                    && let Some(cwm) = self.get_mut(u64::from(c))
                {
                    cwm.paint.frame_whole = covers_window;
                }
            }
        }
        if !had && self.needs_paint(h) {
            self.queue(thread).changed |= QS_PAINT;
            fx.wake.push(thread);
        }
    }

    /// A window hidden (or one of its ancestors): nothing of it or below
    /// stays invalid.
    pub(super) fn clear_paint_tree(&mut self, h: u32) {
        let kids = match self.get_mut(u64::from(h)) {
            Some(w) => {
                let ferase = w.paint.ferase;
                w.paint = Paint { ferase, ..Paint::default() };
                w.children.clone()
            }
            None => return,
        };
        for c in kids {
            self.clear_paint_tree(c);
        }
    }

    /// A visible window minimized (GetUpdateRect.yaml `minimized`): it has
    /// no client area to update, but what was invalid still brings it one
    /// WM_PAINT; its frame is to paint (WM_NCPAINT, all of it). Nothing of
    /// its descendants stays invalid; a frame they had pending stays.
    pub(super) fn minimized_paint(&mut self, h: u32) {
        if !self.display.interactive {
            return;
        }
        let Some(w) = self.get_mut(u64::from(h)) else { return };
        let p = &mut w.paint;
        p.internal = p.internal || !p.upd.is_empty();
        p.upd = Rgn::new();
        p.frame = true;
        p.frame_whole = true;
        let kids = w.children.clone();
        for c in kids {
            self.clear_updates_below_minimized(c);
        }
    }

    fn clear_updates_below_minimized(&mut self, h: u32) {
        let Some(w) = self.get_mut(u64::from(h)) else { return };
        let p = &mut w.paint;
        p.upd = Rgn::new();
        p.erase = false;
        p.internal = false;
        if p.frame {
            p.frame_whole = true;
        }
        let kids = w.children.clone();
        for c in kids {
            self.clear_updates_below_minimized(c);
        }
    }

    /// Whether `h` or a window above it is minimized.
    fn below_minimized(&self, h: u32) -> bool {
        let mut cur = h;
        for _ in 0..64 {
            let Some(w) = self.get(u64::from(cur)) else { return false };
            if cur != h && w.minimized {
                return true;
            }
            if w.parent == 0 || w.parent == self.desktop || w.parent == self.message {
                return false;
            }
            cur = w.parent;
        }
        false
    }

    /// A window shown: its whole client area invalid, frame and erase
    /// pending (`user32` sends them at once where the sheets saw that).
    pub(super) fn shown(&mut self, h: u32, fx: &mut Effects) {
        if !self.display.interactive {
            return;
        }
        self.redraw(h, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_NOCHILDREN, fx);
        let kids = self.get(u64::from(h)).map(|w| w.children.clone()).unwrap_or_default();
        for c in kids {
            if self.get(u64::from(c)).is_some_and(|w| w.style & WS_VISIBLE != 0) {
                self.shown(c, fx);
            }
        }
    }

    /// What is pending for `h`.
    pub(super) fn update_of(&self, h: u32) -> Reply {
        let Some(w) = self.get(u64::from(h)) else { return err(ERROR_INVALID_WINDOW_HANDLE) };
        let p = &w.paint;
        Reply::Update {
            rects: p.upd.rects().to_vec(),
            erase: p.erase,
            frame: p.frame,
            frame_whole: p.frame_whole,
            internal: p.internal,
            ferase: p.ferase,
            visible: self.visible(h),
        }
    }

    /// What `user32` did for `h`: sent the erase or the frame, validated.
    pub(super) fn painted(&mut self, h: u32, what: u32, ferase: u8) {
        let Some(w) = self.get_mut(u64::from(h)) else { return };
        let p = &mut w.paint;
        if what & PAINTED_ERASE != 0 {
            p.erase = false;
        }
        if what & PAINTED_FRAME != 0 {
            p.frame = false;
        }
        if what & PAINTED_FRAME_PARTIAL != 0 {
            p.frame_whole = false;
        }
        if what & PAINTED_VALIDATE != 0 {
            p.upd = Rgn::new();
            p.erase = false;
            p.frame = false;
            p.internal = false;
        }
        match ferase {
            1 => p.ferase = true,
            2 => p.ferase = false,
            _ => {}
        }
    }

    /// The windows of thread `key` to paint, in the order WM_PAINT comes
    /// (within `root` and its descendants, `root` first, when given).
    pub(super) fn paint_order(&self, key: Key, root: Option<u32>) -> Vec<u32> {
        let mut out = Vec::new();
        // Within a given window: any thread's (UpdateWindow / RDW_UPDATENOW
        // of another thread's window sends it WM_PAINT).
        let any_thread = root.is_some();
        let tops: Vec<u32> = match root {
            Some(r) => alloc::vec![r],
            None => {
                let mut v: Vec<(u64, u32)> = self
                    .get(u64::from(self.desktop))
                    .map(|d| d.children.iter().filter_map(|&c| self.get(u64::from(c)).map(|w| (w.seq, c))).collect())
                    .unwrap_or_default();
                v.sort_by_key(|&(seq, _)| core::cmp::Reverse(seq));
                v.into_iter().map(|(_, c)| c).collect()
            }
        };
        for t in tops {
            self.paint_walk(key, any_thread, t, &mut out);
        }
        out
    }

    fn paint_walk(&self, key: Key, any_thread: bool, h: u32, out: &mut Vec<u32>) {
        let Some(w) = self.get(u64::from(h)) else { return };
        if (any_thread || w.thread == key) && self.needs_paint(h) {
            out.push(h);
        }
        let mut kids: Vec<(u64, u32)> =
            w.children.iter().filter_map(|&c| self.get(u64::from(c)).map(|cw| (cw.seq, c))).collect();
        kids.sort_by_key(|&(seq, _)| core::cmp::Reverse(seq));
        for (_, c) in kids {
            self.paint_walk(key, any_thread, c, out);
        }
    }

    /// The `WM_PAINT` the queue yields now (it stays while the window is
    /// invalid: PM_REMOVE does not validate).
    pub(super) fn paint_message(&self, ctx: Ctx, filter: u64, thread_only: bool) -> Option<Msg> {
        if thread_only {
            return None;
        }
        let h = self.paint_order(ctx.key(), None).into_iter().find(|&h| filter == 0 || u64::from(h) == filter)?;
        Some(Msg {
            hwnd: h,
            msg: WM_PAINT,
            w: 0,
            l: 0,
            time: ctx.now as u32,
            x: self.cursor.0,
            y: self.cursor.1,
            private: 0,
        })
    }

    pub(super) fn paint_status(&self, key: Key) -> u32 {
        if self.thread_needs_paint(key) { QS_PAINT } else { 0 }
    }
}
