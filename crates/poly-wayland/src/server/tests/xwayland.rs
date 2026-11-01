//! Xwayland's surfaces (ADR 0118): `xwayland_shell_v1`, associating X
//! windows with surfaces from both sides, and X windows in the scene and
//! the seat.

use super::*;
use crate::compose::{BORDER, TITLE_HEIGHT};
use crate::protocol::{xwayland_shell_v1, xwayland_surface_v1};

const XEYES: u32 = 0x0040_0001;
const MENU: u32 = 0x0040_0020;
const GREEN: u32 = 0xff20_c040;

/// Xwayland: a client the server knows as Xwayland, with the shell, a
/// pointer and a keyboard.
struct Xw {
    p: Peer,
    g: Globals,
    shell: ObjectId,
    keyboard: ObjectId,
}

fn xwayland(srv: &mut Server, shared: &Rc<RefCell<Shared>>) -> Xw {
    srv.set_seat_capabilities(Capabilities::POINTER.union(Capabilities::KEYBOARD));
    let id = ClientId(srv.next_client);
    srv.set_xwayland_client(Some(id));
    let (mut p, registry) = connect(srv, shared);
    assert_eq!(p.id, id);
    let g = globals(&mut p, srv, registry);
    let shell = p.bind(srv, registry, &xwayland_shell_v1::INTERFACE, 1);
    p.bind(srv, registry, &wl_output::INTERFACE, 4);
    let seat = p.bind(srv, registry, &wl_seat::INTERFACE, 7);
    let pointer = p.new_id(&wl_pointer::INTERFACE);
    p.send(srv, seat, wl_seat::Request::GetPointer { id: pointer });
    let keyboard = p.new_id(&wl_keyboard::INTERFACE);
    p.send(srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    p.take();
    Xw { p, g, shell, keyboard }
}

impl Xw {
    /// A surface with the xwayland role and `serial` set (not committed).
    fn surface(&mut self, srv: &mut Server, serial: u64) -> ObjectId {
        let surface = self.p.new_id(&wl_surface::INTERFACE);
        self.p.send(srv, self.g.compositor, wl_compositor::Request::CreateSurface { id: surface });
        let xs = self.p.new_id(&xwayland_surface_v1::INTERFACE);
        self.p.send(srv, self.shell, xwayland_shell_v1::Request::GetXwaylandSurface { id: xs, surface });
        let (serial_lo, serial_hi) = (serial as u32, (serial >> 32) as u32);
        self.p.send(srv, xs, xwayland_surface_v1::Request::SetSerial { serial_lo, serial_hi });
        surface
    }

    /// Attach a `w` × `h` buffer of `pixel` and commit.
    fn draw(&mut self, srv: &mut Server, surface: ObjectId, w: i32, h: i32, pixel: u32) {
        let (buffer, _) = buffer(&mut self.p, srv, &self.g, w, h, pixel, wl_shm::Format::ARGB8888.0);
        self.p.send(srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
        self.p.send(srv, surface, wl_surface::Request::DamageBuffer { x: 0, y: 0, width: w, height: h });
        self.p.send(srv, surface, wl_surface::Request::Commit);
    }
}

fn managed(x: i32, y: i32, title: &str) -> XWindowState {
    XWindowState {
        x,
        y,
        width: 150,
        height: 100,
        mapped: true,
        managed: true,
        override_redirect: false,
        decorated: true,
        title: title.into(),
        transient_for: None,
    }
}

fn shown(srv: &Server, window: u32) -> Option<&crate::compose::Window> {
    srv.x_scene_window(window).and_then(|w| srv.scene().window(w))
}

#[test]
fn only_xwayland_sees_and_binds_the_shell() {
    let (mut srv, shared) = server();
    let (mut other, registry) = connect(&mut srv, &shared);
    assert!(!other.globals.contains_key("xwayland_shell_v1"));
    let xw = xwayland(&mut srv, &shared);
    assert_eq!(xw.p.globals["xwayland_shell_v1"].1, 1);
    // The other client guesses the global's name: refused.
    let name = xw.p.globals["xwayland_shell_v1"].0;
    let id = other.new_id(&xwayland_shell_v1::INTERFACE);
    let new = crate::wire::NewObject { interface: "xwayland_shell_v1".into(), version: 1, id };
    other.send(&mut srv, registry, wl_registry::Request::Bind { name, id: new });
    let (_, code, msg) = other.error().expect("an error");
    assert_eq!(code, wl_display::Error::INVALID_OBJECT.0);
    assert!(msg.contains("only for Xwayland"), "{msg}");
    assert!(srv.failure(xw.p.id).is_none());
}

#[test]
fn a_serial_committed_first_then_named_by_the_window_manager() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let s = xw.surface(&mut srv, 0x1_0000_0007);
    xw.draw(&mut srv, s, 150, 100, GREEN);
    assert!(srv.scene().windows().is_empty(), "not associated yet");
    srv.x_update(XEYES, &managed(100, 80, "xeyes"));
    assert!(srv.scene().windows().is_empty(), "the window manager has not named the surface");
    srv.x_associate(XEYES, SurfaceRef::Serial(0x1_0000_0007));
    let w = shown(&srv, XEYES).expect("shown");
    assert_eq!((w.position(), w.content().logical_size(), w.title()), ((100, 80), (150, 100), "xeyes"));
    assert!(w.is_decorated() && w.is_active());
    // Its content is the buffer committed before the association.
    let mut fb = MemoryFramebuffer::new(640, 480);
    srv.render(&mut fb);
    assert_eq!(fb.pixel(110, 90), Some(GREEN & 0x00ff_ffff));
    // The frame is Poly's, above the content.
    assert_eq!(srv.scene().hit_test(110, 80 - TITLE_HEIGHT / 2), Hit::TitleBar(srv.x_scene_window(XEYES).unwrap()));
    // A managed window that appears takes the focus: X is told, and the
    // keyboard goes to its surface.
    assert_eq!(srv.take_x_requests(), [XRequest::Activate(XEYES)]);
    assert_eq!(srv.keyboard_focus(), Some((xw.p.id, s)));
    xw.p.read(&mut srv);
    let e = xw.p.take();
    assert!(e.iter().any(|e| e.name() == ("wl_keyboard", "enter") && e.object == xw.keyboard));
    assert!(e.iter().any(|e| e.name() == ("wl_surface", "enter")));
}

#[test]
fn the_window_manager_first_then_the_serial_commit() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    srv.x_update(XEYES, &managed(40, 60, "xclock"));
    srv.x_associate(XEYES, SurfaceRef::Serial(9));
    let s = xw.surface(&mut srv, 9);
    assert!(srv.scene().windows().is_empty(), "the serial is not committed yet");
    // Committed without content: associated, not shown.
    xw.p.send(&mut srv, s, wl_surface::Request::Commit);
    assert!(srv.scene().windows().is_empty());
    xw.draw(&mut srv, s, 64, 32, GREEN);
    let w = shown(&srv, XEYES).expect("shown at the first buffer");
    assert_eq!((w.position(), w.content().logical_size()), ((40, 60), (64, 32)));
}

