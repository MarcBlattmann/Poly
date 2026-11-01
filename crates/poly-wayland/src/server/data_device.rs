//! `wl_data_device_manager`, `wl_data_source`, `wl_data_device` and
//! `wl_data_offer` (wayland.xml): the clipboard and drag and drop between
//! clients.
//!
//! - **Selection.** `set_selection` makes a source the seat's selection
//!   (the one it replaces hears `cancelled`). The client with keyboard
//!   focus gets it as a new `wl_data_offer` (`data_offer`, an `offer` per
//!   MIME type, then `selection`), when it is set and whenever a client
//!   gets the focus (before `wl_keyboard.enter`). `receive` on an offer
//!   hands the client's pipe to the source's client as `send`: the bytes go
//!   from one client to the other directly, the compositor only passes the
//!   descriptor on.
//! - **Drag and drop.** `start_drag` during an implicit pointer grab on the
//!   origin surface starts a drag: the pointer leaves its focus, the
//!   surface under the pointer gets `enter` with a new offer (MIME types,
//!   `source_actions`), `motion`, `leave`; `accept` is passed to the source
//!   as `target`, `set_actions` picks the action both sides allow (the
//!   destination's preference first) and tells both. Releasing the button
//!   drops (`drop`, `dnd_drop_performed`) if the destination accepted a
//!   type and an action, else cancels (`leave`, `cancelled`); `finish`
//!   ends it (`dnd_finished`). The drag icon is a surface drawn at the
//!   pointer. A drag without a source stays within its client.

use super::input::{Focus, Grab};
use super::surface::Role;
use super::{ClientId, Data, Res, Server, error};
use crate::compose::{Hit, WindowId};
use crate::protocol::{wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source};
use crate::wire::{Fd, ObjectId};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

const COPY: u32 = wl_data_device_manager::DndAction::COPY.0;
const MOVE: u32 = wl_data_device_manager::DndAction::MOVE.0;
const ASK: u32 = wl_data_device_manager::DndAction::ASK.0;
const ALL_ACTIONS: u32 = COPY | MOVE | ASK;

#[derive(Debug, Default)]
pub(crate) struct DataSource {
    pub mime_types: Vec<String>,
    /// `set_actions` (version 3), if called.
    pub actions: Option<u32>,
    /// Given to `set_selection` or `start_drag` (a source is used once).
    pub used: bool,
}

#[derive(Debug, Default)]
pub(crate) struct DataOffer {
    /// The source it offers, while it is there.
    pub source: Option<(ClientId, ObjectId)>,
    pub dnd: bool,
    pub accepted: Option<String>,
    pub actions: u32,
    pub preferred: u32,
    /// The action chosen for a drag.
    pub action: u32,
    pub dropped: bool,
}

/// A drag in progress.
#[derive(Debug)]
pub(crate) struct Drag {
    pub client: ClientId,
    pub source: Option<ObjectId>,
    pub icon: Option<ObjectId>,
    pub icon_window: Option<WindowId>,
    /// The surface under the pointer and the offers made to it.
    pub focus: Option<Focus>,
    pub offers: Vec<ObjectId>,
}

/// The seat's clipboard and drag.
#[derive(Debug, Default)]
pub(crate) struct DataState {
    pub selection: Option<(ClientId, ObjectId)>,
    pub drag: Option<Drag>,
}

/// The action a drag does: the destination's preference if both allow it,
/// else the first both allow (copy, move, ask), else none.
pub(crate) fn choose_action(source: u32, destination: u32, preferred: u32) -> u32 {
    let both = source & destination & ALL_ACTIONS;
    if preferred & both != 0 {
        return preferred & both;
    }
    [COPY, MOVE, ASK].into_iter().find(|a| both & a != 0).unwrap_or(0)
}

impl Server {
    pub(super) fn data_manager_request(
        &mut self,
        c: ClientId,
        manager: ObjectId,
        req: wl_data_device_manager::Request,
    ) -> Res {
        let client = self.client(c);
        let version = client.version(manager);
        match req {
            wl_data_device_manager::Request::CreateDataSource { id } => {
                let source = Box::new(DataSource::default());
                client.insert(id, &wl_data_source::INTERFACE, version, Data::DataSource(source));
            }
            wl_data_device_manager::Request::GetDataDevice { id, .. } => {
                client.insert(id, &wl_data_device::INTERFACE, version, Data::DataDevice);
                if self.keyboard_focus().is_some_and(|(fc, _)| fc == c) {
                    self.send_selection(c, Some(id));
                }
            }
            wl_data_device_manager::Request::Release => {}
        }
        Ok(())
    }

