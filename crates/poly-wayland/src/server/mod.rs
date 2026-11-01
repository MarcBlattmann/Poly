//! The Wayland server: clients, their objects, request dispatch, and the
//! protocol logic of the core protocol, xdg-shell, xdg-decoration and
//! presentation-time, feeding a [`Scene`].
//!
//! The server never touches a socket, a file descriptor or memory mapping
//! itself. The caller (the compositor binary, or a test) owns the
//! transport: it hands each client's bytes and file descriptors to
//! [`Server::receive`], sends what [`Server::take_output`] returns, and
//! implements [`Host`], which maps a client's shared memory and closes file
//! descriptors. Everything here is therefore host-tested without displays,
//! sockets or another process.
//!
//! # Dispatch
//!
//! [`Server::receive`] appends to the client's input and handles every
//! complete message: it looks up the target object (`wl_display.error`
//! `invalid_object` if there is none), the request by opcode and version
//! (`invalid_method`), decodes the arguments with the interface's generated
//! table (taking file descriptors in order), checks that every `new_id` is
//! a free client ID and every `object` exists and has the interface the XML
//! names, and calls the interface's handler with the typed request. A
//! handler's protocol error goes out as `wl_display.error` and the client
//! is marked failed: no more of its requests are handled, and the caller
//! disconnects it after sending what is queued. After a destructor request
//! the object is removed and `wl_display.delete_id` sent.
//!
//! # Buffers
//!
//! A committed `wl_shm` or `linux-dmabuf` buffer is copied (the damaged
//! part of it) into the window's content in the scene and released at
//! once, so clients may reuse one buffer, and the scene never reads client
//! memory while composing.
//!
//! # Frames
//!
//! [`Server::frame_presented`] answers the frame callbacks and
//! presentation feedback committed so far, for a compositor that shows a
//! frame as it draws it. One that waits for the display (page flips, ADR
//! 0116) calls [`Server::frame_submitted`] when the frame goes to the
//! display, which takes what was committed for it, and
//! [`Server::frame_completed`] when the flip completed, with the vblank's
//! time and sequence number: clients are paced by the display's refresh.

mod cursor;
mod data_device;
mod decoration;
mod display;
mod dmabuf;
mod external;
mod extras;
mod input;
mod output;
mod presentation;
mod shm;
mod subsurface;
mod surface;
mod viewporter;
mod wm;
mod xdg;
mod xwayland;

pub use dmabuf::{MOD_INVALID, MOD_LINEAR, format_table as dmabuf_format_table, formats as dmabuf_formats};
pub use external::WmEvent;
pub use wm::{InternalEvent, WindowInfo};

use crate::compose::{Framebuffer, Hit, Rect, Scene, WindowId};
use crate::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
    wl_display, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_subcompositor, wl_subsurface, wl_surface, wl_touch, wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1, wp_presentation, wp_presentation_feedback, wp_viewport,
    wp_viewporter, xdg_activation_token_v1, xdg_activation_v1, xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel,
    xdg_wm_base, xwayland_shell_v1, xwayland_surface_v1, zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1,
    zwp_linux_dmabuf_v1, zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use crate::seat::{Capabilities, InputEvent};
use crate::wire::{self, Arg, ArgKind, Fd, Fixed, Header, Interface, Message, ObjectId, SERVER_ID_START};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

pub(crate) use shm::{Buffer, Pool};
pub(crate) use surface::{Region, Surface};
pub(crate) use xdg::{Popup, Positioner, Toplevel, XdgSurface};
pub use xwayland::{SurfaceRef, XRequest, XWindowState};

/// A client's shared memory, mapped by the [`Host`].
pub trait ShmMap {
    /// The mapped size in bytes.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Map `size` bytes instead (`wl_shm_pool.resize`); false on failure.
    fn resize(&mut self, size: usize) -> bool;
    /// Copy `dst.len()` bytes from `offset`; false (and nothing copied) if
    /// that is not inside the mapping.
    fn read(&self, offset: usize, dst: &mut [u8]) -> bool;
}

