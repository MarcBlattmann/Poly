//! The X11 window manager Xwayland gets on its `-wm` descriptor
//! (Xwayland(1)): an ordinary X client that redirects the root window's
//! children, as the ICCCM describes a window manager, and tells the
//! compositor what the X windows are so it can show their surfaces.
//!
//! It is a state machine without I/O: [`Wm::feed`] takes the bytes read
//! from the descriptor, [`Wm::take_output`] the requests to write, and
//! [`Wm::take_actions`] what the compositor must do. The compositor tells
//! it what the user did with [`Wm::activate`], [`Wm::close`] and
//! [`Wm::move_to`], and answers placement questions through [`Policy`].
//!
//! # Start
//!
//! After the setup it interns its atoms ([`Atoms`]), selects
//! `SubstructureRedirect`, `SubstructureNotify` and `PropertyChange` on the
//! root (only one client may redirect: a `BadAccess` error means another
//! window manager runs), and makes its check window (EWMH
//! `_NET_SUPPORTING_WM_CHECK`, named "Poly" by `_NET_WM_NAME`, on the root
//! and on itself). Once the atoms are known it sets `_NET_SUPPORTED`, owns
//! the `WM_S0` and `_NET_WM_CM_S0` manager selections (ICCCM 2.8, EWMH
//! "Compositing Managers"), and handles the events that came meanwhile.
//!
//! # Windows
//!
//! - **Created** (`CreateNotify` of a root child): it watches the window's
//!   properties (`PropertyChange`) and reads those it uses: `WM_NAME` and
//!   `_NET_WM_NAME` (the title, the latter preferred), `WM_CLASS`,
//!   `WM_PROTOCOLS`, `WM_HINTS` (the input model), `WM_NORMAL_HINTS` (size
//!   limits), `WM_TRANSIENT_FOR`, `_NET_WM_WINDOW_TYPE`, `_MOTIF_WM_HINTS`
//!   and `_NET_WM_PID`; again whenever one changes.
//! - **Map request**: the window becomes managed. [`Policy::place`] says
//!   where its content goes (the compositor draws the frame around it);
//!   the size is kept within `WM_NORMAL_HINTS` and the border made 0;
//!   `WM_STATE` becomes `NormalState` and `_NET_FRAME_EXTENTS` the frame's
//!   size; then it is mapped and listed in `_NET_CLIENT_LIST`.
//! - **Configure request**: the size within the hints, the position only
//!   for a window not managed yet (a managed window's position is the
//!   compositor's); a request that changes nothing is answered with a
//!   synthetic `ConfigureNotify` (ICCCM 4.1.5).
//! - **Override-redirect** windows (menus, tooltips) are not managed: the
//!   compositor shows them where the client put them, unframed.
//! - **Unmap** (real, or a client's synthetic one to withdraw, ICCCM
//!   4.1.4): `WM_STATE` becomes `WithdrawnState`; **destroy**: forgotten.
//! - **Surfaces**: Xwayland names each window's `wl_surface` with a client
//!   message, `WL_SURFACE_SERIAL` (the serial `xwayland_surface_v1`
//!   commits, low 32 bits in `l[0]`, high in `l[1]`) or the older
//!   `WL_SURFACE_ID` (the surface's object ID): [`Action::Associate`].
//! - **Client requests**: `_NET_ACTIVE_WINDOW` ([`Action::Activate`]) and
//!   `_NET_WM_MOVERESIZE` with `_NET_WM_MOVERESIZE_MOVE` ([`Action::Move`]).
//!
//! # Focus and closing
//!
//! [`Wm::activate`] follows the ICCCM input models (4.1.7): `SetInputFocus`
//! unless `WM_HINTS` says the client takes no input, and `WM_TAKE_FOCUS`
//! for clients in that protocol; the window is raised in X's stacking
//! order and named in `_NET_ACTIVE_WINDOW`. [`Wm::close`] sends
//! `WM_DELETE_WINDOW` to clients that take part in it, and kills the
//! client (`KillClient`) of a window that does not.

