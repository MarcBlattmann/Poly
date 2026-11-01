//! The server driven by a client written with the generated types, over
//! byte buffers; shared memory is test memory behind file descriptor
//! numbers.

use super::*;
use crate::compose::{MemoryFramebuffer, Theme};
use crate::seat::BTN_LEFT;
use crate::wire::{NewObject, decode_args, encode};
use alloc::vec;
use core::cell::RefCell;
use std::collections::BTreeMap as Map;

// ---------------------------------------------------------------------
// Test memory.

type Memory = Rc<RefCell<Vec<u8>>>;

#[derive(Default)]
struct Shared {
    files: Map<i32, Memory>,
    closed: Vec<i32>,
    /// dma-buf format tables handed out.
    tables: Vec<Vec<u8>>,
}

struct TestHost(Rc<RefCell<Shared>>);

struct TestMap {
    mem: Memory,
    len: usize,
    fd: i32,
    shared: Rc<RefCell<Shared>>,
}

impl ShmMap for TestMap {
    fn len(&self) -> usize {
        self.len
    }

    fn resize(&mut self, size: usize) -> bool {
        if size > self.mem.borrow().len() {
            return false;
        }
        self.len = size;
        true
    }

    fn read(&self, offset: usize, dst: &mut [u8]) -> bool {
        let mem = self.mem.borrow();
        let end = offset + dst.len();
        if end > self.len || end > mem.len() {
            return false;
        }
        dst.copy_from_slice(&mem[offset..end]);
        true
    }
}

impl Drop for TestMap {
    fn drop(&mut self) {
        self.shared.borrow_mut().closed.push(self.fd);
    }
}

impl Host for TestHost {
    fn map_shm(&mut self, fd: Fd, size: usize) -> Option<Box<dyn ShmMap>> {
        let mem = self.0.borrow().files.get(&fd.0).cloned();
        match mem {
            Some(mem) if mem.borrow().len() >= size => {
                Some(Box::new(TestMap { mem, len: size, fd: fd.0, shared: Rc::clone(&self.0) }))
            }
            _ => {
                self.close_fd(fd);
                None
            }
        }
    }

    fn close_fd(&mut self, fd: Fd) {
        self.0.borrow_mut().closed.push(fd.0);
    }

    fn keymap(&mut self) -> Option<(Fd, u32)> {
        Some((Fd(KEYMAP_FD), 1234))
    }

    fn dmabuf_device(&mut self) -> Option<u64> {
        Some(DMABUF_DEVICE)
    }

    /// A dma-buf is test memory like a pool's file, all of it mapped.
    fn map_dmabuf(&mut self, fd: Fd) -> Option<Box<dyn ShmMap>> {
        let len = self.0.borrow().files.get(&fd.0).map(|m| m.borrow().len())?;
        self.map_shm(fd, len)
    }

    fn dmabuf_table(&mut self, table: &[u8]) -> Option<Fd> {
        self.0.borrow_mut().tables.push(table.to_vec());
        Some(Fd(TABLE_FD))
    }
}

/// What the test host hands out as the keymap's file descriptor, the
/// format table's, and the DRM device number it names.
const KEYMAP_FD: i32 = 999;
const TABLE_FD: i32 = 998;
const DMABUF_DEVICE: u64 = 226 << 8 | 128;

// ---------------------------------------------------------------------
// A client.

/// An event the client received.
#[derive(Clone, Debug)]
struct Received {
    object: ObjectId,
    interface: &'static Interface,
    opcode: u16,
    args: Vec<Arg>,
}

impl Received {
    fn name(&self) -> (&'static str, &'static str) {
        (self.interface.name, self.interface.events[usize::from(self.opcode)].name)
    }
}

struct Peer {
    id: ClientId,
    next: u32,
    objects: Map<u32, &'static Interface>,
    events: Vec<Received>,
    globals: Map<String, (u32, u32)>,
    shared: Rc<RefCell<Shared>>,
    next_fd: i32,
}

impl Peer {
    fn new_id(&mut self, interface: &'static Interface) -> ObjectId {
        self.next += 1;
        self.objects.insert(self.next, interface);
        ObjectId(self.next)
    }

    /// An ID the server freed (`wl_display.delete_id` came) for a new
    /// object, as libwayland's client reuses them.
    fn reuse_id(&mut self, id: ObjectId, interface: &'static Interface) -> ObjectId {
        assert!(!self.objects.contains_key(&id.0), "{id} was not freed by delete_id");
        self.objects.insert(id.0, interface);
        id
    }

    /// Send a request (and its file descriptors), then read what comes back.
    fn send(&mut self, srv: &mut Server, object: ObjectId, req: impl Message) {
        let (mut bytes, mut fds) = (Vec::new(), Vec::new());
        encode(&mut bytes, &mut fds, object, req.opcode(), &req.into_args()).unwrap();
        srv.receive(self.id, &bytes, fds);
        self.read(srv);
    }

    fn read(&mut self, srv: &mut Server) {
        let (bytes, fds) = srv.take_output(self.id);
        let mut fds = fds.into_iter();
        let mut at = 0;
        while at < bytes.len() {
            let h = Header::parse(&bytes[at..]).unwrap();
            let interface = self.objects[&h.object.0];
            let desc = &interface.events[usize::from(h.opcode)];
            let body = &bytes[at + 8..at + usize::from(h.size)];
            let args = decode_args(desc, body, &mut || fds.next()).unwrap();
            // Objects the server made (a data offer, a dma-buf's wl_buffer).
            for (d, a) in desc.args.iter().zip(&args) {
                if let (Arg::NewId(id), Some(name)) = (a, d.interface) {
                    self.objects.insert(id.0, crate::protocol::interface(name).unwrap());
                }
            }
            if let (wl_display::NAME, "delete_id") = (interface.name, desc.name) {
                let Arg::Uint(id) = args[0] else { panic!() };
                self.objects.remove(&id);
            }
            self.events.push(Received { object: h.object, interface, opcode: h.opcode, args });
            at += usize::from(h.size);
        }
    }