/// What the server needs from its environment.
pub trait Host {
    /// Map `size` bytes of the shared memory file `fd` (`wl_shm.create_pool`).
    /// The host owns `fd` from here on: it keeps it for resizing and closes
    /// it when the mapping is dropped, or at once on failure (`None`).
    fn map_shm(&mut self, fd: Fd, size: usize) -> Option<Box<dyn ShmMap>>;
    /// Close a file descriptor the server does not keep.
    fn close_fd(&mut self, fd: Fd);
    /// A fresh file descriptor of the keyboard's XKB keymap (`xkb_v1`, NUL
    /// included) and its size, for `wl_keyboard.keymap` (ADR 0107). The
    /// server queues it to be sent; whoever sends it closes it afterwards.
    /// `None` (the default): no keymap.
    fn keymap(&mut self) -> Option<(Fd, u32)> {
        None
    }
    /// The `dev_t` of the DRM device clients should allocate on (its render
    /// node), if dma-buf buffers can be taken: then `zwp_linux_dmabuf_v1`
    /// is offered. Asked once, when the server is made. `None` (the
    /// default): no dma-bufs.
    fn dmabuf_device(&mut self) -> Option<u64> {
        None
    }
    /// Map all of the dma-buf `fd` for reading (its size is the host's to
    /// find out: `lseek(SEEK_END)`). The host owns `fd` from here on, as
    /// with [`Host::map_shm`].
    fn map_dmabuf(&mut self, fd: Fd) -> Option<Box<dyn ShmMap>> {
        self.close_fd(fd);
        None
    }
    /// A fresh read-only file descriptor holding `table` (the dma-buf
    /// format table of `zwp_linux_dmabuf_feedback_v1`); whoever sends it
    /// closes it afterwards.
    fn dmabuf_table(&mut self, _table: &[u8]) -> Option<Fd> {
        None
    }
}

/// A connected client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u32);

/// The output the server advertises (`wl_output`) and composes for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputConfig {
    /// The mode, in physical pixels.
    pub width: u32,
    pub height: u32,
    /// The integer scale (HiDPI): logical pixels are `scale` physical ones.
    pub scale: u32,
    /// Refresh rate in millihertz.
    pub refresh_mhz: u32,
    /// Physical size in millimetres (0 if unknown).
    pub physical_mm: (i32, i32),
    pub make: String,
    pub model: String,
    pub name: String,
    pub description: String,
}

impl Default for OutputConfig {
    fn default() -> OutputConfig {
        OutputConfig {
            width: 1024,
            height: 768,
            scale: 1,
            refresh_mhz: 60_000,
            physical_mm: (0, 0),
            make: "Poly".into(),
            model: "virtual output".into(),
            name: "Virtual-1".into(),
            description: "Poly virtual output".into(),
        }
    }
}

/// The server's setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub output: OutputConfig,
    pub seat_name: String,
}

impl Default for Config {
    fn default() -> Config {
        Config { output: OutputConfig::default(), seat_name: "seat0".into() }
    }
}

/// The versions of the globals the server implements.
pub mod versions {
    pub const COMPOSITOR: u32 = 6;
    pub const SHM: u32 = 2;
    pub const OUTPUT: u32 = 4;
    pub const SEAT: u32 = 7;
    pub const XDG_WM_BASE: u32 = 6;
    pub const DECORATION_MANAGER: u32 = 2;
    pub const PRESENTATION: u32 = 1;
    pub const SUBCOMPOSITOR: u32 = 1;
    pub const DATA_DEVICE_MANAGER: u32 = 3;
    pub const VIEWPORTER: u32 = 1;
    pub const LINUX_DMABUF: u32 = 4;
    pub const FRACTIONAL_SCALE: u32 = 1;
    pub const ACTIVATION: u32 = 1;
    pub const CURSOR_SHAPE: u32 = 2;
    pub const XWAYLAND_SHELL: u32 = 1;
}

/// `wp_presentation_feedback.kind` bits (presentation-time.xml): how a
/// frame was shown.
pub mod present_flags {
    /// Shown at a vertical blank (no tearing).
    pub const VSYNC: u32 = 0x1;
    /// The time is the display's own.
    pub const HW_CLOCK: u32 = 0x2;
    /// The display said when it was shown.
    pub const HW_COMPLETION: u32 = 0x4;
    pub const ZERO_COPY: u32 = 0x8;
}

/// A global: its name in the registry, interface and version.
#[derive(Clone, Copy, Debug)]
struct Global {
    name: u32,
    interface: &'static Interface,
    version: u32,
}

/// A protocol error: the object it is about, the interface's error code,
/// and a message for people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtoError {
    pub object: ObjectId,
    pub code: u32,
    pub message: String,
}

pub(crate) fn error(object: ObjectId, code: u32, message: impl Into<String>) -> ProtoError {
    ProtoError { object, code, message: message.into() }
}

pub(crate) type Res<T = ()> = Result<T, ProtoError>;