use crate::atoms::{Atoms, predefined};
use crate::conn::{Broken, Connection, Incoming};
use crate::event::{Event, client_message32, data32, error_code};
use crate::icccm::{self, SizeHints};
use crate::request::{Request, class, composite, config, cw, event_mask, prop_mode, revert, stack};
use crate::{CURRENT_TIME, Xid};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// `_NET_WM_MOVERESIZE` directions (EWMH).
const MOVERESIZE_MOVE: u32 = 8;

/// How a window's `wl_surface` is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRef {
    /// The `xwayland_surface_v1` serial (`WL_SURFACE_SERIAL`).
    Serial(u64),
    /// The surface's object ID on Xwayland's connection (`WL_SURFACE_ID`).
    Id(u32),
}

/// What the window manager knows of an X top-level window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub id: Xid,
    /// Position and size in root coordinates (the border is not counted).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub override_redirect: bool,
    /// Mapped (a `MapNotify` came, no `UnmapNotify` since).
    pub mapped: bool,
    /// Mapped by the window manager after a map request.
    pub managed: bool,
    pub class: (String, String),
    pub transient_for: Option<Xid>,
    pub window_type: Vec<Xid>,
    pub protocols: Vec<Xid>,
    /// `WM_HINTS`' input field.
    pub input: bool,
    /// `_MOTIF_WM_HINTS`' decorations, if it says.
    pub decorations: Option<bool>,
    pub size_hints: SizeHints,
    pub pid: Option<u32>,
    wm_name: Option<String>,
    net_wm_name: Option<String>,
}

impl Window {
    fn new(id: Xid) -> Window {
        Window { id, input: true, ..Window::default() }
    }

    /// The title: `_NET_WM_NAME`, else `WM_NAME`, else nothing.
    pub fn title(&self) -> &str {
        self.net_wm_name.as_deref().or(self.wm_name.as_deref()).unwrap_or("")
    }

    /// Whether the window gets Poly's frame: not override-redirect, no
    /// `_MOTIF_WM_HINTS` asking for no decorations, and of a framed type
    /// (normal, dialog or utility; the first type the window manager
    /// knows decides, and a window of no known type is normal).
    pub fn decorated(&self, atoms: &Atoms) -> bool {
        if self.override_redirect || self.decorations == Some(false) {
            return false;
        }
        let framed = atoms.framed_types();
        let unframed = [
            atoms._NET_WM_WINDOW_TYPE_TOOLBAR,
            atoms._NET_WM_WINDOW_TYPE_SPLASH,
            atoms._NET_WM_WINDOW_TYPE_DOCK,
            atoms._NET_WM_WINDOW_TYPE_DESKTOP,
            atoms._NET_WM_WINDOW_TYPE_MENU,
            atoms._NET_WM_WINDOW_TYPE_DROPDOWN_MENU,
            atoms._NET_WM_WINDOW_TYPE_POPUP_MENU,
            atoms._NET_WM_WINDOW_TYPE_TOOLTIP,
            atoms._NET_WM_WINDOW_TYPE_NOTIFICATION,
            atoms._NET_WM_WINDOW_TYPE_COMBO,
            atoms._NET_WM_WINDOW_TYPE_DND,
        ];
        let first = self.window_type.iter().find(|t| **t != 0 && (framed.contains(t) || unframed.contains(t)));
        first.is_none_or(|t| framed.contains(t))
    }

    /// Whether the client takes part in protocol `atom` (`WM_PROTOCOLS`).
    pub fn supports(&self, atom: Xid) -> bool {
        atom != 0 && self.protocols.contains(&atom)
    }
}

/// What the compositor must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The window manager is up.
    Ready,
    /// Something about the window changed (geometry, mapped, title,
    /// decoration, transient-for): look at [`Wm::window`].
    Changed(Xid),
    /// The window is gone.
    Destroyed(Xid),
    /// The window's `wl_surface`.
    Associate { window: Xid, surface: SurfaceRef },
    /// The client asks for the window to be activated.
    Activate(Xid),
    /// The client asks to move the window with the pointer.
    Move(Xid),
    /// Something worth logging.
    Log(String),
}

