//! The window manager against a scripted X server: requests are decoded
//! from the bytes it writes, replies and events encoded as a server
//! would.

use super::*;
use crate::event::{XError, encode_error};
use crate::request::{encode_reply, opcode};
use crate::setup::{Screen, Setup, encode_success};
use crate::wire::{u16_at, u32_at};
use alloc::vec;

const ROOT: Xid = 0x3b6;
const APP: Xid = 0x0040_0001;
const MENU: Xid = 0x0040_0010;

/// A decoded request: opcode, data byte, the bytes after the header.
#[derive(Clone, Debug)]
struct Req {
    op: u8,
    data: u8,
    body: Vec<u8>,
    seq: u64,
}

impl Req {
    fn u32(&self, i: usize) -> u32 {
        u32_at(&self.body, i * 4)
    }
}

/// The placement policy of the tests: everything at (100, 80), frame
/// 1/1/25/1.
#[derive(Debug, Default)]
struct Place {
    asked: Vec<(Xid, u32, u32, Option<Xid>)>,
}

impl Policy for Place {
    fn place(&mut self, w: &Window, parent: Option<&Window>) -> (i32, i32) {
        self.asked.push((w.id, w.width, w.height, parent.map(|p| p.id)));
        (100, 80)
    }

    fn frame_extents(&self) -> [u32; 4] {
        [1, 1, 25, 1]
    }
}

/// The scripted server.
struct Server {
    wm: Wm,
    place: Place,
    /// Requests seen, numbered as the server counts them.
    seen: Vec<Req>,
    seq: u64,
}

impl Server {
    fn new() -> Server {
        let mut wm = Wm::new();
        let setup = wm.take_output();
        assert_eq!(setup[0], b'l');
        let mut s = Server { wm, place: Place::default(), seen: Vec::new(), seq: 0 };
        let reply = encode_success(&Setup {
            resource_id_base: 0x0020_0000,
            resource_id_mask: 0x001f_ffff,
            screens: vec![Screen { root: ROOT, width: 1280, height: 800, root_depth: 24, ..Screen::default() }],
            ..Setup::default()
        });
        s.feed(&reply);
        s
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.wm.feed(bytes, &mut self.place).unwrap();
    }

    /// Decode what the window manager wrote since last time.
    fn requests(&mut self) -> Vec<Req> {
        let out = self.wm.take_output();
        let mut at = 0;
        let mut got = Vec::new();
        while at < out.len() {
            let units = usize::from(u16_at(&out, at + 2));
            assert!(units > 0, "a zero-length request");
            self.seq += 1;
            got.push(Req { op: out[at], data: out[at + 1], body: out[at + 4..at + units * 4].to_vec(), seq: self.seq });
            at += units * 4;
        }
        assert_eq!(at, out.len(), "requests end on a request boundary");
        self.seen.extend(got.iter().cloned());
        got
    }

    fn event(&mut self, e: Event) {
        let b = e.encode(self.seq as u16);
        self.feed(&b);
    }

    fn reply(&mut self, seq: u64, data: u8, body: &[u8]) {
        let r = encode_reply(data, seq as u16, body);
        self.feed(&r);
    }

    /// Answer every `InternAtom` with 0x100 + its index.
    fn answer_atoms(&mut self) {
        let reqs = self.requests();
        let interns: Vec<Req> = reqs.iter().filter(|r| r.op == opcode::INTERN_ATOM).cloned().collect();
        assert_eq!(interns.len(), Atoms::NAMES.len());
        for (i, r) in interns.iter().enumerate() {
            let n = usize::from(u16_at(&r.body, 0));
            assert_eq!(&r.body[4..4 + n], Atoms::NAMES[i].as_bytes());
            let mut body = vec![0u8; 24];
            body[..4].copy_from_slice(&(0x100 + i as u32).to_le_bytes());
            self.reply(r.seq, 0, &body);
        }
    }

