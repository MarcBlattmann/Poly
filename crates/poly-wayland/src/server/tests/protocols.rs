//! The protocols real toolkits need besides the core and xdg-shell (ADR
//! 0116): subsurfaces, viewports and buffer transforms, the clipboard and
//! drag and drop, dma-bufs, popup constraints and grabs, interactive
//! resize, cursor surfaces and shapes, fractional scale, activation, the
//! frame clock of page flips, and backdrops.

use super::*;
use crate::compose::{CursorLook, Rect as R};
use crate::server::xdg::Positioner;

fn names(e: &[Received]) -> Vec<(&'static str, &'static str)> {
    e.iter().map(Received::name).collect()
}

fn motion(srv: &mut Server, x: i32, y: i32) {
    srv.handle_input(InputEvent::PointerMotionAbsolute { time: 7, x: Fixed::from_int(x), y: Fixed::from_int(y) });
}

fn button(srv: &mut Server, pressed: bool) {
    srv.handle_input(InputEvent::PointerButton { time: 8, button: BTN_LEFT, pressed });
}

/// A client with a pointer on a seat with a pointer and a keyboard.
fn pointer_client(srv: &mut Server, shared: &Rc<RefCell<Shared>>) -> (Peer, ObjectId, Globals, ObjectId) {
    srv.set_seat_capabilities(Capabilities::POINTER.union(Capabilities::KEYBOARD));
    let (mut p, registry) = connect(srv, shared);
    let g = globals(&mut p, srv, registry);
    let seat = p.bind(srv, registry, &wl_seat::INTERFACE, 7);
    let pointer = p.new_id(&wl_pointer::INTERFACE);
    p.send(srv, seat, wl_seat::Request::GetPointer { id: pointer });
    p.take();
    (p, registry, g, pointer)
}

fn surface(p: &mut Peer, srv: &mut Server, g: &Globals) -> ObjectId {
    let s = p.new_id(&wl_surface::INTERFACE);
    p.send(srv, g.compositor, wl_compositor::Request::CreateSurface { id: s });
    s
}

fn attach(p: &mut Peer, srv: &mut Server, s: ObjectId, b: ObjectId) {
    p.send(srv, s, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(srv, s, wl_surface::Request::DamageBuffer { x: 0, y: 0, width: i32::MAX, height: i32::MAX });
    p.send(srv, s, wl_surface::Request::Commit);
}

fn render(srv: &mut Server) -> MemoryFramebuffer {
    let (w, h, _) = srv.scene().output();
    let mut fb = MemoryFramebuffer::new(w, h);
    srv.scene_mut().damage_all();
    srv.render(&mut fb);
    fb
}

// ---------------------------------------------------------------------
// Subsurfaces.

#[test]
fn subsurfaces_are_shown_with_their_parent_and_cache_their_commits() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sub_comp = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let parent = mapped(&mut p, &mut srv, &g, "parent", 40, 30, 0x00ff_0000);
    let (px, py) = srv.scene().windows()[0].position();
    let child = surface(&mut p, &mut srv, &g);
    let sub = p.new_id(&wl_subsurface::INTERFACE);
    p.send(
        &mut srv,
        sub_comp,
        wl_subcompositor::Request::GetSubsurface { id: sub, surface: child, parent: parent.surface },
    );
    p.send(&mut srv, sub, wl_subsurface::Request::SetPosition { x: 10, y: 5 });
    let (b, _) = buffer(&mut p, &mut srv, &g, 8, 6, 0x0000_ff00, 1);
    // Synchronized: cached until the parent commits.
    attach(&mut p, &mut srv, child, b);
    assert_eq!(srv.scene().windows().len(), 1);
    p.send(&mut srv, parent.surface, wl_surface::Request::Commit);
    let windows = srv.scene().windows();
    assert_eq!(windows.len(), 2, "shown with the parent's commit");
    assert_eq!(windows[1].position(), (px + 10, py + 5));
    assert_eq!(windows[1].parent(), Some(windows[0].id()));
    let fb = render(&mut srv);
    assert_eq!(fb.pixel((px + 11) as u32, (py + 6) as u32), Some(0x00ff00), "above its parent");
    // Below the parent, with the parent's next commit.
    p.send(&mut srv, sub, wl_subsurface::Request::PlaceBelow { sibling: parent.surface });
    let fb = render(&mut srv);
    assert_eq!(fb.pixel((px + 11) as u32, (py + 6) as u32), Some(0x00ff00), "not yet");
    p.send(&mut srv, parent.surface, wl_surface::Request::Commit);
    let fb = render(&mut srv);
    assert_eq!(fb.pixel((px + 11) as u32, (py + 6) as u32), Some(0xff0000), "under it now");
    // Moving the parent moves the child.
    let pw = srv.scene().windows().iter().find(|w| w.title() == "parent").unwrap().id();
    srv.scene_mut().set_position(pw, px + 20, py);
    let cw = srv.scene().windows().iter().find(|w| w.parent() == Some(pw)).unwrap();
    assert_eq!(cw.position(), (px + 30, py + 5));
    // Desynchronized: a commit applies at once.
    p.send(&mut srv, sub, wl_subsurface::Request::SetDesync);
    p.send(&mut srv, sub, wl_subsurface::Request::PlaceAbove { sibling: parent.surface });
    p.send(&mut srv, parent.surface, wl_surface::Request::Commit);
    let (b2, _) = buffer(&mut p, &mut srv, &g, 4, 4, 0x0000_00ff, 1);
    attach(&mut p, &mut srv, child, b2);
    let cw = srv.scene().windows().iter().find(|w| w.parent() == Some(pw)).unwrap();
    assert_eq!(cw.content().logical_size(), (4, 4));
    // A null buffer hides it; destroying the role object too.
    p.send(&mut srv, child, wl_surface::Request::Attach { buffer: None, x: 0, y: 0 });
    p.send(&mut srv, child, wl_surface::Request::Commit);
    assert_eq!(srv.scene().windows().len(), 1);
    attach(&mut p, &mut srv, child, b2);
    assert_eq!(srv.scene().windows().len(), 2);
    p.send(&mut srv, sub, wl_subsurface::Request::Destroy);
    assert_eq!(srv.scene().windows().len(), 1);
    assert!(p.error().is_none(), "{:?}", p.error());
}

#[test]
fn a_subsurface_waits_for_its_parent_to_be_mapped_and_a_sync_parent_holds_its_children() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sc = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let (top, xdg, _tl, events) = toplevel(&mut p, &mut srv, &g, "t");
    // A child and a grandchild, both with content before the parent shows.
    let child = surface(&mut p, &mut srv, &g);
    let s1 = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s1, surface: child, parent: top });
    let grand = surface(&mut p, &mut srv, &g);
    let s2 = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s2, surface: grand, parent: child });
    p.send(&mut srv, s2, wl_subsurface::Request::SetDesync);
    let (b, _) = buffer(&mut p, &mut srv, &g, 4, 4, 0x00ff_ffff, 1);
    // The grandchild is desynchronized but in a synchronized child: cached.
    attach(&mut p, &mut srv, grand, b);
    attach(&mut p, &mut srv, child, b);
    assert!(srv.scene().windows().is_empty());
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    let (tb, _) = buffer(&mut p, &mut srv, &g, 20, 20, 0x0000_0000, 1);
    attach(&mut p, &mut srv, top, tb);
    assert_eq!(srv.scene().windows().len(), 3, "the whole tree with the parent's first buffer");
    let ids: Vec<_> = srv.scene().windows().iter().map(|w| w.id()).collect();
    assert_eq!(srv.scene().family(ids[0]), ids);
    // Errors: a surface as its own parent, a second role, a descendant as
    // parent, a stranger as sibling.
    p.send(&mut srv, s1, wl_subsurface::Request::PlaceAbove { sibling: top });
    assert!(p.error().is_none());
    let s3 = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s3, surface: top, parent: grand });
    assert_eq!(p.code(), Some((sc, wl_subcompositor::Error::BAD_SURFACE.0)), "a toplevel has a role");
}

