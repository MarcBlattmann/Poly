//! Input routing (ADR 0107): the pointer, the keyboard and moving windows,
//! driven through `Server::handle_input`.

use super::*;
use crate::compose::{BORDER, Placement, TITLE_HEIGHT};

/// A client with a pointer and a keyboard on a seat that has both.
fn seat_client(srv: &mut Server, shared: &Rc<RefCell<Shared>>) -> (Peer, Globals, ObjectId, ObjectId) {
    srv.set_seat_capabilities(Capabilities::POINTER.union(Capabilities::KEYBOARD));
    let (mut p, registry) = connect(srv, shared);
    let g = globals(&mut p, srv, registry);
    let seat = p.bind(srv, registry, &wl_seat::INTERFACE, 7);
    let pointer = p.new_id(&wl_pointer::INTERFACE);
    p.send(srv, seat, wl_seat::Request::GetPointer { id: pointer });
    let keyboard = p.new_id(&wl_keyboard::INTERFACE);
    p.send(srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    p.take();
    (p, g, pointer, keyboard)
}

fn motion(srv: &mut Server, x: i32, y: i32) {
    srv.handle_input(InputEvent::PointerMotionAbsolute { time: 7, x: Fixed::from_int(x), y: Fixed::from_int(y) });
}

fn button(srv: &mut Server, pressed: bool) {
    srv.handle_input(InputEvent::PointerButton { time: 8, button: BTN_LEFT, pressed });
}

fn key(srv: &mut Server, key: u32, pressed: bool) {
    srv.handle_input(InputEvent::Key { time: 3, key, pressed });
}

fn names(e: &[Received]) -> Vec<(&'static str, &'static str)> {
    e.iter().map(Received::name).collect()
}

#[test]
fn a_new_keyboard_gets_the_keymap_and_repeat_rate() {
    let (mut srv, shared) = server();
    srv.set_seat_capabilities(Capabilities::KEYBOARD);
    let (mut p, registry) = connect(&mut srv, &shared);
    let seat = p.bind(&mut srv, registry, &wl_seat::INTERFACE, 7);
    p.take();
    let keyboard = p.new_id(&wl_keyboard::INTERFACE);
    p.send(&mut srv, seat, wl_seat::Request::GetKeyboard { id: keyboard });
    let e = p.take();
    assert_eq!(names(&e), [("wl_keyboard", "keymap"), ("wl_keyboard", "repeat_info")]);
    assert_eq!(e[0].args, [Arg::Uint(1), Arg::Fd(Fd(KEYMAP_FD)), Arg::Uint(1234)]);
    assert_eq!(e[1].args, [Arg::Int(30), Arg::Int(250)]);
}

#[test]
fn the_pointer_enters_moves_clicks_and_leaves() {
    let (mut srv, shared) = server();
    let (mut p, g, pointer, _keyboard) = seat_client(&mut srv, &shared);
    let w = mapped(&mut p, &mut srv, &g, "w", 100, 80, 0x00ff_0000);
    p.take();
    let (wx, wy) = srv.scene().windows()[0].position();
    motion(&mut srv, wx + 10, wy + 20);
    p.read(&mut srv);
    let e = p.take();
    assert_eq!(names(&e), [("wl_pointer", "enter"), ("wl_pointer", "frame")]);
    assert_eq!(
        &e[0].args[1..],
        [Arg::Object(Some(w.surface)), Arg::Fixed(Fixed::from_int(10)), Arg::Fixed(Fixed::from_int(20))]
    );
    assert_eq!(e[0].object, pointer);
    assert_eq!(srv.pointer_focus(), Some((p.id, w.surface)));
    motion(&mut srv, wx + 11, wy + 20);
    p.read(&mut srv);
    let e = p.take();
    assert_eq!(e[0].name(), ("wl_pointer", "motion"));
    assert_eq!(e[0].args, [Arg::Uint(7), Arg::Fixed(Fixed::from_int(11)), Arg::Fixed(Fixed::from_int(20))]);
    // A press goes to the client; dragging out keeps the focus (implicit
    // grab) until the release.
    button(&mut srv, true);
    p.read(&mut srv);
    let e = p.take();
    let b = e.iter().find(|e| e.name() == ("wl_pointer", "button")).expect("a button event");
    assert_eq!(&b.args[1..], [Arg::Uint(8), Arg::Uint(BTN_LEFT), Arg::Uint(1)]);
    motion(&mut srv, 600, 450);
    p.read(&mut srv);
    assert_eq!(p.names(), [("wl_pointer", "motion"), ("wl_pointer", "frame")]);
    button(&mut srv, false);
    p.read(&mut srv);
    let n = p.names();
    assert_eq!(n[0], ("wl_pointer", "button"));
    assert!(n.contains(&("wl_pointer", "leave")));
    assert_eq!(srv.pointer_focus(), None);
    // The wheel: one step up.
    motion(&mut srv, wx + 5, wy + 5);
    p.read(&mut srv);
    p.take();
    srv.handle_input(InputEvent::PointerAxis { time: 9, horizontal: false, value: Fixed::from_int(-15) });
    p.read(&mut srv);
    let e = p.take();
    assert_eq!(
        names(&e),
        [
            ("wl_pointer", "axis_source"),
            ("wl_pointer", "axis_discrete"),
            ("wl_pointer", "axis"),
            ("wl_pointer", "frame")
        ]
    );
    assert_eq!(e[1].args, [Arg::Uint(0), Arg::Int(-1)]);
}

#[test]
fn the_keyboard_follows_the_active_window() {
    let (mut srv, shared) = server();
    let (mut p, g, _pointer, keyboard) = seat_client(&mut srv, &shared);
    let a = mapped(&mut p, &mut srv, &g, "a", 100, 80, 0);
    let e = p.take();
    let enter = e.iter().find(|e| e.name() == ("wl_keyboard", "enter")).expect("enter when mapped");
    assert_eq!((enter.object, &enter.args[1]), (keyboard, &Arg::Object(Some(a.surface))));
    assert!(e.iter().any(|e| e.name() == ("wl_keyboard", "modifiers")));
    // Shift down, A (and the device's repeat of it, not passed on), up.
    key(&mut srv, 42, true);
    key(&mut srv, 30, true);
    key(&mut srv, 30, true);
    key(&mut srv, 30, false);
    key(&mut srv, 42, false);
    p.read(&mut srv);
    let e = p.take();
    assert_eq!(
        names(&e),
        [
            ("wl_keyboard", "key"),
            ("wl_keyboard", "modifiers"),
            ("wl_keyboard", "key"),
            ("wl_keyboard", "key"),
            ("wl_keyboard", "key"),
            ("wl_keyboard", "modifiers"),
        ]
    );
    assert_eq!(&e[1].args[1..], [Arg::Uint(1), Arg::Uint(0), Arg::Uint(0), Arg::Uint(0)], "Shift depressed");
    assert_eq!(&e[2].args[1..], [Arg::Uint(3), Arg::Uint(30), Arg::Uint(1)]);
    // Caps Lock locks.
    key(&mut srv, 58, true);
    key(&mut srv, 58, false);
    assert_eq!(srv.modifiers(), (0, 2));
    // A second window takes the keyboard; the first hears leave first.
    let b = mapped(&mut p, &mut srv, &g, "b", 100, 80, 0);
    let e = p.take();
    let leave = e.iter().position(|e| e.name() == ("wl_keyboard", "leave")).expect("leave");
    let enter = e.iter().position(|e| e.name() == ("wl_keyboard", "enter")).expect("enter");
    assert!(leave < enter);
    assert_eq!(e[enter].args[1], Arg::Object(Some(b.surface)));
    // The compositor takes the keyboard (its launcher) and gives it back.
    srv.set_keyboard_suspended(true);
    p.read(&mut srv);
    assert_eq!(p.names(), [("wl_keyboard", "leave")]);
    key(&mut srv, 30, true);
    p.read(&mut srv);
    assert!(p.take().is_empty(), "no keys while suspended");
    key(&mut srv, 30, false);
    srv.set_keyboard_suspended(false);
    p.read(&mut srv);
    assert_eq!(p.names()[0], ("wl_keyboard", "enter"));
}

#[test]
fn dragging_the_title_bar_moves_the_window() {
    let (mut srv, shared) = server();
    let (mut p, g, pointer, _) = seat_client(&mut srv, &shared);
    srv.scene_mut().set_reserved_top(24);
    let w = mapped(&mut p, &mut srv, &g, "w", 100, 80, 0);
    let id = srv.scene().windows()[0].id();
    let (wx, wy) = srv.scene().windows()[0].position();
    assert!(wy - TITLE_HEIGHT - BORDER >= 24, "placed below the reserved rows");
    let frame = srv.scene().frame_rect(id).unwrap();
    motion(&mut srv, frame.x + 20, frame.y + 5);
    button(&mut srv, true);
    assert!(srv.is_moving());
    motion(&mut srv, frame.x + 70, frame.y + 45);
    assert_eq!(srv.scene().windows()[0].position(), (wx + 50, wy + 40));
    // Never above the reserved rows.
    motion(&mut srv, frame.x + 70, 0);
    assert_eq!(srv.scene().frame_rect(id).unwrap().y, 24);
    button(&mut srv, false);
    assert!(!srv.is_moving());
    // xdg_toplevel.move while a button is held does the same.
    let (x, y) = srv.scene().windows()[0].position();
    motion(&mut srv, x + 5, y + 5);
    button(&mut srv, true);
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::Move { seat: ObjectId(pointer.0 - 1), serial: 0 });
    assert!(srv.is_moving());
    motion(&mut srv, x + 15, y + 5);
    assert_eq!(srv.scene().windows()[0].position(), (x + 10, y));
    button(&mut srv, false);
}

#[test]
fn centred_placement_and_the_cursor() {
    let (mut srv, shared) = server();
    srv.scene_mut().set_placement(Placement::Centered);
    srv.scene_mut().set_reserved_top(24);
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    mapped(&mut p, &mut srv, &g, "c", 200, 100, 0);
    let frame = srv.scene().frame_rect(srv.scene().windows()[0].id()).unwrap();
    // 640x480 less 24 rows; the frame is 202 x 126.
    assert_eq!((frame.x, frame.y), ((640 - 202) / 2, 24 + (456 - 126) / 2));
    srv.set_cursor_visible(true);
    motion(&mut srv, 300, 20);
    assert_eq!(srv.scene().cursor(), Some((300, 20)));
    let mut fb = MemoryFramebuffer::new(640, 480);
    srv.render(&mut fb);
    assert_eq!(fb.pixel(300, 20), Some(0), "the arrow's outline at the hot spot");
    assert_eq!(fb.pixel(301, 22), Some(0xff_ffff), "its fill");
}
