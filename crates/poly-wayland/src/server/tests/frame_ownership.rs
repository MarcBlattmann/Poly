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
fn the_manager_announces_the_client_mode_as_the_default() {
    let (mut srv, shared) = server();
    let (mut p, registry) = connect(&mut srv, &shared);
    assert_eq!(p.globals["org_kde_kwin_server_decoration_manager"].1, 1);
    p.bind(&mut srv, registry, &org_kde_kwin_server_decoration_manager::INTERFACE, 1);
    let e = p.take();
    let d: Vec<_> =
        e.iter().filter(|e| e.name() == ("org_kde_kwin_server_decoration_manager", "default_mode")).collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].args, [Arg::Uint(KDE_CLIENT)]);
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
    assert!(!decorated(&srv), "the default is the client's frame");
    // GTK 3 asks for the mode in use right after creating the object: no
    // second event.
    p.take();
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_CLIENT });
    assert!(modes(&p.take()).is_empty());
    // Poly's frame when asked for, the client's again when asked for, and
    // releasing the object leaves the client's own (the manager is bound).
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    assert_eq!(modes(&p.take()), [KDE_SERVER]);
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(decorated(&srv));
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_CLIENT });
    assert_eq!(modes(&p.take()), [KDE_CLIENT]);
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(!decorated(&srv));
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    p.take();
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
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    let e = p.take();
    assert_eq!(modes(&e), [KDE_CLIENT, KDE_SERVER]);
    // Its frame changes at its next commit; it is not configured again
    // (GTK 3 does not take a configure of another size on top of the mode).
    assert!(e.iter().all(|e| e.name().1 != "configure"), "{e:?}");
    p.send(&mut srv, w.surface, wl_surface::Request::Commit);
    assert!(decorated(&srv));
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
    let deco = p.new_id(&org_kde_kwin_server_decoration::INTERFACE);
    p.send(&mut srv, manager, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface: w.surface });
    p.take();
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
    p.send(&mut srv, kde, org_kde_kwin_server_decoration_manager::Request::Create { id: deco, surface: w.surface });
    let last_mode = |e: &[Received]| e.iter().rev().find(|e| e.object == xdg_deco).map(|e| e.args.clone());
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_SERVER });
    let server_side = zxdg_toplevel_decoration_v1::Mode::SERVER_SIDE.0;
    assert_eq!(last_mode(&p.take()), Some(vec![Arg::Uint(server_side)]));
    p.send(&mut srv, deco, org_kde_kwin_server_decoration::Request::RequestMode { mode: KDE_CLIENT });
    let client_side = zxdg_toplevel_decoration_v1::Mode::CLIENT_SIDE.0;
    assert_eq!(last_mode(&p.take()), Some(vec![Arg::Uint(client_side)]));
}