#[test]
fn subsurface_errors() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sc = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let a = surface(&mut p, &mut srv, &g);
    let s = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s, surface: a, parent: a });
    assert_eq!(p.code(), Some((sc, wl_subcompositor::Error::BAD_PARENT.0)));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sc = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let (a, b, c) = (surface(&mut p, &mut srv, &g), surface(&mut p, &mut srv, &g), surface(&mut p, &mut srv, &g));
    let s = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s, surface: b, parent: a });
    let s2 = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s2, surface: a, parent: b });
    assert_eq!(p.code(), Some((sc, wl_subcompositor::Error::BAD_PARENT.0)), "a descendant as parent");
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sc = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let (a, b) = (surface(&mut p, &mut srv, &g), surface(&mut p, &mut srv, &g));
    let stranger = surface(&mut p, &mut srv, &g);
    let s = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: s, surface: b, parent: a });
    p.send(&mut srv, s, wl_subsurface::Request::PlaceAbove { sibling: stranger });
    assert_eq!(p.code(), Some((s, wl_subsurface::Error::BAD_SURFACE.0)));
    let _ = c;
}

/// VLC's Qt interface on Wayland (ADR 0120, 0123): a video surface and its
/// subsurface made, destroyed and made again right away, the client
/// reusing the freed IDs crosswise (the new `wl_subsurface` gets the old
/// surface's ID), then a viewport for the new surface. The server's table
/// follows `delete_id` exactly: the viewport is made, the new subsurface
/// shows. Only a client that names the *old* surface's ID (a stale
/// pointer, as VLC's video output keeps one) is told that the ID is a
/// `wl_subsurface` now: the protocol error VLC was cut off with.
#[test]
fn ids_reused_crosswise_after_a_subsurface_is_remade_name_the_new_objects() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sc = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let viewporter = p.bind(&mut srv, registry, &wp_viewporter::INTERFACE, 1);
    let window = mapped(&mut p, &mut srv, &g, "VLC", 60, 40, 0);
    let video = surface(&mut p, &mut srv, &g);
    let sub = p.new_id(&wl_subsurface::INTERFACE);
    p.send(&mut srv, sc, wl_subcompositor::Request::GetSubsurface { id: sub, surface: video, parent: window.surface });
    p.send(&mut srv, sub, wl_subsurface::Request::SetDesync);
    p.send(&mut srv, sub, wl_subsurface::Request::SetPosition { x: 0, y: 18 });
    p.send(&mut srv, video, wl_surface::Request::Commit);
    p.send(&mut srv, sub, wl_subsurface::Request::Destroy);
    p.send(&mut srv, video, wl_surface::Request::Destroy);
    let deleted: Vec<Arg> =
        p.take().into_iter().filter(|e| e.name() == ("wl_display", "delete_id")).map(|e| e.args[0].clone()).collect();
    assert_eq!(deleted, [Arg::Uint(sub.0), Arg::Uint(video.0)]);
    // Made again, the IDs crosswise.
    let video2 = p.reuse_id(sub, &wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: video2 });
    let sub2 = p.reuse_id(video, &wl_subsurface::INTERFACE);
    p.send(
        &mut srv,
        sc,
        wl_subcompositor::Request::GetSubsurface { id: sub2, surface: video2, parent: window.surface },
    );
    p.send(&mut srv, sub2, wl_subsurface::Request::SetDesync);
    p.send(&mut srv, window.surface, wl_surface::Request::Commit);
    let vp = p.new_id(&wp_viewport::INTERFACE);
    p.send(&mut srv, viewporter, wp_viewporter::Request::GetViewport { id: vp, surface: video2 });
    p.send(&mut srv, vp, wp_viewport::Request::SetDestination { width: 30, height: 20 });
    let (b, _) = buffer(&mut p, &mut srv, &g, 8, 8, 0x00ff_0000, 1);
    attach(&mut p, &mut srv, video2, b);
    assert!(p.error().is_none(), "{:?}", p.error());
    let shown = srv.surface_window(p.id, video2).expect("the new subsurface is shown");
    assert_eq!(srv.scene().window(shown).unwrap().content().logical_size(), (30, 20));
    // The stale ID names the new wl_subsurface.
    let vp2 = p.new_id(&wp_viewport::INTERFACE);
    p.send(&mut srv, viewporter, wp_viewporter::Request::GetViewport { id: vp2, surface: video });
    let (_, code, message) = p.error().expect("an error");
    assert_eq!(code, wl_display::Error::INVALID_OBJECT.0);
    assert!(message.contains("is a wl_subsurface, not a wl_surface"), "{message}");
}

/// A surface destroyed before its `wl_subsurface` leaves the subsurface
/// inert: once the client made a new surface with the freed ID, requests
/// on the old subsurface (and its destruction) do not touch the new one.
#[test]
fn an_inert_subsurface_does_not_reach_a_new_surface_with_its_old_surfaces_id() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let sc = p.bind(&mut srv, registry, &wl_subcompositor::INTERFACE, 1);
    let window = mapped(&mut p, &mut srv, &g, "w", 60, 40, 0);
    let old = surface(&mut p, &mut srv, &g);
    let old_sub = p.new_id(&wl_subsurface::INTERFACE);
    p.send(
        &mut srv,
        sc,
        wl_subcompositor::Request::GetSubsurface { id: old_sub, surface: old, parent: window.surface },
    );
    p.send(&mut srv, old_sub, wl_subsurface::Request::SetDesync);
    // The surface goes first; its subsurface stays, inert.
    p.send(&mut srv, old, wl_surface::Request::Destroy);
    p.take();
    let new = p.reuse_id(old, &wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: new });
    let new_sub = p.new_id(&wl_subsurface::INTERFACE);
    p.send(
        &mut srv,
        sc,
        wl_subcompositor::Request::GetSubsurface { id: new_sub, surface: new, parent: window.surface },
    );
    p.send(&mut srv, new_sub, wl_subsurface::Request::SetDesync);
    p.send(&mut srv, new_sub, wl_subsurface::Request::SetPosition { x: 5, y: 6 });
    p.send(&mut srv, window.surface, wl_surface::Request::Commit);
    let (b, _) = buffer(&mut p, &mut srv, &g, 8, 8, 0x0000_ff00, 1);
    attach(&mut p, &mut srv, new, b);
    let win = srv.surface_window(p.id, new).expect("shown");
    let (wx, wy) = srv.scene().windows()[0].position();
    assert_eq!(srv.scene().window(win).unwrap().position(), (wx + 5, wy + 6));
    // The old subsurface's requests go nowhere.
    p.send(&mut srv, old_sub, wl_subsurface::Request::SetPosition { x: 30, y: 30 });
    p.send(&mut srv, old_sub, wl_subsurface::Request::SetSync);
    p.send(&mut srv, old_sub, wl_subsurface::Request::Destroy);
    p.send(&mut srv, window.surface, wl_surface::Request::Commit);
    assert!(p.error().is_none(), "{:?}", p.error());
    let win = srv.surface_window(p.id, new).expect("still shown");
    assert_eq!(srv.scene().window(win).unwrap().position(), (wx + 5, wy + 6), "not moved");
    // Still desynchronized: a commit shows at once.
    let (b2, _) = buffer(&mut p, &mut srv, &g, 4, 4, 0x0000_00ff, 1);
    attach(&mut p, &mut srv, new, b2);
    assert_eq!(srv.scene().window(win).unwrap().content().logical_size(), (4, 4));
}

