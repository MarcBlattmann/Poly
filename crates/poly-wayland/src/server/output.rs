//! `wl_output` (one output, with its scale for HiDPI) and `wl_seat`
//! (capabilities; the input stack fills it in later).

use super::{ClientId, Data, OutputConfig, Res, Server, error};
use crate::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface, wl_touch};
use crate::seat::Capabilities;
use crate::wire::ObjectId;

impl Server {
    /// Describe the output to a new `wl_output` and send `enter` for the
    /// client's surfaces already shown.
    pub(super) fn output_bound(&mut self, c: ClientId, id: ObjectId) {
        let o = self.output.clone();
        let client = self.client(c);
        send_output(client, id, &o);
        for s in client.ids_where(|d| matches!(d, Data::Surface(s) if s.window.is_some())) {
            client.send(s, wl_surface::Event::Enter { output: id });
        }
    }

    /// Change the output (a mode set, another scale): the scene follows, and
    /// every bound `wl_output` and surface hears of it.
    pub fn set_output(&mut self, output: OutputConfig) {
        self.scene.set_output(output.width, output.height, output.scale);
        self.output = output;
        let o = self.output.clone();
        for client in self.clients.values_mut() {
            for id in client.ids_where(|d| matches!(d, Data::Output)) {
                send_output(client, id, &o);
            }
            for s in client.ids_where(|d| matches!(d, Data::Surface(s) if s.window.is_some())) {
                client.send(s, wl_surface::Event::PreferredBufferScale { factor: o.scale as i32 });
            }
        }
        self.fractional_scale_changed();
    }

    pub fn output(&self) -> &OutputConfig {
        &self.output
    }

    pub(super) fn seat_bound(&mut self, c: ClientId, id: ObjectId) {
        let (name, caps) = (self.seat.name.clone(), self.seat.capabilities);
        let client = self.client(c);
        client.send(id, wl_seat::Event::Name { name });
        client.send(id, wl_seat::Event::Capabilities { capabilities: caps.0 });
    }

    pub(super) fn seat_request(&mut self, c: ClientId, seat: ObjectId, req: wl_seat::Request) -> Res {
        let (id, cap, interface) = match req {
            wl_seat::Request::GetPointer { id } => (id, Capabilities::POINTER, &wl_pointer::INTERFACE),
            wl_seat::Request::GetKeyboard { id } => (id, Capabilities::KEYBOARD, &wl_keyboard::INTERFACE),
            wl_seat::Request::GetTouch { id } => (id, Capabilities::TOUCH, &wl_touch::INTERFACE),
            wl_seat::Request::Release => return Ok(()),
        };
        if !self.seat.ever.contains(cap) {
            let code = wl_seat::Error::MISSING_CAPABILITY.0;
            return Err(error(seat, code, alloc::format!("the seat never had a {}", interface.name)));
        }
        let client = self.client(c);
        let version = client.version(seat);
        client.insert(id, interface, version, Data::InputDevice);
        if cap == Capabilities::KEYBOARD {
            self.keyboard_created(c, id);
        }
        Ok(())
    }
}

/// `geometry`, `mode`, then for newer objects `scale`, `name`,
/// `description` and `done`.
fn send_output(client: &mut super::Client, id: ObjectId, o: &OutputConfig) {
    client.send(
        id,
        wl_output::Event::Geometry {
            x: 0,
            y: 0,
            physical_width: o.physical_mm.0,
            physical_height: o.physical_mm.1,
            subpixel: wl_output::Subpixel::UNKNOWN.0 as i32,
            make: o.make.clone(),
            model: o.model.clone(),
            transform: wl_output::Transform::NORMAL.0 as i32,
        },
    );
    client.send(
        id,
        wl_output::Event::Mode {
            flags: wl_output::Mode::CURRENT.0 | wl_output::Mode::PREFERRED.0,
            width: o.width as i32,
            height: o.height as i32,
            refresh: o.refresh_mhz as i32,
        },
    );
    client.send(id, wl_output::Event::Scale { factor: o.scale as i32 });
    client.send(id, wl_output::Event::Name { name: o.name.clone() });
    client.send(id, wl_output::Event::Description { description: o.description.clone() });
    client.send(id, wl_output::Event::Done);
}
