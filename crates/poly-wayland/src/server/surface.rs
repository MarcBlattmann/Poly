//! `wl_compositor`, `wl_surface` and `wl_region`: surfaces and their
//! double-buffered state (buffer, damage, frame callbacks, scale,
//! transform, viewport, regions), applied on `commit` — or, for an
//! effectively synchronized subsurface, cached until its parent's state is
//! applied (wayland.xml's content updates).
//!
//! A surface's content lives in its scene window while it has one (a
//! mapped toplevel, popup or subsurface), else in the surface itself (a
//! cursor surface, a drag icon, a subsurface whose parent is not shown),
//! so a buffer is copied once, at commit, either way.

use super::{ClientId, Data, Res, Server, error, owner_tag};
use crate::compose::{Content, Rect, View, WindowId};
use crate::protocol::{wl_buffer, wl_callback, wl_compositor, wl_region, wl_surface, wp_viewport};
use crate::wire::ObjectId;
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;

/// A region: rectangles added and subtracted, in order. Kept for the input
/// and opaque regions; composing does not use them yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Region {
    pub ops: Vec<(bool, Rect)>,
}

impl Region {
    /// Whether the point is in the region (the last operation covering it
    /// decides).
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.ops.iter().rev().find(|(_, r)| r.contains(x, y)).is_some_and(|(add, _)| *add)
    }
}

/// State that takes effect on the next commit.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    /// `Some(None)`: a null buffer was attached.
    pub buffer: Option<Option<ObjectId>>,
    pub damage: Vec<Rect>,
    pub buffer_damage: Vec<Rect>,
    pub frame_callbacks: Vec<ObjectId>,
    pub feedbacks: Vec<ObjectId>,
    pub scale: Option<i32>,
    pub transform: Option<i32>,
    pub offset: Option<(i32, i32)>,
    pub opaque: Option<Option<Region>>,
    pub input: Option<Option<Region>>,
    /// `wp_viewport.set_source` (1/256 pixels) and `set_destination`;
    /// `Some(None)` unsets.
    pub source: Option<Option<(i32, i32, i32, i32)>>,
    pub destination: Option<Option<(i32, i32)>>,
    /// The parent's side of its subsurfaces: their stacking (this surface
    /// among them) and positions (`wl_subsurface.place_*`, `set_position`).
    pub stack: Option<Vec<ObjectId>>,
    pub child_positions: Vec<(ObjectId, (i32, i32))>,
}

impl Pending {
    /// Fold a newer content update into this one (a synchronized
    /// subsurface's cache): newer values win, lists add up, offsets sum.
    pub fn merge(&mut self, newer: Pending) {
        if newer.buffer.is_some() {
            self.buffer = newer.buffer;
        }
        self.damage.extend(newer.damage);
        self.buffer_damage.extend(newer.buffer_damage);
        self.frame_callbacks.extend(newer.frame_callbacks);
        self.feedbacks.extend(newer.feedbacks);
        macro_rules! newest {
            ($($f:ident),*) => { $(if newer.$f.is_some() { self.$f = newer.$f; })* };
        }
        newest!(scale, transform, opaque, input, source, destination, stack);
        if let Some((x, y)) = newer.offset {
            let (ox, oy) = self.offset.unwrap_or((0, 0));
            self.offset = Some((ox + x, oy + y));
        }
        self.child_positions.extend(newer.child_positions);
    }
}

/// The role a surface was given; permanent once set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    None,
    XdgToplevel,
    XdgPopup,
    Subsurface,
    /// `wl_pointer.set_cursor`.
    Cursor,
    /// `wl_data_device.start_drag`'s icon.
    DndIcon,
    /// Managed by an outside window manager (`external.rs`: XWayland).
    External,
}

/// A subsurface's link to its parent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sub {
    /// The `wl_subsurface`, while it exists.
    pub object: Option<ObjectId>,
    pub parent: ObjectId,
    pub sync: bool,
    /// Where it is in its parent's coordinates (applied with the parent).
    pub position: (i32, i32),
}

