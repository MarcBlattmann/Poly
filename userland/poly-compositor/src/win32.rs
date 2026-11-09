//! The compositor's end of the desktop link (ADR 0128, Decision 3): Windows
//! top-level windows as scene windows.
//!
//! win32k (the NT personality) describes each shown top-level window of a
//! Windows app as a surface in [`ToCompositor`] records read from
//! [`poly_win32k::link::DEVICE`]. [`WindowsSurfaces::apply`] turns them into
//! the scene: a surface is a scene window at the position win32k gives
//! (Windows apps place their windows themselves), framed by Poly when it is
//! an application window with a caption (Poly's frame *is* its non-client
//! caption, [`poly_win32k::frame`]), popups unframed above their owner. Its
//! pixels are copied from the shared buffer when win32k reports damage, so
//! the scene never reads app memory while composing (ADR 0106). What is not
//! the scene's (taking a buffer's handle, activation, cursors, interactive
//! moves, the records to send back) comes back as [`Action`]s for the
//! compositor's loop.
//!
//! This is the part that needs no device, tested on the host; [`Link`]
//! is the device end in the compositor's loop (ADR 0131): it opens
//! [`DEVICE`], greets win32k, reads records, keeps a copy of each buffer
//! whose damaged rows it fetches with the device's
//! [`READ_BUFFER`](poly_win32k::link::READ_BUFFER) request before the
//! scene copies them ([`Copies`]), and writes back what the scene's end
//! answers (buffer releases).

use crate::os;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use poly_wayland::compose::{Content, Rect, Scene, WindowId};
use poly_win32k::frame::FrameMetrics;
use poly_win32k::link::{
    BufferId, CursorShape, DEVICE, Edges, FromCompositor, PixelFormat, READ_BUFFER, ReadBuffer, Record, Stream,
    SurfaceId, SurfaceKind, ToCompositor, VERSION, WindowState,
};

/// The scene owner of Windows surfaces: this bit, or'ed with the surface,
/// keeps them apart from Wayland clients' windows.
pub const OWNER_TAG: u64 = 1 << 62;

/// The mapped bytes of the buffers win32k handed over.
pub trait Buffers {
    /// Buffer `id`'s bytes, if it is mapped.
    fn bytes(&self, id: BufferId) -> Option<&[u8]>;
}

/// What the compositor's loop must do besides the scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Take buffer `id`'s handle from the device and map it.
    TakeBuffer(BufferId),
    /// Unmap buffer `id`; [`Action::Send`] tells win32k.
    DropBuffer(BufferId),
    /// Write this record to the device.
    Send(FromCompositor),
    /// The app asks to be active; the window management decides.
    Activate(WindowId),
    /// The cursor to show over the window.
    Cursor(WindowId, CursorShape),
    /// Start an interactive move or resize of the window.
    BeginMove(WindowId),
    BeginResize(WindowId, Edges),
}

#[derive(Clone, Copy, Debug)]
struct Surface {
    window: WindowId,
    buffer: Option<(BufferId, u32, u32, u32, PixelFormat)>,
    geometry: (i32, i32, u32, u32),
    /// The process of the app (0: not told), to end its startup feedback
    /// when its first window is shown (ADR 0167).
    pid: u32,
}

/// Windows surfaces in the scene.
#[derive(Debug, Default)]
pub struct WindowsSurfaces {
    surfaces: BTreeMap<SurfaceId, Surface>,
}

impl WindowsSurfaces {
    pub fn new() -> WindowsSurfaces {
        WindowsSurfaces::default()
    }

    /// The scene window showing `surface`.
    pub fn window(&self, surface: SurfaceId) -> Option<WindowId> {
        self.surfaces.get(&surface).map(|s| s.window)
    }

    /// The current buffer of `surface`: id, width, height, stride.
    pub fn buffer_of(&self, surface: SurfaceId) -> Option<(BufferId, u32, u32, u32)> {
        let (b, w, h, stride, _) = self.surfaces.get(&surface)?.buffer?;
        Some((b, w, h, stride))
    }