/// What an object is, with its state.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Data {
    Display,
    Registry,
    Callback,
    Compositor,
    Region(Region),
    Surface(Box<Surface>),
    Shm,
    Pool(Rc<Pool>),
    Buffer(Buffer),
    Output,
    Seat,
    /// `wl_pointer`, `wl_keyboard`, `wl_touch` (told apart by interface;
    /// routed in `input.rs`, touch not yet).
    InputDevice,
    WmBase,
    Positioner(Positioner),
    XdgSurface(Box<XdgSurface>),
    Toplevel(Box<Toplevel>),
    Popup(Popup),
    DecorationManager,
    /// The toplevel it decorates.
    Decoration(ObjectId),
    Presentation,
    Feedback,
    Subcompositor,
    /// A `wl_subsurface`: the surface it made a subsurface.
    Subsurface(ObjectId),
    Viewporter,
    /// A `wp_viewport`: its surface, while there is one.
    Viewport(Option<ObjectId>),
    DataDeviceManager,
    DataSource(Box<data_device::DataSource>),
    DataDevice,
    DataOffer(Box<data_device::DataOffer>),
    Dmabuf,
    DmabufParams(Box<dmabuf::Params>),
    DmabufFeedback,
    FractionalManager,
    /// A `wp_fractional_scale_v1`: its surface, while there is one.
    Fractional(Option<ObjectId>),
    Activation,
    ActivationToken(Box<extras::Token>),
    CursorShapeManager,
    /// A `wp_cursor_shape_device_v1`.
    CursorShapeDevice,
    /// `xwayland_shell_v1` (Xwayland only, ADR 0118).
    XwaylandShell,
    /// `xwayland_surface_v1`: the surface it gives the role.
    XwaylandSurface(ObjectId),
}

/// An object of a client: its interface, the version it was made with, and
/// what it is.
#[derive(Debug)]
pub(crate) struct Object {
    pub interface: &'static Interface,
    pub version: u32,
    pub data: Data,
}

/// A client's connection state.
#[derive(Debug, Default)]
pub(crate) struct Client {
    pub objects: BTreeMap<u32, Object>,
    input: Vec<u8>,
    fds: VecDeque<Fd>,
    output: Vec<u8>,
    output_fds: Vec<Fd>,
    next_server_id: u32,
    /// Why the client was cut off, after a protocol error.
    failed: Option<String>,
}

impl Client {
    /// Queue `msg` from object `id`, unless the object is gone or older
    /// than the event.
    pub fn send(&mut self, id: ObjectId, msg: impl Message) {
        let Some(obj) = self.objects.get(&id.0) else { return };
        let opcode = msg.opcode();
        let Some(desc) = obj.interface.events.get(usize::from(opcode)) else { return };
        if desc.since > obj.version {
            return;
        }
        // Too long only for a string past 64 KiB, which the server never makes.
        let _ = wire::encode(&mut self.output, &mut self.output_fds, id, opcode, &msg.into_args());
    }

    pub fn insert(&mut self, id: ObjectId, interface: &'static Interface, version: u32, data: Data) {
        self.objects.insert(id.0, Object { interface, version, data });
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(&id.0)
    }

    pub fn data(&self, id: ObjectId) -> Option<&Data> {
        self.objects.get(&id.0).map(|o| &o.data)
    }

    pub fn data_mut(&mut self, id: ObjectId) -> Option<&mut Data> {
        self.objects.get_mut(&id.0).map(|o| &mut o.data)
    }

    pub fn version(&self, id: ObjectId) -> u32 {
        self.objects.get(&id.0).map_or(0, |o| o.version)
    }

    /// Remove an object; one the client made gets `wl_display.delete_id`.
    pub fn delete(&mut self, id: ObjectId) -> Option<Object> {
        let obj = self.objects.remove(&id.0)?;
        if !id.is_server() {
            self.send(ObjectId::DISPLAY, wl_display::Event::DeleteId { id: id.0 });
        }
        Some(obj)
    }

    /// Objects of one kind, by ID.
    pub fn ids_where(&self, f: impl Fn(&Data) -> bool) -> Vec<ObjectId> {
        self.objects.iter().filter(|(_, o)| f(&o.data)).map(|(id, _)| ObjectId(*id)).collect()
    }

    #[allow(dead_code)]
    pub fn new_server_id(&mut self) -> ObjectId {
        while self.objects.contains_key(&(SERVER_ID_START + self.next_server_id)) {
            self.next_server_id = self.next_server_id.wrapping_add(1) & 0x00ff_ffff;
        }
        let id = ObjectId(SERVER_ID_START + self.next_server_id);
        self.next_server_id = self.next_server_id.wrapping_add(1) & 0x00ff_ffff;
        id
    }

    /// Callbacks are made, answered and gone at once (`wl_display.sync`).
    pub fn callback_done(&mut self, id: ObjectId, data: u32) {
        self.send(id, wl_callback::Event::Done { callback_data: data });
        self.delete(id);
    }
}

/// The state of the seat.
#[derive(Debug, Default)]
struct SeatState {
    name: String,
    capabilities: Capabilities,
    /// Every capability the seat ever had (`missing_capability` is for
    /// those it never had).
    ever: Capabilities,
    pointer: (Fixed, Fixed),
    input: input::InputState,
    /// What the client the pointer is over asked it to look like.
    cursor: Option<cursor::CursorRequest>,
}

