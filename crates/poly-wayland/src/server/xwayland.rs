//! Xwayland's surfaces (ADR 0118): `xwayland_shell_v1`, and X windows
//! shown as scene windows.
//!
//! Xwayland in rootless mode is a Wayland client that makes a `wl_surface`
//! for every mapped X top-level window. Which X window a surface shows is
//! told twice, on two connections that do not wait for each other:
//!
//! - on the Wayland side, `xwayland_shell_v1.get_xwayland_surface` gives the
//!   surface the `xwayland_surface` role and `xwayland_surface_v1.set_serial`
//!   a serial, double-buffered (it counts once the surface is committed);
//! - on the X side, Xwayland sends the window manager a `WL_SURFACE_SERIAL`
//!   client message on the window with the same serial (or, without the
//!   global, `WL_SURFACE_ID` with the surface's object ID).
//!
//! The window manager (`poly-x11`'s `xwm`, in the compositor) passes the
//! X side here ([`Server::x_associate`]) with what it knows of the window
//! ([`Server::x_update`]: geometry, mapped, override-redirect, frame,
//! title, transient-for); whichever side comes second completes the
//! association. A window is shown while it is associated, mapped and its
//! surface has a buffer: a scene window at the X window's position (root
//! coordinates are the output's logical ones), framed like any toplevel
//! when the window manager says so, override-redirect windows (menus,
//! tooltips) on top and unframed, transients above their parent.
//!
//! What the user does to X windows comes back as [`XRequest`]s for the
//! window manager ([`Server::take_x_requests`]): a click activates a
//! managed window (it gets the keyboard focus, as an active toplevel
//! would, and the window manager gives it X's input focus), the close
//! button asks it to close, a drag of the frame moves it. While a window is
//! being dragged its X position updates are not applied, so late
//! `ConfigureNotify`s do not pull it back.
//!
//! `xwayland_shell_v1` is offered only to the client the compositor
//! started as Xwayland ([`Server::set_xwayland_client`]), as the protocol
//! asks: other clients do not see it in the registry and cannot bind it.

use super::*;
use crate::protocol::{xwayland_shell_v1, xwayland_surface_v1};
use alloc::collections::BTreeSet;
use surface::Role;

/// How the window manager names an X window's surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRef {
    /// The serial of `xwayland_surface_v1.set_serial` (`WL_SURFACE_SERIAL`).
    Serial(u64),
    /// The surface's object ID on Xwayland's connection (`WL_SURFACE_ID`).
    Id(u32),
}

/// What the window manager knows of an X top-level window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XWindowState {
    /// The window's position and size in root (output logical) coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub mapped: bool,
    /// Managed by the window manager (a map request it granted).
    pub managed: bool,
    pub override_redirect: bool,
    /// Gets Poly's frame.
    pub decorated: bool,
    pub title: String,
    pub transient_for: Option<u32>,
}

/// What the user did to an X window, for the window manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XRequest {
    /// Give the window the keyboard (X input focus, `_NET_ACTIVE_WINDOW`).
    Activate(u32),
    /// A Wayland window took the keyboard: no X window has it.
    Deactivate,
    /// The close button.
    Close(u32),
    /// The window's content was moved to (`x`, `y`).
    Move { window: u32, x: i32, y: i32 },
}

/// One X window.
#[derive(Debug, Default)]
struct XWin {
    state: XWindowState,
    by: Option<SurfaceRef>,
    surface: Option<ObjectId>,
    window: Option<WindowId>,
}

/// The server's Xwayland state.
#[derive(Debug, Default)]
pub(crate) struct XwaylandState {
    client: Option<ClientId>,
    windows: BTreeMap<u32, XWin>,
    /// Serials committed by surfaces, and the surface each names.
    serials: BTreeMap<u64, ObjectId>,
    /// Serials set but not committed yet, by surface.
    pending_serials: BTreeMap<u32, u64>,
    /// Surfaces that committed a serial (once each).
    serialised: BTreeSet<u32>,
    /// The X window each associated surface shows.
    by_surface: BTreeMap<u32, u32>,
    /// The last buffer each Xwayland surface committed.
    buffers: BTreeMap<u32, ObjectId>,
    requests: Vec<XRequest>,
}

impl Server {
    /// The client that is Xwayland (it alone sees `xwayland_shell_v1`), or
    /// none. Changing it forgets every X window.
    pub fn set_xwayland_client(&mut self, c: Option<ClientId>) {
        if self.xwayland.client != c {
            self.xwayland_forget();
            self.xwayland.client = c;
        }
    }

    pub fn xwayland_client(&self) -> Option<ClientId> {
        self.xwayland.client
    }

    /// What the user did to X windows since last time (taken).
    pub fn take_x_requests(&mut self) -> Vec<XRequest> {
        core::mem::take(&mut self.xwayland.requests)
    }

