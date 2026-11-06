//! The client against `poly-wayland`'s own server, in one process: the
//! bytes and handles each side queues are handed to the other, shared
//! memory is a vector both see.

use crate::proto::{Client, CursorShape, Event};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_wayland::compose::Rect;
use poly_wayland::seat::{Capabilities, InputEvent};
use poly_wayland::server::{ClientId, Config, Host, OutputConfig, Server, ShmMap};
use poly_wayland::wire::{Fd, Fixed};
use std::collections::BTreeMap;

type Memory = Rc<RefCell<Vec<u8>>>;

#[derive(Default)]
struct Files(BTreeMap<i32, Memory>);

struct TestHost(Rc<RefCell<Files>>);

struct Map(Memory, usize);

impl ShmMap for Map {
    fn len(&self) -> usize {
        self.1
    }

    fn resize(&mut self, size: usize) -> bool {
        self.1 = size;
        size <= self.0.borrow().len()
    }

    fn read(&self, offset: usize, dst: &mut [u8]) -> bool {
        let m = self.0.borrow();
        match m.get(offset..offset + dst.len()) {
            Some(src) => {
                dst.copy_from_slice(src);
                true
            }
            None => false,
        }
    }
}

impl Host for TestHost {
    fn map_shm(&mut self, fd: Fd, size: usize) -> Option<Box<dyn ShmMap>> {
        let m = self.0.borrow().0.get(&fd.0).cloned()?;
        Some(Box::new(Map(m, size)))
    }

    fn close_fd(&mut self, _fd: Fd) {}

    fn keymap(&mut self) -> Option<(Fd, u32)> {
        Some((Fd(900), 10))
    }
}

struct World {
    server: Server,
    files: Rc<RefCell<Files>>,
    id: ClientId,
    client: Client,
    next_fd: i32,
}

impl World {
    fn new(scale: u32) -> World {
        let files = Rc::new(RefCell::new(Files::default()));
        let output = OutputConfig { width: 1280 * scale, height: 800 * scale, scale, ..OutputConfig::default() };
        let mut server = Server::new(Config { output, ..Config::default() }, Box::new(TestHost(Rc::clone(&files))));
        server.set_seat_capabilities(Capabilities(Capabilities::POINTER.0 | Capabilities::KEYBOARD.0));
        let id = server.add_client();
        let client = Client::new("Files", "poly-files", (800, 520), (400, 300));
        let mut w = World { server, files, id, client, next_fd: 100 };
        w.pump();
        w
    }

    /// Move bytes both ways until both are quiet.
    fn pump(&mut self) {
        for _ in 0..20 {
            let (bytes, fds) = self.client.take_output();
            let (sb, sf) = self.server.take_output(self.id);
            if bytes.is_empty() && sb.is_empty() {
                return;
            }
            if !bytes.is_empty() {
                self.server.receive(self.id, &bytes, fds);
            }
            if !sb.is_empty() {
                self.client.receive(&sb, sf);
            }
        }
        panic!("the client and the server keep talking");
    }

    fn events(&mut self) -> Vec<Event> {
        core::iter::from_fn(|| self.client.poll_event()).collect()
    }

    /// A buffer the size the client wants, filled with `rgb`, shown.
    fn draw(&mut self, rgb: u32) {
        let (w, h) = self.client.buffer_size();
        let mut mem = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            mem.extend_from_slice(&rgb.to_le_bytes());
        }
        let fd = self.next_fd;
        self.next_fd += 1;
        self.files.borrow_mut().0.insert(fd, Rc::new(RefCell::new(mem)));
        self.client.set_buffer(Fd(fd), w, h);
        self.client.present(&[Rect::new(0, 0, w, h)]);
        self.pump();
    }
}

#[test]
fn a_window_comes_up_configured_and_shows_its_buffer() {
    let mut w = World::new(1);
    assert!(w.server.failure(w.id).is_none(), "{:?}", w.server.failure(w.id));
    assert!(w.client.configured());
    let ev = w.events();
    assert!(ev.contains(&Event::Resized { width: 800, height: 520, scale: 1 }), "{ev:?}");
    assert!(w.client.needs_buffer());
    w.draw(0x20_3040);
    assert!(w.server.failure(w.id).is_none(), "{:?}", w.server.failure(w.id));
    let win = w.server.scene().windows().iter().find(|x| x.title() == "Files").expect("the window is in the scene");
    assert_eq!(win.content().logical_size(), (800, 520));
    assert_eq!(win.content().pixels[0] & 0xff_ffff, 0x20_3040);
    assert!(w.client.frame_pending());
    assert!(!w.client.needs_buffer());
}