    fn take(&mut self) -> Vec<Received> {
        core::mem::take(&mut self.events)
    }

    /// The names of the events received, in order, and forget them.
    fn names(&mut self) -> Vec<(&'static str, &'static str)> {
        self.take().iter().map(Received::name).collect()
    }

    fn bind(&mut self, srv: &mut Server, registry: ObjectId, interface: &'static Interface, version: u32) -> ObjectId {
        let name = self.globals[interface.name].0;
        let id = self.new_id(interface);
        let new = NewObject { interface: interface.name.into(), version, id };
        self.send(srv, registry, wl_registry::Request::Bind { name, id: new });
        id
    }

    /// A shared memory file of `len` bytes, as a file descriptor number.
    fn memfd(&mut self, len: usize) -> (Fd, Memory) {
        self.next_fd += 1;
        let mem = Rc::new(RefCell::new(vec![0u8; len]));
        self.shared.borrow_mut().files.insert(self.next_fd, Rc::clone(&mem));
        (Fd(self.next_fd), mem)
    }

    /// The object and code of the protocol error the client got.
    fn code(&self) -> Option<(ObjectId, u32)> {
        self.error().map(|(o, c, _)| (o, c))
    }

    /// The protocol error the client got, if any: object, code, message.
    fn error(&self) -> Option<(ObjectId, u32, String)> {
        self.events.iter().find(|e| e.name() == ("wl_display", "error")).map(|e| match &e.args[..] {
            [Arg::Object(Some(o)), Arg::Uint(code), Arg::Str(Some(m))] => (*o, *code, m.clone()),
            _ => panic!("malformed error"),
        })
    }
}

fn server_with(output: OutputConfig) -> (Server, Rc<RefCell<Shared>>) {
    let shared = Rc::new(RefCell::new(Shared::default()));
    let srv = Server::new(Config { output, ..Config::default() }, Box::new(TestHost(Rc::clone(&shared))));
    (srv, shared)
}

fn server() -> (Server, Rc<RefCell<Shared>>) {
    server_with(OutputConfig { width: 640, height: 480, ..OutputConfig::default() })
}

/// A connected client with its registry (object 2) read.
fn connect(srv: &mut Server, shared: &Rc<RefCell<Shared>>) -> (Peer, ObjectId) {
    let id = srv.add_client();
    let mut p = Peer {
        id,
        next: 1,
        objects: Map::from([(1, &wl_display::INTERFACE)]),
        events: Vec::new(),
        globals: Map::new(),
        shared: Rc::clone(shared),
        next_fd: 100 * id.0 as i32,
    };
    let registry = p.new_id(&wl_registry::INTERFACE);
    p.send(srv, ObjectId::DISPLAY, wl_display::Request::GetRegistry { registry });
    for e in p.take() {
        if let [Arg::Uint(name), Arg::Str(Some(iface)), Arg::Uint(version)] = &e.args[..] {
            p.globals.insert(iface.clone(), (*name, *version));
        }
    }
    (p, registry)
}

/// The objects of a mapped toplevel.
struct Window {
    surface: ObjectId,
    xdg: ObjectId,
    toplevel: ObjectId,
    buffer: ObjectId,
    mem: Memory,
}

/// Bind what a window needs: compositor, shm, wm_base.
struct Globals {
    compositor: ObjectId,
    shm: ObjectId,
    wm_base: ObjectId,
}

fn globals(p: &mut Peer, srv: &mut Server, registry: ObjectId) -> Globals {
    let compositor = p.bind(srv, registry, &wl_compositor::INTERFACE, 6);
    let shm = p.bind(srv, registry, &wl_shm::INTERFACE, 1);
    let wm_base = p.bind(srv, registry, &xdg_wm_base::INTERFACE, 6);
    p.take();
    Globals { compositor, shm, wm_base }
}

/// The serial of the last `xdg_surface.configure` among `events`.
fn configure_serial(events: &[Received]) -> u32 {
    let e = events.iter().rev().find(|e| e.name() == ("xdg_surface", "configure")).expect("a configure");
    let Arg::Uint(serial) = e.args[0] else { panic!() };
    serial
}

/// A toplevel up to its first configure (not acked): returns its objects
/// but the buffer, and the events of the initial commit.
fn toplevel(p: &mut Peer, srv: &mut Server, g: &Globals, title: &str) -> (ObjectId, ObjectId, ObjectId, Vec<Received>) {
    let surface = p.new_id(&wl_surface::INTERFACE);
    p.send(srv, g.compositor, wl_compositor::Request::CreateSurface { id: surface });
    let xdg = p.new_id(&xdg_surface::INTERFACE);
    p.send(srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: xdg, surface });
    let toplevel = p.new_id(&xdg_toplevel::INTERFACE);
    p.send(srv, xdg, xdg_surface::Request::GetToplevel { id: toplevel });
    p.send(srv, toplevel, xdg_toplevel::Request::SetTitle { title: title.into() });
    p.send(srv, surface, wl_surface::Request::Commit);
    (surface, xdg, toplevel, p.take())
}

/// A buffer of `w` × `h` `format` pixels filled with `pixel`, in a pool of
/// its own.
fn buffer(p: &mut Peer, srv: &mut Server, g: &Globals, w: i32, h: i32, pixel: u32, format: u32) -> (ObjectId, Memory) {
    let (fd, mem) = p.memfd((w * h * 4) as usize);
    for px in mem.borrow_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&pixel.to_le_bytes());
    }
    let pool = p.new_id(&wl_shm_pool::INTERFACE);
    p.send(srv, g.shm, wl_shm::Request::CreatePool { id: pool, fd, size: w * h * 4 });
    let buffer = p.new_id(&wl_buffer::INTERFACE);
    p.send(
        srv,
        pool,
        wl_shm_pool::Request::CreateBuffer { id: buffer, offset: 0, width: w, height: h, stride: w * 4, format },
    );
    p.send(srv, pool, wl_shm_pool::Request::Destroy);
    p.take();
    (buffer, mem)
}