// ---------------------------------------------------------------------
// Viewports and transforms.

#[test]
fn viewports_and_buffer_transforms_set_the_surface_size() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let vpr = p.bind(&mut srv, registry, &wp_viewporter::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "v", 4, 2, 0x0012_3456);
    let vp = p.new_id(&wp_viewport::INTERFACE);
    p.send(&mut srv, vpr, wp_viewporter::Request::GetViewport { id: vp, surface: w.surface });
    p.send(&mut srv, vp, wp_viewport::Request::SetDestination { width: 40, height: 20 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert_eq!(srv.scene().windows()[0].surface_rect().width, 40);
    let (x, y) = srv.scene().windows()[0].position();
    let fb = render(&mut srv);
    assert_eq!(fb.pixel((x + 39) as u32, (y + 19) as u32), Some(0x123456), "scaled up");
    // Unset: back to the buffer's size; a 90° transform swaps it.
    p.send(&mut srv, vp, wp_viewport::Request::SetDestination { width: -1, height: -1 });
    p.send(&mut srv, w.surface, wl_surface::Request::SetBufferTransform { transform: 1 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert_eq!(srv.scene().windows()[0].content().logical_size(), (2, 4));
    // A source outside the buffer, a fractional source without a
    // destination: errors when applied.
    let fixed = |v: i32| Fixed(v);
    p.send(
        &mut srv,
        vp,
        wp_viewport::Request::SetSource { x: fixed(0), y: fixed(0), width: fixed(512), height: fixed(1280) },
    );
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert_eq!(p.code(), Some((vp, wp_viewport::Error::OUT_OF_BUFFER.0)));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let vpr = p.bind(&mut srv, registry, &wp_viewporter::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "v2", 4, 4, 0);
    let vp = p.new_id(&wp_viewport::INTERFACE);
    p.send(&mut srv, vpr, wp_viewporter::Request::GetViewport { id: vp, surface: w.surface });
    let again = p.new_id(&wp_viewport::INTERFACE);
    p.send(
        &mut srv,
        vp,
        wp_viewport::Request::SetSource { x: fixed(0), y: fixed(0), width: fixed(300), height: fixed(256) },
    );
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert_eq!(p.code(), Some((vp, wp_viewport::Error::BAD_SIZE.0)));
    let _ = again;
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let vpr = p.bind(&mut srv, registry, &wp_viewporter::INTERFACE, 1);
    let s = surface(&mut p, &mut srv, &g);
    let vp = p.new_id(&wp_viewport::INTERFACE);
    p.send(&mut srv, vpr, wp_viewporter::Request::GetViewport { id: vp, surface: s });
    let vp2 = p.new_id(&wp_viewport::INTERFACE);
    p.send(&mut srv, vpr, wp_viewporter::Request::GetViewport { id: vp2, surface: s });
    assert_eq!(p.code(), Some((vpr, wp_viewporter::Error::VIEWPORT_EXISTS.0)));
}

// ---------------------------------------------------------------------
// The clipboard and drag and drop.

/// A data device for client `p`.
fn data_device(p: &mut Peer, srv: &mut Server, registry: ObjectId) -> (ObjectId, ObjectId) {
    let manager = p.bind(srv, registry, &wl_data_device_manager::INTERFACE, 3);
    let seat = p.bind(srv, registry, &wl_seat::INTERFACE, 7);
    let device = p.new_id(&wl_data_device::INTERFACE);
    p.send(srv, manager, wl_data_device_manager::Request::GetDataDevice { id: device, seat });
    p.take();
    (manager, device)
}

fn source(p: &mut Peer, srv: &mut Server, manager: ObjectId, mimes: &[&str]) -> ObjectId {
    let s = p.new_id(&wl_data_source::INTERFACE);
    p.send(srv, manager, wl_data_device_manager::Request::CreateDataSource { id: s });
    for m in mimes {
        p.send(srv, s, wl_data_source::Request::Offer { mime_type: (*m).into() });
    }
    s
}

#[test]
fn the_selection_goes_to_the_focused_client_and_data_passes_between_clients() {
    let (mut srv, shared) = server();
    let (mut a, ra) = connect(&mut srv, &shared);
    let ga = globals(&mut a, &mut srv, ra);
    let (ma, _da) = data_device(&mut a, &mut srv, ra);
    mapped(&mut a, &mut srv, &ga, "a", 10, 10, 0);
    let (mut b, rb) = connect(&mut srv, &shared);
    let gb = globals(&mut b, &mut srv, rb);
    let (_mb, db) = data_device(&mut b, &mut srv, rb);
    mapped(&mut b, &mut srv, &gb, "b", 10, 10, 0);
    b.take();
    a.read(&mut srv);
    a.take();
    // A copies; B, which has the focus, is offered it.
    let src = source(&mut a, &mut srv, ma, &["text/plain;charset=utf-8", "UTF8_STRING"]);
    a.send(&mut srv, _da, wl_data_device::Request::SetSelection { source: Some(src), serial: 1 });
    b.read(&mut srv);
    let e = b.take();
    assert_eq!(
        names(&e),
        [
            ("wl_data_device", "data_offer"),
            ("wl_data_offer", "offer"),
            ("wl_data_offer", "offer"),
            ("wl_data_device", "selection")
        ]
    );
    let Arg::NewId(offer) = e[0].args[0] else { panic!() };
    assert_eq!(e[1].args, [Arg::Str(Some("text/plain;charset=utf-8".into()))]);
    assert_eq!((e[3].object, &e[3].args[0]), (db, &Arg::Object(Some(offer))));
    // B pastes: its pipe goes to A as `send`.
    b.send(&mut srv, offer, wl_data_offer::Request::Receive { mime_type: "UTF8_STRING".into(), fd: Fd(4242) });
    a.read(&mut srv);
    let e = a.take();
    assert_eq!(names(&e), [("wl_data_source", "send")]);
    assert_eq!(e[0].args, [Arg::Str(Some("UTF8_STRING".into())), Arg::Fd(Fd(4242))]);
    // A new selection cancels the old source; a cleared one is no offer.
    let src2 = source(&mut a, &mut srv, ma, &["text/plain"]);
    a.send(&mut srv, _da, wl_data_device::Request::SetSelection { source: Some(src2), serial: 2 });
    assert!(a.take().iter().any(|e| e.object == src && e.name() == ("wl_data_source", "cancelled")));
    // Using a source twice is an error.
    a.send(&mut srv, src2, wl_data_source::Request::Destroy);
    b.read(&mut srv);
    let e = b.take();
    assert_eq!(e.last().unwrap().name(), ("wl_data_device", "selection"));
    assert_eq!(e.last().unwrap().args, [Arg::Object(None)]);
    let src3 = source(&mut a, &mut srv, ma, &["text/plain"]);
    a.send(&mut srv, _da, wl_data_device::Request::SetSelection { source: Some(src3), serial: 3 });
    a.send(&mut srv, _da, wl_data_device::Request::SetSelection { source: Some(src3), serial: 4 });
    assert_eq!(a.code(), Some((_da, wl_data_device::Error::USED_SOURCE.0)));
}

#[test]
fn a_new_focus_gets_the_selection_before_the_keyboard() {
    let (mut srv, shared) = server();
    srv.set_seat_capabilities(Capabilities::KEYBOARD);
    let (mut a, ra) = connect(&mut srv, &shared);
    let ga = globals(&mut a, &mut srv, ra);
    let (ma, da) = data_device(&mut a, &mut srv, ra);
    mapped(&mut a, &mut srv, &ga, "a", 10, 10, 0);
    let src = source(&mut a, &mut srv, ma, &["text/plain"]);
    a.send(&mut srv, da, wl_data_device::Request::SetSelection { source: Some(src), serial: 1 });
    let (mut b, rb) = connect(&mut srv, &shared);
    let gb = globals(&mut b, &mut srv, rb);
    let seat = b.bind(&mut srv, rb, &wl_seat::INTERFACE, 7);
    let keyboard = b.new_id(&wl_keyboard::INTERFACE);
    b.send(&mut srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    data_device(&mut b, &mut srv, rb);
    mapped(&mut b, &mut srv, &gb, "b", 10, 10, 0);
    let n = names(&b.take());
    let selection = n.iter().position(|e| *e == ("wl_data_device", "selection")).expect("a selection");
    let enter = n.iter().position(|e| *e == ("wl_keyboard", "enter")).expect("keyboard enter");
    assert!(selection < enter, "{n:?}");
}

#[test]
fn drag_and_drop_between_clients() {
    let (mut srv, shared) = server();
    let (mut a, ra, ga, _pa) = pointer_client(&mut srv, &shared);
    let (ma, da) = data_device(&mut a, &mut srv, ra);
    let wa = mapped(&mut a, &mut srv, &ga, "a", 50, 50, 0);
    let (mut b, rb, gb, _pb) = pointer_client(&mut srv, &shared);
    let (_mb, db) = data_device(&mut b, &mut srv, rb);
    let wb = mapped(&mut b, &mut srv, &gb, "b", 50, 50, 0);
    let (ax, ay) = srv.scene().window(srv.surface_window(a.id, wa.surface).unwrap()).unwrap().position();
    let (bx, by) = srv.scene().window(srv.surface_window(b.id, wb.surface).unwrap()).unwrap().position();
    b.take();
    a.read(&mut srv);
    a.take();
    // Press in A (B's window is placed further down and right), then drag.
    motion(&mut srv, ax + 5, ay + 5);
    button(&mut srv, true);
    a.read(&mut srv);
    a.take();
    b.read(&mut srv);
    b.take();
    let src = source(&mut a, &mut srv, ma, &["text/uri-list"]);
    a.send(&mut srv, src, wl_data_source::Request::SetActions { dnd_actions: 1 | 2 });
    let icon = surface(&mut a, &mut srv, &ga);
    a.send(
        &mut srv,
        da,
        wl_data_device::Request::StartDrag { source: Some(src), origin: wa.surface, icon: Some(icon), serial: 1 },
    );
    assert!(srv.is_dragging());
    assert!(a.take().iter().any(|e| e.name() == ("wl_pointer", "leave")), "the pointer leaves the origin");
    // The icon follows the pointer once it has content.
    let (ib, _) = buffer(&mut a, &mut srv, &ga, 4, 4, 0x00ff_00ff, 1);
    attach(&mut a, &mut srv, icon, ib);
    // Over B.
    motion(&mut srv, bx + 40, by + 40);
    b.read(&mut srv);
    let e = b.take();
    assert_eq!(
        names(&e),
        [
            ("wl_data_device", "data_offer"),
            ("wl_data_offer", "offer"),
            ("wl_data_offer", "source_actions"),
            ("wl_data_device", "enter")
        ]
    );
    let Arg::NewId(offer) = e[0].args[0] else { panic!() };
    assert_eq!(e[2].args, [Arg::Uint(3)]);
    assert_eq!(e[3].args[1], Arg::Object(Some(wb.surface)));
    let icon_window = srv.surface_window(a.id, icon).expect("the icon is shown");
    assert_eq!(srv.scene().window(icon_window).unwrap().position(), (bx + 40, by + 40));
    motion(&mut srv, bx + 41, by + 40);
    b.read(&mut srv);
    assert_eq!(b.names(), [("wl_data_device", "motion")]);
    // B takes it, as a move; A hears the target and the action.
    b.send(&mut srv, offer, wl_data_offer::Request::Accept { serial: 0, mime_type: Some("text/uri-list".into()) });
    b.send(&mut srv, offer, wl_data_offer::Request::SetActions { dnd_actions: 1 | 2, preferred_action: 2 });
    assert_eq!(b.take().last().unwrap().args, [Arg::Uint(2)], "offer.action: move");
    a.read(&mut srv);
    // (A was over its own window first: it heard `leave` when the drag
    // went on to B.)
    let n = names(&a.take());
    assert!(n.contains(&("wl_data_device", "leave")), "{n:?}");
    assert!(n.ends_with(&[("wl_data_source", "target"), ("wl_data_source", "action")]), "{n:?}");
    // Drop.
    button(&mut srv, false);
    b.read(&mut srv);
    assert!(b.names().contains(&("wl_data_device", "drop")));
    a.read(&mut srv);
    assert!(a.names().contains(&("wl_data_source", "dnd_drop_performed")));
    assert!(!srv.is_dragging());
    assert!(srv.surface_window(a.id, icon).is_none(), "the icon is gone");
    b.send(&mut srv, offer, wl_data_offer::Request::Receive { mime_type: "text/uri-list".into(), fd: Fd(77) });
    b.send(&mut srv, offer, wl_data_offer::Request::Finish);
    a.read(&mut srv);
    assert_eq!(names(&a.take()), [("wl_data_source", "send"), ("wl_data_source", "dnd_finished")]);
    let _ = db;
    assert!(a.error().is_none() && b.error().is_none());
}

#[test]
fn a_drop_nobody_accepted_is_cancelled() {
    let (mut srv, shared) = server();
    let (mut a, ra, ga, _pa) = pointer_client(&mut srv, &shared);
    let (ma, da) = data_device(&mut a, &mut srv, ra);
    let wa = mapped(&mut a, &mut srv, &ga, "a", 50, 50, 0);
    let (ax, ay) = srv.scene().windows()[0].position();
    motion(&mut srv, ax + 5, ay + 5);
    button(&mut srv, true);
    let src = source(&mut a, &mut srv, ma, &["text/plain"]);
    a.send(
        &mut srv,
        da,
        wl_data_device::Request::StartDrag { source: Some(src), origin: wa.surface, icon: None, serial: 1 },
    );
    motion(&mut srv, 600, 400);
    button(&mut srv, false);
    a.read(&mut srv);
    assert!(a.take().iter().any(|e| e.object == src && e.name() == ("wl_data_source", "cancelled")));
    // Without a button held there is no drag.
    let src = source(&mut a, &mut srv, ma, &["text/plain"]);
    a.send(
        &mut srv,
        da,
        wl_data_device::Request::StartDrag { source: Some(src), origin: wa.surface, icon: None, serial: 1 },
    );
    assert!(!srv.is_dragging());
    assert!(a.take().iter().any(|e| e.object == src && e.name() == ("wl_data_source", "cancelled")));
}

#[test]
fn drag_actions_are_negotiated() {
    use super::super::data_device::choose_action;
    assert_eq!(choose_action(1 | 2, 1 | 2, 2), 2);
    assert_eq!(choose_action(1, 1 | 2, 2), 1);
    assert_eq!(choose_action(2 | 4, 4, 0), 4);
    assert_eq!(choose_action(1, 2, 2), 0);
}

// ---------------------------------------------------------------------
// dma-bufs.

#[test]
fn dmabuf_feedback_names_the_device_and_the_formats() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let dmabuf = p.bind(&mut srv, registry, &zwp_linux_dmabuf_v1::INTERFACE, 4);
    assert!(p.take().is_empty(), "version 4 sends no format events");
    let fb = p.new_id(&zwp_linux_dmabuf_feedback_v1::INTERFACE);
    p.send(&mut srv, dmabuf, zwp_linux_dmabuf_v1::Request::GetDefaultFeedback { id: fb });
    let e = p.take();
    assert_eq!(
        names(&e),
        [
            ("zwp_linux_dmabuf_feedback_v1", "format_table"),
            ("zwp_linux_dmabuf_feedback_v1", "main_device"),
            ("zwp_linux_dmabuf_feedback_v1", "tranche_target_device"),
            ("zwp_linux_dmabuf_feedback_v1", "tranche_flags"),
            ("zwp_linux_dmabuf_feedback_v1", "tranche_formats"),
            ("zwp_linux_dmabuf_feedback_v1", "tranche_done"),
            ("zwp_linux_dmabuf_feedback_v1", "done"),
        ]
    );
    assert_eq!(e[0].args, [Arg::Fd(Fd(TABLE_FD)), Arg::Uint(8 * 16)]);
    assert_eq!(e[1].args, [Arg::Array(DMABUF_DEVICE.to_ne_bytes().to_vec())]);
    let Arg::Array(indices) = &e[4].args[0] else { panic!() };
    assert_eq!(indices.len(), 8 * 2);
    let table = shared.borrow().tables[0].clone();
    assert_eq!(&table[..4], b"AR24");
    assert_eq!(&table[8..16], &0u64.to_ne_bytes(), "linear");
    // Version 3: format and modifier events.
    let (mut p, registry) = connect(&mut srv, &shared);
    p.bind(&mut srv, registry, &zwp_linux_dmabuf_v1::INTERFACE, 3);
    let n = p.names();
    assert_eq!(n.iter().filter(|e| e.1 == "format").count(), 4);
    assert_eq!(n.iter().filter(|e| e.1 == "modifier").count(), 8);
}

/// A dma-buf (test memory) as a buffer: imported, attached, composed.
#[test]
fn dmabuf_buffers_are_imported_and_composed() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let dmabuf = p.bind(&mut srv, registry, &zwp_linux_dmabuf_v1::INTERFACE, 4);
    let (fd, mem) = p.memfd(16 * 8 * 4);
    for px in mem.borrow_mut().chunks_exact_mut(4) {
        // ABGR8888 little-endian: R, G, B, A.
        px.copy_from_slice(&[0x11, 0x22, 0x33, 0xff]);
    }
    let params = p.new_id(&zwp_linux_buffer_params_v1::INTERFACE);
    p.send(&mut srv, dmabuf, zwp_linux_dmabuf_v1::Request::CreateParams { params_id: params });
    p.send(
        &mut srv,
        params,
        zwp_linux_buffer_params_v1::Request::Add {
            fd,
            plane_idx: 0,
            offset: 0,
            stride: 64,
            modifier_hi: 0,
            modifier_lo: 0,
        },
    );
    let buffer = p.new_id(&wl_buffer::INTERFACE);
    let ab24 = u32::from_le_bytes(*b"AB24");
    p.send(
        &mut srv,
        params,
        zwp_linux_buffer_params_v1::Request::CreateImmed {
            buffer_id: buffer,
            width: 16,
            height: 8,
            format: ab24,
            flags: 0,
        },
    );
    assert!(p.error().is_none(), "{:?}", p.error());
    let (s, xdg, _tl, events) = toplevel(&mut p, &mut srv, &g, "dmabuf");
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    attach(&mut p, &mut srv, s, buffer);
    assert!(p.take().iter().any(|e| e.object == buffer && e.name() == ("wl_buffer", "release")));
    let (x, y) = srv.scene().windows()[0].position();
    let fb = render(&mut srv);
    assert_eq!(fb.pixel(x as u32 + 3, y as u32 + 3), Some(0x112233), "red and blue swapped back");
    // The non-immediate way: created with a buffer the server makes.
    let (fd2, _m) = p.memfd(64);
    let params = p.new_id(&zwp_linux_buffer_params_v1::INTERFACE);
    p.send(&mut srv, dmabuf, zwp_linux_dmabuf_v1::Request::CreateParams { params_id: params });
    p.send(
        &mut srv,
        params,
        zwp_linux_buffer_params_v1::Request::Add {
            fd: fd2,
            plane_idx: 0,
            offset: 0,
            stride: 16,
            modifier_hi: 0,
            modifier_lo: 0,
        },
    );
    let xr24 = u32::from_le_bytes(*b"XR24");
    p.send(
        &mut srv,
        params,
        zwp_linux_buffer_params_v1::Request::Create { width: 4, height: 4, format: xr24, flags: 0 },
    );
    let e = p.take();
    assert_eq!(names(&e), [("zwp_linux_buffer_params_v1", "created")]);
}

#[test]
fn dmabuf_params_errors() {
    use zwp_linux_buffer_params_v1::Error as E;
    let cases: [(&str, u32); 4] = [
        ("plane 4", E::PLANE_IDX.0),
        ("no plane", E::INCOMPLETE.0),
        ("too small", E::OUT_OF_BOUNDS.0),
        ("format", E::INVALID_FORMAT.0),
    ];
    for (case, code) in cases {
        let (mut srv, shared) = server();
        let (mut p, registry) = connect(&mut srv, &shared);
        let dmabuf = p.bind(&mut srv, registry, &zwp_linux_dmabuf_v1::INTERFACE, 4);
        let params = p.new_id(&zwp_linux_buffer_params_v1::INTERFACE);
        p.send(&mut srv, dmabuf, zwp_linux_dmabuf_v1::Request::CreateParams { params_id: params });
        let (fd, _m) = p.memfd(64);
        let plane = if case == "plane 4" { 4 } else { 0 };
        if case != "no plane" {
            let add = zwp_linux_buffer_params_v1::Request::Add {
                fd,
                plane_idx: plane,
                offset: 0,
                stride: 16,
                modifier_hi: 0,
                modifier_lo: 0,
            };
            p.send(&mut srv, params, add);
        }
        let format = if case == "format" { 0x3231_5659 } else { u32::from_le_bytes(*b"XR24") };
        let height = if case == "too small" { 40 } else { 4 };
        let buffer = p.new_id(&wl_buffer::INTERFACE);
        let create =
            zwp_linux_buffer_params_v1::Request::CreateImmed { buffer_id: buffer, width: 4, height, format, flags: 0 };
        p.send(&mut srv, params, create);
        assert_eq!(p.code(), Some((params, code)), "{case}");
    }
}

// ---------------------------------------------------------------------
// Popups: constraints and grabs.

#[test]
fn positioner_constraint_adjustment() {
    let area = R::new(0, 0, 100, 100);
    let base = Positioner {
        size: Some((30, 20)),
        anchor_rect: Some(R::new(0, 0, 10, 10)),
        anchor: 8,  // bottom right
        gravity: 8, // bottom right
        constraint_adjustment: 0,
        offset: (0, 0),
    };
    // At the parent's origin (80, 85) the popup would end at (120, 115).
    let origin = (80, 85);
    assert_eq!(base.place_in(origin, area), Some(R::new(10, 10, 30, 20)), "no adjustment allowed");
    let flip = Positioner { constraint_adjustment: 4 | 8, ..base };
    // Flipped: anchored top-left, growing up and left.
    assert_eq!(flip.place_in(origin, area), Some(R::new(-30, -20, 30, 20)));
    let slide = Positioner { constraint_adjustment: 1 | 2, ..base };
    assert_eq!(slide.place_in(origin, area), Some(R::new(-10, -5, 30, 20)));
    let resize = Positioner { constraint_adjustment: 16 | 32, ..base };
    assert_eq!(resize.place_in(origin, area), Some(R::new(10, 10, 10, 5)));
    // A flip that does not help is not kept; sliding then does.
    let tall = Positioner { size: Some((30, 150)), constraint_adjustment: 8 | 2, ..base };
    assert_eq!(tall.place_in((0, 0), area).map(|r| r.y), Some(0));
}

/// A popup with a grab: it takes the keyboard; a press outside its client
/// dismisses it.
#[test]
fn popup_grabs_take_the_keyboard_and_a_click_outside_dismisses() {
    let (mut srv, shared) = server();
    let (mut p, registry, g, _pointer) = pointer_client(&mut srv, &shared);
    let seat = p.bind(&mut srv, registry, &wl_seat::INTERFACE, 7);
    let keyboard = p.new_id(&wl_keyboard::INTERFACE);
    p.send(&mut srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    let parent = mapped(&mut p, &mut srv, &g, "menu owner", 200, 100, 0);
    let positioner = p.new_id(&xdg_positioner::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::CreatePositioner { id: positioner });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetSize { width: 40, height: 30 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetAnchorRect { x: 0, y: 0, width: 10, height: 10 });
    let ps = surface(&mut p, &mut srv, &g);
    let px = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: px, surface: ps });
    let popup = p.new_id(&xdg_popup::INTERFACE);
    p.send(&mut srv, px, xdg_surface::Request::GetPopup { id: popup, parent: Some(parent.xdg), positioner });
    p.send(&mut srv, popup, xdg_popup::Request::Grab { seat, serial: 1 });
    p.send(&mut srv, ps, wl_surface::Request::Commit);
    let e = p.take();
    p.send(&mut srv, px, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (b, _) = buffer(&mut p, &mut srv, &g, 40, 30, 0, 1);
    attach(&mut p, &mut srv, ps, b);
    assert_eq!(srv.keyboard_focus(), Some((p.id, ps)), "the popup has the keyboard");
    // The popup is part of its parent's window.
    let pw = srv.surface_window(p.id, parent.surface).unwrap();
    let popup_window = srv.surface_window(p.id, ps).unwrap();
    assert_eq!(srv.scene().root(popup_window), pw);
    // A grab after mapping is an error (checked on another popup below).
    motion(&mut srv, 630, 470);
    button(&mut srv, true);
    button(&mut srv, false);
    p.read(&mut srv);
    assert!(p.take().iter().any(|e| e.object == popup && e.name() == ("xdg_popup", "popup_done")));
    assert!(srv.surface_window(p.id, ps).is_none());
    assert_eq!(srv.keyboard_focus(), Some((p.id, parent.surface)), "back to the toplevel");
    p.send(&mut srv, popup, xdg_popup::Request::Grab { seat, serial: 2 });
    assert!(p.error().is_none(), "an unmapped popup may grab");
}

#[test]
fn a_popup_near_the_edge_is_flipped_back_onto_the_output() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    // A wide toplevel near the right edge of the 640 × 480 output.
    let parent = mapped(&mut p, &mut srv, &g, "p", 560, 100, 0);
    let (wx, _) = srv.scene().windows()[0].position();
    let positioner = p.new_id(&xdg_positioner::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::CreatePositioner { id: positioner });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetSize { width: 100, height: 30 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetAnchorRect { x: 540, y: 0, width: 10, height: 10 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetAnchor { anchor: 8 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetGravity { gravity: 8 });
    p.send(&mut srv, positioner, xdg_positioner::Request::SetConstraintAdjustment { constraint_adjustment: 4 });
    let ps = surface(&mut p, &mut srv, &g);
    let px = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: px, surface: ps });
    let popup = p.new_id(&xdg_popup::INTERFACE);
    p.send(&mut srv, px, xdg_surface::Request::GetPopup { id: popup, parent: Some(parent.xdg), positioner });
    p.take();
    p.send(&mut srv, ps, wl_surface::Request::Commit);
    let e = p.take();
    let c = e.iter().find(|e| e.name() == ("xdg_popup", "configure")).unwrap();
    // Unflipped it would start at 550 + wx and end past 640: flipped to
    // end at the anchor rectangle's left edge.
    assert!(wx + 550 + 100 > 640);
    assert_eq!(c.args[..2], [Arg::Int(540 - 100), Arg::Int(10)]);
}

// ---------------------------------------------------------------------
// Interactive resize.

#[test]
fn interactive_resize_configures_sizes_and_keeps_the_far_edge() {
    let (mut srv, shared) = server();
    let (mut p, registry, g, _pointer) = pointer_client(&mut srv, &shared);
    let seat = p.bind(&mut srv, registry, &wl_seat::INTERFACE, 7);
    let w = mapped(&mut p, &mut srv, &g, "r", 100, 80, 0);
    let (x, y) = srv.scene().windows()[0].position();
    motion(&mut srv, x + 2, y + 2);
    button(&mut srv, true);
    p.take();
    // From the top-left corner, dragged up and left by (10, 20).
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::Resize { seat, serial: 1, edges: 5 });
    motion(&mut srv, x - 8, y - 18);
    p.read(&mut srv);
    let e = p.take();
    let c = e.iter().find(|e| e.name() == ("xdg_toplevel", "configure")).expect("a configure");
    assert_eq!(c.args[..2], [Arg::Int(110), Arg::Int(100)]);
    let Arg::Array(states) = &c.args[2] else { panic!() };
    assert!(states.chunks(4).any(|s| s == xdg_toplevel::State::RESIZING.0.to_ne_bytes()));
    // The client follows: the bottom right stays put.
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (b, _) = buffer(&mut p, &mut srv, &g, 110, 100, 0, 1);
    attach(&mut p, &mut srv, w.surface, b);
    let win = srv.scene().windows()[0].clone();
    assert_eq!(win.position(), (x - 10, y - 20));
    assert_eq!(win.bounds().right(), x + 100);
    // Release: one more configure, no longer resizing.
    button(&mut srv, false);
    p.read(&mut srv);
    let e = p.take();
    let c = e.iter().find(|e| e.name() == ("xdg_toplevel", "configure")).expect("a last configure");
    let Arg::Array(states) = &c.args[2] else { panic!() };
    assert!(!states.chunks(4).any(|s| s == xdg_toplevel::State::RESIZING.0.to_ne_bytes()));
    assert!(!srv.is_moving());
}

