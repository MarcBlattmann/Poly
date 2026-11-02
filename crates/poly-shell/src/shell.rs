//! The shell: layout, drawing and input of the ridge, the surfaces that
//! hang from it (the launcher, the system menu, the control centre, the
//! calendar, the notification history and the notices, the window
//! switcher), the wallpaper and the snap preview, as two compositor layers
//! ([`Shell::below`] under the windows, [`Shell::above`] over them), and
//! the settings window's contents. It looks like Alpine Shell (ADR 0127).
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────────────┐
//!  │ ◌                       ● 2 new           [ ⇄ ◖) ]  21:41 Nov 1  │ ← the ridge, 24 px, rgb(32,32,32)
//!  ├─────────────╮      ╭────────────────────╮            ╭──────────┤
//!  │             ╰──────╯ ⌕ Search apps…     ╰────────────╯ ● Hello  │ ← surfaces hang from it:
//!  │                     ╰──────────────────╯                ╰───────┤   one at a time, shoulders
//!  └──────────────────────────────────────────────────────────────────┘   at the free sides
//! ```
//!
//! Only one surface shows at a time, as in Alpine: the launcher above the
//! switcher above a panel opened from the ridge above the notices. Panels
//! open when the pointer comes onto their item on the ridge (or on a
//! click) and close when it leaves the item and the panel.
//!
//! Coordinates are logical; drawing multiplies them by the output's scale.
//! Everything that changes damages its rectangle ([`Shell::take_damage`]),
//! which the compositor hands to the scene, so only that is drawn again.

mod panels;
mod wallpaper;

use crate::apps::App;
use crate::chrome::{Geometry, Motion};
use crate::draw::{self, Face, Style, Text, fill, rounded_outline, rounded_rect};
use crate::icons::{Icon, lucide};
use crate::launcher::{Key, Launcher, Outcome};
use crate::notify::Notifications;
use crate::settings::{Change, Settings};
use crate::settings_ui::{Context, Page, SettingsWindow, SystemInfo};
use crate::switcher::{Entry, Switcher, look as sw};
use crate::theme::{ink, launcher as lt, overlay as ot, ridge as rt};
use crate::tz::TimeZone;
use alloc::string::String;
use alloc::vec::Vec;
use poly_dbus::notifications::Notify;
use poly_wayland::compose::{Framebuffer, Layer, Rect, WindowId};

pub use panels::{ControlView, Panel};

const WHITE: u32 = 0xff_ffff;

/// What a pointer press did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press {
    /// The shell took it.
    Taken,
    /// The shell took it: start this app.
    Launch(App),
    /// The shell took it: switch to this window (a switcher card).
    Switch(WindowId),
    /// The shell took it: open the settings window at this page.
    Settings(Page),
    /// The shell took it: the system menu asks to restart or shut down
    /// (confirmed).
    Power(Power),
    /// Not the shell's: the windows get it.
    NotMine,
}

/// The system menu's actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Restart,
    ShutDown,
}

/// The network as the ridge shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Network {
    /// Not known (yet).
    #[default]
    Unknown,
    /// No interface but loopback is up with an address.
    Offline,
    /// Connected through this interface.
    Online(String),
}

/// The surfaces that hang from the ridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Launcher,
    Switcher,
    Panel(Panel),
    Notices,
}

/// The launcher's parts, logical, for its body at `top`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LauncherLayout {
    body: Rect,
    search: Rect,
    /// The first row's rectangle; the others follow every
    /// `ROW_HEIGHT + ROW_GAP`.
    first_row: Rect,
    rows: usize,
    footer: Option<Rect>,
}

/// The ridge's parts, logical.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RidgeLayout {
    menu: Rect,
    clock: Rect,
    status: Rect,
    /// The unread notifications in the middle, when there are some.
    notice: Option<Rect>,
}

/// The desktop shell.
#[derive(Debug)]
pub struct Shell {
    width: i32,
    height: i32,
    apps: Vec<App>,
    launcher: Launcher,
    text: Text,
    clock: String,
    /// The time last set: Unix seconds and the offset east of UTC.
    time: (i64, i32),
    damage: Vec<Rect>,
    settings: Settings,
    zone: TimeZone,
    network: Network,
    /// The master volume and mute (`None`: no sound card).
    volume: Option<(u32, bool)>,
    sound_card: Option<String>,
    notes: Notifications,
    now_ms: u64,
    switcher: Switcher,
    snap: Option<Rect>,
    /// The panel open from the ridge, and its state.
    panel: Option<Panel>,
    panels: panels::State,
    motion: Motion<Surface>,
    /// What the surface covered when last drawn (logical).
    surface_extent: Rect,
    pointer: (i32, i32),
    /// Settings changes made in the panels, for the compositor.
    changes: Vec<Change>,
    /// The wallpaper drawn once at the screen's size, copied from then on.
    wall: core::cell::RefCell<Option<wallpaper::Cache>>,
    /// The wallpapers' pictures (Alpine Shell's), by preset.
    pictures: crate::picture::Pictures,
    /// The settings window's state (its contents are the shell's).
    pub settings_window: SettingsWindow,
}

