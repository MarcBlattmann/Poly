//! The desktop (ADR 0107, ADR 0117): the shell, the input devices and what
//! goes with them — input routed between the shell and the clients, window
//! management (the switcher, snapping, Super+arrows, Alt+F4), the settings
//! window, the notifications on the session bus, the ridge's status
//! (network, volume, local time) and the settings file.

use crate::desktop::{self, WmKey};
use crate::mixer::Mixer;
use crate::session::SessionBus;
use crate::{evdev, status, unix};
use poly_dbus::notifications::{self as dn, Request};
use poly_input::layout::Layout;
use poly_shell::settings::{Change, Settings};
use poly_shell::settings_ui::{Page, SystemInfo};
use poly_shell::switcher::{Entry, Mru, thumbnail};
use poly_shell::wm::{self, Action, Placed, Zone};
use poly_shell::{Key, Press, Shell};
use poly_wayland::compose::{MemoryFramebuffer, Placement, WindowId};
use poly_wayland::seat::{BTN_LEFT, InputEvent, InputSource};
use poly_wayland::server::{InternalEvent, OutputConfig, Server};
use poly_wayland::wire::Fixed;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The settings window, drawn by the shell.
#[derive(Debug)]
struct SettingsWin {
    id: WindowId,
    size: (i32, i32),
}

/// What the desktop needs from the command line.
#[derive(Debug)]
pub struct Options<'a> {
    pub dir: &'a Path,
    pub fb_name: &'a str,
    pub keymap: Option<&'a str>,
    pub scale: Option<u32>,
    pub settings: Option<PathBuf>,
}

/// The desktop.
#[derive(Debug)]
pub struct Desktop {
    pub shell: Shell,
    pub input: evdev::Evdev,
    hotkeys: desktop::Hotkeys,
    pub layout: &'static Layout,
    keymap: unix::SharedKeymap,
    dir: PathBuf,
    /// A left press the shell took: its release is the shell's too.
    shell_button: bool,
    /// Apps started from the launcher, not reaped yet.
    pub children: Vec<(libc::pid_t, String)>,
    /// Windows seen, to log new ones and to bring the launcher back when
    /// the last one goes.
    windows: BTreeSet<u32>,
    pub ready: bool,
    mru: Mru,
    last_active: Option<WindowId>,
    /// Where the window being dragged would snap.
    snap: Option<Zone>,
    settings_path: Option<PathBuf>,
    settings_win: Option<SettingsWin>,
    info: SystemInfo,
    mixer: Option<Mixer>,
    pub bus: Option<SessionBus>,
    next_status_ms: u64,
}

/// Seconds on `CLOCK_REALTIME`.
pub fn realtime_secs() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid timespec to write.
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    ts.tv_sec
}

fn now_ms() -> u64 {
    unix::monotonic_ns() / 1_000_000
}

/// Where the settings live: `$XDG_CONFIG_HOME/poly/shell.conf`, else
/// `$HOME/.config/poly/shell.conf`, else `/root/.config/...` for root.
pub fn settings_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
        home.join(".config")
    });
    base.join("poly").join("shell.conf")
}