    /// Answer `GetProperty` requests for `window` from `props` (property,
    /// type, format, value); the rest as missing.
    fn answer_properties(&mut self, window: Xid, props: &[(Xid, Xid, u8, Vec<u8>)]) {
        let reqs = self.requests();
        for r in reqs.iter().filter(|r| r.op == opcode::GET_PROPERTY && r.u32(0) == window) {
            let property = r.u32(1);
            match props.iter().find(|(p, ..)| *p == property) {
                Some((_, ty, format, value)) => {
                    let mut body = vec![0u8; 24];
                    body[..4].copy_from_slice(&ty.to_le_bytes());
                    let items = value.len() / usize::from(format / 8);
                    body[8..12].copy_from_slice(&(items as u32).to_le_bytes());
                    body.extend_from_slice(value);
                    self.reply(r.seq, *format, &body);
                }
                None => self.reply(r.seq, 0, &[]),
            }
        }
    }

    fn atoms(&self) -> Atoms {
        *self.wm.atoms()
    }

    fn running() -> Server {
        let mut s = Server::new();
        s.answer_atoms();
        assert!(s.wm.is_running());
        assert_eq!(s.wm.take_actions(), [Action::Ready]);
        s
    }

    fn create(&mut self, window: Xid, override_redirect: bool) {
        self.event(Event::CreateNotify {
            parent: ROOT,
            window,
            x: 10,
            y: 20,
            width: 150,
            height: 100,
            border_width: 1,
            override_redirect,
        });
    }
}

fn find(reqs: &[Req], op: u8) -> Vec<Req> {
    reqs.iter().filter(|r| r.op == op).cloned().collect()
}

/// The `ChangeProperty` requests for (window, property).
fn property_set(reqs: &[Req], window: Xid, property: Xid) -> Option<Req> {
    find(reqs, opcode::CHANGE_PROPERTY).into_iter().rfind(|r| r.u32(0) == window && r.u32(1) == property)
}

fn value32(r: &Req) -> Vec<u32> {
    let n = r.u32(4) as usize;
    (0..n).map(|i| r.u32(5 + i)).collect()
}

#[test]
fn starts_by_redirecting_the_root_and_naming_itself() {
    let mut s = Server::new();
    let reqs = s.requests();
    let select = find(&reqs, opcode::CHANGE_WINDOW_ATTRIBUTES);
    assert_eq!(select.len(), 1);
    assert_eq!(select[0].u32(0), ROOT);
    assert_eq!(select[0].u32(1), cw::EVENT_MASK);
    let mask = select[0].u32(2);
    assert_eq!(mask & event_mask::SUBSTRUCTURE_REDIRECT, event_mask::SUBSTRUCTURE_REDIRECT);
    assert_eq!(mask & event_mask::SUBSTRUCTURE_NOTIFY, event_mask::SUBSTRUCTURE_NOTIFY);
    // Composite: asked for at once, then the root's children redirected.
    let query = find(&reqs, opcode::QUERY_EXTENSION);
    assert_eq!(query.len(), 1);
    assert_eq!(&query[0].body[4..13], b"Composite");
    let create = find(&reqs, opcode::CREATE_WINDOW);
    assert_eq!(create.len(), 1);
    let check = create[0].u32(0);
    assert_eq!(check, 0x0020_0001, "the first resource ID");
    assert_eq!(create[0].u32(1), ROOT);
    // Events before the atoms wait.
    s.create(APP, false);
    assert!(s.wm.take_actions().is_empty());
    // The atoms come back: properties, selections, then the deferred event.
    s.seq = reqs.last().unwrap().seq;
    for (i, r) in reqs.iter().filter(|r| r.op == opcode::INTERN_ATOM).enumerate() {
        let mut body = vec![0u8; 24];
        body[..4].copy_from_slice(&(0x100 + i as u32).to_le_bytes());
        s.reply(r.seq, 0, &body);
    }
    assert!(s.wm.is_running());
    let a = s.atoms();
    assert_eq!(a.WM_PROTOCOLS, 0x100);
    let reqs = s.requests();
    let check_prop = property_set(&reqs, ROOT, a._NET_SUPPORTING_WM_CHECK).unwrap();
    assert_eq!(value32(&check_prop), [check]);
    assert!(property_set(&reqs, check, a._NET_SUPPORTING_WM_CHECK).is_some());
    let name = property_set(&reqs, check, a._NET_WM_NAME).unwrap();
    assert_eq!(&name.body[20..24], b"Poly");
    let supported = value32(&property_set(&reqs, ROOT, a._NET_SUPPORTED).unwrap());
    assert!(supported.contains(&a._NET_ACTIVE_WINDOW) && supported.contains(&a._NET_WM_WINDOW_TYPE));
    let owners: Vec<Xid> = find(&reqs, opcode::SET_SELECTION_OWNER).iter().map(|r| r.u32(1)).collect();
    assert_eq!(owners, [a.WM_S0, a._NET_WM_CM_S0]);
    assert_eq!(s.wm.take_actions(), [Action::Ready, Action::Changed(APP)]);
    // The deferred CreateNotify was handled: the window's properties are read.
    assert!(find(&reqs, opcode::GET_PROPERTY).iter().any(|r| r.u32(0) == APP && r.u32(1) == predefined::WM_NAME));
}