impl Shell {
    /// A shell on a `width` × `height` logical output with these apps and
    /// fonts. The launcher shows at the start (an empty desktop), already
    /// open.
    pub fn new(width: i32, height: i32, apps: Vec<App>, text: Text) -> Shell {
        let settings = Settings::default();
        let zone = settings.zone();
        let mut s = Shell {
            width,
            height,
            apps,
            launcher: Launcher::new(lt::MAX_ROWS),
            text,
            clock: String::new(),
            time: (0, 0),
            damage: Vec::new(),
            settings,
            zone,
            network: Network::Unknown,
            volume: None,
            sound_card: None,
            notes: Notifications::new(),
            now_ms: 0,
            switcher: Switcher::default(),
            snap: None,
            panel: None,
            panels: panels::State::default(),
            motion: Motion::default(),
            surface_extent: Rect::default(),
            pointer: (-1, -1),
            changes: Vec::new(),
            wall: core::cell::RefCell::new(None),
            pictures: crate::picture::Pictures::new(),
            settings_window: SettingsWindow::default(),
        };
        let rows = s.room_for_rows();
        s.launcher.set_limit(rows);
        s.launcher.show();
        // At the start the launcher is simply there.
        s.motion.set_animate(false);
        s.sync();
        s.motion.set_animate(true);
        s
    }

    /// Animate the surfaces (the default) or move them at once.
    pub fn set_animations(&mut self, on: bool) {
        self.motion.set_animate(on);
    }

    pub fn apps(&self) -> &[App] {
        &self.apps
    }