/// The compositor's answers.
pub trait Policy {
    /// Where the content of managed `window` (its size already within its
    /// hints) goes when it maps; `parent` is the window it is transient
    /// for, if known.
    fn place(&mut self, window: &Window, parent: Option<&Window>) -> (i32, i32);
    /// The frame around decorated windows: left, right, top, bottom.
    fn frame_extents(&self) -> [u32; 4];
}

/// What a reply is for.
#[derive(Clone, Copy, Debug)]
enum Pending {
    Atom(usize),
    Property {
        window: Xid,
        property: Xid,
    },
    RootSelect,
    /// `QueryExtension` of Composite.
    Composite,
    /// The reply of `CompositeQueryVersion`.
    CompositeVersion,
}

/// The window manager.
#[derive(Debug)]
pub struct Wm {
    conn: Connection,
    atoms: Atoms,
    atoms_left: usize,
    root: Xid,
    screen: (u16, u16),
    check_window: Xid,
    windows: BTreeMap<Xid, Window>,
    /// Managed windows in the order they were mapped (`_NET_CLIENT_LIST`).
    clients: Vec<Xid>,
    pending: BTreeMap<u64, Pending>,
    /// Events that came before the atoms were known.
    deferred: VecDeque<(Event, bool)>,
    focus: Option<Xid>,
    actions: VecDeque<Action>,
    running: bool,
    broken: bool,
}

impl Default for Wm {
    fn default() -> Self {
        Wm::new()
    }
}

impl Wm {
    /// A window manager with its connection setup queued.
    pub fn new() -> Wm {
        Wm {
            conn: Connection::new(),
            atoms: Atoms::default(),
            atoms_left: 0,
            root: 0,
            screen: (0, 0),
            check_window: 0,
            windows: BTreeMap::new(),
            clients: Vec::new(),
            pending: BTreeMap::new(),
            deferred: VecDeque::new(),
            focus: None,
            actions: VecDeque::new(),
            running: false,
            broken: false,
        }
    }

    /// The requests to write to the descriptor (taken).
    pub fn take_output(&mut self) -> Vec<u8> {
        self.conn.take_output()
    }

    /// What the compositor must do (taken).
    pub fn take_actions(&mut self) -> Vec<Action> {
        self.actions.drain(..).collect()
    }

    /// Whether the atoms are known and the window manager runs.
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn atoms(&self) -> &Atoms {
        &self.atoms
    }

    pub fn root(&self) -> Xid {
        self.root
    }

    /// The root window's size (the X screen).
    pub fn screen_size(&self) -> (u16, u16) {
        self.screen
    }

    pub fn window(&self, id: Xid) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    /// The window with the X input focus the window manager gave.
    pub fn focus(&self) -> Option<Xid> {
        self.focus
    }

    fn send(&mut self, r: &Request<'_>) -> u64 {
        self.conn.send(r)
    }

    fn log(&mut self, s: String) {
        self.actions.push_back(Action::Log(s));
    }

    /// Bytes read from the descriptor: handle everything complete.
    pub fn feed(&mut self, bytes: &[u8], policy: &mut dyn Policy) -> Result<(), Broken> {
        if self.broken {
            return Err(Broken("the X connection failed"));
        }
        self.conn.feed(bytes);
        loop {
            let item = match self.conn.poll() {
                Ok(Some(item)) => item,
                Ok(None) => return Ok(()),
                Err(e) => {
                    self.broken = true;
                    return Err(e);
                }
            };
            self.handle(item, policy);
        }
    }

