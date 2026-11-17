//! Pictures handed to the compositor instead of copied (ADR 0224).
//!
//! A DXGI swap chain presents into a window (Chrome's GPU process: a child window of the browser's
//! top-level window). Its buffers are sections of the presenting process, and a present names the
//! section that holds the frame (`PolyUserPresentBuffer`). Here a window and a process together
//! are one *layer* of the surface the window draws into: its top-level window's surface, at the
//! window's client area there. Each section the layer presented becomes a buffer of the layer that
//! the compositor maps (the section's pages, [`crate::link::layer_buffer_offset`]), and a present
//! is a damage record: no pixel is copied by the app or by win32k.
//!
//! The objects are numbers here (the NT personality's object ids and the kernel's backing ids of
//! the sections); the personality holds a reference on each section object a layer adopted and
//! drops it when [`Layers`] says the buffer went.

use crate::desktop::Desktop;
use crate::link::{BufferId, LayerId, SurfaceId};
use crate::surfaces::Surfaces;
use crate::wm::Wm;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// The most buffers one layer keeps (a swap chain of up to 16 buffers cycles through more, and the
/// oldest is let go, to be adopted again when presented again).
pub const MAX_BUFFERS: usize = 4;

/// A section a process presented: its object (the personality's id), the kernel's backing id of
/// its pages, and the picture in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presented {
    pub object: u64,
    pub backing: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Buf {
    section: Presented,
    id: BufferId,
}

#[derive(Debug)]
struct Entry {
    layer: LayerId,
    surface: SurfaceId,
    bufs: Vec<Buf>,
    current: Option<BufferId>,
}

/// What a present did to the section objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The presented section is new to the layer: the personality keeps a reference on it.
    pub adopted: bool,
    /// Section objects the layer let go of: their references are dropped.
    pub dropped: Vec<u64>,
}

/// Every layer, by window and process.
#[derive(Debug, Default)]
pub struct Layers {
    entries: BTreeMap<(u32, u32), Entry>,
    next: LayerId,
}

/// Where a window's client area is in a surface: the surface and its size, and the client
/// area's top-left corner in it and size.
struct Where {
    surface: SurfaceId,
    size: (u32, u32),
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// Where window `hwnd`'s client area is. A top-level window with a surface is its own surface; a
/// child window is in its top-level window's.
fn place(surfaces: &Surfaces, wm: &Wm, desktop: &Desktop, hwnd: u32) -> Option<Where> {
    if let (Some(surface), Some((w, h))) = (surfaces.surface(hwnd), surfaces.size(desktop, hwnd)) {
        return Some(Where { surface, size: (w, h), x: 0, y: 0, width: w, height: h });
    }
    let a = surfaces.child_area(wm, desktop, hwnd)?;
    Some(Where { surface: surfaces.surface(a.top)?, size: a.surface, x: a.x, y: a.y, width: a.width, height: a.height })
}

/// `a`'s part inside `0 .. limit`, from `at` on for `len`: (start, length).
fn span(at: i32, len: u32, limit: u32) -> (u32, u32) {
    let start = at.clamp(0, limit as i32);
    let end = (i64::from(at) + i64::from(len)).clamp(0, i64::from(limit)) as i32;
    (start as u32, (end - start).max(0) as u32)
}

impl Layers {
    pub fn new() -> Layers {
        Layers { entries: BTreeMap::new(), next: 1 }
    }

