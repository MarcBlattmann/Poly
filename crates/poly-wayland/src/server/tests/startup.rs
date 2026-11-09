//! Startup feedback (ADR 0167): the busy pointer while a launched app has
//! no window, ended by its activation token, a window of its process, a
//! failure or the timeout; a client's own cursor wins.

use super::*;
use crate::compose::{BUSY_SIZE, CursorLook};
use crate::launch::{EXIT_GRACE_MS, Ended, Event, TIMEOUT_MS};
use crate::server::startup::BUSY_STEP_MS;

fn motion(srv: &mut Server, x: i32, y: i32) {
    srv.handle_input(InputEvent::PointerMotionAbsolute { time: 7, x: Fixed::from_int(x), y: Fixed::from_int(y) });
}

/// A pointer over the empty desktop, cursor shown.
fn desktop(srv: &mut Server) {
    srv.set_cursor_visible(true);
    motion(srv, 600, 400);
}

fn busy(srv: &Server) -> bool {
    matches!(srv.scene().cursor_look(), CursorLook::Busy(_))
}

/// A parent for every pid: 1000 + n is a child of 1000 + n - 1, down to 1000.
fn chain(pid: u32) -> Option<u32> {
    (pid > 1000).then(|| pid - 1)
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

/// The whole picture.
fn render_all(srv: &mut Server) -> MemoryFramebuffer {
    let (w, h, _) = srv.scene().output();
    let mut fb = MemoryFramebuffer::new(w, h);
    srv.scene_mut().damage_all();
    srv.render(&mut fb);
    fb
}

fn activate_with(p: &mut Peer, srv: &mut Server, activation: ObjectId, token: &str, surface: ObjectId) {
    p.send(srv, activation, xdg_activation_v1::Request::Activate { token: token.into(), surface });
}

#[test]
fn the_pointer_is_busy_from_the_launch_until_the_token_is_used() {
    let (mut srv, shared) = server();
    desktop(&mut srv);
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Arrow);
    srv.launch_clock(1_000);
    let (_, token) = srv.begin_launch("Files");
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Busy((1_000 / BUSY_STEP_MS % 16) as u8));
    assert!(srv.launch_busy());
    // The app's client activates a surface with the token it found in its environment.
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let activation = p.bind(&mut srv, registry, &xdg_activation_v1::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "files", 20, 20, 0);
    // A token nobody made does nothing.
    activate_with(&mut p, &mut srv, activation, "poly-activation-0-999", w.surface);
    assert!(busy(&srv));
    activate_with(&mut p, &mut srv, activation, &token, w.surface);
    assert!(!busy(&srv));
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Arrow);
    let lines: Vec<String> = srv.take_launch_events().iter().flat_map(Event::describe).collect();
    assert_eq!(lines.first().map(String::as_str), Some("busy pointer on: launching Files (1 pending)"));
    assert_eq!(lines.last().map(String::as_str), Some("busy pointer off"));
}

#[test]
fn the_spinner_turns_with_the_clock() {
    let (mut srv, _shared) = server();
    desktop(&mut srv);
    srv.launch_clock(0);
    srv.begin_launch("A");
    let mut seen = Vec::new();
    for ms in (0..1_000).step_by(BUSY_STEP_MS as usize) {
        srv.launch_clock(ms);
        let CursorLook::Busy(phase) = *srv.scene().cursor_look() else { panic!("busy") };
        seen.push(phase);
    }
    assert_eq!(&seen[..4], &[0, 1, 2, 3]);
    assert!(seen.iter().all(|&p| p < 16));
    // A turn is 16 pictures: the next turn starts over.
    srv.launch_clock(16 * BUSY_STEP_MS);
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Busy(0));
    // The next picture is due when the step ends; with nothing else it is
    // the server's next wake.
    srv.launch_clock(16 * BUSY_STEP_MS + 20);
    assert_eq!(srv.launch_wait_ms(), Some(BUSY_STEP_MS - 20));
}

#[test]
fn a_new_picture_damages_only_the_pointer() {
    let (mut srv, _shared) = server();
    desktop(&mut srv);
    srv.launch_clock(0);
    srv.begin_launch("A");
    let (w, h, _) = srv.scene().output();
    let mut fb = MemoryFramebuffer::new(w, h);
    srv.render(&mut fb);
    srv.launch_clock(BUSY_STEP_MS);
    let damage = srv.render(&mut fb);
    assert!(!damage.is_empty());
    let cursor = srv.scene().cursor_rect(600, 400);
    assert_eq!((cursor.width, cursor.height), BUSY_SIZE);
    for r in &damage {
        assert!(r.x >= cursor.x && r.y >= cursor.y && r.right() <= cursor.right() && r.bottom() <= cursor.bottom());
    }
    // Same picture, nothing to draw.
    srv.launch_clock(BUSY_STEP_MS + 5);
    assert!(srv.render(&mut fb).is_empty());
}