/// Write `text` to `path`: a temporary file, synced, renamed into place.
fn save_file(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("conf.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    // The rename too must reach the disk before anyone pulls the plug.
    // SAFETY: sync has no preconditions.
    unsafe { libc::sync() };
    Ok(())
}

impl Desktop {
    /// The desktop on `server`'s output: the shell with the apps and the
    /// font, the input devices, the settings read and applied, the mixer,
    /// the session bus.
    pub fn new(
        o: &Options<'_>,
        server: &mut Server,
        keymap: unix::SharedKeymap,
        font: Option<poly_font::Font<'static>>,
    ) -> Desktop {
        let (lw, lh) = server.scene().logical_size();
        let apps = poly_shell::apps::catalog(desktop::desktop_files(Path::new(desktop::APPLICATIONS)));
        let mut shell = Shell::new(lw, lh, apps, poly_shell::draw::Text::new(font));
        if let Some(f) = font {
            // Window titles in the same font, 13 pixels as Alpine's rows.
            server.scene_mut().set_text(Box::new(poly_shell::draw::TitleFont::new(f, 13.0, 16)));
        }
        // The settings: read, the command line's choices first.
        let settings_path = o.settings.clone().unwrap_or_else(settings_path);
        let (mut settings, saved) = match std::fs::read_to_string(&settings_path) {
            Ok(text) => {
                println!("poly-compositor: settings read from {}", settings_path.display());
                (Settings::parse(&text), true)
            }
            Err(_) => (Settings::default(), false),
        };
        if let Some(k) = o.keymap {
            settings.keyboard = k.into();
        }
        if let Some(s) = o.scale {
            settings.scale = s;
        }
        let layout = poly_input::layout::by_name(&settings.keyboard).unwrap_or(&poly_input::layout::US);
        match desktop::keymap_file(o.dir, layout) {
            Ok(k) => keymap.set(Some(k)),
            Err(e) => eprintln!("poly-compositor: keymap: {e}"),
        }
        let mixer = match Mixer::open("/dev/snd/controlC0") {
            Ok(m) => {
                if saved {
                    // The volume saved last time.
                    if let Err(e) = m.set_volume(settings.volume).and_then(|()| m.set_muted(settings.muted).or(Ok(())))
                    {
                        eprintln!("poly-compositor: the mixer: {e}");
                    }
                } else if let Ok((v, muted)) = m.read() {
                    // Nothing saved yet: the card's own volume stays.
                    (settings.volume, settings.muted) = (v, muted);
                }
                Some(m)
            }
            Err(e) => {
                println!("poly-compositor: no mixer ({e}): the ridge shows no volume");
                None
            }
        };
        let bus = match SessionBus::open(o.dir, &[dn::NAME]) {
            Ok(b) => {
                println!("poly-compositor: session bus on {} ({} served)", b.address(), dn::NAME);
                Some(b)
            }
            Err(e) => {
                eprintln!("poly-compositor: no session bus: {e}");
                None
            }
        };
        if settings.scale != server.output().scale {
            let out = OutputConfig { scale: settings.scale, ..server.output().clone() };
            server.set_output(out);
            let (lw, lh) = server.scene().logical_size();
            shell.resize(lw, lh);
        }
        let (w, h) = (server.output().width, server.output().height);
        if let Some((rw, rh)) = settings.resolution
            && (rw, rh) != (w, h)
        {
            println!(
                "poly-compositor: resolution {rw}x{rh} asked for; {} stays at {w}x{h} (modes come with KMS)",
                o.fb_name
            );
        }
        shell.set_settings(settings);
        let input = evdev::Evdev::open_all();
        let names: Vec<String> =
            input.devices.iter().map(|d| format!("{} ({:?})", d.name, d.translator.kind)).collect();
        println!(
            "poly-compositor: desktop on {} ({w}x{h}, scale {}), keymap {}, {} input device(s): {}",
            o.fb_name,
            server.output().scale,
            layout.name,
            names.len(),
            names.join(", ")
        );
        println!("poly-compositor: {} app(s) in the launcher", shell.apps().len());
        server.set_seat_capabilities(input.capabilities());
        server.scene_mut().set_placement(Placement::Centered);
        server.scene_mut().set_reserved_top(shell.reserved_top());
        let (lw, lh) = server.scene().logical_size();
        // The middle of the screen until the pointer moves.
        server.handle_input(InputEvent::PointerMotionAbsolute {
            time: 0,
            x: Fixed::from_int(lw / 2),
            y: Fixed::from_int(lh / 2),
        });
        server.set_cursor_visible(true);
        server.scene_mut().damage_all();
        let mut d = Desktop {
            shell,
            input,
            hotkeys: desktop::Hotkeys::default(),
            layout,
            keymap,
            dir: o.dir.to_path_buf(),
            shell_button: false,
            children: Vec::new(),
            windows: BTreeSet::new(),
            ready: false,
            mru: Mru::default(),
            last_active: None,
            snap: None,
            settings_path: Some(settings_path),
            settings_win: None,
            info: SystemInfo::default(),
            mixer,
            bus,
            next_status_ms: 0,
        };
        d.info = d.system_info(server);
        d
    }

    /// What the About page shows.
    fn system_info(&self, server: &Server) -> SystemInfo {
        let (sys, release, version, machine) = status::uname();
        let cpu = std::fs::read_to_string("/proc/cpuinfo").map(|t| status::cpu_model(&t)).unwrap_or_default();
        let memory_kib = std::fs::read_to_string("/proc/meminfo").map(|t| status::mem_total_kib(&t)).unwrap_or(0);
        let o = server.output();
        SystemInfo {
            os: format!("Poly {}", env!("CARGO_PKG_VERSION")),
            kernel: format!("{sys} {release} {version}").trim().to_string(),
            machine,
            cpu,
            memory_kib,
            display: (o.width, o.height),
            modes: vec![(o.width, o.height)],
            sound: self.mixer.as_ref().map(|m| if m.card.is_empty() { "sound card 0".into() } else { m.card.clone() }),
            network: status::describe(self.shell.network()),
            session_bus: self.bus.as_ref().map_or_else(|| "none".into(), SessionBus::address),
        }
    }

    /// An input event: window management keys, the launcher's hotkeys and
    /// keys, the shell's presses, the rest to the server. Returns an app
    /// to start.
    pub fn route(&mut self, ev: InputEvent, server: &mut Server) -> Option<poly_shell::App> {
        match ev {
            InputEvent::Key { key, pressed, .. } => {
                let (depressed, _) = server.modifiers();
                if let Some(action) = desktop::wm_key(key, pressed, depressed, self.shell.switcher_open()) {
                    // Releasing Alt reaches the clients (the modifier
                    // goes); the rest is the compositor's.
                    if !pressed {
                        server.handle_input(ev);
                    }
                    self.hotkeys.key(key, pressed, depressed);
                    self.wm_key(action, server);
                    return None;
                }
                let toggles = self.hotkeys.key(key, pressed, depressed);
                // Escape closes the notification history (the clients do
                // not see it).
                if pressed && self.shell.history_open() && key == 1 {
                    self.shell.overlay_key(Key::Escape);
                    return None;
                }
                let is_super = matches!(key, 125 | 126);
                // Ctrl+Space is the shell's; the Windows key is a modifier
                // the clients may see.
                if !(toggles && !is_super) {
                    server.handle_input(ev);
                }
                if toggles {
                    self.shell.toggle_launcher();
                    return None;
                }
                if pressed && self.shell.launcher().is_visible() {
                    let (depressed, locked) = server.modifiers();
                    if let Some(k) = desktop::launcher_key(key, depressed, locked, self.layout) {
                        let before = self.shell.launcher().query().to_string();
                        let app = self.shell.key(k);
                        if self.shell.launcher().query() != before {
                            println!("poly-compositor: launcher query {:?}", self.shell.launcher().query());
                        }
                        return app;
                    }
                }
                None
            }
            InputEvent::PointerButton { button: BTN_LEFT, pressed, .. } => {
                let (x, y) = server.pointer();
                if pressed {
                    match self.shell.press(x.floor(), y.floor()) {
                        Press::Taken => {
                            self.shell_button = true;
                            return None;
                        }
                        Press::Launch(app) => {
                            self.shell_button = true;
                            return Some(app);
                        }
                        Press::Switch(w) => {
                            self.shell_button = true;
                            self.switch_to(w, server);
                            return None;
                        }
                        Press::Settings(page) => {
                            self.shell_button = true;
                            self.open_settings(page, server);
                            return None;
                        }
                        Press::NotMine => {}
                    }
                } else if std::mem::take(&mut self.shell_button) {
                    return None;
                }
                let moving = server.moving_window();
                server.handle_input(ev);
                if !pressed
                    && let Some(w) = moving
                    && server.moving_window().is_none()
                {
                    // A drag ended: snap where the preview showed.
                    if let Some(zone) = self.snap.take() {
                        self.place(w, zone, server);
                    }
                    self.shell.set_snap_preview(None);
                }
                None
            }
            InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. } => {
                server.handle_input(ev);
                let (x, y) = server.pointer();
                let (x, y) = (x.floor(), y.floor());
                self.shell.pointer_motion(x, y);
                if server.moving_window().is_some() {
                    let work = server.scene().work_area();
                    self.snap = wm::zone_at(x, y, work);
                    self.shell.set_snap_preview(self.snap.map(|z| wm::zone_rect(z, work)));
                } else if self.snap.take().is_some() {
                    self.shell.set_snap_preview(None);
                }
                None
            }
            _ => {
                server.handle_input(ev);
                None
            }
        }
    }

    /// Snap window `w` to `zone`.
    fn place(&mut self, w: WindowId, zone: Zone, server: &mut Server) {
        let work = server.scene().work_area();
        let r = wm::zone_rect(zone, work);
        if zone == Zone::Full {
            server.set_maximized(w, true);
        } else {
            server.tile_window(w, r);
        }
        println!("poly-compositor: window {} snapped {zone:?} to {},{} {}x{}", w.0, r.x, r.y, r.width, r.height);
    }

    /// Switch to window `w`: shown if minimized, raised, focused.
    fn switch_to(&mut self, w: WindowId, server: &mut Server) {
        server.activate_window(w);
        let title = server.scene().window(w).map(|win| win.title().to_string()).unwrap_or_default();
        println!("poly-compositor: switched to {title:?}");
    }

    /// The switcher's entries: the managed windows, most recently used
    /// first, with thumbnails.
    fn switcher_entries(&mut self, server: &Server) -> Vec<Entry> {
        let infos = server.managed_windows();
        let ids: Vec<WindowId> = infos.iter().rev().map(|i| i.window).collect();
        let order = self.mru.order(&ids);
        order
            .into_iter()
            .filter_map(|id| {
                let info = infos.iter().find(|i| i.window == id)?;
                let content = server.scene().window(id)?.content();
                Some(Entry {
                    window: id,
                    title: info.title.clone(),
                    app_id: info.app_id.clone(),
                    minimized: info.minimized,
                    thumb: thumbnail(content, 200, 124, 0x20_20_20),
                })
            })
            .collect()
    }

    fn wm_key(&mut self, action: WmKey, server: &mut Server) {
        match action {
            WmKey::SwitchStart { backward } => {
                let entries = self.switcher_entries(server);
                if !entries.is_empty() {
                    let names: Vec<&str> = entries.iter().map(|e| e.title.as_str()).collect();
                    println!("poly-compositor: switcher open: {names:?}");
                    self.shell.open_switcher(entries, backward);
                }
            }
            WmKey::SwitchStep { forward } => self.shell.switcher_step(forward),
            WmKey::SwitchCommit => {
                if let Some(w) = self.shell.switcher_commit() {
                    self.switch_to(w, server);
                }
            }
            WmKey::SwitchCancel => self.shell.switcher_cancel(),
            WmKey::Close => {
                if let Some(w) = server.active_window() {
                    server.request_close(w);
                }
            }
            WmKey::Place(arrow) => {
                let Some(w) = server.active_window() else { return };
                let Some(info) = server.managed_windows().into_iter().find(|i| i.window == w) else { return };
                let work = server.scene().work_area();
                let placed = match (info.maximized, info.tiled.and_then(|t| wm::zone_of(t, work))) {
                    (true, _) => Placed::Maximized,
                    (_, Some(z)) => Placed::Tiled(z),
                    _ => Placed::Floating,
                };
                match wm::arrow(placed, arrow) {
                    Action::Tile(z) => self.place(w, z, server),
                    Action::Maximize => server.set_maximized(w, true),
                    Action::Restore => server.restore_window(w),
                    Action::Minimize => server.minimize_window(w),
                }
            }
            WmKey::Settings => self.open_settings(Page::Display, server),
            WmKey::History => self.shell.toggle_history(),
        }
    }

    /// An app the launcher started that is the shell's own.
    pub fn open_internal(&mut self, app: &poly_shell::App, server: &mut Server) -> bool {
        match app.internal() {
            Some(poly_shell::apps::SETTINGS) => self.open_settings(Page::Display, server),
            Some(poly_shell::apps::NOTIFICATIONS) => self.shell.toggle_history(),
            Some(_) => {}
            None => return false,
        }
        true
    }

    // -----------------------------------------------------------------
    // The settings window.

    fn open_settings(&mut self, page: Page, server: &mut Server) {
        self.shell.settings_window.page = page;
        match &self.settings_win {
            Some(w) => {
                let id = w.id;
                server.activate_window(id);
            }
            None => {
                use poly_shell::theme::settings as t;
                let (lw, lh) = server.scene().logical_size();
                let size = (t::WIDTH.min(lw - 40), t::HEIGHT.min(lh - 80));
                let id = server.add_internal_window("Settings", "poly.settings", size.0, size.1);
                self.settings_win = Some(SettingsWin { id, size });
                println!("poly-compositor: settings window opened");
            }
        }
        self.draw_settings(server);
    }

    fn draw_settings(&mut self, server: &mut Server) {
        let Some(w) = &self.settings_win else { return };
        let s = server.output().scale as i32;
        let (lw, lh) = w.size;
        let mut fb = MemoryFramebuffer::new((lw * s) as u32, (lh * s) as u32);
        self.shell.draw_settings(&mut fb, lw, lh, s, &self.info, realtime_secs());
        server.set_internal_pixels(w.id, fb.width_px(), fb.height_px(), fb.pixels(), None);
    }

    /// Events of the internal windows.
    fn internal_events(&mut self, server: &mut Server) {
        for ev in server.take_internal_events() {
            let Some(win) = &self.settings_win else { continue };
            let (id, (w, h)) = (win.id, win.size);
            let utc = realtime_secs();
            let (redraw, change) = match ev {
                InternalEvent::Motion { window, x, y } if window == id => {
                    self.shell.settings_motion(x, y, w, h, &self.info, utc)
                }
                InternalEvent::Leave(window) if window == id => (self.shell.settings_window.leave(), None),
                InternalEvent::Button { window, x, y, button: BTN_LEFT, pressed } if window == id => {
                    self.shell.settings_button(x, y, pressed, w, h, &self.info, utc)
                }
                InternalEvent::Key { window, key, pressed: true } if window == id => match key {
                    103 => {
                        self.shell.settings_window.step_page(false);
                        (true, None)
                    }
                    108 => {
                        self.shell.settings_window.step_page(true);
                        (true, None)
                    }
                    _ => (false, None),
                },
                InternalEvent::Resize { window, width, height } if window == id => {
                    if let Some(sw) = &mut self.settings_win {
                        sw.size = (width, height);
                    }
                    (true, None)
                }
                InternalEvent::Close(window) if window == id => {
                    server.remove_internal_window(id);
                    self.settings_win = None;
                    println!("poly-compositor: settings window closed");
                    continue;
                }
                _ => (false, None),
            };
            if let Some(c) = change {
                self.apply_change(c, server);
            } else if redraw {
                self.draw_settings(server);
            }
        }
    }

    /// Apply a settings change: the shell's part, then the system's
    /// (keymap, mixer, output), save the file, draw the window again.
    fn apply_change(&mut self, change: Change, server: &mut Server) {
        if !self.shell.apply(&change) {
            self.draw_settings(server);
            return;
        }
        match &change {
            Change::Keyboard(name) => {
                if let Some(layout) = poly_input::layout::by_name(name) {
                    match desktop::keymap_file(&self.dir, layout) {
                        Ok(k) => {
                            if let Some((old, _)) = self.keymap.replace(Some(k)) {
                                unix::close(old);
                            }
                            server.keymap_changed();
                            self.layout = layout;
                            println!("poly-compositor: keymap {}", layout.name);
                        }
                        Err(e) => eprintln!("poly-compositor: keymap: {e}"),
                    }
                }
            }
            Change::Volume(v) => {
                if let Some(m) = &self.mixer {
                    match m.set_volume(*v) {
                        Ok(()) => println!("poly-compositor: volume {v}"),
                        Err(e) => eprintln!("poly-compositor: the mixer: {e}"),
                    }
                }
                self.read_mixer();
            }
            Change::Muted(muted) => {
                if let Some(m) = &self.mixer {
                    let _ = m.set_muted(*muted);
                    println!("poly-compositor: {}", if *muted { "muted" } else { "unmuted" });
                }
                self.read_mixer();
            }
            Change::Scale(s) => {
                let out = OutputConfig { scale: *s, ..server.output().clone() };
                server.set_output(out);
                let (lw, lh) = server.scene().logical_size();
                self.shell.resize(lw, lh);
                server.scene_mut().damage_all();
                println!("poly-compositor: scale {s}");
            }
            Change::Resolution(w, h) => {
                let o = server.output();
                if (o.width, o.height) != (*w, *h) {
                    println!("poly-compositor: resolution {w}x{h} saved; it applies when the display can change modes");
                }
            }
            Change::Wallpaper(w) => println!("poly-compositor: wallpaper {w}"),
            Change::TimeZone(z) => {
                let t = realtime_secs();
                self.shell.set_time(t, self.shell.local_offset(t));
                println!("poly-compositor: time zone {z}");
            }
        }
        self.save_settings();
        self.draw_settings(server);
    }

    fn save_settings(&self) {
        let Some(path) = &self.settings_path else { return };
        match save_file(path, &self.shell.settings().to_text()) {
            Ok(()) => println!("poly-compositor: settings saved to {}", path.display()),
            Err(e) => eprintln!("poly-compositor: {}: {e}", path.display()),
        }
    }

    fn read_mixer(&mut self) {
        let v = self.mixer.as_ref().and_then(|m| m.read().ok());
        self.shell.set_volume(v);
    }

    // -----------------------------------------------------------------
    // Notifications on the bus.

    /// The descriptors to poll: the bus's and the mixer's.
    pub fn poll_fds(&self) -> Vec<libc::pollfd> {
        let mut v: Vec<libc::pollfd> = Vec::new();
        if let Some(b) = &self.bus {
            v.extend(b.poll_fds().into_iter().map(|(fd, _)| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }));
        }
        if let Some(m) = &self.mixer {
            v.push(libc::pollfd { fd: m.fd(), events: libc::POLLIN, revents: 0 });
        }
        v
    }

    /// The polled descriptors of [`Desktop::poll_fds`] came back.
    pub fn polled(&mut self, fds: &[libc::pollfd]) {
        let mut i = 0;
        if let Some(b) = &mut self.bus {
            let list = b.poll_fds();
            for (fd, conn) in list {
                let ready = fds.get(i).is_some_and(|p| p.fd == fd && p.revents != 0);
                i += 1;
                match (ready, conn) {
                    (true, None) => b.accept(),
                    (true, Some(id)) => b.readable(id),
                    _ => {}
                }
            }
        }
        if let Some(m) = &self.mixer
            && fds.get(i).is_some_and(|p| p.fd == m.fd() && p.revents != 0)
            && m.drain_events()
        {
            self.read_mixer();
            if self.settings_win.is_some()
                && let Some((v, muted)) = self.shell.volume()
            {
                // Another program changed it: the settings follow.
                self.shell.apply(&Change::Volume(v));
                self.shell.apply(&Change::Muted(muted));
            }
        }
        self.serve_notifications();
    }

    /// Method calls to `org.freedesktop.Notifications`, and the signals
    /// that are due.
    fn serve_notifications(&mut self) {
        let Some(bus) = &mut self.bus else { return };
        for call in bus.take_local() {
            let reply = match dn::parse(&call) {
                Ok(Request::Notify(n)) => {
                    let (app, summary) = (n.app_name.clone(), n.summary.clone());
                    let id = self.shell.notify(n);
                    println!("poly-compositor: notification {id} from {app}: {summary:?}");
                    dn::notify_reply(&call, id)
                }
                Ok(Request::Close(id)) => {
                    self.shell.close_notification(id, dn::reason::CLOSED);
                    poly_dbus::Message::method_return(&call, vec![])
                }
                Ok(Request::GetCapabilities) => dn::capabilities_reply(&call, &["body", "actions", "persistence"]),
                Ok(Request::GetServerInformation) => {
                    dn::server_information_reply(&call, "Poly Shell", "Poly", env!("CARGO_PKG_VERSION"))
                }
                Ok(Request::Introspect) => dn::introspect_reply(&call),
                Ok(Request::Ping) => poly_dbus::Message::method_return(&call, vec![]),
                Err(e) => *e,
            };
            if call.wants_reply() {
                bus.send_local(reply);
            }
        }
        for (id, reason) in self.shell.take_closed_notifications() {
            bus.send_local(dn::closed_signal(id, reason));
        }
        for (id, key) in self.shell.take_invoked_actions() {
            println!("poly-compositor: notification {id} action {key:?}");
            bus.send_local(dn::action_signal(id, &key));
        }
        bus.flush();
    }

    /// After events and requests: internal windows' events, the shell's
    /// damage to the scene, the keyboard to the launcher while it shows,
    /// the launcher back on an empty desktop, the status, new windows
    /// logged.
    pub fn sync(&mut self, server: &mut Server) {
        self.internal_events(server);
        let now: BTreeSet<u32> = server.scene().windows().iter().filter(|w| w.is_visible()).map(|w| w.id().0).collect();
        for w in server.scene().windows().iter().filter(|w| w.is_visible() && !self.windows.contains(&w.id().0)) {
            let (x, y) = w.position();
            let (cw, ch) = w.content().logical_size();
            println!("poly-compositor: window {:?} shown at {x},{y} size {cw}x{ch}", w.title());
        }
        if now.is_empty() && !self.windows.is_empty() && !self.shell.launcher().is_visible() {
            self.shell.toggle_launcher();
        }
        self.windows = now;
        let active = server.active_window();
        if active != self.last_active {
            if let Some(w) = active {
                self.mru.touch(w);
            }
            self.last_active = active;
        }
        let t = realtime_secs();
        if self.shell.set_time(t, self.shell.local_offset(t)) && self.settings_win.is_some() {
            self.draw_settings(server);
        }
        let ms = now_ms();
        self.shell.set_now_ms(ms);
        if ms >= self.next_status_ms {
            self.next_status_ms = ms + 2000;
            let net = status::network(&status::interfaces());
            if self.shell.set_network(net) {
                println!("poly-compositor: network {}", status::describe(self.shell.network()));
                self.info.network = status::describe(self.shell.network());
            }
            if self.shell.volume().is_none() {
                self.read_mixer();
            }
        }
        self.serve_notifications();
        for r in self.shell.take_damage() {
            server.scene_mut().damage_logical(r);
        }
        server.set_keyboard_suspended(self.shell.launcher().is_visible());
        self.children.retain(|(pid, name)| match unix::try_wait(*pid) {
            Some(status) => {
                println!("poly-compositor: {name} (pid {pid}) exited with status {status}");
                false
            }
            None => true,
        });
    }

    /// How long the main loop may sleep: the next toast's expiry or the
    /// clock and status (half a second at most).
    pub fn timeout_ms(&self) -> i32 {
        let next = self.shell.next_notification_deadline().map_or(500, |d| d.saturating_sub(now_ms()).min(500));
        next as i32
    }
}

/// A [`MemoryFramebuffer`]'s size (the trait's names, without importing
/// it everywhere).
trait Size {
    fn width_px(&self) -> u32;
    fn height_px(&self) -> u32;
}

impl Size for MemoryFramebuffer {
    fn width_px(&self) -> u32 {
        poly_wayland::compose::Framebuffer::width(self)
    }

    fn height_px(&self) -> u32 {
        poly_wayland::compose::Framebuffer::height(self)
    }
}