#[derive(Debug)]
pub(crate) struct Surface {
    pub pending: Pending,
    /// Content updates of an effectively synchronized subsurface, waiting
    /// for its parent's state to be applied.
    pub cached: Option<Pending>,
    pub scale: i32,
    pub transform: i32,
    /// The current buffer's size in buffer pixels; `None` without content.
    pub buffer_size: Option<(i32, i32)>,
    /// Committed frame callbacks, answered with the next frame.
    pub frame_callbacks: Vec<ObjectId>,
    pub feedbacks: Vec<ObjectId>,
    pub opaque: Option<Region>,
    pub input: Option<Region>,
    pub role: Role,
    /// The xdg_surface made for it, while there is one.
    pub xdg: Option<ObjectId>,
    /// Its window in the scene while it is shown.
    pub window: Option<WindowId>,
    /// Its `wp_viewport`, and the viewport's current source and
    /// destination.
    pub viewport: Option<ObjectId>,
    pub source: Option<(i32, i32, i32, i32)>,
    pub destination: Option<(i32, i32)>,
    /// Its parent, if it is a subsurface.
    pub sub: Option<Sub>,
    /// Its subsurfaces and itself, bottom to top (current state).
    pub stack: Vec<ObjectId>,
    /// The stacking changed since the windows were last put in order.
    pub restack: bool,
    /// The content while the surface has no window.
    pub stash: Content,
    /// The offset (`wl_surface.offset`, `attach`'s x and y) the last
    /// commit applied: a cursor's hot spot moves by it.
    pub applied_offset: (i32, i32),
    /// Its `wp_fractional_scale_v1`, if any.
    pub fractional: Option<ObjectId>,
    /// How an outside window manager shows it (role `external`).
    pub managed: Option<super::external::Managed>,
}

impl Surface {
    fn new(id: ObjectId) -> Surface {
        Surface {
            pending: Pending::default(),
            cached: None,
            scale: 1,
            transform: 0,
            buffer_size: None,
            frame_callbacks: Vec::new(),
            feedbacks: Vec::new(),
            opaque: None,
            input: None,
            role: Role::None,
            xdg: None,
            window: None,
            viewport: None,
            source: None,
            destination: None,
            sub: None,
            stack: alloc::vec![id],
            restack: false,
            stash: Content::default(),
            applied_offset: (0, 0),
            fractional: None,
            managed: None,
        }
    }

    /// How the buffer is shown (transform, viewport).
    pub fn view(&self) -> View {
        View { transform: self.transform as u32, source: self.source, destination: self.destination }
    }

    /// The content's size in surface (logical) coordinates: the viewport's
    /// destination, else its source's size, else the transformed buffer
    /// over its scale.
    pub fn logical_size(&self) -> Option<(i32, i32)> {
        let (w, h) = self.buffer_size?;
        if let Some(d) = self.destination {
            return Some(d);
        }
        if let Some((_, _, sw, sh)) = self.source {
            return Some((sw >> 8, sh >> 8));
        }
        let (w, h) = if self.transform & 1 != 0 { (h, w) } else { (w, h) };
        Some((w / self.scale.max(1), h / self.scale.max(1)))
    }

    /// Whether a surface-local point takes input (the input region, or all
    /// of the surface).
    pub fn accepts_input(&self, x: i32, y: i32) -> bool {
        self.input.as_ref().is_none_or(|r| r.contains(x, y))
    }
}

/// What a commit did to the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attached {
    Nothing,
    Buffer(ObjectId),
    Null,
}

impl Server {
    pub(super) fn surface(&mut self, c: ClientId, id: ObjectId) -> Option<&mut Surface> {
        match self.client(c).data_mut(id) {
            Some(Data::Surface(s)) => Some(s),
            _ => None,
        }
    }

