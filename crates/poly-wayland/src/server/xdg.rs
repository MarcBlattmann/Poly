//! xdg-shell: `xdg_wm_base`, `xdg_positioner`, `xdg_surface`,
//! `xdg_toplevel` and `xdg_popup`. Toplevels become framed windows,
//! popups unframed ones above their parent.
//!
//! The configure sequence (xdg-shell.xml): after the role object is made
//! and the first commit without a buffer, the server sends
//! `wl_surface.preferred_buffer_scale`, the role's events and
//! `xdg_surface.configure` with a serial; the client acks it and may then
//! attach a buffer, whose commit maps the window. A null buffer unmaps it
//! and the sequence starts again.

use super::surface::{Attached, Role};
use super::{ClientId, Data, Res, Server, error, owner_of, owner_tag};
use crate::compose::{BORDER, Rect, TITLE_HEIGHT, WindowId};
use crate::protocol::{
    wl_surface, xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base, zxdg_toplevel_decoration_v1,
};
use crate::wire::ObjectId;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug)]
pub(crate) struct XdgSurface {
    pub wm_base: ObjectId,
    pub surface: ObjectId,
    /// The `xdg_toplevel` or `xdg_popup`.
    pub role: Option<ObjectId>,
    /// Serials of configure events not acked yet, oldest first.
    pub sent: Vec<u32>,
    /// A configure was acked since the role's initial commit: buffers may
    /// be attached.
    pub acked: bool,
    /// The initial commit happened and the first configure went out.
    pub initial_done: bool,
    pub pending_geometry: Option<Rect>,
    pub geometry: Option<Rect>,
    pub mapped: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Toplevel {
    pub xdg: ObjectId,
    pub surface: ObjectId,
    pub title: String,
    pub app_id: String,
    pub min_size: (i32, i32),
    pub max_size: (i32, i32),
    pub parent: Option<ObjectId>,
    /// What the client asked for.
    pub want_maximized: bool,
    pub want_fullscreen: bool,
    /// What is applied.
    pub maximized: bool,
    pub fullscreen: bool,
    /// Where the window was before it was maximized, tiled or made
    /// fullscreen.
    pub restore: Option<(i32, i32)>,
    /// The size an interactive resize asks for (configure's size while
    /// neither maximized nor fullscreen), and whether one is going on.
    pub resize_size: Option<(i32, i32)>,
    pub resizing: bool,
    /// The size of its visible bounds then, suggested when it is restored.
    pub floating: Option<(i32, i32)>,
    /// The frame's outer rectangle the shell tiles it to (ADR 0117):
    /// wanted, and applied once acked.
    pub want_tile: Option<Rect>,
    pub tile: Option<Rect>,
    pub decoration: Option<ObjectId>,
    /// A decoration configure is due with the next configure sequence.
    pub decoration_due: bool,
    /// `wm_capabilities` went out (once, before the first configure).
    pub capabilities_sent: bool,
}

#[derive(Debug)]
pub(crate) struct Popup {
    pub xdg: ObjectId,
    /// The parent's xdg_surface.
    pub parent: Option<ObjectId>,
    /// Relative to the parent's window geometry, as last configured
    /// (constraints applied).
    pub rect: Rect,
    /// The rules it was placed by.
    pub positioner: Positioner,
    /// `xdg_popup.grab` was asked for.
    pub grab: bool,
}

/// `xdg_positioner.constraint_adjustment` bits.
pub(crate) mod adjust {
    pub const SLIDE_X: u32 = 1;
    pub const SLIDE_Y: u32 = 2;
    pub const FLIP_X: u32 = 4;
    pub const FLIP_Y: u32 = 8;
    pub const RESIZE_X: u32 = 16;
    pub const RESIZE_Y: u32 = 32;
}

/// An anchor or gravity mirrored left-right (`x`) or top-bottom (`y`):
/// both enums number the edges and corners alike.
fn mirror(v: u32, x: bool) -> u32 {
    match (v, x) {
        (3, true) => 4,
        (4, true) => 3,
        (5, true) => 7,
        (7, true) => 5,
        (6, true) => 8,
        (8, true) => 6,
        (1, false) => 2,
        (2, false) => 1,
        (5, false) => 6,
        (6, false) => 5,
        (7, false) => 8,
        (8, false) => 7,
        (v, _) => v,
    }
}

/// An `xdg_positioner`'s rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Positioner {
    pub size: Option<(i32, i32)>,
    pub anchor_rect: Option<Rect>,
    pub anchor: u32,
    pub gravity: u32,
    pub constraint_adjustment: u32,
    pub offset: (i32, i32),
}