#[test]
fn the_legacy_surface_id_works_both_ways() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    // A plain surface, named by its object ID (WL_SURFACE_ID).
    let s = xw.p.new_id(&wl_surface::INTERFACE);
    xw.p.send(&mut srv, xw.g.compositor, wl_compositor::Request::CreateSurface { id: s });
    xw.draw(&mut srv, s, 20, 20, GREEN);
    srv.x_update(XEYES, &managed(10, 40, "a"));
    srv.x_associate(XEYES, SurfaceRef::Id(s.0));
    assert!(shown(&srv, XEYES).is_some());
    // Named before the surface exists: shown at its first commit.
    srv.x_update(MENU, &managed(200, 40, "b"));
    srv.x_associate(MENU, SurfaceRef::Id(s.0 + 5));
    assert!(shown(&srv, MENU).is_none());
    let mut t = ObjectId(0);
    while t.0 != s.0 + 5 {
        t = xw.p.new_id(&wl_surface::INTERFACE);
        xw.p.send(&mut srv, xw.g.compositor, wl_compositor::Request::CreateSurface { id: t });
    }
    xw.draw(&mut srv, t, 20, 20, GREEN);
    assert!(shown(&srv, MENU).is_some());
    assert!(srv.failure(xw.p.id).is_none());
}

