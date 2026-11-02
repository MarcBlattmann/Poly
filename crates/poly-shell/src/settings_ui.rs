//! The settings window (ADR 0117), drawn by the shell into a window the
//! compositor frames like any other: Alpine Shell's settings app in its
//! dark theme as its screenshots show it (ADR 0127) — the title in Averia
//! Serif Libre and the sections' pages in a sidebar on the ground colour,
//! back and forward arrows with where we are, the page on a lighter
//! surface with a rounded top-left corner and a hairline edge, a display
//! title, headed sections of rows with their controls on the right
//! (Alpine's outline segmented choices, slider, pill toggle), cards of
//! choices with a check mark, wallpaper tiles.
//!
//! ```text
//!  ┌────────────────────────────────────────────────────────────────┐
//!  │ Settings         ← →  Sound                                    │ ← header, 56
//!  │ PERSONAL        ╭──────────────────────────────────────────────┤
//!  │ ▨ Wallpaper     │ Sound                                        │ ← title, Averia 24 px
//!  │ DEVICES         │ Output                                       │ ← heading, 14 px semibold
//!  │ ▣ Displays      │ Volume                      ━━━━━━━──── 60%  │ ← rows: label 14 px,
//!  │ ⌨ Keyboard      │ virtio-sound                                 │   description 12 px
//!  │ ◖ Sound         │ Mute                                   (● )  │
//!  └─────────────────┴──────────────────────────────────────────────┘
//!    sidebar 224         page: gutter 20
//! ```
//!
//! Everything is laid out once per draw as a list of [`Item`]s with the
//! [`Target`]s the pointer can hit, so drawing and hit-testing agree.
//! Coordinates are logical, relative to the window's content; drawing
//! multiplies them by the output's scale.

use crate::draw::{Face, Style, Text, fill, rounded_border, rounded_outline, rounded_rect};
use crate::icons::{Icon, lucide};
use crate::settings::{Change, KEYBOARDS, SCALES, Settings};
use crate::theme::settings as t;
use crate::theme::wallpaper::PRESETS;
use crate::tz::{TimeZone, ZONES};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use poly_wayland::compose::paint::{self, Radii, Shadow};
use poly_wayland::compose::{Framebuffer, Rect};

const WHITE: u32 = 0xff_ffff;

/// The pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Display,
    Keyboard,
    Wallpaper,
    Sound,
    DateTime,
    About,
}

impl Page {
    pub const ALL: [Page; 6] =
        [Page::Display, Page::Keyboard, Page::Wallpaper, Page::Sound, Page::DateTime, Page::About];

    pub fn label(self) -> &'static str {
        match self {
            Page::Display => "Displays",
            Page::Keyboard => "Keyboard",
            Page::Wallpaper => "Wallpaper",
            Page::Sound => "Sound",
            Page::DateTime => "Date & time",
            Page::About => "About",
        }
    }

    /// Alpine's icon for the page (its settings registry's).
    fn icon(self) -> Icon {
        match self {
            Page::Display => lucide::MONITOR,
            Page::Keyboard => lucide::KEYBOARD,
            Page::Wallpaper => lucide::WALLPAPER,
            Page::Sound => lucide::VOLUME_2,
            Page::DateTime => lucide::CALENDAR_CLOCK,
            Page::About => lucide::INFO,
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }
}

/// The sidebar: Alpine's sections with Poly's pages.
const SECTIONS: [(&str, &[Page]); 3] = [
    ("PERSONAL", &[Page::Wallpaper]),
    ("DEVICES", &[Page::Display, Page::Keyboard, Page::Sound]),
    ("SYSTEM", &[Page::DateTime, Page::About]),
];

/// What the settings show about the system (the compositor finds it out).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemInfo {
    /// "Poly 0.0.1".
    pub os: String,
    /// The kernel's name, release and version (`uname`).
    pub kernel: String,
    pub machine: String,
    pub cpu: String,
    pub memory_kib: u64,
    /// The display's mode now, and the modes it offers.
    pub display: (u32, u32),
    pub modes: Vec<(u32, u32)>,
    /// The sound card with the master volume, if there is one.
    pub sound: Option<String>,
    pub network: String,
    pub session_bus: String,
}

/// What the pointer can hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nav(Page),
    Back,
    Forward,
    Mode(usize),
    Scale(u32),
    Keyboard(usize),
    Wallpaper(usize),
    Volume,
    Mute,
    Zone(usize),
}

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    /// Text with its baseline at `y`, cut at `max_x`.
    Text {
        x: i32,
        y: i32,
        max_x: i32,
        text: String,
        style: Style,
        rgb: u32,
    },
    /// A card: surface 2 with a hairline, rounded.
    Card(Rect),
    /// A hairline.
    Line(Rect),
    /// A hover or selection fill.
    Fill {
        rect: Rect,
        rgb: u32,
        alpha: u8,
        radius: i32,
    },
    /// Outline segmented choices: (option, label, active).
    Segmented {
        rect: Rect,
        options: Vec<(Rect, String, bool)>,
    },
    Slider {
        rect: Rect,
        value: u32,
        enabled: bool,
    },
    Toggle {
        rect: Rect,
        on: bool,
    },
    Tile {
        rect: Rect,
        id: &'static str,
        selected: bool,
    },
    Icon {
        icon: Icon,
        x: i32,
        y: i32,
        size: i32,
        rgb: u32,
        alpha: u8,
    },
}