/// A mapped toplevel of `w` × `h` pixels of `pixel`.
fn mapped(p: &mut Peer, srv: &mut Server, g: &Globals, title: &str, w: i32, h: i32, pixel: u32) -> Window {
    let (surface, xdg, toplevel, events) = toplevel(p, srv, g, title);
    p.send(srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    let (buffer, mem) = buffer(p, srv, g, w, h, pixel, wl_shm::Format::XRGB8888.0);
    p.send(srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(srv, surface, wl_surface::Request::DamageBuffer { x: 0, y: 0, width: w, height: h });
    p.send(srv, surface, wl_surface::Request::Commit);
    Window { surface, xdg, toplevel, buffer, mem }
}

// ---------------------------------------------------------------------
// The tests.

#[test]
fn the_registry_lists_the_globals_and_sync_answers() {
    let (mut srv, shared) = server();
    let (mut p, _) = connect(&mut srv, &shared);
    let names: Vec<&str> = p.globals.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "wl_compositor",
            "wl_data_device_manager",
            "wl_output",
            "wl_seat",
            "wl_shm",
            "wl_subcompositor",
            "wp_cursor_shape_manager_v1",
            "wp_fractional_scale_manager_v1",
            "wp_presentation",
            "wp_viewporter",
            "xdg_activation_v1",
            "xdg_wm_base",
            "zwp_linux_dmabuf_v1",
            "zxdg_decoration_manager_v1"
        ]
    );
    assert_eq!(p.globals["wl_compositor"].1, versions::COMPOSITOR);
    assert_eq!(p.globals["zxdg_decoration_manager_v1"].1, 2);
    let cb = p.new_id(&wl_callback::INTERFACE);
    p.send(&mut srv, ObjectId::DISPLAY, wl_display::Request::Sync { callback: cb });
    let e = p.take();
    assert_eq!(
        e.iter().map(Received::name).collect::<Vec<_>>(),
        [("wl_callback", "done"), ("wl_display", "delete_id")]
    );
    assert_eq!(e[1].args, [Arg::Uint(cb.0)]);
    assert!(srv.failure(p.id).is_none());
}

#[test]
fn binding_checks_name_interface_and_version() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let id = p.new_id(&wl_compositor::INTERFACE);
    let name = p.globals["wl_compositor"].0;
    let new = NewObject { interface: "wl_compositor".into(), version: 99, id };
    p.send(&mut srv, registry, wl_registry::Request::Bind { name, id: new });
    let (object, code, msg) = p.error().unwrap();
    assert_eq!((object, code), (ObjectId::DISPLAY, wl_display::Error::INVALID_OBJECT.0));
    assert!(msg.contains("version 99"), "{msg}");
    assert!(srv.failure(p.id).is_some());
    // Nothing more is handled for a failed client.
    p.take();
    p.send(&mut srv, ObjectId::DISPLAY, wl_display::Request::Sync { callback: ObjectId(50) });
    assert!(p.take().is_empty());
    // Another client: a wrong interface for the name.
    let (mut q, registry) = connect(&mut srv, &shared);
    let id = q.new_id(&wl_shm::INTERFACE);
    let new = NewObject { interface: "wl_shm".into(), version: 1, id };
    q.send(&mut srv, registry, wl_registry::Request::Bind { name, id: new });
    assert!(q.error().unwrap().2.contains("is a wl_compositor"));
}

#[test]
fn bad_objects_opcodes_and_ids_are_protocol_errors() {
    let (mut srv, shared) = server();
    // A request to an object that does not exist.
    let (mut p, _) = connect(&mut srv, &shared);
    p.objects.insert(77, &wl_surface::INTERFACE);
    p.send(&mut srv, ObjectId(77), wl_surface::Request::Commit);
    assert_eq!(p.error().unwrap().1, wl_display::Error::INVALID_OBJECT.0);
    // An opcode the interface does not have.
    let (mut p, registry) = connect(&mut srv, &shared);
    let mut bytes = Vec::new();
    encode(&mut bytes, &mut Vec::new(), registry, 5, &[]).unwrap();
    srv.receive(p.id, &bytes, Vec::new());
    p.read(&mut srv);
    assert_eq!(p.error().unwrap(), (registry, wl_display::Error::INVALID_METHOD.0, "wl_registry: no request 5".into()));
    // A new_id already in use.
    let (mut p, registry) = connect(&mut srv, &shared);
    p.send(&mut srv, ObjectId::DISPLAY, wl_display::Request::GetRegistry { registry });
    assert!(p.error().unwrap().2.contains("in use"));
    // A request newer than the object.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    let shm = p.bind(&mut srv, registry, &wl_shm::INTERFACE, 1);
    p.send(&mut srv, shm, wl_shm::Request::Release);
    assert!(p.error().unwrap().2.contains("needs version 2"));
    // A message whose size is not a whole number of words.
    let (mut p, _) = connect(&mut srv, &shared);
    srv.receive(p.id, &[1, 0, 0, 0, 0, 0, 9, 0], Vec::new());
    p.read(&mut srv);
    assert!(p.error().unwrap().2.contains("message size 9"));
}

