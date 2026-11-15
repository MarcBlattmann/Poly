//! Who draws a window's frame (ADR 0183): KDE's server decoration protocol,
//! which GTK 3 and 4 use, and the shadow margins of a client-drawn frame. The
//! decision itself is in `crate::frame_policy`; this is the protocol side.

use super::*;
use crate::protocol::{org_kde_kwin_server_decoration, org_kde_kwin_server_decoration_manager};

const KDE_SERVER: u32 = 2;
const KDE_CLIENT: u32 = 1;

fn decorated(srv: &Server) -> bool {
    srv.scene().windows()[0].is_decorated()
}

/// The `mode` events among `events`.
fn modes(events: &[Received]) -> Vec<u32> {
    events
        .iter()
        .filter(|e| e.name() == ("org_kde_kwin_server_decoration", "mode"))
        .map(|e| match e.args[0] {
            Arg::Uint(m) => m,
            _ => panic!(),
        })
        .collect()
}

/// A window of a client that bound KDE's manager, with an object that asked
/// for `mode` (if any), and the object.
fn kde_window(
    p: &mut Peer,
    srv: &mut Server,
    g: &Globals,
    registry: ObjectId,
    mode: Option<u32>,
) -> (Window, ObjectId) {
    let manager = p.bind(srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let w = mapped(p, srv, g, "kde", 50, 50, 0);
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    p.send(srv, manager, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface: w.surface });
    if let Some(mode) = mode {
        p.send(srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode });
    }
    p.send(srv, w.surface, wl_surface::Request::Commit);
    (w, deco)
}

#[test]
fn the_manager_announces_the_server_mode_as_the_default() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    assert_eq!(p.globals["org_kde_kwin_server_decoration_manager"].1, 1);
    p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let e = p.take();
    let d: Vec<_> =
        e.iter().filter(|e| e.name() == ("org_kde_kwin_server_decoration_manager", "default_mode")).collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].args, [Arg::Uint(KDE_SERVER)]);
}

#[test]
fn a_window_whose_client_never_binds_the_manager_is_framed() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    mapped(&mut p, &mut srv, &g, "plain", 50, 50, 0);
    assert!(decorated(&srv));
}

#[test]
fn a_gtk3_header_bar_window_makes_no_object_and_gets_no_frame() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    mapped(&mut p, &mut srv, &g, "headerbar", 50, 50, 0);
    assert!(!decorated(&srv), "it draws its own title bar");
}

#[test]
fn an_object_gets_the_default_mode_until_it_asks_for_another() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let (w, deco) = kde_window(&mut p, &mut srv, &g, registry, None);
    // The mode in use is told at once.
    assert!(decorated(&srv), "the default is Poly's frame");
    // GTK 3 asks for the mode in use right after creating the object: no
    // second event.
    p.take();
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    assert!(modes(&p.take()).is_empty());
    // The client's frame when asked for, Poly's again when asked for, and
    // releasing the object leaves the client's own (the manager is bound).
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_CLIENT });
    assert_eq!(modes(&p.take()), [KDE_CLIENT]);
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(!decorated(&srv));
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    assert_eq!(modes(&p.take()), [KDE_SERVER]);
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(decorated(&srv));
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::Release);
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(!decorated(&srv));
}

#[test]
fn creating_the_object_announces_the_mode_and_does_not_configure_a_floating_window() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "gtk3", 50, 50, 0);
    p.take();
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    p.send(&mut srv, manager, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface: w.surface });
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_CLIENT });
    let e = p.take();
    assert_eq!(modes(&e), [KDE_SERVER, KDE_CLIENT]);
    // Its frame changes at its next commit; it is not configured again
    // (GTK 3 does not take a configure of another size on top of the mode).
    assert!(e.iter().all(|e| e.name().1 != "configure"), "{e:?}");
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(!decorated(&srv));
}

#[test]
fn a_maximized_window_is_configured_again_when_the_frame_changes() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "max", 50, 50, 0);
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::SetMaximized);
    let e = p.take();
    // No object: the client's frame, so the whole work area.
    let c = e.iter().find(|e| e.name() == ("xdg_toplevel", "configure")).unwrap();
    assert_eq!(c.args[..2], [Arg::Int(640), Arg::Int(480)]);
    // The object (whose mode is the default, Poly's frame) changes the frame.
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    p.send(&mut srv, manager, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface: w.surface });
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    let e = p.take();
    let c = e.iter().find(|e| e.name() == ("xdg_toplevel", "configure")).expect("configured again");
    // Poly's frame takes its room.
    assert!(matches!((&c.args[0], &c.args[1]), (Arg::Int(w), Arg::Int(h)) if *w < 640 && *h < 480), "{c:?}");
}

