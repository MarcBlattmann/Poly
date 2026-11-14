//! The event loop of a native app, on `poly-rt`: the connection to the
//! compositor (`$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY`), the window's buffer in
//! a memory object mapped shared, `poll` over the socket, the app's own
//! handles (a terminal's pty) and a pending paste, key repeat, the
//! clipboard's pipes, and the settings file looked at every 2 seconds so
//! the theme and the keyboard layout follow the desktop.
//!
//! The compositor copies a buffer when it is committed and releases it at
//! once (`poly-wayland`'s `wl_surface.commit`), so one buffer is enough and
//! keeps what was drawn: an app redraws only what changed and says where
//! ([`Cx::damage`]).

use crate::input::{self, KeyInput, Repeat};
use crate::proto::{Client, CursorShape, Event};
use crate::ui::{Look, Ui};
use alloc::string::String;
use alloc::vec::Vec;
use poly_rt::abi::{self, events};
use poly_rt::fs::File;
use poly_rt::net::UnixStream;
use poly_rt::poll::PollEntry;
use poly_rt::sys;
use poly_wayland::compose::{Framebuffer, Rect, Scale};
use poly_wayland::wire::Fd;

/// What happened, as the app hears it.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// The window is `width` × `height` logical pixels at `scale` now
    /// (also the first event): draw everything.
    Resized {
        width: i32,
        height: i32,
        scale: Scale,
    },
    Key(KeyInput),
    /// The pointer moved to (`x`, `y`), logical.
    Motion {
        x: i32,
        y: i32,
    },
    Leave,
    /// A button (272 left, 273 right, 274 middle); `clicks` counts quick
    /// presses at one place (2 for a double click); `mods` are the
    /// modifiers held.
    Button {
        button: u32,
        pressed: bool,
        x: i32,
        y: i32,
        clicks: u32,
        mods: input::Mods,
    },
    /// The wheel, logical pixels (positive: down).
    Scroll {
        dy: f64,
    },
    /// Text from the clipboard, as asked for with [`Cx::paste`].
    Paste(String),
    Focus(bool),
    /// The settings changed the theme (redraw everything).
    Theme,
}

/// What the app asks of the loop while it handles an event.
#[derive(Debug, Default)]
pub struct Cx {
    redraw: bool,
    damage: Vec<Rect>,
    quit: bool,
    clipboard: Option<String>,
    paste: bool,
    cursor: Option<CursorShape>,
    title: Option<String>,
    /// The monotonic clock, ms.
    pub now_ms: u64,
}

impl Cx {
    /// Draw everything again.
    pub fn redraw(&mut self) {
        self.redraw = true;
    }

    /// Draw again; `r` (logical) is what changed.
    pub fn damage(&mut self, r: Rect) {
        self.damage.push(r);
    }

    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// Put `text` on the clipboard.
    pub fn copy(&mut self, text: String) {
        self.clipboard = Some(text);
    }

    /// Ask for the clipboard's text (an [`AppEvent::Paste`] follows).
    pub fn paste(&mut self) {
        self.paste = true;
    }

    pub fn cursor(&mut self, shape: CursorShape) {
        self.cursor = Some(shape);
    }

    pub fn title(&mut self, title: String) {
        self.title = Some(title);
    }
}

/// A native app's window.
pub trait App {
    /// Something happened.
    fn event(&mut self, ev: AppEvent, cx: &mut Cx);
    /// Draw: everything when `full`, else what the app damaged; return the
    /// logical rectangles drawn (`None`: all).
    fn draw(&mut self, fb: &mut dyn Framebuffer, ui: &Ui, full: bool) -> Option<Vec<Rect>>;
    /// Handles to wait for besides the window (a pty), with the events.
    fn handles(&self) -> Vec<(u32, u16)> {
        Vec::new()
    }
    /// One of [`App::handles`] is ready (`revents`).
    fn ready(&mut self, _handle: u32, _revents: u16, _cx: &mut Cx) {}
    /// The earliest monotonic time (ms) the app wants [`App::tick`] at.
    fn deadline(&self) -> Option<u64> {
        None
    }
    fn tick(&mut self, _cx: &mut Cx) {}
    /// The user closed the window: whether to quit (the default).
    fn close(&mut self, cx: &mut Cx) {
        cx.quit();
    }
}

/// The window's pixels: a memory object mapped shared, `width` ×
/// `height` `u32`s.
struct Canvas {
    file: File,
    ptr: *mut u32,
    width: u32,
    height: u32,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Option<Canvas> {
        let len = width as usize * height as usize * 4;
        let file = poly_rt::mem::memory_object(len as u64).ok()?;
        let ptr = poly_rt::mem::map_handle(file.handle(), 0, len, abi::prot::READ | abi::prot::WRITE, true).ok()?;
        Some(Canvas { file, ptr: ptr.cast(), width, height })
    }
}