    pub fn launcher(&self) -> &Launcher {
        &self.launcher
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// How many logical rows the ridge keeps free at the top.
    pub fn reserved_top(&self) -> i32 {
        rt::HEIGHT
    }

    pub fn ridge_rect(&self) -> Rect {
        Rect::new(0, 0, self.width, rt::HEIGHT)
    }

    fn screen(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// The rectangles that changed since the last call, logical.
    pub fn take_damage(&mut self) -> Vec<Rect> {
        core::mem::take(&mut self.damage)
    }

    /// Settings changes made in the shell's panels (the volume slider, the
    /// mute button) since the last call, for the compositor to apply.
    pub fn take_changes(&mut self) -> Vec<Change> {
        core::mem::take(&mut self.changes)
    }

    /// The output's logical size changed (another scale or mode).
    pub fn resize(&mut self, width: i32, height: i32) {
        (self.width, self.height) = (width, height);
        let rows = self.room_for_rows();
        self.launcher.set_limit(rows);
        self.damage.push(self.screen());
        self.sync();
    }

    /// The clock's text now.
    pub fn clock(&self) -> &str {
        &self.clock
    }

    /// The time is `secs` on the Unix clock (`offset_minutes` east of UTC):
    /// the ridge's clock shows it. True if the text changed.
    pub fn set_time(&mut self, secs: i64, offset_minutes: i32) -> bool {
        self.time = (secs, offset_minutes);
        let text = crate::clock::ridge_text(&crate::clock::civil(secs, offset_minutes));
        if text == self.clock {
            return false;
        }
        self.clock = text;
        self.damage.push(self.ridge_rect());
        if self.panel == Some(Panel::Calendar) {
            self.touch();
        }
        true
    }

    /// Minutes east of UTC at `secs` in the configured time zone.
    pub fn local_offset(&self, secs: i64) -> i32 {
        self.zone.offset_minutes(secs)
    }

    /// The local time at `secs` with the zone's abbreviation.
    pub fn local_time(&self, secs: i64) -> String {
        let c = crate::clock::civil(secs, self.local_offset(secs));
        alloc::format!("{} {}", crate::clock::ridge_text(&c), self.zone.abbreviation(secs))
    }

    // -----------------------------------------------------------------
    // Settings and status.

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Settings read at start: wallpaper and time zone take effect.
    pub fn set_settings(&mut self, s: Settings) {
        self.settings = s;
        self.zone = self.settings.zone();
        self.clock.clear();
        self.damage.push(self.screen());
    }

    /// Apply a change to the settings (the shell's side of it: wallpaper,
    /// time zone); true if anything changed. The compositor applies the
    /// rest (keymap, volume, scale) and saves the file.
    pub fn apply(&mut self, c: &Change) -> bool {
        if !self.settings.apply(c) {
            return false;
        }
        match c {
            Change::Wallpaper(_) => self.damage.push(self.screen()),
            Change::TimeZone(_) => {
                self.zone = self.settings.zone();
                // The clock is drawn again with the next time set.
                self.clock.clear();
            }
            Change::Volume(_) | Change::Muted(_) => {
                if let Some((v, m)) = self.volume.as_mut() {
                    match c {
                        Change::Volume(n) => *v = *n,
                        Change::Muted(b) => *m = *b,
                        _ => {}
                    }
                }
                self.damage.push(self.ridge_rect());
                self.touch();
            }
            _ => {}
        }
        true
    }

    /// The network's state; true if it changed.
    pub fn set_network(&mut self, n: Network) -> bool {
        if n == self.network {
            return false;
        }
        self.network = n;
        self.damage.push(self.ridge_rect());
        self.touch();
        true
    }

    pub fn network(&self) -> &Network {
        &self.network
    }

    /// The mixer's master volume (0-100) and mute, `None` without a sound
    /// card; true if it changed.
    pub fn set_volume(&mut self, v: Option<(u32, bool)>) -> bool {
        if v == self.volume {
            return false;
        }
        self.volume = v;
        self.damage.push(self.ridge_rect());
        self.touch();
        true
    }

    pub fn volume(&self) -> Option<(u32, bool)> {
        self.volume
    }

    /// The picture of wallpaper preset `id` (the compositor reads Alpine
    /// Shell's from the disk).
    pub fn add_wallpaper(&mut self, id: &str, picture: crate::picture::Picture) {
        self.pictures.insert(id.into(), picture);
        *self.wall.borrow_mut() = None;
        self.damage.push(self.screen());
    }

    /// The sound card's name (the control centre shows it).
    pub fn set_sound_card(&mut self, name: Option<String>) {
        if self.sound_card != name {
            self.sound_card = name;
            self.touch();
        }
    }

    // -----------------------------------------------------------------
    // Notifications.

    /// Run `f`, then damage what changed (the notices, the history, the
    /// ridge's unread count).
    fn notes_change<R>(&mut self, f: impl FnOnce(&mut Shell) -> R) -> R {
        let unread = self.notes.unread();
        let r = f(self);
        if self.notes.take_changed() {
            if unread != self.notes.unread() {
                self.damage.push(self.ridge_rect());
            }
            self.touch();
        }
        r
    }

    /// The clock for timeouts and animations, in milliseconds (any
    /// monotonic clock).
    pub fn set_now_ms(&mut self, now: u64) {
        self.now_ms = now;
        self.notes_change(|s| s.notes.tick(now));
        self.sync();
    }

    /// A `Notify` call: shown (or queued), kept in the history; its ID.
    pub fn notify(&mut self, n: Notify) -> u32 {
        let now = self.now_ms;
        self.notes_change(|s| s.notes.notify(n, now))
    }

    /// `CloseNotification` (or another close): true if it was open.
    pub fn close_notification(&mut self, id: u32, reason: u32) -> bool {
        let now = self.now_ms;
        self.notes_change(|s| s.notes.close(id, reason, now))
    }

    /// `(id, reason)` of notifications closed since the last call, for
    /// `NotificationClosed`.
    pub fn take_closed_notifications(&mut self) -> Vec<(u32, u32)> {
        self.notes.take_closed()
    }

    /// `(id, action key)` invoked since the last call, for `ActionInvoked`.
    pub fn take_invoked_actions(&mut self) -> Vec<(u32, String)> {
        self.notes.take_invoked()
    }

    /// When the shell wants to be woken next (on the [`Shell::set_now_ms`]
    /// clock): a notice's expiry, or the next frame while a surface moves.
    pub fn next_notification_deadline(&self) -> Option<u64> {
        let frame = self.motion.animating(self.now_ms).then_some(self.now_ms + crate::theme::chrome::FRAME_MS);
        match (self.notes.next_deadline(), frame) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn notifications(&self) -> &Notifications {
        &self.notes
    }

    pub fn history_open(&self) -> bool {
        self.panel == Some(Panel::History)
    }

    /// Open or close the notification history (opening marks everything
    /// read).
    pub fn toggle_history(&mut self) {
        if self.history_open() {
            self.close_panel();
        } else {
            self.open_panel(Panel::History);
        }
    }

    /// A key while an overlay (a panel) is open; true if taken.
    pub fn overlay_key(&mut self, key: Key) -> bool {
        if self.panel.is_some() && key == Key::Escape {
            self.close_panel();
            return true;
        }
        false
    }

    // -----------------------------------------------------------------
    // Panels.

    /// The panel open from the ridge.
    pub fn panel(&self) -> Option<Panel> {
        self.panel
    }

    /// Open `p` (closing another; the launcher and the switcher go).
    pub fn open_panel(&mut self, p: Panel) {
        self.hide_launcher();
        if self.switcher.is_open() {
            self.switcher.cancel();
        }
        if p == Panel::History {
            self.notes.mark_read();
            self.notes.take_changed();
            self.damage.push(self.ridge_rect());
        }
        if self.panel != Some(p) {
            self.panels = panels::State::default();
        }
        self.panel = Some(p);
        self.damage.push(self.ridge_rect());
        self.touch();
    }

    pub fn close_panel(&mut self) {
        if self.panel.take().is_some() {
            self.panels = panels::State::default();
            self.damage.push(self.ridge_rect());
            self.touch();
        }
    }

    // -----------------------------------------------------------------
    // The surface on the screen.

    /// The surface that should show now and where.
    fn wanted(&self) -> Option<(Surface, Geometry)> {
        if self.launcher.is_visible() {
            let w = self.launcher_width();
            let h = self.launcher_layout(0).body.height;
            return Some((Surface::Launcher, Geometry::centred(self.width, w, h)));
        }
        if self.switcher.is_open() {
            let (p, _) = self.switcher.layout(self.width, self.height);
            return Some((Surface::Switcher, Geometry::centred(self.width, p.width, p.height)));
        }
        if let Some(p) = self.panel {
            return Some((Surface::Panel(p), self.panel_geometry(p)));
        }
        if !self.notes.toasts().is_empty() {
            let h = self.notice_layout(0).iter().map(|n| n.rect.bottom()).max().unwrap_or(0);
            return Some((Surface::Notices, Geometry::at_right(self.width, self.notice_width(), h)));
        }
        None
    }

    /// What the surface covers now, logical.
    fn current_extent(&self) -> Rect {
        match self.motion.drawn(self.now_ms) {
            Some((_, g, h)) => g.chrome(rt::HEIGHT, h, 1).bounds(),
            None => Rect::default(),
        }
    }

    /// Bring the surface up to date: what shows, where, how far open; damage
    /// what it covered and covers.
    fn sync(&mut self) {
        let changed = self.motion.want(self.wanted(), self.now_ms);
        let extent = self.current_extent();
        if changed || extent != self.surface_extent || self.motion.animating(self.now_ms) {
            for r in [self.surface_extent, extent] {
                if !r.is_empty() {
                    self.damage.push(r);
                }
            }
            self.surface_extent = extent;
        }
    }

    /// The surface's contents changed: draw it again.
    fn touch(&mut self) {
        self.sync();
        if !self.surface_extent.is_empty() {
            self.damage.push(self.surface_extent);
        }
    }

    /// The surface that is open (not one closing).
    pub fn surface(&self) -> Option<Surface> {
        self.motion.shown()
    }

    // -----------------------------------------------------------------
    // The switcher and the snap preview.

    pub fn switcher_open(&self) -> bool {
        self.switcher.is_open()
    }

    pub fn switcher(&self) -> &Switcher {
        &self.switcher
    }

    /// The switcher's panel and cards where they hang (logical).
    fn switcher_layout(&self, top: i32) -> (Rect, Vec<Rect>) {
        let (p, cards) = self.switcher.layout(self.width, self.height);
        let dy = top - p.y;
        (p.translate(0, dy), cards.iter().map(|c| c.translate(0, dy)).collect())
    }

    fn switcher_change(&mut self, f: impl FnOnce(&mut Switcher)) {
        f(&mut self.switcher);
        self.touch();
    }

    /// Open the switcher on `entries` (most recently used first).
    pub fn open_switcher(&mut self, entries: Vec<Entry>, backward: bool) {
        self.hide_launcher();
        self.panel = None;
        self.switcher_change(|s| s.open(entries, backward));
    }

    pub fn switcher_step(&mut self, forward: bool) {
        self.switcher_change(|s| s.step(forward));
    }

    /// Close the switcher: the window chosen.
    pub fn switcher_commit(&mut self) -> Option<WindowId> {
        let mut w = None;
        self.switcher_change(|s| w = s.commit());
        w
    }

    pub fn switcher_cancel(&mut self) {
        self.switcher_change(Switcher::cancel);
    }

    /// Show where a dragged window would snap (its frame's rectangle), or
    /// nothing.
    pub fn set_snap_preview(&mut self, r: Option<Rect>) {
        if r == self.snap {
            return;
        }
        if let Some(old) = self.snap {
            self.damage.push(old);
        }
        self.snap = r;
        if let Some(new) = r {
            self.damage.push(new);
        }
    }

    pub fn snap_preview(&self) -> Option<Rect> {
        self.snap
    }

    // -----------------------------------------------------------------
    // The launcher.

    /// How many rows fit in the list (Alpine's cap: 560 pixels or 70 % of
    /// the screen).
    fn room_for_rows(&self) -> usize {
        let cap = lt::LIST_CAP.min(self.height * lt::LIST_SHARE_PERCENT / 100);
        let room = (cap - lt::SECTION_HEIGHT - lt::LIST_PADDING + lt::ROW_GAP) / (lt::ROW_HEIGHT + lt::ROW_GAP);
        (room.max(1) as usize).min(lt::MAX_ROWS)
    }

    fn launcher_width(&self) -> i32 {
        lt::WIDTH.min(self.width - 2 * lt::SIDE_MARGIN).max(200)
    }

    /// The launcher with its body's top at `top`.
    fn launcher_layout(&self, top: i32) -> LauncherLayout {
        let w = self.launcher_width();
        let x = (self.width - w) / 2;
        let search = Rect::new(x, top, w, lt::SEARCH_HEIGHT);
        let n = self.launcher.results().len();
        let list_top = search.bottom() + 1 + lt::SECTION_HEIGHT;
        let first_row = Rect::new(x + lt::LIST_PADDING, list_top, w - 2 * lt::LIST_PADDING, lt::ROW_HEIGHT);
        // Without results (nothing typed, or nothing found) only the search
        // row shows, as in Alpine.
        let (height, footer) = if n == 0 {
            (lt::SEARCH_HEIGHT, None)
        } else {
            let rows = n as i32;
            let list = lt::SECTION_HEIGHT + rows * lt::ROW_HEIGHT + (rows - 1) * lt::ROW_GAP + lt::LIST_PADDING;
            let body = lt::SEARCH_HEIGHT + 1 + list;
            (body + 1 + lt::FOOTER_HEIGHT, Some(Rect::new(x, top + body + 1, w, lt::FOOTER_HEIGHT)))
        };
        LauncherLayout { body: Rect::new(x, top, w, height), search, first_row, rows: n, footer }
    }

    /// Run `f` on the launcher, then bring the surface up to date.
    fn change(&mut self, f: impl FnOnce(&mut Shell) -> Outcome) -> Outcome {
        let was_visible = self.launcher.is_visible();
        let out = f(self);
        if out != Outcome::Unchanged || was_visible != self.launcher.is_visible() {
            self.touch();
        }
        out
    }

    /// The hotkey: show the launcher, or hide it.
    pub fn toggle_launcher(&mut self) {
        if !self.launcher.is_visible() {
            self.panel = None;
        }
        self.change(|s| {
            s.launcher.toggle();
            Outcome::Changed
        });
    }

    pub fn hide_launcher(&mut self) {
        if self.launcher.is_visible() {
            self.change(|s| {
                s.launcher.hide();
                Outcome::Hidden
            });
        }
    }

    /// A key while the launcher is shown; an app to start, if Enter chose
    /// one.
    pub fn key(&mut self, key: Key) -> Option<App> {
        match self.change(|s| s.launcher.key(&s.apps, key)) {
            Outcome::Launch(app) => Some(app),
            _ => None,
        }
    }

    /// The row at logical (`x`, `y`), if the launcher shows one there.
    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.launcher.is_visible() {
            return None;
        }
        let l = self.launcher_layout(rt::HEIGHT);
        (0..l.rows).find(|&i| self.row_rect(&l, i).contains(x, y))
    }

    fn row_rect(&self, l: &LauncherLayout, i: usize) -> Rect {
        l.first_row.translate(0, i as i32 * (lt::ROW_HEIGHT + lt::ROW_GAP))
    }

    // -----------------------------------------------------------------
    // Pointer input.

    /// The pointer is at logical (`x`, `y`): hovering a launcher row or a
    /// switcher card selects it; coming onto an item of the ridge opens its
    /// panel, leaving the item and the panel closes it.
    pub fn pointer_motion(&mut self, x: i32, y: i32) {
        let before = self.pointer;
        self.pointer = (x, y);
        let ridge = self.ridge_layout();
        if ridge.clock.contains(before.0, before.1) != ridge.clock.contains(x, y) {
            self.damage.push(self.ridge_rect());
        }
        if self.switcher.is_open() {
            let (_, cards) = self.switcher_layout(rt::HEIGHT);
            if let Some(i) = cards.iter().position(|c| c.contains(x, y))
                && self.switcher.selected() != i
            {
                self.switcher_change(|s| {
                    s.select(i);
                });
            }
            return;
        }
        if self.launcher.is_visible() {
            let row = self.row_at(x, y);
            if self.launcher.hover() != row {
                self.change(|s| if s.launcher.set_hover(row) { Outcome::Changed } else { Outcome::Unchanged });
            }
            return;
        }
        if self.panels.dragging {
            self.panel_drag(x, y);
            return;
        }
        if let Some(p) = self.trigger_at(&ridge, x, y) {
            if self.panel != Some(p) {
                self.open_panel(p);
            }
            return;
        }
        if let Some(p) = self.panel {
            let body = self.panel_geometry(p).bounds(rt::HEIGHT);
            if !body.contains(x, y) && !self.trigger_rect(&ridge, p).contains(x, y) {
                self.close_panel();
            } else {
                self.panel_hover(x, y);
            }
        }
    }

    /// Which panel the ridge's item at (`x`, `y`) opens.
    fn trigger_at(&self, r: &RidgeLayout, x: i32, y: i32) -> Option<Panel> {
        if !(0..rt::HEIGHT).contains(&y) {
            return None;
        }
        [Panel::Menu, Panel::Control, Panel::Calendar, Panel::History]
            .into_iter()
            .find(|&p| self.trigger_rect(r, p).contains(x, y))
    }

    /// The ridge item that opens `p`, the full height of the ridge.
    fn trigger_rect(&self, r: &RidgeLayout, p: Panel) -> Rect {
        let full = |q: Rect| Rect::new(q.x, 0, q.width, rt::HEIGHT);
        match p {
            Panel::Menu => r.menu,
            Panel::Control => full(r.status),
            Panel::Calendar => full(r.clock),
            Panel::History => r.notice.map(full).unwrap_or_default(),
        }
    }

    /// A press of the left button at logical (`x`, `y`). The switcher and
    /// the launcher take every press while they show (a press outside
    /// closes them, as clicking Alpine's transparent overlay does); an
    /// open panel takes the presses on it (others close it and go on);
    /// the notices and the ridge take theirs.
    pub fn press(&mut self, x: i32, y: i32) -> Press {
        if self.switcher.is_open() {
            let (_, cards) = self.switcher_layout(rt::HEIGHT);
            return match cards.iter().position(|c| c.contains(x, y)) {
                Some(i) => {
                    self.switcher_change(|s| {
                        s.select(i);
                    });
                    self.switcher_commit().map_or(Press::Taken, Press::Switch)
                }
                None => {
                    self.switcher_cancel();
                    Press::Taken
                }
            };
        }
        if self.launcher.is_visible() {
            if let Some(row) = self.row_at(x, y) {
                return match self.change(|s| s.launcher.activate(&s.apps, row)) {
                    Outcome::Launch(app) => Press::Launch(app),
                    _ => Press::Taken,
                };
            }
            let g = self.wanted().map(|w| w.1.bounds(rt::HEIGHT)).unwrap_or_default();
            if !g.contains(x, y) {
                self.hide_launcher();
            }
            return Press::Taken;
        }
        let ridge = self.ridge_layout();
        if let Some(p) = self.panel {
            if self.panel_geometry(p).bounds(rt::HEIGHT).contains(x, y) {
                return self.panel_press(p, x, y);
            }
            if self.trigger_rect(&ridge, p).contains(x, y) {
                self.close_panel();
                return Press::Taken;
            }
            self.close_panel();
        }
        if self.surface() == Some(Surface::Notices)
            && let Some(press) = self.notices_press(x, y)
        {
            return press;
        }
        if self.ridge_rect().contains(x, y) {
            if let Some(p) = self.trigger_at(&ridge, x, y) {
                self.open_panel(p);
            }
            return Press::Taken;
        }
        Press::NotMine
    }

    /// The left button went up (ends a slider's drag).
    pub fn release(&mut self) {
        if self.panels.dragging {
            self.panels.dragging = false;
            self.touch();
        }
    }

    /// The layer under the windows: the wallpaper and the ridge.
    pub fn below(&self) -> Below<'_> {
        Below(self)
    }

    /// The layer over the windows: the snap preview and the surface that
    /// hangs from the ridge.
    pub fn above(&self) -> Above<'_> {
        Above(self)
    }

