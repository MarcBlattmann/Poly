//! win32k's end of the desktop link (ADR 0128, Decision 3): the surfaces of
//! top-level windows as the compositor knows them.
//!
//! win32k tells [`Desktop`] what happens to its surfaces (created, moved,
//! titled, shown, a new buffer, pixels drawn, destroyed); `Desktop` keeps
//! that state and, while a compositor is connected, turns each change into
//! a [`ToCompositor`] record for the device to hand out. When the
//! compositor (re)connects with `Hello`, `Desktop` answers `Welcome` and
//! replays every surface, so a restarted compositor shows the same
//! windows; while none is connected, windows live on unseen (headless
//! Windows apps in tests). The compositor's records come back through
//! [`Desktop::receive`]: the greeting and buffer releases are handled
//! here, everything else (input, focus, moves, the shell's requests) is
//! returned for win32k.
//!
//! Buffers: a surface has one current buffer; one it replaced stays in use
//! until the compositor reports it released (or disconnects), and then
//! [`Desktop::take_released`] names it, so the kernel can free its memory.

use crate::frame::FrameMetrics;
use crate::link::{
    BufferId, CursorShape, Edges, Error, FromCompositor, PixelFormat, Record, Stream, SurfaceId, SurfaceKind,
    ToCompositor, VERSION, WindowState,
};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

/// A surface's current buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferInfo {
    pub id: BufferId,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}

/// One surface as the compositor is told about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    pub kind: SurfaceKind,
    pub owner: SurfaceId,
    pub framed: bool,
    /// On the screen: x, y, width, height (logical pixels).
    pub rect: (i32, i32, u32, u32),
    pub title: String,
    pub shown: bool,
    pub buffer: Option<BufferInfo>,
    pub state: WindowState,
    /// The process of the window's app (0: not told).
    pub pid: u32,
}

/// The output as the compositor reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
}

/// win32k's end of the link.
#[derive(Debug)]
pub struct Desktop {
    surfaces: BTreeMap<SurfaceId, Surface>,
    next_surface: SurfaceId,
    next_buffer: BufferId,
    connected: bool,
    frame: FrameMetrics,
    output: Option<Output>,
    layout: Option<String>,
    out: Vec<u8>,
    incoming: Stream<FromCompositor>,
    /// Replaced buffers the compositor may still read.
    retired: BTreeSet<BufferId>,
    /// Buffers nobody reads any more, for the kernel to free.
    released: Vec<BufferId>,
    errors: u32,
}

impl Default for Desktop {
    fn default() -> Self {
        Desktop::new()
    }
}

impl Desktop {
    pub fn new() -> Desktop {
        Desktop {
            surfaces: BTreeMap::new(),
            next_surface: 1,
            next_buffer: 1,
            connected: false,
            frame: FrameMetrics::POLY,
            output: None,
            layout: None,
            out: Vec::new(),
            incoming: Stream::new(),
            retired: BTreeSet::new(),
            released: Vec::new(),
            errors: 0,
        }
    }

    /// Whether a compositor has greeted us.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Poly's frame (the compositor's numbers once it greeted us).
    pub fn frame(&self) -> FrameMetrics {
        self.frame
    }

    /// The output, once a compositor reported it.
    pub fn output(&self) -> Option<Output> {
        self.output
    }

    /// The keyboard layout the shell's settings name, once known.
    pub fn layout(&self) -> Option<&str> {
        self.layout.as_deref()
    }