    /// The process whose window the scene window is, if win32k said.
    pub fn process_of(&self, window: WindowId) -> Option<u32> {
        self.surfaces.values().find(|s| s.window == window).map(|s| s.pid).filter(|&p| p != 0)
    }

    /// The surface a scene window shows, if it is a Windows one.
    pub fn surface_of(&self, window: WindowId) -> Option<SurfaceId> {
        self.surfaces.iter().find(|(_, s)| s.window == window).map(|(&id, _)| id)
    }

    /// Configure records for Windows surfaces moved or resized in the
    /// scene. Their cached geometry suppresses duplicate records.
    pub fn sync_geometry(&mut self, scene: &Scene) -> Vec<FromCompositor> {
        let mut moved = Vec::new();
        for (&surface, s) in &self.surfaces {
            let Some(win) = scene.window(s.window) else { continue };
            let (x, y) = win.position();
            let (width, height) = win.content().logical_size();
            let geometry = (x, y, width.max(0) as u32, height.max(0) as u32);
            if s.geometry != geometry {
                moved.push((surface, geometry));
            }
        }
        for (surface, (x, y, width, height)) in &moved {
            if let Some(s) = self.surfaces.get_mut(surface) {
                s.geometry = (*x, *y, *width, *height);
            }
        }
        moved
            .into_iter()
            .map(|(surface, (x, y, width, height))| FromCompositor::Configure { surface, x, y, width, height })
            .collect()
    }

    /// win32k's connection ended: its windows leave the scene.
    pub fn clear(&mut self, scene: &mut Scene) -> Vec<Action> {
        let mut actions = Vec::new();
        for (_, s) in core::mem::take(&mut self.surfaces) {
            scene.remove_window(s.window);
            if let Some((b, ..)) = s.buffer {
                actions.push(Action::DropBuffer(b));
            }
        }
        actions
    }

    /// Apply one record to the scene.
    pub fn apply(&mut self, scene: &mut Scene, record: &ToCompositor, buffers: &dyn Buffers) -> Vec<Action> {
        let mut actions = Vec::new();
        match *record {
            ToCompositor::Welcome { .. } => {
                // A (re)connection: the replay that follows rebuilds everything.
                actions = self.clear(scene);
            }
            ToCompositor::Create { surface, kind, owner, framed } => {
                if let Some(old) = self.surfaces.remove(&surface) {
                    scene.remove_window(old.window);
                }
                let framed = framed && kind == SurfaceKind::TopLevel;
                let window = scene.add_window(OWNER_TAG | u64::from(surface), framed);
                // Not shown until win32k says so.
                scene.set_hidden(window, true);
                if let Some(owner) = self.window(owner) {
                    scene.set_parent(window, Some(owner));
                }
                self.surfaces.insert(surface, Surface { window, buffer: None, geometry: (0, 0, 0, 0), pid: 0 });
            }
            ToCompositor::Geometry { surface, x, y, width, height } => {
                if let Some(w) = self.window(surface) {
                    scene.set_position(w, x, y);
                }
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    s.geometry = (x, y, width, height);
                }
            }
            ToCompositor::Process { surface, pid } => {
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    s.pid = pid;
                }
            }
            ToCompositor::Title { surface, ref text } => {
                if let Some(w) = self.window(surface) {
                    scene.set_title(w, text);
                }
            }
            ToCompositor::Visible { surface, shown } => {
                if let Some(w) = self.window(surface) {
                    scene.set_hidden(w, !shown);
                }
            }
            ToCompositor::Buffer { surface, buffer, width, height, stride, format } => {
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    if let Some((old, ..)) = s.buffer.replace((buffer, width, height, stride, format)) {
                        actions.push(Action::DropBuffer(old));
                        actions.push(Action::Send(FromCompositor::Released { buffer: old }));
                    }
                    scene.resize_content(s.window, width, height, 1);
                    actions.push(Action::TakeBuffer(buffer));
                }
            }
            ToCompositor::Damage { surface, x, y, width, height } => {
                if let Some(&Surface { window, buffer: Some((b, bw, bh, stride, format)), .. }) =
                    self.surfaces.get(&surface)
                    && let Some(bytes) = buffers.bytes(b)
                {
                    copy(scene, window, bytes, (bw, bh, stride, format), (x, y, width, height));
                }
            }
            ToCompositor::Activate { surface } => {
                if let Some(w) = self.window(surface) {
                    actions.push(Action::Activate(w));
                }
            }
            ToCompositor::Cursor { surface, shape } => {
                if let Some(w) = self.window(surface) {
                    actions.push(Action::Cursor(w, shape));
                }
            }
            ToCompositor::BeginMove { surface } => {
                if let Some(w) = self.window(surface) {
                    actions.push(Action::BeginMove(w));
                }
            }
            ToCompositor::BeginResize { surface, edges } => {
                if let Some(w) = self.window(surface) {
                    actions.push(Action::BeginResize(w, edges));
                }
            }
            ToCompositor::State { surface, state } => {
                if let Some(w) = self.window(surface) {
                    scene.set_maximized(w, state == WindowState::Maximized);
                }
            }
            ToCompositor::Destroy { surface } => {
                if let Some(s) = self.surfaces.remove(&surface) {
                    scene.remove_window(s.window);
                    if let Some((b, ..)) = s.buffer {
                        actions.push(Action::DropBuffer(b));
                        actions.push(Action::Send(FromCompositor::Released { buffer: b }));
                    }
                }
            }
        }
        actions
    }
}