impl Framebuffer for Canvas {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        assert!(y < self.height, "row {y} of {}", self.height);
        // SAFETY: the mapping is `width * height` u32s, writable and
        // aligned (page-aligned), and lives as long as `self`; `y` is in
        // range (asserted), and the slice borrows `self` mutably, so no
        // other row slice aliases it. The compositor only reads the memory
        // while a commit copies it, which happens between our draws.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.add(y as usize * self.width as usize), self.width as usize) }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        let len = self.width as usize * self.height as usize * 4;
        // SAFETY: exactly the mapping made in `new`; no row slice outlives
        // `&mut self`.
        let _ = unsafe { poly_rt::mem::unmap(self.ptr.cast(), len) };
        let _ = &self.file;
    }
}

/// The settings file the desktop's look comes from.
pub fn settings_path() -> String {
    if let Some(p) = poly_rt::env::var("POLY_SHELL_SETTINGS").filter(|p| !p.is_empty()) {
        return p.into();
    }
    let base = poly_rt::env::var("XDG_CONFIG_HOME")
        .map(String::from)
        .unwrap_or_else(|| poly_rt::fs::join(poly_rt::env::var("HOME").unwrap_or("/root"), ".config"));
    poly_rt::fs::join(&poly_rt::fs::join(&base, "poly"), "shell.conf")
}

/// The desktop's fonts (Geist, Geist Mono; DejaVu Sans without them).
pub fn load_fonts() -> poly_shell::draw::Fonts {
    let load = |path: &str| -> Option<poly_font::Font<'static>> {
        let data = poly_rt::fs::read(path).ok()?;
        let data: &'static [u8] = alloc::boxed::Box::leak(data.into_boxed_slice());
        poly_font::Font::parse(data).ok()
    };
    let f = |name: &str| load(&poly_rt::fs::join(crate::ui::FONTS, name));
    let mut fonts = poly_shell::draw::Fonts {
        sans: f("Geist-Regular.ttf"),
        medium: f("Geist-Medium.ttf"),
        semibold: f("Geist-SemiBold.ttf"),
        display: f("AveriaSerifLibre-Regular.ttf"),
        mono: f("GeistMono-Regular.ttf"),
        display_bold: f("AveriaSerifLibre-Bold.ttf"),
    };
    if fonts.sans.is_none() {
        fonts.sans = load(crate::ui::FALLBACK_FONT);
    }
    fonts
}

fn now_ms() -> u64 {
    poly_rt::sys::clock(poly_rt::abi::Clock::Monotonic) / 1_000_000
}

fn h(fd: Fd) -> u32 {
    fd.0 as u32
}

/// Send everything the client queued.
fn flush(stream: &UnixStream, client: &mut Client) -> Result<(), String> {
    let (bytes, fds) = client.take_output();
    let handles: Vec<u32> = fds.iter().map(|f| h(*f)).collect();
    let mut at = 0;
    while at < bytes.len() {
        let hs: &[u32] = if at == 0 { &handles } else { &[] };
        match stream.send_with_handles(&bytes[at..], hs) {
            Ok(0) => return Err("the compositor went away".into()),
            Ok(n) => at += n,
            Err(abi::Error::Interrupted) => {}
            Err(e) => return Err(alloc::format!("send: {e}")),
        }
    }
    Ok(())
}

/// Clicks: a second press within 400 ms and 4 pixels is a double click.
#[derive(Debug, Default)]
struct Clicks {
    last: Option<(u32, i32, i32, u64)>,
    count: u32,
}

impl Clicks {
    fn press(&mut self, button: u32, x: i32, y: i32, now: u64) -> u32 {
        self.count = match self.last {
            Some((b, lx, ly, t)) if b == button && now - t <= 400 && (lx - x).abs() <= 4 && (ly - y).abs() <= 4 => {
                self.count + 1
            }
            _ => 1,
        };
        self.last = Some((button, x, y, now));
        self.count
    }
}

