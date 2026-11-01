//! `wl_subcompositor` and `wl_subsurface` (wayland.xml): surfaces shown
//! as part of a parent surface — video under a toolbar, a toolkit's
//! shadows and popovers.
//!
//! - A subsurface is **mapped** when it has a buffer and its parent is
//!   mapped, in either order, recursively; it is then a window of the
//!   scene that is part of its parent's window ([`crate::compose::Scene::set_parent`]),
//!   so it moves and is raised with it, at the parent's position plus its
//!   own (`set_position`).
//! - Adding a subsurface, its position and its **stacking** among its
//!   siblings and parent (`place_above`, `place_below`) are double-buffered
//!   state of the parent, applied with the parent's next commit. A new one
//!   goes on top of its parent's stack.
//! - **Synchronized** (the initial mode) subsurfaces cache their commits
//!   until their parent's state is applied; a subsurface in a synchronized
//!   one is synchronized too. Switching to desynchronized applies what is
//!   cached.
//! - Destroying the `wl_subsurface` unmaps the surface at once; destroying
//!   the parent unmaps its subsurfaces.

use super::surface::{Role, Sub};
use super::{ClientId, Data, Res, Server, error};
use crate::protocol::{wl_subcompositor, wl_subsurface};
use crate::wire::ObjectId;
use alloc::format;
use alloc::vec::Vec;

impl Server {
    pub(super) fn subcompositor_request(&mut self, c: ClientId, sc: ObjectId, req: wl_subcompositor::Request) -> Res {
        let wl_subcompositor::Request::GetSubsurface { id, surface, parent } = req else { return Ok(()) };
        let bad_surface = |what: &str| error(sc, wl_subcompositor::Error::BAD_SURFACE.0, what);
        if surface == parent {
            return Err(error(sc, wl_subcompositor::Error::BAD_PARENT.0, "a surface cannot be its own parent"));
        }
        let Some(s) = self.surface_ref(c, surface) else { return Ok(()) };
        if !matches!(s.role, Role::None | Role::Subsurface) || s.sub.is_some_and(|sub| sub.object.is_some()) {
            return Err(bad_surface("the surface already has a role"));
        }
        // The parent must not be one of the surface's descendants.
        let mut at = parent;
        for _ in 0..64 {
            if at == surface {
                return Err(error(sc, wl_subcompositor::Error::BAD_PARENT.0, "the parent is a descendant"));
            }
            match self.surface_ref(c, at).and_then(|p| p.sub) {
                Some(sub) if sub.parent != at => at = sub.parent,
                _ => break,
            }
        }
        if let Some(s) = self.surface(c, surface) {
            s.role = Role::Subsurface;
            s.sub = Some(Sub { object: Some(id), parent, sync: true, position: (0, 0) });
        }
        // Added on top of the parent's stack, with the parent's next commit.
        if let Some(p) = self.surface(c, parent) {
            let mut stack = p.pending.stack.take().unwrap_or_else(|| p.stack.clone());
            stack.retain(|s| *s != surface);
            stack.push(surface);
            p.pending.stack = Some(stack);
        }
        self.client(c).insert(id, &wl_subsurface::INTERFACE, 1, Data::Subsurface(Some(surface)));
        Ok(())
    }

    pub(super) fn subsurface_request(&mut self, c: ClientId, id: ObjectId, req: wl_subsurface::Request) -> Res {
        // Inert once its surface is gone (the surface's ID may be another
        // surface's by now).
        let Some(Data::Subsurface(Some(surface))) = self.client(c).data(id) else { return Ok(()) };
        let surface = *surface;
        let Some(sub) = self.surface_ref(c, surface).and_then(|s| s.sub) else { return Ok(()) };
        let parent = sub.parent;
        match req {
            wl_subsurface::Request::Destroy => {
                self.hide_subsurface_tree(c, surface);
                if let Some(Sub { object, .. }) = self.surface(c, surface).and_then(|s| s.sub.as_mut()) {
                    *object = None;
                }
                if let Some(p) = self.surface(c, parent) {
                    p.stack.retain(|s| *s != surface);
                    if let Some(st) = &mut p.pending.stack {
                        st.retain(|s| *s != surface);
                    }
                }
            }
            wl_subsurface::Request::SetPosition { x, y } => {
                if let Some(p) = self.surface(c, parent) {
                    p.pending.child_positions.push((surface, (x, y)));
                }
            }
            wl_subsurface::Request::PlaceAbove { sibling } | wl_subsurface::Request::PlaceBelow { sibling } => {
                let above = matches!(req, wl_subsurface::Request::PlaceAbove { .. });
                let Some(p) = self.surface(c, parent) else { return Ok(()) };
                let mut stack = p.pending.stack.take().unwrap_or_else(|| p.stack.clone());
                if sibling == surface || !stack.contains(&sibling) {
                    p.pending.stack = Some(stack);
                    let code = wl_subsurface::Error::BAD_SURFACE.0;
                    return Err(error(id, code, format!("surface {sibling} is not a sibling or the parent")));
                }
                stack.retain(|s| *s != surface);
                let at = stack.iter().position(|s| *s == sibling).unwrap_or(0);
                stack.insert(if above { at + 1 } else { at }, surface);
                p.pending.stack = Some(stack);
            }
            wl_subsurface::Request::SetSync => {
                if let Some(Sub { sync, .. }) = self.surface(c, surface).and_then(|s| s.sub.as_mut()) {
                    *sync = true;
                }
            }
            wl_subsurface::Request::SetDesync => {
                if let Some(Sub { sync, .. }) = self.surface(c, surface).and_then(|s| s.sub.as_mut()) {
                    *sync = false;
                }
                // No longer synchronized: what is cached applies now.
                if !self.effectively_sync(c, surface) {
                    self.flush_cached(c, surface)?;
                }
            }
        }
        Ok(())
    }