/// The window's state.
#[derive(Debug)]
pub struct SettingsWindow {
    pub page: Page,
    hover: Option<Target>,
    dragging: bool,
    /// The pages visited and where we are in them (the arrows).
    history: Vec<Page>,
    at: usize,
}

impl Default for SettingsWindow {
    fn default() -> SettingsWindow {
        SettingsWindow { page: Page::Display, hover: None, dragging: false, history: vec![Page::Display], at: 0 }
    }
}

/// What the window needs to lay itself out.
#[derive(Debug)]
pub struct Context<'a> {
    pub settings: &'a Settings,
    pub info: &'a SystemInfo,
    pub text: &'a Text,
    /// The local time as the ridge shows it, and the zone's abbreviation.
    pub now: &'a str,
    /// Unix seconds now (for the zones' offsets).
    pub utc: i64,
}

/// `+hh:mm` of an offset in minutes east of UTC.
pub fn utc_offset(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let m = minutes.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
}

fn mib(kib: u64) -> String {
    if kib >= 1024 * 1024 {
        format!("{:.1} GiB", kib as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} MiB", kib / 1024)
    }
}

/// Builds a page's items. `y` is where the next thing's capitals start.
struct Builder<'a, 'b> {
    cx: &'a Context<'b>,
    items: Vec<Item>,
    targets: Vec<(Rect, Target)>,
    hover: Option<Target>,
    left: i32,
    right: i32,
    y: i32,
}

const LABEL: Style = Style::new(Face::Sans, t::LABEL_SIZE);
const DESC: Style = Style::new(Face::Sans, t::SMALL_SIZE);
const DENSE: Style = Style::new(Face::Sans, t::DENSE_SIZE);

impl Builder<'_, '_> {
    fn text(&mut self, x: i32, y: i32, max_x: i32, text: &str, style: Style, rgb: u32) {
        self.items.push(Item::Text { x, y, max_x, text: text.into(), style, rgb });
    }

    /// The cap height of `st`, rounded.
    fn cap(&self, st: Style) -> i32 {
        (self.cx.text.cap(st) + 0.5) as i32
    }

    /// The baseline that centres a line of `st` in `top..top + h`.
    fn baseline(&self, top: i32, h: i32, st: Style) -> i32 {
        self.cx.text.centre(top, h, st)
    }

    /// A section heading (14 px semibold); the first row's label starts
    /// 43 pixels lower.
    fn heading(&mut self, label: &str) {
        let st = Style::new(Face::Semibold, t::HEADING_SIZE);
        let b = self.y + self.cap(st);
        self.text(self.left, b, self.right, label, st, t::INK);
        self.y += 43;
    }

    /// A row's label (14 px) with a description under it (12 px, quiet):
    /// the line of the label (its top, 20 pixels high) for the control.
    fn row(&mut self, label: &str, desc: Option<&str>) -> Rect {
        let b = self.y + self.cap(LABEL);
        let max = self.right - 200;
        self.text(self.left, b, max, label, LABEL, t::INK);
        let line = Rect::new(self.left, self.y - 5, self.right - self.left, 20);
        match desc {
            Some(d) => {
                self.text(self.left, self.y + 20 + self.cap(DESC), max, d, DESC, t::INK_QUIET);
                self.y += t::ROW_PITCH;
            }
            None => self.y += 44,
        }
        line
    }

    /// After a section's last row: the next heading 14 pixels further.
    fn end_section(&mut self) {
        self.y += 14;
    }

    /// Outline segmented choices at the right of a row's line: (label,
    /// active, target).
    fn segmented(&mut self, line: Rect, options: &[(String, bool, Target)]) {
        let widths: Vec<i32> = options.iter().map(|o| self.cx.text.measure(&o.0, DENSE) as i32 + 24).collect();
        let total: i32 = widths.iter().sum::<i32>() + 10;
        let rect = Rect::new(self.right - total, line.y + line.height / 2 - 19, total, 38);
        let mut x = rect.x + 5;
        let mut opts = Vec::new();
        for (o, w) in options.iter().zip(widths) {
            let r = Rect::new(x, rect.y + 5, w, 28);
            opts.push((r, o.0.clone(), o.1));
            self.targets.push((r, o.2));
            x += w;
        }
        self.items.push(Item::Segmented { rect, options: opts });
    }

    /// A pill toggle at the right, its top on the label's capitals.
    fn toggle(&mut self, line: Rect, on: bool, target: Option<Target>) {
        let rect = Rect::new(self.right - 38, line.y + 5 - 1, 38, 22);
        self.items.push(Item::Toggle { rect, on });
        if let Some(tg) = target {
            self.targets.push((rect, tg));
        }
    }