#[test]
fn messages_may_arrive_in_pieces() {
    let (mut srv, shared) = server();
    let (mut p, _) = connect(&mut srv, &shared);
    let cb = p.new_id(&wl_callback::INTERFACE);
    let mut bytes = Vec::new();
    encode(&mut bytes, &mut Vec::new(), ObjectId::DISPLAY, 0, &[Arg::NewId(cb)]).unwrap();
    for b in &bytes {
        srv.receive(p.id, &[*b], Vec::new());
    }
    p.read(&mut srv);
    assert_eq!(p.names(), [("wl_callback", "done"), ("wl_display", "delete_id")]);
}

#[test]
fn shm_announces_formats_and_checks_buffers() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let shm = p.bind(&mut srv, registry, &wl_shm::INTERFACE, 2);
    let formats: Vec<Arg> = p.take().into_iter().flat_map(|e| e.args).collect();
    // argb8888, xrgb8888, abgr8888 and xbgr8888 (fourcc AB24, XB24).
    let ab24 = u32::from_le_bytes(*b"AB24");
    let xb24 = u32::from_le_bytes(*b"XB24");
    assert_eq!(formats, [Arg::Uint(0), Arg::Uint(1), Arg::Uint(ab24), Arg::Uint(xb24)]);
    let (fd, _mem) = p.memfd(4096);
    let pool = p.new_id(&wl_shm_pool::INTERFACE);
    p.send(&mut srv, shm, wl_shm::Request::CreatePool { id: pool, fd, size: 4096 });
    let b = p.new_id(&wl_buffer::INTERFACE);
    p.send(
        &mut srv,
        pool,
        wl_shm_pool::Request::CreateBuffer { id: b, offset: 0, width: 32, height: 32, stride: 128, format: 0 },
    );
    assert!(p.error().is_none());
    // Growing works (the file is big enough), shrinking does not.
    p.send(&mut srv, pool, wl_shm_pool::Request::Resize { size: 4096 });
    assert!(p.error().is_none());
    // Too big for the pool.
    let b2 = p.new_id(&wl_buffer::INTERFACE);
    p.send(
        &mut srv,
        pool,
        wl_shm_pool::Request::CreateBuffer { id: b2, offset: 4, width: 32, height: 32, stride: 128, format: 1 },
    );
    assert_eq!(p.error().unwrap().1, wl_shm_pool::Error::INVALID_STRIDE.0);
    // An unsupported format.
    let (mut p, registry) = connect(&mut srv, &shared);
    let shm = p.bind(&mut srv, registry, &wl_shm::INTERFACE, 1);
    let (fd, _mem) = p.memfd(64);
    let pool = p.new_id(&wl_shm_pool::INTERFACE);
    p.send(&mut srv, shm, wl_shm::Request::CreatePool { id: pool, fd, size: 64 });
    let b = p.new_id(&wl_buffer::INTERFACE);
    let rgb565 = wl_shm::Format::RGB565.0;
    p.send(
        &mut srv,
        pool,
        wl_shm_pool::Request::CreateBuffer { id: b, offset: 0, width: 4, height: 4, stride: 16, format: rgb565 },
    );
    assert_eq!(p.error().unwrap().1, wl_shm_pool::Error::INVALID_FORMAT.0);
    // A file that cannot be mapped: invalid_fd, and the fd is closed.
    let (mut p, registry) = connect(&mut srv, &shared);
    let shm = p.bind(&mut srv, registry, &wl_shm::INTERFACE, 1);
    let pool = p.new_id(&wl_shm_pool::INTERFACE);
    p.send(&mut srv, shm, wl_shm::Request::CreatePool { id: pool, fd: Fd(999), size: 64 });
    assert_eq!(p.error().unwrap().1, wl_shm::Error::INVALID_FD.0);
    assert!(shared.borrow().closed.contains(&999));
}

/// The whole life of a window: configure, ack, buffer, map, frame
/// callback, composing.
#[test]
fn a_toplevel_is_configured_mapped_framed_and_composed() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let output = p.bind(&mut srv, registry, &wl_output::INTERFACE, 4);
    p.take();
    let (surface, xdg, tl, events) = toplevel(&mut p, &mut srv, &g, "Hello");
    let names: Vec<_> = events.iter().map(Received::name).collect();
    assert_eq!(
        names,
        [
            ("wl_surface", "preferred_buffer_scale"),
            ("xdg_toplevel", "configure_bounds"),
            ("xdg_toplevel", "wm_capabilities"),
            ("xdg_toplevel", "configure"),
            ("xdg_surface", "configure"),
        ]
    );
    // Size up to the client, activated.
    let c = &events[3].args;
    assert_eq!(c[..2], [Arg::Int(0), Arg::Int(0)]);
    assert_eq!(c[2], Arg::Array(xdg_toplevel::State::ACTIVATED.0.to_ne_bytes().to_vec()));
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    let (buffer, _mem) = buffer(&mut p, &mut srv, &g, 100, 50, 0x00_33_66_99, wl_shm::Format::XRGB8888.0);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::DamageBuffer { x: 0, y: 0, width: 100, height: 50 });
    let frame = p.new_id(&wl_callback::INTERFACE);
    p.send(&mut srv, surface, wl_surface::Request::Frame { callback: frame });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    let e = p.take();
    let names: Vec<_> = e.iter().map(Received::name).collect();
    assert_eq!(names, [("wl_surface", "enter"), ("wl_buffer", "release")]);
    assert_eq!(e[0].args, [Arg::Object(Some(output))]);
    // In the scene: framed, titled, active, placed.
    let w = &srv.scene().windows()[0];
    assert_eq!((w.title(), w.is_decorated(), w.is_active()), ("Hello", true, true));
    assert_eq!(w.position(), (32 + crate::compose::BORDER, 32 + crate::compose::TITLE_HEIGHT + crate::compose::BORDER));
    assert_eq!(w.content().logical_size(), (100, 50));
    let (x, y) = w.position();
    let mut fb = MemoryFramebuffer::new(640, 480);
    srv.render(&mut fb);
    assert_eq!(fb.pixel(x as u32, y as u32), Some(0x33_66_99));
    assert_eq!(fb.pixel((x + 99) as u32, (y + 49) as u32), Some(0x33_66_99));
    assert_eq!(fb.pixel((x + 50) as u32, (y - 2) as u32), Some(Theme::NEUTRAL.title_active));
    assert_eq!(fb.pixel(5, 5), Some(Theme::NEUTRAL.background));
    // The frame callback fires with the frame, then is gone.
    assert!(srv.wants_frame());
    srv.frame_presented(super::FrameTime { monotonic_ns: 5_000_000_000 });
    p.read(&mut srv);
    let e = p.take();
    assert_eq!(
        e.iter().map(Received::name).collect::<Vec<_>>(),
        [("wl_callback", "done"), ("wl_display", "delete_id")]
    );
    assert_eq!((e[0].object, &e[0].args[..]), (frame, &[Arg::Uint(5000)][..]));
    assert!(!srv.wants_frame());
    // A new title shows at once (titles are not double-buffered).
    p.send(&mut srv, tl, xdg_toplevel::Request::SetTitle { title: "Bye".into() });
    assert_eq!(srv.scene().windows()[0].title(), "Bye");
}