    fn handle(&mut self, item: Incoming, policy: &mut dyn Policy) {
        match item {
            Incoming::Setup(s) => self.start(&s),
            Incoming::Refused(why) => {
                self.broken = true;
                self.log(format!("the X server refused the window manager: {why}"));
            }
            Incoming::Reply { sequence, bytes } => match self.pending.remove(&sequence) {
                Some(Pending::Atom(i)) => {
                    self.atoms.set(i, crate::request::intern_atom_reply(&bytes));
                    self.atoms_left -= 1;
                    if self.atoms_left == 0 {
                        self.atoms_known(policy);
                    }
                }
                Some(Pending::Property { window, property }) => {
                    let p = crate::request::PropertyReply::parse(&bytes);
                    self.property(window, property, &p);
                }
                Some(Pending::Composite) => self.redirect(&bytes),
                Some(Pending::RootSelect) | Some(Pending::CompositeVersion) | None => {}
            },
            Incoming::Error { sequence, error } => match self.pending.remove(&sequence) {
                Some(Pending::RootSelect) if error.code == error_code::ACCESS => {
                    self.log("another X window manager is running (BadAccess on the root window)".into());
                }
                // The window went away before its properties were read.
                Some(Pending::Property { .. }) if error.code == error_code::WINDOW => {}
                Some(Pending::Atom(_)) => {
                    // Without its atoms the window manager cannot work.
                    self.broken = true;
                    self.log(format!("InternAtom failed: X error {}", error.code));
                }
                _ if error.code == error_code::WINDOW => {}
                _ => self.log(format!(
                    "X error {} for request {} (major {}, value {:#x})",
                    error.code, sequence, error.major_opcode, error.value
                )),
            },
            Incoming::Event { event, send_event } => {
                if self.running {
                    self.event(event, send_event, policy);
                } else {
                    self.deferred.push_back((event, send_event));
                }
            }
        }
    }

    /// The connection is up: intern the atoms, redirect the root, make the
    /// check window.
    fn start(&mut self, setup: &crate::setup::Setup) {
        let Some(screen) = setup.screens.first() else {
            self.broken = true;
            self.log("the X server has no screen".into());
            return;
        };
        self.root = screen.root;
        self.screen = (screen.width, screen.height);
        for (i, name) in Atoms::NAMES.iter().enumerate() {
            let seq = self.send(&Request::InternAtom { only_if_exists: false, name: name.as_bytes() });
            self.pending.insert(seq, Pending::Atom(i));
        }
        self.atoms_left = Atoms::NAMES.len();
        // Rootless Xwayland gives surfaces to the windows a compositing
        // manager redirects (Composite's RedirectSubwindows, manual).
        let seq = self.send(&Request::QueryExtension { name: composite::NAME });
        self.pending.insert(seq, Pending::Composite);
        let mask = event_mask::SUBSTRUCTURE_REDIRECT | event_mask::SUBSTRUCTURE_NOTIFY | event_mask::PROPERTY_CHANGE;
        let root = self.root;
        let seq = self.send(&Request::ChangeWindowAttributes { window: root, values: &[(cw::EVENT_MASK, mask)] });
        self.pending.insert(seq, Pending::RootSelect);
        self.check_window = self.conn.generate_id().unwrap_or(0);
        let check = self.check_window;
        self.send(&Request::CreateWindow {
            depth: 0,
            wid: check,
            parent: root,
            x: -100,
            y: -100,
            width: 1,
            height: 1,
            border_width: 0,
            class: class::INPUT_ONLY,
            visual: 0,
            values: &[(cw::OVERRIDE_REDIRECT, 1)],
        });
    }

    /// Composite is there (or not): negotiate its version and redirect the
    /// root's children, so Xwayland makes a surface for each top-level
    /// window.
    fn redirect(&mut self, reply: &[u8]) {
        let Some(major) = crate::request::query_extension_reply(reply) else {
            self.log("the X server has no Composite extension: windows get no surfaces".into());
            return;
        };
        let version = composite::query_version(0, 4);
        let seq =
            self.send(&Request::Extension { major, minor: composite::QUERY_VERSION, body: &version, reply: true });
        self.pending.insert(seq, Pending::CompositeVersion);
        let body = composite::redirect_subwindows(self.root, composite::REDIRECT_MANUAL);
        self.send(&Request::Extension { major, minor: composite::REDIRECT_SUBWINDOWS, body: &body, reply: false });
    }

    fn set_property32(&mut self, window: Xid, property: Xid, ty: Xid, values: &[u32]) {
        let data = icccm::u32_data(values);
        self.send(&Request::ChangeProperty { mode: prop_mode::REPLACE, window, property, ty, format: 32, data: &data });
    }