    /// The window manager learnt X window `window`'s surface.
    pub fn x_associate(&mut self, window: u32, by: SurfaceRef) {
        let w = self.xwayland.windows.entry(window).or_default();
        if w.by == Some(by) && w.surface.is_some() {
            return;
        }
        // A new surface for the window (it was mapped again): let the old
        // one go.
        if w.by.is_some() && w.by != Some(by) {
            self.x_detach(window);
        }
        if let Some(w) = self.xwayland.windows.get_mut(&window) {
            w.by = Some(by);
        }
        self.x_sync(window);
        self.sync_keyboard_focus();
    }

    /// The window manager's view of X window `window` changed.
    pub fn x_update(&mut self, window: u32, state: &XWindowState) {
        let w = self.xwayland.windows.entry(window).or_default();
        if &w.state == state {
            return;
        }
        w.state = state.clone();
        self.x_sync(window);
        self.sync_keyboard_focus();
    }

    /// X window `window` was destroyed.
    pub fn x_destroy(&mut self, window: u32) {
        self.x_detach(window);
        self.xwayland.windows.remove(&window);
        self.sync_keyboard_focus();
    }

    /// A client asked for X window `window` to be activated: focus it as a
    /// click would.
    pub fn x_activate(&mut self, window: u32) {
        self.x_focus(window);
        self.sync_keyboard_focus();
    }

    /// A client asked to move X window `window` with the pointer
    /// (`_NET_WM_MOVERESIZE`): a move while a button is held.
    pub fn x_start_move(&mut self, window: u32) {
        if let Some(win) = self.xwayland.windows.get(&window).and_then(|w| w.window) {
            self.start_move(win);
        }
    }

    /// The scene window showing X window `window`, if it is shown.
    pub fn x_scene_window(&self, window: u32) -> Option<WindowId> {
        self.xwayland.windows.get(&window).and_then(|w| w.window)
    }

    /// The X window a scene window shows.
    pub(super) fn x_window_at(&self, w: WindowId) -> Option<u32> {
        self.xwayland.windows.iter().find(|(_, x)| x.window == Some(w)).map(|(id, _)| *id)
    }

    /// Whether `surface` of client `c` shows an X window.
    pub(super) fn is_x_surface(&self, c: ClientId, surface: ObjectId) -> bool {
        self.xwayland.client == Some(c) && self.xwayland.by_surface.contains_key(&surface.0)
    }

    /// Forget every X window (Xwayland is gone or replaced).
    fn xwayland_forget(&mut self) {
        let ids: Vec<u32> = self.xwayland.windows.keys().copied().collect();
        for id in ids {
            self.x_detach(id);
        }
        let requests = core::mem::take(&mut self.xwayland.requests);
        self.xwayland = XwaylandState { requests, ..XwaylandState::default() };
    }

    /// Take X window `window` off the output and from its surface.
    fn x_detach(&mut self, window: u32) {
        let Some(w) = self.xwayland.windows.get_mut(&window) else { return };
        let (surface, win) = (w.surface.take(), w.window.take());
        w.by = None;
        let Some(c) = self.xwayland.client else { return };
        if let Some(s) = surface {
            self.xwayland.by_surface.remove(&s.0);
            if self.clients.contains_key(&c)
                && let Some(sf) = self.surface(c, s)
            {
                sf.window = None;
            }
            if self.active == Some((c, s)) {
                self.active = None;
            }
        }
        if let Some(win) = win {
            self.scene.remove_window(win);
        }
    }

    /// The surface `by` names, if it is there.
    fn x_resolve(&mut self, c: ClientId, by: SurfaceRef) -> Option<ObjectId> {
        let id = match by {
            SurfaceRef::Serial(s) => *self.xwayland.serials.get(&s)?,
            SurfaceRef::Id(id) => ObjectId(id),
        };
        let s = self.surface(c, id)?;
        match s.role {
            Role::Xwayland => {}
            // WL_SURFACE_ID names surfaces without the role object.
            Role::None if matches!(by, SurfaceRef::Id(_)) => s.role = Role::Xwayland,
            _ => return None,
        }
        // Not if another window has it already.
        match self.xwayland.by_surface.get(&id.0) {
            Some(_) => None,
            None => Some(id),
        }
    }