    // -----------------------------------------------------------------
    // The settings window's contents.

    /// Draw the settings window, `w` × `h` logical at scale `s`.
    pub fn draw_settings(&self, fb: &mut dyn Framebuffer, w: i32, h: i32, s: i32, info: &SystemInfo, utc: i64) {
        let now = self.local_time(utc);
        let cx = Context { settings: &self.settings, info, text: &self.text, now: &now, utc, pictures: &self.pictures };
        self.settings_window.draw(fb, w, h, s, &cx);
    }

    /// The pointer moved in the settings window: (draw again, a change).
    pub fn settings_motion(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        info: &SystemInfo,
        utc: i64,
    ) -> (bool, Option<Change>) {
        let now = self.local_time(utc);
        let cx = Context { settings: &self.settings, info, text: &self.text, now: &now, utc, pictures: &self.pictures };
        self.settings_window.motion(x, y, w, h, &cx)
    }

    /// A left button in the settings window: (draw again, a change).
    #[allow(clippy::too_many_arguments)]
    pub fn settings_button(
        &mut self,
        x: i32,
        y: i32,
        pressed: bool,
        w: i32,
        h: i32,
        info: &SystemInfo,
        utc: i64,
    ) -> (bool, Option<Change>) {
        let now = self.local_time(utc);
        let cx = Context { settings: &self.settings, info, text: &self.text, now: &now, utc, pictures: &self.pictures };
        self.settings_window.button(x, y, pressed, w, h, &cx)
    }

