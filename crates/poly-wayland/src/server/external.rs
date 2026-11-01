//! The extension point for window managers outside xdg-shell (ADR 0116):
//! XWayland's X11 window manager, which will live in the compositor, makes
//! the `wl_surface`s Xwayland shows its X11 windows on (associated through
//! `xwayland_shell_v1`) windows of the scene with this API instead of
//! `xdg_toplevel`.
//!
//! - [`Server::manage_surface`] gives a surface the role `external`: it is
//!   shown, framed by Poly or not (override-redirect menus and tooltips
//!   are not), as soon as it has content, and hidden by a null buffer.
//! - Its title and position are the window manager's to set
//!   ([`Server::set_managed_title`], [`Server::move_managed`]); a new one is
//!   placed like a toplevel unless a position was given.
//! - It takes part in focus like a toplevel: a click raises and activates
//!   it, and the keyboard follows; the frame's close button becomes
//!   [`WmEvent::Close`] ([`Server::take_wm_events`]), which the window
//!   manager turns into `WM_DELETE_WINDOW`.
//! - Subsurfaces, viewports and the rest of `wl_surface` work as for any
//!   surface.

use super::surface::Role;
use super::{ClientId, Server, owner_tag};
use crate::compose::WindowId;
use crate::wire::ObjectId;
use alloc::string::String;
use alloc::vec::Vec;

/// Something the window manager is to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WmEvent {
    /// The close button of a managed surface's frame was clicked.
    Close { client: ClientId, surface: ObjectId },
}

/// How a managed surface is shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Managed {
    pub decorated: bool,
    pub title: String,
    /// Where its content goes, logical; `None`: placed like a toplevel.
    pub position: Option<(i32, i32)>,
}

impl Server {
    /// Let an outside window manager manage client `c`'s `surface`: it gets
    /// the role `external` (false if it has another role) and is shown,
    /// with Poly's frame if `decorated`, once it has content.
    pub fn manage_surface(&mut self, c: ClientId, surface: ObjectId, decorated: bool) -> bool {
        let Some(s) = self.clients.get_mut(&c).and_then(|cl| match cl.data_mut(surface) {
            Some(super::Data::Surface(s)) => Some(s),
            _ => None,
        }) else {
            return false;
        };
        if !matches!(s.role, Role::None | Role::External) {
            return false;
        }
        s.role = Role::External;
        s.managed = Some(Managed { decorated, ..Managed::default() });
        self.managed_committed(c, surface);
        true
    }

    /// Stop managing a surface: its window goes.
    pub fn unmanage_surface(&mut self, c: ClientId, surface: ObjectId) {
        if !self.clients.contains_key(&c) {
            return;
        }
        self.hide_managed(c, surface);
        if let Some(s) = self.surface(c, surface) {
            s.managed = None;
        }
    }

    /// A managed surface's title (in its frame).
    pub fn set_managed_title(&mut self, c: ClientId, surface: ObjectId, title: &str) {
        let Some(s) = self.clients.get(&c).and_then(|_| self.surface_ref(c, surface)) else { return };
        let window = s.window;
        if let Some(m) = self.surface(c, surface).and_then(|s| s.managed.as_mut()) {
            m.title = title.into();
        }
        if let Some(w) = window {
            self.scene.set_title(w, title);
        }
    }

    /// Put a managed surface's content at (`x`, `y`), logical.
    pub fn move_managed(&mut self, c: ClientId, surface: ObjectId, x: i32, y: i32) {
        if !self.clients.contains_key(&c) {
            return;
        }
        let window = self.surface_ref(c, surface).and_then(|s| s.window);
        if let Some(m) = self.surface(c, surface).and_then(|s| s.managed.as_mut()) {
            m.position = Some((x, y));
        }
        if let Some(w) = window {
            self.scene.set_position(w, x, y);
        }
    }

    /// What the window manager is to act on since the last call.
    pub fn take_wm_events(&mut self) -> Vec<WmEvent> {
        core::mem::take(&mut self.wm_events)
    }

    /// A managed surface was committed (or just became managed): shown when
    /// it has content, hidden when it has none.
    pub(super) fn managed_committed(&mut self, c: ClientId, surface: ObjectId) {
        let Some(s) = self.surface_ref(c, surface) else { return };
        let Some(m) = s.managed.clone() else { return };
        let (has_content, window) = (s.buffer_size.is_some(), s.window);
        match (has_content, window) {
            (true, None) => {
                let Some(w) = self.window_from_stash(c, surface) else { return };
                self.scene.set_decorated(w, m.decorated);
                self.scene.set_title(w, &m.title);
                let (x, y) = m.position.unwrap_or_else(|| {
                    let b = self.scene.window(w).map(|w| w.bounds()).unwrap_or_default();
                    self.scene.place(0, 0, b.width, b.height, m.decorated)
                });
                self.scene.set_position(w, x, y);
                self.surface_entered(c, surface);
                self.activate(c, surface, false);
            }
            (false, Some(_)) => self.hide_managed(c, surface),
            _ => {}
        }
    }

    fn hide_managed(&mut self, c: ClientId, surface: ObjectId) {
        self.window_to_stash(c, surface);
        if self.active == Some((c, surface)) {
            self.active = None;
        }
    }

    /// The window of a managed surface, while it is shown.
    pub fn managed_window(&self, c: ClientId, surface: ObjectId) -> Option<WindowId> {
        let s = self.surface_ref(c, surface)?;
        s.managed
            .as_ref()
            .and(s.window)
            .filter(|w| self.scene.window(*w).is_some_and(|w| w.owner() == owner_tag(c, surface)))
    }
}