#[test]
fn another_window_manager_is_reported() {
    let mut s = Server::new();
    let reqs = s.requests();
    let select = find(&reqs, opcode::CHANGE_WINDOW_ATTRIBUTES)[0].seq;
    let e = XError { code: error_code::ACCESS, sequence: select as u16, value: ROOT, minor_opcode: 0, major_opcode: 2 };
    s.feed(&encode_error(&e));
    assert!(matches!(&s.wm.take_actions()[..], [Action::Log(l)] if l.contains("another X window manager")));
}

#[test]
fn a_window_is_read_placed_mapped_and_associated() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(APP, false);
    let protocols = icccm::u32_data(&[a.WM_DELETE_WINDOW, a.WM_TAKE_FOCUS]);
    s.answer_properties(
        APP,
        &[
            (predefined::WM_NAME, predefined::STRING, 8, b"xeyes".to_vec()),
            (predefined::WM_CLASS, predefined::STRING, 8, b"xeyes\0XEyes\0".to_vec()),
            (a.WM_PROTOCOLS, predefined::ATOM, 32, protocols),
            (a._NET_WM_PID, predefined::CARDINAL, 32, icccm::u32_data(&[42])),
        ],
    );
    let w = s.wm.window(APP).unwrap();
    assert_eq!((w.title(), w.class.1.as_str(), w.pid), ("xeyes", "XEyes", Some(42)));
    assert!(w.supports(a.WM_DELETE_WINDOW) && w.supports(a.WM_TAKE_FOCUS));
    assert_eq!((w.x, w.y, w.width, w.height, w.managed, w.mapped), (10, 20, 150, 100, false, false));
    assert!(w.decorated(&a));
    s.wm.take_actions();

    // The map request: placed by the policy, border 0, WM_STATE, mapped.
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    assert_eq!(s.place.asked, [(APP, 150, 100, None)]);
    let reqs = s.requests();
    let conf = find(&reqs, opcode::CONFIGURE_WINDOW);
    assert_eq!(conf.len(), 1);
    assert_eq!(conf[0].u32(0), APP);
    let mask = u16_at(&conf[0].body, 4);
    assert_eq!(mask, config::X | config::Y | config::BORDER_WIDTH);
    assert_eq!((conf[0].u32(2), conf[0].u32(3), conf[0].u32(4)), (100, 80, 0));
    let state = property_set(&reqs, APP, a.WM_STATE).unwrap();
    assert_eq!((state.u32(2), value32(&state)), (a.WM_STATE, vec![icccm::state::NORMAL, 0]));
    assert_eq!(value32(&property_set(&reqs, APP, a._NET_FRAME_EXTENTS).unwrap()), [1, 1, 25, 1]);
    let map = find(&reqs, opcode::MAP_WINDOW);
    assert_eq!(map.iter().map(|r| r.u32(0)).collect::<Vec<_>>(), [APP]);
    assert!(map[0].seq > conf[0].seq, "placed before it is mapped");
    assert_eq!(value32(&property_set(&reqs, ROOT, a._NET_CLIENT_LIST).unwrap()), [APP]);
    assert!(s.wm.window(APP).unwrap().managed);

    // The server maps it and moves it.
    s.event(Event::MapNotify { event: ROOT, window: APP, override_redirect: false });
    s.event(Event::ConfigureNotify {
        event: ROOT,
        window: APP,
        above_sibling: 0,
        x: 100,
        y: 80,
        width: 150,
        height: 100,
        border_width: 0,
        override_redirect: false,
    });
    let w = s.wm.window(APP).unwrap();
    assert_eq!((w.x, w.y, w.mapped), (100, 80, true));

    // Xwayland names its surface, both ways.
    s.event(client_message32(APP, a.WL_SURFACE_SERIAL, [7, 1, 0, 0, 0]));
    s.event(client_message32(APP, a.WL_SURFACE_ID, [33, 0, 0, 0, 0]));
    let acts = s.wm.take_actions();
    assert!(acts.contains(&Action::Associate { window: APP, surface: SurfaceRef::Serial(0x1_0000_0007) }));
    assert!(acts.contains(&Action::Associate { window: APP, surface: SurfaceRef::Id(33) }));
}