    fn atoms_known(&mut self, policy: &mut dyn Policy) {
        let (a, root, check) = (self.atoms, self.root, self.check_window);
        self.set_property32(root, a._NET_SUPPORTING_WM_CHECK, predefined::WINDOW, &[check]);
        self.set_property32(check, a._NET_SUPPORTING_WM_CHECK, predefined::WINDOW, &[check]);
        self.send(&Request::ChangeProperty {
            mode: prop_mode::REPLACE,
            window: check,
            property: a._NET_WM_NAME,
            ty: a.UTF8_STRING,
            format: 8,
            data: b"Poly",
        });
        self.set_property32(root, a._NET_SUPPORTED, predefined::ATOM, &a.supported());
        self.set_property32(root, a._NET_ACTIVE_WINDOW, predefined::WINDOW, &[0]);
        self.set_property32(root, a._NET_CLIENT_LIST, predefined::WINDOW, &[]);
        for selection in [a.WM_S0, a._NET_WM_CM_S0] {
            self.send(&Request::SetSelectionOwner { owner: check, selection, time: CURRENT_TIME });
        }
        self.running = true;
        self.actions.push_back(Action::Ready);
        while let Some((event, send_event)) = self.deferred.pop_front() {
            self.event(event, send_event, policy);
        }
    }

    /// The properties the window manager reads.
    fn watched(&self) -> [Xid; 10] {
        let a = &self.atoms;
        [
            predefined::WM_NAME,
            a._NET_WM_NAME,
            predefined::WM_CLASS,
            a.WM_PROTOCOLS,
            predefined::WM_HINTS,
            predefined::WM_NORMAL_HINTS,
            predefined::WM_TRANSIENT_FOR,
            a._NET_WM_WINDOW_TYPE,
            a._MOTIF_WM_HINTS,
            a._NET_WM_PID,
        ]
    }

    fn read_property(&mut self, window: Xid, property: Xid) {
        let seq = self.send(&Request::GetProperty {
            delete: false,
            window,
            property,
            ty: 0,
            long_offset: 0,
            long_length: 2048,
        });
        self.pending.insert(seq, Pending::Property { window, property });
    }

    /// Start following a new top-level window.
    fn adopt(&mut self, w: Window) {
        let id = w.id;
        self.windows.insert(id, w);
        let mask = event_mask::PROPERTY_CHANGE | event_mask::FOCUS_CHANGE;
        self.send(&Request::ChangeWindowAttributes { window: id, values: &[(cw::EVENT_MASK, mask)] });
        for p in self.watched() {
            self.read_property(id, p);
        }
        self.actions.push_back(Action::Changed(id));
    }

    fn property(&mut self, window: Xid, property: Xid, p: &crate::request::PropertyReply) {
        let a = self.atoms;
        let Some(w) = self.windows.get_mut(&window) else { return };
        let text = |p| icccm::text(p, a.UTF8_STRING, a.COMPOUND_TEXT);
        match property {
            predefined::WM_NAME => w.wm_name = text(p),
            predefined::WM_CLASS => w.class = icccm::class(p),
            predefined::WM_HINTS => w.input = icccm::input_hint(p),
            predefined::WM_NORMAL_HINTS => w.size_hints = SizeHints::parse(p),
            predefined::WM_TRANSIENT_FOR => {
                w.transient_for = p.u32s().first().copied().filter(|&t| t != 0 && t != window);
            }
            _ if property == a._NET_WM_NAME => w.net_wm_name = text(p),
            _ if property == a.WM_PROTOCOLS => w.protocols = p.u32s(),
            _ if property == a._NET_WM_WINDOW_TYPE => w.window_type = p.u32s(),
            _ if property == a._MOTIF_WM_HINTS => w.decorations = icccm::motif_decorations(p),
            _ if property == a._NET_WM_PID => w.pid = p.u32s().first().copied(),
            _ => return,
        }
        self.actions.push_back(Action::Changed(window));
    }

    fn update_client_list(&mut self) {
        self.clients.retain(|id| self.windows.get(id).is_some_and(|w| w.managed));
        let (root, list, prop) = (self.root, self.clients.clone(), self.atoms._NET_CLIENT_LIST);
        self.set_property32(root, prop, predefined::WINDOW, &list);
    }