/// Copy rectangle `r` (buffer pixels) of a mapped buffer into the window's
/// content: `B, G, R, A` bytes read as little-endian `0xAARRGGBB`, made
/// opaque for `Xrgb8888`, premultiplied already for `Argb8888`.
fn copy(
    scene: &mut Scene,
    window: WindowId,
    bytes: &[u8],
    (bw, bh, stride, format): (u32, u32, u32, PixelFormat),
    (x, y, width, height): (u32, u32, u32, u32),
) {
    let x1 = x.saturating_add(width).min(bw);
    let y1 = y.saturating_add(height).min(bh);
    let Some(content) = scene.content_mut(window) else { return };
    if content.width != bw || content.height != bh {
        return;
    }
    let opaque = if format == PixelFormat::Xrgb8888 { 0xFF00_0000 } else { 0 };
    for row in y..y1 {
        for col in x..x1 {
            let at = row as usize * stride as usize + col as usize * 4;
            let Some(px) = bytes.get(at..at + 4) else { return };
            let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]) | opaque;
            content.pixels[row as usize * bw as usize + col as usize] = v;
        }
    }
    let damaged = Rect::new(x as i32, y as i32, x1.saturating_sub(x) as i32, y1.saturating_sub(y) as i32);
    scene.damage_content(window, damaged);
}

/// The compositor's copies of win32k's buffers, `stride` x `height` bytes
/// each, refreshed from the device before the scene reads them.
#[derive(Debug, Default)]
pub struct Copies(pub BTreeMap<BufferId, Vec<u8>>);

impl Buffers for Copies {
    fn bytes(&self, id: BufferId) -> Option<&[u8]> {
        self.0.get(&id).map(Vec::as_slice)
    }
}

/// The most bytes one `READ_BUFFER` copies (the kernel's limit).
const READ_CHUNK: usize = 512 * 1024;

/// The desktop link's device in the compositor's loop.
#[derive(Debug)]
pub struct Link {
    fd: os::RawFd,
    surfaces: WindowsSurfaces,
    copies: Copies,
    incoming: Stream<ToCompositor>,
}