#[test]
fn titles_follow_property_changes_and_prefer_net_wm_name() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(APP, false);
    s.answer_properties(APP, &[(predefined::WM_NAME, predefined::STRING, 8, b"old".to_vec())]);
    assert_eq!(s.wm.window(APP).unwrap().title(), "old");
    s.event(Event::PropertyNotify { window: APP, atom: a._NET_WM_NAME, time: 0, state: 0 });
    s.answer_properties(APP, &[(a._NET_WM_NAME, a.UTF8_STRING, 8, "Ünïcode".as_bytes().to_vec())]);
    assert_eq!(s.wm.window(APP).unwrap().title(), "Ünïcode");
    assert!(s.wm.take_actions().contains(&Action::Changed(APP)));
    // Deleted: back to WM_NAME.
    s.event(Event::PropertyNotify { window: APP, atom: a._NET_WM_NAME, time: 0, state: 1 });
    assert_eq!(s.wm.window(APP).unwrap().title(), "old");
    // A property nobody reads is not fetched.
    s.event(Event::PropertyNotify { window: APP, atom: 0x7777, time: 0, state: 0 });
    assert!(find(&s.requests(), opcode::GET_PROPERTY).is_empty());
}

#[test]
fn override_redirect_windows_are_shown_but_not_managed() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(MENU, true);
    s.answer_properties(
        MENU,
        &[(a._NET_WM_WINDOW_TYPE, predefined::ATOM, 32, icccm::u32_data(&[a._NET_WM_WINDOW_TYPE_POPUP_MENU]))],
    );
    s.event(Event::MapNotify { event: ROOT, window: MENU, override_redirect: true });
    let w = s.wm.window(MENU).unwrap();
    assert!(w.mapped && !w.managed && w.override_redirect);
    assert!(!w.decorated(&a));
    assert!(s.place.asked.is_empty());
    assert!(find(&s.requests(), opcode::CONFIGURE_WINDOW).is_empty(), "never moved by the window manager");
    s.event(Event::UnmapNotify { event: ROOT, window: MENU, from_configure: false });
    assert!(!s.wm.window(MENU).unwrap().mapped);
    s.event(Event::DestroyNotify { event: ROOT, window: MENU });
    assert!(s.wm.window(MENU).is_none());
    assert!(s.wm.take_actions().contains(&Action::Destroyed(MENU)));
}