    fn lost_focus(&mut self, window: Xid) {
        if self.focus == Some(window) {
            self.focus = None;
            let (root, prop) = (self.root, self.atoms._NET_ACTIVE_WINDOW);
            self.set_property32(root, prop, predefined::WINDOW, &[0]);
        }
    }

    fn event(&mut self, event: Event, send_event: bool, policy: &mut dyn Policy) {
        let a = self.atoms;
        match event {
            Event::CreateNotify { parent, window, x, y, width, height, override_redirect, .. } => {
                if window == self.check_window || parent != self.root || self.windows.contains_key(&window) {
                    return;
                }
                let mut w = Window::new(window);
                (w.x, w.y, w.width, w.height) = (x.into(), y.into(), width.into(), height.into());
                w.override_redirect = override_redirect;
                self.adopt(w);
            }
            Event::MapRequest { window, .. } => self.manage(window, policy),
            Event::MapNotify { window, override_redirect, .. } => {
                if window == self.check_window {
                    return;
                }
                match self.windows.get_mut(&window) {
                    Some(w) => {
                        w.mapped = true;
                        w.override_redirect = override_redirect;
                        self.actions.push_back(Action::Changed(window));
                    }
                    None => {
                        let mut w = Window::new(window);
                        (w.mapped, w.override_redirect) = (true, override_redirect);
                        self.adopt(w);
                    }
                }
            }
            Event::UnmapNotify { event, window, .. } => {
                // Ours (on the root), or a client's synthetic one to
                // withdraw (ICCCM 4.1.4).
                if event != self.root && !send_event {
                    return;
                }
                let Some(w) = self.windows.get_mut(&window) else { return };
                let was_managed = w.managed;
                if !send_event {
                    w.mapped = false;
                }
                w.managed = false;
                if was_managed {
                    self.set_property32(window, a.WM_STATE, a.WM_STATE, &[icccm::state::WITHDRAWN, 0]);
                    self.update_client_list();
                }
                self.lost_focus(window);
                self.actions.push_back(Action::Changed(window));
            }
            Event::DestroyNotify { window, .. } => {
                if self.windows.remove(&window).is_some() {
                    self.lost_focus(window);
                    if self.clients.contains(&window) {
                        self.update_client_list();
                    }
                    self.actions.push_back(Action::Destroyed(window));
                }
            }
            Event::ReparentNotify { window, parent, x, y, override_redirect, .. } => {
                if parent == self.root {
                    if !self.windows.contains_key(&window) {
                        let mut w = Window::new(window);
                        (w.x, w.y, w.override_redirect) = (x.into(), y.into(), override_redirect);
                        self.adopt(w);
                    }
                } else if self.windows.remove(&window).is_some() {
                    // No longer a top-level window.
                    self.lost_focus(window);
                    self.actions.push_back(Action::Destroyed(window));
                }
            }
            Event::ConfigureNotify { event, window, x, y, width, height, override_redirect, .. } => {
                if send_event || event != self.root {
                    return;
                }
                let Some(w) = self.windows.get_mut(&window) else { return };
                (w.x, w.y, w.width, w.height) = (x.into(), y.into(), width.into(), height.into());
                w.override_redirect = override_redirect;
                self.actions.push_back(Action::Changed(window));
            }
            Event::ConfigureRequest { window, x, y, width, height, border_width, value_mask, .. } => {
                self.configure_request(window, (x, y, width, height, border_width), value_mask);
            }
            Event::PropertyNotify { window, atom, state, .. } => {
                if !self.windows.contains_key(&window) || !self.watched().contains(&atom) {
                    return;
                }
                if state == 1 {
                    self.property(window, atom, &crate::request::PropertyReply::default());
                } else {
                    self.read_property(window, atom);
                }
            }
            Event::ClientMessage { window, ty, data, .. } => {
                let d = data32(&data);
                if ty == 0 {
                    return;
                }
                if ty == a.WL_SURFACE_SERIAL {
                    let serial = u64::from(d[0]) | u64::from(d[1]) << 32;
                    self.actions.push_back(Action::Associate { window, surface: SurfaceRef::Serial(serial) });
                } else if ty == a.WL_SURFACE_ID {
                    self.actions.push_back(Action::Associate { window, surface: SurfaceRef::Id(d[0]) });
                } else if self.windows.get(&window).is_some_and(|w| w.managed) {
                    if ty == a._NET_ACTIVE_WINDOW {
                        self.actions.push_back(Action::Activate(window));
                    } else if ty == a._NET_WM_MOVERESIZE && d[2] == MOVERESIZE_MOVE {
                        self.actions.push_back(Action::Move(window));
                    }
                }
            }
            Event::SelectionRequest { time, requestor, selection, target, .. } => {
                // The manager selections convert to nothing (ICCCM 2.2:
                // refuse with property None).
                let refusal = Event::SelectionNotify { time, requestor, selection, target, property: 0 };
                self.send(&Request::SendEvent {
                    propagate: false,
                    destination: requestor,
                    event_mask: 0,
                    event: refusal.encode(0),
                });
            }
            Event::SelectionClear { selection, .. } if selection == a.WM_S0 => {
                self.log("another X window manager took the WM_S0 selection".into());
            }
            _ => {}
        }
    }