    // -----------------------------------------------------------------
    // The ridge.

    fn ridge_layout(&self) -> RidgeLayout {
        let st = Style::new(Face::Sans, rt::TEXT_SIZE);
        let tw = self.text.measure(&self.clock, st) as i32;
        let clock_w = tw + 2 * rt::PILL_PADDING;
        let pill_y = (rt::HEIGHT - rt::PILL_HEIGHT) / 2;
        let clock = Rect::new(self.width - rt::RIGHT_MARGIN - clock_w, pill_y, clock_w, rt::PILL_HEIGHT);
        let icons = 2 * rt::ICON + rt::GAP;
        let status_w = icons + 2 * rt::PILL_PADDING;
        let status = Rect::new(clock.x - rt::GAP - status_w, pill_y, status_w, rt::PILL_HEIGHT);
        let menu = Rect::new(0, 0, rt::MENU_ICON_X * 2 + rt::ICON, rt::HEIGHT);
        let notice = (self.notes.unread() > 0).then(|| {
            let label = self.unread_label();
            let ns = Style::new(Face::Sans, rt::NOTICE_SIZE);
            let w = 6 + rt::NOTICE_DOT + rt::NOTICE_GAP + self.text.measure(&label, ns) as i32 + 6;
            Rect::new((self.width - w) / 2, 0, w, rt::HEIGHT)
        });
        RidgeLayout { menu, clock, status, notice }
    }