    pub(super) fn surface_ref(&self, c: ClientId, id: ObjectId) -> Option<&Surface> {
        match self.clients.get(&c)?.data(id) {
            Some(Data::Surface(s)) => Some(s),
            _ => None,
        }
    }

    pub(super) fn compositor_request(&mut self, c: ClientId, comp: ObjectId, req: wl_compositor::Request) -> Res {
        let client = self.client(c);
        let version = client.version(comp);
        match req {
            wl_compositor::Request::CreateSurface { id } => {
                client.insert(id, &wl_surface::INTERFACE, version, Data::Surface(Box::new(Surface::new(id))));
            }
            wl_compositor::Request::CreateRegion { id } => {
                client.insert(id, &wl_region::INTERFACE, version, Data::Region(Region::default()));
            }
            wl_compositor::Request::Release => {}
        }
        Ok(())
    }

    pub(super) fn region_request(&mut self, c: ClientId, id: ObjectId, req: wl_region::Request) -> Res {
        let Some(Data::Region(r)) = self.client(c).data_mut(id) else { return Ok(()) };
        match req {
            wl_region::Request::Add { x, y, width, height } => r.ops.push((true, Rect::new(x, y, width, height))),
            wl_region::Request::Subtract { x, y, width, height } => r.ops.push((false, Rect::new(x, y, width, height))),
            wl_region::Request::Destroy => {}
        }
        Ok(())
    }

    fn region_copy(&mut self, c: ClientId, id: Option<ObjectId>) -> Option<Region> {
        match self.client(c).data(id?) {
            Some(Data::Region(r)) => Some(r.clone()),
            _ => None,
        }
    }

    pub(super) fn surface_request(&mut self, c: ClientId, id: ObjectId, req: wl_surface::Request) -> Res {
        let version = self.client(c).version(id);
        let opaque = match &req {
            wl_surface::Request::SetOpaqueRegion { region } => Some(self.region_copy(c, *region)),
            wl_surface::Request::SetInputRegion { region } => Some(self.region_copy(c, *region)),
            _ => None,
        };
        if let wl_surface::Request::Frame { callback } = req {
            self.client(c).insert(callback, &wl_callback::INTERFACE, 1, Data::Callback);
        }
        if let wl_surface::Request::Destroy = req {
            return self.surface_destroyed(c, id);
        }
        if let wl_surface::Request::Commit = req {
            return self.commit(c, id);
        }
        let Some(s) = self.surface(c, id) else { return Ok(()) };
        let p = &mut s.pending;
        match req {
            wl_surface::Request::Attach { buffer, x, y } => {
                if version >= 5 && (x, y) != (0, 0) {
                    let code = wl_surface::Error::INVALID_OFFSET.0;
                    return Err(error(id, code, "attach with an offset; use wl_surface.offset"));
                }
                p.buffer = Some(buffer);
                if (x, y) != (0, 0) {
                    p.offset = Some((x, y));
                }
            }
            wl_surface::Request::Damage { x, y, width, height } => p.damage.push(Rect::new(x, y, width, height)),
            wl_surface::Request::DamageBuffer { x, y, width, height } => {
                p.buffer_damage.push(Rect::new(x, y, width, height))
            }
            wl_surface::Request::Frame { callback } => p.frame_callbacks.push(callback),
            wl_surface::Request::SetOpaqueRegion { .. } => p.opaque = opaque,
            wl_surface::Request::SetInputRegion { .. } => p.input = opaque,
            wl_surface::Request::SetBufferTransform { transform } => {
                if !(0..=7).contains(&transform) {
                    let code = wl_surface::Error::INVALID_TRANSFORM.0;
                    return Err(error(id, code, format!("buffer transform {transform}")));
                }
                p.transform = Some(transform);
            }
            wl_surface::Request::SetBufferScale { scale } => {
                if scale <= 0 {
                    return Err(error(id, wl_surface::Error::INVALID_SCALE.0, format!("buffer scale {scale}")));
                }
                p.scale = Some(scale);
            }
            wl_surface::Request::Offset { x, y } => p.offset = Some((x, y)),
            wl_surface::Request::GetRelease { .. } | wl_surface::Request::Destroy | wl_surface::Request::Commit => {}
        }
        Ok(())
    }