impl Positioner {
    /// Where the popup goes relative to the parent's window geometry: the
    /// anchor point on the anchor rectangle, the popup extending from it in
    /// the gravity's direction, moved by the offset. No constraint
    /// adjustment yet: a popup may extend past the output.
    pub fn place(&self) -> Option<Rect> {
        let (w, h) = self.size?;
        let a = self.anchor_rect?;
        let (cx, cy) = (a.x + a.width / 2, a.y + a.height / 2);
        let (ax, ay) = match xdg_positioner::Anchor(self.anchor) {
            xdg_positioner::Anchor::TOP => (cx, a.y),
            xdg_positioner::Anchor::BOTTOM => (cx, a.bottom()),
            xdg_positioner::Anchor::LEFT => (a.x, cy),
            xdg_positioner::Anchor::RIGHT => (a.right(), cy),
            xdg_positioner::Anchor::TOP_LEFT => (a.x, a.y),
            xdg_positioner::Anchor::BOTTOM_LEFT => (a.x, a.bottom()),
            xdg_positioner::Anchor::TOP_RIGHT => (a.right(), a.y),
            xdg_positioner::Anchor::BOTTOM_RIGHT => (a.right(), a.bottom()),
            _ => (cx, cy),
        };
        let (x, y) = match xdg_positioner::Gravity(self.gravity) {
            xdg_positioner::Gravity::TOP => (ax - w / 2, ay - h),
            xdg_positioner::Gravity::BOTTOM => (ax - w / 2, ay),
            xdg_positioner::Gravity::LEFT => (ax - w, ay - h / 2),
            xdg_positioner::Gravity::RIGHT => (ax, ay - h / 2),
            xdg_positioner::Gravity::TOP_LEFT => (ax - w, ay - h),
            xdg_positioner::Gravity::BOTTOM_LEFT => (ax - w, ay),
            xdg_positioner::Gravity::TOP_RIGHT => (ax, ay - h),
            xdg_positioner::Gravity::BOTTOM_RIGHT => (ax, ay),
            _ => (ax - w / 2, ay - h / 2),
        };
        Some(Rect::new(x + self.offset.0, y + self.offset.1, w, h))
    }

    /// [`Positioner::place`] kept inside `area` (output coordinates) by the
    /// constraint adjustments allowed, axis by axis as xdg-shell.xml orders
    /// them: flip (kept only if it is no longer constrained), then slide
    /// (towards the area until the edge is inside, the leading edge winning
    /// if it is too large), then resize (cut to the area). `origin` is
    /// where the parent's window geometry starts; the result is relative to
    /// it again.
    pub fn place_in(&self, origin: (i32, i32), area: Rect) -> Option<Rect> {
        let mut out = self.place()?.translate(origin.0, origin.1);
        let ca = self.constraint_adjustment;
        let off_x = |r: Rect| r.x < area.x || r.right() > area.right();
        let off_y = |r: Rect| r.y < area.y || r.bottom() > area.bottom();
        if off_x(out) {
            if ca & adjust::FLIP_X != 0 {
                let flipped =
                    Positioner { anchor: mirror(self.anchor, true), gravity: mirror(self.gravity, true), ..*self };
                if let Some(f) = flipped.place().map(|r| r.translate(origin.0, origin.1))
                    && !off_x(f)
                {
                    out.x = f.x;
                }
            }
            if off_x(out) && ca & adjust::SLIDE_X != 0 {
                if out.right() > area.right() {
                    out.x -= out.right() - area.right();
                }
                if out.x < area.x {
                    out.x = area.x;
                }
            }
            if off_x(out) && ca & adjust::RESIZE_X != 0 {
                let (left, right) = (out.x.max(area.x), out.right().min(area.right()));
                if right > left {
                    out = Rect::new(left, out.y, right - left, out.height);
                }
            }
        }
        if off_y(out) {
            if ca & adjust::FLIP_Y != 0 {
                let flipped =
                    Positioner { anchor: mirror(self.anchor, false), gravity: mirror(self.gravity, false), ..*self };
                if let Some(f) = flipped.place().map(|r| r.translate(origin.0, origin.1))
                    && !off_y(f)
                {
                    out.y = f.y;
                }
            }
            if off_y(out) && ca & adjust::SLIDE_Y != 0 {
                if out.bottom() > area.bottom() {
                    out.y -= out.bottom() - area.bottom();
                }
                if out.y < area.y {
                    out.y = area.y;
                }
            }
            if off_y(out) && ca & adjust::RESIZE_Y != 0 {
                let (top, bottom) = (out.y.max(area.y), out.bottom().min(area.bottom()));
                if bottom > top {
                    out = Rect::new(out.x, top, out.width, bottom - top);
                }
            }
        }
        Some(out.translate(-origin.0, -origin.1))
    }
}

/// `xdg_toplevel.resize_edge` values, and its bits.
const RESIZE_EDGES: [u32; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];
pub(crate) const RESIZE_TOP: u32 = 1;
pub(crate) const RESIZE_BOTTOM: u32 = 2;
pub(crate) const RESIZE_LEFT: u32 = 4;
pub(crate) const RESIZE_RIGHT: u32 = 8;

impl Server {
    fn xdg(&mut self, c: ClientId, id: ObjectId) -> Option<&mut XdgSurface> {
        match self.client(c).data_mut(id) {
            Some(Data::XdgSurface(x)) => Some(x),
            _ => None,
        }
    }

    pub(super) fn toplevel(&mut self, c: ClientId, id: ObjectId) -> Option<&mut Toplevel> {
        match self.client(c).data_mut(id) {
            Some(Data::Toplevel(t)) => Some(t),
            _ => None,
        }
    }