/// Run `app` in a window titled `title` (`app_id`), `size` logical pixels
/// to begin with, at least `min`. Returns the exit status.
pub fn run(app: &mut dyn App, title: &str, app_id: &str, size: (i32, i32), min: (i32, i32)) -> i32 {
    let name = poly_rt::env::var("WAYLAND_DISPLAY").unwrap_or("wayland-0");
    let path = if name.starts_with('/') {
        String::from(name)
    } else {
        poly_rt::fs::join(poly_rt::env::var("XDG_RUNTIME_DIR").unwrap_or("/run/user/0"), name)
    };
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            poly_rt::eprintln!("{app_id}: cannot connect to the compositor at {path}: {e}");
            return 1;
        }
    };
    let settings = settings_path();
    let mut look_text = poly_rt::fs::read_to_string(&settings).unwrap_or_default();
    let mut look = Look::parse(&look_text);
    poly_shell::theme::set_light(look.light);
    let mut ui = Ui::new(load_fonts());
    let mut client = Client::new(title, app_id, size, min);
    let mut canvas: Option<Canvas> = None;
    let mut repeat = Repeat::default();
    let mut clicks = Clicks::default();
    let mut full = true;
    let mut dirty: Vec<Rect> = Vec::new();
    let mut want_draw = false;
    // A paste being read: the pipe's read end and what came so far.
    let mut paste: Option<(u32, Vec<u8>)> = None;
    let mut close_after_send: Vec<u32> = Vec::new();
    let mut next_look = now_ms() + 2000;
    let mut buf = alloc::vec![0u8; 64 * 1024];
    let status = 'outer: loop {
        let mut cx = Cx { now_ms: now_ms(), ..Cx::default() };
        if let Err(e) = flush(&stream, &mut client) {
            poly_rt::eprintln!("{app_id}: {e}");
            break 1;
        }
        // The write ends of paste pipes went with their requests.
        for w in close_after_send.drain(..) {
            let _ = sys::close(w);
        }
        // Wait.
        let extra = app.handles();
        let mut entries: Vec<PollEntry> = Vec::with_capacity(2 + extra.len());
        entries.push(poly_rt::poll::entry(stream.handle(), events::READABLE));
        if let Some((r, _)) = &paste {
            entries.push(poly_rt::poll::entry(*r, events::READABLE));
        } else {
            entries.push(poly_rt::poll::entry(abi::NO_HANDLE, 0));
        }
        for (hd, ev) in &extra {
            entries.push(poly_rt::poll::entry(*hd, *ev));
        }
        let now = cx.now_ms;
        let deadline =
            [app.deadline(), repeat.deadline(), Some(next_look)].into_iter().flatten().min().unwrap_or(u64::MAX);
        let timeout = if want_draw && !client.frame_pending() { 0 } else { deadline.saturating_sub(now) };
        let _ = poly_rt::poll::poll(&mut entries, Some(core::time::Duration::from_millis(timeout)));
        cx.now_ms = now_ms();

        // The compositor.
        if entries[0].revents != 0 {
            let mut fds = Vec::new();
            match stream.recv_with_handles(&mut buf, &mut fds) {
                Ok(0) => {
                    poly_rt::eprintln!("{app_id}: the compositor closed the connection");
                    break 1;
                }
                Ok(n) => client.receive(&buf[..n], fds.into_iter().map(|f| Fd(f as i32)).collect()),
                Err(abi::Error::Interrupted | abi::Error::WouldBlock) => {}
                Err(e) => {
                    poly_rt::eprintln!("{app_id}: receive: {e}");
                    break 1;
                }
            }
            for fd in client.take_unused_fds() {
                let _ = sys::close(h(fd));
            }
        }
        while let Some(ev) = client.poll_event() {
            match ev {
                Event::Resized { width, height, scale } => {
                    ui.scale = scale;
                    full = true;
                    want_draw = true;
                    app.event(AppEvent::Resized { width, height, scale }, &mut cx);
                }
                Event::Close => app.close(&mut cx),
                Event::Focus(on) => {
                    if !on {
                        repeat.stop();
                    }
                    app.event(AppEvent::Focus(on), &mut cx);
                }
                Event::Key { code, pressed, depressed, locked } => {
                    let k = input::translate(code, pressed, depressed, locked, look.layout());
                    repeat.rate = client.repeat.0;
                    repeat.delay = client.repeat.1;
                    repeat.key(code, pressed, input::is_modifier(k.key), cx.now_ms);
                    app.event(AppEvent::Key(k), &mut cx);
                }
                Event::PointerMotion { x, y } => app.event(AppEvent::Motion { x: x as i32, y: y as i32 }, &mut cx),
                Event::PointerLeave => app.event(AppEvent::Leave, &mut cx),
                Event::Button { button, pressed, x, y } => {
                    let (x, y) = (x as i32, y as i32);
                    let n = if pressed { clicks.press(button, x, y, cx.now_ms) } else { clicks.count };
                    let mods = input::Mods::from_xkb(client.modifiers().0);
                    app.event(AppEvent::Button { button, pressed, x, y, clicks: n, mods }, &mut cx);
                }
                Event::Scroll { dy } => app.event(AppEvent::Scroll { dy }, &mut cx),
                Event::FrameDone => {}
                Event::SendClipboard { fd, .. } => {
                    if let Some(text) = client.clipboard_text() {
                        let mut at = 0;
                        while at < text.len() {
                            match sys::write(h(fd), &text.as_bytes()[at..]) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => at += n,
                            }
                        }
                    }
                    let _ = sys::close(h(fd));
                }
                Event::ClipboardChanged { .. } => {}
                Event::Failed(why) => {
                    poly_rt::eprintln!("{app_id}: {why}");
                    break 'outer 1;
                }
            }
        }

        // A paste coming in.
        if let Some((r, data)) = &mut paste
            && entries[1].revents != 0
        {
            match sys::read(*r, &mut buf) {
                Ok(n) if n > 0 => data.extend_from_slice(&buf[..n]),
                Err(abi::Error::Interrupted | abi::Error::WouldBlock) => {}
                _ => {
                    let (r, data) = paste.take().unwrap_or_default();
                    let _ = sys::close(r);
                    let text = String::from_utf8_lossy(&data).into_owned();
                    app.event(AppEvent::Paste(text), &mut cx);
                }
            }
        }

        // The app's handles.
        for (i, (hd, _)) in extra.iter().enumerate() {
            let rev = entries[2 + i].revents;
            if rev != 0 {
                app.ready(*hd, rev, &mut cx);
            }
        }

        // Timers: key repeat, the app's, the settings.
        if let Some(code) = repeat.fire(cx.now_ms) {
            let (d, l) = client.modifiers();
            let mut k = input::translate(code, true, d, l, look.layout());
            k.repeat = true;
            app.event(AppEvent::Key(k), &mut cx);
        }
        if app.deadline().is_some_and(|d| d <= cx.now_ms) {
            app.tick(&mut cx);
        }
        if cx.now_ms >= next_look {
            next_look = cx.now_ms + 2000;
            let text = poly_rt::fs::read_to_string(&settings).unwrap_or_default();
            if text != look_text {
                look_text = text;
                let new = Look::parse(&look_text);
                if new.light != look.light {
                    poly_shell::theme::set_light(new.light);
                    full = true;
                    want_draw = true;
                    app.event(AppEvent::Theme, &mut cx);
                }
                look = new;
            }
        }

        // What the app asked for.
        if let Some(text) = cx.clipboard.take() {
            client.set_clipboard(text);
        }
        if core::mem::take(&mut cx.paste) {
            if let Some(own) = client.clipboard_text() {
                // Our own selection (our source was not cancelled): no
                // round trip through the compositor, which would have us
                // write into the pipe we read.
                let text = String::from(own);
                app.event(AppEvent::Paste(text), &mut cx);
            } else if paste.is_none() {
                start_paste(&mut client, &mut paste, &mut close_after_send);
            }
        }
        if let Some(c) = cx.cursor.take() {
            client.set_cursor(c);
        }
        if let Some(t) = cx.title.take() {
            client.set_title(&t);
        }
        if cx.quit {
            break 0;
        }
        if cx.redraw {
            full = true;
            want_draw = true;
        }
        if !cx.damage.is_empty() {
            dirty.append(&mut cx.damage);
            want_draw = true;
        }

        // Draw when the compositor is ready for a frame.
        if want_draw && client.configured() && !client.frame_pending() {
            let (bw, bh) = client.buffer_size();
            if client.needs_buffer() || canvas.is_none() {
                match Canvas::new(bw as u32, bh as u32) {
                    Some(c) => {
                        client.set_buffer(Fd(c.file.handle() as i32), bw, bh);
                        canvas = Some(c);
                        full = true;
                    }
                    None => {
                        poly_rt::eprintln!("{app_id}: no memory for a {bw}x{bh} buffer");
                        break 1;
                    }
                }
            }
            let Some(c) = canvas.as_mut() else { continue };
            let drawn = app.draw(c, &ui, full);
            let s = ui.scale;
            let buffer = Rect::new(0, 0, bw, bh);
            let damage: Vec<Rect> = if full {
                alloc::vec![buffer]
            } else {
                let (lw, lh) = client.size();
                drawn
                    .unwrap_or_else(|| alloc::vec![Rect::new(0, 0, lw, lh)])
                    .into_iter()
                    .chain(dirty.drain(..))
                    .map(|r| s.rect_out(r).intersect(&buffer))
                    .filter(|r| !r.is_empty())
                    .collect()
            };
            dirty.clear();
            client.present(&damage);
            full = false;
            want_draw = false;
        }
    };
    let _ = flush(&stream, &mut client);
    status
}

/// Ask for the clipboard's text into a new pipe; its write end goes with
/// the request and is closed once that is sent.
fn start_paste(client: &mut Client, paste: &mut Option<(u32, Vec<u8>)>, close: &mut Vec<u32>) {
    let Ok([r, w]) = sys::pipe() else { return };
    if client.receive_clipboard(Fd(w as i32)) {
        *paste = Some((r, Vec::new()));
        close.push(w);
    } else {
        let _ = sys::close(r);
        let _ = sys::close(w);
    }
}
