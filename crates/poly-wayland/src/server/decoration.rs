//! `zxdg_decoration_manager_v1` and KDE's server decoration manager: who
//! draws a toplevel's frame.
//!
//! Poly draws one neutral frame around every app (ADR 0106) unless the
//! client draws its own (Chromium's tab strip, a GTK header bar): then
//! the window has one title bar, not two (ADR 0144, 0183).
//! [`crate::frame_policy`] decides from what the client said (xdg-decoration
//! `set_mode`, KDE's server decoration object, shadow margins); this file
//! collects it from the two protocols and tells the client the answer.

use super::{ClientId, Data, Res, Server, error};
use crate::frame_policy::{self, Ask, Frame, Kde, KdeMode, Signals};
use crate::protocol::{
    org_kde_kwin_server_decoration, org_kde_kwin_server_decoration_manager, zxdg_decoration_manager_v1,
    zxdg_toplevel_decoration_v1,
};
use crate::wire::ObjectId;
use alloc::format;

impl Server {
    pub(super) fn decoration_manager_request(
        &mut self,
        c: ClientId,
        manager: ObjectId,
        req: zxdg_decoration_manager_v1::Request,
    ) -> Res {
        let zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, toplevel } = req else { return Ok(()) };
        let client = self.client(c);
        let version = client.version(manager);
        client.insert(id, &zxdg_toplevel_decoration_v1::INTERFACE, version, Data::Decoration(toplevel));
        let Some(Data::Toplevel(t)) = client.data(toplevel) else { return Ok(()) };
        let (surface, existing) = (t.surface, t.decoration);
        if existing.is_some() {
            let code = zxdg_toplevel_decoration_v1::Error::ALREADY_CONSTRUCTED.0;
            return Err(error(id, code, format!("toplevel {toplevel} already has a decoration object")));
        }
        // Version 1 forbids a decoration for a toplevel that has a buffer.
        let has_buffer = matches!(client.data(surface), Some(Data::Surface(s)) if s.buffer_size.is_some() || matches!(s.pending.buffer, Some(Some(_))));
        if version == 1 && has_buffer {
            let code = zxdg_toplevel_decoration_v1::Error::UNCONFIGURED_BUFFER.0;
            return Err(error(id, code, "the toplevel already has a buffer"));
        }
        self.decoration_due(c, toplevel, Some(id));
        Ok(())
    }

    pub(super) fn decoration_request(
        &mut self,
        c: ClientId,
        id: ObjectId,
        req: zxdg_toplevel_decoration_v1::Request,
    ) -> Res {
        let Some(Data::Decoration(toplevel)) = self.client(c).data(id) else { return Ok(()) };
        let toplevel = *toplevel;
        match req {
            zxdg_toplevel_decoration_v1::Request::Destroy => {
                if let Some(Data::Toplevel(t)) = self.client(c).data_mut(toplevel) {
                    t.decoration = None;
                    t.decoration_due = false;
                    t.ask = Ask::Silent;
                }
                self.frame_changed(c, toplevel);
            }
            zxdg_toplevel_decoration_v1::Request::SetMode { mode } => {
                let known =
                    [zxdg_toplevel_decoration_v1::Mode::CLIENT_SIDE, zxdg_toplevel_decoration_v1::Mode::SERVER_SIDE];
                if !known.contains(&zxdg_toplevel_decoration_v1::Mode(mode)) {
                    let code = zxdg_toplevel_decoration_v1::Error::INVALID_MODE.0;
                    return Err(error(id, code, format!("decoration mode {mode}")));
                }
                let client_side =
                    zxdg_toplevel_decoration_v1::Mode(mode) == zxdg_toplevel_decoration_v1::Mode::CLIENT_SIDE;
                if let Some(Data::Toplevel(t)) = self.client(c).data_mut(toplevel) {
                    t.ask = if client_side { Ask::Client } else { Ask::Server };
                }
                self.decoration_due(c, toplevel, Some(id));
            }
            zxdg_toplevel_decoration_v1::Request::UnsetMode => {
                if let Some(Data::Toplevel(t)) = self.client(c).data_mut(toplevel) {
                    t.ask = Ask::Silent;
                }
                self.decoration_due(c, toplevel, Some(id));
            }
        }
        Ok(())
    }

    /// A `configure` with the decoration mode is due: now, with
    /// a configure sequence, if the toplevel was configured before;
    /// otherwise with its first.
    fn decoration_due(&mut self, c: ClientId, toplevel: ObjectId, decoration: Option<ObjectId>) {
        let Some(Data::Toplevel(t)) = self.client(c).data_mut(toplevel) else { return };
        t.decoration = decoration;
        t.decoration_due = true;
        let xid = t.xdg;
        let configured = matches!(self.client(c).data(xid), Some(Data::XdgSurface(x)) if x.initial_done);
        if configured {
            self.configure_toplevel(c, toplevel);
        }
    }

    pub(super) fn kde_decoration_manager_bound(&mut self, c: ClientId, manager: ObjectId) {
        self.client(c).kde_decoration = true;
        // The mode for a client that does not ask: Poly's frame (ADR 0183).
        // Sent before anything else of the manager, as the protocol says.
        let mode = frame_policy::KDE_DEFAULT.wire();
        self.client(c).send(manager, org_kde_kwin_server_decoration_manager::Event::DefaultMode { mode });
        // A toplevel that already exists made no object: the client's own.
        for tl in self.toplevels_of(c) {
            self.frame_changed(c, tl);
        }
    }

    pub(super) fn kde_decoration_manager_request(
        &mut self,
        c: ClientId,
        manager: ObjectId,
        req: org_kde_kwin_server_decoration_manager::Request,
    ) -> Res {
        let org_kde_kwin_server_decoration_manager::Request::Create { id, surface } = req;
        let client = self.client(c);
        let version = client.version(manager);
        client.insert(id, &org_kde_kwin_server_decoration::INTERFACE, version, Data::KdeDecoration(surface));
        if let Some(s) = self.surface(c, surface) {
            s.kde_decoration = Some((id, None));
        }
        self.kde_announce(c, id, surface);
        for tl in self.toplevels_on(c, surface) {
            self.frame_changed(c, tl);
        }
        Ok(())
    }

    pub(super) fn kde_decoration_request(
        &mut self,
        c: ClientId,
        id: ObjectId,
        req: org_kde_kwin_server_decoration::Request,
    ) -> Res {
        let Some(Data::KdeDecoration(surface)) = self.client(c).data(id) else { return Ok(()) };
        let surface = *surface;
        match req {
            org_kde_kwin_server_decoration::Request::Release => {
                if let Some(s) = self.surface(c, surface)
                    && s.kde_decoration.is_some_and(|(d, _)| d == id)
                {
                    s.kde_decoration = None;
                }
            }
            org_kde_kwin_server_decoration::Request::RequestMode { mode } => {
                // An unknown mode changes nothing; so does asking for the
                // mode that is in use (no event: GTK 3 does it right after
                // creating the object).
                if let Some(mode) = KdeMode::from_wire(mode)
                    && let Some(s) = self.surface(c, surface)
                    && let Some((d, before)) = s.kde_decoration
                    && d == id
                {
                    s.kde_decoration = Some((id, Some(mode)));
                    if kde_mode_in_use(before) != kde_mode_in_use(Some(mode)) {
                        self.kde_announce(c, id, surface);
                    }
                }
            }
        }
        for tl in self.toplevels_on(c, surface) {
            self.frame_changed(c, tl);
        }
        Ok(())
    }

    /// The `mode` event of a decoration object: what the compositor uses.
    fn kde_announce(&mut self, c: ClientId, id: ObjectId, surface: ObjectId) {
        let requested = self.surface(c, surface).and_then(|s| s.kde_decoration).and_then(|(_, r)| r);
        let mode = kde_mode_in_use(requested);
        self.client(c).send(id, org_kde_kwin_server_decoration::Event::Mode { mode: mode.wire() });
    }

    /// The toplevels of client `c`.
    fn toplevels_of(&mut self, c: ClientId) -> alloc::vec::Vec<ObjectId> {
        self.client(c).ids_where(|d| matches!(d, Data::Toplevel(_)))
    }

    /// The toplevels whose surface is `surface`.
    fn toplevels_on(&mut self, c: ClientId, surface: ObjectId) -> alloc::vec::Vec<ObjectId> {
        self.client(c).ids_where(|d| matches!(d, Data::Toplevel(t) if t.surface == surface))
    }

    /// Everything the policy goes by, for toplevel `tl`.
    fn frame_signals(&mut self, c: ClientId, tl: ObjectId) -> Option<Signals> {
        let Some(Data::Toplevel(t)) = self.client(c).data(tl) else { return None };
        let (ask, shadow, surface) = (t.ask, t.shadow, t.surface);
        let bound = self.client(c).kde_decoration;
        let (object, shown) = match self.surface(c, surface) {
            Some(s) => (s.kde_decoration, s.buffer_size.is_some()),
            None => (None, false),
        };
        // GTK 3 makes its object after its first commit: a surface that
        // shows nothing yet has not said it makes none.
        let kde = match (bound, object) {
            (_, Some((_, requested))) => Kde::Object { requested },
            (true, None) if shown => Kde::NoObject,
            _ => Kde::NotBound,
        };
        Some(Signals { xdg: ask, kde, shadow })
    }

    /// Decide who draws `tl`'s frame again, from what is known now; whether
    /// the answer changed.
    pub(super) fn refresh_frame(&mut self, c: ClientId, tl: ObjectId) -> bool {
        let Some(signals) = self.frame_signals(c, tl) else { return false };
        let client_side = frame_policy::decide(&signals).0 == Frame::Client;
        let Some(Data::Toplevel(t)) = self.client(c).data_mut(tl) else { return false };
        core::mem::replace(&mut t.client_side, client_side) != client_side
    }

    /// Something that decides who draws the frame changed: if the answer
    /// did, the window is framed or not at its next commit, and a toplevel
    /// that was configured is configured again when that tells the client
    /// something: it has a decoration object to tell the mode, or its size
    /// follows the frame (maximized, fullscreen, tiled). A floating window
    /// of a client that only uses KDE's protocol is left alone: GTK 3
    /// changes the decorations it draws on its own when it hears the mode,
    /// and a configure of another size on top of that crashes it.
    pub(super) fn frame_changed(&mut self, c: ClientId, tl: ObjectId) {
        if !self.refresh_frame(c, tl) {
            return;
        }
        let Some(Data::Toplevel(t)) = self.client(c).data_mut(tl) else { return };
        t.decoration_due = t.decoration.is_some();
        let sized_by_frame = t.want_maximized
            || t.want_fullscreen
            || t.want_tile.is_some()
            || t.maximized
            || t.fullscreen
            || t.tile.is_some();
        let tells = t.decoration.is_some() || sized_by_frame;
        let xid = t.xdg;
        let configured = matches!(self.client(c).data(xid), Some(Data::XdgSurface(x)) if x.initial_done);
        if configured && tells {
            self.configure_toplevel(c, tl);
        }
    }
}

/// The mode the compositor uses for a surface whose client asked for
/// `requested` (the manager's default if it did not ask).
fn kde_mode_in_use(requested: Option<KdeMode>) -> KdeMode {
    requested.unwrap_or(frame_policy::KDE_DEFAULT)
}