    /// Apply the cached state of `surface` and of the subsurfaces in it
    /// that are no longer synchronized.
    fn flush_cached(&mut self, c: ClientId, surface: ObjectId) -> Res {
        if let Some(cached) = self.surface(c, surface).and_then(|s| s.cached.take()) {
            self.apply(c, surface, cached)?;
        }
        let children: Vec<ObjectId> = self.surface_ref(c, surface).map(|s| s.stack.clone()).unwrap_or_default();
        for child in children.into_iter().filter(|ch| *ch != surface) {
            if !self.effectively_sync(c, child) {
                self.flush_cached(c, child)?;
            }
        }
        Ok(())
    }

    /// Bring the windows of `parent`'s subsurfaces in line with the state:
    /// shown when they have content and `parent` is shown, at their
    /// position, in their stacking order; recursively.
    pub(super) fn sync_subsurfaces(&mut self, c: ClientId, parent: ObjectId) {
        let Some(p) = self.surface_ref(c, parent) else { return };
        if p.stack.len() <= 1 && !p.restack {
            return;
        }
        let (parent_win, stack, restack) = (p.window, p.stack.clone(), p.restack);
        let mut changed = restack;
        for child in stack.iter().copied().filter(|ch| *ch != parent) {
            let Some(cs) = self.surface_ref(c, child) else { continue };
            let Some(sub) = cs.sub.filter(|s| s.parent == parent && s.object.is_some()) else { continue };
            let show = parent_win.is_some() && cs.buffer_size.is_some();
            match (show, cs.window, parent_win) {
                (true, None, Some(pw)) => {
                    if let Some(win) = self.window_from_stash(c, child) {
                        self.scene.set_parent(win, Some(pw));
                        let (px, py) = self.scene.window(pw).map_or((0, 0), |w| w.position());
                        self.scene.set_position(win, px + sub.position.0, py + sub.position.1);
                        self.surface_entered(c, child);
                        changed = true;
                    }
                }
                (false, Some(_), _) => self.hide_subsurface_tree(c, child),
                (true, Some(win), Some(pw)) => {
                    let (px, py) = self.scene.window(pw).map_or((0, 0), |w| w.position());
                    self.scene.set_position(win, px + sub.position.0, py + sub.position.1);
                }
                _ => {}
            }
            self.sync_subsurfaces(c, child);
        }
        if let Some(p) = self.surface(c, parent) {
            p.restack = false;
        }
        if changed && let Some(pw) = parent_win {
            self.restack_subsurfaces(c, parent, pw, &stack);
        }
    }

    /// Put the windows of `parent`'s subsurfaces in the order of `stack`
    /// around the parent's window `pw`.
    fn restack_subsurfaces(&mut self, c: ClientId, parent: ObjectId, pw: crate::compose::WindowId, stack: &[ObjectId]) {
        let me = stack.iter().position(|s| *s == parent).unwrap_or(0);
        let window = |srv: &mut Server, s: ObjectId| srv.surface_ref(c, s).and_then(|cs| cs.window);
        let mut prev = pw;
        for s in &stack[me + 1..] {
            if let Some(w) = window(self, *s) {
                self.scene.restack_above(w, prev);
                prev = w;
            }
        }
        let mut prev = pw;
        for s in stack[..me].iter().rev() {
            if let Some(w) = window(self, *s) {
                self.scene.restack_below(w, prev);
                prev = w;
            }
        }
    }

    /// Take a subsurface and the subsurfaces in it off the output (their
    /// content kept for when they show again).
    pub(super) fn hide_subsurface_tree(&mut self, c: ClientId, surface: ObjectId) {
        let stack = self.surface_ref(c, surface).map(|s| s.stack.clone()).unwrap_or_default();
        for child in stack.into_iter().filter(|ch| *ch != surface) {
            self.hide_subsurface_tree(c, child);
        }
        if self.surface_ref(c, surface).is_some_and(|s| s.role == Role::Subsurface) {
            self.window_to_stash(c, surface);
        }
    }

    /// The top-level surface a subsurface is ultimately part of.
    pub(super) fn main_surface(&self, c: ClientId, surface: ObjectId) -> ObjectId {
        let mut at = surface;
        for _ in 0..64 {
            match self.surface_ref(c, at).and_then(|s| s.sub) {
                Some(sub) if sub.parent != at && sub.object.is_some() => at = sub.parent,
                _ => break,
            }
        }
        at
    }
}