// ---------------------------------------------------------------------
// Cursors.

#[test]
fn cursor_surfaces_replace_the_arrow_while_the_pointer_is_over_their_client() {
    let (mut srv, shared) = server();
    let (mut p, registry, g, pointer) = pointer_client(&mut srv, &shared);
    let _ = registry;
    srv.set_cursor_visible(true);
    let w = mapped(&mut p, &mut srv, &g, "c", 100, 80, 0);
    let (x, y) = srv.scene().windows()[0].position();
    motion(&mut srv, x + 10, y + 10);
    p.read(&mut srv);
    let cursor = surface(&mut p, &mut srv, &g);
    p.send(
        &mut srv,
        pointer,
        wl_pointer::Request::SetCursor { serial: 1, surface: Some(cursor), hotspot_x: 2, hotspot_y: 3 },
    );
    let (b, _) = buffer(&mut p, &mut srv, &g, 6, 6, 0xff12_3456, 0);
    attach(&mut p, &mut srv, cursor, b);
    let CursorLook::Image { hx, hy, content } = srv.scene().cursor_look().clone() else { panic!("an image cursor") };
    assert_eq!((hx, hy, content.width), (2, 3, 6));
    let fb = render(&mut srv);
    assert_eq!(fb.pixel((x + 10 - 2) as u32, (y + 10 - 3) as u32), Some(0x123456));
    // An offset moves the hot spot.
    p.send(&mut srv, cursor, wl_surface::Request::Offset { x: 1, y: 1 });
    p.send(&mut srv, cursor, wl_surface::Request::Commit);
    assert!(matches!(srv.scene().cursor_look(), CursorLook::Image { hx: 1, hy: 2, .. }));
    // Off the client: the arrow.
    motion(&mut srv, 630, 470);
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Arrow);
    // A null surface hides it; a role conflict is an error.
    motion(&mut srv, x + 10, y + 10);
    p.send(&mut srv, pointer, wl_pointer::Request::SetCursor { serial: 2, surface: None, hotspot_x: 0, hotspot_y: 0 });
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Hidden);
    p.send(
        &mut srv,
        pointer,
        wl_pointer::Request::SetCursor { serial: 3, surface: Some(w.surface), hotspot_x: 0, hotspot_y: 0 },
    );
    assert_eq!(p.code(), Some((pointer, wl_pointer::Error::ROLE.0)));
}

