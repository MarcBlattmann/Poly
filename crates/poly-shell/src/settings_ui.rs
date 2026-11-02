//! The settings window (ADR 0117), drawn by the shell into a window the
//! compositor frames like any other, in the layout of Alpine Shell's
//! current settings app (ADR 0127, measured on the live app): the
//! sections as tabs in the header (Home, Personal, Devices, System) with
//! the back and forward arrows before them and the settings search on the
//! right; the active section's pages in the sidebar; the page on a lighter
//! surface with a rounded top-left corner and a hairline edge, its title in
//! Geist, labelled sections of cards of rows with their controls on the
//! right (outline segmented choices, a slider, a pill toggle, choices with
//! a check mark, wallpaper tiles). Home shows the machine, its memory and
//! cards to the pages. Dark theme, Alpine's dark tokens.
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────────────┐
//!  │ ← →  Home [Personal] Devices System          ⌕ Search settings ⌃K │ ← header, 40
//!  │ ▨ Wallpaper    ╭──────────────────────────────────────────────────┤
//!  │                │ Wallpaper                                        │ ← title, 22 px medium
//!  │                │ WALLPAPER                                        │ ← label, 11 px
//!  │                │ ╭──────────────────────────────────────────────╮ │
//!  │                │ │ tiles                                        │ │
//!  └────────────────┴──────────────────────────────────────────────────┘
//!    sidebar 224          page: gutter 20, cards radius 12
//! ```
//!
//! Everything is laid out once per draw as a list of [`Item`]s with the
//! [`Target`]s the pointer can hit, so drawing and hit-testing agree.
//! Coordinates are logical, relative to the window's content; drawing
//! multiplies them by the output's scale.

use crate::draw::{Face, Style, Text, fill, rounded_border, rounded_outline, rounded_rect};
use crate::icons::{Icon, lucide};
use crate::launcher::Key;
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
    Home,
    Display,
    Keyboard,
    Wallpaper,
    Sound,
    DateTime,
    About,
}

impl Page {
    pub const ALL: [Page; 7] =
        [Page::Home, Page::Wallpaper, Page::Display, Page::Sound, Page::Keyboard, Page::DateTime, Page::About];

    pub fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
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
            Page::Home => lucide::HOUSE,
            Page::Display => lucide::MONITOR,
            Page::Keyboard => lucide::KEYBOARD,
            Page::Wallpaper => lucide::WALLPAPER,
            Page::Sound => lucide::VOLUME_2,
            Page::DateTime => lucide::CALENDAR_CLOCK,
            Page::About => lucide::INFO,
        }
    }

    /// Words the settings search finds the page by (besides its label).
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Page::Home => &["overview", "system", "memory"],
            Page::Display => &["resolution", "scale", "screen", "monitor", "hidpi"],
            Page::Keyboard => &["layout", "input", "language", "keys"],
            Page::Wallpaper => &["background", "desktop", "picture"],
            Page::Sound => &["volume", "mute", "audio", "speaker"],
            Page::DateTime => &["clock", "time zone", "timezone", "date"],
            Page::About => &["kernel", "processor", "cpu", "memory", "network", "version"],
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }

    /// The section (tab) the page is in.
    fn section(self) -> usize {
        SECTIONS.iter().position(|s| s.1.contains(&self)).unwrap_or(0)
    }
}