    fn surface_destroyed(&mut self, c: ClientId, id: ObjectId) -> Res {
        let Some(s) = self.surface(c, id) else { return Ok(()) };
        if s.xdg.is_some() {
            let code = wl_surface::Error::DEFUNCT_ROLE_OBJECT.0;
            return Err(error(id, code, "the surface was destroyed before its xdg_surface"));
        }
        let (window, sub, stack) = (s.window.take(), s.sub, s.stack.clone());
        if let Some(w) = window {
            self.scene.remove_window(w);
        }
        // Its subsurfaces are unmapped; it leaves its parent's stack.
        for child in stack.into_iter().filter(|ch| *ch != id) {
            self.hide_subsurface_tree(c, child);
            if let Some(cs) = self.surface(c, child)
                && let Some(sub) = &mut cs.sub
            {
                sub.parent = child;
            }
        }
        if let Some(sub) = sub
            && let Some(parent) = self.surface(c, sub.parent)
        {
            parent.stack.retain(|s| *s != id);
            if let Some(st) = &mut parent.pending.stack {
                st.retain(|s| *s != id);
            }
        }
        self.surface_gone(c, id);
        Ok(())
    }

    /// Commit: apply the pending state, or cache it for a synchronized
    /// subsurface.
    pub(super) fn commit(&mut self, c: ClientId, id: ObjectId) -> Res {
        self.xdg_check_commit(c, id)?;
        let sync = self.effectively_sync(c, id);
        let Some(s) = self.surface(c, id) else { return Ok(()) };
        let p = core::mem::take(&mut s.pending);
        if sync {
            match &mut s.cached {
                Some(cached) => cached.merge(p),
                None => s.cached = Some(p),
            }
            return Ok(());
        }
        self.apply(c, id, p)
    }

    /// Whether a surface is an effectively synchronized subsurface: it, or
    /// a subsurface it is in, is synchronized.
    pub(super) fn effectively_sync(&self, c: ClientId, id: ObjectId) -> bool {
        let mut at = id;
        for _ in 0..64 {
            let Some(sub) = self.surface_ref(c, at).and_then(|s| s.sub) else { return false };
            if sub.object.is_none() {
                return false;
            }
            if sub.sync {
                return true;
            }
            if sub.parent == at {
                return false;
            }
            at = sub.parent;
        }
        false
    }

