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
use crate::shortcuts::{Action, Keymap, Recorded, Recorder, Refusal, key_name};
use crate::theme::pal;
use crate::theme::settings as t;
use crate::theme::wallpaper::PRESETS;
use crate::tz::{TimeZone, ZONES};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use poly_input::layout::Layout;
use poly_wayland::compose::paint::{self, Radii, Shadow};
use poly_wayland::compose::{Framebuffer, Rect};

mod builder;
mod draw;
mod input;

use builder::{Builder, Control, DENSE, DESC};

/// The pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Appearance,
    Search,
    Tiling,
    Shortcuts,
    Display,
    Keyboard,
    Wallpaper,
    Sound,
    DateTime,
    About,
}

impl Page {
    pub const ALL: [Page; 11] = [
        Page::Home,
        Page::Wallpaper,
        Page::Appearance,
        Page::Search,
        Page::Tiling,
        Page::Shortcuts,
        Page::Display,
        Page::Sound,
        Page::Keyboard,
        Page::DateTime,
        Page::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Appearance => "Appearance",
            Page::Search => "Search",
            Page::Tiling => "Window tiling",
            Page::Shortcuts => "Keyboard shortcuts",
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
            Page::Appearance => lucide::SUN_MOON,
            Page::Search => lucide::SEARCH,
            Page::Tiling => lucide::LAYOUT_DASHBOARD,
            Page::Shortcuts => lucide::COMMAND,
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
            Page::Appearance => &["theme", "dark", "light", "colour", "color"],
            Page::Search => &["launcher", "hotkey", "results"],
            Page::Tiling => &["snap", "snapping", "tile", "gap", "windows"],
            Page::Shortcuts => &["hotkeys", "keys", "shortcut"],
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
const SECTIONS: [(&str, &[Page]); 5] = [
    ("Home", &[Page::Home]),
    ("Personal", &[Page::Wallpaper, Page::Appearance]),
    ("Desktop", &[Page::Search, Page::Tiling, Page::Shortcuts]),
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

/// The Search hotkey's description (Alpine's, less what Poly's launcher
/// does not do).
const LAUNCHER_HINT: &str = "Search apps and switch to open apps";

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
    Theme(bool),
    Tiling,
    SnapKeys,
    SnapDrag,
    /// A shortcut's Set (or Cancel) button, its reset, every reset.
    Record(Action),
    ResetShortcut(Action),
    ResetShortcuts,
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
    /// Key caps (Alpine's `KeyCaps`): 22 pixels high, radius 5, a ghost
    /// fill, a hairline ring and a stronger line under, Geist Mono 11 px.
    Caps(Vec<(Rect, String)>),
    /// Alpine's small square ghost button (`h-7 px-3`, 13 px medium,
    /// radius 8, the ghost fill on hover).
    Button {
        rect: Rect,
        label: String,
        hover: bool,
    },
    /// Alpine's `IconButton` (28 × 28, radius 8; faint when it does
    /// nothing).
    IconButton {
        rect: Rect,
        icon: Icon,
        size: i32,
        hover: bool,
        enabled: bool,
    },
    /// Recording: "Press a shortcut" (22 high, radius 5, a quiet ring,
    /// 11 px).
    Pill {
        rect: Rect,
        text: String,
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
    /// Recording a shortcut, and the last refusal (the row says why).
    pub recorder: Recorder,
    refusal: Option<(Action, String)>,
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
            recorder: Recorder::default(),
            refusal: None,
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
            recording: self.recorder.active(),
            refusal: self.refusal.clone(),
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
                b.items.push(Item::Fill { rect: r, rgb: pal().ink, alpha: pal().ghost, radius: t::FIELD_RADIUS });
            }
            let ink = if enabled { pal().ink_primary } else { pal().faint };
            b.items.push(Item::Icon { icon, x: r.x + 7, y: r.y + 7, size: 15, rgb: ink, alpha: 255 });
            if enabled {
                b.targets.push((r, target));
            }
        }
        let searching = !self.search.is_empty();
        for (r, i) in self.tabs(cx.text) {
            let active = i == self.page.section() && !searching;
            if active {
                b.items.push(Item::Fill { rect: r, rgb: pal().s3, alpha: 255, radius: t::FIELD_RADIUS });
            } else if self.hover == Some(Target::Tab(i)) {
                b.items.push(Item::Fill { rect: r, rgb: pal().ink, alpha: pal().ghost, radius: t::FIELD_RADIUS });
            }
            let (st, ink) = if active {
                (Style::new(Face::Medium, t::DENSE_SIZE), pal().ink_primary)
            } else {
                (DENSE, pal().quiet)
            };
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
            rgb: pal().faint,
            alpha: 255,
        });
        let fb = b.baseline(f.y, f.height, DENSE);
        let (shown, ink) =
            if searching { (self.search.as_str(), pal().ink_primary) } else { ("Search settings", pal().faint) };
        let end = f.x + 11 + 14 + 8 + cx.text.measure(if searching { shown } else { "" }, DENSE) as i32;
        b.text(f.x + 11 + 14 + 8, fb, f.right() - 50, shown, DENSE, ink);
        if self.searching {
            b.items.push(Item::Fill {
                rect: Rect::new(end + 1, f.y + 8, 1, 16),
                rgb: pal().ink_primary,
                alpha: 255,
                radius: 0,
            });
        }
        let mono = Style::new(Face::Mono, 11.0);
        let kw = cx.text.measure("Ctrl K", mono) as i32;
        b.text(f.right() - 10 - kw, fb, f.right(), "Ctrl K", mono, pal().faint);
        b.targets.push((f, Target::Search));
        // The sidebar: the section's pages (13 px, the active one medium on
        // surface 3).
        for (r, page) in self.nav() {
            let active = page == self.page && !searching;
            if active {
                b.items.push(Item::Fill { rect: r, rgb: pal().s3, alpha: 255, radius: t::FIELD_RADIUS });
            } else if self.hover == Some(Target::Nav(page)) {
                b.items.push(Item::Fill { rect: r, rgb: pal().ink, alpha: pal().ghost, radius: t::FIELD_RADIUS });
            }
            let (ink, alpha) = if active { (pal().ink_primary, 230) } else { (pal().quiet, 153) };
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
            Page::Appearance => {
                b.title("Appearance");
                b.label("Windows");
                let light = st.theme == "light";
                b.rows(vec![(
                    "Color mode".into(),
                    Some("Choose whether the shell and your apps use light or dark colors.".into()),
                    Control::Segmented(vec![
                        ("Light".into(), light, Target::Theme(true)),
                        ("Dark".into(), !light, Target::Theme(false)),
                    ]),
                )]);
            }
            Page::Search => {
                b.title("Search");
                b.label("Hotkeys");
                b.bindings(&[(Action::Launcher, Some(LAUNCHER_HINT))]);
                b.label("Search results");
                b.rows(vec![(
                    "Maximum results".into(),
                    Some("How many results Search shows at once.".into()),
                    Control::Value("8".into()),
                )]);
            }
            Page::Tiling => {
                // Alpine's page: the master switch, and the rest only while
                // it is on. Poly snaps into zones (no auto-tile mode, no
                // layouts), with a fixed gap and margin.
                b.title("Window tiling");
                b.label("General");
                b.rows(vec![(
                    "Enable window tiling".into(),
                    Some("Arrange windows into layouts.".into()),
                    Control::Toggle(st.tiling, Some(Target::Tiling)),
                )]);
                if st.tiling {
                    b.label("Snapping");
                    b.rows(vec![
                        (
                            "Win+Arrow snapping".into(),
                            Some("Use Win+Left / Right / Up / Down to snap the focused window.".into()),
                            Control::Toggle(st.snap_keys, Some(Target::SnapKeys)),
                        ),
                        (
                            "Drag to zone".into(),
                            Some("Drag a window to a screen edge or corner to snap it there.".into()),
                            Control::Toggle(st.snap_drag, Some(Target::SnapDrag)),
                        ),
                    ]);
                    b.label("Spacing");
                    b.rows(vec![
                        ("Gap".into(), Some("Space between tiled windows.".into()), Control::Value("8px".into())),
                        (
                            "Outer margin".into(),
                            Some("Space between the tiled area and the edges of the screen.".into()),
                            Control::Value("0px".into()),
                        ),
                    ]);
                }
            }
            Page::Shortcuts => {
                // Alpine's page: two columns from its `lg` width (1024),
                // every row recordable (Set, Escape cancels, the reset when
                // changed); once one is changed, Poly's "Restore all
                // defaults" on the title's line (a small ghost button).
                let title_top = b.y;
                b.title("Keyboard shortcuts");
                if st.shortcuts.any_custom() {
                    let label = "Restore all defaults";
                    let bw = cx.text.measure(label, Style::new(Face::Medium, t::DENSE_SIZE)) as i32 + 24;
                    let rect = Rect::new(b.right - bw, title_top + 2, bw, 28);
                    let hover = b.hover == Some(Target::ResetShortcuts);
                    b.items.push(Item::Button { rect, label: label.into(), hover });
                    b.targets.push((rect, Target::ResetShortcuts));
                }
                let (left, right, top) = (b.left, b.right, b.y);
                let two = w >= 1024;
                if two {
                    b.right = (left + right - 32) / 2;
                }
                b.label("Search");
                b.bindings(&[(Action::Launcher, Some(LAUNCHER_HINT)), (Action::LauncherAlt, None)]);
                b.label("Apps");
                b.bindings(&[
                    (Action::Settings, None),
                    (Action::Notifications, None),
                    (Action::Files, None),
                    (Action::Terminal, None),
                ]);
                let end = b.y;
                if two {
                    (b.left, b.right, b.y) = ((left + right + 32) / 2, right, top);
                }
                b.label("Windows");
                b.bindings(&[(Action::SwitchWindows, None), (Action::SnapWindow, None), (Action::CloseWindow, None)]);
                (b.left, b.right, b.y) = (left, right, b.y.max(end));
            }
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
                    let ink = if st.wallpaper == p.0 { pal().ink_primary } else { pal().quiet };
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
        b.text(x, nb, b.right, "Poly", name, pal().ink_primary);
        let i = cx.info;
        let b1 = b.baseline(hero.y + 30, 18, DENSE);
        b.text(x, b1, b.right, &i.os, DENSE, pal().ink_primary);
        let b2 = b.baseline(hero.y + 48, 16, DESC);
        let machine = if i.cpu.is_empty() { i.machine.clone() } else { format!("{} \u{b7} {}", i.machine, i.cpu) };
        b.text(x, b2, b.right, &machine, DESC, pal().quiet);
        // Memory in use.
        let label = Style::new(Face::Medium, t::EYEBROW_SIZE);
        let used = i.memory_kib.saturating_sub(i.memory_available_kib);
        let pct = (used * 100).checked_div(i.memory_kib).unwrap_or(0) as u32;
        let gib = |k: u64| format!("{:.1} GB", k as f64 / (1024.0 * 1024.0));
        let ly = hero.y + 96;
        let lb = b.baseline(ly, 16, label);
        b.text(x, lb, b.right, "Memory", label, pal().ink_primary);
        let v = format!("{} of {}", gib(used), gib(i.memory_kib));
        let vw = cx.text.measure(&v, DESC) as i32;
        b.text(b.right - vw, lb, b.right + 1, &v, DESC, pal().quiet);
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
}

#[cfg(test)]
mod tests;