    /// A slider at the right with its value after it.
    fn slider(&mut self, line: Rect, value: u32, suffix: &str, target: Option<Target>) {
        let label_w = 40;
        let track = Rect::new(self.right - label_w - 12 - 176, line.y + line.height / 2 - 4, 176, 8);
        self.items.push(Item::Slider { rect: track, value, enabled: target.is_some() });
        let v = format!("{value}{suffix}");
        let w = self.cx.text.measure(&v, DESC) as i32;
        let b = self.baseline(line.y, line.height, DESC);
        self.text(self.right - w, b, self.right + 1, &v, DESC, t::INK_QUIET);
        if let Some(tg) = target {
            self.targets.push((Rect::new(track.x - 6, line.y - 8, track.width + 12, line.height + 16), tg));
        }
    }

    /// A card of choices (48-pixel rows with hairlines between, 36 for
    /// long lists): label, detail on the right, a check if chosen.
    fn choices(&mut self, rows: &[(String, String, bool, Target)]) {
        let top = self.y - 8;
        let rh = if rows.len() > 4 { 36 } else { 48 };
        let card = Rect::new(self.left, top, self.right - self.left, rows.len() as i32 * rh);
        self.items.push(Item::Card(card));
        for (i, (label, detail, chosen, target)) in rows.iter().enumerate() {
            let r = Rect::new(card.x, top + i as i32 * rh, card.width, rh);
            if i > 0 {
                self.items.push(Item::Line(Rect::new(r.x + 1, r.y, r.width - 2, 1)));
            }
            if self.hover == Some(*target) {
                let radius = if i == 0 || i == rows.len() - 1 { t::CARD_RADIUS } else { 0 };
                self.items.push(Item::Fill { rect: r.grow(-1), rgb: WHITE, alpha: t::GHOST_ALPHA, radius });
            }
            let b = self.baseline(r.y, r.height, DENSE);
            self.text(r.x + t::ROW_PADDING, b, r.right() - 140, label, DENSE, t::INK);
            let check_x = r.right() - t::ROW_PADDING - 16;
            if *chosen {
                self.items.push(Item::Icon {
                    icon: lucide::CHECK,
                    x: check_x,
                    y: r.y + (rh - 16) / 2,
                    size: 16,
                    rgb: t::INK,
                    alpha: 255,
                });
            }
            if !detail.is_empty() {
                let w = self.cx.text.measure(detail, DESC) as i32;
                let b = self.baseline(r.y, r.height, DESC);
                self.text(check_x - 12 - w, b, check_x - 4, detail, DESC, t::INK_QUIET);
            }
            self.targets.push((r, *target));
        }
        self.y = card.bottom() + 8 + 32;
    }

    /// A card of label/value rows.
    fn facts(&mut self, rows: &[(&str, String)]) {
        let top = self.y - 8;
        let card = Rect::new(self.left, top, self.right - self.left, rows.len() as i32 * 44);
        self.items.push(Item::Card(card));
        for (i, (label, value)) in rows.iter().enumerate() {
            let r = Rect::new(card.x, top + i as i32 * 44, card.width, 44);
            if i > 0 {
                self.items.push(Item::Line(Rect::new(r.x + 1, r.y, r.width - 2, 1)));
            }
            let b = self.baseline(r.y, r.height, DENSE);
            self.text(r.x + t::ROW_PADDING, b, r.x + 140, label, DENSE, t::INK_QUIET);
            let room = (r.right() - t::ROW_PADDING - (r.x + 150)) as f32;
            let v = self.cx.text.fit_style(value, DENSE, room);
            let w = self.cx.text.measure(&v, DENSE) as i32;
            self.text(r.right() - t::ROW_PADDING - w, b, r.right(), &v, DENSE, t::INK);
        }
        self.y = card.bottom() + 8 + 32;
    }

    /// A note under a section (12 px, faint).
    fn note(&mut self, text: &str) {
        let b = self.y + self.cap(DESC);
        self.text(self.left, b, self.right, text, DESC, t::INK_FAINT);
        self.y += 30;
    }
}

impl SettingsWindow {
    /// The sidebar's rows: (row, page).
    fn nav(&self) -> Vec<(Rect, Page)> {
        let mut out = Vec::new();
        // The first section label where Alpine's second is, under "Home".
        let mut y = t::HEADER - 2;
        for (_, pages) in SECTIONS {
            y += t::EYEBROW_GAP;
            for &p in pages {
                out.push((Rect::new(t::NAV_PADDING, y, t::SIDEBAR - 2 * t::NAV_PADDING, t::NAV_HEIGHT), p));
                y += t::NAV_HEIGHT + t::NAV_GAP;
            }
            y -= t::NAV_GAP;
        }
        out
    }