#[test]
fn role_and_serial_errors() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    // A surface with an xdg role cannot be an X window's.
    let (surface, _, _, _) = toplevel(&mut xw.p, &mut srv, &xw.g, "t");
    let xs = xw.p.new_id(&xwayland_surface_v1::INTERFACE);
    xw.p.send(&mut srv, xw.shell, xwayland_shell_v1::Request::GetXwaylandSurface { id: xs, surface });
    assert_eq!(xw.p.code(), Some((surface, xwayland_shell_v1::Error::ROLE.0)));

    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let surface = xw.p.new_id(&wl_surface::INTERFACE);
    xw.p.send(&mut srv, xw.g.compositor, wl_compositor::Request::CreateSurface { id: surface });
    let xs = xw.p.new_id(&xwayland_surface_v1::INTERFACE);
    xw.p.send(&mut srv, xw.shell, xwayland_shell_v1::Request::GetXwaylandSurface { id: xs, surface });
    xw.p.send(&mut srv, xs, xwayland_surface_v1::Request::SetSerial { serial_lo: 0, serial_hi: 0 });
    assert_eq!(xw.p.code(), Some((xs, xwayland_surface_v1::Error::INVALID_SERIAL.0)));

    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let s = xw.surface(&mut srv, 5);
    xw.p.send(&mut srv, s, wl_surface::Request::Commit);
    let xs = ObjectId(s.0 + 1);
    xw.p.send(&mut srv, xs, xwayland_surface_v1::Request::SetSerial { serial_lo: 6, serial_hi: 0 });
    xw.p.send(&mut srv, s, wl_surface::Request::Commit);
    assert_eq!(xw.p.code(), Some((xs, xwayland_surface_v1::Error::ALREADY_ASSOCIATED.0)));
}

#[test]
fn override_redirect_windows_are_unframed_on_top_and_take_no_focus() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let s = xw.surface(&mut srv, 1);
    xw.draw(&mut srv, s, 150, 100, GREEN);
    srv.x_update(XEYES, &managed(100, 80, "app"));
    srv.x_associate(XEYES, SurfaceRef::Serial(1));
    srv.take_x_requests();
    let m = xw.surface(&mut srv, 2);
    xw.draw(&mut srv, m, 60, 90, 0xff00_00ff);
    let menu = XWindowState {
        x: 120,
        y: 100,
        width: 60,
        height: 90,
        mapped: true,
        override_redirect: true,
        ..XWindowState::default()
    };
    srv.x_update(MENU, &menu);
    srv.x_associate(MENU, SurfaceRef::Serial(2));
    let w = shown(&srv, MENU).unwrap();
    assert!(!w.is_decorated() && !w.is_active());
    assert_eq!(srv.scene().windows().last().map(|w| w.id()), srv.x_scene_window(MENU), "on top");
    assert!(srv.take_x_requests().is_empty(), "no focus for a menu");
    assert_eq!(srv.keyboard_focus(), Some((xw.p.id, s)), "the app keeps the keyboard");
    // A click on the menu goes to Xwayland's pointer, the focus stays.
    motion(&mut srv, 130, 110);
    button(&mut srv, true);
    assert!(srv.take_x_requests().is_empty());
    xw.p.read(&mut srv);
    let e = xw.p.take();
    assert!(e.iter().any(|e| e.name() == ("wl_pointer", "enter") && e.args[1] == Arg::Object(Some(m))));
    assert!(e.iter().any(|e| e.name() == ("wl_pointer", "button")));
}

