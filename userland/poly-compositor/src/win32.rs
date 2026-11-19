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
    /// An application window (not a popup).
    top_level: bool,
    /// What win32k last said: shown, minimized (`Visible`, `State`).
    shown: bool,
    minimized: bool,
    /// A restore or minimize was asked of win32k for this state.
    asked: bool,
    /// The window's own borders around the surface that take the pointer
    /// (`Margins`): left, top, right, bottom.
    margins: [i32; 4],
}

/// Windows surfaces in the scene.
#[derive(Debug)]
pub struct WindowsSurfaces {
    surfaces: BTreeMap<SurfaceId, Surface>,
    /// Windows pixels per logical pixel (ADR 0219): 1, or the output's whole scale when Windows
    /// apps live in output pixels (`--windows-hidpi`). Positions and sizes on the link are in
    /// these; the scene's are logical.
    units: i32,
}

impl Default for WindowsSurfaces {
    fn default() -> WindowsSurfaces {
        WindowsSurfaces::new()
    }
}

impl WindowsSurfaces {
    pub fn new() -> WindowsSurfaces {
        WindowsSurfaces { surfaces: BTreeMap::new(), units: 1 }
    }

    /// Windows apps live in `units` output pixels per logical pixel (ADR 0219).
    pub fn set_units(&mut self, units: i32) {
        self.units = units.max(1);
    }

    pub fn units(&self) -> i32 {
        self.units
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

    /// The borders around a Windows surface's scene window that take the
    /// pointer (left, top, right, bottom).
    pub fn margins_of(&self, window: WindowId) -> [i32; 4] {
        self.surfaces.values().find(|s| s.window == window).map_or([0; 4], |s| s.margins)
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
        let u = self.units;
        moved
            .into_iter()
            .map(|(surface, (x, y, width, height))| FromCompositor::Configure {
                surface,
                x: x * u,
                y: y * u,
                width: width * u as u32,
                height: height * u as u32,
            })
            .collect()
    }

    /// The shell showed or hid an application window itself (a minimized
    /// window picked in the window switcher, the shell's own minimize):
    /// the window's restore or minimize, for win32k to have the app run
    /// (so it draws again, and knows it is minimized). Once per state
    /// win32k reports.
    pub fn shell_requests(&mut self, scene: &Scene) -> Vec<FromCompositor> {
        use poly_win32k::link::Request;
        let mut out = Vec::new();
        for (&surface, s) in &mut self.surfaces {
            if s.asked || !s.top_level {
                continue;
            }
            let Some(win) = scene.window(s.window) else { continue };
            let request = if s.minimized && !win.is_hidden() {
                Request::Restore
            } else if s.shown && !s.minimized && win.is_hidden() {
                Request::Minimize
            } else {
                continue;
            };
            s.asked = true;
            out.push(FromCompositor::Request { surface, request });
        }
        out
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
                self.surfaces.insert(
                    surface,
                    Surface {
                        window,
                        buffer: None,
                        geometry: (0, 0, 0, 0),
                        pid: 0,
                        top_level: kind == SurfaceKind::TopLevel,
                        shown: false,
                        minimized: false,
                        asked: false,
                        margins: [0; 4],
                    },
                );
            }
            ToCompositor::Geometry { surface, x, y, width, height } => {
                // Windows pixels to logical ones (ADR 0219).
                let u = self.units;
                let (x, y, width, height) = (x.div_euclid(u), y.div_euclid(u), width / u as u32, height / u as u32);
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
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    (s.shown, s.asked) = (shown, false);
                }
            }
            ToCompositor::Buffer { surface, buffer, width, height, stride, format } => {
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    if let Some((old, ..)) = s.buffer.replace((buffer, width, height, stride, format)) {
                        actions.push(Action::DropBuffer(old));
                        actions.push(Action::Send(FromCompositor::Released { buffer: old }));
                    }
                    // A buffer in Windows pixels: `units` of them a logical pixel. One the
                    // output enlarges is enlarged smoothly, not in blocks (ADR 0219).
                    scene.resize_content(s.window, width, height, self.units as u32);
                    scene.set_smooth(s.window, true);
                    actions.push(Action::TakeBuffer(buffer));
                }
            }
            ToCompositor::Damage { surface, x, y, width, height } => {
                println!("feel-t: damage s={surface} {x},{y} {width}x{height} us={}", os::monotonic_ns() / 1000); // TEMP-FEEL
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
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    (s.minimized, s.asked) = (state == WindowState::Minimized, false);
                }
            }
            // A request for the desktop, not for a surface ([`Link::take_shell`]).
            ToCompositor::Shell { .. } => {}
            ToCompositor::Margins { surface, left, top, right, bottom } => {
                let u = self.units;
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    s.margins = [left, top, right, bottom].map(|m| i32::from(m) / u);
                    if let Some(win) = scene.window(s.window) {
                        println!("poly-compositor: window {:?} own borders {left},{top},{right},{bottom}", win.title());
                    }
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
    /// Interactive moves win32k asked for (`DefWindowProc`'s `SC_MOVE`),
    /// started once the server is at hand.
    moves: Vec<WindowId>,
    /// Interactive resizes (`SC_SIZE` by a border), likewise.
    resizes: Vec<(WindowId, Edges)>,
    /// The keyboard layout win32k was told (`poly-input`'s name).
    layout: alloc::string::String,
    /// What Windows programs asked the desktop to open (`false`) or show in Files (`true`),
    /// by path (ADR 0223), for the compositor's loop.
    shell: Vec<(bool, alloc::string::String)>,
}