    fn unread_label(&self) -> String {
        let n = self.notes.unread();
        if n == 1 { "1 new notification".into() } else { alloc::format!("{n} new notifications") }
    }

    fn draw_ridge(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let ridge = self.ridge_rect().scale(s);
        if ridge.intersect(&clip).is_empty() {
            return;
        }
        fill(fb, clip, ridge, rt::BACKGROUND, 255);
        let l = self.ridge_layout();
        let fs = s as f32;
        let icon_y = ((rt::HEIGHT - rt::ICON) / 2 * s) as f32;
        // The system menu: Lucide's dashed circle.
        lucide::CIRCLE_DASHED.draw(fb, clip, (rt::MENU_ICON_X * s) as f32, icon_y, (rt::ICON * s) as f32, WHITE, 255);
        // The clock, in a pill that lights up under the pointer.
        let st = Style::new(Face::Sans, rt::TEXT_SIZE).scaled(s);
        let clock = l.clock.scale(s);
        if l.clock.contains(self.pointer.0, self.pointer.1) || self.panel == Some(Panel::Calendar) {
            rounded_rect(fb, clip, clock, clock.height / 2, WHITE, rt::PILL_ALPHA);
        }
        let baseline = self.text.centre(clock.y, clock.height, st);
        let tx = (clock.x + rt::PILL_PADDING * s) as f32;
        self.text.draw_style(fb, clip, tx, baseline, ridge.right(), &self.clock, st, rt::TEXT, 255);
        // The status pill: network and volume.
        let pill = l.status.scale(s);
        rounded_rect(fb, clip, pill, pill.height / 2, WHITE, rt::PILL_ALPHA);
        let ix = (pill.x + rt::PILL_PADDING * s) as f32;
        let size = (rt::ICON * s) as f32;
        let net: (Icon, u8) = match self.network {
            Network::Online(_) => (lucide::ETHERNET_PORT, 255),
            Network::Unknown => (lucide::ETHERNET_PORT, 128),
            Network::Offline => (lucide::WIFI_OFF, 255),
        };
        net.0.draw(fb, clip, ix, icon_y, size, WHITE, net.1);
        let (vol, a) = match self.volume {
            Some((_, true)) => (lucide::VOLUME_X, 255),
            Some((0, false)) => (lucide::VOLUME, 255),
            Some((v, false)) if v <= 50 => (lucide::VOLUME_1, 255),
            Some(_) => (lucide::VOLUME_2, 255),
            None => (lucide::VOLUME_X, 128),
        };
        vol.draw(fb, clip, ix + ((rt::ICON + rt::GAP) * s) as f32, icon_y, size, WHITE, a);
        // The unread notifications in the middle: a dot and the count.
        if let Some(n) = l.notice {
            let n = n.scale(s);
            let cy = n.y + n.height / 2;
            let dr = rt::NOTICE_DOT * s / 2;
            let dx = n.x + 6 * s + dr;
            draw::circle(fb, clip, dx, cy, dr, None, WHITE, rt::NOTICE_DOT_ALPHA);
            let ns = Style::new(Face::Sans, rt::NOTICE_SIZE).scaled(s);
            let x = (n.x + (6 + rt::NOTICE_DOT + rt::NOTICE_GAP) * s) as f32;
            let b = self.text.centre(n.y, n.height, ns);
            self.text.draw_style(fb, clip, x, b, n.right(), &self.unread_label(), ns, WHITE, rt::NOTICE_ALPHA);
        }
        let _ = fs;
    }

    // -----------------------------------------------------------------
    // The surface.