#[test]
fn a_shadow_around_the_window_geometry_is_a_client_drawn_frame() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "shadowed", 200, 150, 0);
    assert!(decorated(&srv));
    // GTK 3's: the geometry is the window without its shadow.
    p.send(&mut srv, w.xdg, xdg_surface::Request::SetWindowGeometry { x: 26, y: 23, width: 148, height: 100 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(!decorated(&srv));
    // Maximized, the shadow is gone but the client still draws its title bar.
    p.send(&mut srv, w.xdg, xdg_surface::Request::SetWindowGeometry { x: 0, y: 0, width: 200, height: 150 });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(!decorated(&srv));
}

#[test]
fn a_protocol_s_word_outranks_a_shadow() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &zxdg_decoration_manager_v1::INTERFACE, 2);
    let w = mapped(&mut p, &mut srv, &g, "asks", 200, 150, 0);
    let deco = p.new_id(&zxdg_toplevel_decoration_v1::INTERFACE);
    p.send(
        &mut srv,
        manager,
        zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id: deco, toplevel: w.toplevel },
    );
    let server_side = zxdg_toplevel_decoration_v1::Mode::SERVER_SIDE.0;
    p.send(&mut srv, deco, zxdg_toplevel_decoration_v1::Request::SetMode { mode: server_side });
    p.send(&mut srv, w.xdg, xdg_surface::Request::SetWindowGeometry { x: 26, y: 23, width: 148, height: 100 });
    let e = p.take();
    p.send(&mut srv, w.xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(decorated(&srv), "it asked for Poly's frame");
}

#[test]
fn the_xdg_decoration_answer_follows_the_decision() {
    // A client that asks for nothing through xdg-decoration but makes a KDE
    // object is told what the KDE object decides.
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let xdg_manager = p.bind(&mut srv, registry, &zxdg_decoration_manager_v1::INTERFACE, 2);
    let kde = p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let w = mapped(&mut p, &mut srv, &g, "both", 50, 50, 0);
    let xdg_deco = p.new_id(&zxdg_toplevel_decoration_v1::INTERFACE);
    p.send(
        &mut srv,
        xdg_manager,
        zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id: xdg_deco, toplevel: w.toplevel },
    );
    p.take();
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    let last_mode = |e: &[Received]| e.iter().rev().find(|e| e.object == xdg_deco).map(|e| e.args.clone());
    p.send(&mut srv, kde, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface: w.surface });
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    let server_side = zxdg_toplevel_decoration_v1::Mode::SERVER_SIDE.0;
    assert_eq!(last_mode(&p.take()), Some(vec![Arg::Uint(server_side)]));
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_CLIENT });
    let client_side = zxdg_toplevel_decoration_v1::Mode::CLIENT_SIDE.0;
    assert_eq!(last_mode(&p.take()), Some(vec![Arg::Uint(client_side)]));
}

/// The `(width, height)` of every `xdg_toplevel` `configure` and
/// `configure_bounds` among `events`.
fn sizes(events: &[Received], event: &str) -> Vec<(i32, i32)> {
    events
        .iter()
        .filter(|e| e.name() == ("xdg_toplevel", event))
        .map(|e| match e.args[..2] {
            [Arg::Int(w), Arg::Int(h)] => (w, h),
            _ => panic!("{e:?}"),
        })
        .collect()
}