/// The `xdg_toplevel.resize_edge` bits of the server's interactive resize
/// (top 1, bottom 2, left 4, right 8) for the desktop link's edges.
pub fn resize_edges(e: Edges) -> u32 {
    let mut bits = 0;
    for (edge, bit) in [(Edges::TOP, 1), (Edges::BOTTOM, 2), (Edges::LEFT, 4), (Edges::RIGHT, 8)] {
        if e.contains(edge) {
            bits |= bit;
        }
    }
    bits
}

/// What the compositor says first on the link: `Hello` with the output's logical size (and Poly's
/// frame) in Windows pixels, `units` of them a logical pixel, then `Units` when that is more than
/// one (ADR 0219): Windows apps then live in output pixels at 96 times `units` DPI.
pub fn greeting(width: u32, height: u32, scale: u32, units: u32) -> Vec<FromCompositor> {
    let u = units.max(1);
    let f = FrameMetrics::POLY;
    let k = |v: u16| (u32::from(v) * u).min(u32::from(u16::MAX)) as u16;
    let mut out = vec![FromCompositor::Hello {
        version: VERSION,
        width: width * u,
        height: height * u,
        // Windows apps draw at a whole scale; the compositor shrinks
        // their pictures to a fractional output's (ADR 0179).
        scale,
        frame: FrameMetrics { title: k(f.title), border: k(f.border), resize: k(f.resize) },
    }];
    if u > 1 {
        out.push(FromCompositor::Units { units: u });
    }
    out
}