#[test]
fn only_damaged_pixels_are_copied() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "d", 8, 8, 0x11_11_11);
    // The client draws everything new but damages one pixel.
    for px in w.mem.borrow_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&0x00ee_eeeeu32.to_le_bytes());
    }
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(w.buffer), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::DamageBuffer { x: 2, y: 3, width: 1, height: 1 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let content = srv.scene().windows()[0].content();
    assert_eq!(content.pixels[3 * 8 + 2], 0xffee_eeee);
    assert_eq!(content.pixels[0], 0xff11_1111);
    assert_eq!(content.pixels.iter().filter(|p| **p == 0xffee_eeee).count(), 1);
    // Surface damage counts too (in surface coordinates).
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(w.buffer), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Damage { x: 0, y: 0, width: 2, height: 1 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let content = srv.scene().windows()[0].content();
    assert_eq!(content.pixels.iter().filter(|p| **p == 0xffee_eeee).count(), 3);
}

#[test]
fn argb_buffers_are_blended_premultiplied() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (surface, xdg, _tl, events) = toplevel(&mut p, &mut srv, &g, "a");
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    // Half-transparent black, premultiplied.
    let (buffer, _) = buffer(&mut p, &mut srv, &g, 4, 4, 0x8000_0000, wl_shm::Format::ARGB8888.0);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    let mut fb = MemoryFramebuffer::new(640, 480);
    srv.render(&mut fb);
    let (x, y) = srv.scene().windows()[0].position();
    // Over the background 0x202020: 0x20 * 127 / 255 = 0x10.
    assert_eq!(fb.pixel(x as u32, y as u32), Some(0x10_10_10));
}

#[test]
fn buffers_need_an_acked_configure() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (surface, xdg, _tl, _events) = toplevel(&mut p, &mut srv, &g, "early");
    let (buffer, _) = buffer(&mut p, &mut srv, &g, 4, 4, 0, 1);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    assert_eq!(p.code(), Some((xdg, xdg_surface::Error::UNCONFIGURED_BUFFER.0)));
}

#[test]
fn acks_must_name_a_sent_serial() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (_s, xdg, _tl, events) = toplevel(&mut p, &mut srv, &g, "ack");
    let serial = configure_serial(&events);
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial });
    assert!(p.error().is_none());
    // The same serial twice.
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial });
    let (o, code, _) = p.error().unwrap();
    assert_eq!((o, code), (xdg, xdg_surface::Error::INVALID_SERIAL.0));
}

#[test]
fn role_rules() {
    let (mut srv, shared) = server();
    // A surface destroyed before its xdg_surface.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (surface, _xdg, _tl, _) = toplevel(&mut p, &mut srv, &g, "r");
    p.send(&mut srv, surface, wl_surface::Request::Destroy);
    assert_eq!(p.code(), Some((surface, wl_surface::Error::DEFUNCT_ROLE_OBJECT.0)));
    // An xdg_surface destroyed before its toplevel.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (_s, xdg, _tl, _) = toplevel(&mut p, &mut srv, &g, "r");
    p.send(&mut srv, xdg, xdg_surface::Request::Destroy);
    assert_eq!(p.code(), Some((xdg, xdg_surface::Error::DEFUNCT_ROLE_OBJECT.0)));
    // Two xdg_surfaces for one surface.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (surface, _, _, _) = toplevel(&mut p, &mut srv, &g, "r");
    let again = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: again, surface });
    assert_eq!(p.code(), Some((g.wm_base, xdg_wm_base::Error::ROLE.0)));
    // A commit on an xdg_surface without a role object.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let surface = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: surface });
    let xdg = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: xdg, surface });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    assert_eq!(p.code(), Some((xdg, xdg_surface::Error::NOT_CONSTRUCTED.0)));
    // Destroying the wm_base with surfaces alive.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    toplevel(&mut p, &mut srv, &g, "r");
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::Destroy);
    assert_eq!(p.code(), Some((g.wm_base, xdg_wm_base::Error::DEFUNCT_SURFACES.0)));
}