    /// Apply a content update.
    pub(super) fn apply(&mut self, c: ClientId, id: ObjectId, p: Pending) -> Res {
        let client = self.client(c);
        let buffer_size = |b: ObjectId| match client.data(b) {
            Some(Data::Buffer(b)) => Some((b.width, b.height)),
            _ => None,
        };
        let Some(Data::Surface(s)) = client.objects.get(&id.0).map(|o| &o.data) else { return Ok(()) };
        let scale = p.scale.unwrap_or(s.scale);
        let attached = match p.buffer {
            None => Attached::Nothing,
            Some(None) => Attached::Null,
            // A buffer destroyed while pending: no content, as most do.
            Some(Some(b)) => buffer_size(b).map_or(Attached::Null, |_| Attached::Buffer(b)),
        };
        let new_size = match attached {
            Attached::Buffer(b) => buffer_size(b),
            Attached::Nothing => s.buffer_size,
            Attached::Null => None,
        };
        let source = p.source.unwrap_or(s.source);
        let destination = p.destination.unwrap_or(s.destination);
        let transform = p.transform.unwrap_or(s.transform);
        let viewport = s.viewport;
        if let (Attached::Buffer(_), Some((w, h))) = (attached, new_size)
            && destination.is_none()
            && source.is_none()
            && (w % scale != 0 || h % scale != 0)
        {
            let msg = format!("buffer {w}x{h} is not a multiple of scale {scale}");
            return Err(error(id, wl_surface::Error::INVALID_SIZE.0, msg));
        }
        // The viewport's rules (viewporter.xml), checked when applied.
        if let (Some(vp), Some((w, h)), Some((sx, sy, sw, sh))) = (viewport, new_size, source) {
            let (tw, th) = if transform & 1 != 0 { (h, w) } else { (w, h) };
            let (lw, lh) = (i64::from(tw) * 256 / i64::from(scale), i64::from(th) * 256 / i64::from(scale));
            if i64::from(sx) + i64::from(sw) > lw || i64::from(sy) + i64::from(sh) > lh {
                let code = wp_viewport::Error::OUT_OF_BUFFER.0;
                return Err(error(vp, code, "the source rectangle extends outside the buffer"));
            }
            if destination.is_none() && (sw & 0xff != 0 || sh & 0xff != 0) {
                let code = wp_viewport::Error::BAD_SIZE.0;
                return Err(error(vp, code, "a source of a fractional size without a destination"));
            }
        }
        // Feedback for content that is replaced before it was shown.
        let discarded: Vec<ObjectId> = match attached {
            Attached::Nothing => Vec::new(),
            _ => s.feedbacks.clone(),
        };
        for f in discarded {
            client.send(f, crate::protocol::wp_presentation_feedback::Event::Discarded);
            client.delete(f);
        }
        let Some(Data::Surface(s)) = client.data_mut(id) else { return Ok(()) };
        s.scale = scale;
        s.transform = transform;
        s.source = source;
        s.destination = destination;
        if let Some(r) = p.opaque {
            s.opaque = r;
        }
        if let Some(r) = p.input {
            s.input = r;
        }
        s.applied_offset = p.offset.unwrap_or((0, 0));
        match attached {
            Attached::Buffer(_) => s.buffer_size = new_size,
            Attached::Null => s.buffer_size = None,
            Attached::Nothing => {}
        }
        if attached != Attached::Nothing {
            s.feedbacks.clear();
        }
        s.frame_callbacks.extend(p.frame_callbacks);
        s.feedbacks.extend(p.feedbacks);
        if let Some(stack) = p.stack {
            s.restack |= stack != s.stack;
            s.stack = stack;
        }
        let children = s.stack.clone();
        let view = s.view();
        let role = s.role;
        for (child, pos) in p.child_positions {
            if let Some(Sub { position, .. }) = self.surface(c, child).and_then(|cs| cs.sub.as_mut()) {
                *position = pos;
            }
        }
        // The role maps or unmaps the window...
        self.xdg_commit(c, id, attached)?;
        // ...and what the buffer shows is copied into it.
        let window = self.surface(c, id).and_then(|s| s.window);
        if let Some(win) = window {
            self.scene.set_view(win, view);
        } else if let Some(s) = self.surface(c, id) {
            s.stash.view = view;
            if attached == Attached::Null {
                s.stash = Content::default();
            }
        }
        if let Attached::Buffer(b) = attached {
            // Surface damage in buffer pixels: the new scale (a transform or
            // viewport damages all of it).
            let damage: Vec<Rect> =
                p.buffer_damage.iter().copied().chain(p.damage.iter().map(|d| d.scale(scale))).collect();
            self.copy_buffer(c, b, id, window, scale, &damage);
            self.client(c).send(b, wl_buffer::Event::Release);
        }
        // Cached state of synchronized subsurfaces goes with the parent's.
        for child in children.iter().copied().filter(|ch| *ch != id) {
            let cached = self.surface(c, child).filter(|cs| cs.sub.is_some()).and_then(|cs| cs.cached.take());
            if let Some(cached) = cached {
                self.apply(c, child, cached)?;
            }
        }
        // Subsurfaces follow: this one's place, its children's.
        if role == Role::Subsurface
            && let Some(parent) = self.surface(c, id).and_then(|s| s.sub).map(|s| s.parent).filter(|p| *p != id)
        {
            self.sync_subsurfaces(c, parent);
        }
        self.sync_subsurfaces(c, id);
        match role {
            Role::Cursor => self.cursor_committed(c, id),
            Role::DndIcon => self.dnd_icon_committed(c, id),
            Role::External => self.managed_committed(c, id),
            _ => {}
        }
        Ok(())
    }