    /// A map request: place, size, `WM_STATE`, map.
    fn manage(&mut self, window: Xid, policy: &mut dyn Policy) {
        let a = self.atoms;
        if !self.windows.contains_key(&window) {
            self.adopt(Window::new(window));
        }
        let Some(w) = self.windows.get(&window) else { return };
        let (sw, sh) = (u32::from(self.screen.0.max(1)), u32::from(self.screen.1.max(1)));
        let (mut width, mut height) = w.size_hints.constrain(w.width, w.height);
        (width, height) = (width.min(sw), height.min(sh));
        let (old_size, decorated) = ((w.width, w.height), w.decorated(&a));
        let mut sized = w.clone();
        (sized.width, sized.height) = (width, height);
        let parent = w.transient_for.and_then(|p| self.windows.get(&p));
        let (x, y) = policy.place(&sized, parent);
        let mut values = alloc::vec![(config::X, x as u32), (config::Y, y as u32), (config::BORDER_WIDTH, 0)];
        if (width, height) != old_size {
            values.push((config::WIDTH, width));
            values.push((config::HEIGHT, height));
        }
        self.send(&Request::ConfigureWindow { window, values: &values });
        let extents = policy.frame_extents();
        if let Some(w) = self.windows.get_mut(&window) {
            (w.x, w.y, w.width, w.height, w.managed) = (x, y, width, height, true);
        }
        self.set_property32(window, a.WM_STATE, a.WM_STATE, &[icccm::state::NORMAL, 0]);
        let extents = if decorated { extents } else { [0; 4] };
        self.set_property32(window, a._NET_FRAME_EXTENTS, predefined::CARDINAL, &extents);
        self.send(&Request::MapWindow { window });
        if !self.clients.contains(&window) {
            self.clients.push(window);
        }
        self.update_client_list();
        self.actions.push_back(Action::Changed(window));
    }

