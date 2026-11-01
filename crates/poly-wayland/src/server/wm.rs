//! Window management for a shell (ADR 0117): what a shell needs to manage
//! the windows without knowing whose they are.
//!
//! - **The managed windows** ([`Server::managed_windows`]): every mapped
//!   toplevel and every window the compositor draws itself, bottom to top,
//!   with title, app ID and state (active, minimized, maximized, tiled).
//! - **State changes**: activate (un-minimizing), minimize, maximize,
//!   tile to a rectangle, restore, close. For a client's toplevel a state
//!   change is a configure sequence (xdg-shell.xml): the new size and
//!   states (`maximized`, `tiled_*` from version 2, `activated`) go out,
//!   and the window moves when the client has acked and committed a
//!   buffer. Minimizing has no event in xdg-shell: the window is taken off
//!   the output and the focus goes to the next window.
//! - **Internal windows**: windows the compositor draws itself (the
//!   shell's settings window), in Poly's frame like any toplevel, moved,
//!   raised, maximized, tiled, minimized and switched to the same way.
//!   Their input is queued as [`InternalEvent`]s for their owner, which
//!   draws their pixels ([`Server::set_internal_pixels`]).

use super::{ClientId, Data, Server, Toplevel, owner_of, owner_tag};
use crate::compose::{BORDER, Rect, TITLE_HEIGHT, WindowId};
use crate::protocol::{xdg_popup, xdg_toplevel};
use crate::wire::ObjectId;
use alloc::string::String;
use alloc::vec::Vec;

/// The client in the owner tag of internal windows: never a real client
/// (client IDs start at 1).
const INTERNAL: ClientId = ClientId(0);

/// A managed window as a shell sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub window: WindowId,
    pub title: String,
    pub app_id: String,
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
    /// The frame's outer rectangle (logical) it is tiled to.
    pub tiled: Option<Rect>,
    /// Drawn by the compositor itself.
    pub internal: bool,
}

/// Input and requests for an internal window, for its owner to handle.
/// Coordinates are logical, relative to the window's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InternalEvent {
    Motion {
        window: WindowId,
        x: i32,
        y: i32,
    },
    /// The pointer left the window.
    Leave(WindowId),
    Button {
        window: WindowId,
        x: i32,
        y: i32,
        button: u32,
        pressed: bool,
    },
    /// The wheel: logical pixels (15 a step), positive down or right.
    Axis {
        window: WindowId,
        horizontal: bool,
        value: i32,
    },
    /// An evdev key code while the window has the keyboard (the modifiers
    /// are [`Server::modifiers`]).
    Key {
        window: WindowId,
        key: u32,
        pressed: bool,
    },
    /// The close button (or a close request): the owner removes it.
    Close(WindowId),
    /// Maximized, tiled or restored: draw the content this size (logical).
    Resize {
        window: WindowId,
        width: i32,
        height: i32,
    },
}

/// The state of an internal window.
#[derive(Debug)]
pub(crate) struct Internal {
    pub app_id: String,
    pub maximized: bool,
    pub tile: Option<Rect>,
    /// The content's floating rectangle to go back to (logical).
    pub restore: Option<Rect>,
    /// The content's size now (logical).
    pub size: (i32, i32),
}

/// The bounds a frame's outer rectangle leaves for the content.
fn inside_frame(outer: Rect) -> Rect {
    Rect::new(
        outer.x + BORDER,
        outer.y + TITLE_HEIGHT + BORDER,
        outer.width - 2 * BORDER,
        outer.height - TITLE_HEIGHT - 2 * BORDER,
    )
}

impl Server {
    /// The toplevel shown in window `w`, without changing anything.
    pub(super) fn toplevel_of(&self, w: WindowId) -> Option<(ClientId, ObjectId, &Toplevel)> {
        let (c, surface) = owner_of(self.scene.window(w)?.owner());
        let client = self.clients.get(&c)?;
        let Some(Data::Surface(s)) = client.data(surface) else { return None };
        let Some(Data::XdgSurface(x)) = client.data(s.xdg?) else { return None };
        let role = x.role?;
        match client.data(role) {
            Some(Data::Toplevel(t)) => Some((c, role, t)),
            _ => None,
        }
    }

    /// Whether `w` is drawn by the compositor itself.
    pub fn is_internal(&self, w: WindowId) -> bool {
        self.internal.contains_key(&w)
    }