impl Link {
    /// Open [`DEVICE`] and greet win32k with the output and Poly's frame;
    /// `None` (logged) without the device.
    pub fn open(scene: &Scene) -> Option<Link> {
        let fd = match os::open_device(DEVICE, true, true) {
            Ok(fd) => fd,
            Err(e) => {
                println!("poly-compositor: no desktop link to Windows apps ({e})");
                return None;
            }
        };
        let (width, height) = scene.logical_size();
        let (_, _, scale) = scene.output();
        let hello = FromCompositor::Hello {
            version: VERSION,
            width: width as u32,
            height: height as u32,
            scale,
            frame: FrameMetrics::POLY,
        };
        let mut out = Vec::new();
        hello.encode(&mut out);
        if let Err(e) = os::write(fd, &out) {
            println!("poly-compositor: {DEVICE}: {e}");
            return None;
        }
        println!("poly-compositor: desktop link to Windows apps on {DEVICE}");
        Some(Link { fd, surfaces: WindowsSurfaces::new(), copies: Copies::default(), incoming: Stream::new() })
    }

    /// The device, to poll for reading.
    pub fn fd(&self) -> os::RawFd {
        self.fd
    }

    /// The surfaces in the scene.
    pub fn surfaces(&self) -> &WindowsSurfaces {
        &self.surfaces
    }

    /// Input for a Windows surface's scene window (the server's internal
    /// events of adopted windows): forwarded to win32k. False when the
    /// event is not about one of ours.
    pub fn input(&mut self, ev: &poly_wayland::server::InternalEvent) -> bool {
        use poly_wayland::server::InternalEvent as E;
        let window = match *ev {
            E::Motion { window, .. }
            | E::Button { window, .. }
            | E::Axis { window, .. }
            | E::Key { window, .. }
            | E::Resize { window, .. } => window,
            E::Leave(window) | E::Close(window) => window,
        };
        let Some(surface) = self.surfaces.surface_of(window) else { return false };
        let time = (os::monotonic_ns() / 1_000_000) as u32;
        let records: Vec<FromCompositor> = match *ev {
            E::Motion { x, y, .. } => vec![FromCompositor::Pointer { surface, x, y, time }],
            E::Button { x, y, button, pressed, .. } => vec![
                FromCompositor::Pointer { surface, x, y, time },
                FromCompositor::Button { surface, button: button as u16, pressed, time },
            ],
            E::Axis { horizontal, value, .. } => {
                // 15 logical pixels a step, WHEEL_DELTA (120) a notch, up positive.
                let notches = -value * 120 / 15;
                vec![FromCompositor::Axis {
                    surface,
                    vertical: if horizontal { 0 } else { notches },
                    horizontal: if horizontal { notches } else { 0 },
                    time,
                }]
            }
            E::Key { key, pressed, .. } => vec![FromCompositor::Key { code: key as u16, pressed, time }],
            E::Leave(_) => vec![FromCompositor::Leave { surface }],
            E::Close(_) => vec![FromCompositor::Request { surface, request: poly_win32k::link::Request::Close }],
            E::Resize { .. } => Vec::new(),
        };
        let mut out = Vec::new();
        for r in &records {
            r.encode(&mut out);
        }
        if !out.is_empty()
            && let Err(e) = os::write(self.fd, &out)
        {
            eprintln!("poly-compositor: {DEVICE}: {e}");
        }
        true
    }

    /// Read what win32k sent and apply it to the scene, adopting shown
    /// Windows windows into the server's focus and input as internal
    /// windows.
    pub fn pump_server(&mut self, server: &mut poly_wayland::server::Server) {
        let before: Vec<WindowId> = self.surfaces.surfaces.values().map(|s| s.window).collect();
        self.pump(server.scene_mut());
        let after: Vec<(WindowId, bool)> = self
            .surfaces
            .surfaces
            .values()
            .map(|s| (s.window, server.scene().window(s.window).is_some_and(|w| w.is_visible())))
            .collect();
        for &(w, visible) in &after {
            if visible && !server.is_internal(w) {
                server.adopt_internal_window(w, "windows", true);
                // Its app has a window: the busy pointer of its launch ends.
                if let Some(pid) = self.surfaces.process_of(w) {
                    server.launch_window_mapped(pid);
                }
            }
        }
        for w in before {
            if !after.iter().any(|a| a.0 == w) {
                server.release_internal_window(w);
            }
        }
    }