    fn draw_surface(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let Some((kind, g, h)) = self.motion.drawn(self.now_ms) else { return };
        let shape = g.chrome(rt::HEIGHT, h, s);
        if shape.bounds().intersect(&clip).is_empty() {
            return;
        }
        shape.fill(fb, clip, rt::BACKGROUND, 255);
        // The contents slide down with the opening surface and are cut at
        // its edge (logical body at full height, moved up by what is hidden).
        let hidden = (g.height - h).max(0.0) as i32;
        let top = rt::HEIGHT - hidden;
        let body = shape.body.pixels();
        let inner = clip.intersect(&Rect::new(body.x, rt::HEIGHT * s, body.width, body.height));
        if inner.is_empty() {
            return;
        }
        match kind {
            Surface::Launcher => self.draw_launcher(fb, inner, s, top),
            Surface::Switcher => self.draw_switcher(fb, inner, s, top),
            Surface::Panel(p) => self.draw_panel(fb, inner, s, p, g.body(top)),
            Surface::Notices => self.draw_notices(fb, inner, s, g.body(top)),
        }
    }

    fn draw_launcher(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, top: i32) {
        let l = self.launcher_layout(top);
        let fs = s as f32;
        // The search row: Lucide's search glyph at 60 %, the text or the
        // placeholder, the caret.
        let search = l.search.scale(s);
        let cy = search.y + search.height / 2;
        let gx = search.x + lt::SEARCH_PADDING * s;
        let gs = lt::SEARCH_ICON * s;
        lucide::SEARCH.draw(fb, clip, gx as f32, (cy - gs / 2) as f32, gs as f32, WHITE, lt::SEARCH_ICON_ALPHA);
        let tx = gx + (lt::SEARCH_ICON + lt::SEARCH_GAP) * s;
        let st = Style::new(Face::Sans, lt::INPUT_SIZE).scaled(s);
        let baseline = self.text.centre(search.y, search.height, st);
        let right = search.right() - lt::SEARCH_PADDING * s;
        let query = self.launcher.query();
        let end = if query.is_empty() {
            self.text.draw_style(
                fb,
                clip,
                tx as f32,
                baseline,
                right,
                lt::PLACEHOLDER,
                st,
                WHITE,
                lt::PLACEHOLDER_ALPHA,
            );
            tx as f32
        } else {
            self.text.draw_style(fb, clip, tx as f32, baseline, right, query, st, WHITE, 255)
        };
        let caret_h = 18 * s;
        fill(fb, clip, Rect::new(end as i32 + s / 2, cy - caret_h / 2, s.max(1), caret_h), WHITE, 255);
        if l.rows == 0 {
            return;
        }
        let w = l.body.width * s;
        let bx = l.body.x * s;
        fill(fb, clip, Rect::new(bx, search.bottom(), w, s), WHITE, lt::DIVIDER_ALPHA);
        // The section title: 11 px medium at 50 %, `px-2.5 pt-2`.
        let sst = Style::new(Face::Medium, lt::SECTION_SIZE).scaled(s);
        let sx = bx + (lt::LIST_PADDING + lt::ROW_PADDING) * s;
        let sb = search.bottom() + s + 8 * s + (sst.size * 1.5 / 2.0 + self.text.cap(sst) / 2.0) as i32;
        self.text.draw_style(fb, clip, sx as f32, sb, bx + w, "Apps", sst, WHITE, lt::SECTION_ALPHA);
        for (i, &app) in self.launcher.results().iter().enumerate() {
            let row = self.row_rect(&l, i).scale(s);
            if row.intersect(&clip).is_empty() {
                continue;
            }
            let app = &self.apps[app];
            if self.launcher.selected() == Some(i) {
                rounded_rect(fb, clip, row, lt::ROW_RADIUS * s, WHITE, lt::SELECTED_ALPHA);
            } else if self.launcher.hover() == Some(i) {
                rounded_rect(fb, clip, row, lt::ROW_RADIUS * s, WHITE, lt::HOVER_ALPHA);
            }
            // The icon: a Lucide glyph at 40 % in the 28-pixel box.
            let ib = lt::ICON_BOX * s;
            let icon = Rect::new(row.x + lt::ROW_PADDING * s, row.y + (row.height - ib) / 2, ib, ib);
            let glyph = match app.internal() {
                Some(crate::apps::SETTINGS) => lucide::SETTINGS,
                Some(crate::apps::NOTIFICATIONS) => lucide::BELL,
                _ => lucide::APP_WINDOW,
            };
            let gsz = lt::GLYPH * s;
            let (gx, gy) = (icon.x + (ib - gsz) / 2, icon.y + (ib - gsz) / 2);
            glyph.draw(fb, clip, gx as f32, gy as f32, gsz as f32, WHITE, ink::ICON);
            // Name and description: 13 px over 11 px at 50 %.
            let tx = (icon.right() + lt::ROW_ICON_GAP * s) as f32;
            let tst = Style::new(Face::Sans, lt::TITLE_SIZE).scaled(s);
            let sst = Style::new(Face::Sans, lt::SUBTITLE_SIZE).scaled(s);
            let right = row.right() - lt::ROW_PADDING * s;
            let (title_b, sub_b) = if app.comment.is_empty() {
                (self.text.centre(row.y, row.height, tst), 0)
            } else {
                // Two lines of 19.5 and 16.5 pixels, centred together.
                let block = (13.0 * 1.5 + 11.0 * 1.5) * fs;
                let top = row.y as f32 + (row.height as f32 - block) / 2.0;
                let tb = top + (19.5 * fs + self.text.cap(tst)) / 2.0;
                let sb = top + 19.5 * fs + (16.5 * fs + self.text.cap(sst)) / 2.0;
                (tb as i32, sb as i32)
            };
            self.text.draw_style(fb, clip, tx, title_b, right, &app.name, tst, WHITE, 255);
            if !app.comment.is_empty() {
                self.text.draw_style(fb, clip, tx, sub_b, right, &app.comment, sst, WHITE, lt::SUBTITLE_ALPHA);
            }
        }
        // The footer: what Enter does.
        if let Some(f) = l.footer {
            let f = f.scale(s);
            fill(fb, clip, Rect::new(f.x, f.y - s, f.width, s), WHITE, lt::DIVIDER_ALPHA);
            let fst = Style::new(Face::Sans, lt::FOOTER_SIZE).scaled(s);
            let cst = Style::new(Face::Sans, lt::CHIP_SIZE).scaled(s);
            let chip = "\u{21b5}";
            let cw = self.text.measure(chip, cst) as i32 + 12 * s;
            let ch = 16 * s + 2 * s;
            let x = f.right() - lt::SEARCH_PADDING * s - cw;
            let chip_rect = Rect::new(x, f.y + (f.height - ch) / 2, cw, ch);
            rounded_rect(fb, clip, chip_rect, 4 * s, WHITE, ink::SELECTED);
            let cb = self.text.centre(chip_rect.y, chip_rect.height, cst);
            self.text.draw_style(fb, clip, (x + 6 * s) as f32, cb, chip_rect.right(), chip, cst, WHITE, ink::CHIP_TEXT);
            let label = "Open";
            let lw = self.text.measure(label, fst) as i32;
            let lx = x - 6 * s - lw;
            let lb = self.text.centre(f.y, f.height, fst);
            self.text.draw_style(fb, clip, lx as f32, lb, lx + lw + s, label, fst, WHITE, lt::SUBTITLE_ALPHA);
        }
    }