/// What one client's frame callbacks and feedback wait for: the frame
/// that went to the display ([`Server::frame_submitted`]).
#[derive(Debug, Default)]
struct InFlight {
    callbacks: Vec<ObjectId>,
    /// Feedback, and whether its surface was shown.
    feedbacks: Vec<(ObjectId, bool)>,
}

/// One frame's presentation time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTime {
    /// When it was shown, in nanoseconds on `CLOCK_MONOTONIC`.
    pub monotonic_ns: u64,
}

/// The Wayland server.
pub struct Server {
    host: Box<dyn Host>,
    pub(crate) scene: Scene,
    pub(crate) clients: BTreeMap<ClientId, Client>,
    next_client: u32,
    globals: Vec<Global>,
    pub(crate) output: OutputConfig,
    seat: SeatState,
    serial: u32,
    /// The toplevel with focus.
    pub(crate) active: Option<(ClientId, ObjectId)>,
    frame_seq: u64,
    /// The clipboard and drag and drop.
    data: data_device::DataState,
    /// Popups with a grab, bottom to top: client and `xdg_popup`.
    popup_grabs: Vec<(ClientId, ObjectId)>,
    /// Activation tokens handed out and not used yet.
    tokens: Vec<String>,
    token_serial: u64,
    /// The DRM device dma-bufs are made on, if they are taken.
    dmabuf_device: Option<u64>,
    /// Frame callbacks and feedback of the frame on its way to the display.
    in_flight: BTreeMap<ClientId, InFlight>,
    /// What an outside window manager is to act on (`external.rs`).
    wm_events: Vec<WmEvent>,
    /// Windows the compositor draws itself (ADR 0117), their events, the
    /// one with focus (never at the same time as `active`), and the next
    /// owner tag.
    pub(crate) internal: BTreeMap<WindowId, wm::Internal>,
    pub(crate) internal_events: Vec<InternalEvent>,
    pub(crate) active_internal: Option<WindowId>,
    next_internal: u32,
    /// Xwayland's client and its X windows (ADR 0118).
    xwayland: xwayland::XwaylandState,
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server").field("clients", &self.clients.len()).field("scene", &self.scene).finish()
    }
}

/// A window's owner in the scene: the client and its surface.
pub(crate) fn owner_tag(c: ClientId, surface: ObjectId) -> u64 {
    u64::from(c.0) << 32 | u64::from(surface.0)
}

pub(crate) fn owner_of(tag: u64) -> (ClientId, ObjectId) {
    (ClientId((tag >> 32) as u32), ObjectId(tag as u32))
}

impl Server {
    pub fn new(config: Config, mut host: Box<dyn Host>) -> Server {
        let o = &config.output;
        let dmabuf_device = host.dmabuf_device();
        let mut globals = alloc::vec![
            (&wl_compositor::INTERFACE, versions::COMPOSITOR),
            (&wl_shm::INTERFACE, versions::SHM),
            (&wl_output::INTERFACE, versions::OUTPUT),
            (&wl_seat::INTERFACE, versions::SEAT),
            (&xdg_wm_base::INTERFACE, versions::XDG_WM_BASE),
            (&zxdg_decoration_manager_v1::INTERFACE, versions::DECORATION_MANAGER),
            (&wp_presentation::INTERFACE, versions::PRESENTATION),
            (&wl_subcompositor::INTERFACE, versions::SUBCOMPOSITOR),
            (&wl_data_device_manager::INTERFACE, versions::DATA_DEVICE_MANAGER),
            (&wp_viewporter::INTERFACE, versions::VIEWPORTER),
            (&wp_fractional_scale_manager_v1::INTERFACE, versions::FRACTIONAL_SCALE),
            (&xdg_activation_v1::INTERFACE, versions::ACTIVATION),
            (&wp_cursor_shape_manager_v1::INTERFACE, versions::CURSOR_SHAPE),
            (&xwayland_shell_v1::INTERFACE, versions::XWAYLAND_SHELL),
        ];
        if dmabuf_device.is_some() {
            globals.push((&zwp_linux_dmabuf_v1::INTERFACE, versions::LINUX_DMABUF));
        }
        Server {
            host,
            scene: Scene::new(o.width, o.height, o.scale),
            clients: BTreeMap::new(),
            next_client: 1,
            globals: globals
                .iter()
                .enumerate()
                .map(|(i, (interface, version))| Global { name: i as u32 + 1, interface, version: *version })
                .collect(),
            output: config.output,
            seat: SeatState { name: config.seat_name, ..SeatState::default() },
            serial: 0,
            active: None,
            frame_seq: 0,
            data: data_device::DataState::default(),
            popup_grabs: Vec::new(),
            tokens: Vec::new(),
            token_serial: 0,
            dmabuf_device,
            in_flight: BTreeMap::new(),
            wm_events: Vec::new(),
            internal: BTreeMap::new(),
            internal_events: Vec::new(),
            active_internal: None,
            next_internal: 1,
            xwayland: xwayland::XwaylandState::default(),
        }
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    pub(crate) fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }

    /// A new client; its `wl_display` is object 1.
    pub fn add_client(&mut self) -> ClientId {
        let id = ClientId(self.next_client);
        self.next_client += 1;
        let mut client = Client::default();
        client.insert(ObjectId::DISPLAY, &wl_display::INTERFACE, 1, Data::Display);
        self.clients.insert(id, client);
        id
    }

    /// Forget a client (disconnected or cut off): its windows go, its
    /// shared memory is unmapped, file descriptors it sent are closed.
    pub fn remove_client(&mut self, c: ClientId) {
        let Some(mut client) = self.clients.remove(&c) else { return };
        for fd in client.fds.drain(..) {
            self.host.close_fd(fd);
        }
        for fd in client.output_fds.drain(..) {
            self.host.close_fd(fd);
        }
        let windows: Vec<WindowId> =
            self.scene.windows().iter().filter(|w| owner_of(w.owner()).0 == c).map(|w| w.id()).collect();
        for w in windows {
            self.scene.remove_window(w);
        }
        self.input_client_gone(c);
        self.data_client_gone(c);
        self.in_flight.remove(&c);
        if self.active.is_some_and(|(ac, _)| ac == c) {
            self.active = None;
            self.focus_next();
        }
        if self.xwayland_client() == Some(c) {
            self.set_xwayland_client(None);
        }
    }

    /// The scene window showing client `c`'s surface, while it is shown.
    pub fn surface_window(&self, c: ClientId, surface: ObjectId) -> Option<WindowId> {
        self.surface_ref(c, surface)?.window
    }

    /// Keep the picture of what lies behind client `c`'s surface up to date
    /// (or stop): [`crate::compose::Backdrop`], read with
    /// [`Server::surface_backdrop`]. What a material like Liquid Glass
    /// samples (ADR 0116). False if the surface is not shown.
    pub fn set_surface_backdrop(&mut self, c: ClientId, surface: ObjectId, keep: bool) -> bool {
        let Some(w) = self.surface_window(c, surface) else { return false };
        self.scene.set_backdrop(w, keep);
        true
    }

    /// The picture behind client `c`'s surface, if it is kept, and the parts
    /// of it drawn again since the last call.
    pub fn surface_backdrop(
        &mut self,
        c: ClientId,
        surface: ObjectId,
    ) -> Option<(&crate::compose::Backdrop, Vec<Rect>)> {
        let w = self.surface_window(c, surface)?;
        let damage = self.scene.take_backdrop_damage(w);
        self.scene.backdrop(w).map(|b| (b, damage))
    }

    /// A surface of client `c` was destroyed: whatever pointed at it lets
    /// go (its viewport, the cursor).
    pub(crate) fn surface_gone(&mut self, c: ClientId, surface: ObjectId) {
        self.viewport_surface_gone(c, surface);
        if matches!(self.seat.cursor, Some(cursor::CursorRequest::Surface { client, surface: s, .. }) if (client, s) == (c, surface))
        {
            self.seat.cursor = Some(cursor::CursorRequest::Hidden);
            self.update_cursor_look();
        }
    }

    pub fn clients(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients.keys().copied()
    }

    /// Why a client was cut off, if it was: send it what is queued, then
    /// disconnect it.
    pub fn failure(&self, c: ClientId) -> Option<&str> {
        self.clients.get(&c)?.failed.as_deref()
    }

    /// What to send a client: bytes and the file descriptors to go with
    /// them (empty if nothing is queued).
    pub fn take_output(&mut self, c: ClientId) -> (Vec<u8>, Vec<Fd>) {
        let Some(client) = self.clients.get_mut(&c) else { return (Vec::new(), Vec::new()) };
        (core::mem::take(&mut client.output), core::mem::take(&mut client.output_fds))
    }

    /// Bytes and file descriptors from a client: handle every complete
    /// message.
    pub fn receive(&mut self, c: ClientId, bytes: &[u8], fds: Vec<Fd>) {
        let Some(client) = self.clients.get_mut(&c).filter(|cl| cl.failed.is_none()) else {
            for fd in fds {
                self.host.close_fd(fd);
            }
            return;
        };
        client.input.extend_from_slice(bytes);
        client.fds.extend(fds);
        while let Some(header) = self.next_message(c) {
            let Some(client) = self.clients.get_mut(&c) else { return };
            let size = usize::from(header.size);
            let body: Vec<u8> = client.input[wire::HEADER_SIZE..size].to_vec();
            client.input.drain(..size);
            if let Err(e) = self.handle(c, header, &body) {
                self.post_error(c, e);
                return;
            }
        }
        // A window mapped or unmapped: the keyboard follows the active one.
        self.sync_keyboard_focus();
    }