    /// The window with the focus: an active toplevel's or an internal one.
    pub fn active_window(&self) -> Option<WindowId> {
        if self.active_internal.is_some() {
            return self.active_internal;
        }
        let (c, tl) = self.active?;
        self.scene
            .windows()
            .iter()
            .map(|w| w.id())
            .find(|&w| self.toplevel_of(w).is_some_and(|(wc, wt, _)| (wc, wt) == (c, tl)))
    }

    /// Every mapped toplevel and internal window, bottom to top (minimized
    /// ones included, where they were).
    pub fn managed_windows(&self) -> Vec<WindowInfo> {
        let active = self.active_window();
        let mut out = Vec::new();
        for w in self.scene.windows() {
            let (cw, ch) = w.content().logical_size();
            if cw <= 0 || ch <= 0 {
                continue;
            }
            let id = w.id();
            let (app_id, maximized, tiled, internal) = if let Some(i) = self.internal.get(&id) {
                (i.app_id.clone(), i.maximized, i.tile, true)
            } else if let Some((_, _, t)) = self.toplevel_of(id) {
                (t.app_id.clone(), t.maximized, t.tile, false)
            } else {
                continue;
            };
            out.push(WindowInfo {
                window: id,
                title: w.title().into(),
                app_id,
                active: active == Some(id),
                minimized: w.is_hidden(),
                maximized,
                tiled,
                internal,
            });
        }
        out
    }

    /// The window being moved with the pointer, if one is.
    pub fn moving_window(&self) -> Option<WindowId> {
        self.seat.input.grab.map(|super::input::Grab::Move { window, .. }| window)
    }

    /// Show `w` if it was minimized, raise it and give it the focus.
    pub fn activate_window(&mut self, w: WindowId) {
        if self.scene.window(w).is_none() {
            return;
        }
        self.scene.set_hidden(w, false);
        self.focus_window(w);
        self.sync_keyboard_focus();
    }

    /// Minimize `w`: off the output (its popups dismissed), the focus to
    /// the next window if it had it.
    pub fn minimize_window(&mut self, w: WindowId) {
        if self.scene.window(w).is_none_or(|win| win.is_hidden()) {
            return;
        }
        if let Some((c, _, t)) = self.toplevel_of(w) {
            let xid = t.xdg;
            let popups = self.client(c).ids_where(|d| matches!(d, Data::Popup(p) if p.parent == Some(xid)));
            for p in popups {
                self.client(c).send(p, xdg_popup::Event::PopupDone);
            }
        }
        self.scene.set_hidden(w, true);
        if self.active_window() == Some(w) {
            self.focus_next();
        }
        if self.seat.input.grab.is_some() && self.moving_window() == Some(w) {
            self.seat.input.grab = None;
        }
        self.sync_keyboard_focus();
    }

    /// Give the focus to the topmost shown managed window, or to nobody.
    pub(super) fn focus_next(&mut self) {
        let next = self
            .scene
            .windows()
            .iter()
            .rev()
            .filter(|win| win.is_visible())
            .map(|win| win.id())
            .find(|&id| self.internal.contains_key(&id) || self.toplevel_of(id).is_some());
        match next {
            Some(id) => self.focus_window(id),
            None => {
                if let Some(iw) = self.active_internal.take() {
                    self.scene.set_active(iw, false);
                }
                if let Some((c, tl)) = self.active.take()
                    && self.clients.contains_key(&c)
                {
                    if let Some(win) = self.toplevel_window(c, tl) {
                        self.scene.set_active(win, false);
                    }
                    self.configure_toplevel(c, tl);
                }
            }
        }
        self.sync_keyboard_focus();
    }

    /// Ask `w` to close: a toplevel's client gets `xdg_toplevel.close`, an
    /// internal window's owner [`InternalEvent::Close`].
    pub fn request_close(&mut self, w: WindowId) {
        self.close_window(w);
    }

    /// Maximize `w` (to the work area) or put it back.
    pub fn set_maximized(&mut self, w: WindowId, on: bool) {
        if self.internal.contains_key(&w) {
            let target = if on { Some(self.scene.work_area()) } else { None };
            self.internal_place(w, on, None, target);
            return;
        }
        let Some((c, tl, _)) = self.toplevel_of(w) else { return };
        if let Some(t) = self.toplevel(c, tl) {
            t.want_maximized = on;
            t.want_tile = None;
        }
        self.reconfigure(c, tl);
    }