#[test]
fn the_busy_pointer_is_drawn_with_a_spinner() {
    let (mut srv, _shared) = server();
    desktop(&mut srv);
    srv.launch_clock(0);
    let before = render_all(&mut srv);
    srv.begin_launch("A");
    let fb = render_all(&mut srv);
    // The arrow's tip is the same; the ring at its lower right is new.
    assert_eq!(fb.pixel(600, 400), before.pixel(600, 400));
    // Phase 0: the arc is on the ring's top, bright over the desktop.
    let top = fb.pixel(600 + 17, 400 + 13).unwrap();
    let bg = before.pixel(600 + 17, 400 + 13).unwrap();
    assert_ne!(top, bg);
    assert!(top > bg, "{top:#x} over {bg:#x}");
    // Nothing outside the pointer's rectangle changed.
    assert_eq!(fb.pixel(600 + BUSY_SIZE.0 as u32 + 2, 400), before.pixel(600 + BUSY_SIZE.0 as u32 + 2, 400));
    assert_eq!(fb.pixel(600, 400 + BUSY_SIZE.1 as u32 + 2), before.pixel(600, 400 + BUSY_SIZE.1 as u32 + 2));
}

#[test]
fn a_clients_own_cursor_wins_over_its_window_and_the_desktop_is_busy_again_after() {
    let (mut srv, shared) = server();
    let (mut p, registry, g, pointer) = pointer_client(&mut srv, &shared);
    let manager = p.bind(&mut srv, registry, &wp_cursor_shape_manager_v1::INTERFACE, 1);
    srv.set_cursor_visible(true);
    srv.launch_clock(100);
    srv.begin_launch("Slow");
    let w = mapped(&mut p, &mut srv, &g, "c", 100, 80, 0);
    let (x, y) = srv.scene().windows()[0].position();
    // Over the client, which has not set a cursor yet: still the desktop's.
    motion(&mut srv, x + 10, y + 10);
    assert!(busy(&srv));
    // It sets a cursor surface: that wins.
    let cursor = surface(&mut p, &mut srv, &g);
    p.send(
        &mut srv,
        pointer,
        wl_pointer::Request::SetCursor { serial: 1, surface: Some(cursor), hotspot_x: 0, hotspot_y: 0 },
    );
    let (b, _) = buffer(&mut p, &mut srv, &g, 6, 6, 0xff12_3456, 0);
    attach(&mut p, &mut srv, cursor, b);
    assert!(matches!(srv.scene().cursor_look(), CursorLook::Image { .. }));
    srv.launch_clock(500);
    assert!(matches!(srv.scene().cursor_look(), CursorLook::Image { .. }));
    // Hidden wins too.
    p.send(&mut srv, pointer, wl_pointer::Request::SetCursor { serial: 2, surface: None, hotspot_x: 0, hotspot_y: 0 });
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Hidden);
    // So does a named shape.
    let device = p.new_id(&wp_cursor_shape_device_v1::INTERFACE);
    p.send(&mut srv, manager, wp_cursor_shape_manager_v1::Request::GetPointer { cursor_shape_device: device, pointer });
    p.send(&mut srv, device, wp_cursor_shape_device_v1::Request::SetShape { serial: 3, shape: 1 });
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Arrow);
    srv.launch_clock(900);
    assert_eq!(*srv.scene().cursor_look(), CursorLook::Arrow);
    // Back on the desktop: busy.
    motion(&mut srv, 630, 470);
    assert!(busy(&srv));
    let _ = w;
}

#[test]
fn the_first_window_of_the_process_or_its_children_ends_the_wait() {
    let (mut srv, shared) = server();
    desktop(&mut srv);
    srv.set_pid_parent(chain);
    srv.launch_clock(10);
    let (id, _token) = srv.begin_launch("Browser");
    srv.launch_pid(id, 1000);
    // A client of an unrelated process maps a window: the wait goes on.
    let (mut other, registry) = connect(&mut srv, &shared);
    srv.set_client_pid(other.id, 4000);
    let g = globals(&mut other, &mut srv, registry);
    mapped(&mut other, &mut srv, &g, "unrelated", 20, 20, 0);
    assert!(busy(&srv));
    // A client with no known process neither.
    let (mut anon, registry) = connect(&mut srv, &shared);
    let g = globals(&mut anon, &mut srv, registry);
    mapped(&mut anon, &mut srv, &g, "anonymous", 20, 20, 0);
    assert!(busy(&srv));
    // Its grandchild's first toplevel.
    let (mut kid, registry) = connect(&mut srv, &shared);
    srv.set_client_pid(kid.id, 1002);
    let g = globals(&mut kid, &mut srv, registry);
    // Not mapped yet: a toplevel without content is not a window.
    let (_, _, _, _) = toplevel(&mut kid, &mut srv, &g, "kid");
    assert!(busy(&srv));
    mapped(&mut kid, &mut srv, &g, "kid", 20, 20, 0);
    assert!(!busy(&srv));
    let events = srv.take_launch_events();
    let Some(Event::Finished { how, name, .. }) = events.last() else { panic!("finished: {events:?}") };
    assert_eq!((how, name.as_str()), (&Ended::Window { pid: 1002 }, "Browser"));
}