    /// Tell win32k where the compositor moved or resized a Windows
    /// surface. This keeps the app's GetWindowRect and later input in
    /// step with Poly's frame. (Closing needs no path of its own: the
    /// server reports a close of an adopted window as an internal event,
    /// which [`Link::input`] forwards.)
    pub fn sync_geometry(&mut self, scene: &Scene) {
        for record in self.surfaces.sync_geometry(scene) {
            self.send(record);
        }
    }

    fn send(&mut self, record: FromCompositor) {
        let mut bytes = Vec::new();
        record.encode(&mut bytes);
        if let Err(e) = os::write(self.fd, &bytes) {
            eprintln!("poly-compositor: {DEVICE}: {e}");
        }
    }

    /// Read what win32k sent and apply it to the scene.
    pub fn pump(&mut self, scene: &mut Scene) {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match os::read(self.fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => self.incoming.push(&buf[..n]),
                Err(e) => {
                    eprintln!("poly-compositor: {DEVICE}: {e}");
                    break;
                }
            }
        }
        let mut out = Vec::new();
        while let Some(r) = self.incoming.next_record() {
            let Ok(r) = r else { continue };
            match r {
                ToCompositor::Buffer { buffer, height, stride, .. } => {
                    self.copies.0.insert(buffer, vec![0u8; stride as usize * height as usize]);
                }
                ToCompositor::Damage { surface, y, height, .. } => self.fetch(surface, y, height),
                _ => {}
            }
            for a in self.surfaces.apply(scene, &r, &self.copies) {
                match a {
                    Action::DropBuffer(b) => {
                        self.copies.0.remove(&b);
                    }
                    Action::Send(rec) => rec.encode(&mut out),
                    Action::Activate(w) => scene.raise(w),
                    // The copy was made with the buffer record; input,
                    // cursors and interactive moves come with a later slice.
                    Action::TakeBuffer(_) | Action::Cursor(..) | Action::BeginMove(_) | Action::BeginResize(..) => {}
                }
            }
        }
        if !out.is_empty()
            && let Err(e) = os::write(self.fd, &out)
        {
            eprintln!("poly-compositor: {DEVICE}: {e}");
        }
    }

    /// Fetch rows `y .. y + rows` of `surface`'s buffer into its copy.
    fn fetch(&mut self, surface: SurfaceId, y: u32, rows: u32) {
        let Some((b, _, height, stride)) = self.surfaces.buffer_of(surface) else { return };
        let Some(copy) = self.copies.0.get_mut(&b) else { return };
        let per = (READ_CHUNK / (stride as usize).max(1)).max(1) as u32;
        let end = y.saturating_add(rows).min(height);
        let mut row = y;
        while row < end {
            let n = per.min(end - row);
            let at = row as usize * stride as usize;
            if copy.len() < at + n as usize * stride as usize {
                return;
            }
            let arg = ReadBuffer { buffer: b, row, rows: n, dst: copy[at..].as_mut_ptr() as u64 }.to_bytes();
            // SAFETY: READ_BUFFER reads a ReadBuffer from `arg` and writes
            // `n` rows of `stride` bytes at `dst`, inside `copy` (checked
            // above).
            if unsafe { os::control(self.fd, READ_BUFFER, arg.as_ptr() as u64) }.is_err() {
                return;
            }
            row += n;
        }
    }
}