    pub(super) fn wm_base_request(&mut self, c: ClientId, base: ObjectId, req: xdg_wm_base::Request) -> Res {
        match req {
            xdg_wm_base::Request::Destroy => {
                let client = self.client(c);
                if !client.ids_where(|d| matches!(d, Data::XdgSurface(x) if x.wm_base == base)).is_empty() {
                    let code = xdg_wm_base::Error::DEFUNCT_SURFACES.0;
                    return Err(error(base, code, "xdg_wm_base destroyed before its surfaces"));
                }
            }
            xdg_wm_base::Request::CreatePositioner { id } => {
                let client = self.client(c);
                let version = client.version(base);
                client.insert(id, &xdg_positioner::INTERFACE, version, Data::Positioner(Positioner::default()));
            }
            xdg_wm_base::Request::GetXdgSurface { id, surface } => {
                let Some(s) = self.surface(c, surface) else { return Ok(()) };
                if s.xdg.is_some() || !matches!(s.role, Role::None | Role::XdgToplevel | Role::XdgPopup) {
                    let code = xdg_wm_base::Error::ROLE.0;
                    return Err(error(base, code, format!("surface {surface} already has a role object")));
                }
                if s.buffer_size.is_some() || matches!(s.pending.buffer, Some(Some(_))) {
                    let code = xdg_wm_base::Error::INVALID_SURFACE_STATE.0;
                    return Err(error(base, code, format!("surface {surface} already has a buffer")));
                }
                s.xdg = Some(id);
                let client = self.client(c);
                let version = client.version(base);
                let x = XdgSurface {
                    wm_base: base,
                    surface,
                    role: None,
                    sent: Vec::new(),
                    acked: false,
                    initial_done: false,
                    pending_geometry: None,
                    geometry: None,
                    mapped: false,
                };
                client.insert(id, &xdg_surface::INTERFACE, version, Data::XdgSurface(Box::new(x)));
            }
            xdg_wm_base::Request::Pong { .. } => {}
        }
        Ok(())
    }

    pub(super) fn positioner_request(&mut self, c: ClientId, id: ObjectId, req: xdg_positioner::Request) -> Res {
        let Some(Data::Positioner(p)) = self.client(c).data_mut(id) else { return Ok(()) };
        let invalid = |what: String| error(id, xdg_positioner::Error::INVALID_INPUT.0, what);
        match req {
            xdg_positioner::Request::SetSize { width, height } => {
                if width <= 0 || height <= 0 {
                    return Err(invalid(format!("size {width}x{height}")));
                }
                p.size = Some((width, height));
            }
            xdg_positioner::Request::SetAnchorRect { x, y, width, height } => {
                if width < 0 || height < 0 {
                    return Err(invalid(format!("anchor rectangle {width}x{height}")));
                }
                p.anchor_rect = Some(Rect::new(x, y, width, height));
            }
            xdg_positioner::Request::SetAnchor { anchor } => {
                if anchor > 8 {
                    return Err(invalid(format!("anchor {anchor}")));
                }
                p.anchor = anchor;
            }
            xdg_positioner::Request::SetGravity { gravity } => {
                if gravity > 8 {
                    return Err(invalid(format!("gravity {gravity}")));
                }
                p.gravity = gravity;
            }
            xdg_positioner::Request::SetConstraintAdjustment { constraint_adjustment } => {
                p.constraint_adjustment = constraint_adjustment;
            }
            xdg_positioner::Request::SetOffset { x, y } => p.offset = (x, y),
            xdg_positioner::Request::SetReactive
            | xdg_positioner::Request::SetParentSize { .. }
            | xdg_positioner::Request::SetParentConfigure { .. }
            | xdg_positioner::Request::Destroy => {}
        }
        Ok(())
    }

    fn positioner(&mut self, c: ClientId, id: ObjectId) -> Option<Positioner> {
        match self.client(c).data(id) {
            Some(Data::Positioner(p)) => Some(*p),
            _ => None,
        }
    }

