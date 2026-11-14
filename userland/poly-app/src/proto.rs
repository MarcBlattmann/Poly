//! The Wayland client, without I/O: one `xdg_toplevel` window, its
//! shared-memory buffer, the seat (pointer and keyboard), cursor shapes and
//! the clipboard. Bytes and handles come in through [`Client::receive`] and
//! go out through [`Client::take_output`]; what the app has to hear comes
//! out of [`Client::poll_event`]. The runner (`run.rs`) moves the bytes over
//! the socket, and the tests move them to and from `poly-wayland`'s own
//! server in the same process.
//!
//! Messages are encoded and decoded with `poly-wayland`'s wire format and
//! the interface tables generated from the protocol XML, so the client
//! speaks exactly the protocol the compositor implements. Object IDs are
//! handed out upwards from 2 and never reused (a window's life is far
//! shorter than 2^24 objects); objects the server makes (`wl_data_offer`)
//! are learnt from the `new_id` arguments of its events.
//!
//! The order of things (the xdg-shell protocol's rules): the registry's
//! globals are bound as they are announced; after the first
//! `wl_display.sync` comes back every global is known, and the surface, its
//! `xdg_surface` and `xdg_toplevel` are made and committed without a
//! buffer; the first `configure` is acknowledged, and only then is a buffer
//! attached ([`Client::present`]). The window's size is in logical pixels;
//! the buffer is `scale` times larger. The scale is the exact fraction of
//! `wp_fractional_scale_v1` when the compositor has it and `wp_viewporter`
//! (a buffer of `round(size * scale)` pixels of buffer scale 1, shown at the
//! logical size through a viewport; ADR 0179), else the whole number of
//! `wl_surface.preferred_buffer_scale` or of the output.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use poly_wayland::compose::Scale;
use poly_wayland::protocol::*;
use poly_wayland::wire::{self, Arg, ArgKind, Fd, Fixed, Header, Interface, Message, NewObject, ObjectId};

/// The pixel format of the window's buffer: `XRGB8888`, one `u32` per pixel
/// (`0x00RRGGBB`), the format every compositor takes (`wl_shm.format`).
pub const FORMAT_XRGB8888: u32 = 1;

/// The cursor shapes an app asks for (`wp_cursor_shape_device_v1.shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Default,
    Pointer,
    Text,
    Wait,
}

impl CursorShape {
    fn code(self) -> u32 {
        match self {
            CursorShape::Default => wp_cursor_shape_device_v1::Shape::DEFAULT.0,
            CursorShape::Pointer => wp_cursor_shape_device_v1::Shape::POINTER.0,
            CursorShape::Text => wp_cursor_shape_device_v1::Shape::TEXT.0,
            CursorShape::Wait => wp_cursor_shape_device_v1::Shape::WAIT.0,
        }
    }
}

/// What the app hears from the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The window has this size (logical pixels) and scale now: draw it all.
    Resized {
        width: i32,
        height: i32,
        scale: Scale,
    },
    /// The user asked to close the window.
    Close,
    /// The keyboard came (true) or went (false).
    Focus(bool),
    /// A key went down (or repeats) or up: its evdev code, with the
    /// modifiers (XKB bits) at that moment.
    Key {
        code: u32,
        pressed: bool,
        depressed: u32,
        locked: u32,
    },
    /// The pointer is at (`x`, `y`) on the window (logical pixels).
    PointerMotion {
        x: f64,
        y: f64,
    },
    PointerLeave,
    /// A button (`BTN_LEFT` 272, `BTN_RIGHT` 273, `BTN_MIDDLE` 274).
    Button {
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// The wheel: positive scrolls down, in logical pixels (15 a notch).
    Scroll {
        dy: f64,
    },
    /// The compositor showed the last frame: a good moment for the next.
    FrameDone,
    /// Someone pastes what we put on the clipboard: write it into `fd` (in
    /// `mime`) and close it.
    SendClipboard {
        mime: String,
        fd: Fd,
    },
    /// The clipboard's contents changed hands (a text offer is there or not).
    ClipboardChanged {
        has_text: bool,
    },
    /// The connection failed: a protocol error from the compositor.
    Failed(String),
}

/// A global the registry announced.
#[derive(Clone, Debug)]
struct Global {
    name: u32,
    interface: String,
    version: u32,
}

