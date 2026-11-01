//! `wp_viewporter` and `wp_viewport` (viewporter.xml): a surface shows a
//! source rectangle of its buffer (after the buffer transform and scale)
//! scaled to a destination size. Both are double-buffered surface state;
//! the checks that need the buffer (`out_of_buffer`, `bad_size`) are made
//! when the state is applied (`surface.rs`), the renderer samples through
//! the view ([`crate::compose::View`]).

use super::{ClientId, Data, Res, Server, error};
use crate::protocol::{wp_viewport, wp_viewporter};
use crate::wire::{Fixed, ObjectId};
use alloc::format;

impl Server {
    pub(super) fn viewporter_request(&mut self, c: ClientId, vp: ObjectId, req: wp_viewporter::Request) -> Res {
        let wp_viewporter::Request::GetViewport { id, surface } = req else { return Ok(()) };
        let Some(s) = self.surface(c, surface) else { return Ok(()) };
        if s.viewport.is_some() {
            let code = wp_viewporter::Error::VIEWPORT_EXISTS.0;
            return Err(error(vp, code, format!("surface {surface} already has a viewport")));
        }
        s.viewport = Some(id);
        self.client(c).insert(id, &wp_viewport::INTERFACE, 1, Data::Viewport(Some(surface)));
        Ok(())
    }

    pub(super) fn viewport_request(&mut self, c: ClientId, id: ObjectId, req: wp_viewport::Request) -> Res {
        let Some(Data::Viewport(surface)) = self.client(c).data(id) else { return Ok(()) };
        let surface = *surface;
        if let wp_viewport::Request::Destroy = req {
            // Unset with the next commit.
            if let Some(s) = surface.and_then(|s| self.surface(c, s)) {
                s.viewport = None;
                s.pending.source = Some(None);
                s.pending.destination = Some(None);
            }
            return Ok(());
        }
        let Some(s) = surface.and_then(|s| self.surface(c, s)) else {
            return Err(error(id, wp_viewport::Error::NO_SURFACE.0, "the surface is gone"));
        };
        let bad = |what: alloc::string::String| error(id, wp_viewport::Error::BAD_VALUE.0, what);
        match req {
            wp_viewport::Request::SetSource { x, y, width, height } => {
                let unset = Fixed::from_int(-1);
                if [x, y, width, height].iter().all(|v| *v == unset) {
                    s.pending.source = Some(None);
                } else if x.0 < 0 || y.0 < 0 || width.0 <= 0 || height.0 <= 0 {
                    return Err(bad(format!("source {}x{} at {},{} (1/256)", width.0, height.0, x.0, y.0)));
                } else {
                    s.pending.source = Some(Some((x.0, y.0, width.0, height.0)));
                }
            }
            wp_viewport::Request::SetDestination { width, height } => {
                if (width, height) == (-1, -1) {
                    s.pending.destination = Some(None);
                } else if width <= 0 || height <= 0 {
                    return Err(bad(format!("destination {width}x{height}")));
                } else {
                    s.pending.destination = Some(Some((width, height)));
                }
            }
            wp_viewport::Request::Destroy => {}
        }
        Ok(())
    }

    /// A surface went away: its viewport has none any more, its
    /// fractional scale object and its role objects (`wl_subsurface`,
    /// `xwayland_surface_v1`) neither: they are inert, and the surface's ID,
    /// which the client may use again once `delete_id` came, no longer
    /// leads anywhere from them.
    pub(super) fn viewport_surface_gone(&mut self, c: ClientId, surface: ObjectId) {
        let client = self.client(c);
        for o in client.objects.values_mut() {
            match &mut o.data {
                Data::Viewport(s) | Data::Fractional(s) | Data::Subsurface(s) | Data::XwaylandSurface(s)
                    if *s == Some(surface) =>
                {
                    *s = None
                }
                _ => {}
            }
        }
    }
}