    fn data_source(&mut self, c: ClientId, id: ObjectId) -> Option<&mut DataSource> {
        match self.clients.get_mut(&c)?.data_mut(id) {
            Some(Data::DataSource(s)) => Some(s),
            _ => None,
        }
    }

    fn data_offer(&mut self, c: ClientId, id: ObjectId) -> Option<&mut DataOffer> {
        match self.clients.get_mut(&c)?.data_mut(id) {
            Some(Data::DataOffer(o)) => Some(o),
            _ => None,
        }
    }

    pub(super) fn data_source_request(&mut self, c: ClientId, id: ObjectId, req: wl_data_source::Request) -> Res {
        let Some(src) = self.data_source(c, id) else { return Ok(()) };
        match req {
            wl_data_source::Request::Offer { mime_type } => {
                if !src.mime_types.contains(&mime_type) {
                    src.mime_types.push(mime_type);
                }
            }
            wl_data_source::Request::SetActions { dnd_actions } => {
                if dnd_actions & !ALL_ACTIONS != 0 {
                    let code = wl_data_source::Error::INVALID_ACTION_MASK.0;
                    return Err(error(id, code, alloc::format!("actions {dnd_actions:#x}")));
                }
                if src.used {
                    let code = wl_data_source::Error::INVALID_SOURCE.0;
                    return Err(error(id, code, "set_actions after the source was used"));
                }
                src.actions = Some(dnd_actions);
            }
            wl_data_source::Request::Destroy => self.data_source_gone(c, id),
        }
        Ok(())
    }

    /// A source went away: no longer the selection, its drag cancelled,
    /// its offers inert.
    fn data_source_gone(&mut self, c: ClientId, id: ObjectId) {
        for client in self.clients.values_mut() {
            for o in client.objects.values_mut() {
                if let Data::DataOffer(offer) = &mut o.data
                    && offer.source == Some((c, id))
                {
                    offer.source = None;
                }
            }
        }
        if self.data.selection == Some((c, id)) {
            self.data.selection = None;
            if let Some((fc, _)) = self.keyboard_focus() {
                self.send_selection(fc, None);
            }
        }
        if self.data.drag.as_ref().is_some_and(|d| d.client == c && d.source == Some(id)) {
            if let Some(d) = self.data.drag.as_mut() {
                d.source = None;
            }
            self.drag_leave();
            self.end_drag();
        }
    }

    pub(super) fn data_device_request(&mut self, c: ClientId, device: ObjectId, req: wl_data_device::Request) -> Res {
        match req {
            wl_data_device::Request::SetSelection { source, .. } => {
                if let Some(s) = source {
                    let Some(src) = self.data_source(c, s) else { return Ok(()) };
                    if src.used {
                        let code = wl_data_device::Error::USED_SOURCE.0;
                        return Err(error(device, code, "the source was used already"));
                    }
                    src.used = true;
                }
                let new = source.map(|s| (c, s));
                let old = core::mem::replace(&mut self.data.selection, new);
                if let Some((oc, os)) = old
                    && old != new
                    && let Some(client) = self.clients.get_mut(&oc)
                {
                    client.send(os, wl_data_source::Event::Cancelled);
                }
                if let Some((fc, _)) = self.keyboard_focus() {
                    self.send_selection(fc, None);
                }
            }
            wl_data_device::Request::StartDrag { source, origin, icon, .. } => {
                return self.start_drag(c, device, source, origin, icon);
            }
            wl_data_device::Request::Release => {}
        }
        Ok(())
    }

    /// A new offer object of client `c` for `device`, describing `source`
    /// (`data_offer`, then an `offer` per MIME type and, for a drag,
    /// `source_actions`).
    fn make_offer(
        &mut self,
        c: ClientId,
        device: ObjectId,
        source: Option<(ClientId, ObjectId)>,
        dnd: bool,
    ) -> ObjectId {
        let (mimes, actions) = match source {
            Some((sc, s)) => match self.clients.get(&sc).and_then(|cl| cl.data(s)) {
                Some(Data::DataSource(src)) => (src.mime_types.clone(), src.actions.unwrap_or(COPY)),
                _ => (Vec::new(), 0),
            },
            None => (Vec::new(), 0),
        };
        let client = self.client(c);
        let version = client.version(device);
        let id = client.new_server_id();
        let offer = DataOffer { source, dnd, ..DataOffer::default() };
        client.insert(id, &wl_data_offer::INTERFACE, version, Data::DataOffer(Box::new(offer)));
        client.send(device, wl_data_device::Event::DataOffer { id });
        for mime_type in mimes {
            client.send(id, wl_data_offer::Event::Offer { mime_type });
        }
        if dnd {
            client.send(id, wl_data_offer::Event::SourceActions { source_actions: actions });
        }
        id
    }