#[test]
fn window_types_and_motif_hints_decide_the_frame() {
    let mut s = Server::running();
    let a = s.atoms();
    let mut w = Window::new(1);
    assert!(w.decorated(&a), "no type: normal");
    w.window_type = vec![0x7777, a._NET_WM_WINDOW_TYPE_DIALOG];
    assert!(w.decorated(&a), "the first known type decides");
    w.window_type = vec![a._NET_WM_WINDOW_TYPE_SPLASH, a._NET_WM_WINDOW_TYPE_NORMAL];
    assert!(!w.decorated(&a));
    w.window_type = vec![0x7777];
    assert!(w.decorated(&a), "unknown types only: normal");
    w.window_type = vec![a._NET_WM_WINDOW_TYPE_UTILITY];
    w.decorations = Some(false);
    assert!(!w.decorated(&a), "_MOTIF_WM_HINTS without decorations");
    // Read from the properties.
    s.create(APP, false);
    let motif = icccm::u32_data(&[icccm::MWM_HINTS_DECORATIONS, 0, 0, 0, 0]);
    s.answer_properties(APP, &[(a._MOTIF_WM_HINTS, a._MOTIF_WM_HINTS, 32, motif)]);
    assert!(!s.wm.window(APP).unwrap().decorated(&a));
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    let reqs = s.requests();
    assert_eq!(value32(&property_set(&reqs, APP, a._NET_FRAME_EXTENTS).unwrap()), [0, 0, 0, 0]);
}

#[test]
fn size_hints_and_the_screen_limit_the_size() {
    let mut s = Server::running();
    s.create(APP, false);
    let mut hints = vec![icccm::P_MIN_SIZE, 0, 0, 0, 0, 200, 150];
    hints.resize(18, 0);
    s.answer_properties(APP, &[(predefined::WM_NORMAL_HINTS, predefined::WM_SIZE_HINTS, 32, icccm::u32_data(&hints))]);
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    assert_eq!(s.place.asked, [(APP, 200, 150, None)]);
    let conf = find(&s.requests(), opcode::CONFIGURE_WINDOW).remove(0);
    assert_eq!(u16_at(&conf.body, 4), config::X | config::Y | config::WIDTH | config::HEIGHT | config::BORDER_WIDTH);
    assert_eq!((conf.u32(4), conf.u32(5)), (200, 150));
}

#[test]
fn transients_are_placed_against_their_parent() {
    let mut s = Server::running();
    s.create(APP, false);
    s.answer_properties(APP, &[]);
    let dialog = APP + 5;
    s.create(dialog, false);
    s.answer_properties(dialog, &[(predefined::WM_TRANSIENT_FOR, predefined::WINDOW, 32, icccm::u32_data(&[APP]))]);
    assert_eq!(s.wm.window(dialog).unwrap().transient_for, Some(APP));
    s.event(Event::MapRequest { parent: ROOT, window: dialog });
    assert_eq!(s.place.asked, [(dialog, 150, 100, Some(APP))]);
}