#[test]
fn gtk3_starting_with_the_server_default_is_told_the_mode_once_and_left_its_size() {
    // GTK 3.24's start as it shows with WAYLAND_DEBUG=1 on Poly: it binds
    // the manager (and hears the default before anything about a window),
    // makes its toplevel, commits, then makes the decoration object and asks
    // for `server`, then acks and draws its first buffer.
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let e = p.take();
    assert_eq!(
        e.iter().map(Received::name).collect::<Vec<_>>(),
        [("org_kde_kwin_server_decoration_manager", "default_mode")]
    );
    let (surface, xdg, toplevel, initial) = toplevel(&mut p, &mut srv, &g, "Poly GTK 3");
    // The client picks the size (0 x 0) within bounds that are positive.
    assert_eq!(sizes(&initial, "configure"), [(0, 0)]);
    let bounds = sizes(&initial, "configure_bounds");
    assert!(bounds.len() == 1 && bounds[0].0 > 0 && bounds[0].1 > 0, "{bounds:?}");
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    p.send(&mut srv, manager, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface });
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    let e = p.take();
    // One `mode` (the default, which it asked for again): no second one,
    // and no other configure on top.
    assert_eq!(modes(&e), [KDE_SERVER]);
    assert!(e.iter().all(|e| e.name().1 != "configure"), "{e:?}");
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&initial) });
    let (buffer, _mem) = buffer(&mut p, &mut srv, &g, 360, 240, 0x2a6fb0, wl_shm::Format::XRGB8888.0);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(&mut srv, toplevel, xdg_toplevel::Request::SetMinSize { width: 94, height: 32 });
    p.send(&mut srv, xdg, xdg_surface::Request::SetWindowGeometry { x: 0, y: 0, width: 360, height: 240 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    assert!(decorated(&srv), "Poly draws its frame");
    let e = p.take();
    assert!(e.iter().all(|e| e.name().1 != "configure"), "mapping it configures nothing: {e:?}");
    assert!(srv.failure(p.id).is_none());
}

#[test]
fn gtk4_making_the_object_before_its_toplevel_gets_the_frame_from_the_start() {
    // GTK 4.14 makes the decoration object and asks for `server` before
    // its xdg_surface: the mode is told then, the first configure is the
    // only one, and the window is framed when it maps.
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let manager = p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    p.take();
    let surface = p.new_id(&wl_surface::INTERFACE);
    p.send(&mut srv, g.compositor, wl_compositor::Request::CreateSurface { id: surface });
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    p.send(&mut srv, manager, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface });
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    assert_eq!(modes(&p.take()), [KDE_SERVER]);
    let xdg = p.new_id(&xdg_surface::INTERFACE);
    p.send(&mut srv, g.wm_base, xdg_wm_base::Request::GetXdgSurface { id: xdg, surface });
    let toplevel = p.new_id(&xdg_toplevel::INTERFACE);
    p.send(&mut srv, xdg, xdg_surface::Request::GetToplevel { id: toplevel });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    p.send(&mut srv, toplevel, xdg_toplevel::Request::UnsetMaximized);
    p.send(&mut srv, toplevel, xdg_toplevel::Request::UnsetFullscreen);
    let e = p.take();
    assert!(sizes(&e, "configure").iter().all(|s| *s == (0, 0)), "{e:?}");
    p.send(&mut srv, xdg, xdg_surface::Request::AckConfigure { serial: configure_serial(&e) });
    let (buffer, _mem) = buffer(&mut p, &mut srv, &g, 360, 240, 0x2a8a4a, wl_shm::Format::XRGB8888.0);
    p.send(&mut srv, surface, wl_surface::Request::Attach { buffer: Some(buffer), x: 0, y: 0 });
    p.send(&mut srv, surface, wl_surface::Request::Commit);
    assert!(decorated(&srv));
    assert!(srv.failure(p.id).is_none());
}

#[test]
fn configure_sizes_are_never_negative_on_an_output_smaller_than_the_frame() {
    // xdg-shell.xml gives 0 a meaning in `configure` and `configure_bounds`
    // and none to a negative size: an output too small for Poly's frame
    // gives 0, not the frame subtracted past it.
    let (mut srv, shared) = server_with(OutputConfig { width: 20, height: 16, ..OutputConfig::default() });
    let (mut p, registry) = connect(&mut srv, &shared);
    let g = globals(&mut p, &mut srv, registry);
    let w = mapped(&mut p, &mut srv, &g, "tiny", 10, 10, 0);
    assert!(decorated(&srv));
    p.take();
    p.send(&mut srv, w.toplevel, xdg_toplevel::Request::SetMaximized);
    let e = p.take();
    let all: Vec<_> = sizes(&e, "configure").into_iter().chain(sizes(&e, "configure_bounds")).collect();
    assert!(!all.is_empty());
    assert!(all.iter().all(|&(w, h)| w >= 0 && h >= 0), "{all:?}");
    assert!(srv.failure(p.id).is_none());
}