    /// Maximize `w`, or restore it if it is maximized or tiled.
    pub fn toggle_maximized(&mut self, w: WindowId) {
        let (max, tiled) = match self.managed_windows().into_iter().find(|i| i.window == w) {
            Some(i) => (i.maximized, i.tiled.is_some()),
            None => return,
        };
        if max || tiled { self.restore_window(w) } else { self.set_maximized(w, true) }
    }

    /// Tile `w` so that its frame's outer rectangle is `outer` (logical).
    pub fn tile_window(&mut self, w: WindowId, outer: Rect) {
        if self.internal.contains_key(&w) {
            self.internal_place(w, false, Some(outer), Some(outer));
            return;
        }
        let Some((c, tl, _)) = self.toplevel_of(w) else { return };
        if let Some(t) = self.toplevel(c, tl) {
            t.want_maximized = false;
            t.want_tile = Some(outer);
        }
        self.reconfigure(c, tl);
    }

    /// Neither maximized nor tiled any more: back to the floating size and
    /// place.
    pub fn restore_window(&mut self, w: WindowId) {
        if self.internal.contains_key(&w) {
            self.internal_place(w, false, None, None);
            return;
        }
        let Some((c, tl, _)) = self.toplevel_of(w) else { return };
        if let Some(t) = self.toplevel(c, tl) {
            t.want_maximized = false;
            t.want_tile = None;
        }
        self.reconfigure(c, tl);
    }

    /// A configure sequence for a toplevel whose wanted state changed (if
    /// it had its initial one).
    fn reconfigure(&mut self, c: ClientId, tl: ObjectId) {
        let Some(xid) = self.toplevel(c, tl).map(|t| t.xdg) else { return };
        let initial_done = matches!(self.client(c).data(xid), Some(Data::XdgSurface(x)) if x.initial_done);
        if initial_done {
            self.configure_toplevel(c, tl);
        }
    }

    // -----------------------------------------------------------------
    // Internal windows.

    /// A window the compositor draws itself, `width` × `height` logical,
    /// in a frame, placed like a new toplevel and given the focus. Its
    /// content is transparent until [`Server::set_internal_pixels`].
    pub fn add_internal_window(&mut self, title: &str, app_id: &str, width: i32, height: i32) -> WindowId {
        let tag = owner_tag(INTERNAL, ObjectId(self.next_internal));
        self.next_internal += 1;
        let win = self.scene.add_window(tag, true);
        let s = self.output.scale;
        self.scene.resize_content(win, width.max(1) as u32 * s, height.max(1) as u32 * s, s);
        self.scene.set_title(win, title);
        let (x, y) = self.scene.place(0, 0, width, height, true);
        self.scene.set_position(win, x, y);
        let state =
            Internal { app_id: app_id.into(), maximized: false, tile: None, restore: None, size: (width, height) };
        self.internal.insert(win, state);
        self.activate_internal(win);
        win
    }

    /// Remove an internal window.
    pub fn remove_internal_window(&mut self, w: WindowId) {
        if self.internal.remove(&w).is_none() {
            return;
        }
        let was_active = self.active_internal == Some(w);
        if was_active {
            self.active_internal = None;
        }
        let input = &mut self.seat.input;
        if input.internal_grab == Some(w) {
            input.internal_grab = None;
        }
        if input.internal_hover == Some(w) {
            input.internal_hover = None;
        }
        if self.moving_window() == Some(w) {
            self.seat.input.grab = None;
        }
        self.scene.remove_window(w);
        if was_active {
            self.focus_next();
        }
    }

    /// An internal window's content size now (logical).
    pub fn internal_size(&self, w: WindowId) -> Option<(i32, i32)> {
        self.internal.get(&w).map(|i| i.size)
    }

    pub fn set_internal_title(&mut self, w: WindowId, title: &str) {
        if self.internal.contains_key(&w) {
            self.scene.set_title(w, title);
        }
    }

    /// New pixels for an internal window: `width` × `height` physical
    /// `xrgb8888` (drawn at the output's scale), `damage` (physical) the
    /// part that changed (`None`: all).
    pub fn set_internal_pixels(&mut self, w: WindowId, width: u32, height: u32, pixels: &[u32], damage: Option<Rect>) {
        if !self.internal.contains_key(&w) || pixels.len() < width as usize * height as usize {
            return;
        }
        let s = self.output.scale;
        self.scene.resize_content(w, width, height, s);
        let Some(content) = self.scene.content_mut(w) else { return };
        for (dst, src) in content.pixels.iter_mut().zip(pixels) {
            *dst = src | 0xff00_0000;
        }
        let all = Rect::new(0, 0, width as i32, height as i32);
        self.scene.damage_content(w, damage.unwrap_or(all));
    }

