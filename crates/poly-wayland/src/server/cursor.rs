//! The pointer's look: `wl_pointer.set_cursor` surfaces and
//! `wp_cursor_shape_device_v1` shapes (cursor-shape-v1.xml).
//!
//! - The client with pointer focus may give the pointer a **cursor
//!   surface** (role `cursor`) with a hot spot: its content is drawn at the
//!   pointer instead of Poly's arrow, and replaced with every commit of
//!   that surface; an offset in a commit moves the hot spot back by as
//!   much (wayland.xml). No surface hides the cursor.
//! - A **shape** asks for a named cursor; Poly has only its arrow, so every
//!   shape shows it (the shape is remembered for when there are more).
//! - When the pointer focus moves to another client (or the desktop) the
//!   arrow is back; clients set their cursor on `enter`.

use super::surface::Role;
use super::{ClientId, Data, Res, Server, error};
use crate::compose::CursorLook;
use crate::protocol::{wl_pointer, wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1};
use crate::wire::ObjectId;
use alloc::format;

/// What the client with pointer focus asked the pointer to look like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CursorRequest {
    Surface { client: ClientId, surface: ObjectId, hx: i32, hy: i32 },
    Hidden,
    Shape(u32),
}

/// The highest `wp_cursor_shape_device_v1.shape` of each version.
const LAST_SHAPE: [u32; 2] = [0x22, 0x24];

impl Server {
    pub(super) fn pointer_request(&mut self, c: ClientId, pointer: ObjectId, req: wl_pointer::Request) -> Res {
        let wl_pointer::Request::SetCursor { surface, hotspot_x, hotspot_y, .. } = req else { return Ok(()) };
        if let Some(s) = surface {
            let Some(cs) = self.surface(c, s) else { return Ok(()) };
            if !matches!(cs.role, Role::None | Role::Cursor) {
                return Err(error(pointer, wl_pointer::Error::ROLE.0, format!("surface {s} has another role")));
            }
            cs.role = Role::Cursor;
        }
        // Only the client the pointer is over decides.
        if self.seat.input.pointer_focus.is_none_or(|f| f.client != c) {
            return Ok(());
        }
        self.seat.cursor = Some(match surface {
            Some(s) => CursorRequest::Surface { client: c, surface: s, hx: hotspot_x, hy: hotspot_y },
            None => CursorRequest::Hidden,
        });
        self.update_cursor_look();
        Ok(())
    }

    /// A cursor surface was committed: its hot spot moves by the offset,
    /// the picture follows if it is the pointer's.
    pub(super) fn cursor_committed(&mut self, c: ClientId, surface: ObjectId) {
        let (dx, dy) = self.surface_ref(c, surface).map_or((0, 0), |s| s.applied_offset);
        if let Some(CursorRequest::Surface { client, surface: s, hx, hy }) = &mut self.seat.cursor
            && (*client, *s) == (c, surface)
        {
            *hx -= dx;
            *hy -= dy;
            self.update_cursor_look();
        }
    }

    /// Show what the pointer should look like.
    pub(super) fn update_cursor_look(&mut self) {
        let look = match self.seat.cursor {
            Some(CursorRequest::Surface { client, surface, hx, hy }) => match self.surface_ref(client, surface) {
                Some(s) if s.buffer_size.is_some() => CursorLook::Image { content: s.stash.clone(), hx, hy },
                _ => CursorLook::Hidden,
            },
            Some(CursorRequest::Hidden) => CursorLook::Hidden,
            Some(CursorRequest::Shape(_)) | None => CursorLook::Arrow,
        };
        self.scene.set_cursor_look(look);
    }

    /// The pointer focus moved to another client: the arrow again.
    pub(super) fn reset_cursor(&mut self) {
        if self.seat.cursor.take().is_some() {
            self.update_cursor_look();
        }
    }

    pub(super) fn cursor_shape_manager_request(
        &mut self,
        c: ClientId,
        manager: ObjectId,
        req: wp_cursor_shape_manager_v1::Request,
    ) -> Res {
        let version = self.client(c).version(manager);
        match req {
            wp_cursor_shape_manager_v1::Request::GetPointer { cursor_shape_device, .. } => {
                let iface = &wp_cursor_shape_device_v1::INTERFACE;
                self.client(c).insert(cursor_shape_device, iface, version, Data::CursorShapeDevice);
            }
            // Poly has no tablets: there is no tablet tool to name.
            wp_cursor_shape_manager_v1::Request::GetTabletToolV2 { cursor_shape_device, .. } => {
                let iface = &wp_cursor_shape_device_v1::INTERFACE;
                self.client(c).insert(cursor_shape_device, iface, version, Data::CursorShapeDevice);
            }
            wp_cursor_shape_manager_v1::Request::Destroy => {}
        }
        Ok(())
    }

    pub(super) fn cursor_shape_request(
        &mut self,
        c: ClientId,
        device: ObjectId,
        req: wp_cursor_shape_device_v1::Request,
    ) -> Res {
        let wp_cursor_shape_device_v1::Request::SetShape { shape, .. } = req else { return Ok(()) };
        let version = self.client(c).version(device);
        let last = LAST_SHAPE[(version.clamp(1, 2) - 1) as usize];
        if shape == 0 || shape > last {
            let code = wp_cursor_shape_device_v1::Error::INVALID_SHAPE.0;
            return Err(error(device, code, format!("cursor shape {shape}")));
        }
        if self.seat.input.pointer_focus.is_some_and(|f| f.client == c) {
            self.seat.cursor = Some(CursorRequest::Shape(shape));
            self.update_cursor_look();
        }
        Ok(())
    }

    /// The pointer's look as asked for (for tests and the compositor).
    #[cfg(test)]
    pub(crate) fn cursor_request(&self) -> Option<CursorRequest> {
        self.seat.cursor
    }
}
