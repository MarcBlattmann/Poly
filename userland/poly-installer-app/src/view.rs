//! The installer's window (ADR 0173), in the Alpine Shell look of the
//! settings window (ADR 0127): on the ground, a package glyph in a rounded
//! box (the installed app's own icon once it is there), the package's name
//! and summary, its facts (version, size, who it is from, what kind of
//! package it is, what it needs), one sentence about what is happening, a
//! progress bar while the package tools run, and the buttons at the bottom
//! right, the main one solid.
//!
//! What the window says and does is `poly_pkg::installer::Installer`'s;
//! this module lays it out, draws it and maps clicks and keys onto it. What
//! touches the system is behind [`Host`], so the window is tested on the
//! host.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use poly_app::ui::{DENSE, DENSE_MEDIUM, FIELD_RADIUS, SMALL, TITLE};
use poly_app::{App, AppEvent, CursorShape, Cx, Ui};
use poly_input::key::Key;
use poly_pkg::installer::{Action, Installer, Phase, Removal, Step};
use poly_shell::appicon::AppIcons;
use poly_shell::draw::Style;
use poly_shell::icons::lucide;
use poly_shell::theme::pal;
use poly_wayland::compose::{Framebuffer, Rect};

/// The window's size, and the smallest it gets.
pub const SIZE: (i32, i32) = (520, 400);
pub const MIN_SIZE: (i32, i32) = (460, 360);

/// The margin around the content, the icon box, a fact's row, a line of
/// the message, the buttons.
pub const MARGIN: i32 = 28;
pub const ICON_BOX: i32 = 56;
pub const FACT_ROW: i32 = 24;
pub const LINE: i32 = 20;
pub const BUTTON_H: i32 = 32;
pub const BUTTON_MIN_W: i32 = 92;

/// How often the window looks at a running command and moves the bar.
pub const TICK_MS: u64 = 120;

/// What a running command wrote since the last look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Lines of its output and error streams.
    pub output: Vec<String>,
    /// Lines of its status descriptor.
    pub status: Vec<String>,
    /// It ended: whether with exit status 0.
    pub ended: Option<bool>,
}

/// What the installer does to the system.
/// What installing an AppImage made: its desktop file ID and the icon file
/// it took out of the image (if it had one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: String,
    pub icon: Option<String>,
}

pub trait Host {
    /// Start the command `argv`.
    fn start(&mut self, argv: &[String]) -> Result<(), String>;
    /// The handles to wait on while it runs, with the events.
    fn handles(&self) -> Vec<(u32, u16)>;
    /// What it wrote, and whether it ended.
    fn pump(&mut self) -> Pumped;
    /// The launcher entries package `name` brought: desktop file ID, the
    /// app's name, its icon's name.
    fn apps_of(&mut self, name: &str) -> Vec<(String, String, Option<String>)>;
    /// Install the AppImage at `path`: copy it into the user's folder and
    /// write the desktop file (and the icon) the launcher reads (ADR 0180).
    fn place_image(&mut self, path: &str, image: &poly_pkg::appimage::Image) -> Result<Placed, String>;
    /// Start the AppImage at `path` from where it is.
    fn run_image(&mut self, path: &str) -> Result<(), String>;
    /// Take an installed AppImage's files away (its directory and desktop
    /// file); `slug` names it (ADR 0180).
    fn delete_image(&mut self, slug: &str) -> Result<(), String>;
    /// Tell the desktop that the installed apps changed.
    fn apps_changed(&mut self);
    /// Ask the desktop to start the app with desktop file ID `id`.
    fn open(&mut self, id: &str);
    /// A line for the console's log: what the installer is doing.
    fn report(&mut self, line: &str);
}

/// How many of a failed command's last lines go to the console.
pub const REPORTED_LINES: usize = 30;

/// The window.
pub struct InstallerApp {
    pub model: Installer,
    host: Box<dyn Host>,
    size: (i32, i32),
    hover: Option<Action>,
    now_ms: u64,
    next_tick: Option<u64>,
    /// The installed app's icon name, and the icons' cache.
    app_icon: Option<String>,
    icons: RefCell<AppIcons>,
    /// The buttons as last drawn: where a click lands.
    buttons: Vec<(Rect, Action)>,
    /// A step the program does itself waits for the next tick (so the
    /// window shows it first).
    work_pending: bool,
}