    /// The internal events since the last call.
    pub fn take_internal_events(&mut self) -> Vec<InternalEvent> {
        core::mem::take(&mut self.internal_events)
    }

    /// Give internal window `w` the focus: the active toplevel hears it
    /// lost it, the keyboard leaves the clients.
    pub(super) fn activate_internal(&mut self, w: WindowId) {
        if self.active_internal == Some(w) {
            self.scene.raise(w);
            return;
        }
        if let Some((c, tl)) = self.active.take()
            && self.clients.contains_key(&c)
        {
            if let Some(pw) = self.toplevel_window(c, tl) {
                self.scene.set_active(pw, false);
            }
            self.configure_toplevel(c, tl);
        }
        if let Some(old) = self.active_internal.replace(w) {
            self.scene.set_active(old, false);
        }
        self.scene.set_active(w, true);
        self.scene.raise(w);
        self.sync_keyboard_focus();
    }

    /// Maximize, tile (`tile`, its outer rectangle `target`) or restore
    /// (`target` `None`) internal window `w`, at once.
    fn internal_place(&mut self, w: WindowId, maximized: bool, tile: Option<Rect>, target: Option<Rect>) {
        let Some(win) = self.scene.window(w) else { return };
        let (x, y) = win.position();
        let Some(i) = self.internal.get_mut(&w) else { return };
        let floating = !i.maximized && i.tile.is_none();
        if floating && target.is_some() {
            i.restore = Some(Rect::new(x, y, i.size.0, i.size.1));
        }
        let content = match target {
            Some(outer) => inside_frame(outer),
            None => match i.restore.take() {
                Some(r) => r,
                None if floating => return,
                None => Rect::new(x, y, i.size.0, i.size.1),
            },
        };
        (i.maximized, i.tile) = (maximized, tile);
        i.size = (content.width.max(1), content.height.max(1));
        let size = i.size;
        self.scene.set_maximized(w, maximized || tile.is_some());
        self.scene.set_position(w, content.x, content.y);
        self.internal_events.push(InternalEvent::Resize { window: w, width: size.0, height: size.1 });
    }

    /// The start of dragging maximized or tiled window `w` off its place:
    /// restore it with the pointer (`px`, `py`) at the same share of its
    /// title bar. Returns the content position it will have.
    pub(super) fn unsnap_for_drag(&mut self, w: WindowId, px: i32) -> Option<(i32, i32)> {
        let win = self.scene.window(w)?;
        let (wx, wy) = win.position();
        let cur_w = win.bounds().width.max(1);
        let gx = win.bounds().x - wx;
        let share = |floating_w: i32| (win.bounds().x - px) * floating_w / cur_w;
        if let Some(i) = self.internal.get(&w) {
            let r = i.restore?;
            let pos = (px + share(r.width) - gx, wy);
            if let Some(i) = self.internal.get_mut(&w) {
                i.restore = Some(Rect::new(pos.0, pos.1, r.width, r.height));
            }
            self.internal_place(w, false, None, None);
            return Some(pos);
        }
        let (c, tl, t) = self.toplevel_of(w)?;
        let floating_w = t.floating.map_or(cur_w, |f| f.0);
        let pos = (px + share(floating_w) - gx, wy);
        if let Some(t) = self.toplevel(c, tl) {
            t.want_maximized = false;
            t.want_tile = None;
            t.restore = Some(pos);
        }
        self.reconfigure(c, tl);
        Some(pos)
    }

    /// `xdg_toplevel` states for a tiled rectangle: the work area edges it
    /// touches (version 2 and later).
    pub(super) fn tiled_states(&self, outer: Rect, states: &mut Vec<u32>) {
        let a = self.scene.work_area();
        let near = |p: i32, q: i32| (p - q).abs() <= 16;
        for (touches, state) in [
            (near(outer.x, a.x), xdg_toplevel::State::TILED_LEFT),
            (near(outer.right(), a.right()), xdg_toplevel::State::TILED_RIGHT),
            (near(outer.y, a.y), xdg_toplevel::State::TILED_TOP),
            (near(outer.bottom(), a.bottom()), xdg_toplevel::State::TILED_BOTTOM),
        ] {
            if touches {
                states.push(state.0);
            }
        }
    }
}