    /// The header of the next complete message, if there is one; a
    /// malformed size cuts the client off.
    fn next_message(&mut self, c: ClientId) -> Option<Header> {
        let client = self.clients.get(&c)?;
        if client.failed.is_some() {
            return None;
        }
        let header = Header::parse(&client.input)?;
        let size = usize::from(header.size);
        if size < wire::HEADER_SIZE || size % 4 != 0 {
            let e = error(ObjectId::DISPLAY, wl_display::Error::INVALID_METHOD.0, format!("message size {size}"));
            self.post_error(c, e);
            return None;
        }
        (client.input.len() >= size).then_some(header)
    }

    /// Send `wl_display.error` and stop handling the client's requests.
    pub(crate) fn post_error(&mut self, c: ClientId, e: ProtoError) {
        let Some(client) = self.clients.get_mut(&c) else { return };
        client.send(
            ObjectId::DISPLAY,
            wl_display::Event::Error { object_id: e.object, code: e.code, message: e.message.clone() },
        );
        client.failed = Some(format!("object {} error {}: {}", e.object, e.code, e.message));
    }

    pub(crate) fn client(&mut self, c: ClientId) -> &mut Client {
        self.clients.get_mut(&c).expect("a client being handled exists")
    }

    pub(crate) fn host(&mut self) -> &mut dyn Host {
        &mut *self.host
    }

    /// Close the file descriptors among `args`.
    fn close_fds(&mut self, args: &[Arg]) {
        for a in args {
            if let Arg::Fd(fd) = a {
                self.host.close_fd(*fd);
            }
        }
    }

    fn handle(&mut self, c: ClientId, h: Header, body: &[u8]) -> Res {
        let client = self.client(c);
        let Some(obj) = client.get(h.object) else {
            let msg = format!("invalid object {}", h.object);
            return Err(error(ObjectId::DISPLAY, wl_display::Error::INVALID_OBJECT.0, msg));
        };
        let (interface, version) = (obj.interface, obj.version);
        let invalid_method = |what: String| error(h.object, wl_display::Error::INVALID_METHOD.0, what);
        let Some(desc) = interface.requests.get(usize::from(h.opcode)) else {
            return Err(invalid_method(format!("{}: no request {}", interface.name, h.opcode)));
        };
        if desc.since > version {
            return Err(invalid_method(format!("{}.{} needs version {}", interface.name, desc.name, desc.since)));
        }
        let mut taken = Vec::new();
        let decoded = wire::decode_args(desc, body, &mut || {
            let fd = client.fds.pop_front();
            taken.extend(fd);
            fd
        });
        let args = match decoded {
            Ok(args) => args,
            Err(e) => {
                for fd in taken {
                    self.host.close_fd(fd);
                }
                return Err(invalid_method(format!("{}.{}: {e}", interface.name, desc.name)));
            }
        };
        if let Err(e) = self.check_args(c, desc, &args) {
            self.close_fds(&args);
            return Err(e);
        }
        self.request(c, h.object, interface, h.opcode, args)?;
        if desc.destructor {
            self.client(c).delete(h.object);
        }
        Ok(())
    }