#[test]
fn surface_requests_are_checked() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    p.send(&mut srv, s, wl_surface::Request::SetBufferScale { scale: 0 });
    assert_eq!(p.code(), Some((s, wl_surface::Error::INVALID_SCALE.0)));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    p.send(&mut srv, s, wl_surface::Request::Attach { buffer: None, x: 1, y: 0 });
    assert_eq!(p.code(), Some((s, wl_surface::Error::INVALID_OFFSET.0)));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    p.send(&mut srv, s, wl_surface::Request::SetBufferTransform { transform: 8 });
    assert_eq!(p.code(), Some((s, wl_surface::Error::INVALID_TRANSFORM.0)));
    // A buffer that is not a multiple of the scale.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    let (b, _) = buffer(&mut p, &mut srv, &g, 5, 4, 0, 1);
    p.send(&mut srv, s, wl_surface::Request::SetBufferScale { scale: 2 });
    p.send(&mut srv, s, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(&mut srv, s, wl_surface::Request::Commit);
    assert_eq!(p.code(), Some((s, wl_surface::Error::INVALID_SIZE.0)));
    // A surface without a role takes buffers (and releases them).
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    let (b, _) = buffer(&mut p, &mut srv, &g, 4, 4, 0, 1);
    p.send(&mut srv, s, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(&mut srv, s, wl_surface::Request::Commit);
    assert_eq!(p.names(), [("wl_buffer", "release")]);
    assert!(srv.scene().windows().is_empty());
}

#[test]
fn hidpi_outputs_and_buffer_scales() {
    let (mut srv, shared) = server_with(OutputConfig { width: 1280, height: 960, scale: 2, ..OutputConfig::default() });
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    p.bind(&mut srv, registry, &wl_output::INTERFACE, 4);
    let e = p.take();
    let names: Vec<_> = e.iter().map(Received::name).collect();
    assert_eq!(
        names,
        [
            ("wl_output", "geometry"),
            ("wl_output", "mode"),
            ("wl_output", "scale"),
            ("wl_output", "name"),
            ("wl_output", "description"),
            ("wl_output", "done")
        ]
    );
    assert_eq!(e[1].args[1..3], [Arg::Int(1280), Arg::Int(960)]);
    assert_eq!(e[2].args, [Arg::Int(2)]);
    let (surface, xdg, _tl, events) = toplevel(&mut p, &mut srv, &g, "hi");
    assert_eq!(events[0].args, [Arg::Int(2)], "preferred_buffer_scale");
    // The work area is in logical pixels: 640x480 less the frame.
    assert_eq!(events[1].args, [Arg::Int(640 - 2), Arg::Int(480 - 24 - 2)]);
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    // A 2x buffer: 40x20 pixels are 20x10 logical ones, drawn 1:1.
    let (b, mem) = buffer(&mut p, &mut srv, &g, 40, 20, 0x00ff_0000, 1);
    mem.borrow_mut()[..4].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
    p.send(&mut srv, surface, wl_surface::Request::SetBufferScale { scale: 2 });
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    let w = &srv.scene().windows()[0];
    assert_eq!((w.content().scale, w.content().logical_size()), (2, (20, 10)));
    let (x, y) = w.position();
    let mut fb = MemoryFramebuffer::new(1280, 960);
    srv.render(&mut fb);
    let (px, py) = ((x * 2) as u32, (y * 2) as u32);
    assert_eq!((fb.pixel(px, py), fb.pixel(px + 1, py)), (Some(0x00ff00), Some(0xff0000)));
    // The output's scale changes: wl_output and the surface hear of it.
    p.take();
    srv.set_output(OutputConfig { width: 1280, height: 960, scale: 1, ..OutputConfig::default() });
    p.read(&mut srv);
    let names = p.names();
    assert!(names.contains(&("wl_output", "scale")) && names.contains(&("wl_surface", "preferred_buffer_scale")));
}

#[test]
fn decorations_are_always_server_side() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &zxdg_decoration_manager_v1::INTERFACE, 1);
    let surface = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: surface });
    let xdg = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: xdg, surface });
    let tl = p.new_id(&xdg_toplevel::INTERFACE);
    p.send(&mut srv, xdg, xdg_surface::Request::GetToplevel { id: tl });
    let deco = p.new_id(&zxdg_toplevel_decoration_v1::INTERFACE);
    p.send(&mut srv, manager, zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id: deco, toplevel: tl });
    // Asking for client-side decorations changes nothing.
    let client_side = zxdg_toplevel_decoration_v1::Mode::CLIENT_SIDE.0;
    p.send(&mut srv, deco, zxdg_toplevel_decoration_v1::Request::SetMode { mode: client_side });
    assert!(p.take().is_empty(), "nothing before the initial commit");
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    let e = p.take();
    let server_side = Arg::Uint(zxdg_toplevel_decoration_v1::Mode::SERVER_SIDE.0);
    let d: Vec<_> = e.iter().filter(|e| e.object == deco).collect();
    assert_eq!((d.len(), d[0].name().1, &d[0].args[..]), (1, "configure", &[server_side.clone()][..]));
    // The decoration configure comes before xdg_surface.configure.
    let order: Vec<_> = e.iter().map(|e| e.name().1).collect();
    assert_eq!(order[order.len() - 3..], ["configure", "configure", "configure"]);
    assert_eq!(e[e.len() - 3].object, deco);
    // Later requests get a new configure sequence, still server-side.
    p.send(&mut srv, deco, zxdg_toplevel_decoration_v1::Request::UnsetMode);
    let e = p.take();
    assert_eq!(e.iter().find(|e| e.object == deco).unwrap().args, [server_side]);
    assert_eq!(e.last().unwrap().name(), ("xdg_surface", "configure"));
    // Invalid modes, a second decoration, and an orphaned one are errors.
    p.send(&mut srv, deco, zxdg_toplevel_decoration_v1::Request::SetMode { mode: 7 });
    assert_eq!(p.code(), Some((deco, zxdg_toplevel_decoration_v1::Error::INVALID_MODE.0)));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &zxdg_decoration_manager_v1::INTERFACE, 2);
    let (_s, _x, tl, _) = toplevel(&mut p, &mut srv, &g, "o");
    let deco = p.new_id(&zxdg_toplevel_decoration_v1::INTERFACE);
    p.send(&mut srv, manager, zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id: deco, toplevel: tl });
    // Made after the initial commit: configured at once.
    let e = p.take();
    assert_eq!(e.iter().filter(|e| e.object == deco).count(), 1);
    let second = p.new_id(&zxdg_toplevel_decoration_v1::INTERFACE);
    p.send(&mut srv, manager, zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id: second, toplevel: tl });
    assert_eq!(p.code(), Some((second, zxdg_toplevel_decoration_v1::Error::ALREADY_CONSTRUCTED.0)));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &zxdg_decoration_manager_v1::INTERFACE, 2);
    let (_s, _x, tl, _) = toplevel(&mut p, &mut srv, &g, "o");
    let deco = p.new_id(&zxdg_toplevel_decoration_v1::INTERFACE);
    p.send(&mut srv, manager, zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id: deco, toplevel: tl });
    p.send(&mut srv, tl, xdg_toplevel::Request::Destroy);
    assert_eq!(p.code(), Some((deco, zxdg_toplevel_decoration_v1::Error::ORPHANED.0)));
}

