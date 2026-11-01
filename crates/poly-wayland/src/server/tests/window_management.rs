//! Window management for the shell (ADR 0117): the frame's buttons,
//! double clicks, tiling, minimizing, the focus moving on, and internal
//! windows.

use super::*;
use crate::compose::{BORDER, CLOSE_SIZE, TITLE_HEIGHT};
use crate::server::InternalEvent;
use crate::server::input::DOUBLE_CLICK_MS;

fn motion(srv: &mut Server, x: i32, y: i32) {
    srv.handle_input(InputEvent::PointerMotionAbsolute { time: 7, x: Fixed::from_int(x), y: Fixed::from_int(y) });
}

fn click_at(srv: &mut Server, time: u32, x: i32, y: i32) {
    motion(srv, x, y);
    srv.handle_input(InputEvent::PointerButton { time, button: BTN_LEFT, pressed: true });
    srv.handle_input(InputEvent::PointerButton { time: time + 1, button: BTN_LEFT, pressed: false });
}

/// The configure among `events`: (width, height, states).
fn configure(events: &[Received]) -> (i32, i32, Vec<u32>) {
    let c = events.iter().rev().find(|e| e.name() == ("xdg_toplevel", "configure")).expect("a configure");
    let (Arg::Int(w), Arg::Int(h), Arg::Array(states)) = (&c.args[0], &c.args[1], &c.args[2]) else { panic!() };
    (*w, *h, states.chunks_exact(4).map(|b| u32::from_ne_bytes(b.try_into().unwrap())).collect())
}

/// Ack the last configure and commit a new buffer of that size.
fn ack_and_commit(p: &mut Peer, srv: &mut Server, g: &Globals, w: &Window, events: &[Received]) {
    let (cw, ch, _) = configure(events);
    p.send(srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(events) });
    let (cw, ch) = if cw > 0 { (cw, ch) } else { (50, 50) };
    let (b, _) = buffer(p, srv, g, cw, ch, 0, wl_shm::Format::XRGB8888.0);
    p.send(srv, w.surface, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(srv, w.surface, wl_surface::Request::Commit);
}

/// The title bar's buttons of the window at `pos` (its content) `width`
/// wide: (close, maximize, minimize) centres.
fn buttons(pos: (i32, i32), width: i32) -> [(i32, i32); 3] {
    let cy = pos.1 - TITLE_HEIGHT / 2;
    let close = pos.0 + width - 4 - CLOSE_SIZE / 2;
    [(close, cy), (close - 22, cy), (close - 44, cy)]
}

#[test]
fn the_maximize_button_maximizes_and_restores() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "m", 100, 80, 0);
    p.take();
    let before = srv.scene().windows()[0].position();
    let [_, max, _] = buttons(before, 100);
    click_at(&mut srv, 100, max.0, max.1);
    p.read(&mut srv);
    let e = p.take();
    let (cw, ch, states) = configure(&e);
    assert_eq!((cw, ch), (640 - 2 * BORDER, 480 - TITLE_HEIGHT - 2 * BORDER));
    assert!(states.contains(&xdg_toplevel::State::MAXIMIZED.0));
    ack_and_commit(&mut p, &mut srv, &g, &w, &e);
    let win = &srv.scene().windows()[0];
    assert_eq!(win.position(), (BORDER, TITLE_HEIGHT + BORDER));
    assert!(win.is_maximized(), "the frame shows the restore button");
    let info = &srv.managed_windows()[0];
    assert!(info.maximized && info.active && !info.minimized && !info.internal);
    // The same button restores: the old size is suggested, the old place
    // taken again.
    let [_, max, _] = buttons(win.position(), 638);
    click_at(&mut srv, 200, max.0, max.1);
    p.read(&mut srv);
    let e = p.take();
    let (cw, ch, states) = configure(&e);
    assert_eq!((cw, ch), (100, 80));
    assert!(!states.contains(&xdg_toplevel::State::MAXIMIZED.0));
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (b, _) = buffer(&mut p, &mut srv, &g, 100, 80, 0, 1);
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let win = &srv.scene().windows()[0];
    assert_eq!(win.position(), before);
    assert!(!win.is_maximized());
}

#[test]
fn a_double_click_on_the_title_maximizes() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let _w = mapped(&mut p, &mut srv, &g, "d", 100, 80, 0);
    p.take();
    let (x, y) = srv.scene().windows()[0].position();
    // Two clicks too far apart in time: nothing.
    click_at(&mut srv, 1000, x + 10, y - 10);
    click_at(&mut srv, 1000 + DOUBLE_CLICK_MS + 50, x + 10, y - 10);
    p.read(&mut srv);
    assert!(!p.take().iter().any(|e| e.name() == ("xdg_toplevel", "configure")));
    click_at(&mut srv, 3000, x + 10, y - 10);
    click_at(&mut srv, 3200, x + 10, y - 10);
    p.read(&mut srv);
    let (_, _, states) = configure(&p.take());
    assert!(states.contains(&xdg_toplevel::State::MAXIMIZED.0));
}