#[test]
fn cursor_shapes_show_the_arrow() {
    let (mut srv, shared) = server();
    let (mut p, registry, g, pointer) = pointer_client(&mut srv, &shared);
    let manager = p.bind(&mut srv, registry, &wp_cursor_shape_manager_v1::INTERFACE, 1);
    mapped(&mut p, &mut srv, &g, "s", 100, 80, 0);
    let (x, y) = srv.scene().windows()[0].position();
    motion(&mut srv, x + 10, y + 10);
    let device = p.new_id(&wp_cursor_shape_device_v1::INTERFACE);
    p.send(&mut srv, manager, wp_cursor_shape_manager_v1::Request::GetPointer { cursor_shape_device: device, pointer });
    p.send(&mut srv, device, wp_cursor_shape_device_v1::Request::SetShape { serial: 1, shape: 2 });
    assert_eq!(srv.cursor_request(), Some(super::super::cursor::CursorRequest::Shape(2)));
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Arrow);
    // Version 1 has 34 shapes.
    p.send(&mut srv, device, wp_cursor_shape_device_v1::Request::SetShape { serial: 1, shape: 35 });
    assert_eq!(p.code(), Some((device, wp_cursor_shape_device_v1::Error::INVALID_SHAPE.0)));
}

// ---------------------------------------------------------------------
// Fractional scale, activation.