    pub(super) fn xdg_surface_request(&mut self, c: ClientId, id: ObjectId, req: xdg_surface::Request) -> Res {
        let Some(x) = self.xdg(c, id) else { return Ok(()) };
        let (surface, role, base) = (x.surface, x.role, x.wm_base);
        let constructed = |what: &str| -> Res {
            match role {
                Some(_) => {
                    let code = xdg_surface::Error::ALREADY_CONSTRUCTED.0;
                    Err(error(id, code, format!("{what}: the xdg_surface already has a role object")))
                }
                None => Ok(()),
            }
        };
        match req {
            xdg_surface::Request::Destroy => {
                if role.is_some_and(|r| self.client(c).get(r).is_some()) {
                    let code = xdg_surface::Error::DEFUNCT_ROLE_OBJECT.0;
                    return Err(error(id, code, "xdg_surface destroyed before its role object"));
                }
                self.unmap(c, id);
                if let Some(s) = self.surface(c, surface) {
                    s.xdg = None;
                }
            }
            xdg_surface::Request::GetToplevel { id: tl } => {
                constructed("get_toplevel")?;
                let Some(s) = self.surface(c, surface) else { return Ok(()) };
                if s.role == Role::XdgPopup {
                    return Err(error(base, xdg_wm_base::Error::ROLE.0, "the surface is a popup"));
                }
                s.role = Role::XdgToplevel;
                let client = self.client(c);
                let version = client.version(id);
                let t = Toplevel { xdg: id, surface, ..Toplevel::default() };
                client.insert(tl, &xdg_toplevel::INTERFACE, version, Data::Toplevel(Box::new(t)));
                if let Some(x) = self.xdg(c, id) {
                    x.role = Some(tl);
                }
            }
            xdg_surface::Request::GetPopup { id: popup, parent, positioner } => {
                constructed("get_popup")?;
                let rules = self.positioner(c, positioner);
                let (Some(rules), Some(rect)) = (rules, rules.and_then(|p| p.place())) else {
                    let code = xdg_wm_base::Error::INVALID_POSITIONER.0;
                    return Err(error(base, code, "the positioner has no size or anchor rectangle"));
                };
                let Some(s) = self.surface(c, surface) else { return Ok(()) };
                if s.role == Role::XdgToplevel {
                    return Err(error(base, xdg_wm_base::Error::ROLE.0, "the surface is a toplevel"));
                }
                s.role = Role::XdgPopup;
                let client = self.client(c);
                let version = client.version(id);
                let p = Popup { xdg: id, parent, rect, positioner: rules, grab: false };
                client.insert(popup, &xdg_popup::INTERFACE, version, Data::Popup(p));
                if let Some(x) = self.xdg(c, id) {
                    x.role = Some(popup);
                }
            }
            xdg_surface::Request::SetWindowGeometry { x, y, width, height } => {
                if width <= 0 || height <= 0 {
                    let code = xdg_surface::Error::INVALID_SIZE.0;
                    return Err(error(id, code, format!("window geometry {width}x{height}")));
                }
                if let Some(xs) = self.xdg(c, id) {
                    xs.pending_geometry = Some(Rect::new(x, y, width, height));
                }
            }
            xdg_surface::Request::AckConfigure { serial } => {
                if role.is_none() {
                    let code = xdg_surface::Error::NOT_CONSTRUCTED.0;
                    return Err(error(id, code, "ack_configure before a role object"));
                }
                let Some(xs) = self.xdg(c, id) else { return Ok(()) };
                let Some(i) = xs.sent.iter().position(|s| *s == serial) else {
                    let code = xdg_surface::Error::INVALID_SERIAL.0;
                    return Err(error(id, code, format!("serial {serial} was not sent or was acked already")));
                };
                xs.sent.drain(..=i);
                xs.acked = true;
            }
        }
        Ok(())
    }

    /// Before a commit of `surface`: an xdg_surface needs its role object,
    /// and no buffer before the first configure is acked.
    pub(super) fn xdg_check_commit(&mut self, c: ClientId, surface: ObjectId) -> Res {
        let Some(s) = self.surface(c, surface) else { return Ok(()) };
        let (Some(xid), attaching) = (s.xdg, matches!(s.pending.buffer, Some(Some(_)))) else { return Ok(()) };
        let Some(x) = self.xdg(c, xid) else { return Ok(()) };
        if x.role.is_none() {
            let code = xdg_surface::Error::NOT_CONSTRUCTED.0;
            return Err(error(xid, code, "commit of an xdg_surface without a role object"));
        }
        if attaching && !x.acked {
            let code = xdg_surface::Error::UNCONFIGURED_BUFFER.0;
            return Err(error(xid, code, "a buffer was attached before the first configure was acked"));
        }
        Ok(())
    }

    /// After a commit of `surface`: the configure sequence on the initial
    /// commit, mapping on the first buffer, unmapping on a null one, and the
    /// window following the state.
    pub(super) fn xdg_commit(&mut self, c: ClientId, surface: ObjectId, attached: Attached) -> Res {
        let Some(s) = self.surface(c, surface) else { return Ok(()) };
        let Some(xid) = s.xdg else { return Ok(()) };
        let has_content = s.buffer_size.is_some();
        let Some(x) = self.xdg(c, xid) else { return Ok(()) };
        if let Some(g) = x.pending_geometry.take() {
            x.geometry = Some(g);
        }
        let (role, initial_done, mapped) = (x.role, x.initial_done, x.mapped);
        let Some(role) = role else { return Ok(()) };
        if !initial_done {
            if let Some(x) = self.xdg(c, xid) {
                x.initial_done = true;
            }
            let scale = self.output.scale as i32;
            self.client(c).send(surface, wl_surface::Event::PreferredBufferScale { factor: scale });
            self.configure_role(c, role);
            return Ok(());
        }
        match attached {
            Attached::Null if mapped => self.unmap(c, xid),
            Attached::Buffer(_) if !mapped && has_content => self.map(c, surface, xid, role),
            _ => {}
        }
        self.update_window(c, surface, xid, role);
        Ok(())
    }

    /// Send the role's configure sequence.
    fn configure_role(&mut self, c: ClientId, role: ObjectId) {
        match self.client(c).data(role) {
            Some(Data::Toplevel(_)) => self.configure_toplevel(c, role),
            Some(Data::Popup(_)) => self.configure_popup(c, role),
            _ => {}
        }
    }

    /// The area a maximized window's frame may fill, and its bounds inside.
    fn work_area(&self) -> (i32, i32) {
        let a = self.scene.work_area();
        (a.width - 2 * BORDER, a.height - TITLE_HEIGHT - 2 * BORDER)
    }