/// Alpine's sections with Poly's pages (sections without pages are left
/// out, as Alpine does: Poly has nothing in Desktop yet).
const SECTIONS: [(&str, &[Page]); 4] = [
    ("Home", &[Page::Home]),
    ("Personal", &[Page::Wallpaper]),
    ("Devices", &[Page::Display, Page::Sound, Page::Keyboard]),
    ("System", &[Page::DateTime, Page::About]),
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
    /// Memory available (`MemAvailable`), for Home's bar.
    pub memory_available_kib: u64,
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
    Tab(usize),
    Search,
    Result(usize),
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
    /// A card: the ghost fill, rounded.
    Card(Rect),
    /// A hairline.
    Line(Rect),
    /// A fill (hover, selection, tabs).
    Fill {
        rect: Rect,
        rgb: u32,
        alpha: u8,
        radius: i32,
    },
    /// The settings search field.
    Field {
        rect: Rect,
        focused: bool,
    },
    /// A bar of `value` out of 100 (Home's memory).
    Bar {
        rect: Rect,
        value: u32,
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
    /// The settings search: its text, and whether it has the keys.
    search: String,
    searching: bool,
}

impl Default for SettingsWindow {
    fn default() -> SettingsWindow {
        SettingsWindow {
            page: Page::Home,
            hover: None,
            dragging: false,
            history: vec![Page::Home],
            at: 0,
            search: String::new(),
            searching: false,
        }
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
    /// The wallpapers' pictures (for the tiles).
    pub pictures: &'a crate::picture::Pictures,
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

/// A row of a card: label, description, and what goes on its right.
enum Control {
    Value(String),
    Segmented(Vec<(String, bool, Target)>),
    Slider(u32, Option<Target>),
    Toggle(bool, Option<Target>),
}

/// Builds a page's items. `y` is the top of the next thing.
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

    /// The baseline that centres a line of `st` in `top..top + h`.
    fn baseline(&self, top: i32, h: i32, st: Style) -> i32 {
        self.cx.text.centre(top, h, st)
    }

    /// The page's title: Geist 22 px medium, `tracking-tight`, a 32-pixel
    /// line; the first section 32 pixels below it.
    fn title(&mut self, title: &str) {
        let st = Style::new(Face::Medium, t::TITLE_SIZE).tracking(-0.025);
        let b = self.baseline(self.y, 32, st);
        self.text(self.left, b, self.right, title, st, t::INK);
        self.y += 32 + 32;
    }

    /// A section's label (11 px medium, quiet, `pl-1`), 8 pixels above
    /// its card.
    fn label(&mut self, label: &str) {
        let st = Style::new(Face::Medium, t::EYEBROW_SIZE);
        let b = self.baseline(self.y, 16, st);
        self.text(self.left + 4, b, self.right, label, st, t::INK_QUIET);
        self.y += 16 + 8;
    }

    /// A card of rows (Alpine's group card: `px-3.5 py-2.5`, at least 48
    /// pixels, hairlines between): (label, description, control).
    fn rows(&mut self, rows: Vec<(String, Option<String>, Control)>) {
        let heights: Vec<i32> = rows.iter().map(|r| if r.1.is_some() { 58 } else { 48 }).collect();
        let total: i32 = heights.iter().sum();
        let card = Rect::new(self.left, self.y, self.right - self.left, total);
        self.items.push(Item::Card(card));
        let mut y = self.y;
        for (i, ((label, desc, control), h)) in rows.into_iter().zip(heights).enumerate() {
            let r = Rect::new(card.x, y, card.width, h);
            if i > 0 {
                self.items.push(Item::Line(Rect::new(r.x, r.y, r.width, 1)));
            }
            let x = r.x + t::ROW_PADDING;
            let max = r.right() - 200;
            match &desc {
                None => {
                    let b = self.baseline(r.y, r.height, LABEL);
                    self.text(x, b, max, &label, LABEL, t::INK);
                }
                Some(d) => {
                    let b = self.baseline(r.y + 10, 20, LABEL);
                    self.text(x, b, max, &label, LABEL, t::INK);
                    let b = self.baseline(r.y + 32, 16, DESC);
                    self.text(x, b, max, d, DESC, t::INK_QUIET);
                }
            }
            self.control(r, control);
            y += h;
        }
        self.y = card.bottom() + 32;
    }

    fn control(&mut self, r: Rect, c: Control) {
        let right = r.right() - t::ROW_PADDING;
        let mid = r.y + r.height / 2;
        match c {
            Control::Value(v) => {
                let room = (right - (r.x + 200)) as f32;
                let v = self.cx.text.fit_style(&v, DENSE, room);
                let w = self.cx.text.measure(&v, DENSE) as i32;
                let b = self.baseline(r.y, r.height, DENSE);
                self.text(right - w, b, right + 1, &v, DENSE, t::INK_QUIET);
            }
            Control::Segmented(options) => {
                let widths: Vec<i32> = options.iter().map(|o| self.cx.text.measure(&o.0, DENSE) as i32 + 24).collect();
                let total: i32 = widths.iter().sum::<i32>() + 10;
                let rect = Rect::new(right - total, mid - 19, total, 38);
                let mut x = rect.x + 5;
                let mut opts = Vec::new();
                for (o, w) in options.iter().zip(widths) {
                    let or = Rect::new(x, rect.y + 5, w, 28);
                    opts.push((or, o.0.clone(), o.1));
                    self.targets.push((or, o.2));
                    x += w;
                }
                self.items.push(Item::Segmented { rect, options: opts });
            }
            Control::Slider(value, target) => {
                let track = Rect::new(right - 40 - 12 - 176, mid - 4, 176, 8);
                self.items.push(Item::Slider { rect: track, value, enabled: target.is_some() });
                let v = format!("{value}%");
                let w = self.cx.text.measure(&v, DESC) as i32;
                let b = self.baseline(r.y, r.height, DESC);
                self.text(right - w, b, right + 1, &v, DESC, t::INK_QUIET);
                if let Some(tg) = target {
                    self.targets.push((Rect::new(track.x - 6, r.y, track.width + 12, r.height), tg));
                }
            }
            Control::Toggle(on, target) => {
                let rect = Rect::new(right - 38, mid - 11, 38, 22);
                self.items.push(Item::Toggle { rect, on });
                if let Some(tg) = target {
                    self.targets.push((rect, tg));
                }
            }
        }
    }

    /// A card of choices (48-pixel rows, 36 for long lists): label,
    /// detail on the right, a check if chosen.
    fn choices(&mut self, rows: &[(String, String, bool, Target)]) {
        let top = self.y;
        let rh = if rows.len() > 4 { 36 } else { 48 };
        let card = Rect::new(self.left, top, self.right - self.left, rows.len() as i32 * rh);
        self.items.push(Item::Card(card));
        for (i, (label, detail, chosen, target)) in rows.iter().enumerate() {
            let r = Rect::new(card.x, top + i as i32 * rh, card.width, rh);
            if i > 0 {
                self.items.push(Item::Line(Rect::new(r.x, r.y, r.width, 1)));
            }
            if self.hover == Some(*target) {
                let radius = if i == 0 || i == rows.len() - 1 { t::CARD_RADIUS } else { 0 };
                self.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius });
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
        self.y = card.bottom() + 32;
    }

    /// A card of rows that lead to pages: icon, label, detail, chevron.
    fn links(&mut self, rows: &[(Page, String)]) {
        let top = self.y;
        let card = Rect::new(self.left, top, self.right - self.left, rows.len() as i32 * 48);
        self.items.push(Item::Card(card));
        for (i, (page, detail)) in rows.iter().enumerate() {
            let r = Rect::new(card.x, top + i as i32 * 48, card.width, 48);
            if i > 0 {
                self.items.push(Item::Line(Rect::new(r.x, r.y, r.width, 1)));
            }
            if self.hover == Some(Target::Nav(*page)) {
                let radius = if i == 0 || i == rows.len() - 1 { t::CARD_RADIUS } else { 0 };
                self.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius });
            }
            let ix = r.x + t::ROW_PADDING;
            self.items.push(Item::Icon {
                icon: page.icon(),
                x: ix,
                y: r.y + 16,
                size: 15,
                rgb: t::INK_QUIET,
                alpha: 255,
            });
            let b = self.baseline(r.y, r.height, DENSE);
            self.text(ix + 15 + 12, b, r.right() - 160, page.label(), DENSE, t::INK);
            let cx = r.right() - t::ROW_PADDING - 16;
            self.items.push(Item::Icon {
                icon: lucide::CHEVRON_RIGHT,
                x: cx,
                y: r.y + 16,
                size: 16,
                rgb: t::INK_FAINT,
                alpha: 255,
            });
            if !detail.is_empty() {
                let w = self.cx.text.measure(detail, DENSE) as i32;
                self.text(cx - 12 - w, b, cx - 4, detail, DENSE, t::INK_QUIET);
            }
            self.targets.push((r, Target::Nav(*page)));
        }
        self.y = card.bottom() + 32;
    }

    /// A note (12 px, faint).
    fn note(&mut self, text: &str) {
        let b = self.baseline(self.y - 24, 16, DESC);
        self.text(self.left + 4, b, self.right, text, DESC, t::INK_FAINT);
        self.y += 8;
    }
}

