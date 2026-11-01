//! The settings window (ADR 0117), drawn by the shell into a window the
//! compositor frames like any other: Alpine Shell's settings app re-created
//! in its dark theme — a sidebar of pages on the ground colour, the page on
//! a lighter surface with a rounded top-left corner and a hairline edge,
//! a title, and cards of rows with their controls on the right
//! (segmented choices, a slider, a toggle, lists with a check mark,
//! wallpaper tiles).
//!
//! ```text
//!  ┌────────────────────────────────────────────────────────────────┐
//!  │ Settings › Sound                                               │ ← header, 40
//!  │ ┌────────────┐╭───────────────────────────────────────────────┐│
//!  │ │▣ Display   ││ Sound                                         ││ ← title, 22 px
//!  │ │⌨ Keyboard  ││ ╭───────────────────────────────────────────╮ ││
//!  │ │▨ Wallpaper ││ │ Output volume        ━━━━━━━━━──────  60 % │ ││ ← rows, 48 (60 with a line under)
//!  │ │◖ Sound     ││ │ Mute                               ( ●)   │ ││
//!  │ │◷ Date&Time ││ ╰───────────────────────────────────────────╯ ││
//!  │ │ⓘ About     ││                                               ││
//!  │ └────────────┘╰───────────────────────────────────────────────┘│
//!  └────────────────────────────────────────────────────────────────┘
//!    sidebar 224                page: gutter 20, cards radius 12
//! ```
//!
//! Everything is laid out once per draw as a list of [`Item`]s with the
//! [`Target`]s the pointer can hit, so drawing and hit-testing agree.
//! Coordinates are logical, relative to the window's content; drawing
//! multiplies them by the output's scale.

use crate::draw::{self, Text, fill, rounded_outline, rounded_rect};
use crate::settings::{Change, KEYBOARDS, SCALES, Settings};
use crate::theme::settings as t;
use crate::theme::wallpaper::PRESETS;
use crate::tz::{TimeZone, ZONES};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
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
            Page::Display => "Display",
            Page::Keyboard => "Keyboard",
            Page::Wallpaper => "Wallpaper",
            Page::Sound => "Sound",
            Page::DateTime => "Date & Time",
            Page::About => "About",
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }
}

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
        size: f32,
        rgb: u32,
        alpha: u8,
    },
    /// A card: the ghost fill, rounded.
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
        colours: (u32, u32, u32),
        selected: bool,
    },
    Check(Rect),
    Glyph {
        page: Page,
        x: i32,
        y: i32,
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
}

