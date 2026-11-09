//! `zxdg_decoration_manager_v1`: every toplevel is decorated by the server.
//!
//! Poly draws one neutral frame around every app (ADR 0106): toplevels of
//! clients that never ask, or ask for `server_side` or `unset_mode`, are
//! framed. A client that explicitly asks for `client_side` draws its own
//! frame (Chromium's tab strip with its window buttons), and Poly answers
//! `client_side` and draws none, so the window has one title bar, not two
//! (ADR 0144).

use super::{ClientId, Data, Res, Server, error};
use crate::protocol::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1};
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
                    t.client_side = false;
                }
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
                    t.client_side = client_side;
                }
                self.decoration_due(c, toplevel, Some(id));
            }
            zxdg_toplevel_decoration_v1::Request::UnsetMode => {
                if let Some(Data::Toplevel(t)) = self.client(c).data_mut(toplevel) {
                    t.client_side = false;
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
}