#[test]
fn x_and_windows_windows_report_their_process() {
    let (mut srv, _shared) = server();
    desktop(&mut srv);
    srv.set_pid_parent(chain);
    let (a, _) = srv.begin_launch("XEyes");
    let (b, _) = srv.begin_launch("Poly Fill");
    srv.launch_pid(a, 2000);
    srv.launch_pid(b, 3000);
    assert_eq!(srv.launches_pending(), 2);
    srv.launch_window_mapped(2001);
    assert_eq!(srv.launches_pending(), 1);
    assert!(busy(&srv));
    srv.launch_window_mapped(3000);
    assert!(!busy(&srv));
}

#[test]
fn several_launches_keep_the_pointer_busy_until_all_are_done() {
    let (mut srv, shared) = server();
    desktop(&mut srv);
    srv.launch_clock(0);
    let (_, t1) = srv.begin_launch("One");
    srv.launch_clock(1_000);
    let (id2, _t2) = srv.begin_launch("Two");
    srv.launch_pid(id2, 77);
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let activation = p.bind(&mut srv, registry, &xdg_activation_v1::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "one", 20, 20, 0);
    activate_with(&mut p, &mut srv, activation, &t1, w.surface);
    assert_eq!(srv.launches_pending(), 1);
    assert!(busy(&srv));
    srv.launch_window_mapped(77);
    assert_eq!(srv.launches_pending(), 0);
    assert!(!busy(&srv));
}

#[test]
fn a_launch_without_a_window_stops_after_thirty_seconds() {
    let (mut srv, _shared) = server();
    desktop(&mut srv);
    srv.launch_clock(5_000);
    srv.begin_launch("Never");
    srv.launch_clock(5_000 + TIMEOUT_MS - 1);
    assert!(busy(&srv));
    // The server asks to be woken for the end, not later.
    assert_eq!(srv.launch_wait_ms(), Some(1));
    srv.launch_clock(5_000 + TIMEOUT_MS);
    assert!(!busy(&srv));
    assert_eq!(srv.launch_wait_ms(), None);
    let lines: Vec<String> = srv.take_launch_events().iter().flat_map(Event::describe).collect();
    assert_eq!(lines[1], "launch Never: timed out after 30000 ms, no window (0 pending)");
    assert_eq!(lines[2], "busy pointer off");
}

#[test]
fn a_cancelled_or_failed_launch_ends_it() {
    let (mut srv, _shared) = server();
    desktop(&mut srv);
    let (a, _) = srv.begin_launch("Missing");
    srv.cancel_launch(a);
    assert!(!busy(&srv));
    let (b, _) = srv.begin_launch("Crashes");
    srv.launch_pid(b, 55);
    // A clean exit keeps waiting a moment (it may have handed over to a
    // running instance); a failure does not.
    srv.launch_exited(55, 139);
    assert!(!busy(&srv));
    let (c, _) = srv.begin_launch("Notifies");
    srv.launch_pid(c, 56);
    srv.launch_clock(10_000);
    srv.launch_exited(56, 0);
    srv.launch_clock(10_000 + EXIT_GRACE_MS - 1);
    assert!(busy(&srv));
    srv.launch_clock(10_000 + EXIT_GRACE_MS);
    assert!(!busy(&srv));
}

#[test]
fn a_program_that_starts_apps_adopts_its_own_token() {
    let (mut srv, shared) = server();
    desktop(&mut srv);
    let id = srv.adopt_launch("Text Editor", "poly-launch-31-1");
    // Said twice, it is one launch.
    assert_eq!(srv.adopt_launch("Text Editor", "poly-launch-31-1"), id);
    assert_eq!(srv.launches_pending(), 1);
    srv.launch_pid_by_token("poly-launch-31-1", 99);
    assert!(busy(&srv));
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let activation = p.bind(&mut srv, registry, &xdg_activation_v1::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "editor", 20, 20, 0);
    activate_with(&mut p, &mut srv, activation, "poly-launch-31-1", w.surface);
    assert!(!busy(&srv));
    // Cancelling something that already ended does nothing.
    srv.cancel_launch_token("poly-launch-31-1");
    assert_eq!(srv.launches_pending(), 0);
}

#[test]
fn the_token_works_once_even_if_the_window_had_no_pid() {
    // The app's token is its way in when the process cannot be matched
    // (a launcher script that handed over to another process).
    let (mut srv, shared) = server();
    desktop(&mut srv);
    let (_, token) = srv.begin_launch("Wrapped");
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let activation = p.bind(&mut srv, registry, &xdg_activation_v1::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "w", 20, 20, 0);
    assert!(busy(&srv));
    activate_with(&mut p, &mut srv, activation, &token, w.surface);
    assert!(!busy(&srv));
    // A second launch is not ended by the first one's token.
    let (_, second) = srv.begin_launch("Second");
    activate_with(&mut p, &mut srv, activation, &token, w.surface);
    assert!(busy(&srv));
    activate_with(&mut p, &mut srv, activation, &second, w.surface);
    assert!(!busy(&srv));
}
