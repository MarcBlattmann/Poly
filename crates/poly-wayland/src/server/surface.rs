//! `wl_compositor`, `wl_surface` and `wl_region`: surfaces and their
//! double-buffered state (buffer, damage, frame callbacks, scale, regions),
//! applied on `commit`.

use super::{ClientId, Data, Res, Server, error};
use crate::compose::{Rect, WindowId};
use crate::protocol::{wl_buffer, wl_callback, wl_compositor, wl_region, wl_surface};
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
    #[allow(dead_code)]
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
}

/// The role a surface was given; permanent once set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    None,
    XdgToplevel,
    XdgPopup,
}

#[derive(Debug)]
pub(crate) struct Surface {
    pub pending: Pending,
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
}

impl Surface {
    fn new() -> Surface {
        Surface {
            pending: Pending::default(),
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
        }
    }

    /// The content's size in surface (logical) coordinates.
    pub fn logical_size(&self) -> Option<(i32, i32)> {
        self.buffer_size.map(|(w, h)| (w / self.scale.max(1), h / self.scale.max(1)))
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

    pub(super) fn compositor_request(&mut self, c: ClientId, comp: ObjectId, req: wl_compositor::Request) -> Res {
        let client = self.client(c);
        let version = client.version(comp);
        match req {
            wl_compositor::Request::CreateSurface { id } => {
                client.insert(id, &wl_surface::INTERFACE, version, Data::Surface(Box::new(Surface::new())));
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
        if let Some(w) = s.window.take() {
            self.scene.remove_window(w);
        }
        Ok(())
    }

    /// Apply the pending state.
    pub(super) fn commit(&mut self, c: ClientId, id: ObjectId) -> Res {
        self.xdg_check_commit(c, id)?;
        let client = self.client(c);
        let buffer_size = |b: ObjectId| match client.data(b) {
            Some(Data::Buffer(b)) => Some((b.width, b.height)),
            _ => None,
        };
        let Some(Data::Surface(s)) = client.objects.get(&id.0).map(|o| &o.data) else { return Ok(()) };
        let scale = s.pending.scale.unwrap_or(s.scale);
        let attached = match s.pending.buffer {
            None => Attached::Nothing,
            Some(None) => Attached::Null,
            // A buffer destroyed while pending: no content, as most do.
            Some(Some(b)) => buffer_size(b).map_or(Attached::Null, |_| Attached::Buffer(b)),
        };
        let new_size = match attached {
            Attached::Buffer(b) => buffer_size(b),
            _ => None,
        };
        if let Some((w, h)) = new_size
            && (w % scale != 0 || h % scale != 0)
        {
            let msg = format!("buffer {w}x{h} is not a multiple of scale {scale}");
            return Err(error(id, wl_surface::Error::INVALID_SIZE.0, msg));
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
        let p = core::mem::take(&mut s.pending);
        s.scale = scale;
        if let Some(t) = p.transform {
            s.transform = t;
        }
        if let Some(r) = p.opaque {
            s.opaque = r;
        }
        if let Some(r) = p.input {
            s.input = r;
        }
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
        // The role maps or unmaps the window...
        self.xdg_commit(c, id, attached)?;
        // ...and what the buffer shows is copied into it.
        let Some(s) = self.surface(c, id) else { return Ok(()) };
        if let (Attached::Buffer(b), Some(win)) = (attached, s.window) {
            // Surface damage in buffer pixels: the new scale (transforms
            // other than normal are not applied yet).
            let damage: Vec<Rect> =
                p.buffer_damage.iter().copied().chain(p.damage.iter().map(|d| d.scale(scale))).collect();
            self.copy_buffer(c, b, win, scale, &damage);
        }
        if let Attached::Buffer(b) = attached {
            self.client(c).send(b, wl_buffer::Event::Release);
        }
        Ok(())
    }

    /// Copy the damaged part of buffer `b` into window `win`'s content (all
    /// of it if the size or scale changed).
    fn copy_buffer(&mut self, c: ClientId, b: ObjectId, win: WindowId, scale: i32, damage: &[Rect]) {
        let Some(Data::Buffer(buf)) = self.client(c).data(b) else { return };
        let (pool, offset, width, height, stride, format) =
            (Rc::clone(&buf.pool), buf.offset, buf.width, buf.height, buf.stride, buf.format);
        let buffer = super::Buffer { pool, offset, width, height, stride, format };
        let resized = self.scene.window(win).is_none_or(|w| {
            let content = w.content();
            (content.width, content.height, content.scale) != (width as u32, height as u32, scale as u32)
        });
        self.scene.resize_content(win, width as u32, height as u32, scale as u32);
        let all = Rect::new(0, 0, width, height);
        let rects: Vec<Rect> = if resized {
            alloc::vec![all]
        } else {
            damage.iter().map(|d| d.intersect(&all)).filter(|d| !d.is_empty()).collect()
        };
        if let Some(content) = self.scene.content_mut(win) {
            for r in &rects {
                buffer.copy_rect(*r, &mut content.pixels, width as usize);
            }
        }
        for r in rects {
            self.scene.damage_content(win, r);
        }
    }

    /// Show a surface's window on the output: `enter` for each of the
    /// client's `wl_output`s.
    pub(super) fn surface_entered(&mut self, c: ClientId, id: ObjectId) {
        let client = self.client(c);
        for o in client.ids_where(|d| matches!(d, Data::Output)) {
            client.send(id, wl_surface::Event::Enter { output: o });
        }
    }
}