fn motion(srv: &mut Server, x: i32, y: i32) {
    srv.handle_input(InputEvent::PointerMotionAbsolute { time: 1, x: Fixed::from_int(x), y: Fixed::from_int(y) });
}

fn button(srv: &mut Server, pressed: bool) {
    srv.handle_input(InputEvent::PointerButton { time: 2, button: BTN_LEFT, pressed });
}

#[test]
fn focus_moves_between_x_and_wayland_windows() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let s = xw.surface(&mut srv, 1);
    xw.draw(&mut srv, s, 150, 100, GREEN);
    srv.x_update(XEYES, &managed(300, 200, "xeyes"));
    srv.x_associate(XEYES, SurfaceRef::Serial(1));
    assert_eq!(srv.take_x_requests(), [XRequest::Activate(XEYES)]);
    // A Wayland window maps: it takes the focus, X loses it.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let win = mapped(&mut p, &mut srv, &g, "wayland", 100, 60, 0xff80_8080);
    assert_eq!(srv.take_x_requests(), [XRequest::Deactivate]);
    assert!(!shown(&srv, XEYES).unwrap().is_active());
    assert_eq!(srv.keyboard_focus(), Some((p.id, win.surface)));
    // A click on the X window gives it back.
    motion(&mut srv, 320, 220);
    button(&mut srv, true);
    button(&mut srv, false);
    assert_eq!(srv.take_x_requests(), [XRequest::Activate(XEYES)]);
    assert!(shown(&srv, XEYES).unwrap().is_active());
    assert_eq!(srv.keyboard_focus(), Some((xw.p.id, s)));
    assert_eq!(srv.scene().windows().last().map(|w| w.id()), srv.x_scene_window(XEYES), "raised");
    // A client's own request (_NET_ACTIVE_WINDOW) while focused: nothing new.
    srv.x_activate(XEYES);
    assert!(srv.take_x_requests().is_empty());
}

#[test]
fn the_close_button_and_dragging_reach_the_window_manager() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let s = xw.surface(&mut srv, 1);
    xw.draw(&mut srv, s, 150, 100, GREEN);
    srv.x_update(XEYES, &managed(100, 80, "xeyes"));
    srv.x_associate(XEYES, SurfaceRef::Serial(1));
    srv.take_x_requests();
    // Drag the title bar by (50, 30).
    let ty = 80 - BORDER - TITLE_HEIGHT / 2;
    motion(&mut srv, 120, ty);
    button(&mut srv, true);
    motion(&mut srv, 145, ty + 15);
    // A late ConfigureNotify with the old place does not pull it back.
    srv.x_update(XEYES, &managed(100, 80, "xeyes"));
    motion(&mut srv, 170, ty + 30);
    button(&mut srv, false);
    assert_eq!(shown(&srv, XEYES).unwrap().position(), (150, 110));
    let moves: Vec<XRequest> = srv.take_x_requests();
    assert_eq!(moves.last(), Some(&XRequest::Move { window: XEYES, x: 150, y: 110 }));
    // X confirms: nothing changes.
    srv.x_update(XEYES, &managed(150, 110, "xeyes"));
    assert_eq!(shown(&srv, XEYES).unwrap().position(), (150, 110));
    // The close button (top right of the frame).
    let frame = srv.scene().frame_rect(srv.x_scene_window(XEYES).unwrap()).unwrap();
    let close = match (frame.x..frame.right())
        .rev()
        .find(|&x| matches!(srv.scene().hit_test(x, frame.y + TITLE_HEIGHT / 2 + BORDER), Hit::Close(_)))
    {
        Some(x) => x,
        None => panic!("no close button in {frame:?}"),
    };
    motion(&mut srv, close, frame.y + TITLE_HEIGHT / 2 + BORDER);
    button(&mut srv, true);
    assert_eq!(srv.take_x_requests(), [XRequest::Close(XEYES)]);
    // A move asked by the client (_NET_WM_MOVERESIZE) while the button is held.
    button(&mut srv, false);
    motion(&mut srv, 200, 150);
    button(&mut srv, true);
    srv.take_x_requests();
    srv.x_start_move(XEYES);
    motion(&mut srv, 210, 155);
    button(&mut srv, false);
    assert_eq!(shown(&srv, XEYES).unwrap().position(), (160, 115));
    assert_eq!(srv.take_x_requests().last(), Some(&XRequest::Move { window: XEYES, x: 160, y: 115 }));
}