/// What the settings search finds for `query`: pages whose label or
/// keywords contain it (case-insensitive), with the word that matched.
pub fn search(query: &str) -> Vec<(Page, String)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in Page::ALL {
        let label = p.label().to_lowercase();
        if label.contains(&q) {
            out.push((p, String::new()));
        } else if let Some(k) = p.keywords().iter().find(|k| k.contains(q.as_str())) {
            out.push((p, (*k).to_string()));
        }
    }
    out
}

impl SettingsWindow {
    /// The sidebar's rows for the active section: (row, page).
    fn nav(&self) -> Vec<(Rect, Page)> {
        let pages = SECTIONS[self.page.section()].1;
        pages
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let y = t::HEADER + 4 + i as i32 * (t::NAV_HEIGHT + t::NAV_GAP);
                (Rect::new(t::NAV_PADDING, y, t::SIDEBAR - 2 * t::NAV_PADDING, t::NAV_HEIGHT), p)
            })
            .collect()
    }

    /// The header's tabs: (tab, section).
    fn tabs(&self, text: &Text) -> Vec<(Rect, usize)> {
        let mut x = 12 + 28 + 4 + 28 + 8;
        SECTIONS
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let st = if i == self.page.section() { Style::new(Face::Medium, t::DENSE_SIZE) } else { DENSE };
                let w = text.measure(label, st) as i32 + 24;
                let r = Rect::new(x, (t::HEADER - 28) / 2, w, 28);
                x += w + 2;
                (r, i)
            })
            .collect()
    }

    /// The search field.
    fn field(w: i32) -> Rect {
        Rect::new(w - 8 - 256, (t::HEADER - 32) / 2, 256, 32)
    }

    /// The page's items and the pointer's targets, for a window `w` × `h`.
    fn layout(&self, w: i32, h: i32, cx: &Context<'_>) -> (Vec<Item>, Vec<(Rect, Target)>) {
        let mut b = Builder {
            cx,
            items: Vec::new(),
            targets: Vec::new(),
            hover: self.hover,
            left: t::SIDEBAR + 1 + t::GUTTER,
            right: w - t::GUTTER,
            y: t::HEADER + 1 + t::GUTTER,
        };
        // The header: the arrows, the sections' tabs, the search.
        let arrows = [
            (Target::Back, lucide::ARROW_LEFT, self.at > 0),
            (Target::Forward, lucide::ARROW_RIGHT, self.at + 1 < self.history.len()),
        ];
        for (i, (target, icon, enabled)) in arrows.into_iter().enumerate() {
            let r = Rect::new(12 + i as i32 * 32, (t::HEADER - 28) / 2, 28, 28);
            if enabled && self.hover == Some(target) {
                b.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius: t::FIELD_RADIUS });
            }
            let ink = if enabled { t::INK } else { t::INK_FAINT };
            b.items.push(Item::Icon { icon, x: r.x + 7, y: r.y + 7, size: 15, rgb: ink, alpha: 255 });
            if enabled {
                b.targets.push((r, target));
            }
        }
        let searching = !self.search.is_empty();
        for (r, i) in self.tabs(cx.text) {
            let active = i == self.page.section() && !searching;
            if active {
                b.items.push(Item::Fill { rect: r, rgb: t::SURFACE_3, alpha: 255, radius: t::FIELD_RADIUS });
            } else if self.hover == Some(Target::Tab(i)) {
                b.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius: t::FIELD_RADIUS });
            }
            let (st, ink) =
                if active { (Style::new(Face::Medium, t::DENSE_SIZE), t::INK) } else { (DENSE, t::INK_QUIET) };
            let tb = b.baseline(r.y, r.height, st);
            b.text(r.x + 12, tb, r.right(), SECTIONS[i].0, st, ink);
            b.targets.push((r, Target::Tab(i)));
        }
        let f = Self::field(w);
        b.items.push(Item::Field { rect: f, focused: self.searching });
        b.items.push(Item::Icon {
            icon: lucide::SEARCH,
            x: f.x + 11,
            y: f.y + 9,
            size: 14,
            rgb: t::INK_FAINT,
            alpha: 255,
        });
        let fb = b.baseline(f.y, f.height, DENSE);
        let (shown, ink) = if searching { (self.search.as_str(), t::INK) } else { ("Search settings", t::INK_FAINT) };
        let end = f.x + 11 + 14 + 8 + cx.text.measure(if searching { shown } else { "" }, DENSE) as i32;
        b.text(f.x + 11 + 14 + 8, fb, f.right() - 50, shown, DENSE, ink);
        if self.searching {
            b.items.push(Item::Fill { rect: Rect::new(end + 1, f.y + 8, 1, 16), rgb: t::INK, alpha: 255, radius: 0 });
        }
        let mono = Style::new(Face::Mono, 11.0);
        let kw = cx.text.measure("Ctrl K", mono) as i32;
        b.text(f.right() - 10 - kw, fb, f.right(), "Ctrl K", mono, t::INK_FAINT);
        b.targets.push((f, Target::Search));
        // The sidebar: the section's pages (13 px, the active one medium on
        // surface 3).
        for (r, page) in self.nav() {
            let active = page == self.page && !searching;
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
        if searching {
            let found = search(&self.search);
            b.title("Search");
            b.label(&format!("{} result{}", found.len(), if found.len() == 1 { "" } else { "s" }));
            let rows: Vec<(String, String, bool, Target)> = found
                .iter()
                .enumerate()
                .map(|(i, (p, why))| (p.label().to_string(), why.clone(), false, Target::Result(i)))
                .collect();
            if rows.is_empty() {
                b.note("No settings match.");
            } else {
                b.choices(&rows);
            }
            let _ = h;
            return (b.items, b.targets);
        }
        let st = cx.settings;
        match self.page {
            Page::Home => self.home(&mut b),
            Page::Display => {
                b.title("Displays");
                let mut modes = cx.info.modes.clone();
                if modes.is_empty() {
                    modes.push(cx.info.display);
                }
                let current = st.resolution.unwrap_or(cx.info.display);
                b.label("Display");
                let res: Vec<(String, bool, Target)> = modes
                    .iter()
                    .enumerate()
                    .take(3)
                    .map(|(i, &(mw, mh))| (format!("{mw}\u{d7}{mh}"), (mw, mh) == current, Target::Mode(i)))
                    .collect();
                let scales: Vec<(String, bool, Target)> =
                    SCALES.iter().map(|&s| (format!("{}%", s * 100), s == st.scale, Target::Scale(s))).collect();
                b.rows(vec![
                    ("Resolution".into(), Some("The display's mode".into()), Control::Segmented(res)),
                    ("Scale".into(), Some("The size of everything on the screen".into()), Control::Segmented(scales)),
                ]);
                if modes.len() <= 1 {
                    b.note("Other modes come with the KMS display path.");
                }
            }
            Page::Keyboard => {
                b.title("Keyboard");
                b.label("Keyboard layout");
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
                b.title("Wallpaper");
                b.label("Wallpaper");
                let cols = 3;
                let gap = 12;
                let tw = (b.right - b.left - gap * (cols - 1)) / cols;
                let th = tw * 9 / 16;
                let top = b.y;
                for (i, p) in PRESETS.iter().enumerate() {
                    let (c, r) = (i as i32 % cols, i as i32 / cols);
                    let rect = Rect::new(b.left + c * (tw + gap), top + r * (th + 30), tw, th);
                    b.items.push(Item::Tile { rect, id: p.0, selected: st.wallpaper == p.0 });
                    let lb = b.baseline(rect.bottom() + 6, 16, DESC);
                    let ink = if st.wallpaper == p.0 { t::INK } else { t::INK_QUIET };
                    b.text(rect.x, lb, rect.right(), p.1, DESC, ink);
                    b.targets.push((Rect::new(rect.x, rect.y, rect.width, rect.height + 24), Target::Wallpaper(i)));
                }
            }
            Page::Sound => {
                b.title("Sound");
                b.label("Output");
                let card = cx.info.sound.clone().unwrap_or_else(|| "No sound card found".into());
                let enabled = cx.info.sound.is_some();
                b.rows(vec![
                    ("Volume".into(), Some(card), Control::Slider(st.volume, enabled.then_some(Target::Volume))),
                    (
                        "Mute".into(),
                        Some("Silence every program".into()),
                        Control::Toggle(st.muted, enabled.then_some(Target::Mute)),
                    ),
                ]);
            }
            Page::DateTime => {
                b.title("Date & time");
                b.label("Clock");
                b.rows(vec![("Local time".into(), None, Control::Value(cx.now.into()))]);
                b.label("Time zone");
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
                b.title("About");
                b.label("System");
                let rows: Vec<(String, Option<String>, Control)> = vec![
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
                ]
                .into_iter()
                .map(|(l, v)| (l.to_string(), None, Control::Value(v)))
                .collect();
                b.rows(rows);
            }
        }
        let _ = h;
        (b.items, b.targets)
    }

    /// Home: the wallpaper, the machine and its memory, cards to pages.
    fn home(&self, b: &mut Builder<'_, '_>) {
        let cx = b.cx;
        let st = cx.settings;
        let hero = Rect::new(b.left, b.y, 236, 132);
        b.items.push(Item::Tile {
            rect: hero,
            id: PRESETS.iter().find(|p| p.0 == st.wallpaper).map_or(PRESETS[0].0, |p| p.0),
            selected: false,
        });
        let x = hero.right() + 20;
        let name = Style::new(Face::Medium, t::TITLE_SIZE).tracking(-0.025);
        let nb = b.baseline(hero.y, 30, name);
        b.text(x, nb, b.right, "Poly", name, t::INK);
        let i = cx.info;
        let b1 = b.baseline(hero.y + 30, 18, DENSE);
        b.text(x, b1, b.right, &i.os, DENSE, t::INK);
        let b2 = b.baseline(hero.y + 48, 16, DESC);
        let machine = if i.cpu.is_empty() { i.machine.clone() } else { format!("{} \u{b7} {}", i.machine, i.cpu) };
        b.text(x, b2, b.right, &machine, DESC, t::INK_QUIET);
        // Memory in use.
        let label = Style::new(Face::Medium, t::EYEBROW_SIZE);
        let used = i.memory_kib.saturating_sub(i.memory_available_kib);
        let pct = (used * 100).checked_div(i.memory_kib).unwrap_or(0) as u32;
        let gib = |k: u64| format!("{:.1} GB", k as f64 / (1024.0 * 1024.0));
        let ly = hero.y + 96;
        let lb = b.baseline(ly, 16, label);
        b.text(x, lb, b.right, "Memory", label, t::INK);
        let v = format!("{} of {}", gib(used), gib(i.memory_kib));
        let vw = cx.text.measure(&v, DESC) as i32;
        b.text(b.right - vw, lb, b.right + 1, &v, DESC, t::INK_QUIET);
        b.items.push(Item::Bar { rect: Rect::new(x, ly + 20, b.right - x, 6), value: pct });
        b.y = hero.bottom() + 26;
        let wall = PRESETS.iter().find(|p| p.0 == st.wallpaper).map_or("", |p| p.1);
        let kb = KEYBOARDS.iter().find(|k| k.0 == st.keyboard).map_or("", |k| k.1);
        b.links(&[
            (Page::Wallpaper, wall.to_string()),
            (Page::Display, format!("{}\u{d7}{}", i.display.0, i.display.1)),
            (Page::Sound, if i.sound.is_some() { format!("{}%", st.volume) } else { String::new() }),
            (Page::Keyboard, kb.to_string()),
            (Page::DateTime, cx.now.to_string()),
        ]);
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

    /// Go to page `p` (remembered for the arrows); the search ends.
    fn go(&mut self, p: Page) -> bool {
        let was_searching = !self.search.is_empty();
        self.search.clear();
        self.searching = false;
        if p == self.page {
            return was_searching;
        }
        self.history.truncate(self.at + 1);
        self.history.push(p);
        self.at = self.history.len() - 1;
        self.page = p;
        true
    }

    /// Open the window at page `p` (from outside: the ridge, a hotkey).
    pub fn open_at(&mut self, p: Page) {
        self.go(p);
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
        let target = self.target_at(x, y, w, h, cx);
        if target != Some(Target::Search) && self.searching && self.search.is_empty() {
            self.searching = false;
        }
        let Some(target) = target else { return (false, None) };
        let st = cx.settings;
        let change = match target {
            Target::Nav(p) => return (self.go(p), None),
            Target::Tab(i) => return (self.go(SECTIONS[i].1[0]), None),
            Target::Search => {
                self.searching = true;
                return (true, None);
            }
            Target::Result(i) => {
                let found = search(&self.search);
                return match found.get(i) {
                    Some(&(p, _)) => {
                        self.go(p);
                        (true, None)
                    }
                    None => (false, None),
                };
            }
            Target::Back | Target::Forward => {
                let at = if target == Target::Back { self.at.checked_sub(1) } else { Some(self.at + 1) };
                if let Some(&p) = at.and_then(|a| self.history.get(a)) {
                    self.at = at.unwrap_or(0);
                    self.page = p;
                    self.search.clear();
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

    /// A key while the window has the keyboard: typing into the search
    /// (once its field was clicked, or any letter: typing searches, as in
    /// Alpine), Enter opens the first result, Escape clears it; otherwise
    /// Up and Down step through the pages. Returns whether to draw again.
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Char(c) if !c.is_control() => {
                self.searching = true;
                self.search.push(c);
                true
            }
            Key::Backspace if self.searching => self.search.pop().is_some(),
            Key::BackspaceWord | Key::Clear if self.searching => {
                self.search.clear();
                true
            }
            Key::Escape if self.searching || !self.search.is_empty() => {
                self.search.clear();
                self.searching = false;
                true
            }
            Key::Enter if !self.search.is_empty() => match search(&self.search).first() {
                Some(&(p, _)) => self.go(p),
                None => false,
            },
            Key::Up => {
                self.step_page(false);
                true
            }
            Key::Down => {
                self.step_page(true);
                true
            }
            _ => false,
        }
    }

    /// The search's text.
    pub fn search_text(&self) -> &str {
        &self.search
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
                Item::Card(c) => rounded_rect(fb, all, c.scale(s), t::CARD_RADIUS * s, WHITE, t::GHOST_ALPHA),
                Item::Line(l) => fill(fb, all, Rect::new(l.x * s, l.y * s, l.width * s, s), WHITE, t::HAIRLINE_ALPHA),
                Item::Fill { rect, rgb, alpha, radius } => {
                    rounded_rect(fb, all, rect.scale(s), radius * s, *rgb, *alpha)
                }
                Item::Field { rect, focused } => {
                    let f = rect.scale(s);
                    rounded_rect(fb, all, f, t::FIELD_RADIUS * s, t::SURFACE_2, 255);
                    let line = if *focused { t::INK_FAINT } else { t::LINE };
                    rounded_border(fb, all, f, t::FIELD_RADIUS * s, s, line, 255);
                }
                Item::Bar { rect, value } => {
                    let r = rect.scale(s);
                    rounded_rect(fb, all, r, r.height / 2, WHITE, t::TRACK_ALPHA);
                    let filled = Rect::new(r.x, r.y, r.width * (*value).min(100) as i32 / 100, r.height);
                    if filled.width > 0 {
                        rounded_rect(fb, all.intersect(&filled), r, r.height / 2, t::INK, 255);
                    }
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
                    crate::shell::draw_wallpaper(&mut tile, full, id, r.width, r.height, cx.pictures);
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
            memory_available_kib: 256 * 1024,
            display: (1280, 800),
            modes: vec![(1280, 800)],
            sound: Some("virtio-sound".into()),
            network: "eth0 up".into(),
            session_bus: "unix:path=/run/user/0/bus".into(),
        }
    }

    static NO_PICTURES: crate::picture::Pictures = crate::picture::Pictures::new();

    fn cx<'a>(st: &'a Settings, i: &'a SystemInfo, text: &'a Text) -> Context<'a> {
        Context { settings: st, info: i, text, now: "12:00 Nov 1", utc: 1_793_491_200, pictures: &NO_PICTURES }
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
    fn tabs_pages_and_changes() {
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        assert_eq!(win.page, Page::Home);
        // The Devices tab opens its first page; its pages are in the sidebar.
        let (x, y) = at(&win, &c, Target::Tab(2));
        assert_eq!(win.button(x, y, true, W, H, &c), (true, None));
        assert_eq!(win.page, Page::Display);
        assert_eq!(win.nav().iter().map(|n| n.1).collect::<Vec<_>>(), [Page::Display, Page::Sound, Page::Keyboard]);
        let (x, y) = at(&win, &c, Target::Nav(Page::Sound));
        win.button(x, y, true, W, H, &c);
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
        let (wx, wy) = at(&win, &c, Target::Wallpaper(4));
        assert_eq!(win.button(wx, wy, true, W, H, &c).1, Some(Change::Wallpaper("forest".into())));
        win.page = Page::DateTime;
        let (zx, zy) = at(&win, &c, Target::Zone(1));
        assert_eq!(win.button(zx, zy, true, W, H, &c).1, Some(Change::TimeZone("Europe/Zurich".into())));
        win.page = Page::Display;
        let (dx, dy) = at(&win, &c, Target::Scale(2));
        assert_eq!(win.button(dx, dy, true, W, H, &c).1, Some(Change::Scale(2)));
        // Home's cards lead to pages.
        win.page = Page::Home;
        let (hx, hy) = at(&win, &c, Target::Nav(Page::Sound));
        win.button(hx, hy, true, W, H, &c);
        assert_eq!(win.page, Page::Sound);
    }

    #[test]
    fn the_search_finds_pages_by_words() {
        assert_eq!(search("vol").first().map(|r| r.0), Some(Page::Sound));
        assert_eq!(search("zone").first().map(|r| r.0), Some(Page::DateTime));
        assert!(search("  ").is_empty());
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        let (fx, fy) = at(&win, &c, Target::Search);
        win.button(fx, fy, true, W, H, &c);
        for ch in "layout".chars() {
            assert!(win.key(Key::Char(ch)));
        }
        let (rx, ry) = at(&win, &c, Target::Result(0));
        win.button(rx, ry, true, W, H, &c);
        assert_eq!(win.page, Page::Keyboard);
        assert_eq!(win.search_text(), "", "going to a page ends the search");
        // Enter opens the first result; Escape clears.
        for ch in "scale".chars() {
            win.key(Key::Char(ch));
        }
        assert!(win.key(Key::Enter));
        assert_eq!(win.page, Page::Display);
        win.key(Key::Char('x'));
        assert!(win.key(Key::Escape));
        assert_eq!(win.search_text(), "");
    }

    #[test]
    fn the_arrows_go_back_and_forward() {
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        let (_, targets) = win.layout(W, H, &c);
        assert!(!targets.iter().any(|(_, t)| *t == Target::Back), "nowhere to go back to");
        let (x, y) = at(&win, &c, Target::Tab(3));
        win.button(x, y, true, W, H, &c);
        let (bx, by) = at(&win, &c, Target::Back);
        win.button(bx, by, true, W, H, &c);
        assert_eq!(win.page, Page::Home);
        let (fx, fy) = at(&win, &c, Target::Forward);
        win.button(fx, fy, true, W, H, &c);
        assert_eq!(win.page, Page::DateTime);
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
        // The active page is highlighted in the sidebar, its tab in the header.
        win.page = Page::Display;
        let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
        win.draw(&mut fb, W, H, 1, &c);
        let row = win.nav().into_iter().find(|n| n.1 == Page::Display).unwrap().0;
        assert_eq!(fb.pixel((row.right() - 6) as u32, (row.y + 4) as u32), Some(t::SURFACE_3));
        let tab = win.tabs(&text).into_iter().find(|t| t.1 == 2).unwrap().0;
        assert_eq!(fb.pixel((tab.x + 4) as u32, (tab.y + 14) as u32), Some(t::SURFACE_3));
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

    /// The places the `desktop-shell` boot test clicks (tools/xtask/src/desktop.rs).
    #[test]
    fn the_boot_tests_clicks_land() {
        let text = Text::new(None);
        let (st, i) = (Settings::default(), info());
        let c = cx(&st, &i, &text);
        let mut win = SettingsWindow::default();
        assert_eq!(win.target_at(400, 339, W, H, &c), Some(Target::Nav(Page::Sound)));
        win.button(400, 339, true, W, H, &c);
        assert_eq!(win.button(691, 178, true, W, H, &c).1, Some(Change::Volume(30)));
        win.button(691, 178, false, W, H, &c);
        assert_eq!(win.target_at(100, 138, W, H, &c), Some(Target::Nav(Page::Keyboard)));
        win.button(100, 138, true, W, H, &c);
        assert_eq!(win.target_at(500, 221, W, H, &c), Some(Target::Keyboard(1)));
    }

    #[test]
    fn offsets() {
        assert_eq!(utc_offset(60), "UTC+01:00");
        assert_eq!(utc_offset(-300), "UTC-05:00");
        assert_eq!(utc_offset(330), "UTC+05:30");
    }
}