    fn configure_request(&mut self, window: Xid, geometry: (i16, i16, u16, u16, u16), mask: u16) {
        let (x, y, width, height, border) = geometry;
        let Some(w) = self.windows.get(&window) else {
            // Not a window followed here: grant it as asked.
            let all = [
                (config::X, x as i32 as u32),
                (config::Y, y as i32 as u32),
                (config::WIDTH, u32::from(width)),
                (config::HEIGHT, u32::from(height)),
                (config::BORDER_WIDTH, u32::from(border)),
            ];
            let values: Vec<(u16, u32)> = all.into_iter().filter(|(bit, _)| mask & bit != 0).collect();
            self.send(&Request::ConfigureWindow { window, values: &values });
            return;
        };
        let (managed, wx, wy, ww, wh, hints) = (w.managed, w.x, w.y, w.width, w.height, w.size_hints);
        let mut values = Vec::new();
        if !managed {
            if mask & config::X != 0 {
                values.push((config::X, x as i32 as u32));
            }
            if mask & config::Y != 0 {
                values.push((config::Y, y as i32 as u32));
            }
        }
        if mask & (config::WIDTH | config::HEIGHT) != 0 {
            let want_w = if mask & config::WIDTH != 0 { u32::from(width) } else { ww };
            let want_h = if mask & config::HEIGHT != 0 { u32::from(height) } else { wh };
            let (cw, ch) = hints.constrain(want_w, want_h);
            if cw != ww {
                values.push((config::WIDTH, cw));
            }
            if ch != wh {
                values.push((config::HEIGHT, ch));
            }
        }
        if mask & config::BORDER_WIDTH != 0 && border != 0 {
            values.push((config::BORDER_WIDTH, 0));
        }
        let changes_geometry = values.iter().any(|(bit, _)| *bit != config::BORDER_WIDTH);
        if !values.is_empty() {
            self.send(&Request::ConfigureWindow { window, values: &values });
        }
        if !changes_geometry {
            // Nothing moved or resized: tell the client where it is.
            let notify = Event::ConfigureNotify {
                event: window,
                window,
                above_sibling: 0,
                x: wx as i16,
                y: wy as i16,
                width: ww as u16,
                height: wh as u16,
                border_width: 0,
                override_redirect: false,
            };
            self.send(&Request::SendEvent {
                propagate: false,
                destination: window,
                event_mask: event_mask::STRUCTURE_NOTIFY,
                event: notify.encode(0),
            });
        }
    }

    /// Give the keyboard to `window` (a managed window), or take it from
    /// every X window (`None`).
    pub fn activate(&mut self, window: Option<Xid>) {
        let (a, root) = (self.atoms, self.root);
        let Some(id) = window else {
            if self.focus.take().is_some() {
                self.send(&Request::SetInputFocus { revert_to: revert::NONE, focus: 0, time: CURRENT_TIME });
                self.set_property32(root, a._NET_ACTIVE_WINDOW, predefined::WINDOW, &[0]);
            }
            return;
        };
        let Some(w) = self.windows.get(&id).filter(|w| w.managed) else { return };
        let (input, take_focus) = (w.input, w.supports(a.WM_TAKE_FOCUS));
        self.focus = Some(id);
        if input {
            self.send(&Request::SetInputFocus { revert_to: revert::POINTER_ROOT, focus: id, time: CURRENT_TIME });
        }
        if take_focus {
            let msg = client_message32(id, a.WM_PROTOCOLS, [a.WM_TAKE_FOCUS, CURRENT_TIME, 0, 0, 0]);
            self.send(&Request::SendEvent { propagate: false, destination: id, event_mask: 0, event: msg.encode(0) });
        }
        self.send(&Request::ConfigureWindow { window: id, values: &[(config::STACK_MODE, stack::ABOVE)] });
        self.set_property32(root, a._NET_ACTIVE_WINDOW, predefined::WINDOW, &[id]);
    }

    /// The user asked to close `window`: `WM_DELETE_WINDOW`, or kill its
    /// client.
    pub fn close(&mut self, window: Xid) {
        let a = self.atoms;
        let Some(w) = self.windows.get(&window) else { return };
        if w.supports(a.WM_DELETE_WINDOW) {
            let msg = client_message32(window, a.WM_PROTOCOLS, [a.WM_DELETE_WINDOW, CURRENT_TIME, 0, 0, 0]);
            self.send(&Request::SendEvent {
                propagate: false,
                destination: window,
                event_mask: 0,
                event: msg.encode(0),
            });
        } else {
            self.send(&Request::KillClient { resource: window });
        }
    }

    /// The compositor moved `window`'s content to (`x`, `y`).
    pub fn move_to(&mut self, window: Xid, x: i32, y: i32) {
        let Some(w) = self.windows.get_mut(&window) else { return };
        if (w.x, w.y) == (x, y) {
            return;
        }
        (w.x, w.y) = (x, y);
        self.send(&Request::ConfigureWindow { window, values: &[(config::X, x as u32), (config::Y, y as u32)] });
    }
}

#[cfg(test)]
mod tests;