#[test]
fn fractional_scale_is_the_output_scale_in_120ths() {
    let (mut srv, shared) = server_with(OutputConfig { width: 1280, height: 960, scale: 2, ..OutputConfig::default() });
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &wp_fractional_scale_manager_v1::INTERFACE, 1);
    let s = surface(&mut p, &mut srv, &g);
    let f = p.new_id(&wp_fractional_scale_v1::INTERFACE);
    p.send(&mut srv, manager, wp_fractional_scale_manager_v1::Request::GetFractionalScale { id: f, surface: s });
    let e = p.take();
    assert_eq!((e[0].name(), &e[0].args[..]), (("wp_fractional_scale_v1", "preferred_scale"), &[Arg::Uint(240)][..]));
    srv.set_output(OutputConfig { width: 1280, height: 960, scale: 1, ..OutputConfig::default() });
    p.read(&mut srv);
    assert!(p.take().iter().any(|e| e.object == f && e.args == [Arg::Uint(120)]));
    let f2 = p.new_id(&wp_fractional_scale_v1::INTERFACE);
    p.send(&mut srv, manager, wp_fractional_scale_manager_v1::Request::GetFractionalScale { id: f2, surface: s });
    assert_eq!(p.code(), Some((manager, wp_fractional_scale_manager_v1::Error::FRACTIONAL_SCALE_EXISTS.0)));
}