    /// Copy the damaged part of buffer `b` into surface `surface`'s content
    /// (its window's, or its own while it has none); all of it if the size
    /// or scale changed.
    fn copy_buffer(
        &mut self,
        c: ClientId,
        b: ObjectId,
        surface: ObjectId,
        win: Option<WindowId>,
        scale: i32,
        damage: &[Rect],
    ) {
        let Some(Data::Buffer(buf)) = self.client(c).data(b) else { return };
        let buffer = super::Buffer {
            pool: Rc::clone(&buf.pool),
            offset: buf.offset,
            width: buf.width,
            height: buf.height,
            stride: buf.stride,
            format: buf.format,
            y_invert: buf.y_invert,
        };
        let (width, height) = (buffer.width, buffer.height);
        let all = Rect::new(0, 0, width, height);
        let fits = |content: &Content| {
            (content.width, content.height, content.scale) == (width as u32, height as u32, scale as u32)
        };
        let whole = buffer.y_invert;
        let rects = |resized: bool| -> Vec<Rect> {
            if resized || whole {
                alloc::vec![all]
            } else {
                damage.iter().map(|d| d.intersect(&all)).filter(|d| !d.is_empty()).collect()
            }
        };
        match win {
            Some(win) => {
                let resized = self.scene.window(win).is_none_or(|w| !fits(w.content()));
                self.scene.resize_content(win, width as u32, height as u32, scale as u32);
                let rects = rects(resized);
                if let Some(content) = self.scene.content_mut(win) {
                    for r in &rects {
                        buffer.copy_rect(*r, &mut content.pixels, width as usize);
                    }
                }
                for r in rects {
                    self.scene.damage_content(win, r);
                }
            }
            None => {
                let Some(s) = self.surface(c, surface) else { return };
                let resized = !fits(&s.stash);
                if resized {
                    let view = s.stash.view;
                    s.stash = Content {
                        width: width as u32,
                        height: height as u32,
                        scale: scale as u32,
                        pixels: alloc::vec![0; width as usize * height as usize],
                        view,
                    };
                }
                for r in rects(resized) {
                    buffer.copy_rect(r, &mut s.stash.pixels, width as usize);
                }
            }
        }
    }

    /// Show a surface's window on the output: `enter` for each of the
    /// client's `wl_output`s, and the preferred scale.
    pub(super) fn surface_entered(&mut self, c: ClientId, id: ObjectId) {
        let client = self.client(c);
        for o in client.ids_where(|d| matches!(d, Data::Output)) {
            client.send(id, wl_surface::Event::Enter { output: o });
        }
    }

    /// Give surface `id` a window of the scene showing its stashed content
    /// (undecorated), on top; returns it.
    pub(super) fn window_from_stash(&mut self, c: ClientId, id: ObjectId) -> Option<WindowId> {
        let content = core::mem::take(&mut self.surface(c, id)?.stash);
        let win = self.scene.add_window(owner_tag(c, id), false);
        self.scene.set_content(win, content);
        if let Some(s) = self.surface(c, id) {
            s.window = Some(win);
        }
        Some(win)
    }

    /// Take surface `id`'s window away, keeping its content for later.
    pub(super) fn window_to_stash(&mut self, c: ClientId, id: ObjectId) {
        let Some(win) = self.surface(c, id).and_then(|s| s.window.take()) else { return };
        let content = self.scene.window(win).map(|w| w.content().clone()).unwrap_or_default();
        self.scene.remove_window(win);
        if let Some(s) = self.surface(c, id) {
            s.stash = content;
        }
    }
}