    /// Process `pid` presented `section` into window `hwnd`; `rects` (left, top, right, bottom,
    /// buffer pixels) changed, all of it when there are none. `None` when the window shows
    /// nowhere (no surface, hidden, gone): nothing is adopted.
    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        surfaces: &Surfaces,
        wm: &Wm,
        desktop: &mut Desktop,
        hwnd: u32,
        pid: u32,
        section: Presented,
        rects: &[(i32, i32, i32, i32)],
    ) -> Option<Outcome> {
        let Where { surface, size: (sw, sh), x, y, width: cw, height: ch } = place(surfaces, wm, desktop, hwnd)?;
        let mut out = Outcome::default();
        let next = &mut self.next;
        let e = self.entries.entry((hwnd, pid)).or_insert_with(|| {
            let layer = *next;
            *next += 1;
            Entry { layer, surface, bufs: Vec::new(), current: None }
        });
        // The window's surface changed (its top-level window was shown again, or it moved to
        // another one): the layer starts again there with the buffers it has.
        if e.surface != surface {
            desktop.remove_layer(e.surface, e.layer);
            e.surface = surface;
            e.current = None;
            for b in &mut e.bufs {
                let (w, h, st) = (b.section.width, b.section.height, b.section.stride);
                b.id = desktop.layer_buffer(surface, e.layer, w, h, st)?;
            }
        }
        let same = |b: &Buf| b.section == section;
        if !e.bufs.iter().any(same) {
            out.adopted = !e.bufs.iter().any(|b| b.section.object == section.object);
            // A buffer of another size (the swap chain was resized), the same object with another
            // picture, and the oldest past the limit go.
            let mut keep = Vec::new();
            for b in core::mem::take(&mut e.bufs) {
                let stale = (b.section.width, b.section.height) != (section.width, section.height)
                    || b.section.object == section.object;
                if stale {
                    desktop.drop_layer_buffer(surface, e.layer, b.id);
                    if b.section.object != section.object {
                        out.dropped.push(b.section.object);
                    }
                } else {
                    keep.push(b);
                }
            }
            while keep.len() >= MAX_BUFFERS {
                let i = keep.iter().position(|b| Some(b.id) != e.current).unwrap_or(0);
                let b = keep.remove(i);
                desktop.drop_layer_buffer(surface, e.layer, b.id);
                out.dropped.push(b.section.object);
            }
            e.bufs = keep;
            let id = desktop.layer_buffer(surface, e.layer, section.width, section.height, section.stride)?;
            e.bufs.push(Buf { section, id });
        }
        let id = e.bufs.iter().find(|b| same(b))?.id;
        // Shown inside the window's client area and the surface.
        let (cx, cwid) = span(x, cw.min(section.width), sw);
        let (cy, chgt) = span(y, ch.min(section.height), sh);
        desktop.place_layer(surface, e.layer, x, y, (cx, cy, cwid, chgt));
        e.current = Some(id);
        if rects.is_empty() {
            desktop.damage_layer(surface, e.layer, id, (0, 0, section.width, section.height));
        }
        for &(l, t, r, b) in rects {
            let (l, t) = (l.max(0) as u32, t.max(0) as u32);
            let (r, b) = (r.max(0) as u32, b.max(0) as u32);
            if r > l && b > t {
                desktop.damage_layer(surface, e.layer, id, (l, t, r - l, b - t));
            }
        }
        Some(out)
    }

    /// Process `pid`'s layer of window `hwnd` goes (its swap chain went): the section objects to
    /// let go of.
    pub fn remove(&mut self, desktop: &mut Desktop, hwnd: u32, pid: u32) -> Vec<u64> {
        let Some(e) = self.entries.remove(&(hwnd, pid)) else { return Vec::new() };
        desktop.remove_layer(e.surface, e.layer);
        e.bufs.iter().map(|b| b.section.object).collect()
    }

    /// Process `pid` ended: its layers go.
    pub fn remove_process(&mut self, desktop: &mut Desktop, pid: u32) -> Vec<u64> {
        let keys: Vec<(u32, u32)> = self.entries.keys().filter(|k| k.1 == pid).copied().collect();
        keys.into_iter().flat_map(|(h, p)| self.remove(desktop, h, p)).collect()
    }

    /// The kernel's backing id of buffer `id`'s pages, for the compositor's mapping.
    pub fn backing(&self, id: BufferId) -> Option<u64> {
        self.entries.values().flat_map(|e| &e.bufs).find(|b| b.id == id).map(|b| b.section.backing)
    }

    /// How many layers there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