    /// Tell client `c`'s data devices (or only `only`) what the selection
    /// is: a new offer, or none.
    pub(super) fn send_selection(&mut self, c: ClientId, only: Option<ObjectId>) {
        let devices = match only {
            Some(d) => alloc::vec![d],
            None => self.client(c).ids_where(|d| matches!(d, Data::DataDevice)),
        };
        let selection = self.data.selection;
        for device in devices {
            let id = selection.map(|src| self.make_offer(c, device, Some(src), false));
            self.client(c).send(device, wl_data_device::Event::Selection { id });
        }
    }

    pub(super) fn data_offer_request(&mut self, c: ClientId, id: ObjectId, req: wl_data_offer::Request) -> Res {
        let version = self.client(c).version(id);
        let Some(offer) = self.data_offer(c, id) else {
            if let wl_data_offer::Request::Receive { fd, .. } = req {
                self.host().close_fd(fd);
            }
            return Ok(());
        };
        match req {
            wl_data_offer::Request::Receive { mime_type, fd } => {
                let source = offer.source;
                self.data_send(source, mime_type, fd);
            }
            wl_data_offer::Request::Accept { mime_type, .. } => {
                offer.accepted = mime_type.clone();
                if let Some((sc, s)) = offer.source.filter(|_| offer.dnd)
                    && let Some(client) = self.clients.get_mut(&sc)
                {
                    client.send(s, wl_data_source::Event::Target { mime_type });
                }
            }
            wl_data_offer::Request::SetActions { dnd_actions, preferred_action } => {
                if dnd_actions & !ALL_ACTIONS != 0 {
                    let code = wl_data_offer::Error::INVALID_ACTION_MASK.0;
                    return Err(error(id, code, alloc::format!("actions {dnd_actions:#x}")));
                }
                if preferred_action.count_ones() > 1 || preferred_action & !ALL_ACTIONS != 0 {
                    let code = wl_data_offer::Error::INVALID_ACTION.0;
                    return Err(error(id, code, alloc::format!("preferred action {preferred_action:#x}")));
                }
                if !offer.dnd {
                    let code = wl_data_offer::Error::INVALID_OFFER.0;
                    return Err(error(id, code, "set_actions on a selection offer"));
                }
                offer.actions = dnd_actions;
                offer.preferred = preferred_action;
                self.update_action(c, id);
            }
            wl_data_offer::Request::Finish => {
                if !offer.dropped || (version >= 3 && offer.action == 0) || offer.accepted.is_none() {
                    let code = wl_data_offer::Error::INVALID_FINISH.0;
                    return Err(error(id, code, "finish before a drop with an accepted type and action"));
                }
                if let Some((sc, s)) = offer.source.take()
                    && let Some(client) = self.clients.get_mut(&sc)
                {
                    client.send(s, wl_data_source::Event::DndFinished);
                }
            }
            wl_data_offer::Request::Destroy => {}
        }
        Ok(())
    }

    /// Ask the source to write `mime_type` into `fd` (or close it if the
    /// source is gone).
    fn data_send(&mut self, source: Option<(ClientId, ObjectId)>, mime_type: String, fd: Fd) {
        let alive = source.filter(|(sc, s)| self.clients.get(sc).is_some_and(|cl| cl.get(*s).is_some()));
        match alive {
            Some((sc, s)) => self.client(sc).send(s, wl_data_source::Event::Send { mime_type, fd }),
            None => self.host().close_fd(fd),
        }
    }

    /// The drag's action after a `set_actions`: both sides hear it.
    fn update_action(&mut self, c: ClientId, offer_id: ObjectId) {
        let Some(offer) = self.data_offer(c, offer_id) else { return };
        let (source, actions, preferred) = (offer.source, offer.actions, offer.preferred);
        let source_actions = source
            .and_then(|(sc, s)| match self.clients.get(&sc)?.data(s) {
                Some(Data::DataSource(src)) => Some(src.actions.unwrap_or(COPY)),
                _ => None,
            })
            .unwrap_or(0);
        let action = choose_action(source_actions, actions, preferred);
        if let Some(offer) = self.data_offer(c, offer_id) {
            offer.action = action;
        }
        self.client(c).send(offer_id, wl_data_offer::Event::Action { dnd_action: action });
        if let Some((sc, s)) = source
            && let Some(client) = self.clients.get_mut(&sc)
        {
            client.send(s, wl_data_source::Event::Action { dnd_action: action });
        }
    }