impl Default for SettingsWindow {
    fn default() -> SettingsWindow {
        SettingsWindow { page: Page::Display, hover: None, dragging: false }
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

/// Builds a page's items.
struct Builder<'a, 'b> {
    cx: &'a Context<'b>,
    items: Vec<Item>,
    targets: Vec<(Rect, Target)>,
    hover: Option<Target>,
    left: i32,
    right: i32,
    y: i32,
}

impl Builder<'_, '_> {
    fn text(&mut self, x: i32, y: i32, max_x: i32, text: &str, size: f32, rgb: u32) {
        self.items.push(Item::Text { x, y, max_x, text: text.into(), size, rgb, alpha: 255 });
    }

    /// The baseline that centres a line of `size` in `top..top + h`.
    fn baseline(&self, top: i32, h: i32, size: f32) -> i32 {
        top + (h + self.cx.text.cap_height(size) as i32) / 2
    }

    /// A section label (11 px, quiet) and room under it.
    fn section(&mut self, label: &str) {
        let y = self.y + 12;
        self.text(self.left + 4, y, self.right, label, t::EYEBROW_SIZE, t::INK_QUIET);
        self.y += 20;
    }

    /// A card of rows of the given heights: their rectangles.
    fn card(&mut self, heights: &[i32]) -> Vec<Rect> {
        let total: i32 = heights.iter().sum();
        let card = Rect::new(self.left, self.y, self.right - self.left, total);
        self.items.push(Item::Card(card));
        let mut rows = Vec::new();
        let mut y = self.y;
        for (i, h) in heights.iter().enumerate() {
            if i > 0 {
                self.items.push(Item::Line(Rect::new(card.x, y, card.width, 1)));
            }
            rows.push(Rect::new(card.x, y, card.width, *h));
            y += h;
        }
        self.y = y + 16;
        rows
    }

    /// A row's label (and the line under it); returns where controls may
    /// start from the right.
    fn label(&mut self, row: Rect, label: &str, under: Option<&str>) {
        let x = row.x + t::ROW_PADDING;
        let max = row.right() - t::ROW_PADDING;
        match under {
            None => {
                let b = self.baseline(row.y, row.height, t::LABEL_SIZE);
                self.text(x, b, max, label, t::LABEL_SIZE, t::INK);
            }
            Some(u) => {
                self.text(x, row.y + 26, max, label, t::LABEL_SIZE, t::INK);
                self.text(x, row.y + 44, max, u, t::SMALL_SIZE, t::INK_QUIET);
            }
        }
    }

    /// A value on the right of a row.
    fn value(&mut self, row: Rect, value: &str, from_x: i32) {
        let size = t::DENSE_SIZE;
        let room = (row.right() - t::ROW_PADDING - from_x) as f32;
        let v = self.cx.text.fit(value, size, room);
        let w = self.cx.text.width(&v, size) as i32;
        let b = self.baseline(row.y, row.height, size);
        self.text(row.right() - t::ROW_PADDING - w, b, row.right(), &v, size, t::INK_QUIET);
    }

    /// Segmented choices at the right of a row: (label, active, target).
    fn segmented(&mut self, row: Rect, options: &[(String, bool, Target)]) {
        let size = t::SMALL_SIZE;
        let widths: Vec<i32> = options.iter().map(|o| self.cx.text.width(&o.0, size) as i32 + 20).collect();
        let total: i32 = widths.iter().sum::<i32>() + 2 * (options.len() as i32 - 1) + 4;
        let rect = Rect::new(row.right() - t::ROW_PADDING - total, row.y + (row.height - 28) / 2, total, 28);
        let mut x = rect.x + 2;
        let mut opts = Vec::new();
        for (o, w) in options.iter().zip(widths) {
            let r = Rect::new(x, rect.y + 2, w, 24);
            opts.push((r, o.0.clone(), o.1));
            self.targets.push((r, o.2));
            x += w + 2;
        }
        self.items.push(Item::Segmented { rect, options: opts });
    }

    /// A list row: a hover fill, the label, a value and a check if chosen.
    fn choice(&mut self, row: Rect, label: &str, value: &str, chosen: bool, target: Target) {
        if self.hover == Some(target) {
            self.items.push(Item::Fill { rect: row, rgb: WHITE, alpha: t::GHOST_ALPHA, radius: 0 });
        }
        let b = self.baseline(row.y, row.height, t::DENSE_SIZE);
        self.text(row.x + t::ROW_PADDING, b, row.right() - 120, label, t::DENSE_SIZE, t::INK);
        let check_x = row.right() - t::ROW_PADDING - 14;
        if chosen {
            self.items.push(Item::Check(Rect::new(check_x, row.y + (row.height - 14) / 2, 14, 14)));
        }
        if !value.is_empty() {
            let w = self.cx.text.width(value, t::SMALL_SIZE) as i32;
            let b = self.baseline(row.y, row.height, t::SMALL_SIZE);
            self.text(check_x - 10 - w, b, check_x - 4, value, t::SMALL_SIZE, t::INK_QUIET);
        }
        self.targets.push((row, target));
    }
}

impl SettingsWindow {
    /// The page's items and the pointer's targets, for a window `w` × `h`.
    fn layout(&self, w: i32, h: i32, cx: &Context<'_>) -> (Vec<Item>, Vec<(Rect, Target)>) {
        let mut b = Builder {
            cx,
            items: Vec::new(),
            targets: Vec::new(),
            hover: self.hover,
            left: t::SIDEBAR + t::GUTTER,
            right: w - t::GUTTER,
            y: t::HEADER + t::GUTTER,
        };
        // The header: where we are.
        let crumb = format!("Settings \u{203a} {}", self.page.label());
        let hb = b.baseline(0, t::HEADER, t::DENSE_SIZE);
        b.text(16, hb, w - 16, &crumb, t::DENSE_SIZE, t::INK_QUIET);
        // The sidebar.
        for (i, page) in Page::ALL.into_iter().enumerate() {
            let r = Rect::new(
                t::NAV_PADDING,
                t::HEADER + 4 + i as i32 * (t::NAV_HEIGHT + t::NAV_GAP),
                t::SIDEBAR - 2 * t::NAV_PADDING,
                t::NAV_HEIGHT,
            );
            let active = page == self.page;
            if active {
                b.items.push(Item::Fill { rect: r, rgb: t::SURFACE_3, alpha: 255, radius: t::FIELD_RADIUS });
            } else if self.hover == Some(Target::Nav(page)) {
                b.items.push(Item::Fill { rect: r, rgb: WHITE, alpha: t::GHOST_ALPHA, radius: t::FIELD_RADIUS });
            }
            let ink = if active { t::INK } else { t::INK_QUIET };
            let alpha = if active { 230 } else { 153 };
            b.items.push(Item::Glyph { page, x: r.x + 12, y: r.y + (r.height - 15) / 2, rgb: WHITE, alpha });
            let tb = b.baseline(r.y, r.height, t::DENSE_SIZE);
            b.text(r.x + 12 + 15 + 10, tb, r.right() - 8, page.label(), t::DENSE_SIZE, ink);
            b.targets.push((r, Target::Nav(page)));
        }
        // The page: its title, then its cards.
        let ty = b.y + 22;
        b.text(b.left, ty, b.right, self.page.label(), t::TITLE_SIZE, t::INK);
        b.y += 40;
        let st = cx.settings;
        match self.page {
            Page::Display => {
                b.section("Display");
                let rows = b.card(&[60, 60]);
                b.label(rows[0], "Resolution", Some("The display's mode"));
                let mut modes = cx.info.modes.clone();
                if modes.is_empty() {
                    modes.push(cx.info.display);
                }
                let current = st.resolution.unwrap_or(cx.info.display);
                let opts: Vec<(String, bool, Target)> = modes
                    .iter()
                    .enumerate()
                    .take(3)
                    .map(|(i, &(mw, mh))| (format!("{mw}\u{d7}{mh}"), (mw, mh) == current, Target::Mode(i)))
                    .collect();
                b.segmented(rows[0], &opts);
                b.label(rows[1], "Scale", Some("The size of everything on the screen"));
                let opts: Vec<(String, bool, Target)> =
                    SCALES.iter().map(|&s| (format!("{}%", s * 100), s == st.scale, Target::Scale(s))).collect();
                b.segmented(rows[1], &opts);
                if modes.len() <= 1 {
                    let y = b.y + 2;
                    b.text(
                        b.left + 4,
                        y,
                        b.right,
                        "Other modes come with the KMS display path.",
                        t::SMALL_SIZE,
                        t::INK_FAINT,
                    );
                }
            }
            Page::Keyboard => {
                b.section("Layout");
                let heights = vec![40; KEYBOARDS.len()];
                let rows = b.card(&heights);
                for (i, (id, label)) in KEYBOARDS.iter().enumerate() {
                    b.choice(rows[i], label, id, st.keyboard == *id, Target::Keyboard(i));
                }
                let y = b.y + 2;
                b.text(
                    b.left + 4,
                    y,
                    b.right,
                    "Every window gets the new layout at once.",
                    t::SMALL_SIZE,
                    t::INK_FAINT,
                );
            }
            Page::Wallpaper => {
                b.section("Wallpaper");
                let cols = 3;
                let gap = 12;
                let tw = (b.right - b.left - gap * (cols - 1)) / cols;
                let th = tw * 10 / 16;
                for (i, p) in PRESETS.iter().enumerate() {
                    let (c, r) = (i as i32 % cols, i as i32 / cols);
                    let rect = Rect::new(b.left + c * (tw + gap), b.y + r * (th + 28), tw, th);
                    b.items.push(Item::Tile { rect, colours: (p.2, p.3, p.4), selected: st.wallpaper == p.0 });
                    let lb = rect.bottom() + 18;
                    let ink = if st.wallpaper == p.0 { t::INK } else { t::INK_QUIET };
                    b.text(rect.x + 2, lb, rect.right(), p.1, t::SMALL_SIZE, ink);
                    b.targets.push((Rect::new(rect.x, rect.y, rect.width, rect.height + 24), Target::Wallpaper(i)));
                }
            }
            Page::Sound => {
                b.section("Output");
                let rows = b.card(&[60, 48]);
                let card = cx.info.sound.clone().unwrap_or_else(|| "No sound card found".into());
                b.label(rows[0], "Volume", Some(&card));
                let enabled = cx.info.sound.is_some();
                let pct = format!("{} %", st.volume);
                let tw = 220;
                let track =
                    Rect::new(rows[0].right() - t::ROW_PADDING - 44 - tw, rows[0].y + rows[0].height / 2 - 4, tw, 8);
                b.items.push(Item::Slider { rect: track, value: st.volume, enabled });
                let pb = b.baseline(rows[0].y, rows[0].height, t::SMALL_SIZE);
                b.text(track.right() + 10, pb, rows[0].right(), &pct, t::SMALL_SIZE, t::INK_QUIET);
                if enabled {
                    b.targets
                        .push((Rect::new(track.x - 6, rows[0].y, track.width + 12, rows[0].height), Target::Volume));
                }
                b.label(rows[1], "Mute", None);
                let toggle =
                    Rect::new(rows[1].right() - t::ROW_PADDING - 38, rows[1].y + (rows[1].height - 22) / 2, 38, 22);
                b.items.push(Item::Toggle { rect: toggle, on: st.muted });
                if enabled {
                    b.targets.push((toggle, Target::Mute));
                }
            }
            Page::DateTime => {
                b.section("Clock");
                let rows = b.card(&[48]);
                b.label(rows[0], "Local time", None);
                let x = rows[0].x + 120;
                b.value(rows[0], cx.now, x);
                b.section("Time zone");
                let heights = vec![32; ZONES.len()];
                let rows = b.card(&heights);
                for (i, (name, label, rule)) in ZONES.iter().enumerate() {
                    let off = TimeZone::parse(rule).map_or(0, |z| z.offset_minutes(cx.utc));
                    b.choice(rows[i], label, &utc_offset(off), st.time_zone == *name, Target::Zone(i));
                }
            }
            Page::About => {
                let i = cx.info;
                let rows_text: Vec<(&str, String)> = vec![
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
                let heights = vec![40; rows_text.len()];
                let rows = b.card(&heights);
                for (row, (label, value)) in rows.iter().zip(&rows_text) {
                    let lb = b.baseline(row.y, row.height, t::DENSE_SIZE);
                    b.text(row.x + t::ROW_PADDING, lb, row.x + 130, label, t::DENSE_SIZE, t::INK);
                    b.value(*row, value, row.x + 130);
                }
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
            Target::Nav(p) => {
                let redraw = p != self.page;
                self.page = p;
                return (redraw, None);
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
        self.page = Page::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }];
    }

    /// Draw the window, `w` × `h` logical at scale `s`, into `fb` (which is
    /// at least that big, physical).
    pub fn draw(&self, fb: &mut dyn Framebuffer, w: i32, h: i32, s: i32, cx: &Context<'_>) {
        let all = Rect::new(0, 0, w * s, h * s);
        fill(fb, all, all, t::SURFACE_0, 255);
        // The page's surface: rounded at the top left, a hairline along
        // its top and left edges.
        let page = Rect::new(t::SIDEBAR, t::HEADER, w - t::SIDEBAR, h - t::HEADER).scale(s);
        let r = t::CARD_RADIUS * s;
        let grown = Rect::new(page.x, page.y, page.width + r, page.height + r);
        rounded_rect(fb, page, grown, r, t::SURFACE_1, 255);
        rounded_outline(fb, page, grown, r, WHITE, t::HAIRLINE_ALPHA);
        let (items, _) = self.layout(w, h, cx);
        let text = cx.text;
        for item in &items {
            match item {
                Item::Text { x, y, max_x, text: s_, size, rgb, alpha } => {
                    text.draw(fb, all, (x * s) as f32, y * s, max_x * s, s_, size * s as f32, *rgb, *alpha);
                }
                Item::Card(c) => rounded_rect(fb, all, c.scale(s), t::CARD_RADIUS * s, WHITE, t::GHOST_ALPHA),
                Item::Line(l) => fill(fb, all, Rect::new(l.x * s, l.y * s, l.width * s, s), WHITE, t::HAIRLINE_ALPHA),
                Item::Fill { rect, rgb, alpha, radius } => {
                    rounded_rect(fb, all, rect.scale(s), radius * s, *rgb, *alpha)
                }
                Item::Segmented { rect, options } => {
                    rounded_rect(fb, all, rect.scale(s), t::FIELD_RADIUS * s, WHITE, t::GHOST_ALPHA);
                    for (r, label, active) in options {
                        let size = t::SMALL_SIZE * s as f32;
                        if *active {
                            rounded_rect(fb, all, r.scale(s), 6 * s, t::INK, 255);
                        }
                        let ink = if *active { t::SURFACE_0 } else { t::INK_QUIET };
                        let lw = text.width(label, size) as i32;
                        let rs = r.scale(s);
                        let base = rs.y + (rs.height + text.cap_height(size) as i32) / 2;
                        text.draw(
                            fb,
                            all,
                            (rs.x + (rs.width - lw) / 2) as f32,
                            base,
                            rs.right(),
                            label,
                            size,
                            ink,
                            255,
                        );
                    }
                }
                Item::Slider { rect, value, enabled } => {
                    let r = rect.scale(s);
                    let a = if *enabled { 255 } else { 90 };
                    rounded_rect(fb, all, r, r.height / 2, WHITE, t::TRACK_ALPHA);
                    let filled = Rect::new(r.x, r.y, r.width * *value as i32 / 100, r.height);
                    if filled.width > 0 {
                        rounded_rect(fb, all, filled, r.height / 2, t::INK, a);
                    }
                }
                Item::Toggle { rect, on } => {
                    let r = rect.scale(s);
                    if *on {
                        rounded_rect(fb, all, r, r.height / 2, t::INK, 255);
                    } else {
                        rounded_rect(fb, all, r, r.height / 2, WHITE, t::TRACK_ALPHA);
                    }
                    let knob = 16 * s;
                    let kx = r.x + if *on { 19 * s } else { 3 * s };
                    let colour = if *on { t::SURFACE_0 } else { t::KNOB_OFF };
                    draw::circle(fb, all, kx + knob / 2, r.y + r.height / 2, knob / 2, None, colour, 255);
                }
                Item::Tile { rect, colours, selected } => {
                    let r = rect.scale(s);
                    let rad = t::FIELD_RADIUS * s;
                    // The preset's gradient, row by row, inside the rounded tile.
                    let (top, bottom, glow) = *colours;
                    for y in r.y..r.bottom() {
                        let k = ((y - r.y) * 256 / r.height.max(1)) as u32;
                        let mixc = |sh: u32| {
                            let (a, b) = ((top >> sh) & 0xff, (bottom >> sh) & 0xff);
                            ((a * (256 - k) + b * k) / 256) << sh
                        };
                        let row_colour = mixc(16) | mixc(8) | mixc(0);
                        rounded_rect(fb, Rect::new(all.x, y, all.width, 1), r, rad, row_colour, 255);
                    }
                    // The glow, soft: discs growing fainter outwards.
                    for k in 1..=4 {
                        let rad = r.height * (6 - k) / 10;
                        draw::circle(fb, r, r.x + r.width / 2, r.y + r.height / 3, rad, None, glow, 22);
                    }
                    if *selected {
                        rounded_outline(fb, all, r, rad, WHITE, 179);
                        let inner = Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2);
                        rounded_outline(fb, all, inner, rad - 1, WHITE, 179);
                    } else {
                        rounded_outline(fb, all, r, rad, WHITE, t::HAIRLINE_ALPHA);
                    }
                }
                Item::Check(c) => {
                    // A tick: a short stroke down-right, a long one up-right.
                    let r = c.scale(s);
                    let (x0, y0) = (r.x + r.width / 8, r.y + r.height / 2);
                    let (x1, y1) = (r.x + r.width * 3 / 8, r.y + r.height * 3 / 4);
                    let (x2, y2) = (r.right() - r.width / 8, r.y + r.height / 5);
                    line(fb, all, (x0, y0), (x1, y1), s.max(1) * 2, t::INK);
                    line(fb, all, (x1, y1), (x2, y2), s.max(1) * 2, t::INK);
                }
                Item::Glyph { page, x, y, rgb, alpha } => glyph(fb, all, *page, x * s, y * s, s, *rgb, *alpha),
            }
        }
    }
}

/// A straight line `thick` pixels wide (squares along it).
fn line(fb: &mut dyn Framebuffer, clip: Rect, a: (i32, i32), b: (i32, i32), thick: i32, rgb: u32) {
    let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
    for i in 0..=steps {
        let x = a.0 + (b.0 - a.0) * i / steps;
        let y = a.1 + (b.1 - a.1) * i / steps;
        fill(fb, clip, Rect::new(x - thick / 2, y - thick / 2, thick, thick), rgb, 255);
    }
}

/// A page's 15-pixel icon at physical (`x`, `y`), scale `s`.
#[allow(clippy::too_many_arguments)]
fn glyph(fb: &mut dyn Framebuffer, clip: Rect, page: Page, x: i32, y: i32, s: i32, rgb: u32, alpha: u8) {
    let r = |dx: i32, dy: i32, w: i32, h: i32| Rect::new(x + dx * s, y + dy * s, w * s, h * s);
    let outline = |fb: &mut dyn Framebuffer, b: Rect| {
        for e in [
            Rect::new(b.x, b.y, b.width, s),
            Rect::new(b.x, b.bottom() - s, b.width, s),
            Rect::new(b.x, b.y, s, b.height),
            Rect::new(b.right() - s, b.y, s, b.height),
        ] {
            fill(fb, clip, e, rgb, alpha);
        }
    };
    match page {
        Page::Display => {
            outline(fb, r(0, 2, 15, 9));
            fill(fb, clip, r(7, 11, 1, 2), rgb, alpha);
            fill(fb, clip, r(4, 13, 7, 1), rgb, alpha);
        }
        Page::Keyboard => {
            outline(fb, r(0, 3, 15, 9));
            for i in 0..4 {
                fill(fb, clip, r(3 + 3 * i, 6, 1, 1), rgb, alpha);
            }
            fill(fb, clip, r(4, 9, 7, 1), rgb, alpha);
        }
        Page::Wallpaper => {
            outline(fb, r(0, 2, 15, 11));
            draw::circle(fb, clip, x + 10 * s, y + 6 * s, 2 * s, None, rgb, alpha);
            for i in 0..5 {
                fill(fb, clip, r(2 + i, 10 - i, 1, 2 + i), rgb, alpha);
                fill(fb, clip, r(10 - i, 10 - i, 1, 2 + i), rgb, alpha);
            }
        }
        Page::Sound => {
            fill(fb, clip, r(1, 5, 3, 5), rgb, alpha);
            for i in 0..4 {
                fill(fb, clip, r(4 + i, 5 - i, 1, 5 + 2 * i), rgb, alpha);
            }
            let half = clip.intersect(&r(9, 0, 6, 15));
            draw::circle(fb, half, x + 8 * s, y + 7 * s, 4 * s, Some(s), rgb, alpha);
        }
        Page::DateTime => {
            draw::circle(fb, clip, x + 7 * s, y + 7 * s, 7 * s, Some(s), rgb, alpha);
            fill(fb, clip, r(7, 3, 1, 5), rgb, alpha);
            fill(fb, clip, r(7, 7, 3, 1), rgb, alpha);
        }
        Page::About => {
            draw::circle(fb, clip, x + 7 * s, y + 7 * s, 7 * s, Some(s), rgb, alpha);
            fill(fb, clip, r(7, 4, 1, 1), rgb, alpha);
            fill(fb, clip, r(7, 6, 1, 5), rgb, alpha);
        }
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
        let (wx, wy) = at(&win, &c, Target::Wallpaper(2));
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
            assert_eq!(fb.pixel(10, 300), Some(t::SURFACE_0), "the sidebar's ground");
            assert_eq!(fb.pixel(W as u32 - 5, H as u32 - 5), Some(t::SURFACE_1), "the page");
        }
        // The active page is highlighted in the sidebar.
        win.page = Page::Display;
        let mut fb = MemoryFramebuffer::new(W as u32, H as u32);
        win.draw(&mut fb, W, H, 1, &c);
        assert_eq!(fb.pixel(t::SIDEBAR as u32 - 20, t::HEADER as u32 + 8), Some(t::SURFACE_3));
        // At scale 2 too.
        let mut fb = MemoryFramebuffer::new(2 * W as u32, 2 * H as u32);
        win.draw(&mut fb, W, H, 2, &c);
        assert_eq!(fb.pixel(2 * W as u32 - 5, 2 * H as u32 - 5), Some(t::SURFACE_1));
    }

    #[test]
    fn offsets() {
        assert_eq!(utc_offset(60), "UTC+01:00");
        assert_eq!(utc_offset(-300), "UTC-05:00");
        assert_eq!(utc_offset(330), "UTC+05:30");
    }
}