/// The objects of the connection and the bytes going out.
#[derive(Debug, Default)]
pub struct Conn {
    out: Vec<u8>,
    out_fds: Vec<Fd>,
    next_id: u32,
    objects: BTreeMap<u32, &'static Interface>,
    input: Vec<u8>,
    fds: VecDeque<Fd>,
}

/// One message from the compositor.
#[derive(Debug)]
pub struct Incoming {
    pub object: ObjectId,
    pub interface: &'static str,
    pub opcode: u16,
    pub args: Vec<Arg>,
}

impl Conn {
    fn new() -> Conn {
        let mut c = Conn { next_id: 2, ..Conn::default() };
        c.objects.insert(1, &wl_display::INTERFACE);
        c
    }

    /// A new client-side object of `interface`.
    pub fn new_id(&mut self, interface: &'static Interface) -> ObjectId {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(id, interface);
        ObjectId(id)
    }

    /// Queue request `msg` of `object`.
    pub fn send(&mut self, object: ObjectId, msg: impl Message) {
        let opcode = msg.opcode();
        let args = msg.into_args();
        // A request too long for the wire is the app's error (a title of
        // 64 KiB); it is dropped rather than sent broken.
        let _ = wire::encode(&mut self.out, &mut self.out_fds, object, opcode, &args);
    }

    /// Forget a destroyed object: events for it are dropped from now on.
    pub fn forget(&mut self, object: ObjectId) {
        self.objects.remove(&object.0);
    }

    /// Take in bytes and handles; return the complete messages.
    fn receive(&mut self, bytes: &[u8], fds: Vec<Fd>, bad: &mut Option<String>) -> Vec<Incoming> {
        self.input.extend_from_slice(bytes);
        self.fds.extend(fds);
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(h) = Header::parse(&self.input[at..]) {
            let size = usize::from(h.size);
            if size < wire::HEADER_SIZE || size % 4 != 0 {
                *bad = Some(alloc::format!("bad message size {size}"));
                self.input.clear();
                return out;
            }
            if self.input.len() - at < size {
                break;
            }
            let body = &self.input[at + wire::HEADER_SIZE..at + size];
            at += size;
            let Some(iface) = self.objects.get(&h.object.0).copied() else {
                // An event for an object we destroyed: skip it, but take
                // the handles it carries, which we cannot know the count of
                // without its interface; none of ours carry any.
                continue;
            };
            let Some(desc) = iface.events.get(usize::from(h.opcode)) else {
                *bad = Some(alloc::format!("{}: no event {}", iface.name, h.opcode));
                continue;
            };
            let fds = &mut self.fds;
            match wire::decode_args(desc, body, &mut || fds.pop_front()) {
                Ok(args) => {
                    for (a, d) in args.iter().zip(desc.args) {
                        if let (Arg::NewId(id), ArgKind::NewId, Some(name)) = (a, d.kind, d.interface)
                            && let Some(i) = interface(name)
                        {
                            self.objects.insert(id.0, i);
                        }
                    }
                    if desc.destructor {
                        self.objects.remove(&h.object.0);
                    }
                    out.push(Incoming { object: h.object, interface: iface.name, opcode: h.opcode, args });
                }
                Err(e) => *bad = Some(alloc::format!("{}.{}: {e}", iface.name, desc.name)),
            }
        }
        self.input.drain(..at);
        out
    }
}

/// Where the window's pixels are: a shared memory pool of one buffer,
/// allocated by the runner (`run.rs`: a memory object mapped shared; the
/// tests: a vector).
#[derive(Debug)]
pub struct Buffer {
    pub pool: ObjectId,
    pub buffer: ObjectId,
    pub width: i32,
    pub height: i32,
}