#[test]
fn configure_requests_resize_but_do_not_move_managed_windows() {
    let mut s = Server::running();
    s.create(APP, false);
    s.answer_properties(APP, &[]);
    // Not managed yet: position and size as asked.
    s.event(Event::ConfigureRequest {
        stack_mode: 0,
        parent: ROOT,
        window: APP,
        sibling: 0,
        x: 5,
        y: 6,
        width: 300,
        height: 200,
        border_width: 0,
        value_mask: config::X | config::Y | config::WIDTH | config::HEIGHT,
    });
    let conf = find(&s.requests(), opcode::CONFIGURE_WINDOW).remove(0);
    assert_eq!(u16_at(&conf.body, 4), 0x0f);
    assert_eq!((conf.u32(2), conf.u32(3), conf.u32(4), conf.u32(5)), (5, 6, 300, 200));
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    s.requests();
    // Managed: the size only.
    s.event(Event::ConfigureRequest {
        stack_mode: 0,
        parent: ROOT,
        window: APP,
        sibling: 0,
        x: 500,
        y: 600,
        width: 320,
        height: 100,
        border_width: 0,
        value_mask: config::X | config::Y | config::WIDTH,
    });
    let conf = find(&s.requests(), opcode::CONFIGURE_WINDOW).remove(0);
    assert_eq!(u16_at(&conf.body, 4), config::WIDTH);
    assert_eq!(conf.u32(2), 320);
    // Only a move: a synthetic ConfigureNotify with where it is.
    s.event(Event::ConfigureRequest {
        stack_mode: 0,
        parent: ROOT,
        window: APP,
        sibling: 0,
        x: 1,
        y: 1,
        width: 0,
        height: 0,
        border_width: 0,
        value_mask: config::X | config::Y,
    });
    let reqs = s.requests();
    assert!(find(&reqs, opcode::CONFIGURE_WINDOW).is_empty());
    let sent = find(&reqs, opcode::SEND_EVENT).remove(0);
    assert_eq!((sent.u32(0), sent.u32(1)), (APP, event_mask::STRUCTURE_NOTIFY));
    let mut ev = [0u8; 32];
    ev.copy_from_slice(&sent.body[8..40]);
    match Event::decode(&ev) {
        Event::ConfigureNotify { window, x, y, width, height, .. } => {
            assert_eq!((window, x, y, width, height), (APP, 100, 80, 150, 100));
        }
        e => panic!("{e:?}"),
    }
}

#[test]
fn activation_follows_the_input_model() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(APP, false);
    s.answer_properties(APP, &[(a.WM_PROTOCOLS, predefined::ATOM, 32, icccm::u32_data(&[a.WM_TAKE_FOCUS]))]);
    // Not managed: nothing.
    s.wm.activate(Some(APP));
    assert!(s.requests().is_empty());
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    s.requests();
    s.wm.activate(Some(APP));
    assert_eq!(s.wm.focus(), Some(APP));
    let reqs = s.requests();
    let focus = find(&reqs, opcode::SET_INPUT_FOCUS).remove(0);
    assert_eq!((focus.data, focus.u32(0)), (revert::POINTER_ROOT, APP));
    let sent = find(&reqs, opcode::SEND_EVENT).remove(0);
    let mut ev = [0u8; 32];
    ev.copy_from_slice(&sent.body[8..40]);
    let Event::ClientMessage { ty, data, window, .. } = Event::decode(&ev) else { panic!() };
    assert_eq!((window, ty, data32(&data)[0]), (APP, a.WM_PROTOCOLS, a.WM_TAKE_FOCUS));
    let raise = find(&reqs, opcode::CONFIGURE_WINDOW).remove(0);
    assert_eq!((u16_at(&raise.body, 4), raise.u32(2)), (config::STACK_MODE, stack::ABOVE));
    assert_eq!(value32(&property_set(&reqs, ROOT, a._NET_ACTIVE_WINDOW).unwrap()), [APP]);
    // A Wayland window takes the keyboard.
    s.wm.activate(None);
    let reqs = s.requests();
    assert_eq!(find(&reqs, opcode::SET_INPUT_FOCUS).remove(0).u32(0), 0);
    assert_eq!(value32(&property_set(&reqs, ROOT, a._NET_ACTIVE_WINDOW).unwrap()), [0]);
    assert_eq!(s.wm.focus(), None);
    s.wm.activate(None);
    assert!(s.requests().is_empty(), "nothing to take");
    // A client without input (WM_HINTS input False) gets no SetInputFocus.
    s.event(Event::PropertyNotify { window: APP, atom: predefined::WM_HINTS, time: 0, state: 0 });
    s.answer_properties(
        APP,
        &[(predefined::WM_HINTS, predefined::WM_HINTS, 32, icccm::u32_data(&[1, 0, 1, 0, 0, 0, 0, 0, 0]))],
    );
    s.wm.activate(Some(APP));
    let reqs = s.requests();
    assert!(find(&reqs, opcode::SET_INPUT_FOCUS).is_empty());
    assert_eq!(find(&reqs, opcode::SEND_EVENT).len(), 1, "WM_TAKE_FOCUS only");
}