#[test]
fn a_null_buffer_unmaps_and_the_configure_sequence_starts_over() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "u", 10, 10, 0);
    assert_eq!(srv.scene().windows().len(), 1);
    p.take();
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: None, x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(srv.scene().windows().is_empty());
    assert!(p.take().is_empty());
    // A new initial commit gets a new configure.
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert_eq!(p.take().last().unwrap().name(), ("xdg_surface", "configure"));
    // Destroying the toplevel and the xdg_surface, then the surface, is fine.
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::Destroy);
    p.send(&mut srv, w.xdg, xdg_surface::Request::Destroy);
    p.send(&mut srv, w.surface, wl_surface::Request::Destroy);
    assert!(p.error().is_none());
}

#[test]
fn clicks_focus_windows_and_the_close_button_closes() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let a = mapped(&mut p, &mut srv, &g, "a", 100, 100, 0x00ff_0000);
    let b = mapped(&mut p, &mut srv, &g, "b", 100, 100, 0x0000_ff00);
    // b mapped last: active, and a heard it is not any more.
    let windows = srv.scene().windows();
    assert_eq!(
        (windows[0].title(), windows[0].is_active(), windows[1].title(), windows[1].is_active()),
        ("a", false, "b", true)
    );
    let deactivated = p.take().into_iter().rev().find(|e| e.object == a.toplevel && e.name().1 == "configure").unwrap();
    assert_eq!(deactivated.args[2], Arg::Array(Vec::new()));
    // A click on a (where b does not cover it) activates and raises it.
    let (ax, ay) = srv.scene().windows()[0].position();
    let click = |srv: &mut Server, x: i32, y: i32| {
        srv.handle_input(InputEvent::PointerMotionAbsolute { time: 0, x: Fixed::from_int(x), y: Fixed::from_int(y) });
        srv.handle_input(InputEvent::PointerButton { time: 0, button: BTN_LEFT, pressed: true });
    };
    click(&mut srv, ax + 2, ay + 2);
    p.read(&mut srv);
    let windows = srv.scene().windows();
    assert_eq!((windows[1].title(), windows[1].is_active()), ("a", true));
    let e = p.take();
    let activated = e.iter().rev().find(|e| e.object == a.toplevel && e.name().1 == "configure").unwrap();
    assert_eq!(activated.args[2], Arg::Array(xdg_toplevel::State::ACTIVATED.0.to_ne_bytes().to_vec()));
    assert!(e.iter().any(|e| e.object == b.toplevel && e.name().1 == "configure"));
    // The close button of a.
    let frame = srv.scene().frame_rect(srv.scene().windows()[1].id()).unwrap();
    click(&mut srv, frame.right() - 1 - 4 - 8, frame.y + 1 + 4 + 8);
    p.read(&mut srv);
    let e = p.take();
    assert_eq!((e[0].object, e[0].name()), (a.toplevel, ("xdg_toplevel", "close")));
    let _ = b.surface;
}

#[test]
fn maximize_and_fullscreen() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "m", 50, 50, 0);
    let before = srv.scene().windows()[0].position();
    p.take();
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::SetMaximized);
    let e = p.take();
    let c = e.iter().find(|e| e.name() == ("xdg_toplevel", "configure")).unwrap();
    assert_eq!(c.args[..2], [Arg::Int(640 - 2), Arg::Int(480 - 24 - 2)]);
    let states = [xdg_toplevel::State::ACTIVATED.0, xdg_toplevel::State::MAXIMIZED.0];
    assert_eq!(c.args[2], Arg::Array(states.iter().flat_map(|s| s.to_ne_bytes()).collect()));
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (big, _) = buffer(&mut p, &mut srv, &g, 638, 454, 0, 1);
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(big), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert_eq!(srv.scene().windows()[0].position(), (1, 25));
    // Fullscreen: no frame, at the origin.
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::SetFullscreen { output: None });
    let e = p.take();
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let win = &srv.scene().windows()[0];
    assert_eq!((win.position(), win.is_decorated()), ((0, 0), false));
    // Back to normal: where it was, framed.
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::UnsetFullscreen);
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::UnsetMaximized);
    let e = p.take();
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let win = &srv.scene().windows()[0];
    assert_eq!((win.position(), win.is_decorated()), (before, true));
}