#[test]
fn activation_tokens_activate_a_toplevel_once() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let activation = p.bind(&mut srv, registry, &xdg_activation_v1::INTERFACE, 1);
    let a = mapped(&mut p, &mut srv, &g, "a", 20, 20, 0);
    let b = mapped(&mut p, &mut srv, &g, "b", 20, 20, 0);
    assert_eq!(srv.active, Some((p.id, b.toplevel)));
    let token = p.new_id(&xdg_activation_token_v1::INTERFACE);
    p.send(&mut srv, activation, xdg_activation_v1::Request::GetActivationToken { id: token });
    p.send(&mut srv, token, xdg_activation_token_v1::Request::SetAppId { app_id: "a".into() });
    p.send(&mut srv, token, xdg_activation_token_v1::Request::Commit);
    let e = p.take();
    let done = e.iter().find(|e| e.name() == ("xdg_activation_token_v1", "done")).expect("done");
    let Arg::Str(Some(t)) = &done.args[0] else { panic!() };
    p.send(&mut srv, activation, xdg_activation_v1::Request::Activate { token: t.clone(), surface: a.surface });
    assert_eq!(srv.active, Some((p.id, a.toplevel)));
    // Used once: a second use does nothing.
    p.send(&mut srv, activation, xdg_activation_v1::Request::Activate { token: t.clone(), surface: b.surface });
    assert_eq!(srv.active, Some((p.id, a.toplevel)));
    // The compositor can hand out tokens too (an app the shell starts).
    let t = srv.activation_token();
    p.send(&mut srv, activation, xdg_activation_v1::Request::Activate { token: t, surface: b.surface });
    assert_eq!(srv.active, Some((p.id, b.toplevel)));
    p.send(&mut srv, token, xdg_activation_token_v1::Request::Commit);
    assert_eq!(p.code(), Some((token, xdg_activation_token_v1::Error::ALREADY_USED.0)));
}

/// A window that asks for the focus before it is first shown (as GTK and
/// Qt present a new window), launched after another app's window closed:
/// it is drawn active once mapped, and a click on it keeps it so. (It was
/// drawn inactive for good: the server thought it active already, so the
/// map and later clicks changed nothing, though keys reached it.)
#[test]
fn a_window_activated_before_its_first_map_is_drawn_active() {
    let (mut srv, shared) = server();
    // The first app: shown, active, then closed.
    let (mut p1, registry1) = connect(&mut srv, &shared);
    let g1 = globals(&mut p1, &mut srv, registry1);
    let first = mapped(&mut p1, &mut srv, &g1, "first", 40, 40, 0);
    assert_eq!(srv.active, Some((p1.id, first.toplevel)));
    srv.remove_client(p1.id);
    assert_eq!(srv.active, None);
    // The second app activates its toplevel before the first commit with a
    // buffer.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let activation = p.bind(&mut srv, registry, &xdg_activation_v1::INTERFACE, 1);
    let (surface, xdg, toplevel, events) = toplevel(&mut p, &mut srv, &g, "second");
    let token = p.new_id(&xdg_activation_token_v1::INTERFACE);
    p.send(&mut srv, activation, xdg_activation_v1::Request::GetActivationToken { id: token });
    p.send(&mut srv, token, xdg_activation_token_v1::Request::SetSurface { surface });
    p.send(&mut srv, token, xdg_activation_token_v1::Request::Commit);
    let e = p.take();
    let done = e.iter().find(|e| e.name() == ("xdg_activation_token_v1", "done")).expect("done");
    let Arg::Str(Some(t)) = &done.args[0] else { panic!() };
    p.send(&mut srv, activation, xdg_activation_v1::Request::Activate { token: t.clone(), surface });
    assert_eq!(srv.active, Some((p.id, toplevel)));
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&events) });
    let (buffer, _) = buffer(&mut p, &mut srv, &g, 40, 40, 0, wl_shm::Format::XRGB8888.0);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    let win = srv.surface_window(p.id, surface).expect("mapped");
    assert!(srv.scene().window(win).unwrap().is_active(), "the new window is drawn active");
    assert_eq!(srv.active_window(), Some(win));
    assert!(srv.managed_windows().iter().any(|w| w.window == win && w.active));
    // A click on its content keeps it active.
    let (x, y) = srv.scene().window(win).unwrap().position();
    motion(&mut srv, x + 10, y + 10);
    button(&mut srv, true);
    button(&mut srv, false);
    assert!(srv.scene().window(win).unwrap().is_active());
}

