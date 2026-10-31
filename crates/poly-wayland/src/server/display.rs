//! `wl_display`, `wl_registry` and `wl_callback`: the registry of globals
//! and binding them.

use super::{ClientId, Data, Res, Server, error};
use crate::protocol::{
    wl_compositor, wl_display, wl_output, wl_registry, wl_seat, wl_shm, wp_presentation, xdg_wm_base,
    zxdg_decoration_manager_v1,
};
use crate::wire::ObjectId;
use alloc::format;

impl Server {
    pub(super) fn display_request(&mut self, c: ClientId, req: wl_display::Request) -> Res {
        match req {
            wl_display::Request::Sync { callback } => {
                let serial = self.next_serial();
                let client = self.client(c);
                client.insert(callback, &crate::protocol::wl_callback::INTERFACE, 1, Data::Callback);
                client.callback_done(callback, serial);
            }
            wl_display::Request::GetRegistry { registry } => {
                let globals = self.globals.clone();
                let client = self.client(c);
                client.insert(registry, &wl_registry::INTERFACE, 1, Data::Registry);
                for g in globals {
                    client.send(
                        registry,
                        wl_registry::Event::Global {
                            name: g.name,
                            interface: g.interface.name.into(),
                            version: g.version,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn registry_request(&mut self, c: ClientId, req: wl_registry::Request) -> Res {
        let wl_registry::Request::Bind { name, id: new } = req;
        let invalid = |what: alloc::string::String| error(ObjectId::DISPLAY, wl_display::Error::INVALID_OBJECT.0, what);
        let Some(g) = self.globals.iter().find(|g| g.name == name).copied() else {
            return Err(invalid(format!("no global {name}")));
        };
        if new.interface != g.interface.name {
            return Err(invalid(format!("global {name} is a {}, not a {}", g.interface.name, new.interface)));
        }
        if new.version == 0 || new.version > g.version {
            return Err(invalid(format!("{} version {} (have {})", new.interface, new.version, g.version)));
        }
        let (id, version) = (new.id, new.version);
        let data = match g.interface.name {
            wl_compositor::NAME => Data::Compositor,
            wl_shm::NAME => Data::Shm,
            wl_output::NAME => Data::Output,
            wl_seat::NAME => Data::Seat,
            xdg_wm_base::NAME => Data::WmBase,
            zxdg_decoration_manager_v1::NAME => Data::DecorationManager,
            wp_presentation::NAME => Data::Presentation,
            _ => return Err(invalid(format!("{} cannot be bound", g.interface.name))),
        };
        self.client(c).insert(id, g.interface, version, data);
        match g.interface.name {
            wl_shm::NAME => self.shm_bound(c, id),
            wl_output::NAME => self.output_bound(c, id),
            wl_seat::NAME => self.seat_bound(c, id),
            wp_presentation::NAME => self.presentation_bound(c, id),
            _ => {}
        }
        Ok(())
    }
}