#[test]
fn tiling_sends_the_size_and_the_tiled_edges() {
    let (mut srv, shared) = server();
    srv.scene_mut().set_reserved_top(24);
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "t", 100, 80, 0);
    p.take();
    let win = srv.scene().windows()[0].id();
    // The left half of the work area (y 24..480).
    let left = Rect::new(0, 24, 320, 456);
    srv.tile_window(win, left);
    p.read(&mut srv);
    let e = p.take();
    let (cw, ch, states) = configure(&e);
    assert_eq!((cw, ch), (320 - 2, 456 - 24 - 2));
    for s in [xdg_toplevel::State::TILED_LEFT, xdg_toplevel::State::TILED_TOP, xdg_toplevel::State::TILED_BOTTOM] {
        assert!(states.contains(&s.0), "{s:?}");
    }
    assert!(!states.contains(&xdg_toplevel::State::TILED_RIGHT.0));
    ack_and_commit(&mut p, &mut srv, &g, &w, &e);
    let win = &srv.scene().windows()[0];
    assert_eq!(win.position(), (1, 24 + 25));
    assert_eq!(srv.scene().frame_rect(win.id()), Some(left));
    assert_eq!(srv.managed_windows()[0].tiled, Some(left));
}

#[test]
fn minimizing_hides_and_moves_the_focus() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let _a = mapped(&mut p, &mut srv, &g, "a", 100, 80, 0);
    let _b = mapped(&mut p, &mut srv, &g, "b", 100, 80, 0);
    p.take();
    let (wa, wb) = (srv.scene().windows()[0].id(), srv.scene().windows()[1].id());
    assert_eq!(srv.active_window(), Some(wb));
    let pos = srv.scene().window(wb).unwrap().position();
    let [_, _, min] = buttons(pos, 100);
    click_at(&mut srv, 10, min.0, min.1);
    assert!(srv.scene().window(wb).unwrap().is_hidden());
    assert_eq!(srv.active_window(), Some(wa), "the focus moved on");
    // Bottom to top: the minimized one where it was, the focused one raised.
    let infos = srv.managed_windows();
    assert_eq!(infos.iter().map(|i| (i.title.as_str(), i.minimized)).collect::<Vec<_>>(), [("b", true), ("a", false)]);
    // A click where it was hits what is below.
    assert_ne!(srv.scene().hit_test(min.0, min.1), Hit::Minimize(wb));
    // Activating shows it again, on top.
    srv.activate_window(wb);
    assert!(!srv.scene().window(wb).unwrap().is_hidden());
    assert_eq!(srv.active_window(), Some(wb));
    assert_eq!(srv.scene().windows().last().unwrap().id(), wb);
    // The client may minimize itself too.
    let w = srv.managed_windows();
    assert!(w.iter().all(|i| !i.minimized));
}

#[test]
fn closing_the_active_window_focuses_the_next() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let _a = mapped(&mut p, &mut srv, &g, "a", 100, 80, 0);
    let b = mapped(&mut p, &mut srv, &g, "b", 100, 80, 0);
    let wa = srv.scene().windows()[0].id();
    p.send(&mut srv, b.surface, wl_surface::Request::Attach { buffer: None, x: 0, y: 0 });
    p.send(&mut srv, b.surface, wl_surface::Request::Commit);
    assert_eq!(srv.active_window(), Some(wa));
}