/// The window and the connection's state.
#[derive(Debug)]
pub struct Client {
    pub conn: Conn,
    globals: Vec<Global>,
    registry: ObjectId,
    sync: Option<ObjectId>,
    compositor: Option<ObjectId>,
    shm: Option<ObjectId>,
    wm_base: Option<ObjectId>,
    seat: Option<ObjectId>,
    data_manager: Option<ObjectId>,
    cursor_manager: Option<ObjectId>,
    output: Option<ObjectId>,
    /// `wp_fractional_scale_manager_v1` and `wp_viewporter`, the surface's
    /// fractional scale object and viewport (made with the surface).
    fractional_manager: Option<ObjectId>,
    viewporter: Option<ObjectId>,
    fractional: Option<ObjectId>,
    viewport: Option<ObjectId>,
    surface: Option<ObjectId>,
    xdg_surface: Option<ObjectId>,
    toplevel: Option<ObjectId>,
    pointer: Option<ObjectId>,
    keyboard: Option<ObjectId>,
    cursor_device: Option<ObjectId>,
    data_device: Option<ObjectId>,
    frame: Option<ObjectId>,
    buffer: Option<Buffer>,
    /// The title and app ID to give the toplevel.
    title: String,
    app_id: String,
    /// The size the app wants when the compositor leaves it the choice.
    default_size: (i32, i32),
    min_size: (i32, i32),
    /// The configured size (logical), the scale, and whether the first
    /// configure came.
    size: (i32, i32),
    pending_size: (i32, i32),
    scale: Scale,
    output_scale: i32,
    surface_scale: Option<i32>,
    /// What the compositor prefers for the surface, exact
    /// (`wp_fractional_scale_v1.preferred_scale`).
    preferred_fraction: Option<Scale>,
    /// What was last told to the compositor: the viewport's destination
    /// (`None`: no viewport in use) and the buffer scale.
    shown_as: (Option<(i32, i32)>, i32),
    configured: bool,
    /// The pointer: where, the serial of its enter (for the cursor), the
    /// last button's serial.
    pointer_at: (f64, f64),
    enter_serial: u32,
    cursor: Option<CursorShape>,
    /// The keyboard: modifiers and the last key event's serial (the
    /// clipboard wants one).
    depressed: u32,
    locked: u32,
    pub key_serial: u32,
    /// Key repeat: per second, delay in milliseconds (`wl_keyboard`'s).
    pub repeat: (i32, i32),
    /// The keymap handle the compositor sent (closed by the runner).
    keymap_fds: Vec<Fd>,
    /// Offers being described (`wl_data_offer.offer`), and the current
    /// selection's.
    offers: BTreeMap<u32, Vec<String>>,
    selection: Option<(ObjectId, Vec<String>)>,
    /// What we put on the clipboard.
    source: Option<(ObjectId, String)>,
    events: VecDeque<Event>,
    failed: bool,
}

/// The MIME types of text a clipboard offer may carry, best first.
pub const TEXT_TYPES: [&str; 4] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"];

impl Client {
    /// A client for a window titled `title` (`app_id` names the app), of
    /// `default_size` logical pixels unless the compositor says otherwise.
    /// The first requests (the registry and a sync) are queued.
    pub fn new(title: &str, app_id: &str, default_size: (i32, i32), min_size: (i32, i32)) -> Client {
        let mut conn = Conn::new();
        let registry = conn.new_id(&wl_registry::INTERFACE);
        conn.send(ObjectId::DISPLAY, wl_display::Request::GetRegistry { registry });
        let sync = conn.new_id(&wl_callback::INTERFACE);
        conn.send(ObjectId::DISPLAY, wl_display::Request::Sync { callback: sync });
        Client {
            conn,
            globals: Vec::new(),
            registry,
            sync: Some(sync),
            compositor: None,
            shm: None,
            wm_base: None,
            seat: None,
            data_manager: None,
            cursor_manager: None,
            output: None,
            fractional_manager: None,
            viewporter: None,
            fractional: None,
            viewport: None,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            pointer: None,
            keyboard: None,
            cursor_device: None,
            data_device: None,
            frame: None,
            buffer: None,
            title: title.into(),
            app_id: app_id.into(),
            default_size,
            min_size,
            size: (0, 0),
            pending_size: (0, 0),
            scale: Scale::ONE,
            output_scale: 1,
            surface_scale: None,
            preferred_fraction: None,
            shown_as: (None, 1),
            configured: false,
            pointer_at: (0.0, 0.0),
            enter_serial: 0,
            cursor: None,
            depressed: 0,
            locked: 0,
            key_serial: 0,
            repeat: (25, 600),
            keymap_fds: Vec::new(),
            offers: BTreeMap::new(),
            selection: None,
            source: None,
            events: VecDeque::new(),
            failed: false,
        }
    }

    /// The bytes and handles to send.
    pub fn take_output(&mut self) -> (Vec<u8>, Vec<Fd>) {
        (core::mem::take(&mut self.conn.out), core::mem::take(&mut self.conn.out_fds))
    }