    /// Make the scene follow X window `window`: associate its surface when
    /// both sides are known, show it while it is mapped and has content,
    /// keep its position, frame and title.
    fn x_sync(&mut self, window: u32) {
        let Some(c) = self.xwayland.client.filter(|c| self.clients.contains_key(c)) else { return };
        let Some(w) = self.xwayland.windows.get(&window) else { return };
        let (by, mut surface, state) = (w.by, w.surface, w.state.clone());
        if surface.is_none()
            && let Some(by) = by
            && let Some(s) = self.x_resolve(c, by)
        {
            self.xwayland.by_surface.insert(s.0, window);
            if let Some(w) = self.xwayland.windows.get_mut(&window) {
                w.surface = Some(s);
            }
            surface = Some(s);
        }
        let Some(surface) = surface else { return };
        let has_content = self.surface(c, surface).is_some_and(|s| s.buffer_size.is_some());
        let show = state.mapped && has_content;
        let current = self.xwayland.windows.get(&window).and_then(|w| w.window);
        match (show, current) {
            (true, None) => self.x_show(c, window, surface, &state),
            (false, Some(win)) => {
                self.scene.remove_window(win);
                if let Some(s) = self.surface(c, surface) {
                    s.window = None;
                }
                if let Some(w) = self.xwayland.windows.get_mut(&window) {
                    w.window = None;
                }
                if self.active == Some((c, surface)) {
                    self.active = None;
                }
            }
            (true, Some(win)) => {
                let moving = matches!(self.seat.input.grab, Some(input::Grab::Move { window, .. }) if window == win);
                if !moving && self.scene.window(win).is_some_and(|w| w.position() != (state.x, state.y)) {
                    self.scene.set_position(win, state.x, state.y);
                }
                self.scene.set_title(win, &state.title);
                self.scene.set_decorated(win, state.decorated);
            }
            (false, None) => {}
        }
    }

    /// A new scene window for X window `window`, with the content of the
    /// surface's last buffer.
    fn x_show(&mut self, c: ClientId, window: u32, surface: ObjectId, state: &XWindowState) {
        let win = self.scene.add_window(owner_tag(c, surface), state.decorated);
        self.scene.set_position(win, state.x, state.y);
        self.scene.set_title(win, &state.title);
        if let Some(w) = self.xwayland.windows.get_mut(&window) {
            w.window = Some(win);
        }
        let scale = match self.surface(c, surface) {
            Some(s) => {
                s.window = Some(win);
                s.scale
            }
            None => 1,
        };
        if let Some(&b) = self.xwayland.buffers.get(&surface.0)
            && matches!(self.client(c).data(b), Some(Data::Buffer(_)))
        {
            self.copy_buffer(c, b, surface, Some(win), scale, &[]);
        }
        self.surface_entered(c, surface);
        let parent = state.transient_for.and_then(|p| self.x_scene_window(p));
        if state.override_redirect {
            // Menus and tooltips go over everything.
        } else if let Some(pw) = parent {
            self.scene.place_above(win, pw);
        }
        if state.managed && !state.override_redirect {
            self.x_focus(window);
        }
    }

    /// Give managed X window `window` the focus: raised, shown active, the
    /// keyboard (the window manager gives it X's focus).
    pub(super) fn x_focus(&mut self, window: u32) {
        let Some(c) = self.xwayland.client else { return };
        let Some(w) = self.xwayland.windows.get(&window) else { return };
        let (Some(surface), Some(win)) = (w.surface, w.window) else { return };
        if !w.state.managed || w.state.override_redirect {
            return;
        }
        let previous = self.active.replace((c, surface));
        self.scene.raise(win);
        if previous == Some((c, surface)) {
            return;
        }
        if let Some((pc, ptl)) = previous
            && self.clients.contains_key(&pc)
        {
            if let Some(pw) = self.toplevel_window(pc, ptl) {
                self.scene.set_active(pw, false);
                self.configure_toplevel(pc, ptl);
            } else if let Some(&px) = self.xwayland.by_surface.get(&ptl.0)
                && let Some(pw) = self.x_scene_window(px)
            {
                self.scene.set_active(pw, false);
            }
        }
        self.scene.set_active(win, true);
        self.xwayland.requests.push(XRequest::Activate(window));
    }

    /// An xdg toplevel took the focus from `(c, surface)`: if that was an
    /// X window, it is not active any more and X windows lose the keyboard.
    pub(super) fn x_lost_focus(&mut self, c: ClientId, surface: ObjectId) {
        if !self.is_x_surface(c, surface) {
            return;
        }
        if let Some(&x) = self.xwayland.by_surface.get(&surface.0)
            && let Some(w) = self.x_scene_window(x)
        {
            self.scene.set_active(w, false);
        }
        self.xwayland.requests.push(XRequest::Deactivate);
    }

    /// The user clicked X window shown in scene window `w`.
    pub(super) fn x_clicked(&mut self, w: WindowId) -> bool {
        match self.x_window_at(w) {
            Some(x) => {
                self.x_focus(x);
                true
            }
            None => false,
        }
    }