#[test]
fn hidpi_buffers_are_scale_times_larger() {
    let mut w = World::new(2);
    let ev = w.events();
    assert!(ev.iter().any(|e| matches!(e, Event::Resized { scale: 2, .. })), "{ev:?}");
    assert_eq!(w.client.buffer_size(), (1600, 1040));
    w.draw(0x11_2233);
    let win = w.server.scene().windows().iter().find(|x| x.title() == "Files").unwrap();
    assert_eq!(win.content().logical_size(), (800, 520));
    assert_eq!(win.content().scale, 2);
}

#[test]
fn keys_and_buttons_reach_the_app() {
    let mut w = World::new(1);
    w.draw(0);
    w.events();
    // The pointer onto the window (its content starts below the frame's
    // title bar), a click, a key.
    let win = w.server.scene().windows().iter().find(|x| x.title() == "Files").unwrap();
    let r = win.surface_rect();
    let (x, y) = (r.x + 50, r.y + 40);
    w.server.handle_input(InputEvent::PointerMotionAbsolute { time: 1, x: Fixed::from_int(x), y: Fixed::from_int(y) });
    w.server.handle_input(InputEvent::PointerButton { time: 2, button: crate::BTN_LEFT, pressed: true });
    w.server.handle_input(InputEvent::PointerButton { time: 3, button: crate::BTN_LEFT, pressed: false });
    w.server.handle_input(InputEvent::Key { time: 4, key: 30, pressed: true });
    w.server.handle_input(InputEvent::Key { time: 5, key: 30, pressed: false });
    w.pump();
    let ev = w.events();
    assert!(
        ev.iter().any(|e| matches!(e, Event::Button { button: 0x110, pressed: true, x, y } if (*x - 50.0).abs() < 1.0 && (*y - 40.0).abs() < 1.0)),
        "{ev:?}"
    );
    assert!(ev.iter().any(|e| matches!(e, Event::Key { code: 30, pressed: true, .. })), "{ev:?}");
    assert!(ev.iter().any(|e| matches!(e, Event::Key { code: 30, pressed: false, .. })), "{ev:?}");
    assert_eq!(w.client.take_unused_fds(), alloc::vec![Fd(900)], "the keymap's handle is the runner's to close");
    // A cursor shape after the enter goes out without a protocol error.
    w.client.set_cursor(CursorShape::Text);
    w.pump();
    assert!(w.server.failure(w.id).is_none(), "{:?}", w.server.failure(w.id));
}

#[test]
fn the_clipboard_goes_to_another_client() {
    let mut w = World::new(1);
    w.draw(0);
    // Keyboard focus is what lets a client set the selection.
    w.server.handle_input(InputEvent::Key { time: 1, key: 30, pressed: true });
    w.server.handle_input(InputEvent::Key { time: 2, key: 30, pressed: false });
    w.pump();
    w.events();
    w.client.set_clipboard("hello".into());
    w.pump();
    assert!(w.server.failure(w.id).is_none(), "{:?}", w.server.failure(w.id));
    assert_eq!(w.client.clipboard_text(), Some("hello"));
    let ev = w.events();
    assert!(ev.contains(&Event::ClipboardChanged { has_text: true }), "{ev:?}");
    // Asking for it (as another app would): the server asks the source to
    // write into the pipe.
    assert!(w.client.receive_clipboard(Fd(555)));
    w.pump();
    let ev = w.events();
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::SendClipboard { mime, fd } if mime.starts_with("text/plain") && fd.0 == 555)),
        "{ev:?}"
    );
}

#[test]
fn close_and_titles() {
    let mut w = World::new(1);
    w.draw(0);
    w.client.set_title("Terminal — ~/src");
    w.pump();
    let win = w.server.scene().windows().iter().find(|x| x.title().starts_with("Terminal")).map(|x| x.id());
    let win = win.expect("the new title");
    w.server.request_close(win);
    w.pump();
    assert!(w.events().contains(&Event::Close));
}