    /// The page's items and the pointer's targets, for a window `w` × `h`.
    fn layout(&self, w: i32, h: i32, cx: &Context<'_>) -> (Vec<Item>, Vec<(Rect, Target)>) {
        let main_top = t::HEADER + 1;
        let mut b = Builder {
            cx,
            items: Vec::new(),
            targets: Vec::new(),
            hover: self.hover,
            left: t::SIDEBAR + 1 + t::GUTTER,
            right: w - t::GUTTER,
            y: main_top + 26,
        };
        // The header: the app's name in Averia, the arrows, where we are.
        let app = Style::new(Face::DisplayBold, t::APP_TITLE_SIZE);
        let ab = 22 + b.cap(app);
        b.text(20, ab, t::SIDEBAR - 8, "Settings", app, t::INK);
        let arrows = [
            (Target::Back, lucide::ARROW_LEFT, self.at > 0),
            (Target::Forward, lucide::ARROW_RIGHT, self.at + 1 < self.history.len()),
        ];
        for (i, (target, icon, enabled)) in arrows.into_iter().enumerate() {
            let r = Rect::new(t::SIDEBAR + 14 + i as i32 * 32, (t::HEADER - 28) / 2, 28, 28);
            if enabled && self.hover == Some(target) {
                b.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius: t::FIELD_RADIUS });
            }
            let ink = if enabled { t::INK } else { t::INK_FAINT };
            b.items.push(Item::Icon { icon, x: r.x + 7, y: r.y + 7, size: 15, rgb: ink, alpha: 255 });
            if enabled {
                b.targets.push((r, target));
            }
        }
        let crumb = self.page.label();
        let cb = 22 + b.cap(DENSE);
        let cxp = t::SIDEBAR + 84;
        b.text(cxp, cb, w - 16, crumb, DENSE, t::INK);
        let cw = cx.text.measure(crumb, DENSE) as i32;
        b.items.push(Item::Fill { rect: Rect::new(cxp, cb + 3, cw, 1), rgb: t::UNDERLINE, alpha: 255, radius: 0 });
        // The sidebar: section labels (11 px semibold, faint, spaced) and
        // rows (13 px, the active one medium on surface 3).
        let eyebrow = Style::new(Face::Semibold, t::EYEBROW_SIZE).tracking(0.05);
        let mut y = t::HEADER - 2;
        for (label, pages) in SECTIONS {
            b.text(20, y + 18 + b.cap(eyebrow), t::SIDEBAR - 8, label, eyebrow, t::INK_FAINT);
            y += t::EYEBROW_GAP + pages.len() as i32 * (t::NAV_HEIGHT + t::NAV_GAP) - t::NAV_GAP;
        }
        for (r, page) in self.nav() {
            let active = page == self.page;
            if active {
                b.items.push(Item::Fill { rect: r, rgb: t::SURFACE_3, alpha: 255, radius: t::FIELD_RADIUS });
            } else if self.hover == Some(Target::Nav(page)) {
                b.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius: t::FIELD_RADIUS });
            }
            let (ink, alpha) = if active { (t::INK, 230) } else { (t::INK_QUIET, 153) };
            let iy = r.y + (r.height - t::NAV_ICON) / 2;
            b.items.push(Item::Icon {
                icon: page.icon(),
                x: r.x + t::NAV_INSET,
                y: iy,
                size: t::NAV_ICON,
                rgb: ink,
                alpha,
            });
            let st = if active { Style::new(Face::Medium, t::DENSE_SIZE) } else { DENSE };
            let tb = b.baseline(r.y, r.height, st);
            b.text(r.x + t::NAV_INSET + t::NAV_ICON + 10, tb, r.right() - 8, page.label(), st, ink);
            b.targets.push((r, Target::Nav(page)));
        }
        // The page: its title in Averia, then its sections.
        let title = Style::new(Face::Display, t::TITLE_SIZE).tracking(-0.025);
        let tb = b.y + b.cap(title);
        b.text(b.left, tb, b.right, self.page.label(), title, t::INK);
        b.y += 62;
        let st = cx.settings;
        match self.page {
            Page::Display => {
                b.heading("Display");
                let mut modes = cx.info.modes.clone();
                if modes.is_empty() {
                    modes.push(cx.info.display);
                }
                let current = st.resolution.unwrap_or(cx.info.display);
                let line = b.row("Resolution", Some("The display's mode"));
                let opts: Vec<(String, bool, Target)> = modes
                    .iter()
                    .enumerate()
                    .take(3)
                    .map(|(i, &(mw, mh))| (format!("{mw}\u{d7}{mh}"), (mw, mh) == current, Target::Mode(i)))
                    .collect();
                b.segmented(line, &opts);
                let line = b.row("Scale", Some("The size of everything on the screen"));
                let opts: Vec<(String, bool, Target)> =
                    SCALES.iter().map(|&s| (format!("{}%", s * 100), s == st.scale, Target::Scale(s))).collect();
                b.segmented(line, &opts);
                if modes.len() <= 1 {
                    b.end_section();
                    b.note("Other modes come with the KMS display path.");
                }
            }
            Page::Keyboard => {
                b.heading("Keyboard layout");
                let rows: Vec<(String, String, bool, Target)> = KEYBOARDS
                    .iter()
                    .enumerate()
                    .map(|(i, (id, label))| {
                        (label.to_string(), id.to_string(), st.keyboard == *id, Target::Keyboard(i))
                    })
                    .collect();
                b.choices(&rows);
                b.note("Every window gets the new layout at once.");
            }
            Page::Wallpaper => {
                b.heading("Wallpaper");
                let cols = 3;
                let gap = 12;
                let tw = (b.right - b.left - gap * (cols - 1)) / cols;
                let th = tw * 9 / 16;
                let top = b.y - 8;
                for (i, p) in PRESETS.iter().enumerate() {
                    let (c, r) = (i as i32 % cols, i as i32 / cols);
                    let rect = Rect::new(b.left + c * (tw + gap), top + r * (th + 30), tw, th);
                    b.items.push(Item::Tile { rect, id: p.0, selected: st.wallpaper == p.0 });
                    let lb = rect.bottom() + 8 + b.cap(DESC);
                    let ink = if st.wallpaper == p.0 { t::INK } else { t::INK_QUIET };
                    b.text(rect.x, lb, rect.right(), p.1, DESC, ink);
                    b.targets.push((Rect::new(rect.x, rect.y, rect.width, rect.height + 24), Target::Wallpaper(i)));
                }
            }
            Page::Sound => {
                b.heading("Output");
                let card = cx.info.sound.clone().unwrap_or_else(|| "No sound card found".into());
                let enabled = cx.info.sound.is_some();
                let line = b.row("Volume", Some(&card));
                b.slider(line, st.volume, "%", enabled.then_some(Target::Volume));
                let line = b.row("Mute", Some("Silence every program"));
                b.toggle(line, st.muted, enabled.then_some(Target::Mute));
            }
            Page::DateTime => {
                b.heading("Clock");
                let line = b.row("Local time", None);
                let v = cx.now;
                let w = cx.text.measure(v, DENSE) as i32;
                let vb = b.baseline(line.y, line.height, DENSE);
                b.text(b.right - w, vb, b.right + 1, v, DENSE, t::INK_QUIET);
                b.end_section();
                b.heading("Time zone");
                let rows: Vec<(String, String, bool, Target)> = ZONES
                    .iter()
                    .enumerate()
                    .map(|(i, (name, label, rule))| {
                        let off = TimeZone::parse(rule).map_or(0, |z| z.offset_minutes(cx.utc));
                        (label.to_string(), utc_offset(off), st.time_zone == *name, Target::Zone(i))
                    })
                    .collect();
                b.choices(&rows);
            }
            Page::About => {
                let i = cx.info;
                b.heading("System");
                let rows: Vec<(&str, String)> = vec![
                    ("System", i.os.clone()),
                    ("Kernel", i.kernel.clone()),
                    (
                        "Processor",
                        if i.cpu.is_empty() { i.machine.clone() } else { format!("{} ({})", i.cpu, i.machine) },
                    ),
                    ("Memory", mib(i.memory_kib)),
                    ("Display", format!("{}\u{d7}{} at {}%", i.display.0, i.display.1, st.scale * 100)),
                    ("Sound", i.sound.clone().unwrap_or_else(|| "none".into())),
                    ("Network", i.network.clone()),
                    ("Session bus", i.session_bus.clone()),
                ];
                b.facts(&rows);
            }
        }
        let _ = h;
        (b.items, b.targets)
    }

    /// The target at logical (`x`, `y`).
    fn target_at(&self, x: i32, y: i32, w: i32, h: i32, cx: &Context<'_>) -> Option<Target> {
        let (_, targets) = self.layout(w, h, cx);
        targets.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, t)| *t)
    }

    /// The volume at pointer `x` on the slider.
    fn volume_at(&self, x: i32, w: i32, h: i32, cx: &Context<'_>) -> Option<u32> {
        let (items, _) = self.layout(w, h, cx);
        items.iter().find_map(|i| match i {
            Item::Slider { rect, .. } => Some(((x - rect.x) * 100 / rect.width.max(1)).clamp(0, 100) as u32),
            _ => None,
        })
    }

    /// The pointer moved to (`x`, `y`): the hover changes, a drag moves
    /// the volume. Returns whether to draw again, and a change.
    pub fn motion(&mut self, x: i32, y: i32, w: i32, h: i32, cx: &Context<'_>) -> (bool, Option<Change>) {
        if self.dragging {
            let v = self.volume_at(x, w, h, cx);
            return match v.filter(|&v| v != cx.settings.volume) {
                Some(v) => (true, Some(Change::Volume(v))),
                None => (false, None),
            };
        }
        let hover = self.target_at(x, y, w, h, cx);
        let redraw = hover != self.hover;
        self.hover = hover;
        (redraw, None)
    }

    /// The pointer left the window.
    pub fn leave(&mut self) -> bool {
        self.hover.take().is_some()
    }

    /// Go to page `p` (remembered for the arrows).
    fn go(&mut self, p: Page) -> bool {
        if p == self.page {
            return false;
        }
        self.history.truncate(self.at + 1);
        self.history.push(p);
        self.at = self.history.len() - 1;
        self.page = p;
        true
    }

    /// A left button press or release at (`x`, `y`).
    pub fn button(
        &mut self,
        x: i32,
        y: i32,
        pressed: bool,
        w: i32,
        h: i32,
        cx: &Context<'_>,
    ) -> (bool, Option<Change>) {
        if !pressed {
            self.dragging = false;
            return (false, None);
        }
        let Some(target) = self.target_at(x, y, w, h, cx) else { return (false, None) };
        let st = cx.settings;
        let change = match target {
            Target::Nav(p) => return (self.go(p), None),
            Target::Back | Target::Forward => {
                let at = if target == Target::Back { self.at.checked_sub(1) } else { Some(self.at + 1) };
                if let Some(&p) = at.and_then(|a| self.history.get(a)) {
                    self.at = at.unwrap_or(0);
                    self.page = p;
                    return (true, None);
                }
                return (false, None);
            }
            Target::Mode(i) => {
                let mut modes = cx.info.modes.clone();
                if modes.is_empty() {
                    modes.push(cx.info.display);
                }
                modes.get(i).map(|&(mw, mh)| Change::Resolution(mw, mh))
            }
            Target::Scale(s) => Some(Change::Scale(s)),
            Target::Keyboard(i) => KEYBOARDS.get(i).map(|k| Change::Keyboard(k.0.to_string())),
            Target::Wallpaper(i) => PRESETS.get(i).map(|p| Change::Wallpaper(p.0.to_string())),
            Target::Volume => {
                self.dragging = true;
                self.volume_at(x, w, h, cx).map(Change::Volume)
            }
            Target::Mute => Some(Change::Muted(!st.muted)),
            Target::Zone(i) => ZONES.get(i).map(|z| Change::TimeZone(z.0.to_string())),
        };
        (true, change)
    }

    /// Up or Down: the previous or next page.
    pub fn step_page(&mut self, forward: bool) {
        let i = self.page.index();
        let n = Page::ALL.len();
        let p = Page::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }];
        self.go(p);
    }

    /// Draw the window, `w` × `h` logical at scale `s`, into `fb` (which is
    /// at least that big, physical).
    pub fn draw(&self, fb: &mut dyn Framebuffer, w: i32, h: i32, s: i32, cx: &Context<'_>) {
        let all = Rect::new(0, 0, w * s, h * s);
        fill(fb, all, all, t::SURFACE_0, 255);
        // The page's surface: rounded at the top left, a hairline along its
        // top and left edges.
        let page = Rect::new(t::SIDEBAR, t::HEADER, w - t::SIDEBAR, h - t::HEADER).scale(s);
        let r = (t::CARD_RADIUS * s) as f32;
        let grown = Rect::new(page.x, page.y, page.width + 2 * r as i32, page.height + 2 * r as i32);
        paint::fill_rounded(fb, page, grown, Radii::all(r), t::LINE, 255);
        let inner = Rect::new(grown.x + s, grown.y + s, grown.width - s, grown.height - s);
        paint::fill_rounded(fb, page, inner, Radii::all(r - s as f32), t::SURFACE_1, 255);
        let (items, _) = self.layout(w, h, cx);
        let text = cx.text;
        for item in &items {
            match item {
                Item::Text { x, y, max_x, text: s_, style, rgb } => {
                    text.draw_style(fb, all, (x * s) as f32, y * s, max_x * s, s_, style.scaled(s), *rgb, 255);
                }
                Item::Card(c) => {
                    let c = c.scale(s);
                    rounded_rect(fb, all, c, t::CARD_RADIUS * s, t::SURFACE_2, 255);
                    rounded_border(fb, all, c, t::CARD_RADIUS * s, s, t::LINE, 255);
                }
                Item::Line(l) => fill(fb, all, Rect::new(l.x * s, l.y * s, l.width * s, s), t::LINE, 255),
                Item::Fill { rect, rgb, alpha, radius } => {
                    rounded_rect(fb, all, rect.scale(s), radius * s, *rgb, *alpha)
                }
                Item::Segmented { rect, options } => {
                    let rs = rect.scale(s);
                    rounded_outline(fb, all, rs, rs.height / 2, t::LINE, 255);
                    for (r, label, active) in options {
                        let st = DENSE.scaled(s);
                        let o = r.scale(s);
                        if *active {
                            rounded_rect(fb, all, o, o.height / 2, t::INK, 255);
                        }
                        let ink = if *active { t::SURFACE_0 } else { t::INK_QUIET };
                        let lw = text.measure(label, st);
                        let base = text.centre(o.y, o.height, st);
                        text.draw_style(
                            fb,
                            all,
                            o.x as f32 + (o.width as f32 - lw) / 2.0,
                            base,
                            o.right(),
                            label,
                            st,
                            ink,
                            255,
                        );
                    }
                }
                Item::Slider { rect, value, enabled } => {
                    let r = rect.scale(s);
                    let a = if *enabled { 255 } else { 110 };
                    rounded_rect(fb, all, r, r.height / 2, WHITE, t::TRACK_ALPHA);
                    let filled = Rect::new(r.x, r.y, r.width * *value as i32 / 100, r.height);
                    if filled.width > 0 {
                        rounded_rect(fb, all.intersect(&filled), r, r.height / 2, t::INK, a);
                    }
                }
                Item::Toggle { rect, on } => {
                    let r = rect.scale(s);
                    let (track, track_a, knob, kx) = if *on {
                        (t::INK, 255, t::SURFACE_0, r.x + 19 * s)
                    } else {
                        (WHITE, t::TRACK_ALPHA, t::KNOB_OFF, r.x + 3 * s)
                    };
                    rounded_rect(fb, all, r, r.height / 2, track, track_a);
                    let k = Rect::new(kx, r.y + 3 * s, 16 * s, 16 * s);
                    // Alpine's knob shadow, `0 1px 2px rgba(0,0,0,.25)`.
                    Shadow::new(0.0, 1.0, 2.0, 0.0, 0, 64).scaled(s).draw(
                        fb,
                        all,
                        k,
                        Radii::all(8.0 * s as f32),
                        Rect::default(),
                    );
                    rounded_rect(fb, all, k, 8 * s, knob, 255);
                }
                Item::Tile { rect, id, selected } => {
                    let r = rect.scale(s);
                    let mut tile = TileFb { fb, origin: (r.x, r.y), size: (r.width, r.height) };
                    let full = Rect::new(0, 0, r.width, r.height);
                    crate::shell::draw_wallpaper(&mut tile, full, id, r.width, r.height);
                    // Cut the corners round: the page's colour over them.
                    corner_mask(fb, all, r, 8 * s);
                    if *selected {
                        rounded_border(fb, all, r.grow(3 * s), 11 * s, 2 * s, t::INK, 255);
                    } else {
                        rounded_border(fb, all, r, 8 * s, s, t::LINE, 255);
                    }
                }
                Item::Icon { icon, x, y, size, rgb, alpha } => {
                    icon.draw(fb, all, (x * s) as f32, (y * s) as f32, (size * s) as f32, *rgb, *alpha);
                }
            }
        }
    }
}