    /// Handles the client received and does not keep (keymaps), for the
    /// runner to close.
    pub fn take_unused_fds(&mut self) -> Vec<Fd> {
        core::mem::take(&mut self.keymap_fds)
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    pub fn configured(&self) -> bool {
        self.configured
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The window's size in logical pixels.
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    pub fn scale(&self) -> Scale {
        self.scale
    }

    /// The buffer's size in physical pixels ([`Scale::buffer_size`]).
    pub fn buffer_size(&self) -> (i32, i32) {
        self.scale.buffer_size(self.size.0, self.size.1)
    }

    pub fn modifiers(&self) -> (u32, u32) {
        (self.depressed, self.locked)
    }

    pub fn has_clipboard_text(&self) -> bool {
        self.selection.as_ref().is_some_and(|(_, types)| text_type(types).is_some())
    }

    /// Whether a frame callback is outstanding (drawing again before it
    /// comes only costs work the compositor would throw away).
    pub fn frame_pending(&self) -> bool {
        self.frame.is_some()
    }

    /// Take bytes and handles from the compositor.
    pub fn receive(&mut self, bytes: &[u8], fds: Vec<Fd>) {
        let mut bad = None;
        for m in self.conn.receive(bytes, fds, &mut bad) {
            if let Err(e) = self.dispatch(m) {
                bad = Some(e);
            }
        }
        if let Some(why) = bad {
            self.fail(why);
        }
    }

    fn fail(&mut self, why: String) {
        if !self.failed {
            self.failed = true;
            self.events.push_back(Event::Failed(why));
        }
    }

    fn global(&self, interface: &str) -> Option<&Global> {
        self.globals.iter().find(|g| g.interface == interface)
    }

    /// Bind global `interface` at most at version `max`.
    fn bind(&mut self, interface: &'static Interface, max: u32) -> Option<ObjectId> {
        let g = self.global(interface.name)?.clone();
        let version = g.version.min(max).min(interface.version);
        let id = self.conn.new_id(interface);
        let new = NewObject { interface: interface.name.into(), version, id };
        self.conn.send(self.registry, wl_registry::Request::Bind { name: g.name, id: new });
        Some(id)
    }

    fn version_of(&self, interface: &str) -> u32 {
        self.global(interface).map_or(0, |g| g.version)
    }

    fn dispatch(&mut self, m: Incoming) -> Result<(), String> {
        let bad = |e: wire::WireError| alloc::format!("{}: {e}", m.interface);
        let obj = m.object;
        match m.interface {
            "wl_display" => match wl_display::Event::from_args(m.opcode, m.args).map_err(bad)? {
                wl_display::Event::Error { object_id, code, message } => {
                    return Err(alloc::format!("error {code} on object {}: {message}", object_id.0));
                }
                wl_display::Event::DeleteId { .. } => {}
            },
            "wl_registry" => match wl_registry::Event::from_args(m.opcode, m.args).map_err(bad)? {
                wl_registry::Event::Global { name, interface, version } => {
                    self.globals.push(Global { name, interface, version });
                    if self.sync.is_none() {
                        // A global that came later (an output): bind what
                        // we still lack.
                        self.bind_globals();
                    }
                }
                wl_registry::Event::GlobalRemove { name } => self.globals.retain(|g| g.name != name),
            },
            "wl_callback" => {
                wl_callback::Event::from_args(m.opcode, m.args).map_err(bad)?;
                if self.sync == Some(obj) {
                    self.sync = None;
                    self.bind_globals();
                    self.make_window()?;
                } else if self.frame == Some(obj) {
                    self.frame = None;
                    self.events.push_back(Event::FrameDone);
                }
            }
            "wl_shm" => {}
            "wl_buffer" => {}
            "wl_output" => {
                if let wl_output::Event::Scale { factor } =
                    wl_output::Event::from_args(m.opcode, m.args).map_err(bad)?
                {
                    self.output_scale = factor.clamp(1, 8);
                    self.rescale();
                }
            }
            "wp_fractional_scale_v1" => {
                let wp_fractional_scale_v1::Event::PreferredScale { scale } =
                    wp_fractional_scale_v1::Event::from_args(m.opcode, m.args).map_err(bad)?;
                self.preferred_fraction = Some(Scale::from_120ths(scale));
                self.rescale();
            }
            "wl_surface" => {
                if let wl_surface::Event::PreferredBufferScale { factor } =
                    wl_surface::Event::from_args(m.opcode, m.args).map_err(bad)?
                {
                    self.surface_scale = Some(factor.clamp(1, 8));
                    self.rescale();
                }
            }
            "xdg_wm_base" => {
                let xdg_wm_base::Event::Ping { serial } =
                    xdg_wm_base::Event::from_args(m.opcode, m.args).map_err(bad)?;
                self.conn.send(obj, xdg_wm_base::Request::Pong { serial });
            }
            "xdg_surface" => {
                let xdg_surface::Event::Configure { serial } =
                    xdg_surface::Event::from_args(m.opcode, m.args).map_err(bad)?;
                self.conn.send(obj, xdg_surface::Request::AckConfigure { serial });
                let (mut w, mut h) = self.pending_size;
                if w <= 0 {
                    w = if self.size.0 > 0 { self.size.0 } else { self.default_size.0 };
                }
                if h <= 0 {
                    h = if self.size.1 > 0 { self.size.1 } else { self.default_size.1 };
                }
                let (w, h) = (w.max(self.min_size.0).max(1), h.max(self.min_size.1).max(1));
                let first = !self.configured;
                self.configured = true;
                if first || (w, h) != self.size {
                    self.size = (w, h);
                    self.resized();
                }
            }
            "xdg_toplevel" => match xdg_toplevel::Event::from_args(m.opcode, m.args).map_err(bad)? {
                xdg_toplevel::Event::Configure { width, height, .. } => self.pending_size = (width, height),
                xdg_toplevel::Event::Close => self.events.push_back(Event::Close),
                _ => {}
            },
            "wl_seat" => {
                if let wl_seat::Event::Capabilities { capabilities } =
                    wl_seat::Event::from_args(m.opcode, m.args).map_err(bad)?
                {
                    self.seat_capabilities(capabilities);
                }
            }
            "wl_pointer" => self.pointer_event(wl_pointer::Event::from_args(m.opcode, m.args).map_err(bad)?),
            "wl_keyboard" => self.keyboard_event(wl_keyboard::Event::from_args(m.opcode, m.args).map_err(bad)?),
            "wl_data_device" => match wl_data_device::Event::from_args(m.opcode, m.args).map_err(bad)? {
                wl_data_device::Event::DataOffer { id } => {
                    self.offers.insert(id.0, Vec::new());
                }
                wl_data_device::Event::Selection { id } => {
                    if let Some((old, _)) = self.selection.take() {
                        self.conn.send(old, wl_data_offer::Request::Destroy);
                        self.conn.forget(old);
                    }
                    if let Some(id) = id {
                        let types = self.offers.remove(&id.0).unwrap_or_default();
                        self.selection = Some((id, types));
                    }
                    let has_text = self.has_clipboard_text();
                    self.events.push_back(Event::ClipboardChanged { has_text });
                }
                wl_data_device::Event::Enter { id: Some(id), .. } => {
                    // Drag and drop is not taken: the offer is let go.
                    self.offers.remove(&id.0);
                    self.conn.send(id, wl_data_offer::Request::Destroy);
                    self.conn.forget(id);
                }
                _ => {}
            },
            "wl_data_offer" => {
                if let wl_data_offer::Event::Offer { mime_type } =
                    wl_data_offer::Event::from_args(m.opcode, m.args).map_err(bad)?
                    && let Some(types) = self.offers.get_mut(&obj.0)
                {
                    types.push(mime_type);
                }
            }
            "wl_data_source" => match wl_data_source::Event::from_args(m.opcode, m.args).map_err(bad)? {
                wl_data_source::Event::Send { mime_type, fd } => {
                    self.events.push_back(Event::SendClipboard { mime: mime_type, fd });
                }
                wl_data_source::Event::Cancelled => {
                    if self.source.as_ref().is_some_and(|(s, _)| *s == obj) {
                        self.source = None;
                    }
                    self.conn.send(obj, wl_data_source::Request::Destroy);
                    self.conn.forget(obj);
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    /// Bind the globals the window uses that are not bound yet.
    fn bind_globals(&mut self) {
        if self.compositor.is_none() {
            self.compositor = self.bind(&wl_compositor::INTERFACE, 6);
        }
        if self.shm.is_none() {
            self.shm = self.bind(&wl_shm::INTERFACE, 1);
        }
        if self.wm_base.is_none() {
            self.wm_base = self.bind(&xdg_wm_base::INTERFACE, 6);
        }
        if self.seat.is_none() {
            self.seat = self.bind(&wl_seat::INTERFACE, 7);
        }
        if self.data_manager.is_none() {
            self.data_manager = self.bind(&wl_data_device_manager::INTERFACE, 3);
        }
        if self.cursor_manager.is_none() {
            self.cursor_manager = self.bind(&wp_cursor_shape_manager_v1::INTERFACE, 1);
        }
        if self.output.is_none() {
            self.output = self.bind(&wl_output::INTERFACE, 3);
        }
        if self.fractional_manager.is_none() {
            self.fractional_manager = self.bind(&wp_fractional_scale_manager_v1::INTERFACE, 1);
        }
        if self.viewporter.is_none() {
            self.viewporter = self.bind(&wp_viewporter::INTERFACE, 1);
        }
        if let (Some(dm), Some(seat), None) = (self.data_manager, self.seat, self.data_device) {
            let id = self.conn.new_id(&wl_data_device::INTERFACE);
            self.conn.send(dm, wl_data_device_manager::Request::GetDataDevice { id, seat });
            self.data_device = Some(id);
        }
    }

    /// The surface and its toplevel role, committed without a buffer.
    fn make_window(&mut self) -> Result<(), String> {
        let (Some(compositor), Some(wm_base)) = (self.compositor, self.wm_base) else {
            return Err("the compositor has no wl_compositor or xdg_wm_base".into());
        };
        if self.shm.is_none() {
            return Err("the compositor has no wl_shm".into());
        }
        let surface = self.conn.new_id(&wl_surface::INTERFACE);
        self.conn.send(compositor, wl_compositor::Request::CreateSurface { id: surface });
        let xdg = self.conn.new_id(&xdg_surface::INTERFACE);
        self.conn.send(wm_base, xdg_wm_base::Request::GetXdgSurface { id: xdg, surface });
        let top = self.conn.new_id(&xdg_toplevel::INTERFACE);
        self.conn.send(xdg, xdg_surface::Request::GetToplevel { id: top });
        self.conn.send(top, xdg_toplevel::Request::SetTitle { title: self.title.clone() });
        self.conn.send(top, xdg_toplevel::Request::SetAppId { app_id: self.app_id.clone() });
        let (mw, mh) = self.min_size;
        if mw > 0 && mh > 0 {
            self.conn.send(top, xdg_toplevel::Request::SetMinSize { width: mw, height: mh });
        }
        // The exact scale, and a viewport to show a buffer of that scale at
        // the logical size, when the compositor has both.
        if let (Some(manager), Some(vp)) = (self.fractional_manager, self.viewporter) {
            let f = self.conn.new_id(&wp_fractional_scale_v1::INTERFACE);
            self.conn.send(manager, wp_fractional_scale_manager_v1::Request::GetFractionalScale { id: f, surface });
            let v = self.conn.new_id(&wp_viewport::INTERFACE);
            self.conn.send(vp, wp_viewporter::Request::GetViewport { id: v, surface });
            (self.fractional, self.viewport) = (Some(f), Some(v));
        }
        self.conn.send(surface, wl_surface::Request::Commit);
        self.surface = Some(surface);
        self.xdg_surface = Some(xdg);
        self.toplevel = Some(top);
        Ok(())
    }

    fn rescale(&mut self) {
        let whole = Scale::from_int(self.surface_scale.unwrap_or(self.output_scale).clamp(1, 8) as u32);
        // The exact fraction only with a viewport to show it through.
        let scale = match (self.preferred_fraction, self.viewport) {
            (Some(f), Some(_)) => f,
            _ => whole,
        };
        if scale != self.scale {
            self.scale = scale;
            if self.configured {
                self.resized();
            }
        }
    }

    fn resized(&mut self) {
        let (width, height) = self.size;
        self.events.push_back(Event::Resized { width, height, scale: self.scale });
    }

    fn seat_capabilities(&mut self, caps: u32) {
        let Some(seat) = self.seat else { return };
        if caps & 1 != 0 && self.pointer.is_none() {
            let id = self.conn.new_id(&wl_pointer::INTERFACE);
            self.conn.send(seat, wl_seat::Request::GetPointer { id });
            self.pointer = Some(id);
            if let Some(m) = self.cursor_manager {
                let dev = self.conn.new_id(&wp_cursor_shape_device_v1::INTERFACE);
                self.conn
                    .send(m, wp_cursor_shape_manager_v1::Request::GetPointer { cursor_shape_device: dev, pointer: id });
                self.cursor_device = Some(dev);
            }
        }
        if caps & 2 != 0 && self.keyboard.is_none() {
            let id = self.conn.new_id(&wl_keyboard::INTERFACE);
            self.conn.send(seat, wl_seat::Request::GetKeyboard { id });
            self.keyboard = Some(id);
        }
    }

    fn pointer_event(&mut self, e: wl_pointer::Event) {
        match e {
            wl_pointer::Event::Enter { serial, surface_x, surface_y, .. } => {
                self.enter_serial = serial;
                self.pointer_at = (surface_x.to_f64(), surface_y.to_f64());
                if let Some(shape) = self.cursor.take() {
                    self.set_cursor(shape);
                }
                self.events.push_back(Event::PointerMotion { x: self.pointer_at.0, y: self.pointer_at.1 });
            }
            wl_pointer::Event::Leave { .. } => self.events.push_back(Event::PointerLeave),
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                self.pointer_at = (surface_x.to_f64(), surface_y.to_f64());
                self.events.push_back(Event::PointerMotion { x: self.pointer_at.0, y: self.pointer_at.1 });
            }
            wl_pointer::Event::Button { serial, button, state, .. } => {
                self.key_serial = serial;
                let (x, y) = self.pointer_at;
                self.events.push_back(Event::Button { button, pressed: state == 1, x, y });
            }
            wl_pointer::Event::Axis { axis: 0, value, .. } => {
                self.events.push_back(Event::Scroll { dy: value.to_f64() });
            }
            _ => {}
        }
    }

    fn keyboard_event(&mut self, e: wl_keyboard::Event) {
        match e {
            wl_keyboard::Event::Keymap { fd, .. } => self.keymap_fds.push(fd),
            wl_keyboard::Event::Enter { serial, .. } => {
                self.key_serial = serial;
                self.events.push_back(Event::Focus(true));
            }
            wl_keyboard::Event::Leave { .. } => {
                self.depressed = 0;
                self.events.push_back(Event::Focus(false));
            }
            wl_keyboard::Event::Key { serial, key, state, .. } => {
                self.key_serial = serial;
                let (depressed, locked) = (self.depressed, self.locked);
                self.events.push_back(Event::Key { code: key, pressed: state == 1, depressed, locked });
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, .. } => {
                self.depressed = mods_depressed | mods_latched;
                self.locked = mods_locked;
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => self.repeat = (rate, delay),
        }
    }

    /// The pointer's shape over the window (remembered until the pointer
    /// comes in, as the shape needs the enter's serial).
    pub fn set_cursor(&mut self, shape: CursorShape) {
        match self.cursor_device {
            Some(dev) if self.enter_serial != 0 => {
                self.conn.send(
                    dev,
                    wp_cursor_shape_device_v1::Request::SetShape { serial: self.enter_serial, shape: shape.code() },
                );
            }
            _ => self.cursor = Some(shape),
        }
    }

    /// A new title for the window.
    pub fn set_title(&mut self, title: &str) {
        if title == self.title {
            return;
        }
        self.title = title.into();
        if let Some(top) = self.toplevel {
            self.conn.send(top, xdg_toplevel::Request::SetTitle { title: self.title.clone() });
        }
    }

    /// The window's buffer needs `size` bytes (`width` × `height` pixels):
    /// whether the current one is the wrong size.
    pub fn needs_buffer(&self) -> bool {
        let (w, h) = self.buffer_size();
        self.buffer.as_ref().is_none_or(|b| (b.width, b.height) != (w, h))
    }

    /// Make the buffer from a memory object `fd` of `width * height * 4`
    /// bytes (the client sends `fd`; the runner closes its copy after the
    /// send). The old buffer is destroyed.
    pub fn set_buffer(&mut self, fd: Fd, width: i32, height: i32) {
        let Some(shm) = self.shm else { return };
        if let Some(old) = self.buffer.take() {
            self.conn.send(old.buffer, wl_buffer::Request::Destroy);
            self.conn.send(old.pool, wl_shm_pool::Request::Destroy);
            self.conn.forget(old.buffer);
            self.conn.forget(old.pool);
        }
        let pool = self.conn.new_id(&wl_shm_pool::INTERFACE);
        let size = width * height * 4;
        self.conn.send(shm, wl_shm::Request::CreatePool { id: pool, fd, size });
        let buffer = self.conn.new_id(&wl_buffer::INTERFACE);
        self.conn.send(
            pool,
            wl_shm_pool::Request::CreateBuffer {
                id: buffer,
                offset: 0,
                width,
                height,
                stride: width * 4,
                format: FORMAT_XRGB8888,
            },
        );
        self.buffer = Some(Buffer { pool, buffer, width, height });
    }

    /// Tell the compositor how the buffer maps onto the window: a whole
    /// scale as the buffer scale, a fraction as a buffer of scale 1 shown at
    /// the logical size through the viewport. Only what changed is sent.
    fn show_as_scaled(&mut self, surface: ObjectId) {
        let want = match self.viewport {
            Some(_) if !self.scale.is_integer() => (Some(self.size), 1),
            _ => (None, self.scale.ceil() as i32),
        };
        if want == self.shown_as {
            return;
        }
        if want.1 != self.shown_as.1 {
            self.conn.send(surface, wl_surface::Request::SetBufferScale { scale: want.1 });
        }
        if want.0 != self.shown_as.0
            && let Some(v) = self.viewport
        {
            let (width, height) = want.0.unwrap_or((-1, -1));
            self.conn.send(v, wp_viewport::Request::SetDestination { width, height });
        }
        self.shown_as = want;
    }

    /// Show the buffer, `damage` (physical pixels) redrawn, and ask for a
    /// frame callback.
    pub fn present(&mut self, damage: &[poly_wayland::compose::Rect]) {
        let (Some(surface), Some(b)) = (self.surface, self.buffer.as_ref()) else { return };
        let buffer = b.buffer;
        self.show_as_scaled(surface);
        self.conn.send(surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
        for r in damage {
            self.conn
                .send(surface, wl_surface::Request::DamageBuffer { x: r.x, y: r.y, width: r.width, height: r.height });
        }
        if self.frame.is_none() {
            let cb = self.conn.new_id(&wl_callback::INTERFACE);
            self.conn.send(surface, wl_surface::Request::Frame { callback: cb });
            self.frame = Some(cb);
        }
        self.conn.send(surface, wl_surface::Request::Commit);
    }

    /// Put `text` on the clipboard (as the selection, with the last input
    /// event's serial).
    pub fn set_clipboard(&mut self, text: String) {
        let (Some(dm), Some(dev)) = (self.data_manager, self.data_device) else { return };
        let source = self.conn.new_id(&wl_data_source::INTERFACE);
        self.conn.send(dm, wl_data_device_manager::Request::CreateDataSource { id: source });
        for t in TEXT_TYPES {
            self.conn.send(source, wl_data_source::Request::Offer { mime_type: t.into() });
        }
        self.conn.send(dev, wl_data_device::Request::SetSelection { source: Some(source), serial: self.key_serial });
        if let Some((old, _)) = self.source.replace((source, text)) {
            self.conn.send(old, wl_data_source::Request::Destroy);
            self.conn.forget(old);
        }
    }

    /// What we put on the clipboard, for [`Event::SendClipboard`].
    pub fn clipboard_text(&self) -> Option<&str> {
        self.source.as_ref().map(|(_, t)| t.as_str())
    }

    /// Ask for the clipboard's text, written into `fd` (the write end of a
    /// pipe the runner reads). False if there is no text on it.
    pub fn receive_clipboard(&mut self, fd: Fd) -> bool {
        let Some((offer, types)) = self.selection.as_ref() else { return false };
        let Some(t) = text_type(types) else { return false };
        let (offer, mime_type) = (*offer, String::from(t));
        self.conn.send(offer, wl_data_offer::Request::Receive { mime_type, fd });
        true
    }

    /// The version of `interface` the compositor offers (0: none).
    pub fn offered(&self, interface: &str) -> u32 {
        self.version_of(interface)
    }

    /// Where the pointer is (logical pixels).
    pub fn pointer(&self) -> (f64, f64) {
        self.pointer_at
    }
}

/// The best text type among `types`.
fn text_type(types: &[String]) -> Option<&'static str> {
    TEXT_TYPES.iter().copied().find(|t| types.iter().any(|x| x == t))
}

/// A fixed-point value as `f64` (re-exported for the apps' tests).
pub fn fixed(v: f64) -> Fixed {
    Fixed::from_f64(v)
}