    /// `configure_bounds`, `wm_capabilities` (once), the decoration mode if
    /// due, `xdg_toplevel.configure`, then `xdg_surface.configure`.
    pub(super) fn configure_toplevel(&mut self, c: ClientId, tl: ObjectId) {
        let active = self.active == Some((c, tl));
        let (ow, oh) = self.scene.logical_size();
        let work = self.work_area();
        let Some(t) = self.toplevel(c, tl) else { return };
        let xid = t.xdg;
        let mapped = matches!(self.client(c).data(xid), Some(Data::XdgSurface(x)) if x.mapped);
        let bounds = self.toplevel_window(c, tl).and_then(|w| self.scene.window(w)).map(|w| w.bounds());
        let Some(t) = self.toplevel(c, tl) else { return };
        if (t.want_maximized || t.want_fullscreen || t.want_tile.is_some())
            && !(t.maximized || t.fullscreen || t.tile.is_some())
            && let Some(b) = bounds
        {
            // Leaving the floating state: the size to suggest when it comes back.
            t.floating = Some((b.width, b.height));
        }
        // A window not shown yet gets focus when it is.
        let mut states = Vec::new();
        if active || !mapped {
            states.push(xdg_toplevel::State::ACTIVATED.0);
        }
        let mut tile = None;
        let size = if t.want_fullscreen {
            states.push(xdg_toplevel::State::FULLSCREEN.0);
            (ow, oh)
        } else if t.want_maximized {
            states.push(xdg_toplevel::State::MAXIMIZED.0);
            work
        } else if let Some(outer) = t.want_tile {
            // Tiled (ADR 0117): the bounds the frame leaves.
            tile = Some(outer);
            (outer.width - 2 * BORDER, outer.height - TITLE_HEIGHT - 2 * BORDER)
        } else if t.maximized || t.fullscreen || t.tile.is_some() {
            // Going back to floating: the size it had.
            t.floating.unwrap_or((0, 0))
        } else {
            t.resize_size.unwrap_or((0, 0))
        };
        if t.resizing {
            states.push(xdg_toplevel::State::RESIZING.0);
        }
        let send_caps = !core::mem::replace(&mut t.capabilities_sent, true);
        let decoration = t.decoration.filter(|_| core::mem::take(&mut t.decoration_due));
        if let Some(outer) = tile
            && self.client(c).version(tl) >= 2
        {
            self.tiled_states(outer, &mut states);
        }
        let serial = self.next_serial();
        let client = self.client(c);
        client.send(tl, xdg_toplevel::Event::ConfigureBounds { width: work.0, height: work.1 });
        if send_caps {
            let caps = [
                xdg_toplevel::WmCapabilities::MAXIMIZE.0,
                xdg_toplevel::WmCapabilities::FULLSCREEN.0,
                xdg_toplevel::WmCapabilities::MINIMIZE.0,
            ];
            client.send(tl, xdg_toplevel::Event::WmCapabilities { capabilities: u32_array(&caps) });
        }
        if let Some(d) = decoration {
            let mode = zxdg_toplevel_decoration_v1::Mode::SERVER_SIDE.0;
            client.send(d, zxdg_toplevel_decoration_v1::Event::Configure { mode });
        }
        client.send(tl, xdg_toplevel::Event::Configure { width: size.0, height: size.1, states: u32_array(&states) });
        client.send(xid, xdg_surface::Event::Configure { serial });
        if let Some(Data::XdgSurface(x)) = client.data_mut(xid) {
            x.sent.push(serial);
        }
    }

    /// Where a popup's parent's window geometry starts on the output, if
    /// the parent is shown.
    fn popup_origin(&mut self, c: ClientId, parent: Option<ObjectId>) -> Option<(i32, i32)> {
        let px = parent?;
        let pw = self.xdg_window(c, px)?;
        let ps = self.xdg(c, px)?.surface;
        let pg = self.geometry(c, ps, px);
        let (wx, wy) = self.scene.window(pw)?.position();
        Some((wx + pg.x, wy + pg.y))
    }

    fn configure_popup(&mut self, c: ClientId, popup: ObjectId) {
        let Some(Data::Popup(p)) = self.client(c).data(popup) else { return };
        let (xid, parent, rules, mut r) = (p.xdg, p.parent, p.positioner, p.rect);
        // Kept on the output (the work area) where the rules allow.
        let origin = self.popup_origin(c, parent);
        if let Some(origin) = origin {
            let area = self.scene.work_area();
            r = rules.place_in(origin, area).unwrap_or(r);
        }
        if let Some(Data::Popup(p)) = self.client(c).data_mut(popup) {
            p.rect = r;
        }
        // Shown already (a reposition): it moves now.
        if let (Some(origin), Some(win)) = (origin, self.xdg_window(c, xid)) {
            let surface = self.xdg(c, xid).map(|x| x.surface).unwrap_or(popup);
            let g = self.geometry(c, surface, xid);
            self.scene.set_position(win, origin.0 + r.x - g.x, origin.1 + r.y - g.y);
        }
        let serial = self.next_serial();
        let client = self.client(c);
        client.send(popup, xdg_popup::Event::Configure { x: r.x, y: r.y, width: r.width, height: r.height });
        client.send(xid, xdg_surface::Event::Configure { serial });
        if let Some(Data::XdgSurface(x)) = client.data_mut(xid) {
            x.sent.push(serial);
        }
    }

    /// The surface's visible bounds relative to it: the window geometry or
    /// all of it.
    fn geometry(&mut self, c: ClientId, surface: ObjectId, xid: ObjectId) -> Rect {
        let size = self.surface(c, surface).and_then(|s| s.logical_size()).unwrap_or((0, 0));
        let g = self.xdg(c, xid).and_then(|x| x.geometry);
        g.unwrap_or(Rect::new(0, 0, size.0, size.1))
    }