    fn draw_switcher(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32, top: i32) {
        let (_, cards) = self.switcher_layout(top);
        let st = Style::new(Face::Sans, sw::TEXT_SIZE).scaled(s);
        for (i, (card, e)) in cards.iter().zip(self.switcher.entries()).enumerate() {
            let c = card.scale(s);
            let r = sw::RADIUS * s;
            rounded_rect(fb, clip, c, r, WHITE, sw::CARD_ALPHA);
            if i == self.switcher.selected() {
                draw::rounded_border(fb, clip, c, r, 2 * s, WHITE, sw::SELECTED_ALPHA);
            } else {
                rounded_outline(fb, clip, c, r, WHITE, sw::RING_ALPHA);
            }
            // The name row: an app glyph and the title.
            let isz = sw::ICON * s;
            lucide::APP_WINDOW.draw(fb, clip, (c.x + 6 * s) as f32, (c.y + 6 * s) as f32, isz as f32, WHITE, 200);
            let base = self.text.centre(c.y, sw::HEADER * s, st);
            let alpha = if e.minimized { 153 } else { 255 };
            let tx = (c.x + 6 * s + isz + 8 * s) as f32;
            self.text.draw_style(fb, clip, tx, base, c.right() - 6 * s, &e.title, st, WHITE, alpha);
            // The thumbnail, centred in its area (dimmed if minimized).
            let area = Rect::new(c.x, c.y + sw::HEADER * s, c.width, sw::THUMB_HEIGHT * s);
            let th = &e.thumb;
            let (tw, thh) = ((th.width as i32 * s).min(area.width - 8 * s), (th.height as i32 * s).min(area.height));
            let tr = Rect::new(area.x + (area.width - tw) / 2, area.y + (area.height - thh) / 2, tw, thh);
            let vis = tr.intersect(&clip);
            for y in vis.y..vis.bottom() {
                let sy = ((y - tr.y) / s) as u32;
                let row = fb.row_mut(y as u32);
                for x in vis.x..vis.right() {
                    let sx = ((x - tr.x) / s) as u32;
                    let p = th.pixels.get((sy * th.width + sx) as usize).copied().unwrap_or(0);
                    row[x as usize] = if e.minimized { draw::mix(row[x as usize], p, 128) } else { p | 0xff00_0000 };
                }
            }
        }
    }

    fn draw_snap(&self, fb: &mut dyn Framebuffer, clip: Rect, s: i32) {
        let Some(r) = self.snap else { return };
        let r = r.scale(s);
        if r.intersect(&clip).is_empty() {
            return;
        }
        rounded_rect(fb, clip, r, ot::SNAP_RADIUS * s, WHITE, ot::SNAP_ALPHA);
        rounded_outline(fb, clip, r, ot::SNAP_RADIUS * s, WHITE, ot::SNAP_OUTLINE_ALPHA);
    }
}

/// Draw wallpaper `id` on a `w` × `h` (physical) screen inside `clip`
/// (the settings window's tiles use it).
pub fn draw_wallpaper(
    fb: &mut dyn Framebuffer,
    clip: Rect,
    id: &str,
    w: i32,
    h: i32,
    pictures: &crate::picture::Pictures,
) {
    wallpaper::draw(fb, clip, id, w, h, pictures);
}

/// The layer under the windows.
#[derive(Debug)]
pub struct Below<'a>(&'a Shell);

/// The layer over the windows.
#[derive(Debug)]
pub struct Above<'a>(&'a Shell);

impl Layer for Below<'_> {
    fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, scale: i32) {
        let sh = self.0;
        wallpaper::Cache::draw(
            &sh.wall,
            fb,
            clip,
            &sh.settings.wallpaper,
            sh.width * scale,
            sh.height * scale,
            &sh.pictures,
        );
        sh.draw_ridge(fb, clip, scale);
    }
}

impl Layer for Above<'_> {
    fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, scale: i32) {
        self.0.draw_snap(fb, clip, scale);
        self.0.draw_surface(fb, clip, scale);
    }
}

#[cfg(test)]
mod tests;