    /// Records the compositor sent that were malformed.
    pub fn errors(&self) -> u32 {
        self.errors
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&Surface> {
        self.surfaces.get(&id)
    }

    pub fn surfaces(&self) -> impl Iterator<Item = (SurfaceId, &Surface)> {
        self.surfaces.iter().map(|(&id, s)| (id, s))
    }

    fn send(&mut self, r: ToCompositor) {
        if self.connected {
            r.encode(&mut self.out);
        }
    }

    /// A top-level window got a surface (hidden, empty, untitled).
    pub fn create(&mut self, kind: SurfaceKind, owner: SurfaceId, framed: bool) -> SurfaceId {
        let id = self.next_surface;
        self.next_surface += 1;
        let framed = framed && kind == SurfaceKind::TopLevel;
        let owner = if self.surfaces.contains_key(&owner) { owner } else { 0 };
        self.surfaces.insert(
            id,
            Surface {
                kind,
                owner,
                framed,
                rect: (0, 0, 0, 0),
                title: String::new(),
                shown: false,
                buffer: None,
                state: WindowState::Normal,
                pid: 0,
            },
        );
        self.send(ToCompositor::Create { surface: id, kind, owner, framed });
        id
    }

    fn with(&mut self, id: SurfaceId, f: impl FnOnce(&mut Surface) -> Option<ToCompositor>) -> bool {
        let Some(s) = self.surfaces.get_mut(&id) else { return false };
        if let Some(r) = f(s) {
            self.send(r);
        }
        true
    }

    /// The surface's rectangle on the screen changed.
    pub fn set_geometry(&mut self, id: SurfaceId, x: i32, y: i32, width: u32, height: u32) -> bool {
        self.with(id, |s| {
            (s.rect != (x, y, width, height)).then(|| {
                s.rect = (x, y, width, height);
                ToCompositor::Geometry { surface: id, x, y, width, height }
            })
        })
    }

    /// The process whose window the surface is; the compositor uses it to
    /// end the busy pointer of the app it launched (ADR 0167).
    pub fn set_process(&mut self, id: SurfaceId, pid: u32) -> bool {
        self.with(id, |s| {
            (pid != 0 && s.pid != pid).then(|| {
                s.pid = pid;
                ToCompositor::Process { surface: id, pid }
            })
        })
    }

    pub fn set_title(&mut self, id: SurfaceId, text: &str) -> bool {
        self.with(id, |s| {
            (s.title != text).then(|| {
                s.title = String::from(text);
                ToCompositor::Title { surface: id, text: String::from(text) }
            })
        })
    }

    pub fn set_visible(&mut self, id: SurfaceId, shown: bool) -> bool {
        self.with(id, |s| {
            (s.shown != shown).then(|| {
                s.shown = shown;
                ToCompositor::Visible { surface: id, shown }
            })
        })
    }

    pub fn set_state(&mut self, id: SurfaceId, state: WindowState) -> bool {
        self.with(id, |s| {
            (s.state != state).then(|| {
                s.state = state;
                ToCompositor::State { surface: id, state }
            })
        })
    }

    /// A new buffer for the surface; the one it replaces is retired until
    /// the compositor releases it. `None` for an unknown surface.
    pub fn attach_buffer(
        &mut self,
        id: SurfaceId,
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Option<BufferId> {
        if !self.surfaces.contains_key(&id) {
            return None;
        }
        let buffer = self.next_buffer;
        self.next_buffer += 1;
        let info = BufferInfo { id: buffer, width, height, stride, format };
        let old = self.surfaces.get_mut(&id)?.buffer.replace(info);
        if let Some(old) = old {
            self.retire(old.id);
        }
        self.send(ToCompositor::Buffer { surface: id, buffer, width, height, stride, format });
        Some(buffer)
    }

    fn retire(&mut self, buffer: BufferId) {
        if self.connected {
            self.retired.insert(buffer);
        } else {
            self.released.push(buffer);
        }
    }

    /// Pixels of the current buffer changed (clipped to it; nothing is sent
    /// for an empty rectangle or a surface without a buffer).
    pub fn damage(&mut self, id: SurfaceId, x: u32, y: u32, width: u32, height: u32) -> bool {
        self.with(id, |s| {
            let b = s.buffer?;
            let (x, y) = (x.min(b.width), y.min(b.height));
            let width = width.min(b.width - x);
            let height = height.min(b.height - y);
            (width > 0 && height > 0).then_some(ToCompositor::Damage { surface: id, x, y, width, height })
        })
    }

    /// The app asks to be the foreground window.
    pub fn request_activate(&mut self, id: SurfaceId) -> bool {
        self.with(id, |_| Some(ToCompositor::Activate { surface: id }))
    }

    pub fn set_cursor(&mut self, id: SurfaceId, shape: CursorShape) -> bool {
        self.with(id, |_| Some(ToCompositor::Cursor { surface: id, shape }))
    }

    pub fn begin_move(&mut self, id: SurfaceId) -> bool {
        self.with(id, |_| Some(ToCompositor::BeginMove { surface: id }))
    }

    pub fn begin_resize(&mut self, id: SurfaceId, edges: Edges) -> bool {
        self.with(id, |_| Some(ToCompositor::BeginResize { surface: id, edges }))
    }

    /// The surface is gone; its buffers go once the compositor lets go of
    /// them (at once without a compositor). Surfaces it owned lose their
    /// owner.
    pub fn destroy(&mut self, id: SurfaceId) -> bool {
        let Some(s) = self.surfaces.remove(&id) else { return false };
        if let Some(b) = s.buffer {
            self.retire(b.id);
        }
        for other in self.surfaces.values_mut() {
            if other.owner == id {
                other.owner = 0;
            }
        }
        self.send(ToCompositor::Destroy { surface: id });
        true
    }

    /// The bytes to hand to the compositor's next `read`.
    pub fn take_output(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    /// Buffers nobody reads any more.
    pub fn take_released(&mut self) -> Vec<BufferId> {
        core::mem::take(&mut self.released)
    }

    /// The compositor closed the device: forget the connection (and what
    /// it had not read yet); retired buffers are free.
    pub fn disconnect(&mut self) {
        self.connected = false;
        self.out.clear();
        self.incoming = Stream::new();
        let retired = core::mem::take(&mut self.retired);
        self.released.extend(retired);
    }

    /// Bytes the compositor wrote. Returns what is for win32k: input,
    /// focus, moves, the shell's requests, output and layout changes (also
    /// kept here). A record of a newer link version that this side does not
    /// know is skipped; malformed ones are counted.
    pub fn receive(&mut self, bytes: &[u8]) -> Vec<FromCompositor> {
        self.incoming.push(bytes);
        let mut events = Vec::new();
        while let Some(r) = self.incoming.next_record() {
            match r {
                Ok(FromCompositor::Hello { version: _, width, height, scale, frame }) => {
                    self.output = Some(Output { width, height, scale });
                    self.frame = frame;
                    self.welcome();
                }
                Ok(_) if !self.connected => self.errors += 1,
                Ok(FromCompositor::Released { buffer }) => {
                    if self.retired.remove(&buffer) {
                        self.released.push(buffer);
                    }
                }
                Ok(r) => {
                    match &r {
                        FromCompositor::Output { width, height, scale } => {
                            self.output = Some(Output { width: *width, height: *height, scale: *scale });
                        }
                        FromCompositor::Layout { name } => self.layout = Some(name.clone()),
                        FromCompositor::Configure { surface, x, y, width, height } => {
                            if let Some(s) = self.surfaces.get_mut(surface) {
                                s.rect = (*x, *y, *width, *height);
                            }
                        }
                        _ => {}
                    }
                    events.push(r);
                }
                Err(Error::UnknownKind(_)) => {}
                Err(_) => self.errors += 1,
            }
        }
        events
    }

    /// `Welcome`, then every surface as it is now. Replaced buffers of an
    /// earlier connection are free: the new compositor never saw them.
    fn welcome(&mut self) {
        self.connected = true;
        self.out.clear();
        let retired = core::mem::take(&mut self.retired);
        self.released.extend(retired);
        let mut out = Vec::new();
        ToCompositor::Welcome { version: VERSION }.encode(&mut out);
        for (&id, s) in &self.surfaces {
            let mut put = |r: ToCompositor| r.encode(&mut out);
            put(ToCompositor::Create { surface: id, kind: s.kind, owner: s.owner, framed: s.framed });
            if s.pid != 0 {
                put(ToCompositor::Process { surface: id, pid: s.pid });
            }
            let (x, y, width, height) = s.rect;
            put(ToCompositor::Geometry { surface: id, x, y, width, height });
            if !s.title.is_empty() {
                put(ToCompositor::Title { surface: id, text: s.title.clone() });
            }
            if s.state != WindowState::Normal {
                put(ToCompositor::State { surface: id, state: s.state });
            }
            if let Some(b) = s.buffer {
                put(ToCompositor::Buffer {
                    surface: id,
                    buffer: b.id,
                    width: b.width,
                    height: b.height,
                    stride: b.stride,
                    format: b.format,
                });
                if b.width > 0 && b.height > 0 {
                    put(ToCompositor::Damage { surface: id, x: 0, y: 0, width: b.width, height: b.height });
                }
            }
            if s.shown {
                put(ToCompositor::Visible { surface: id, shown: true });
            }
        }
        self.out = out;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::link::{Request, encode_all};
    use alloc::vec;

    fn hello() -> Vec<u8> {
        encode_all(&[FromCompositor::Hello {
            version: VERSION,
            width: 1280,
            height: 800,
            scale: 1,
            frame: FrameMetrics::POLY,
        }])
    }

    fn records(bytes: &[u8]) -> Vec<ToCompositor> {
        let mut s = Stream::<ToCompositor>::new();
        s.push(bytes);
        s.map(Result::unwrap).collect()
    }

    #[test]
    fn a_window_becomes_a_surface_of_the_compositor() {
        let mut d = Desktop::new();
        assert!(d.receive(&hello()).is_empty());
        assert!(d.is_connected());
        assert_eq!(records(&d.take_output()), vec![ToCompositor::Welcome { version: VERSION }]);

        let s = d.create(SurfaceKind::TopLevel, 0, true);
        d.set_geometry(s, 100, 125, 320, 200);
        d.set_title(s, "Hello");
        let b = d.attach_buffer(s, 320, 200, 1280, PixelFormat::Xrgb8888).unwrap();
        d.damage(s, 0, 0, 320, 200);
        d.set_visible(s, true);
        // Repeating a state sends nothing.
        d.set_visible(s, true);
        d.set_title(s, "Hello");
        // Damage is clipped to the buffer.
        d.damage(s, 300, 190, 100, 100);
        d.damage(s, 400, 0, 10, 10);
        d.destroy(s);
        assert_eq!(
            records(&d.take_output()),
            vec![
                ToCompositor::Create { surface: s, kind: SurfaceKind::TopLevel, owner: 0, framed: true },
                ToCompositor::Geometry { surface: s, x: 100, y: 125, width: 320, height: 200 },
                ToCompositor::Title { surface: s, text: String::from("Hello") },
                ToCompositor::Buffer {
                    surface: s,
                    buffer: b,
                    width: 320,
                    height: 200,
                    stride: 1280,
                    format: PixelFormat::Xrgb8888
                },
                ToCompositor::Damage { surface: s, x: 0, y: 0, width: 320, height: 200 },
                ToCompositor::Visible { surface: s, shown: true },
                ToCompositor::Damage { surface: s, x: 300, y: 190, width: 20, height: 10 },
                ToCompositor::Destroy { surface: s },
            ]
        );
        // The destroyed surface's buffer waits for the compositor.
        assert!(d.take_released().is_empty());
        d.receive(&encode_all(&[FromCompositor::Released { buffer: b }]));
        assert_eq!(d.take_released(), vec![b]);
    }

    #[test]
    fn popups_are_never_framed_and_owners_must_exist() {
        let mut d = Desktop::new();
        let top = d.create(SurfaceKind::TopLevel, 0, true);
        let menu = d.create(SurfaceKind::Popup, top, true);
        let stray = d.create(SurfaceKind::Popup, 99, false);
        assert!(!d.surface(menu).unwrap().framed);
        assert_eq!(d.surface(menu).unwrap().owner, top);
        assert_eq!(d.surface(stray).unwrap().owner, 0);
        d.destroy(top);
        assert_eq!(d.surface(menu).unwrap().owner, 0);
    }

    #[test]
    fn headless_until_a_compositor_greets_then_everything_is_replayed() {
        let mut d = Desktop::new();
        let s = d.create(SurfaceKind::TopLevel, 0, true);
        d.set_geometry(s, 10, 35, 100, 50);
        d.set_title(s, "Headless");
        let b1 = d.attach_buffer(s, 100, 50, 400, PixelFormat::Xrgb8888).unwrap();
        let b2 = d.attach_buffer(s, 100, 50, 400, PixelFormat::Xrgb8888).unwrap();
        d.set_visible(s, true);
        let hidden = d.create(SurfaceKind::Popup, s, false);
        // Nothing goes out without a compositor; a replaced buffer is free at once.
        assert!(d.take_output().is_empty());
        assert_eq!(d.take_released(), vec![b1]);
        // Input before the greeting is an error, not an event.
        assert!(d.receive(&encode_all(&[FromCompositor::Focus { surface: s }])).is_empty());
        assert_eq!(d.errors(), 1);

        d.receive(&hello());
        assert_eq!(
            records(&d.take_output()),
            vec![
                ToCompositor::Welcome { version: VERSION },
                ToCompositor::Create { surface: s, kind: SurfaceKind::TopLevel, owner: 0, framed: true },
                ToCompositor::Geometry { surface: s, x: 10, y: 35, width: 100, height: 50 },
                ToCompositor::Title { surface: s, text: String::from("Headless") },
                ToCompositor::Buffer {
                    surface: s,
                    buffer: b2,
                    width: 100,
                    height: 50,
                    stride: 400,
                    format: PixelFormat::Xrgb8888
                },
                ToCompositor::Damage { surface: s, x: 0, y: 0, width: 100, height: 50 },
                ToCompositor::Visible { surface: s, shown: true },
                ToCompositor::Create { surface: hidden, kind: SurfaceKind::Popup, owner: s, framed: false },
                ToCompositor::Geometry { surface: hidden, x: 0, y: 0, width: 0, height: 0 },
            ]
        );

        // A buffer replaced while connected is retired until released; a
        // compositor that goes away frees it, and a new one is replayed to.
        let b3 = d.attach_buffer(s, 120, 60, 480, PixelFormat::Argb8888).unwrap();
        d.take_output();
        assert!(d.take_released().is_empty());
        d.disconnect();
        assert_eq!(d.take_released(), vec![b2]);
        d.set_title(s, "Renamed");
        assert!(d.take_output().is_empty());
        d.receive(&hello());
        let replay = records(&d.take_output());
        assert!(replay.contains(&ToCompositor::Title { surface: s, text: String::from("Renamed") }));
        assert!(replay.iter().any(|r| matches!(r, ToCompositor::Buffer { buffer, .. } if *buffer == b3)));
    }

    #[test]
    fn the_process_of_a_window_is_told_once_and_replayed() {
        let mut d = Desktop::new();
        d.receive(&hello());
        d.take_output();
        let s = d.create(SurfaceKind::TopLevel, 0, true);
        // Unknown (0) says nothing; a process is told once.
        d.set_process(s, 0);
        d.set_process(s, 31);
        d.set_process(s, 31);
        assert_eq!(
            records(&d.take_output()),
            vec![
                ToCompositor::Create { surface: s, kind: SurfaceKind::TopLevel, owner: 0, framed: true },
                ToCompositor::Process { surface: s, pid: 31 },
            ]
        );
        d.disconnect();
        d.receive(&hello());
        assert!(records(&d.take_output()).contains(&ToCompositor::Process { surface: s, pid: 31 }));
    }

    #[test]
    fn the_compositors_records_reach_win32k() {
        let mut d = Desktop::new();
        let s = d.create(SurfaceKind::TopLevel, 0, true);
        let mut bytes = hello();
        bytes.extend(encode_all(&[
            FromCompositor::Configure { surface: s, x: 0, y: 25, width: 1280, height: 775 },
            FromCompositor::Pointer { surface: s, x: 5, y: -10, time: 1 },
            FromCompositor::Request { surface: s, request: Request::Close },
            FromCompositor::Layout { name: String::from("de") },
            FromCompositor::Output { width: 1920, height: 1080, scale: 2 },
        ]));
        // In two pieces, split inside a record.
        let (a, b) = bytes.split_at(bytes.len() - 7);
        let mut events = d.receive(a);
        events.extend(d.receive(b));
        assert_eq!(events.len(), 5);
        assert_eq!(d.surface(s).unwrap().rect, (0, 25, 1280, 775));
        assert_eq!(d.layout(), Some("de"));
        assert_eq!(d.output(), Some(Output { width: 1920, height: 1080, scale: 2 }));
        assert_eq!(events[2], FromCompositor::Request { surface: s, request: Request::Close });
        assert_eq!(d.frame(), FrameMetrics::POLY);
        assert_eq!(d.errors(), 0);
    }
}