/// A window's content (for tests and the PPM dump's probes).
pub fn content_of(scene: &Scene, window: WindowId) -> Option<&Content> {
    scene.window(window).map(|w| w.content())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::string::String;
    use alloc::vec;
    use poly_wayland::compose::{BORDER, TITLE_HEIGHT};
    use poly_win32k::desktop::Desktop;
    use poly_win32k::frame::FrameMetrics;
    use poly_win32k::link::{Record, Stream, VERSION, encode_all};

    struct Memory(BTreeMap<BufferId, Vec<u8>>);

    impl Buffers for Memory {
        fn bytes(&self, id: BufferId) -> Option<&[u8]> {
            self.0.get(&id).map(Vec::as_slice)
        }
    }

    /// Everything win32k's end sent, applied to the scene.
    fn pump(d: &mut Desktop, ws: &mut WindowsSurfaces, scene: &mut Scene, mem: &Memory) -> Vec<Action> {
        let mut s = Stream::<ToCompositor>::new();
        s.push(&d.take_output());
        let mut actions = Vec::new();
        for r in s {
            actions.extend(ws.apply(scene, &r.unwrap(), mem));
        }
        actions
    }

    #[test]
    fn polys_frame_metrics_are_the_compositors() {
        assert_eq!(i32::from(FrameMetrics::POLY.title), TITLE_HEIGHT);
        assert_eq!(i32::from(FrameMetrics::POLY.border), BORDER);
        assert_eq!(FrameMetrics::BUTTON, poly_wayland::compose::CLOSE_SIZE);
        assert_eq!(FrameMetrics::GAP, poly_wayland::compose::BUTTON_GAP);
    }

    #[test]
    fn a_windows_window_is_a_framed_scene_window_with_its_pixels() {
        let mut scene = Scene::new(800, 600, 1);
        let mut ws = WindowsSurfaces::new();
        let mut d = Desktop::new();
        let mut mem = Memory(BTreeMap::new());

        // The compositor greets win32k's end.
        let hello =
            FromCompositor::Hello { version: VERSION, width: 800, height: 600, scale: 1, frame: FrameMetrics::POLY };
        d.receive(&encode_all(&[hello]));
        assert!(pump(&mut d, &mut ws, &mut scene, &mem).is_empty());

        // A 4x2 window whose window rectangle starts at (49, 75): its
        // surface is inside Poly's frame.
        let s = d.create(SurfaceKind::TopLevel, 0, true);
        let surface = FrameMetrics::POLY.surface_from_window(poly_win32k::frame::Rect::new(49, 75, 55, 103));
        assert_eq!((surface.left, surface.top, surface.width(), surface.height()), (50, 100, 4, 2));
        d.set_geometry(s, surface.left, surface.top, 4, 2);
        d.set_title(s, "Notepad");
        let b = d.attach_buffer(s, 4, 2, 16, PixelFormat::Xrgb8888).unwrap();
        // B, G, R, A in memory; the alpha byte undefined (0) for an opaque window.
        let mut pixels = vec![0u8; 32];
        pixels[0..4].copy_from_slice(&[0x30, 0x20, 0x10, 0x00]);
        pixels[28..32].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
        mem.0.insert(b, pixels);
        let actions = pump(&mut d, &mut ws, &mut scene, &mem);
        assert_eq!(actions, vec![Action::TakeBuffer(b)]);
        let w = ws.window(s).unwrap();
        // Created hidden until shown.
        assert!(!scene.window(w).unwrap().is_visible());
        d.damage(s, 0, 0, 4, 2);
        d.set_visible(s, true);
        assert!(pump(&mut d, &mut ws, &mut scene, &mem).is_empty());

        let win = scene.window(w).unwrap();
        assert!(win.is_visible() && win.is_decorated());
        assert_eq!(win.position(), (50, 100));
        assert_eq!(win.title(), "Notepad");
        assert_eq!(win.owner(), OWNER_TAG | u64::from(s));
        assert_eq!(ws.surface_of(w), Some(s));
        let c = content_of(&scene, w).unwrap();
        assert_eq!((c.width, c.height), (4, 2));
        assert_eq!(c.pixels[0], 0xFF10_2030);
        assert_eq!(c.pixels[7], 0xFFFF_FFFF);
        assert_eq!(c.pixels[1], 0xFF00_0000);

        // The compositor reports moved client geometry once.
        assert!(ws.sync_geometry(&scene).is_empty());
        scene.set_position(w, 75, 120);
        assert_eq!(
            ws.sync_geometry(&scene),
            vec![FromCompositor::Configure { surface: s, x: 75, y: 120, width: 4, height: 2 }]
        );
        assert!(ws.sync_geometry(&scene).is_empty(), "unchanged geometry is not resent");

        // A new buffer releases the old one back to win32k, which frees it.
        let b2 = d.attach_buffer(s, 8, 4, 32, PixelFormat::Xrgb8888).unwrap();
        let actions = pump(&mut d, &mut ws, &mut scene, &mem);
        assert_eq!(
            actions,
            vec![Action::DropBuffer(b), Action::Send(FromCompositor::Released { buffer: b }), Action::TakeBuffer(b2)]
        );
        let replies: Vec<FromCompositor> =
            actions.iter().filter_map(|a| if let Action::Send(r) = a { Some(r.clone()) } else { None }).collect();
        d.receive(&encode_all(&replies));
        assert_eq!(d.take_released(), vec![b]);

        // An owned popup: unframed, part of its owner.
        let menu = d.create(SurfaceKind::Popup, s, true);
        d.request_activate(s);
        pump(&mut d, &mut ws, &mut scene, &mem);
        let m = scene.window(ws.window(menu).unwrap()).unwrap();
        assert!(!m.is_decorated());
        assert_eq!(m.parent(), Some(w));

        // Destroyed: out of the scene, its buffer released.
        d.destroy(s);
        let actions = pump(&mut d, &mut ws, &mut scene, &mem);
        assert!(scene.window(w).is_none());
        assert!(actions.contains(&Action::Send(FromCompositor::Released { buffer: b2 })));
    }

    #[test]
    fn a_surface_knows_its_process_for_the_startup_feedback() {
        let mut scene = Scene::new(800, 600, 1);
        let mut ws = WindowsSurfaces::new();
        let mut d = Desktop::new();
        let mem = Memory(BTreeMap::new());
        let hello =
            FromCompositor::Hello { version: VERSION, width: 800, height: 600, scale: 1, frame: FrameMetrics::POLY };
        d.receive(&encode_all(&[hello]));
        let s = d.create(SurfaceKind::TopLevel, 0, true);
        let t = d.create(SurfaceKind::TopLevel, 0, true);
        d.set_process(s, 4321);
        pump(&mut d, &mut ws, &mut scene, &mem);
        assert_eq!(ws.process_of(ws.window(s).unwrap()), Some(4321));
        // Not told: none.
        assert_eq!(ws.process_of(ws.window(t).unwrap()), None);
    }

    #[test]
    fn a_reconnection_rebuilds_the_scene_from_the_replay() {
        let mut scene = Scene::new(800, 600, 1);
        let mut ws = WindowsSurfaces::new();
        let mut d = Desktop::new();
        let mem = Memory(BTreeMap::new());
        let s = d.create(SurfaceKind::TopLevel, 0, true);
        d.set_title(s, "Before");
        d.set_visible(s, true);
        let hello =
            FromCompositor::Hello { version: VERSION, width: 800, height: 600, scale: 1, frame: FrameMetrics::POLY };
        d.receive(&encode_all(core::slice::from_ref(&hello)));
        pump(&mut d, &mut ws, &mut scene, &mem);
        let first = ws.window(s).unwrap();
        assert_eq!(scene.window(first).unwrap().title(), "Before");
        // The compositor restarts: the old windows go, the replay brings them back.
        d.disconnect();
        d.set_title(s, "After");
        d.receive(&encode_all(&[hello]));
        pump(&mut d, &mut ws, &mut scene, &mem);
        let second = ws.window(s).unwrap();
        assert!(scene.window(first).is_none());
        assert_eq!(scene.window(second).unwrap().title(), "After");
        assert_eq!(scene.windows().len(), 1);
        // Records about unknown surfaces are ignored.
        let mut out = Vec::new();
        ToCompositor::Title { surface: 99, text: String::from("x") }.encode(&mut out);
        let mut st = Stream::<ToCompositor>::new();
        st.push(&out);
        assert!(ws.apply(&mut scene, &st.next().unwrap().unwrap(), &mem).is_empty());
    }
}