#[test]
fn popups_are_placed_by_their_positioner() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let parent = mapped(&mut p, &mut srv, &g, "p", 200, 100, 0x00ff_0000);
    let positioner = p.new_id(&xdg_positioner::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::CreatePositioner { id: positioner });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetSize { width: 40, height: 30 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetAnchorRect { x: 10, y: 10, width: 20, height: 20 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetAnchor { anchor: xdg_positioner::Anchor::BOTTOM_LEFT.0 });
    let bottom_right = xdg_positioner::Gravity::BOTTOM_RIGHT.0;
    p.send(&mut srv, positioner, xdg_positioner::Request::SetGravity { gravity: bottom_right });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetOffset { x: 1, y: 2 });
    let surface = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: surface });
    let xdg = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: xdg, surface });
    let popup = p.new_id(&xdg_popup::INTERFACE);
    p.send(&mut srv, xdg, xdg_surface::Request::GetPopup { id: popup, parent: Some(parent.xdg), positioner });
    p.take();
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    let e = p.take();
    let c = e.iter().find(|e| e.name() == ("xdg_popup", "configure")).unwrap();
    assert_eq!(c.args, [Arg::Int(11), Arg::Int(32), Arg::Int(40), Arg::Int(30)]);
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (b, _) = buffer(&mut p, &mut srv, &g, 40, 30, 0x0000_00ff, 1);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    p.take();
    let windows = srv.scene().windows();
    let (px, py) = windows[0].position();
    assert_eq!(windows.len(), 2);
    assert_eq!((windows[1].position(), windows[1].is_decorated()), ((px + 11, py + 32), false));
    // Repositioning answers with the token and a configure.
    p.send(&mut srv, positioner, xdg_positioner::Request::SetOffset { x: 0, y: 0 });
    p.send(&mut srv, popup, xdg_popup::Request::Reposition { positioner, token: 9 });
    let names = p.names();
    assert_eq!(names, [("xdg_popup", "repositioned"), ("xdg_popup", "configure"), ("xdg_surface", "configure")]);
    // Unmapping the parent dismisses the popup.
    p.send(&mut srv, parent.surface, wl_surface::Request::Attach { buffer: None, x: 0, y: 0 });
    p.send(&mut srv, parent.surface, wl_surface::Request::Commit);
    assert_eq!(p.names(), [("xdg_popup", "popup_done")]);
    assert!(srv.scene().windows().is_empty());
    // A positioner without a size is refused.
    let empty = p.new_id(&xdg_positioner::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::CreatePositioner { id: empty });
    let s2 = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: s2 });
    let x2 = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: x2, surface: s2 });
    let p2 = p.new_id(&xdg_popup::INTERFACE);
    p.send(&mut srv, x2, xdg_surface::Request::GetPopup { id: p2, parent: None, positioner: empty });
    assert_eq!(p.code(), Some((g.wm_base, xdg_wm_base::Error::INVALID_POSITIONER.0)));
}

#[test]
fn the_seat_has_capabilities_only() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let seat = p.bind(&mut srv, registry, &wl_seat::INTERFACE, 7);
    let e = p.take();
    assert_eq!(e.iter().map(Received::name).collect::<Vec<_>>(), [("wl_seat", "name"), ("wl_seat", "capabilities")]);
    assert_eq!((&e[0].args[0], &e[1].args[0]), (&Arg::Str(Some("seat0".into())), &Arg::Uint(0)));
    srv.set_seat_capabilities(Capabilities::POINTER.union(Capabilities::KEYBOARD));
    p.read(&mut srv);
    assert_eq!(p.take()[0].args, [Arg::Uint(3)]);
    let pointer = p.new_id(&wl_pointer::INTERFACE);
    p.send(&mut srv, seat, wl_seat::Request::GetPointer { id: pointer });
    assert!(p.error().is_none());
    let touch = p.new_id(&wl_touch::INTERFACE);
    p.send(&mut srv, seat, wl_seat::Request::GetTouch { id: touch });
    assert_eq!(p.code(), Some((seat, wl_seat::Error::MISSING_CAPABILITY.0)));
}

#[test]
fn presentation_feedback() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let pres = p.bind(&mut srv, registry, &wp_presentation::INTERFACE, 1);
    assert_eq!(p.take()[0].args, [Arg::Uint(1)], "CLOCK_MONOTONIC");
    let w = mapped(&mut p, &mut srv, &g, "f", 4, 4, 0);
    let f1 = p.new_id(&wp_presentation_feedback::INTERFACE);
    p.send(&mut srv, pres, wp_presentation::Request::Feedback { surface: w.surface, callback: f1 });
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(w.buffer), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    // Replaced before a frame showed it: discarded.
    let f2 = p.new_id(&wp_presentation_feedback::INTERFACE);
    p.send(&mut srv, pres, wp_presentation::Request::Feedback { surface: w.surface, callback: f2 });
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(w.buffer), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let e = p.take();
    assert!(e.iter().any(|e| e.object == f1 && e.name().1 == "discarded"));
    srv.frame_presented(super::FrameTime { monotonic_ns: (1 << 32) * 1_000_000_000 + 7 });
    p.read(&mut srv);
    let e = p.take();
    let presented = e.iter().find(|e| e.object == f2 && e.name().1 == "presented").unwrap();
    assert_eq!(presented.args[..3], [Arg::Uint(1), Arg::Uint(0), Arg::Uint(7)]);
    assert_eq!(presented.args[3], Arg::Uint(16_666_666), "refresh at 60 Hz");
}

#[test]
fn removing_a_client_frees_its_windows_and_memory() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    mapped(&mut p, &mut srv, &g, "gone", 4, 4, 0);
    let (fd, _) = p.memfd(64);
    srv.receive(p.id, &[], vec![fd]);
    assert_eq!(srv.scene().windows().len(), 1);
    let pool_fd = 100 * p.id.0 as i32 + 1;
    srv.remove_client(p.id);
    assert!(srv.scene().windows().is_empty());
    let closed = shared.borrow().closed.clone();
    assert!(closed.contains(&pool_fd), "the pool's file is closed: {closed:?}");
    assert!(closed.contains(&fd.0), "an unused fd is closed: {closed:?}");
    assert_eq!(srv.clients().count(), 0);
}
mod input_routing;
mod protocols;
mod window_management;
mod xwayland;