/// Paint the corners of `r` outside a radius-`rad` rounding in the page's
/// colour (wallpaper tiles are drawn square, then rounded this way).
fn corner_mask(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32) {
    let b = paint::Box2::from_rect(r);
    for (cx, cy) in [(r.x, r.y), (r.right() - rad, r.y), (r.x, r.bottom() - rad), (r.right() - rad, r.bottom() - rad)] {
        let area = Rect::new(cx, cy, rad, rad).intersect(&clip);
        for y in area.y..area.bottom() {
            let row = fb.row_mut(y as u32);
            for x in area.x..area.right() {
                let cov = paint::rounded_coverage(&b, Radii::all(rad as f32), x, y);
                let a = ((1.0 - cov) * 255.0 + 0.5) as u32;
                if a > 0 {
                    row[x as usize] = paint::mix(row[x as usize], t::SURFACE_1, a);
                }
            }
        }
    }
}

/// A window onto part of a framebuffer, for drawing a wallpaper tile with
/// the wallpaper's own code.
struct TileFb<'a> {
    fb: &'a mut dyn Framebuffer,
    origin: (i32, i32),
    size: (i32, i32),
}

impl Framebuffer for TileFb<'_> {
    fn width(&self) -> u32 {
        self.size.0 as u32
    }

    fn height(&self) -> u32 {
        self.size.1 as u32
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        let (ox, oy) = self.origin;
        let w = self.size.0 as usize;
        let row = self.fb.row_mut((oy + y as i32) as u32);
        &mut row[ox as usize..ox as usize + w]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::compose::MemoryFramebuffer;

    fn info() -> SystemInfo {
        SystemInfo {
            os: "Poly 0.0.1".into(),
            kernel: "Poly 0.0.1".into(),
            machine: "x86_64".into(),
            cpu: "QEMU Virtual CPU".into(),
            memory_kib: 512 * 1024,
            display: (1280, 800),
            modes: vec![(1280, 800)],
            sound: Some("virtio-sound".into()),
            network: "eth0 up".into(),
            session_bus: "unix:path=/run/user/0/bus".into(),
        }
    }

    fn cx<'a>(st: &'a Settings, i: &'a SystemInfo, text: &'a Text) -> Context<'a> {
        Context { settings: st, info: i, text, now: "12:00 Nov 1", utc: 1_793_491_200 }
    }

    const W: i32 = t::WIDTH;
    const H: i32 = t::HEIGHT;

    /// The centre of the first target `want`.
    fn at(win: &SettingsWindow, c: &Context<'_>, want: Target) -> (i32, i32) {
        let (_, targets) = win.layout(W, H, c);
        let r = targets.iter().find(|(_, t)| *t == want).unwrap_or_else(|| panic!("{want:?}")).0;
        (r.x + r.width / 2, r.y + r.height / 2)
    }

    #[test]
    fn pages_and_changes() {
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        // The sidebar switches pages.
        let (x, y) = at(&win, &c, Target::Nav(Page::Sound));
        assert_eq!(win.button(x, y, true, W, H, &c), (true, None));
        assert_eq!(win.page, Page::Sound);
        win.button(x, y, false, W, H, &c);
        // The volume slider: a press at its middle is 50 %, a drag moves it.
        let (sx, sy) = at(&win, &c, Target::Volume);
        let (_, change) = win.button(sx, sy, true, W, H, &c);
        assert_eq!(change, Some(Change::Volume(50)));
        let (_, change) = win.motion(sx + 1000, sy, W, H, &c);
        assert_eq!(change, Some(Change::Volume(100)));
        win.button(sx, sy, false, W, H, &c);
        let (mx, my) = at(&win, &c, Target::Mute);
        assert_eq!(win.button(mx, my, true, W, H, &c).1, Some(Change::Muted(true)));
        // The keyboard layouts.
        win.page = Page::Keyboard;
        let (kx, ky) = at(&win, &c, Target::Keyboard(1));
        assert_eq!(win.button(kx, ky, true, W, H, &c).1, Some(Change::Keyboard("de-ch".into())));
        // Wallpapers, time zones, the scale.
        win.page = Page::Wallpaper;
        let (wx, wy) = at(&win, &c, Target::Wallpaper(3));
        assert_eq!(win.button(wx, wy, true, W, H, &c).1, Some(Change::Wallpaper("forest".into())));
        win.page = Page::DateTime;
        let (zx, zy) = at(&win, &c, Target::Zone(1));
        assert_eq!(win.button(zx, zy, true, W, H, &c).1, Some(Change::TimeZone("Europe/Zurich".into())));
        win.page = Page::Display;
        let (dx, dy) = at(&win, &c, Target::Scale(2));
        assert_eq!(win.button(dx, dy, true, W, H, &c).1, Some(Change::Scale(2)));
        // Pages by key.
        win.step_page(false);
        assert_eq!(win.page, Page::About);
        win.step_page(true);
        assert_eq!(win.page, Page::Display);
    }

    #[test]
    fn the_arrows_go_back_and_forward() {
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        let (_, targets) = win.layout(W, H, &c);
        assert!(!targets.iter().any(|(_, t)| *t == Target::Back), "nowhere to go back to");
        let (x, y) = at(&win, &c, Target::Nav(Page::Sound));
        win.button(x, y, true, W, H, &c);
        let (bx, by) = at(&win, &c, Target::Back);
        win.button(bx, by, true, W, H, &c);
        assert_eq!(win.page, Page::Display);
        let (fx, fy) = at(&win, &c, Target::Forward);
        win.button(fx, fy, true, W, H, &c);
        assert_eq!(win.page, Page::Sound);
    }

    #[test]
    fn every_page_fits_and_draws() {
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        for page in Page::ALL {
            win.page = page;
            let (items, targets) = win.layout(W, H, &c);
            for (r, _) in &targets {
                assert!(r.right() <= W && r.bottom() <= H, "{page:?}: {r:?} is outside the window");
            }
            assert!(!items.is_empty());
            let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
            win.draw(&mut fb, W, H, 1, &c);
            assert_eq!(fb.pixel(10, H as u32 - 10), Some(t::SURFACE_0), "the sidebar's ground");
            assert_eq!(fb.pixel(W as u32 - 5, H as u32 - 5), Some(t::SURFACE_1), "the page");
            assert_eq!(fb.pixel(t::SIDEBAR as u32 + 40, t::HEADER as u32), Some(t::LINE), "the page's hairline");
        }
        // The active page is highlighted in the sidebar.
        win.page = Page::Display;
        let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
        win.draw(&mut fb, W, H, 1, &c);
        let row = win.nav().into_iter().find(|n| n.1 == Page::Display).unwrap().0;
        assert_eq!(fb.pixel((row.right() - 6) as u32, (row.y + 4) as u32), Some(t::SURFACE_3));
        // At scale 2 too.
        let mut fb = MemoryFramebuffer::new(2 * W as u32, 2 * H as u32);
        win.draw(&mut fb, W, H, 2, &c);
        assert_eq!(fb.pixel(2 * W as u32 - 5, 2 * H as u32 - 5), Some(t::SURFACE_1));
    }

    #[test]
    fn controls_are_drawn_in_alpines_colours() {
        let text = Text::new(None);
        let st = Settings { muted: true, ..Settings::default() };
        let i = info();
        let c = cx(&st, &i, &text);
        let win = SettingsWindow { page: Page::Sound, ..SettingsWindow::default() };
        let (items, _) = win.layout(W, H, &c);
        let toggle =
            items.iter().find_map(|i| if let Item::Toggle { rect, .. } = i { Some(*rect) } else { None }).unwrap();
        let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
        win.draw(&mut fb, W, H, 1, &c);
        // On: the track in the ink, the knob in the ground colour on the right.
        assert_eq!(fb.pixel((toggle.x + 6) as u32, (toggle.y + 11) as u32), Some(t::INK));
        assert_eq!(fb.pixel((toggle.x + 27) as u32, (toggle.y + 11) as u32), Some(t::SURFACE_0));
    }

    #[test]
    fn offsets() {
        assert_eq!(utc_offset(60), "UTC+01:00");
        assert_eq!(utc_offset(-300), "UTC-05:00");
        assert_eq!(utc_offset(330), "UTC+05:30");
    }
}