#[test]
fn dragging_a_maximized_window_restores_it() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "m", 100, 80, 0);
    p.take();
    let win = srv.scene().windows()[0].id();
    srv.set_maximized(win, true);
    p.read(&mut srv);
    let e = p.take();
    ack_and_commit(&mut p, &mut srv, &g, &w, &e);
    p.take();
    // Press on the title bar and move a little: still maximized.
    motion(&mut srv, 300, 12);
    srv.handle_input(InputEvent::PointerButton { time: 50, button: BTN_LEFT, pressed: true });
    motion(&mut srv, 303, 13);
    p.read(&mut srv);
    assert!(!p.take().iter().any(|e| e.name() == ("xdg_toplevel", "configure")));
    // Further: restored to its old size, under the pointer.
    motion(&mut srv, 330, 40);
    p.read(&mut srv);
    let e = p.take();
    let (cw, ch, states) = configure(&e);
    assert_eq!((cw, ch), (100, 80));
    assert!(!states.contains(&xdg_toplevel::State::MAXIMIZED.0));
    srv.handle_input(InputEvent::PointerButton { time: 60, button: BTN_LEFT, pressed: false });
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (b, _) = buffer(&mut p, &mut srv, &g, 100, 80, 0, 1);
    p.send(&mut srv, w.surface, wl_surface::Request::Attach { buffer: Some(b), x: 0, y: 0 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    let window = srv.scene().window(win).unwrap();
    let b = window.bounds();
    assert!(!window.is_maximized());
    // The pointer was at 300 of 638 across; it stays about that share
    // across the 100 wide window.
    assert!((b.x..b.right()).contains(&300), "{b:?}");
}

#[test]
fn a_new_keymap_reaches_every_keyboard() {
    let (mut srv, shared) = server();
    srv.set_seat_capabilities(Capabilities::KEYBOARD);
    let (mut p, registry) = connect(&mut srv, &shared);
    let seat = p.bind(&mut srv, registry, &wl_seat::INTERFACE, 7);
    let keyboard = p.new_id(&wl_keyboard::INTERFACE);
    p.send(&mut srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    p.take();
    srv.keymap_changed();
    p.read(&mut srv);
    let e = p.take();
    assert_eq!(e.iter().filter(|e| e.name() == ("wl_keyboard", "keymap")).count(), 1);
    assert_eq!(e[0].object, keyboard);
}

#[test]
fn internal_windows_get_events_and_frames() {
    let (mut srv, shared) = server();
    srv.set_seat_capabilities(Capabilities::POINTER.union(Capabilities::KEYBOARD));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let seat = p.bind(&mut srv, registry, &wl_seat::INTERFACE, 7);
    let keyboard = p.new_id(&wl_keyboard::INTERFACE);
    p.send(&mut srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    let a = mapped(&mut p, &mut srv, &g, "a", 100, 80, 0);
    p.take();
    assert_eq!(srv.keyboard_focus(), Some((p.id, a.surface)));
    let s = srv.add_internal_window("Settings", "poly.settings", 200, 100);
    assert!(srv.is_internal(s));
    assert_eq!(srv.active_window(), Some(s));
    assert_eq!(srv.keyboard_focus(), None, "the client's keyboard left");
    p.read(&mut srv);
    let n = p.names();
    assert!(n.contains(&("wl_keyboard", "leave")));
    assert!(n.contains(&("xdg_toplevel", "configure")), "the toplevel hears it is not active");
    let info = srv.managed_windows().into_iter().find(|i| i.window == s).unwrap();
    assert!(info.internal && info.active && info.app_id == "poly.settings" && info.title == "Settings");
    srv.set_internal_pixels(s, 200, 100, &[0x12_3456; 200 * 100], None);
    let (x, y) = srv.scene().window(s).unwrap().position();
    // A click inside: events with local coordinates; the release too.
    click_at(&mut srv, 5, x + 20, y + 30);
    let e = srv.take_internal_events();
    assert!(e.contains(&InternalEvent::Motion { window: s, x: 20, y: 30 }));
    assert!(e.contains(&InternalEvent::Button { window: s, x: 20, y: 30, button: BTN_LEFT, pressed: true }));
    assert!(e.contains(&InternalEvent::Button { window: s, x: 20, y: 30, button: BTN_LEFT, pressed: false }));
    // Keys go to it, not to the client.
    srv.handle_input(InputEvent::Key { time: 1, key: 30, pressed: true });
    assert_eq!(srv.take_internal_events(), [InternalEvent::Key { window: s, key: 30, pressed: true }]);
    p.read(&mut srv);
    assert!(!p.names().contains(&("wl_keyboard", "key")));
    // Maximize: the owner is told the new size.
    srv.set_maximized(s, true);
    assert_eq!(srv.take_internal_events(), [InternalEvent::Resize { window: s, width: 638, height: 454 }]);
    assert_eq!(srv.scene().window(s).unwrap().position(), (1, 25));
    srv.restore_window(s);
    assert_eq!(srv.take_internal_events(), [InternalEvent::Resize { window: s, width: 200, height: 100 }]);
    assert_eq!(srv.scene().window(s).unwrap().position(), (x, y));
    // The close button asks the owner.
    let [close, _, _] = buttons((x, y), 200);
    click_at(&mut srv, 9, close.0, close.1);
    assert!(srv.take_internal_events().contains(&InternalEvent::Close(s)));
    // Clicking the client's window gives the focus back.
    srv.remove_internal_window(s);
    assert_eq!(srv.keyboard_focus(), Some((p.id, a.surface)));
    assert!(srv.managed_windows().iter().all(|i| !i.internal));
}