impl core::fmt::Debug for InstallerApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InstallerApp").field("model", &self.model).finish_non_exhaustive()
    }
}

/// `text` in lines of at most `max_w` pixels, at most `max_lines` (the
/// last one cut with an ellipsis).
pub fn wrap(ui: &Ui, text: &str, st: Style, max_w: i32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { alloc::format!("{cur} {word}") };
        if cur.is_empty() || ui.measure(&candidate, st) <= max_w {
            cur = candidate;
        } else {
            lines.push(core::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max_lines {
        let rest = lines.split_off(max_lines - 1).join(" ");
        lines.push(rest);
    }
    lines.into_iter().map(|l| ui.fit(&l, st, max_w)).collect()
}

impl InstallerApp {
    pub fn new(model: Installer, host: Box<dyn Host>) -> InstallerApp {
        InstallerApp {
            model,
            host,
            size: SIZE,
            hover: None,
            now_ms: 0,
            next_tick: None,
            app_icon: None,
            icons: RefCell::new(AppIcons::default()),
            buttons: Vec::new(),
            work_pending: false,
        }
    }

    /// The icon of the app the window is about (an uninstall shows it
    /// before the app is gone).
    pub fn set_app_icon(&mut self, icon: Option<String>) {
        self.app_icon = icon;
    }

    /// Where app icons are read from (the icon theme's files).
    pub fn set_icon_files(&mut self, files: Box<dyn poly_shell::appicon::IconFiles>) {
        self.icons.borrow_mut().set_files(files);
    }

    /// The window's title.
    pub fn title(&self) -> String {
        let verb = if self.model.uninstalling() { "Uninstall" } else { "Install" };
        alloc::format!("{verb} {}", self.model.name())
    }

    /// The buttons' rectangles, right to left from the bottom right corner,
    /// the main one (the model's last) rightmost.
    pub fn button_rects(&self, ui: &Ui) -> Vec<(Rect, Action)> {
        let mut x = self.size.0 - MARGIN;
        let y = self.size.1 - MARGIN + 6 - BUTTON_H;
        let mut out = Vec::new();
        for (label, action) in self.model.buttons().into_iter().rev() {
            let w = (ui.measure(label, DENSE_MEDIUM) + 36).max(BUTTON_MIN_W);
            x -= w;
            out.push((Rect::new(x, y, w, BUTTON_H), action));
            x -= 10;
        }
        out.reverse();
        out
    }

    fn action_at(&self, x: i32, y: i32) -> Option<Action> {
        self.buttons.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a)
    }

    fn run(&mut self, argv: Option<Vec<String>>, cx: &mut Cx) {
        let Some(argv) = argv else { return };
        if argv.is_empty() {
            // A step the program does itself: after the next look at the
            // window.
            self.work_pending = true;
            self.next_tick = Some(cx.now_ms + TICK_MS);
            cx.redraw();
            return;
        }
        // The program and what it is asked to do (the apt-get verb).
        let verb = argv
            .iter()
            .skip(1)
            .find(|a| matches!(a.as_str(), "update" | "install" | "remove" | "autoremove"))
            .map_or("", String::as_str);
        self.host.report(alloc::format!("running {} {verb}", argv[0]).trim_end());
        if let Err(why) = self.host.start(&argv) {
            self.model.output(&alloc::format!("E: {why}"));
            self.model.finished(false, |_| Vec::new());
            self.report_end();
        }
        self.next_tick = Some(cx.now_ms + TICK_MS);
        cx.redraw();
    }

    /// Do what a button does.
    pub fn act(&mut self, action: Action, cx: &mut Cx) {
        match action {
            Action::Install | Action::Uninstall => {
                let first = self.model.start();
                self.run(first, cx);
            }
            Action::Close => {
                if !self.model.busy() {
                    cx.quit();
                }
            }
            Action::Run => {
                let path = self.model.path.clone();
                match self.host.run_image(&path) {
                    Ok(()) => {
                        self.host.report(&alloc::format!("started {} from {path}", self.model.name()));
                        cx.quit();
                    }
                    Err(why) => {
                        self.model.fail(&alloc::format!("{} could not be started: {why}.", self.model.name()));
                        self.report_end();
                        cx.redraw();
                    }
                }
            }
            Action::Open => {
                if let Some((id, _)) = self.model.apps.first() {
                    self.host.open(&id.clone());
                }
                cx.quit();
            }
        }
    }

    /// Take what the running command wrote; when it ended, go on with the
    /// next command or finish.
    fn pump(&mut self, cx: &mut Cx) {
        if !self.model.busy() {
            return;
        }
        let p = self.host.pump();
        let changed = !p.output.is_empty() || !p.status.is_empty() || p.ended.is_some();
        for line in &p.output {
            self.model.output(line);
        }
        for line in &p.status {
            self.model.status(line);
        }
        if let Some(ok) = p.ended {
            self.step_ended(ok, cx);
        }
        if changed {
            cx.redraw();
        }
    }

    /// The running step ended: go on with the next, or finish.
    fn step_ended(&mut self, ok: bool, cx: &mut Cx) {
        let host = &mut self.host;
        let mut icon = None;
        let next = self.model.finished(ok, |name| {
            let apps = host.apps_of(name);
            icon = apps.first().and_then(|(_, _, icon)| icon.clone());
            apps.into_iter().map(|(id, name, _)| (id, name)).collect()
        });
        if self.model.phase == Phase::Done {
            if !self.model.uninstalling() {
                self.app_icon = icon;
            }
            self.host.apps_changed();
        }
        self.report_end();
        self.run(next, cx);
    }

    /// Install the AppImage: its files are copied and its desktop file
    /// written by the program.
    fn place(&mut self, cx: &mut Cx) {
        let (Some(image), path) = (self.model.image.clone(), self.model.path.clone()) else { return };
        match self.host.place_image(&path, &image) {
            Ok(placed) => {
                self.app_icon = placed.icon;
                let apps = alloc::vec![(placed.id, image.name.clone())];
                self.model.finished(true, |_| apps);
                self.host.apps_changed();
            }
            Err(why) => {
                self.model.output(&alloc::format!("E: {why}"));
                self.model.finished(false, |_| Vec::new());
            }
        }
        self.report_end();
        cx.redraw();
    }

    /// The step the program does itself (an AppImage's files taken away).
    fn do_work(&mut self, cx: &mut Cx) {
        if self.model.phase == Phase::Working(Step::Place) {
            return self.place(cx);
        }
        let result = match (&self.model.phase, &self.model.removal) {
            (Phase::Working(Step::Delete), Some(Removal::Image { slug, .. })) => {
                let slug = slug.clone();
                self.host.delete_image(&slug)
            }
            _ => Err("nothing to do".into()),
        };
        if let Err(why) = &result {
            self.model.output(&alloc::format!("E: {why}"));
        }
        self.step_ended(result.is_ok(), cx);
        cx.redraw();
    }

    /// Tell the console how an installation ended, if it has: a failure
    /// with the tools' last lines.
    fn report_end(&mut self) {
        match &self.model.phase {
            Phase::Done if self.model.uninstalling() => {
                let apps: Vec<String> =
                    self.model.apps.iter().map(|(id, name)| alloc::format!("{name} ({id})")).collect();
                self.host.report(&alloc::format!("uninstalled {}: {}", self.model.name(), apps.join(", ")));
            }
            Phase::Done => {
                let version = match (&self.model.package, &self.model.image) {
                    (Some(p), _) => p.version.as_str(),
                    (None, Some(i)) => i.version.as_str(),
                    _ => "",
                };
                let apps: Vec<String> =
                    self.model.apps.iter().map(|(id, name)| alloc::format!("{name} ({id})")).collect();
                let apps = if apps.is_empty() { "no launcher entry".to_string() } else { apps.join(", ") };
                let name = if version.is_empty() {
                    self.model.name().to_string()
                } else {
                    alloc::format!("{} {version}", self.model.name())
                };
                self.host.report(&alloc::format!("installed {name}: {apps}"));
            }
            Phase::Failed(why) => {
                self.host.report(&alloc::format!("failed: {why}"));
                let from = self.model.log.len().saturating_sub(REPORTED_LINES);
                for line in &self.model.log[from..] {
                    self.host.report(&alloc::format!("| {line}"));
                }
            }
            _ => {}
        }
    }

    fn draw_all(&mut self, ui: &Ui, fb: &mut dyn Framebuffer, clip: Rect) {
        let (w, h) = self.size;
        let p = pal();
        ui.fill(fb, clip, Rect::new(0, 0, w, h), p.s0, 255);

        // The icon box, the name and the summary.
        let icon = Rect::new(MARGIN, MARGIN - 2, ICON_BOX, ICON_BOX);
        ui.round(fb, clip, icon, 14, p.s2, 255);
        ui.border(fb, clip, icon, 14, p.line, 255);
        let s = ui.scale;
        let own = self.app_icon.as_deref().and_then(|name| self.icons.borrow_mut().get(name, (40 * s) as u32));
        match own {
            Some(bitmap) => bitmap.draw(fb, clip, (icon.x + 8) * s, (icon.y + 8) * s, 40 * s),
            None => ui.icon(fb, clip, lucide::PACKAGE, icon.x + 14, icon.y + 14, 28, p.ink_primary, 255),
        }
        let text_x = icon.right() + 16;
        let right = w - MARGIN;
        let name = ui.fit(self.model.name(), TITLE, right - text_x);
        ui.text(fb, clip, text_x, icon.y + 3, 28, right, &name, TITLE, p.ink_primary);
        let summary = match self.model.summary() {
            "" => poly_pkg::installer::file_name(&self.model.path).to_string(),
            s => s.to_string(),
        };
        let summary = ui.fit(&summary, DENSE, right - text_x);
        ui.text(fb, clip, text_x, icon.y + 32, 20, right, &summary, DENSE, p.quiet);

        // The facts.
        let mut y = icon.bottom() + 18;
        for (label, value) in self.model.facts() {
            ui.text(fb, clip, MARGIN, y, FACT_ROW, MARGIN + 90, label, SMALL, p.quiet);
            let value = ui.fit(&value, DENSE, right - (MARGIN + 96));
            ui.text(fb, clip, MARGIN + 96, y, FACT_ROW, right, &value, DENSE, p.ink_primary);
            y += FACT_ROW;
        }

        // The sentence: a failure in the danger colour.
        y += 10;
        let failed = matches!(self.model.phase, Phase::Failed(_) | Phase::Refused(_));
        let ink = if failed { p.danger } else { p.quiet };
        let bar_y = h - MARGIN - BUTTON_H - 22;
        let room = ((bar_y - 8 - y) / LINE).clamp(1, 5) as usize;
        for line in wrap(ui, &self.model.message(), DENSE, w - 2 * MARGIN, room) {
            ui.text(fb, clip, MARGIN, y, LINE, right, &line, DENSE, ink);
            y += LINE;
        }

        // The progress bar: while a command runs, and full when done.
        if self.model.busy() || self.model.phase == Phase::Done {
            let track = Rect::new(MARGIN, bar_y, w - 2 * MARGIN, 6);
            ui.round(fb, clip, track, 3, p.ink, p.track);
            let fill = match self.model.percent {
                Some(pct) => Rect::new(track.x, track.y, (track.width * i32::from(pct.min(100)) / 100).max(6), 6),
                // Not known yet: a piece that runs along the track.
                None => {
                    let piece = track.width / 4;
                    let travel = (track.width - piece).max(1);
                    let at = ((self.now_ms / 6) % (2 * travel as u64)) as i32;
                    let at = if at > travel { 2 * travel - at } else { at };
                    Rect::new(track.x + at, track.y, piece, 6)
                }
            };
            ui.round(fb, clip, fill, 3, p.ink_primary, 255);
        }

        // The buttons: the last is the main one.
        self.buttons = self.button_rects(ui);
        let labels = self.model.buttons();
        let main = labels.len().saturating_sub(1);
        for (i, ((r, action), (label, _))) in self.buttons.iter().zip(&labels).enumerate() {
            let hover = self.hover == Some(*action);
            let solid = i == main && labels.len() > 1 || *action == Action::Open;
            // Removing something is the danger button, as Alpine's settings
            // draw it (its ConfirmButton's confirm state): solid danger,
            // white text.
            let danger = *action == Action::Uninstall;
            if danger {
                ui.round(fb, clip, *r, FIELD_RADIUS, p.danger, if hover { 230 } else { 255 });
            } else if solid {
                ui.round(fb, clip, *r, FIELD_RADIUS, p.ink_primary, if hover { 230 } else { 255 });
            } else {
                ui.round(fb, clip, *r, FIELD_RADIUS, p.ink, if hover { p.track } else { p.ghost });
            }
            let ink = if danger {
                0xff_ffff
            } else if solid {
                p.on_solid
            } else {
                p.ink_primary
            };
            let tw = ui.measure(label, DENSE_MEDIUM);
            ui.text(fb, clip, r.x + (r.width - tw) / 2, r.y, r.height, r.right(), label, DENSE_MEDIUM, ink);
        }
    }
}

impl App for InstallerApp {
    fn event(&mut self, ev: AppEvent, cx: &mut Cx) {
        self.now_ms = cx.now_ms;
        match ev {
            AppEvent::Resized { width, height, .. } => {
                self.size = (width, height);
                cx.redraw();
            }
            AppEvent::Key(k) if k.pressed && !k.repeat => match k.key {
                Some(Key::Enter | Key::NumpadEnter) => {
                    if let Some((_, action)) = self.model.buttons().last().copied() {
                        self.act(action, cx);
                    }
                }
                Some(Key::Escape) => self.act(Action::Close, cx),
                _ => {}
            },
            AppEvent::Key(_) => {}
            AppEvent::Motion { x, y } => {
                let at = self.action_at(x, y);
                if at != self.hover {
                    self.hover = at;
                    cx.redraw();
                }
                cx.cursor(CursorShape::Default);
            }
            AppEvent::Leave => {
                if self.hover.take().is_some() {
                    cx.redraw();
                }
            }
            AppEvent::Button { button, pressed, x, y, .. } => {
                if button == poly_app::BTN_LEFT
                    && !pressed
                    && let Some(action) = self.action_at(x, y)
                {
                    self.act(action, cx);
                }
            }
            AppEvent::Scroll { .. } | AppEvent::Paste(_) => {}
            AppEvent::Focus(_) | AppEvent::Theme => cx.redraw(),
        }
    }

    fn draw(&mut self, fb: &mut dyn Framebuffer, ui: &Ui, _full: bool) -> Option<Vec<Rect>> {
        let all = Rect::new(0, 0, self.size.0 * ui.scale, self.size.1 * ui.scale);
        self.draw_all(ui, fb, all);
        None
    }

    fn handles(&self) -> Vec<(u32, u16)> {
        if self.model.busy() { self.host.handles() } else { Vec::new() }
    }

    fn ready(&mut self, _handle: u32, _revents: u16, cx: &mut Cx) {
        self.now_ms = cx.now_ms;
        self.pump(cx);
    }

    fn deadline(&self) -> Option<u64> {
        if self.model.busy() { self.next_tick } else { None }
    }

    fn tick(&mut self, cx: &mut Cx) {
        self.now_ms = cx.now_ms;
        if self.model.busy() {
            self.next_tick = Some(cx.now_ms + TICK_MS);
            if core::mem::take(&mut self.work_pending) {
                self.do_work(cx);
                return;
            }
            self.pump(cx);
            // The piece of an unknown progress moves.
            if self.model.percent.is_none() {
                cx.redraw();
            }
        }
    }

    /// The window's close button: not while the package tools run (`dpkg`
    /// must not be cut off half way).
    fn close(&mut self, cx: &mut Cx) {
        if !self.model.busy() {
            cx.quit();
        }
    }
}