    /// Every `new_id` a free client ID, every `object` there with the
    /// interface the protocol names.
    fn check_args(&mut self, c: ClientId, desc: &wire::MessageDesc, args: &[Arg]) -> Res {
        let client = self.client(c);
        let invalid = |what: String| error(ObjectId::DISPLAY, wl_display::Error::INVALID_OBJECT.0, what);
        for (d, a) in desc.args.iter().zip(args) {
            let new = match a {
                Arg::NewId(id) => Some(*id),
                Arg::DynNewId(n) => Some(n.id),
                _ => None,
            };
            if let Some(id) = new
                && (id.is_server() || client.get(id).is_some())
            {
                return Err(invalid(format!("{}: new id {id} is in use or not a client id", desc.name)));
            }
            if let (ArgKind::Object, Arg::Object(Some(id))) = (d.kind, a) {
                let Some(obj) = client.get(*id) else {
                    return Err(invalid(format!("{}: invalid object {id}", desc.name)));
                };
                if let Some(want) = d.interface
                    && obj.interface.name != want
                {
                    return Err(invalid(format!(
                        "{}: object {id} is a {}, not a {want}",
                        desc.name, obj.interface.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Hand a decoded request to its interface's handler.
    fn request(&mut self, c: ClientId, id: ObjectId, iface: &'static Interface, op: u16, args: Vec<Arg>) -> Res {
        macro_rules! parse {
            ($m:ident) => {
                $m::Request::from_args(op, args)
                    .map_err(|e| error(id, wl_display::Error::INVALID_METHOD.0, format!("{}: {e}", iface.name)))?
            };
        }
        match iface.name {
            wl_display::NAME => self.display_request(c, parse!(wl_display)),
            wl_registry::NAME => self.registry_request(c, parse!(wl_registry)),
            wl_compositor::NAME => self.compositor_request(c, id, parse!(wl_compositor)),
            wl_region::NAME => self.region_request(c, id, parse!(wl_region)),
            wl_surface::NAME => self.surface_request(c, id, parse!(wl_surface)),
            wl_shm::NAME => self.shm_request(c, id, parse!(wl_shm)),
            wl_shm_pool::NAME => self.pool_request(c, id, parse!(wl_shm_pool)),
            wl_buffer::NAME => Ok(()), // destroy: the content was copied
            wl_output::NAME => Ok(()), // release
            wl_seat::NAME => self.seat_request(c, id, parse!(wl_seat)),
            wl_pointer::NAME => self.pointer_request(c, id, parse!(wl_pointer)),
            wl_keyboard::NAME | wl_touch::NAME => Ok(()), // release
            wl_subcompositor::NAME => self.subcompositor_request(c, id, parse!(wl_subcompositor)),
            wl_subsurface::NAME => self.subsurface_request(c, id, parse!(wl_subsurface)),
            wp_viewporter::NAME => self.viewporter_request(c, id, parse!(wp_viewporter)),
            wp_viewport::NAME => self.viewport_request(c, id, parse!(wp_viewport)),
            wl_data_device_manager::NAME => self.data_manager_request(c, id, parse!(wl_data_device_manager)),
            wl_data_source::NAME => self.data_source_request(c, id, parse!(wl_data_source)),
            wl_data_device::NAME => self.data_device_request(c, id, parse!(wl_data_device)),
            wl_data_offer::NAME => self.data_offer_request(c, id, parse!(wl_data_offer)),
            zwp_linux_dmabuf_v1::NAME => self.dmabuf_request(c, id, parse!(zwp_linux_dmabuf_v1)),
            zwp_linux_buffer_params_v1::NAME => self.params_request(c, id, parse!(zwp_linux_buffer_params_v1)),
            zwp_linux_dmabuf_feedback_v1::NAME => Ok(()), // destroy
            wp_fractional_scale_manager_v1::NAME => {
                self.fractional_manager_request(c, id, parse!(wp_fractional_scale_manager_v1))
            }
            wp_fractional_scale_v1::NAME => self.fractional_request(c, id, parse!(wp_fractional_scale_v1)),
            xdg_activation_v1::NAME => self.activation_request(c, id, parse!(xdg_activation_v1)),
            xdg_activation_token_v1::NAME => self.activation_token_request(c, id, parse!(xdg_activation_token_v1)),
            wp_cursor_shape_manager_v1::NAME => {
                self.cursor_shape_manager_request(c, id, parse!(wp_cursor_shape_manager_v1))
            }
            wp_cursor_shape_device_v1::NAME => self.cursor_shape_request(c, id, parse!(wp_cursor_shape_device_v1)),
            xdg_wm_base::NAME => self.wm_base_request(c, id, parse!(xdg_wm_base)),
            xdg_positioner::NAME => self.positioner_request(c, id, parse!(xdg_positioner)),
            xdg_surface::NAME => self.xdg_surface_request(c, id, parse!(xdg_surface)),
            xdg_toplevel::NAME => self.toplevel_request(c, id, parse!(xdg_toplevel)),
            xdg_popup::NAME => self.popup_request(c, id, parse!(xdg_popup)),
            zxdg_decoration_manager_v1::NAME => {
                self.decoration_manager_request(c, id, parse!(zxdg_decoration_manager_v1))
            }
            zxdg_toplevel_decoration_v1::NAME => self.decoration_request(c, id, parse!(zxdg_toplevel_decoration_v1)),
            wp_presentation::NAME => self.presentation_request(c, id, parse!(wp_presentation)),
            wp_presentation_feedback::NAME => Ok(()),
            xwayland_shell_v1::NAME => self.xwayland_shell_request(c, parse!(xwayland_shell_v1)),
            xwayland_surface_v1::NAME => self.xwayland_surface_request(c, id, parse!(xwayland_surface_v1)),
            _ => {
                self.close_fds(&args);
                Err(error(id, wl_display::Error::INVALID_METHOD.0, format!("{} is not implemented", iface.name)))
            }
        }
    }

    // -----------------------------------------------------------------
    // Frames, output, input.

    /// Draw what changed into `fb` (and present it); returns the physical
    /// rectangles drawn.
    pub fn render(&mut self, fb: &mut dyn Framebuffer) -> Vec<Rect> {
        self.scene.render(fb)
    }

    /// [`Server::render`] with layers below and above the windows (a
    /// shell's; [`crate::compose::Layer`]).
    pub fn render_with(
        &mut self,
        fb: &mut dyn Framebuffer,
        below: Option<&dyn crate::compose::Layer>,
        above: Option<&dyn crate::compose::Layer>,
    ) -> Vec<Rect> {
        self.scene.render_with(fb, below, above)
    }

    /// Whether a frame is wanted: something to draw, or clients waiting for
    /// frame callbacks or presentation feedback.
    pub fn wants_frame(&self) -> bool {
        self.scene.has_damage()
            || self.clients.values().any(|cl| {
                cl.objects.values().any(|o| match &o.data {
                    Data::Surface(s) => !s.frame_callbacks.is_empty() || !s.feedbacks.is_empty(),
                    _ => false,
                })
            })
    }

    /// A frame was shown at `time`: answer the frame callbacks committed
    /// before it (`wl_callback.done` with the time in milliseconds) and the
    /// presentation feedback. For a display shown as it is drawn; with page
    /// flips use [`Server::frame_submitted`] and [`Server::frame_completed`].
    pub fn frame_presented(&mut self, time: FrameTime) {
        self.frame_submitted();
        self.frame_completed(time, None, 0);
    }

    /// The frame just drawn went to the display (a page flip was queued):
    /// the frame callbacks and feedback committed so far belong to it.
    /// Called again before [`Server::frame_completed`], the newer ones
    /// join them.
    pub fn frame_submitted(&mut self) {
        for (id, client) in self.clients.iter_mut() {
            let flight = self.in_flight.entry(*id).or_default();
            for o in client.objects.values_mut() {
                if let Data::Surface(s) = &mut o.data {
                    flight.callbacks.append(&mut s.frame_callbacks);
                    let shown = s.window.is_some();
                    flight.feedbacks.extend(s.feedbacks.drain(..).map(|f| (f, shown)));
                }
            }
        }
    }

    /// Whether a submitted frame's callbacks or feedback still wait for
    /// [`Server::frame_completed`].
    pub fn frame_in_flight(&self) -> bool {
        self.in_flight.values().any(|f| !f.callbacks.is_empty() || !f.feedbacks.is_empty())
    }

    /// The submitted frame is on the screen since `time` (the vblank it
    /// was flipped at): answer its frame callbacks and feedback. `seq` is
    /// the display's vblank counter (else the server counts frames),
    /// `flags` the `wp_presentation_feedback.kind` bits
    /// ([`present_flags`]).
    pub fn frame_completed(&mut self, time: FrameTime, seq: Option<u64>, flags: u32) {
        self.frame_seq += 1;
        let ms = (time.monotonic_ns / 1_000_000) as u32;
        let refresh_ns =
            if self.output.refresh_mhz == 0 { 0 } else { 1_000_000_000_000 / u64::from(self.output.refresh_mhz) };
        let seq = seq.unwrap_or(self.frame_seq);
        for (id, flight) in core::mem::take(&mut self.in_flight) {
            let Some(client) = self.clients.get_mut(&id) else { continue };
            for cb in flight.callbacks {
                if client.get(cb).is_some() {
                    client.callback_done(cb, ms);
                }
            }
            let outputs = client.ids_where(|d| matches!(d, Data::Output));
            for (f, shown) in flight.feedbacks {
                if client.get(f).is_none() {
                    continue;
                }
                if shown {
                    for o in &outputs {
                        client.send(f, wp_presentation_feedback::Event::SyncOutput { output: *o });
                    }
                    let secs = time.monotonic_ns / 1_000_000_000;
                    client.send(
                        f,
                        wp_presentation_feedback::Event::Presented {
                            tv_sec_hi: (secs >> 32) as u32,
                            tv_sec_lo: secs as u32,
                            tv_nsec: (time.monotonic_ns % 1_000_000_000) as u32,
                            refresh: refresh_ns as u32,
                            seq_hi: (seq >> 32) as u32,
                            seq_lo: seq as u32,
                            flags,
                        },
                    );
                } else {
                    client.send(f, wp_presentation_feedback::Event::Discarded);
                }
                client.delete(f);
            }
        }
    }

    /// The seat's devices changed (the input stack's
    /// [`crate::seat::InputSource::capabilities`]): tell every bound seat.
    pub fn set_seat_capabilities(&mut self, caps: Capabilities) {
        self.seat.capabilities = caps;
        self.seat.ever = self.seat.ever.union(caps);
        for client in self.clients.values_mut() {
            for s in client.ids_where(|d| matches!(d, Data::Seat)) {
                client.send(s, wl_seat::Event::Capabilities { capabilities: caps.0 });
            }
        }
    }

    pub fn seat_capabilities(&self) -> Capabilities {
        self.seat.capabilities
    }

    /// The pointer's position, logical.
    pub fn pointer(&self) -> (Fixed, Fixed) {
        self.seat.pointer
    }
}

#[cfg(test)]
mod tests;