impl Link {
    /// Open [`DEVICE`] and greet win32k with the output and Poly's frame;
    /// `None` (logged) without the device. With `hidpi` Windows apps live in
    /// output pixels at the output's whole scale (ADR 0219).
    pub fn open(scene: &Scene, hidpi: bool) -> Option<Link> {
        let fd = match os::open_device(DEVICE, true, true) {
            Ok(fd) => fd,
            Err(e) => {
                println!("poly-compositor: no desktop link to Windows apps ({e})");
                return None;
            }
        };
        let (width, height) = scene.logical_size();
        let (_, _, scale) = scene.output();
        let units = if hidpi { scale.ceil().max(1) } else { 1 };
        let mut out = Vec::new();
        for r in greeting(width as u32, height as u32, scale.ceil(), units) {
            r.encode(&mut out);
        }
        if let Err(e) = os::write(fd, &out) {
            println!("poly-compositor: {DEVICE}: {e}");
            return None;
        }
        println!("poly-compositor: desktop link to Windows apps on {DEVICE} ({units} Windows pixels a logical pixel)");
        let mut surfaces = WindowsSurfaces::new();
        surfaces.set_units(units as i32);
        Some(Link {
            fd,
            surfaces,
            copies: Copies::default(),
            incoming: Stream::new(),
            moves: Vec::new(),
            resizes: Vec::new(),
            layout: alloc::string::String::new(),
            shell: Vec::new(),
        })
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
            | E::Resize { window, .. }
            | E::Configure { window, .. } => window,
            E::Leave(window) | E::Close(window) => window,
        };
        let Some(surface) = self.surfaces.surface_of(window) else { return false };
        let time = (os::monotonic_ns() / 1_000_000) as u32;
        // TEMP-FEEL
        if let E::Key { key, pressed: true, .. } = *ev {
            println!("feel-t: in key={key} us={}", os::monotonic_ns() / 1000);
        }
        if let E::Button { pressed: true, .. } = *ev {
            println!("feel-t: in button us={}", os::monotonic_ns() / 1000);
        }
        // Logical positions to Windows pixels (ADR 0219).
        let u = self.surfaces.units();
        let records: Vec<FromCompositor> = match *ev {
            E::Motion { x, y, .. } => vec![FromCompositor::Pointer { surface, x: x * u, y: y * u, time }],
            E::Button { x, y, button, pressed, .. } => vec![
                FromCompositor::Pointer { surface, x: x * u, y: y * u, time },
                FromCompositor::Button { surface, button: button as u16, pressed, time },
            ],
            E::Axis { horizontal, value, .. } => {
                // 15 logical pixels a step, WHEEL_DELTA (120) a notch, up and right positive.
                let notches = if horizontal { value * 120 / 15 } else { -value * 120 / 15 };
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
            // A resize by the window's border: win32k has the app's thread
            // resize the window (its `SetWindowPos`), and the surface
            // follows with the new size.
            E::Configure { x, y, width, height, .. } => vec![FromCompositor::Configure {
                surface,
                x: x * u,
                y: y * u,
                width: (width.max(1) * u) as u32,
                height: (height.max(1) * u) as u32,
            }],
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
        // A press on a Windows window's own caption: the server moves it
        // with the pointer while the button is held (ADR 0128).
        for w in core::mem::take(&mut self.moves) {
            server.internal_start_move(w);
        }
        // A press on its own border: the server resizes it with the pointer.
        for (w, edges) in core::mem::take(&mut self.resizes) {
            println!("poly-compositor: resize of a Windows window by its border (edges {:#x})", edges.0);
            server.internal_start_resize(w, resize_edges(edges));
        }
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
        for &(w, _) in &after {
            // Its own sizing borders take the pointer too.
            server.set_internal_margins(w, self.surfaces.margins_of(w));
        }
    }

    /// Tell win32k where the compositor moved or resized a Windows
    /// surface. This keeps the app's GetWindowRect and later input in
    /// step with Poly's frame. (Closing needs no path of its own: the
    /// server reports a close of an adopted window as an internal event,
    /// which [`Link::input`] forwards.)
    /// What Windows programs asked the desktop to open or show since the last call (ADR 0223).
    pub fn take_shell(&mut self) -> Vec<(bool, alloc::string::String)> {
        core::mem::take(&mut self.shell)
    }

    pub fn sync_geometry(&mut self, scene: &Scene) {
        for record in self.surfaces.sync_geometry(scene) {
            self.send(record);
        }
        for record in self.surfaces.shell_requests(scene) {
            self.send(record);
        }
    }

    /// The desktop's keyboard layout (`poly-input`'s name): win32k reads
    /// Windows apps' keys with it (ADR 0209). Sent when it changes.
    pub fn set_layout(&mut self, name: &str) {
        if self.layout != name {
            self.layout = name.into();
            self.send(FromCompositor::Layout { name: name.into() });
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
                ToCompositor::Shell { show, ref path } => self.shell.push((show, path.clone())),
                _ => {}
            }
            for a in self.surfaces.apply(scene, &r, &self.copies) {
                match a {
                    Action::DropBuffer(b) => {
                        self.copies.0.remove(&b);
                    }
                    Action::Send(rec) => rec.encode(&mut out),
                    Action::Activate(w) => scene.raise(w),
                    Action::BeginMove(w) => self.moves.push(w),
                    Action::BeginResize(w, edges) => self.resizes.push((w, edges)),
                    // The copy was made with the buffer record; cursors
                    // come with a later slice.
                    Action::TakeBuffer(_) | Action::Cursor(..) => {}
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

    /// ADR 0219: with Windows apps in output pixels (`--windows-hidpi` at 200 %), the greeting
    /// gives win32k the output and Poly's frame in those pixels and the units; a surface's buffer
    /// is shown at scale 2 (pixel for pixel on the output, not enlarged), its position and borders
    /// come back to logical pixels, and moves go out in Windows pixels.
    #[test]
    fn windows_apps_in_output_pixels_are_shown_pixel_for_pixel() {
        let g = greeting(1280, 800, 2, 2);
        let f = FrameMetrics::POLY;
        assert_eq!(
            g,
            vec![
                FromCompositor::Hello {
                    version: VERSION,
                    width: 2560,
                    height: 1600,
                    scale: 2,
                    frame: FrameMetrics { title: f.title * 2, border: f.border * 2, resize: f.resize * 2 },
                },
                FromCompositor::Units { units: 2 },
            ]
        );
        assert_eq!(greeting(1280, 800, 2, 1).len(), 1, "no Units record at one unit");

        let mut scene = Scene::new(2560, 1600, 2);
        let mut ws = WindowsSurfaces::new();
        ws.set_units(2);
        let mut d = Desktop::new();
        let mut mem = Memory(BTreeMap::new());
        d.receive(&encode_all(&g));
        pump(&mut d, &mut ws, &mut scene, &mem);
        let s = d.create(SurfaceKind::TopLevel, 0, false);
        d.set_geometry(s, 36, 68, 8, 6);
        let b = d.attach_buffer(s, 8, 6, 32, PixelFormat::Xrgb8888).unwrap();
        mem.0.insert(b, vec![0x40; 8 * 6 * 4]);
        d.set_margins(s, [8, 0, 8, 8]);
        d.damage(s, 0, 0, 8, 6);
        d.set_visible(s, true);
        pump(&mut d, &mut ws, &mut scene, &mem);
        let w = ws.window(s).unwrap();
        let win = scene.window(w).unwrap();
        assert_eq!(win.position(), (18, 34), "logical");
        let c = content_of(&scene, w).unwrap();
        assert_eq!((c.width, c.height, c.scale), (8, 6, 2));
        assert_eq!(c.logical_size(), (4, 3));
        assert_eq!(ws.margins_of(w), [4, 0, 4, 4]);
        assert!(ws.sync_geometry(&scene).is_empty());
        scene.set_position(w, 20, 40);
        assert_eq!(
            ws.sync_geometry(&scene),
            vec![FromCompositor::Configure { surface: s, x: 40, y: 80, width: 8, height: 6 }]
        );
        // Drawn on a 200 % output without enlarging: one buffer pixel, one output pixel.
        let mut fb = poly_wayland::compose::MemoryFramebuffer::new(2560, 1600);
        scene.render(&mut fb);
        assert_eq!(fb.pixel(40, 80), Some(0x40_4040));
        assert_eq!(fb.pixel(47, 85), Some(0x40_4040));
        assert_ne!(fb.pixel(48, 80), Some(0x40_4040), "8 output pixels wide, not 16");
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

    #[test]
    fn a_resize_by_a_windows_border_uses_the_servers_edges() {
        assert_eq!(resize_edges(Edges::LEFT), 4);
        assert_eq!(resize_edges(Edges::RIGHT.union(Edges::BOTTOM)), 8 | 2);
        assert_eq!(resize_edges(Edges::TOP.union(Edges::LEFT)), 1 | 4);
        assert_eq!(resize_edges(Edges(0)), 0);
    }

    #[test]
    fn the_shell_restores_and_minimizes_a_windows_window_through_win32k() {
        use poly_win32k::link::{Request, WindowState};
        let mut scene = Scene::new(800, 600, 1);
        let mut ws = WindowsSurfaces::new();
        let mut d = Desktop::new();
        let mem = Memory(BTreeMap::new());
        let hello =
            FromCompositor::Hello { version: VERSION, width: 800, height: 600, scale: 1, frame: FrameMetrics::POLY };
        d.receive(&encode_all(&[hello]));
        pump(&mut d, &mut ws, &mut scene, &mem);
        let s = d.create(SurfaceKind::TopLevel, 0, false);
        d.set_geometry(s, 10, 10, 4, 2);
        d.set_visible(s, true);
        pump(&mut d, &mut ws, &mut scene, &mem);
        let w = ws.window(s).unwrap();
        assert!(ws.shell_requests(&scene).is_empty(), "as win32k said");
        // The app minimized it: hidden, nothing to ask.
        d.set_state(s, WindowState::Minimized);
        d.set_visible(s, false);
        pump(&mut d, &mut ws, &mut scene, &mem);
        assert!(scene.window(w).unwrap().is_hidden());
        assert!(ws.shell_requests(&scene).is_empty());
        // The window switcher shows it: the app restores it, once.
        scene.set_hidden(w, false);
        assert_eq!(ws.shell_requests(&scene), vec![FromCompositor::Request { surface: s, request: Request::Restore }]);
        assert!(ws.shell_requests(&scene).is_empty());
        d.set_state(s, WindowState::Normal);
        d.set_visible(s, true);
        pump(&mut d, &mut ws, &mut scene, &mem);
        assert!(ws.shell_requests(&scene).is_empty());
        // The shell hides a shown window: the app minimizes it.
        scene.set_hidden(w, true);
        assert_eq!(ws.shell_requests(&scene), vec![FromCompositor::Request { surface: s, request: Request::Minimize }]);
        // A window the app hid itself (not minimized) is left alone.
        d.set_visible(s, false);
        pump(&mut d, &mut ws, &mut scene, &mem);
        assert!(ws.shell_requests(&scene).is_empty());
    }
}