    fn start_drag(
        &mut self,
        c: ClientId,
        device: ObjectId,
        source: Option<ObjectId>,
        origin: ObjectId,
        icon: Option<ObjectId>,
    ) -> Res {
        if let Some(i) = icon {
            let Some(s) = self.surface(c, i) else { return Ok(()) };
            if !matches!(s.role, Role::None | Role::DndIcon) {
                let code = wl_data_device::Error::ROLE.0;
                return Err(error(device, code, "the icon surface has another role"));
            }
            s.role = Role::DndIcon;
        }
        if let Some(s) = source {
            let Some(src) = self.data_source(c, s) else { return Ok(()) };
            if src.used {
                let code = wl_data_device::Error::USED_SOURCE.0;
                return Err(error(device, code, "the source was used already"));
            }
            src.used = true;
        }
        // Only during an implicit grab on the origin.
        let grabbing = !self.seat.input.buttons.is_empty()
            && self.seat.input.grab.is_none()
            && self
                .seat
                .input
                .pointer_focus
                .is_some_and(|f| f.client == c && self.main_surface(c, f.surface) == self.main_surface(c, origin));
        if !grabbing || self.data.drag.is_some() {
            if let Some(s) = source {
                self.client(c).send(s, wl_data_source::Event::Cancelled);
            }
            return Ok(());
        }
        self.set_pointer_focus(None);
        self.seat.input.grab = Some(Grab::Drag);
        self.data.drag = Some(Drag { client: c, source, icon, icon_window: None, focus: None, offers: Vec::new() });
        if let Some(i) = icon {
            self.dnd_icon_committed(c, i);
        }
        self.drag_motion(0);
        Ok(())
    }

    /// The pointer moved during a drag: the icon follows, the surface under
    /// it hears `enter`/`motion`/`leave`.
    pub(super) fn drag_motion(&mut self, time: u32) {
        let (x, y) = (self.seat.pointer.0.floor(), self.seat.pointer.1.floor());
        let Some(drag) = &self.data.drag else { return };
        let (dc, has_source, icon_window) = (drag.client, drag.source.is_some(), drag.icon_window);
        if let Some(w) = icon_window {
            self.scene.set_position(w, x, y);
            self.scene.raise(w);
        }
        // The icon takes no input: what is under it does.
        let target = match self.scene.hit_test_where(x, y, |w, _, _| Some(w.id()) != icon_window) {
            Hit::Surface { window, .. } => self.focus_of(window),
            _ => None,
        }
        // A drag without a source stays within its client.
        .filter(|f| has_source || f.client == dc);
        let current = self.data.drag.as_ref().and_then(|d| d.focus);
        if target.map(|f| (f.client, f.surface)) != current.map(|f| (f.client, f.surface)) {
            self.drag_leave();
            if let Some(f) = target {
                self.drag_enter(f);
            }
            return;
        }
        if let Some(f) = target {
            let (sx, sy) = self.local(f.window);
            for d in self.client(f.client).ids_where(|d| matches!(d, Data::DataDevice)) {
                self.client(f.client).send(d, wl_data_device::Event::Motion { time, x: sx, y: sy });
            }
        }
    }

    fn drag_enter(&mut self, f: Focus) {
        let Some(drag) = &self.data.drag else { return };
        let source = drag.source.map(|s| (drag.client, s));
        let serial = self.next_serial();
        let (sx, sy) = self.local(f.window);
        let mut offers = Vec::new();
        for d in self.client(f.client).ids_where(|d| matches!(d, Data::DataDevice)) {
            let id = source.map(|src| self.make_offer(f.client, d, Some(src), true));
            offers.extend(id);
            let enter = wl_data_device::Event::Enter { serial, surface: f.surface, x: sx, y: sy, id };
            self.client(f.client).send(d, enter);
        }
        if let Some(drag) = &mut self.data.drag {
            drag.focus = Some(f);
            drag.offers = offers;
        }
    }