// ---------------------------------------------------------------------
// The frame clock of page flips.

/// Callbacks committed before a frame went to the display are answered
/// when its flip completes, with the vblank's time and sequence and the
/// vsync flags; ones committed after wait for the next.
#[test]
fn frame_callbacks_follow_flip_completion() {
    let (mut srv, shared) = server_with(OutputConfig { refresh_mhz: 75_000, ..OutputConfig::default() });
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let pres = p.bind(&mut srv, registry, &wp_presentation::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "f", 4, 4, 0);
    let first = p.new_id(&wl_callback::INTERFACE);
    p.send(&mut srv, w.surface, wl_surface::Request::Frame { callback: first });
    let fb = p.new_id(&wp_presentation_feedback::INTERFACE);
    p.send(&mut srv, pres, wp_presentation::Request::Feedback { surface: w.surface, callback: fb });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    p.take();
    srv.frame_submitted();
    assert!(srv.frame_in_flight());
    // Committed while the flip is pending.
    let second = p.new_id(&wl_callback::INTERFACE);
    p.send(&mut srv, w.surface, wl_surface::Request::Frame { callback: second });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let flags = present_flags::VSYNC | present_flags::HW_CLOCK | present_flags::HW_COMPLETION;
    srv.frame_completed(FrameTime { monotonic_ns: 5_013_333_333 }, Some(376), flags);
    p.read(&mut srv);
    let e = p.take();
    let done: Vec<ObjectId> = e.iter().filter(|e| e.name() == ("wl_callback", "done")).map(|e| e.object).collect();
    assert_eq!(done, [first]);
    assert_eq!(e[0].args, [Arg::Uint(5013)], "milliseconds");
    let presented = e.iter().find(|e| e.name() == ("wp_presentation_feedback", "presented")).unwrap();
    assert_eq!(presented.args[3], Arg::Uint(13_333_333), "75 Hz");
    assert_eq!(presented.args[5..], [Arg::Uint(376), Arg::Uint(flags)]);
    assert!(!srv.frame_in_flight());
    assert!(srv.wants_frame(), "the second waits for the next frame");
    srv.frame_submitted();
    srv.frame_completed(FrameTime { monotonic_ns: 5_026_666_666 }, Some(377), flags);
    p.read(&mut srv);
    assert!(p.take().iter().any(|e| e.object == second && e.name() == ("wl_callback", "done")));
}

// ---------------------------------------------------------------------
// The extension point for an outside window manager (XWayland's).

#[test]
fn an_outside_window_manager_manages_surfaces_like_toplevels() {
    let (mut srv, shared) = server();
    let (mut p, _registry, g, _pointer) = pointer_client(&mut srv, &shared);
    let s = surface(&mut p, &mut srv, &g);
    assert!(srv.manage_surface(p.id, s, true));
    srv.set_managed_title(p.id, s, "xterm");
    srv.move_managed(p.id, s, 100, 80);
    assert!(srv.scene().windows().is_empty(), "shown once it has content");
    let (b, _) = buffer(&mut p, &mut srv, &g, 60, 40, 0x0011_2233, 1);
    attach(&mut p, &mut srv, s, b);
    let w = srv.managed_window(p.id, s).expect("shown");
    let win = srv.scene().window(w).unwrap();
    assert_eq!((win.position(), win.title(), win.is_decorated(), win.is_active()), ((100, 80), "xterm", true, true));
    assert_eq!(srv.keyboard_focus(), Some((p.id, s)), "the keyboard follows it");
    // Its close button is the window manager's to act on.
    let outer = srv.scene().frame_rect(w).unwrap();
    let close = (outer.x..outer.right())
        .flat_map(|x| (outer.y..outer.bottom()).map(move |y| (x, y)))
        .find(|&(x, y)| srv.scene().hit_test(x, y) == Hit::Close(w))
        .expect("a close button");
    motion(&mut srv, close.0, close.1);
    button(&mut srv, true);
    button(&mut srv, false);
    assert_eq!(srv.take_wm_events(), [WmEvent::Close { client: p.id, surface: s }]);
    assert!(srv.take_wm_events().is_empty());
    // A null buffer hides it; a surface with another role cannot be managed.
    p.send(&mut srv, s, wl_surface::Request::Attach { buffer: None, x: 0, y: 0 });
    p.send(&mut srv, s, wl_surface::Request::Commit);
    assert!(srv.managed_window(p.id, s).is_none());
    let t = mapped(&mut p, &mut srv, &g, "t", 10, 10, 0);
    assert!(!srv.manage_surface(p.id, t.surface, false));
    srv.unmanage_surface(p.id, s);
    attach(&mut p, &mut srv, s, b);
    assert!(srv.managed_window(p.id, s).is_none());
}

// ---------------------------------------------------------------------
// Backdrops.

#[test]
fn a_surface_s_backdrop_is_what_is_behind_its_window() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let below = mapped(&mut p, &mut srv, &g, "below", 200, 150, 0x00c0_0000);
    let glass = mapped(&mut p, &mut srv, &g, "glass", 50, 40, 0x0000_0000);
    assert!(srv.set_surface_backdrop(p.id, glass.surface, true));
    let mut fb = MemoryFramebuffer::new(640, 480);
    srv.render(&mut fb);
    let gw = srv.surface_window(p.id, glass.surface).unwrap();
    let (gx, gy) = srv.scene().window(gw).unwrap().position();
    let (b, damage) = srv.surface_backdrop(p.id, glass.surface).unwrap();
    assert_eq!(b.pixel(gx + 5, gy + 5), Some(0xc00000), "the window below shows through");
    assert!(!damage.is_empty());
    // The window below changes: the backdrop follows, with damage.
    let (nb, _) = buffer(&mut p, &mut srv, &g, 200, 150, 0x0000_c000, 1);
    attach(&mut p, &mut srv, below.surface, nb);
    srv.render(&mut fb);
    let (b, damage) = srv.surface_backdrop(p.id, glass.surface).unwrap();
    assert_eq!(b.pixel(gx + 5, gy + 5), Some(0x00c000));
    assert!(damage.iter().any(|d| d.contains(gx + 5, gy + 5)));
    // Nothing changed: no damage.
    srv.render(&mut fb);
    assert!(srv.surface_backdrop(p.id, glass.surface).unwrap().1.is_empty());
}