    /// The window of an xdg_surface, if it is shown.
    fn xdg_window(&mut self, c: ClientId, xid: ObjectId) -> Option<WindowId> {
        let surface = self.xdg(c, xid)?.surface;
        self.surface(c, surface)?.window
    }

    fn map(&mut self, c: ClientId, surface: ObjectId, xid: ObjectId, role: ObjectId) {
        let g = self.geometry(c, surface, xid);
        let toplevel = matches!(self.client(c).data(role), Some(Data::Toplevel(_)));
        let win = self.scene.add_window(owner_tag(c, surface), toplevel);
        if toplevel {
            let (x, y) = self.scene.place(g.x, g.y, g.width, g.height, true);
            self.scene.set_position(win, x, y);
            let parent = self.toplevel(c, role).and_then(|t| t.parent);
            let parent_win =
                parent.and_then(|p| self.toplevel(c, p).map(|t| t.xdg)).and_then(|x| self.xdg_window(c, x));
            if let Some(pw) = parent_win {
                self.scene.place_above(win, pw);
            }
        } else if let Some(Data::Popup(p)) = self.client(c).data(role) {
            let (parent, rect, grab) = (p.parent, p.rect, p.grab);
            if grab {
                self.popup_grabs.push((c, role));
            }
            let parent_win = parent.and_then(|px| self.xdg_window(c, px));
            let origin = match (parent, parent_win) {
                (Some(px), Some(pw)) => {
                    let ps = self.xdg(c, px).map(|x| x.surface).unwrap_or(surface);
                    let pg = self.geometry(c, ps, px);
                    let (wx, wy) = self.scene.window(pw).map_or((0, 0), |w| w.position());
                    (wx + pg.x, wy + pg.y)
                }
                _ => (0, 0),
            };
            self.scene.set_position(win, origin.0 + rect.x - g.x, origin.1 + rect.y - g.y);
            if let Some(pw) = parent_win {
                // Part of its parent: moves and is raised with it.
                self.scene.set_parent(win, Some(pw));
                self.scene.place_above(win, pw);
            }
        }
        if let Some(s) = self.surface(c, surface) {
            s.window = Some(win);
        }
        if let Some(x) = self.xdg(c, xid) {
            x.mapped = true;
        }
        self.surface_entered(c, surface);
        if toplevel {
            self.activate(c, role, false);
        }
    }

    /// Take the xdg_surface's window off the output; the role starts over
    /// (initial commit, configure). Its popups are dismissed.
    pub(super) fn unmap(&mut self, c: ClientId, xid: ObjectId) {
        let Some(x) = self.xdg(c, xid) else { return };
        let (surface, role) = (x.surface, x.role);
        (x.mapped, x.initial_done, x.acked) = (false, false, false);
        x.sent.clear();
        x.geometry = None;
        if let Some(w) = self.surface(c, surface).and_then(|s| s.window.take()) {
            self.scene.remove_window(w);
        }
        if let Some(t) = role.and_then(|r| self.toplevel(c, r)) {
            let (decoration, xdg, surface) = (t.decoration, t.xdg, t.surface);
            *t = Toplevel { xdg, surface, decoration, decoration_due: decoration.is_some(), ..Toplevel::default() };
        }
        if let Some(r) = role
            && self.active == Some((c, r))
        {
            self.active = None;
            // The next window gets the focus (ADR 0117).
            self.focus_next();
        }
        if let Some(r) = role {
            self.popup_grabs.retain(|g| *g != (c, r));
            self.end_grabs_of(c, r);
        }
        let client = self.client(c);
        let children = client.ids_where(|d| matches!(d, Data::Popup(p) if p.parent == Some(xid)));
        for p in children {
            self.client(c).send(p, xdg_popup::Event::PopupDone);
            if let Some(Data::Popup(popup)) = self.client(c).data(p) {
                let child = popup.xdg;
                self.unmap(c, child);
            }
        }
    }

    /// The window follows the surface: visible bounds, title, and the
    /// maximized and fullscreen states once the client acked them.
    fn update_window(&mut self, c: ClientId, surface: ObjectId, xid: ObjectId, role: ObjectId) {
        let Some(win) = self.surface(c, surface).and_then(|s| s.window) else { return };
        let geometry = self.xdg(c, xid).and_then(|x| x.geometry);
        self.scene.set_geometry(win, geometry);
        let acked_all = self.xdg(c, xid).is_some_and(|x| x.sent.is_empty());
        let g = self.geometry(c, surface, xid);
        let pos = self.scene.window(win).map_or((0, 0), |w| w.position());
        let top = self.scene.reserved_top();
        // An interactive resize from the left or top keeps the opposite
        // edge where it was.
        let anchored = self.resize_anchor(win).map(|(edges, right, bottom)| {
            let x = if edges & RESIZE_LEFT != 0 { right - g.width - g.x } else { pos.0 };
            let y = if edges & RESIZE_TOP != 0 { bottom - g.height - g.y } else { pos.1 };
            (x, y)
        });
        let Some(t) = self.toplevel(c, role) else { return };
        let title = t.title.clone();
        let mut position = anchored.filter(|p| *p != pos);
        let mut decorated = !t.fullscreen;
        let want = (t.want_maximized, t.want_fullscreen, t.want_tile);
        if acked_all && (t.maximized, t.fullscreen, t.tile) != want {
            if !t.maximized && !t.fullscreen && t.tile.is_none() {
                // Leaving the floating state: where it was.
                t.restore = Some(pos);
            }
            (t.maximized, t.fullscreen, t.tile) = want;
            decorated = !t.fullscreen;
            position = Some(if t.fullscreen {
                (-g.x, -g.y)
            } else if t.maximized {
                (BORDER - g.x, top + TITLE_HEIGHT + BORDER - g.y)
            } else if let Some(outer) = t.tile {
                (outer.x + BORDER - g.x, outer.y + TITLE_HEIGHT + BORDER - g.y)
            } else {
                t.restore.take().unwrap_or(pos)
            });
        }
        let snapped = t.maximized || t.tile.is_some();
        self.scene.set_title(win, &title);
        self.scene.set_decorated(win, decorated);
        self.scene.set_maximized(win, snapped);
        if let Some((x, y)) = position {
            self.scene.set_position(win, x, y);
        }
    }