#[test]
fn closing_asks_politely_or_kills() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(APP, false);
    s.answer_properties(APP, &[(a.WM_PROTOCOLS, predefined::ATOM, 32, icccm::u32_data(&[a.WM_DELETE_WINDOW]))]);
    s.wm.close(APP);
    let sent = find(&s.requests(), opcode::SEND_EVENT).remove(0);
    assert_eq!((sent.data, sent.u32(0), sent.u32(1)), (0, APP, 0));
    let mut ev = [0u8; 32];
    ev.copy_from_slice(&sent.body[8..40]);
    let Event::ClientMessage { format, ty, data, .. } = Event::decode(&ev) else { panic!() };
    assert_eq!((format, ty, data32(&data)[0]), (32, a.WM_PROTOCOLS, a.WM_DELETE_WINDOW));
    let other = APP + 9;
    s.create(other, false);
    s.answer_properties(other, &[]);
    s.wm.close(other);
    let kill = find(&s.requests(), opcode::KILL_CLIENT).remove(0);
    assert_eq!(kill.u32(0), other);
}

#[test]
fn unmapping_withdraws_and_destroying_forgets() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(APP, false);
    s.answer_properties(APP, &[]);
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    s.event(Event::MapNotify { event: ROOT, window: APP, override_redirect: false });
    s.wm.activate(Some(APP));
    s.requests();
    s.event(Event::UnmapNotify { event: ROOT, window: APP, from_configure: false });
    let reqs = s.requests();
    assert_eq!(value32(&property_set(&reqs, APP, a.WM_STATE).unwrap()), [icccm::state::WITHDRAWN, 0]);
    assert_eq!(value32(&property_set(&reqs, ROOT, a._NET_CLIENT_LIST).unwrap()), Vec::<u32>::new());
    assert_eq!(value32(&property_set(&reqs, ROOT, a._NET_ACTIVE_WINDOW).unwrap()), [0]);
    let w = s.wm.window(APP).unwrap();
    assert!(!w.mapped && !w.managed);
    assert_eq!(s.wm.focus(), None);
    // Unmap events of other windows' children are not ours.
    s.event(Event::UnmapNotify { event: APP, window: APP + 1, from_configure: false });
    s.event(Event::DestroyNotify { event: ROOT, window: APP });
    assert!(s.wm.window(APP).is_none());
    let acts = s.wm.take_actions();
    assert_eq!(acts.last(), Some(&Action::Destroyed(APP)));
    // Destroying a window never seen is nothing.
    s.event(Event::DestroyNotify { event: ROOT, window: 0x999 });
    assert!(s.wm.take_actions().is_empty());
}

#[test]
fn client_requests_for_activation_and_moves() {
    let mut s = Server::running();
    let a = s.atoms();
    s.create(APP, false);
    s.answer_properties(APP, &[]);
    // Before it is managed: ignored.
    s.event(client_message32(APP, a._NET_ACTIVE_WINDOW, [1, 0, 0, 0, 0]));
    s.wm.take_actions();
    s.event(Event::MapRequest { parent: ROOT, window: APP });
    s.wm.take_actions();
    s.event(client_message32(APP, a._NET_ACTIVE_WINDOW, [1, 0, 0, 0, 0]));
    s.event(client_message32(APP, a._NET_WM_MOVERESIZE, [300, 200, 8, 1, 1]));
    s.event(client_message32(APP, a._NET_WM_MOVERESIZE, [300, 200, 4, 1, 1]));
    assert_eq!(s.wm.take_actions(), [Action::Activate(APP), Action::Move(APP)]);
}