    /// The close button of scene window `w`, if it shows an X window.
    pub(super) fn x_close(&mut self, w: WindowId) -> bool {
        match self.x_window_at(w) {
            Some(x) => {
                self.xwayland.requests.push(XRequest::Close(x));
                true
            }
            None => false,
        }
    }

    /// Scene window `w` was moved to (`x`, `y`): tell X if it is an X
    /// window.
    pub(super) fn x_moved(&mut self, w: WindowId, x: i32, y: i32) {
        if let Some(window) = self.x_window_at(w) {
            if let Some(xw) = self.xwayland.windows.get_mut(&window) {
                (xw.state.x, xw.state.y) = (x, y);
            }
            self.xwayland.requests.push(XRequest::Move { window, x, y });
        }
    }

    /// After a commit of `surface`: its serial counts, its buffer is
    /// remembered, and the X window it shows (or that waits for it) follows.
    pub(super) fn xwayland_commit(&mut self, c: ClientId, surface: ObjectId, attached: surface::Attached) -> Res {
        if self.xwayland.client != Some(c) {
            return Ok(());
        }
        let x = &mut self.xwayland;
        match attached {
            surface::Attached::Buffer(b) => {
                x.buffers.insert(surface.0, b);
            }
            surface::Attached::Null => {
                x.buffers.remove(&surface.0);
            }
            surface::Attached::Nothing => {}
        }
        let mut waiting: Vec<u32> = Vec::new();
        if let Some(serial) = x.pending_serials.remove(&surface.0) {
            if !x.serialised.insert(surface.0) {
                let code = xwayland_surface_v1::Error::ALREADY_ASSOCIATED.0;
                let role = self.client(c).ids_where(|d| matches!(d, Data::XwaylandSurface(s) if *s == Some(surface)));
                let object = role.first().copied().unwrap_or(surface);
                return Err(error(object, code, "the surface committed a serial before"));
            }
            x.serials.insert(serial, surface);
            waiting
                .extend(x.windows.iter().filter(|(_, w)| w.by == Some(SurfaceRef::Serial(serial))).map(|(id, _)| *id));
        }
        waiting.extend(
            x.windows
                .iter()
                .filter(|(_, w)| w.surface.is_none() && w.by == Some(SurfaceRef::Id(surface.0)))
                .map(|(id, _)| *id),
        );
        if let Some(&window) = x.by_surface.get(&surface.0) {
            waiting.push(window);
        }
        for window in waiting {
            self.x_sync(window);
        }
        Ok(())
    }

    /// A surface of Xwayland's was destroyed.
    pub(super) fn xwayland_surface_destroyed(&mut self, c: ClientId, surface: ObjectId) {
        if self.xwayland.client != Some(c) {
            return;
        }
        let x = &mut self.xwayland;
        x.buffers.remove(&surface.0);
        x.pending_serials.remove(&surface.0);
        x.serialised.remove(&surface.0);
        x.serials.retain(|_, s| *s != surface);
        if let Some(window) = x.by_surface.remove(&surface.0)
            && let Some(w) = x.windows.get_mut(&window)
        {
            w.surface = None;
            if let Some(win) = w.window.take() {
                self.scene.remove_window(win);
            }
            if self.active == Some((c, surface)) {
                self.active = None;
            }
        }
    }

    pub(super) fn xwayland_shell_request(&mut self, c: ClientId, req: xwayland_shell_v1::Request) -> Res {
        match req {
            xwayland_shell_v1::Request::GetXwaylandSurface { id, surface } => {
                let Some(s) = self.surface(c, surface) else { return Ok(()) };
                if s.role != Role::None {
                    let code = xwayland_shell_v1::Error::ROLE.0;
                    return Err(error(surface, code, "the surface has another role"));
                }
                s.role = Role::Xwayland;
                self.client(c).insert(id, &xwayland_surface_v1::INTERFACE, 1, Data::XwaylandSurface(Some(surface)));
            }
            xwayland_shell_v1::Request::Destroy => {}
        }
        Ok(())
    }

    pub(super) fn xwayland_surface_request(
        &mut self,
        c: ClientId,
        id: ObjectId,
        req: xwayland_surface_v1::Request,
    ) -> Res {
        let Some(Data::XwaylandSurface(Some(surface))) = self.client(c).data(id) else { return Ok(()) };
        let surface = *surface;
        match req {
            xwayland_surface_v1::Request::SetSerial { serial_lo, serial_hi } => {
                let serial = u64::from(serial_lo) | u64::from(serial_hi) << 32;
                if serial == 0 {
                    let code = xwayland_surface_v1::Error::INVALID_SERIAL.0;
                    return Err(error(id, code, "serial 0"));
                }
                self.xwayland.pending_serials.insert(surface.0, serial);
            }
            xwayland_surface_v1::Request::Destroy => {}
        }
        Ok(())
    }
}