    /// Give a toplevel focus: it is raised and shown active, the one that
    /// had focus is not any more; both hear of it through configure events
    /// (`configure` says so only if the window was configured as inactive).
    pub(super) fn activate(&mut self, c: ClientId, tl: ObjectId, configure: bool) {
        if let Some(iw) = self.active_internal.take() {
            self.scene.set_active(iw, false);
        }
        let previous = self.active.replace((c, tl));
        if previous == Some((c, tl)) {
            // It may have been given the focus before it had a window (an
            // activation request before the first map): the window is
            // shown active now.
            if let Some(w) = self.toplevel_window(c, tl) {
                self.scene.set_active(w, true);
                self.scene.raise(w);
            }
            return;
        }
        if let Some((pc, ptl)) = previous
            && self.clients.contains_key(&pc)
            && let Some(w) = self.toplevel_window(pc, ptl)
        {
            self.scene.set_active(w, false);
            self.configure_toplevel(pc, ptl);
        }
        if let Some((pc, ptl)) = previous {
            self.x_lost_focus(pc, ptl);
        }
        if let Some(w) = self.toplevel_window(c, tl) {
            self.scene.set_active(w, true);
            self.scene.raise(w);
            if configure {
                self.configure_toplevel(c, tl);
            }
        }
    }

    /// The window of a toplevel — or of a surface an outside window manager
    /// manages (`external.rs`), which takes a toplevel's place.
    pub(super) fn toplevel_window(&mut self, c: ClientId, tl: ObjectId) -> Option<WindowId> {
        if let Some(s) = self.surface(c, tl).filter(|s| s.role == Role::External) {
            return s.window;
        }
        let xid = self.toplevel(c, tl)?.xdg;
        self.xdg_window(c, xid)
    }

    /// The toplevel (or externally managed surface) shown in scene window
    /// `w`.
    pub(super) fn window_toplevel(&mut self, w: WindowId) -> Option<(ClientId, ObjectId)> {
        // A subsurface's or popup's window is part of its toplevel's.
        let w = self.scene.root(w);
        let (c, surface) = owner_of(self.scene.window(w)?.owner());
        if !self.clients.contains_key(&c) {
            return None;
        }
        if self.surface(c, surface)?.role == Role::External {
            return Some((c, surface));
        }
        let xid = self.surface(c, surface)?.xdg?;
        let role = self.xdg(c, xid)?.role?;
        self.toplevel(c, role).map(|_| (c, role))
    }

    /// A click on a window: focus it.
    pub(super) fn focus_window(&mut self, w: WindowId) {
        if self.internal.contains_key(&w) {
            self.activate_internal(w);
        } else if let Some((c, tl)) = self.window_toplevel(w) {
            self.activate(c, tl, true);
        } else {
            self.x_clicked(w);
        }
    }

    /// A click on a window's close button: ask its client to close it (an
    /// internal window's owner, ADR 0117; an externally managed one: its
    /// window manager, through [`Server::take_wm_events`]).
    pub(super) fn close_window(&mut self, w: WindowId) {
        if self.internal.contains_key(&w) {
            self.internal_events.push(super::InternalEvent::Close(w));
        } else if let Some((c, tl)) = self.window_toplevel(w) {
            if self.surface_ref(c, tl).is_some_and(|s| s.role == Role::External) {
                self.wm_events.push(super::WmEvent::Close { client: c, surface: tl });
            } else {
                self.client(c).send(tl, xdg_toplevel::Event::Close);
            }
        } else {
            self.x_close(w);
        }
    }