#[test]
fn transients_go_above_their_parent() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let a = xw.surface(&mut srv, 1);
    xw.draw(&mut srv, a, 150, 100, GREEN);
    srv.x_update(XEYES, &managed(100, 80, "parent"));
    srv.x_associate(XEYES, SurfaceRef::Serial(1));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    mapped(&mut p, &mut srv, &g, "wayland", 100, 60, 0xff80_8080);
    let d = xw.surface(&mut srv, 2);
    xw.draw(&mut srv, d, 50, 40, GREEN);
    let dialog = XWindowState { transient_for: Some(XEYES), ..managed(120, 100, "dialog") };
    srv.x_update(MENU, &dialog);
    srv.x_associate(MENU, SurfaceRef::Serial(2));
    let ids: Vec<WindowId> = srv.scene().windows().iter().map(|w| w.id()).collect();
    let (pi, di) = (
        ids.iter().position(|w| Some(*w) == srv.x_scene_window(XEYES)).unwrap(),
        ids.iter().position(|w| Some(*w) == srv.x_scene_window(MENU)).unwrap(),
    );
    assert!(di > pi, "the dialog is above its parent");
}

#[test]
fn unmapping_destroying_and_losing_xwayland() {
    let (mut srv, shared) = server();
    let mut xw = xwayland(&mut srv, &shared);
    let s = xw.surface(&mut srv, 1);
    xw.draw(&mut srv, s, 150, 100, GREEN);
    srv.x_update(XEYES, &managed(100, 80, "xeyes"));
    srv.x_associate(XEYES, SurfaceRef::Serial(1));
    assert!(shown(&srv, XEYES).is_some());
    // Unmapped: off the output, the keyboard with nobody.
    srv.x_update(XEYES, &XWindowState { mapped: false, managed: false, ..managed(100, 80, "xeyes") });
    assert!(shown(&srv, XEYES).is_none() && srv.scene().windows().is_empty());
    assert_eq!(srv.keyboard_focus(), None);
    // Mapped again with the same surface.
    srv.x_update(XEYES, &managed(100, 80, "xeyes"));
    assert!(shown(&srv, XEYES).is_some());
    // The surface goes: so does the window.
    xw.p.send(&mut srv, s, wl_surface::Request::Destroy);
    assert!(srv.scene().windows().is_empty());
    // Another window, then Xwayland disconnects: everything is forgotten.
    let t = xw.surface(&mut srv, 2);
    xw.draw(&mut srv, t, 10, 10, GREEN);
    srv.x_update(MENU, &managed(0, 30, "m"));
    srv.x_associate(MENU, SurfaceRef::Serial(2));
    assert_eq!(srv.scene().windows().len(), 1);
    srv.x_destroy(MENU);
    assert!(srv.scene().windows().is_empty());
    srv.x_update(MENU, &managed(0, 30, "m"));
    srv.x_associate(MENU, SurfaceRef::Serial(2));
    srv.remove_client(xw.p.id);
    assert_eq!(srv.xwayland_client(), None);
    assert!(srv.scene().windows().is_empty() && srv.x_scene_window(MENU).is_none());
}