    /// The drag leaves the surface it was over (its offers go inert).
    fn drag_leave(&mut self) {
        let Some(drag) = &mut self.data.drag else { return };
        let (focus, offers) = (drag.focus.take(), core::mem::take(&mut drag.offers));
        let Some(f) = focus else { return };
        if !self.clients.contains_key(&f.client) {
            return;
        }
        for o in offers {
            if let Some(offer) = self.data_offer(f.client, o) {
                offer.source = None;
            }
        }
        for d in self.client(f.client).ids_where(|d| matches!(d, Data::DataDevice)) {
            self.client(f.client).send(d, wl_data_device::Event::Leave);
        }
    }

    /// The button was released: drop on the surface under the pointer if it
    /// accepted, else cancel.
    pub(super) fn drag_drop(&mut self) {
        let Some(drag) = &self.data.drag else { return };
        let (dc, source, focus, offers) = (drag.client, drag.source, drag.focus, drag.offers.clone());
        let accepted = focus.and_then(|f| {
            offers.iter().copied().find(|o| {
                let version = self.clients.get(&f.client).map_or(0, |cl| cl.version(*o));
                match self.clients.get(&f.client).and_then(|cl| cl.data(*o)) {
                    Some(Data::DataOffer(offer)) => offer.accepted.is_some() && (offer.action != 0 || version < 3),
                    _ => false,
                }
            })
        });
        match (focus, accepted) {
            (Some(f), Some(offer)) => {
                if let Some(o) = self.data_offer(f.client, offer) {
                    o.dropped = true;
                }
                for d in self.client(f.client).ids_where(|d| matches!(d, Data::DataDevice)) {
                    self.client(f.client).send(d, wl_data_device::Event::Drop);
                }
                if let Some(s) = source {
                    self.client(dc).send(s, wl_data_source::Event::DndDropPerformed);
                }
                // The offer stays usable for `receive` and `finish`.
                if let Some(drag) = &mut self.data.drag {
                    drag.offers.clear();
                }
            }
            (None, Some(_)) => {}
            _ => {
                self.drag_leave();
                if let Some(s) = source
                    && let Some(client) = self.clients.get_mut(&dc)
                {
                    client.send(s, wl_data_source::Event::Cancelled);
                }
            }
        }
        self.end_drag();
    }

    /// The drag is over: the icon goes.
    fn end_drag(&mut self) {
        let Some(drag) = self.data.drag.take() else { return };
        if let (Some(icon), true) = (drag.icon, self.clients.contains_key(&drag.client)) {
            self.window_to_stash(drag.client, icon);
        } else if let Some(w) = drag.icon_window {
            self.scene.remove_window(w);
        }
        if matches!(self.seat.input.grab, Some(Grab::Drag)) {
            self.seat.input.grab = None;
        }
    }

    /// The drag icon was committed: shown at the pointer while the drag
    /// lasts.
    pub(super) fn dnd_icon_committed(&mut self, c: ClientId, icon: ObjectId) {
        let Some(drag) = &self.data.drag else { return };
        if drag.client != c || drag.icon != Some(icon) {
            return;
        }
        let has_content = self.surface_ref(c, icon).is_some_and(|s| s.buffer_size.is_some());
        let window = self.surface_ref(c, icon).and_then(|s| s.window);
        let window = match (has_content, window) {
            (true, None) => self.window_from_stash(c, icon),
            (false, Some(_)) => {
                self.window_to_stash(c, icon);
                None
            }
            (_, w) => w,
        };
        if let Some(d) = &mut self.data.drag {
            d.icon_window = window;
        }
        if let Some(w) = window {
            let (x, y) = (self.seat.pointer.0.floor(), self.seat.pointer.1.floor());
            self.scene.set_position(w, x, y);
            self.scene.raise(w);
        }
    }

    /// Client `c` left: its selection and drag go.
    pub(super) fn data_client_gone(&mut self, c: ClientId) {
        if self.data.selection.is_some_and(|(sc, _)| sc == c) {
            self.data.selection = None;
            if let Some((fc, _)) = self.keyboard_focus().filter(|(fc, _)| *fc != c) {
                self.send_selection(fc, None);
            }
        }
        if let Some(drag) = &self.data.drag {
            if drag.client == c {
                if let Some(w) = drag.icon_window {
                    self.scene.remove_window(w);
                }
                if let Some(d) = &mut self.data.drag {
                    d.icon_window = None;
                    d.source = None;
                }
                self.drag_leave();
                self.end_drag();
            } else if drag.focus.is_some_and(|f| f.client == c)
                && let Some(d) = &mut self.data.drag
            {
                d.focus = None;
                d.offers.clear();
            }
        }
    }

    /// Whether a drag is going on.
    pub fn is_dragging(&self) -> bool {
        self.data.drag.is_some()
    }
}