    pub(super) fn toplevel_request(&mut self, c: ClientId, tl: ObjectId, req: xdg_toplevel::Request) -> Res {
        let Some(t) = self.toplevel(c, tl) else { return Ok(()) };
        let xid = t.xdg;
        let mut reconfigure = false;
        match req {
            xdg_toplevel::Request::Destroy => {
                if let Some(d) = t.decoration {
                    let code = zxdg_toplevel_decoration_v1::Error::ORPHANED.0;
                    return Err(error(d, code, "xdg_toplevel destroyed before its decoration"));
                }
                self.unmap(c, xid);
                if let Some(x) = self.xdg(c, xid) {
                    x.role = None;
                }
                return Ok(());
            }
            xdg_toplevel::Request::SetParent { parent } => {
                if parent == Some(tl) {
                    let code = xdg_toplevel::Error::INVALID_PARENT.0;
                    return Err(error(tl, code, "a toplevel cannot be its own parent"));
                }
                t.parent = parent;
            }
            // Not double-buffered: shown at once.
            xdg_toplevel::Request::SetTitle { title } => {
                t.title = title.clone();
                if let Some(w) = self.toplevel_window(c, tl) {
                    self.scene.set_title(w, &title);
                }
            }
            xdg_toplevel::Request::SetAppId { app_id } => t.app_id = app_id,
            xdg_toplevel::Request::SetMaxSize { width, height }
            | xdg_toplevel::Request::SetMinSize { width, height }
                if width < 0 || height < 0 =>
            {
                let code = xdg_toplevel::Error::INVALID_SIZE.0;
                return Err(error(tl, code, format!("size limit {width}x{height}")));
            }
            xdg_toplevel::Request::SetMaxSize { width, height } => t.max_size = (width, height),
            xdg_toplevel::Request::SetMinSize { width, height } => t.min_size = (width, height),
            xdg_toplevel::Request::Resize { edges, .. } => {
                if !RESIZE_EDGES.contains(&edges) {
                    let code = xdg_toplevel::Error::INVALID_RESIZE_EDGE.0;
                    return Err(error(tl, code, format!("resize edge {edges}")));
                }
                // An interactive resize while a button is held.
                if edges != 0
                    && let Some(w) = self.toplevel_window(c, tl)
                {
                    self.start_resize(w, edges);
                }
                return Ok(());
            }
            xdg_toplevel::Request::SetMaximized => (t.want_maximized, reconfigure) = (true, true),
            xdg_toplevel::Request::UnsetMaximized => (t.want_maximized, reconfigure) = (false, true),
            xdg_toplevel::Request::SetFullscreen { .. } => (t.want_fullscreen, reconfigure) = (true, true),
            xdg_toplevel::Request::UnsetFullscreen => (t.want_fullscreen, reconfigure) = (false, true),
            // An interactive move while a button is held (ADR 0107).
            xdg_toplevel::Request::Move { .. } => {
                if let Some(w) = self.toplevel_window(c, tl) {
                    self.start_move(w);
                }
                return Ok(());
            }
            // Minimizing (ADR 0117): off the output, the focus moves on.
            xdg_toplevel::Request::SetMinimized => {
                if let Some(w) = self.toplevel_window(c, tl) {
                    self.minimize_window(w);
                }
                return Ok(());
            }
            // Interactive resize and the window menu are not offered yet.
            xdg_toplevel::Request::ShowWindowMenu { .. } => {}
        }
        if reconfigure && self.xdg(c, xid).is_some_and(|x| x.initial_done) {
            self.configure_toplevel(c, tl);
        }
        Ok(())
    }

    pub(super) fn popup_request(&mut self, c: ClientId, popup: ObjectId, req: xdg_popup::Request) -> Res {
        let Some(Data::Popup(p)) = self.client(c).data(popup) else { return Ok(()) };
        let xid = p.xdg;
        match req {
            xdg_popup::Request::Destroy => {
                // Popups with a grab go top down.
                let top = self.popup_grabs.iter().rposition(|g| g.0 == c);
                let mine = self.popup_grabs.iter().position(|g| *g == (c, popup));
                if let (Some(top), Some(mine)) = (top, mine)
                    && mine < top
                {
                    let base = self.xdg(c, xid).map_or(popup, |x| x.wm_base);
                    let code = xdg_wm_base::Error::NOT_THE_TOPMOST_POPUP.0;
                    return Err(error(base, code, "a popup was destroyed before the one above it"));
                }
                self.unmap(c, xid);
                if let Some(x) = self.xdg(c, xid) {
                    x.role = None;
                }
            }
            // The popup takes the keyboard; a click outside the client's
            // surfaces dismisses it (`input.rs`).
            xdg_popup::Request::Grab { .. } => {
                if self.xdg(c, xid).is_some_and(|x| x.mapped) {
                    let code = xdg_popup::Error::INVALID_GRAB.0;
                    return Err(error(popup, code, "a grab after the popup was mapped"));
                }
                if let Some(Data::Popup(p)) = self.client(c).data_mut(popup) {
                    p.grab = true;
                }
            }
            xdg_popup::Request::Reposition { positioner, token } => {
                let rules = self.positioner(c, positioner);
                let (Some(rules), Some(rect)) = (rules, rules.and_then(|p| p.place())) else {
                    let base = self.xdg(c, xid).map_or(popup, |x| x.wm_base);
                    let code = xdg_wm_base::Error::INVALID_POSITIONER.0;
                    return Err(error(base, code, "the positioner has no size or anchor rectangle"));
                };
                if let Some(Data::Popup(p)) = self.client(c).data_mut(popup) {
                    (p.rect, p.positioner) = (rect, rules);
                }
                self.client(c).send(popup, xdg_popup::Event::Repositioned { token });
                self.configure_popup(c, popup);
            }
        }
        Ok(())
    }
}

/// 32-bit values in the host's byte order, as `array` arguments carry them.
pub(super) fn u32_array(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}