#[test]
fn moves_from_the_compositor_reach_x() {
    let mut s = Server::running();
    s.create(APP, false);
    s.answer_properties(APP, &[]);
    s.wm.move_to(APP, 300, 200);
    let conf = find(&s.requests(), opcode::CONFIGURE_WINDOW).remove(0);
    assert_eq!((u16_at(&conf.body, 4), conf.u32(2), conf.u32(3)), (config::X | config::Y, 300, 200));
    s.wm.move_to(APP, 300, 200);
    assert!(s.requests().is_empty(), "already there");
    // Negative coordinates are INT16 values in 32-bit slots.
    s.wm.move_to(APP, -4, 30);
    let conf = find(&s.requests(), opcode::CONFIGURE_WINDOW).remove(0);
    assert_eq!(conf.u32(2) as i32, -4);
}

#[test]
fn manager_selection_conversions_are_refused() {
    let mut s = Server::running();
    let a = s.atoms();
    s.event(Event::SelectionRequest {
        time: 5,
        owner: 0x0020_0001,
        requestor: 77,
        selection: a.WM_S0,
        target: 9,
        property: 10,
    });
    let sent = find(&s.requests(), opcode::SEND_EVENT).remove(0);
    assert_eq!(sent.u32(0), 77);
    let mut ev = [0u8; 32];
    ev.copy_from_slice(&sent.body[8..40]);
    assert_eq!(
        Event::decode(&ev),
        Event::SelectionNotify { time: 5, requestor: 77, selection: a.WM_S0, target: 9, property: 0 }
    );
    s.event(Event::SelectionClear { time: 1, owner: 0x0020_0001, selection: a.WM_S0 });
    assert!(matches!(&s.wm.take_actions()[..], [Action::Log(l)] if l.contains("WM_S0")));
}

#[test]
fn property_errors_for_vanished_windows_are_quiet() {
    let mut s = Server::running();
    s.create(APP, false);
    let reqs = s.requests();
    s.wm.take_actions();
    for r in find(&reqs, opcode::GET_PROPERTY) {
        let e =
            XError { code: error_code::WINDOW, sequence: r.seq as u16, value: APP, minor_opcode: 0, major_opcode: 20 };
        s.feed(&encode_error(&e));
    }
    assert!(s.wm.take_actions().is_empty());
    let e = XError { code: error_code::MATCH, sequence: s.seq as u16, value: 0, minor_opcode: 0, major_opcode: 12 };
    s.feed(&encode_error(&e));
    assert!(matches!(&s.wm.take_actions()[..], [Action::Log(l)] if l.contains("X error 8")));
}

#[test]
fn the_root_children_are_redirected_through_composite() {
    let mut s = Server::new();
    let reqs = s.requests();
    let query = find(&reqs, opcode::QUERY_EXTENSION).remove(0);
    let mut body = vec![0u8; 24];
    (body[0], body[1]) = (1, 142);
    s.reply(query.seq, 0, &body);
    let reqs = s.requests();
    let ext: Vec<&Req> = reqs.iter().filter(|r| r.op == 142).collect();
    assert_eq!(ext.len(), 2);
    assert_eq!((ext[0].data, ext[0].u32(0), ext[0].u32(1)), (composite::QUERY_VERSION, 0, 4));
    assert_eq!((ext[1].data, ext[1].u32(0), ext[1].body[4]), (composite::REDIRECT_SUBWINDOWS, ROOT, 1));
    // Without Composite: said so.
    let mut s = Server::new();
    let query = find(&s.requests(), opcode::QUERY_EXTENSION).remove(0);
    s.reply(query.seq, 0, &[]);
    assert!(matches!(&s.wm.take_actions()[..], [Action::Log(l)] if l.contains("no Composite")));
}
